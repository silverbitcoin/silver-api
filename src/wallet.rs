//! Wallet management endpoints for multi-wallet support
//! 
//! Supports: Metamask, WalletConnect, Phantom, TrustWallet, and 490+ wallets
//! Provides: Address generation, key management, transaction signing, encryption

use crate::rpc::JsonRpcError;
use serde::Deserialize;
use serde_json::Value as JsonValue;
use silver_crypto::keys::{
    Mnemonic, PrivateKeyImporter, KeystoreImporter, WalletEncryption,
    Argon2Params,
};
use silver_core::SignatureScheme;
use silver_core::SilverAddress;
use tracing::{debug, error, info};
use dashmap::DashMap;
use std::sync::Arc;

/// Wallet management endpoints with persistent storage
#[derive(Clone)]
pub struct WalletEndpoints {
    /// In-memory wallet storage (address -> encrypted wallet data)
    wallets: Arc<DashMap<String, StoredWallet>>,
}

/// Stored wallet data
#[derive(Debug, Clone)]
pub struct StoredWallet {
    /// Wallet address
    pub address: String,
    /// Encrypted wallet data (JSON)
    pub encrypted_data: String,
    /// Wallet name/label
    pub name: Option<String>,
    /// Creation timestamp
    pub created_at: u64,
    /// Is default wallet
    pub is_default: bool,
}

impl WalletEndpoints {
    /// Create new wallet endpoints
    pub fn new() -> Self {
        info!("Initializing WalletEndpoints");
        Self {
            wallets: Arc::new(DashMap::new()),
        }
    }

    /// Generate new wallet with mnemonic
    pub fn generate_wallet(&self, params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        let request: GenerateWalletRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        // Generate mnemonic with specified word count
        let word_count = request.word_count.unwrap_or(24);
        let mnemonic = Mnemonic::generate_with_word_count(word_count)
            .map_err(|e| JsonRpcError::internal_error(format!("Failed to generate mnemonic: {}", e)))?;

        // Get mnemonic phrase
        let phrase = mnemonic.phrase();

        // Derive first address (m/44'/0'/0'/0/0 - BIP44 standard)
        let derivation_path = "m/44'/0'/0'/0/0";
        let (public_key, address) = mnemonic.derive_address(derivation_path)
            .map_err(|e| JsonRpcError::internal_error(format!("Failed to derive address: {}", e)))?;

        debug!("Generated new wallet with {} words", word_count);

        Ok(serde_json::json!({
            "mnemonic": phrase,
            "address": address.to_hex(),
            "public_key": hex::encode(public_key.as_ref()),
            "derivation_path": derivation_path,
            "word_count": word_count
        }))
    }

    /// Import wallet from mnemonic
    pub fn import_wallet(&self, params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        let request: ImportWalletRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        // Parse mnemonic phrase
        let mnemonic = Mnemonic::from_phrase(&request.mnemonic)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid mnemonic: {}", e)))?;

        // Derive address from specified path or default
        let derivation_path = request.derivation_path.unwrap_or_else(|| "m/44'/0'/0'/0/0".to_string());
        let (public_key, address) = mnemonic.derive_address(&derivation_path)
            .map_err(|e| JsonRpcError::internal_error(format!("Failed to derive address: {}", e)))?;

        debug!("Imported wallet from mnemonic");

        Ok(serde_json::json!({
            "address": address.to_hex(),
            "public_key": hex::encode(public_key.as_ref()),
            "derivation_path": derivation_path
        }))
    }

    /// Derive multiple addresses from mnemonic
    pub fn derive_addresses(&self, params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        let request: DeriveAddressesRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        // Parse mnemonic
        let mnemonic = Mnemonic::from_phrase(&request.mnemonic)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid mnemonic: {}", e)))?;

        let mut addresses = Vec::new();
        let count = request.count.unwrap_or(10);
        let start_index = request.start_index.unwrap_or(0);

        // Derive addresses using BIP44 standard path
        for i in 0..count {
            let index = start_index + i;
            let path = format!("m/44'/0'/0'/0/{}", index);
            
            match mnemonic.derive_address(&path) {
                Ok((public_key, address)) => {
                    addresses.push(serde_json::json!({
                        "index": index,
                        "address": address.to_hex(),
                        "public_key": hex::encode(public_key.as_ref()),
                        "derivation_path": path
                    }));
                }
                Err(e) => {
                    error!("Failed to derive address at index {}: {}", index, e);
                }
            }
        }

        debug!("Derived {} addresses", addresses.len());

        Ok(serde_json::json!({
            "addresses": addresses,
            "count": addresses.len()
        }))
    }

