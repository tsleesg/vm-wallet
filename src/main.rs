use solana_sdk::{
    pubkey::Pubkey,
    signer::Signer,
    signature::{Keypair, SeedDerivable},
    transaction::Transaction,
    sysvar::{self, clock::Clock, Sysvar},
    account_info::AccountInfo,
};

use solana_client::rpc_client::RpcClient;
use code_vm_api::prelude::*;
use std::{str::FromStr, fs};
use serde::Deserialize;
use chrono::{DateTime, NaiveDateTime, Utc};
use bip39::Mnemonic;
use std::io::{self, Write};

const RPC_URL: &str = "https://api.mainnet-beta.solana.com";

const VM_PROGRAM_ID: &str = "vmZ1WUq8SxjBWcaeTCvgJRZbS84R61uniFsQy5YMRTJ";
const MINT_ADDRESS: &str = "kinXdEcpDQeHPEuQnqmUgtYykqKGVFq6CeVX5iAHJq6";
const VM_STATE_ACCOUNT: &str = "FDrssd3RVeCkgHAT2NkEpkxC5UgfJpKHeebXUMnuzD6D";
const VM_AUTHORITY: &str = "f1ipC31qd2u88MjNYp1T4Cc7rnWfM9ivYpTV1Z8FHnD";

const LOCK_DURATION: u8 = 21;

// PDA seeds
const CODE_VM: &[u8] = b"code_vm";
const VM_UNLOCK_ACCOUNT: &[u8] = b"vm_unlock_pda_account";

#[derive(Deserialize)]
struct KeyFileFormat {
    #[serde(with = "serde_bytes")]
    private_key: Vec<u8>,
    pubkey: String,
}

struct UnlockContext {
    client: RpcClient,
    program_id: Pubkey,
    vm_state: Pubkey,
    mint: Pubkey,
    vm_authority: Pubkey,
    owner: Keypair,
    payer: Keypair,
}

fn format_timestamp(timestamp: i64) -> String {
    let naive = NaiveDateTime::from_timestamp_opt(timestamp, 0)
        .unwrap_or_default();
    let datetime: DateTime<Utc> = DateTime::from_naive_utc_and_offset(naive, Utc);
    datetime.format("%Y-%m-%d %H:%M:%S UTC").to_string()
}

fn setup_owner_keypair() -> Result<(), Box<dyn std::error::Error>> {
    print!("Enter your 12-word mnemonic phrase: ");
    io::stdout().flush()?;
    
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let phrase = input.trim();

    // Validate mnemonic
    let words: Vec<&str> = phrase.split_whitespace().collect();
    if words.len() != 12 {
        return Err("Mnemonic must be exactly 12 words".into());
    }
    
    if phrase.chars().any(|c| !c.is_ascii_lowercase() && !c.is_whitespace()) {
        return Err("Mnemonic can only contain lowercase letters and spaces".into());
    }

    // Generate keypair
    let mnemonic = Mnemonic::parse_normalized(phrase)?;
    let seed = mnemonic.to_seed("");
    let keypair = Keypair::from_seed(&seed[..32])?;

    // Format and save keypair
    let private_key_vec = keypair.secret().to_bytes().to_vec();
    let private_key_str = private_key_vec
        .iter()
        .map(|num| num.to_string())
        .collect::<Vec<String>>()
        .join(", ");

    let formatted = format!(
        "{{\n    \"private_key\": [{}],\n    \"pubkey\": \"{}\"\n}}",
        private_key_str,
        keypair.pubkey().to_string()
    );

    fs::write("owner_key.json", formatted)?;
    println!("Keypair saved to owner_key.json");
    Ok(())
}

