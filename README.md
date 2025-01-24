# VM Wallet Unlock Utility

A command-line utility to unlock timelock tokens on Solana.

⚠️ **DISCLAIMER**

This software is provided "as is", without warranty of any kind. It has:
- Not undergone security audits
- Not been tested in production environments
- No guarantees of reliability or safety
- Potential risks of loss if used incorrectly

Use at your own risk. This is experimental software intended for testing purposes only.

## Prerequisites

- Rust toolchain installed
- Solana CLI tools installed
- Your 12-word mnemonic phrase for the owner wallet
- A funded payer wallet for transaction fees

## Setup

1. Create `payer_key.json` with the following format:
    ```
    {
        "private_key": [123, 456, ...], // 32 bytes array
        "pubkey": "SolanaPubkeyString..."
    }
    ```

2. First run will prompt for your 12-word mnemonic to generate `owner_key.json`. You can get this from your flipchat wallet.

## Usage

    ```bash
    cargo run
    ```

The utility will:
1. Generate owner keypair from mnemonic if not present
2. Verify PDA derivation
3. Check if unlock is already initialised
4. Initialise unlock if needed
5. Wait for timelock duration (21 days)
6. Finalise unlock automatically

## Key File Formats

Both `owner_key.json` and `payer_key.json` must follow this structure:

    ```
    {
        "private_key": [1, 2, 3, ...], // Array of 32 integers (0-255)
        "pubkey": "PublicKeyString"    // Base58 encoded public key
    }
    ```

## Security Notes

- Keep your mnemonic phrase and key files secure
- Never share private keys
- Back up your key files safely
- Use only on trusted machines

## Network

Configured for Solana mainnet at:
- RPC: https://api.mainnet-beta.solana.com