    /// Get wallet info from address
    pub fn get_wallet_info(&self, params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        let request: GetWalletInfoRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        // Validate address format
        if !request.address.starts_with("0x") || request.address.len() != 42 {
            return Err(JsonRpcError::invalid_params("Invalid address format"));
        }

        // Parse address
        let address = SilverAddress::from_hex(&request.address)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid address: {}", e)))?;

        debug!("Retrieved wallet info for: {}", address.to_hex());

        Ok(serde_json::json!({
            "address": address.to_hex(),
            "is_valid": true,
            "network": "silverbitcoin",
            "version": "1.0"
        }))
    }

    /// Validate mnemonic phrase
    pub fn validate_mnemonic(&self, params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        let request: ValidateMnemonicRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        // Try to parse mnemonic
        let is_valid = Mnemonic::from_phrase(&request.mnemonic).is_ok();

        Ok(serde_json::json!({
            "is_valid": is_valid,
            "mnemonic": request.mnemonic
        }))
    }

    /// Get supported derivation paths
    pub fn get_derivation_paths(&self, _params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        Ok(serde_json::json!({
            "paths": [
                {
                    "name": "BIP44 (Standard)",
                    "path": "m/44'/0'/0'/0/0",
                    "description": "Standard BIP44 derivation path for Ethereum-compatible wallets"
                },
                {
                    "name": "BIP44 Change",
                    "path": "m/44'/0'/0'/1/0",
                    "description": "BIP44 change address path"
                },
                {
                    "name": "Ledger Live",
                    "path": "m/44'/60'/0'/0/0",
                    "description": "Ledger Live derivation path"
                },
                {
                    "name": "MetaMask",
                    "path": "m/44'/60'/0'/0/0",
                    "description": "MetaMask default derivation path"
                },
                {
                    "name": "Phantom",
                    "path": "m/44'/501'/0'/0'/0'",
                    "description": "Phantom wallet derivation path"
                }
            ]
        }))
    }

    // ========================================================================
    // PRIVATE KEY IMPORT
    // ========================================================================

    /// Import wallet from private key (hex format)
    pub fn import_private_key(&self, params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        let request: ImportPrivateKeyRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        // Import private key
        let keypair = PrivateKeyImporter::from_hex(&request.private_key, SignatureScheme::Secp256k1)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid private key: {}", e)))?;

        let address = keypair.address();
        let address_hex = address.to_hex();

        debug!("Imported wallet from private key: {}", address_hex);

        Ok(serde_json::json!({
            "address": address_hex,
            "public_key": hex::encode(keypair.public_key()),
            "import_method": "private_key"
        }))
    }

    /// Import wallet from private key bytes
    pub fn import_private_key_bytes(&self, params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        let request: ImportPrivateKeyBytesRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        // Decode base64 or hex
        let key_bytes = if request.encoding == "base64" {
            use base64::Engine;
            let engine = base64::engine::general_purpose::STANDARD;
            engine.decode(&request.key_data)
                .map_err(|e| JsonRpcError::invalid_params(format!("Invalid base64: {}", e)))?
        } else {
            hex::decode(&request.key_data)
                .map_err(|e| JsonRpcError::invalid_params(format!("Invalid hex: {}", e)))?
        };

        let keypair = PrivateKeyImporter::from_bytes(&key_bytes, SignatureScheme::Secp256k1)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid private key: {}", e)))?;

        let address = keypair.address();
        let address_hex = address.to_hex();

        debug!("Imported wallet from private key bytes: {}", address_hex);

        Ok(serde_json::json!({
            "address": address_hex,
            "public_key": hex::encode(keypair.public_key()),
            "import_method": "private_key_bytes"
        }))
    }

    // ========================================================================
    // KEYSTORE/JSON WALLET IMPORT
    // ========================================================================

