use solana_sdk::{
    pubkey::Pubkey,
    signer::Signer,
    signature::{Keypair, SeedDerivable},
    transaction::Transaction,
};
use solana_client::rpc_client::RpcClient;
use code_vm_api::prelude::*;
use spl_associated_token_account::get_associated_token_address;
use spl_associated_token_account::instruction::create_associated_token_account;
use std::{str::FromStr, fs};
use serde::{Deserialize, Serialize};

const RPC_URL: &str = "https://api.mainnet-beta.solana.com";
const MINT_ADDRESS: &str = "kinXdEcpDQeHPEuQnqmUgtYykqKGVFq6CeVX5iAHJq6";
const VM_STATE_ACCOUNT: &str = "FDrssd3RVeCkgHAT2NkEpkxC5UgfJpKHeebXUMnuzD6D";
const VM_AUTHORITY: &str = "f1ipC31qd2u88MjNYp1T4Cc7rnWfM9ivYpTV1Z8FHnD";
const LOCK_DURATION: u8 = 21;

#[derive(Serialize, Deserialize)]
struct KeyFileFormat {
    #[serde(with = "serde_bytes")]
    private_key: Vec<u8>,
    pubkey: String,
}

fn get_instance_hash() -> Result<Hash, Box<dyn std::error::Error>> {
    let current_time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let hash_input = format!("instance_{}", current_time);
    let mut hash_bytes = [0u8; 32];
    let input_bytes = hash_input.as_bytes();
    hash_bytes[..input_bytes.len().min(32)].copy_from_slice(&input_bytes[..input_bytes.len().min(32)]);
    Ok(Hash::new_from_array(hash_bytes))}

fn get_account_index() -> Result<u16, Box<dyn std::error::Error>> {
    // In production this should be fetched from state management
    Ok(0)
}

struct WithdrawContext {
    client: RpcClient,
    vm_state: Pubkey,
    mint: Pubkey,
    vm_authority: Pubkey,
    owner: Keypair,
    payer: Keypair,
    instance_hash: Hash,
    account_index: u16,
    vm_memory: Option<Pubkey>,
}

impl WithdrawContext {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            client: RpcClient::new(RPC_URL),
            vm_state: Pubkey::from_str(VM_STATE_ACCOUNT)?,
            mint: Pubkey::from_str(MINT_ADDRESS)?,
            vm_authority: Pubkey::from_str(VM_AUTHORITY)?,
            owner: load_keypair_from_file("owner_key.json")?,
            payer: load_keypair_from_file("payer_key.json")?,
            instance_hash: get_instance_hash()?,
            account_index: get_account_index()?,
            vm_memory: None,
        })
    }

    fn get_withdraw_pdas(&self) -> (Pubkey, Pubkey, Pubkey, u8) {
        let (timelock_address, _) = find_virtual_timelock_address(
            &self.mint,
            &self.vm_authority,
            &self.owner.pubkey(),
            LOCK_DURATION
        );

        let (unlock_pda, _) = find_unlock_address(
            &self.owner.pubkey(),
            &timelock_address,
            &self.vm_state
        );

        let (receipt_pda, receipt_bump) = find_withdraw_receipt_address(
            &unlock_pda,
            &self.instance_hash,
            &self.vm_state
        );

        (timelock_address, unlock_pda, receipt_pda, receipt_bump)
    }

    fn verify_account_state(&self) -> Result<(), Box<dyn std::error::Error>> {
        let unlock_state = self.get_unlock_state()?;
        if !unlock_state.is_unlocked() {
            return Err("Account not unlocked".into());
        }
        Ok(())
    }

    fn get_unlock_state(&self) -> Result<UnlockStateAccount, Box<dyn std::error::Error>> {
        let (_, unlock_pda, _, _) = self.get_withdraw_pdas();
        let account = self.client.get_account(&unlock_pda)?;
        Ok(UnlockStateAccount::unpack(&account.data))
    }

    fn create_withdraw_ix(
        &self,
        destination_ata: &Pubkey,
    ) -> Result<solana_sdk::instruction::Instruction, Box<dyn std::error::Error>> {
        let (_, unlock_pda, receipt_pda, _) = self.get_withdraw_pdas();
        let vm = self.client.get_account(&self.vm_state)?;
        let vm_data = CodeVmAccount::unpack(&vm.data);
    
        Ok(timelock_withdraw(
            self.owner.pubkey(),
            self.payer.pubkey(), 
            self.vm_state,
            Some(vm_data.omnibus.vault),
            self.vm_memory,
            None,                // vm_storage
            None,               // deposit_pda
            None,               // deposit_ata
            unlock_pda,
            Some(receipt_pda),
            *destination_ata,   
            WithdrawIxData::FromMemory { 
                account_index: self.account_index 
            }
        ))
    }     
    
    fn execute_withdraw(&self, destination_ata: &Pubkey) -> Result<(), Box<dyn std::error::Error>> {
        self.verify_account_state()?;
        
        let ix = self.create_withdraw_ix(destination_ata)?;
        let recent_blockhash = self.client.get_latest_blockhash()?;
        
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&self.payer.pubkey()),
            &[&self.payer, &self.owner],
            recent_blockhash
        );

        let sig = self.client.send_and_confirm_transaction_with_spinner(&tx)?;
        println!("Withdrawal successful!\nTransaction: https://solscan.io/tx/{}", sig);
        
        Ok(())
    }
}

fn load_keypair_from_file(path: &str) -> Result<Keypair, Box<dyn std::error::Error>> {
    let file_content = fs::read_to_string(path)?;
    let stored: KeyFileFormat = serde_json::from_str(&file_content)?;
    let seed: [u8; 32] = stored.private_key.try_into()
        .map_err(|_| "Invalid private key length")?;
    Ok(Keypair::from_seed(&seed)?)
}

fn setup_destination_ata(
    context: &WithdrawContext
) -> Result<Pubkey, Box<dyn std::error::Error>> {
    let destination = get_associated_token_address(
        &context.owner.pubkey(),
        &context.mint
    );

    if context.client.get_account(&destination).is_err() {
        let ix = create_associated_token_account(
            &context.payer.pubkey(),
            &context.owner.pubkey(),
            &context.mint,
            &solana_sdk::system_program::ID  // Add system program ID
        );

        let recent_blockhash = context.client.get_latest_blockhash()?;
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&context.payer.pubkey()),
            &[&context.payer],
            recent_blockhash
        );

        context.client.send_and_confirm_transaction(&tx)?;
    }

    Ok(destination)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let context = WithdrawContext::new()?;    
    println!("Initializing withdrawal process...");
    println!("Owner: {}", context.owner.pubkey());
    
    let destination_ata = setup_destination_ata(&context)?;
    println!("Destination ATA: {}", destination_ata);
    
    println!("Executing withdrawal...");
    context.execute_withdraw(&destination_ata)?;
    
    println!("Withdrawal completed successfully!");
    Ok(())
}