impl UnlockContext {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            client: RpcClient::new(RPC_URL),
            program_id: Pubkey::from_str(VM_PROGRAM_ID)?,
            vm_state: Pubkey::from_str(VM_STATE_ACCOUNT)?,
            mint: Pubkey::from_str(MINT_ADDRESS)?,
            vm_authority: Pubkey::from_str(VM_AUTHORITY)?,
            owner: load_keypair_from_file("owner_key.json")?,
            payer: load_keypair_from_file("payer_key.json")?,
        })
    }

    fn get_unlock_pda(&self) -> (Pubkey, u8) {
        let (timelock_address, _) = find_virtual_timelock_address(
            &self.mint,
            &self.vm_authority,
            &self.owner.pubkey(),
            LOCK_DURATION
        );
    
        find_unlock_address(
            &self.owner.pubkey(),
            &timelock_address,
            &self.vm_state  // Using renamed field
        )
    }

    fn check_unlock_account(&self, unlock_pda: &Pubkey) -> Result<bool, Box<dyn std::error::Error>> {
        match self.client.get_account(unlock_pda) {
            Ok(_) => Ok(true),  // Account exists
            Err(_) => Ok(false) // Account doesn't exist
        }
    }    
    
    fn create_unlock_ix(&self, unlock_pda: &Pubkey) -> solana_sdk::instruction::Instruction {
        timelock_unlock_init(
            self.owner.pubkey(),
            self.payer.pubkey(),
            self.vm_state,  // Using renamed field
            *unlock_pda
        )
    }

    fn verify_unlock_pda(&self, unlock_pda: &Pubkey) -> Result<bool, Box<dyn std::error::Error>> {
        let owner_pubkey = self.owner.pubkey();
        let (timelock_address, _) = find_virtual_timelock_address(
            &self.mint,
            &self.vm_authority, 
            &owner_pubkey,
            LOCK_DURATION
        );
        
        let seeds = &[
            CODE_VM,
            VM_UNLOCK_ACCOUNT,
            owner_pubkey.as_ref(),
            timelock_address.as_ref(),
            self.vm_state.as_ref()
        ];
    
        let (expected_pda, _) = Pubkey::find_program_address(seeds, &self.program_id);
        
        Ok(*unlock_pda == expected_pda)
    }    

    fn send_unlock_tx(&self) -> Result<(), Box<dyn std::error::Error>> {
        let (unlock_pda, _) = self.get_unlock_pda();
        let ix = self.create_unlock_ix(&unlock_pda);
        
        // Print instruction details
        println!("Instruction Data:");
        println!("Program ID: {}", ix.program_id);
        println!("Accounts:");
        for (i, acc) in ix.accounts.iter().enumerate() {
            println!("  {}: {} (is_signer: {}, is_writable: {})", 
                i, acc.pubkey, acc.is_signer, acc.is_writable);
        }
        
        let recent_blockhash = self.client.get_latest_blockhash()?;
        println!("Derived Unlock PDA: {}", unlock_pda);
        
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&self.payer.pubkey()),
            &[&self.payer, &self.owner],
            recent_blockhash
        );
    
        let sig = self.client.send_and_confirm_transaction(&tx)?;
        println!("Unlock transaction successful! Signature: {}", sig);
        Ok(())
    }

    fn create_finalize_unlock_ix(&self, unlock_pda: &Pubkey) -> solana_sdk::instruction::Instruction {
        timelock_unlock_finalize(
            self.owner.pubkey(),
            self.payer.pubkey(),
            self.vm_state,
            *unlock_pda
        )
    }

    fn get_unlock_state(&self, unlock_pda: &Pubkey) -> Result<UnlockStateAccount, Box<dyn std::error::Error>> {
        let account = self.client.get_account(unlock_pda)?;
        Ok(UnlockStateAccount::unpack(&account.data))
    }

    fn send_finalize_unlock_tx(&self, unlock_pda: &Pubkey) -> Result<(), Box<dyn std::error::Error>> {
        let ix = self.create_finalize_unlock_ix(unlock_pda);
        let recent_blockhash = self.client.get_latest_blockhash()?;
        
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&self.payer.pubkey()),
            &[&self.payer, &self.owner],
            recent_blockhash
        );

        let sig = self.client.send_and_confirm_transaction(&tx)?;
        println!("Finalize unlock transaction successful! Signature: {}", sig);
        Ok(())
    }

    fn wait_for_unlock(&self, unlock_pda: &Pubkey) -> Result<(), Box<dyn std::error::Error>> {
        loop {
            let unlock_state = self.get_unlock_state(unlock_pda)?;
            
            if unlock_state.is_unlocked() {
                println!("Account is already unlocked!");
                return Ok(());
            }

            if !unlock_state.is_waiting() {
                return Err("Invalid unlock state".into());
            }

            let clock_account = self.client.get_account(&sysvar::clock::id())?;
            let mut lamports = clock_account.lamports;
            let mut data = clock_account.data.clone();
            let current_time = Clock::from_account_info(&AccountInfo::new(
                &sysvar::clock::id(),
                false,
                false,
                &mut lamports,
                &mut data,
                &clock_account.owner,
                clock_account.executable,
                clock_account.rent_epoch,
            ))?.unix_timestamp;            

            if current_time >= unlock_state.unlock_at {
                println!("Timelock duration has passed, proceeding with finalization");
                return self.send_finalize_unlock_tx(unlock_pda);
            }

            println!(
                "Waiting for timelock...\nCurrent time: {} ({})\nUnlock at: {} ({})", 
                current_time, format_timestamp(current_time),
                unlock_state.unlock_at, format_timestamp(unlock_state.unlock_at)
            );
            std::thread::sleep(std::time::Duration::from_secs(60));
        }
    }
    
}

fn load_keypair_from_file(path: &str) -> Result<Keypair, Box<dyn std::error::Error>> {
    let file_content = fs::read_to_string(path)?;
    let stored: KeyFileFormat = serde_json::from_str(&file_content)?;
    let seed: [u8; 32] = stored.private_key.try_into()
        .expect("Invalid private key length");
    Ok(Keypair::from_seed(&seed)?)
}


fn main() -> Result<(), Box<dyn std::error::Error>> {
    // First check if owner_key.json exists
    if !std::path::Path::new("owner_key.json").exists() {
        setup_owner_keypair()?;
    }
    
    let context = UnlockContext::new()?;
    
    // Get and verify the PDA
    let (unlock_pda, _) = context.get_unlock_pda();
    if !context.verify_unlock_pda(&unlock_pda)? {
        println!("PDA verification failed!");
        return Ok(());
    }
    
    println!("PDA verification passed, checking unlock status...");

    // Check if unlock account exists before initializing
    if context.check_unlock_account(&unlock_pda)? {
        println!("Unlock account already initialized, proceeding to wait for unlock");
        context.wait_for_unlock(&unlock_pda)?;
    } else {
        println!("Initializing new unlock...");
        context.send_unlock_tx()?;
        println!("Unlock initialized, waiting for timelock duration...");
        context.wait_for_unlock(&unlock_pda)?;
    }

    println!("Unlock process completed successfully!");
    Ok(())
}