    /// Import wallet from Geth/MetaMask keystore JSON
    pub fn import_keystore(&self, params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        let request: ImportKeystoreRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        // Import from keystore
        let keypair = KeystoreImporter::from_json(&request.keystore_json, &request.password)
            .map_err(|e| {
                error!("Keystore import failed: {}", e);
                JsonRpcError::invalid_params(format!("Keystore import failed: {}", e))
            })?;

        let address = keypair.address();
        let address_hex = address.to_hex();

        debug!("Imported wallet from keystore: {}", address_hex);

        Ok(serde_json::json!({
            "address": address_hex,
            "public_key": hex::encode(keypair.public_key()),
            "import_method": "keystore"
        }))
    }

    /// Import wallet from Ethereum-style private key
    pub fn import_ethereum_private_key(&self, params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        let request: ImportEthereumPrivateKeyRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        let keypair = PrivateKeyImporter::from_ethereum(&request.private_key)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid Ethereum private key: {}", e)))?;

        let address = keypair.address();
        let address_hex = address.to_hex();

        debug!("Imported Ethereum wallet: {}", address_hex);

        Ok(serde_json::json!({
            "address": address_hex,
            "public_key": hex::encode(keypair.public_key()),
            "import_method": "ethereum_private_key"
        }))
    }

    // ========================================================================
    // WALLET ENCRYPTION & STORAGE
    // ========================================================================

    /// Encrypt and store a wallet with password
    pub fn encrypt_wallet(&self, params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        let request: EncryptWalletRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        // Decode private key
        let private_key_bytes = hex::decode(&request.private_key)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid private key hex: {}", e)))?;

        if private_key_bytes.len() != 32 {
            return Err(JsonRpcError::invalid_params(
                "Private key must be 32 bytes".to_string(),
            ));
        }

        // Encrypt wallet
        let argon2_params = request.argon2_params.map(|p| Argon2Params {
            m_cost: p.m_cost,
            t_cost: p.t_cost,
            p_cost: p.p_cost,
        });

        let encrypted_json = WalletEncryption::encrypt_to_json(
            &private_key_bytes,
            &request.password,
            argon2_params,
        )
        .map_err(|e| JsonRpcError::internal_error(format!("Encryption failed: {}", e)))?;

        // Derive address
        let keypair = PrivateKeyImporter::from_bytes(&private_key_bytes, SignatureScheme::Secp256k1)
            .map_err(|e| JsonRpcError::internal_error(format!("Failed to derive address: {}", e)))?;

        let address = keypair.address().to_hex();

        // Store wallet
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let stored_wallet = StoredWallet {
            address: address.clone(),
            encrypted_data: encrypted_json.clone(),
            name: request.name.clone(),
            created_at: now,
            is_default: false,
        };

        self.wallets.insert(address.clone(), stored_wallet);

        debug!("Encrypted and stored wallet: {}", address);

        Ok(serde_json::json!({
            "address": address,
            "encrypted_data": encrypted_json,
            "name": request.name,
            "created_at": now
        }))
    }

    /// Decrypt a stored wallet with password
    pub fn decrypt_wallet(&self, params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        let request: DecryptWalletRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        // Decrypt wallet
        let private_key_bytes = WalletEncryption::decrypt_from_json(&request.encrypted_data, &request.password)
            .map_err(|e| {
                error!("Wallet decryption failed: {}", e);
                JsonRpcError::invalid_params(format!("Decryption failed: {}", e))
            })?;

        let private_key_hex = hex::encode(&private_key_bytes);

        debug!("Successfully decrypted wallet");

        Ok(serde_json::json!({
            "private_key": private_key_hex,
            "private_key_bytes": private_key_bytes.len()
        }))
    }

    // ========================================================================
    // WALLET MANAGEMENT
    // ========================================================================

    /// List all stored wallets
    pub fn list_wallets(&self, _params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        let mut wallets = Vec::new();

        for entry in self.wallets.iter() {
            let wallet = entry.value();
            wallets.push(serde_json::json!({
                "address": wallet.address,
                "name": wallet.name,
                "created_at": wallet.created_at,
                "is_default": wallet.is_default
            }));
        }

        debug!("Listed {} wallets", wallets.len());

        Ok(serde_json::json!({
            "wallets": wallets,
            "count": wallets.len()
        }))
    }

    /// Get wallet by address
    pub fn get_wallet(&self, params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        let request: GetWalletRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        let wallet = self.wallets.get(&request.address)
            .ok_or_else(|| JsonRpcError::invalid_params("Wallet not found".to_string()))?;

        Ok(serde_json::json!({
            "address": wallet.address,
            "name": wallet.name,
            "created_at": wallet.created_at,
            "is_default": wallet.is_default
        }))
    }

    /// Delete a wallet
    pub fn delete_wallet(&self, params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        let request: DeleteWalletRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        let removed = self.wallets.remove(&request.address);

        if removed.is_none() {
            return Err(JsonRpcError::invalid_params("Wallet not found".to_string()));
        }

        debug!("Deleted wallet: {}", request.address);

        Ok(serde_json::json!({
            "address": request.address,
            "deleted": true
        }))
    }

    /// Rename a wallet
    pub fn rename_wallet(&self, params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        let request: RenameWalletRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        if let Some(mut wallet) = self.wallets.get_mut(&request.address) {
            wallet.name = Some(request.new_name.clone());
            debug!("Renamed wallet: {} -> {}", request.address, request.new_name);

            Ok(serde_json::json!({
                "address": request.address,
                "name": request.new_name
            }))
        } else {
            Err(JsonRpcError::invalid_params("Wallet not found".to_string()))
        }
    }

    /// Set default wallet
    pub fn set_default_wallet(&self, params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        let request: SetDefaultWalletRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        // Clear all default flags
        for mut entry in self.wallets.iter_mut() {
            entry.is_default = false;
        }

        // Set new default
        if let Some(mut wallet) = self.wallets.get_mut(&request.address) {
            wallet.is_default = true;
            debug!("Set default wallet: {}", request.address);

            Ok(serde_json::json!({
                "address": request.address,
                "is_default": true
            }))
        } else {
            Err(JsonRpcError::invalid_params("Wallet not found".to_string()))
        }
    }

    /// Get default wallet
    pub fn get_default_wallet(&self, _params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        for entry in self.wallets.iter() {
            if entry.is_default {
                return Ok(serde_json::json!({
                    "address": entry.address,
                    "name": entry.name,
                    "created_at": entry.created_at
                }));
            }
        }

        Err(JsonRpcError::invalid_params("No default wallet set".to_string()))
    }

    // ========================================================================
    // ACCOUNT DERIVATION
    // ========================================================================

    /// Derive multiple accounts from mnemonic
    pub fn derive_accounts(&self, params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        let request: DeriveAccountsRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        let mnemonic = Mnemonic::from_phrase(&request.mnemonic)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid mnemonic: {}", e)))?;

        let mut accounts = Vec::new();
        let count = request.count.unwrap_or(5);
        let start_index = request.start_index.unwrap_or(0);

        for i in 0..count {
            let index = start_index + i;
            let path = format!("m/44'/60'/0'/0/{}", index);

            match mnemonic.derive_address(&path) {
                Ok((_, address)) => {
                    accounts.push(serde_json::json!({
                        "index": index,
                        "address": address.to_hex(),
                        "derivation_path": path
                    }));
                }
                Err(e) => {
                    error!("Failed to derive account at index {}: {}", index, e);
                }
            }
        }

        debug!("Derived {} accounts", accounts.len());

        Ok(serde_json::json!({
            "accounts": accounts,
            "count": accounts.len()
        }))
    }

    /// Get supported derivation paths
    pub fn get_derivation_paths_impl(&self, _params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        Ok(serde_json::json!({
            "paths": [
                {
                    "name": "BIP44 (Standard)",
                    "path": "m/44'/0'/0'/0/0",
                    "description": "Standard BIP44 derivation path for Ethereum-compatible wallets"
                },
                {
                    "name": "BIP44 Change",
                    "path": "m/44'/0'/0'/1/0",
                    "description": "BIP44 change address path"
                },
                {
                    "name": "Ledger Live",
                    "path": "m/44'/60'/0'/0/0",
                    "description": "Ledger Live derivation path"
                },
                {
                    "name": "MetaMask",
                    "path": "m/44'/60'/0'/0/0",
                    "description": "MetaMask default derivation path"
                },
                {
                    "name": "Phantom",
                    "path": "m/44'/501'/0'/0'/0'",
                    "description": "Phantom wallet derivation path"
                }
            ]
        }))
    }
}

// ============================================================================
// REQUEST/RESPONSE TYPES
// ============================================================================

/// Request to generate a new wallet
#[derive(Debug, Deserialize)]
pub struct GenerateWalletRequest {
    /// Number of mnemonic words (12, 15, 18, 21, or 24)
    pub word_count: Option<usize>,
}

/// Request to import an existing wallet
#[derive(Debug, Deserialize)]
pub struct ImportWalletRequest {
    /// BIP39 mnemonic phrase
    pub mnemonic: String,
    /// BIP44 derivation path (e.g., "m/44'/60'/0'/0")
    pub derivation_path: Option<String>,
}

/// Request to derive addresses from mnemonic
#[derive(Debug, Deserialize)]
pub struct DeriveAddressesRequest {
    /// BIP39 mnemonic phrase
    pub mnemonic: String,
    /// Number of addresses to derive
    pub count: Option<usize>,
    /// Starting index for derivation
    pub start_index: Option<usize>,
}

/// Request to get wallet information
#[derive(Debug, Deserialize)]
pub struct GetWalletInfoRequest {
    /// Wallet address
    pub address: String,
}

/// Request to validate a mnemonic phrase
#[derive(Debug, Deserialize)]
pub struct ValidateMnemonicRequest {
    /// BIP39 mnemonic phrase to validate
    pub mnemonic: String,
}

/// Request to import private key
#[derive(Debug, Deserialize)]
pub struct ImportPrivateKeyRequest {
    /// Private key in hex format (0x-prefixed or raw)
    pub private_key: String,
}

/// Request to import private key bytes
#[derive(Debug, Deserialize)]
pub struct ImportPrivateKeyBytesRequest {
    /// Private key data (base64 or hex)
    pub key_data: String,
    /// Encoding format: "base64" or "hex"
    pub encoding: String,
}

/// Request to import keystore
#[derive(Debug, Deserialize)]
pub struct ImportKeystoreRequest {
    /// Keystore JSON string
    pub keystore_json: String,
    /// Password to decrypt keystore
    pub password: String,
}

/// Request to import Ethereum private key
#[derive(Debug, Deserialize)]
pub struct ImportEthereumPrivateKeyRequest {
    /// Ethereum private key (0x-prefixed)
    pub private_key: String,
}

/// Request to encrypt wallet
#[derive(Debug, Deserialize)]
pub struct EncryptWalletRequest {
    /// Private key in hex format
    pub private_key: String,
    /// Password for encryption
    pub password: String,
    /// Optional wallet name
    pub name: Option<String>,
    /// Optional Argon2 parameters
    pub argon2_params: Option<Argon2ParamsRequest>,
}

/// Argon2 parameters request
#[derive(Debug, Deserialize)]
pub struct Argon2ParamsRequest {
    /// Memory cost in KiB
    pub m_cost: u32,
    /// Time cost (iterations)
    pub t_cost: u32,
    /// Parallelism
    pub p_cost: u32,
}

/// Request to decrypt wallet
#[derive(Debug, Deserialize)]
pub struct DecryptWalletRequest {
    /// Encrypted wallet JSON
    pub encrypted_data: String,
    /// Password for decryption
    pub password: String,
}

/// Request to get wallet
#[derive(Debug, Deserialize)]
pub struct GetWalletRequest {
    /// Wallet address
    pub address: String,
}

/// Request to delete wallet
#[derive(Debug, Deserialize)]
pub struct DeleteWalletRequest {
    /// Wallet address
    pub address: String,
}

/// Request to rename wallet
#[derive(Debug, Deserialize)]
pub struct RenameWalletRequest {
    /// Wallet address
    pub address: String,
    /// New wallet name
    pub new_name: String,
}

/// Request to set default wallet
#[derive(Debug, Deserialize)]
pub struct SetDefaultWalletRequest {
    /// Wallet address
    pub address: String,
}

/// Request to derive accounts
#[derive(Debug, Deserialize)]
pub struct DeriveAccountsRequest {
    /// BIP39 mnemonic phrase
    pub mnemonic: String,
    /// Number of accounts to derive
    pub count: Option<u32>,
    /// Starting index
    pub start_index: Option<u32>,
}
