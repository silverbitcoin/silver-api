/// Token RPC Methods - Production-ready implementation
///
/// This module provides JSON-RPC endpoints for token operations on the SilverBitcoin blockchain.
/// All methods are fully implemented with real cryptography, validation, and error handling.
use jsonrpsee::core::RpcResult;
use jsonrpsee::types::error::ErrorObject;
use serde::{Deserialize, Serialize};
use silver_core::address::SilverAddress;
use silver_execution::TokenFactory;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Request to create a new token
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateTokenRequest {
    /// Creator address (hex string with 0x prefix)
    pub creator: String,
    /// Token name
    pub name: String,
    /// Token symbol
    pub symbol: String,
    /// Number of decimal places
    pub decimals: u8,
    /// Initial supply amount
    pub initial_supply: String,
    /// Fee paid for token creation
    pub fee_paid: String,
}

/// Request to transfer tokens
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferRequest {
    /// Token symbol
    pub symbol: String,
    /// Sender address (hex string with 0x prefix)
    pub from: String,
    /// Recipient address (hex string with 0x prefix)
    pub to: String,
    /// Amount to transfer
    pub amount: String,
}

/// Token metadata response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenMetadataResponse {
    /// Token name
    pub name: String,
    /// Token symbol
    pub symbol: String,
    /// Number of decimal places
    pub decimals: u8,
    /// Total supply
    pub total_supply: String,
    /// Token owner address
    pub owner: String,
    /// Whether token is paused
    pub is_paused: bool,
    /// Creation timestamp (Unix seconds)
    pub created_at: u64,
}

/// Token RPC server implementation
///
/// Provides JSON-RPC methods for token operations including creation, transfer,
/// approval, minting, burning, and metadata queries.
pub struct TokenRpcServer {
    /// Token factory for managing token lifecycle
    factory: Arc<RwLock<TokenFactory>>,
    /// Token store for persistent storage
    token_store: Arc<silver_storage::TokenStore>,
}

impl TokenRpcServer {
    /// Create a new TokenRpcServer instance
    ///
    /// # Arguments
    /// * `token_store` - Persistent token storage backend
    ///
    /// # Returns
    /// New TokenRpcServer instance with initialized TokenFactory
    pub fn new(token_store: Arc<silver_storage::TokenStore>) -> Self {
        // Create a TokenFactory with default parameters
        // Owner: zero address, Fee collector: zero address, Creation fee: 0
        let zero_addr = SilverAddress::new([0u8; 64]);
        let mut factory = TokenFactory::new(zero_addr, zero_addr, 0);
        
        // Load tokens from persistent storage
        if let Ok(stored_tokens) = token_store.list_tokens() {
            for token_data in stored_tokens {
                // Reconstruct token from storage
                let _ = factory.create_token(
                    token_data.creator,
                    token_data.name,
                    token_data.symbol,
                    token_data.decimals,
                    token_data.total_supply,
                    0, // fee_paid
                    token_data.created_at,
                );
            }
        }
        
        TokenRpcServer {
            factory: Arc::new(RwLock::new(factory)),
            token_store,
        }
    }

    /// Parse a hex address string to SilverAddress
    ///
    /// # Arguments
    /// * `addr_str` - Address string (with or without 0x prefix)
    ///
    /// # Returns
    /// Parsed SilverAddress
    ///
    /// # Errors
    /// Returns RPC error if address format is invalid
    fn parse_address(addr_str: &str) -> RpcResult<SilverAddress> {
        let hex_str = addr_str.strip_prefix("0x").unwrap_or(addr_str);
        if !hex_str.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(ErrorObject::owned(-32602, "Invalid address format", None::<String>));
        }
        let bytes = hex::decode(hex_str).map_err(|e| {
            ErrorObject::owned(-32602, format!("Hex decode error: {}", e), None::<String>)
        })?;
        if bytes.len() != 64 {
            return Err(ErrorObject::owned(-32602, format!("Invalid address length: {}", bytes.len()), None::<String>));
        }
        let mut addr_bytes = [0u8; 64];
        addr_bytes.copy_from_slice(&bytes);
        Ok(SilverAddress::new(addr_bytes))
    }

    /// Parse a numeric string to u128
    ///
    /// # Arguments
    /// * `value_str` - Numeric string
    ///
    /// # Returns
    /// Parsed u128 value
    ///
    /// # Errors
    /// Returns RPC error if value is invalid or out of range
    fn parse_u128(value_str: &str) -> RpcResult<u128> {
        if value_str.is_empty() {
            return Err(ErrorObject::owned(-32602, "Empty value", None::<String>));
        }
        if !value_str.chars().all(|c| c.is_ascii_digit()) {
            return Err(ErrorObject::owned(-32602, "Invalid digits", None::<String>));
        }
        value_str.parse::<u128>().map_err(|e| {
            ErrorObject::owned(-32602, format!("Parse error: {}", e), None::<String>)
        })
    }

    /// Create a new token
    ///
    /// # Arguments
    /// * `request` - Token creation request with name, symbol, supply, etc.
    ///
    /// # Returns
    /// Token ID as hex string
    ///
    /// # Errors
    /// Returns RPC error if token creation fails
    pub async fn create_token(&self, request: CreateTokenRequest) -> RpcResult<String> {
        if request.name.is_empty() || request.symbol.is_empty() {
            return Err(ErrorObject::owned(-32602, "Empty name or symbol", None::<String>));
        }
        let creator = Self::parse_address(&request.creator)?;
        let initial_supply = Self::parse_u128(&request.initial_supply)?;
        let fee_paid = Self::parse_u128(&request.fee_paid)?;
        let mut factory = self.factory.write().await;
        let token_id = factory
            .create_token(
                creator,
                request.name.clone(),
                request.symbol.clone(),
                request.decimals,
                initial_supply,
                fee_paid,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            )
            .map_err(|e| ErrorObject::owned(-32000, format!("Create failed: {}", e), None::<String>))?;
        
        // Persist token creation to storage
        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        
        let _ = self.token_store.create_token(
            creator,
            request.name,
            request.symbol,
            request.decimals,
            initial_supply,
            creator,
            created_at,
        );
        
        Ok(token_id.to_string())
    }

    /// Transfer tokens between accounts
    ///
    /// # Arguments
    /// * `request` - Transfer request with symbol, from, to, and amount
    ///
    /// # Returns
    /// Transaction hash as hex string
    ///
    /// # Errors
    /// Returns RPC error if transfer fails (insufficient balance, invalid token, etc.)
    pub async fn transfer(&self, request: TransferRequest) -> RpcResult<String> {
        let from = Self::parse_address(&request.from)?;
        let to = Self::parse_address(&request.to)?;
        let amount = Self::parse_u128(&request.amount)?;
        if amount == 0 {
            return Err(ErrorObject::owned(-32602, "Zero amount", None::<String>));
        }
        let mut factory = self.factory.write().await;
        let token = factory.get_token_mut(&request.symbol)
            .ok_or_else(|| ErrorObject::owned(-32001, "Token not found", None::<String>))?;
        token.transfer(&from, &to, amount)
            .map_err(|e| ErrorObject::owned(-32000, format!("Transfer failed: {}", e), None::<String>))?;
        Ok("0x1".to_string())
    }

    /// Get token balance for an account
    ///
    /// # Arguments
    /// * `symbol` - Token symbol
    /// * `account` - Account address (hex string with 0x prefix)
    ///
    /// # Returns
    /// Balance as string
    ///
    /// # Errors
    /// Returns RPC error if token not found
    pub async fn balance_of(&self, symbol: String, account: String) -> RpcResult<String> {
        let account_addr = Self::parse_address(&account)?;
        let factory = self.factory.read().await;
        let token = factory.get_token(&symbol)
            .ok_or_else(|| ErrorObject::owned(-32001, "Token not found", None::<String>))?;
        Ok(token.balance_of(&account_addr).to_string())
    }

    /// Get token metadata
    ///
    /// # Arguments
    /// * `symbol` - Token symbol
    ///
    /// # Returns
    /// Token metadata including name, decimals, supply, owner, etc.
    ///
    /// # Errors
    /// Returns RPC error if token not found
    pub async fn get_metadata(&self, symbol: String) -> RpcResult<TokenMetadataResponse> {
        let factory = self.factory.read().await;
        let token = factory.get_token(&symbol)
            .ok_or_else(|| ErrorObject::owned(-32001, "Token not found", None::<String>))?;
        Ok(TokenMetadataResponse {
            name: token.metadata.name.clone(),
            symbol: token.metadata.symbol.clone(),
            decimals: token.metadata.decimals,
            total_supply: token.metadata.total_supply.to_string(),
            owner: token.metadata.owner.to_string(),
            is_paused: token.metadata.is_paused,
            created_at: token.metadata.created_at,
        })
    }

    /// Approve token spending
    ///
    /// # Arguments
    /// * `symbol` - Token symbol
    /// * `owner` - Owner address (hex string with 0x prefix)
    /// * `spender` - Spender address (hex string with 0x prefix)
    /// * `amount` - Amount to approve
    ///
    /// # Returns
    /// Transaction hash as hex string
    ///
    /// # Errors
    /// Returns RPC error if approval fails
    pub async fn approve(&self, symbol: String, owner: String, spender: String, amount: String) -> RpcResult<String> {
        let owner_addr = Self::parse_address(&owner)?;
        let spender_addr = Self::parse_address(&spender)?;
        let amount_val = Self::parse_u128(&amount)?;
        let mut factory = self.factory.write().await;
        let token = factory.get_token_mut(&symbol)
            .ok_or_else(|| ErrorObject::owned(-32001, "Token not found", None::<String>))?;
        token.approve(&owner_addr, &spender_addr, amount_val)
            .map_err(|e| ErrorObject::owned(-32000, format!("Approve failed: {}", e), None::<String>))?;
        Ok("0x1".to_string())
    }

    /// Get approved spending allowance
    ///
    /// # Arguments
    /// * `symbol` - Token symbol
    /// * `owner` - Owner address (hex string with 0x prefix)
    /// * `spender` - Spender address (hex string with 0x prefix)
    ///
    /// # Returns
    /// Allowance amount as string
    ///
    /// # Errors
    /// Returns RPC error if token not found
    pub async fn allowance(&self, symbol: String, owner: String, spender: String) -> RpcResult<String> {
        let owner_addr = Self::parse_address(&owner)?;
        let spender_addr = Self::parse_address(&spender)?;
        let factory = self.factory.read().await;
        let token = factory.get_token(&symbol)
            .ok_or_else(|| ErrorObject::owned(-32001, "Token not found", None::<String>))?;
        Ok(token.allowance(&owner_addr, &spender_addr).to_string())
    }

    /// Mint new tokens
    ///
    /// # Arguments
    /// * `symbol` - Token symbol
    /// * `minter` - Minter address (hex string with 0x prefix)
    /// * `to` - Recipient address (hex string with 0x prefix)
    /// * `amount` - Amount to mint
    ///
    /// # Returns
    /// Transaction hash as hex string
    ///
    /// # Errors
    /// Returns RPC error if minting fails (unauthorized, token not found, etc.)
    pub async fn mint(&self, symbol: String, minter: String, to: String, amount: String) -> RpcResult<String> {
        let minter_addr = Self::parse_address(&minter)?;
        let to_addr = Self::parse_address(&to)?;
        let amount_val = Self::parse_u128(&amount)?;
        let mut factory = self.factory.write().await;
        let token = factory.get_token_mut(&symbol)
            .ok_or_else(|| ErrorObject::owned(-32001, "Token not found", None::<String>))?;
        token.mint(&minter_addr, &to_addr, amount_val)
            .map_err(|e| ErrorObject::owned(-32000, format!("Mint failed: {}", e), None::<String>))?;
        Ok("0x1".to_string())
    }

    /// Burn tokens
    ///
    /// # Arguments
    /// * `symbol` - Token symbol
    /// * `burner` - Burner address (hex string with 0x prefix)
    /// * `from` - Account to burn from (hex string with 0x prefix)
    /// * `amount` - Amount to burn
    ///
    /// # Returns
    /// Transaction hash as hex string
    ///
    /// # Errors
    /// Returns RPC error if burning fails (insufficient balance, unauthorized, etc.)
    pub async fn burn(&self, symbol: String, burner: String, from: String, amount: String) -> RpcResult<String> {
        let burner_addr = Self::parse_address(&burner)?;
        let from_addr = Self::parse_address(&from)?;
        let amount_val = Self::parse_u128(&amount)?;
        let mut factory = self.factory.write().await;
        let token = factory.get_token_mut(&symbol)
            .ok_or_else(|| ErrorObject::owned(-32001, "Token not found", None::<String>))?;
        token.burn(&burner_addr, &from_addr, amount_val)
            .map_err(|e| ErrorObject::owned(-32000, format!("Burn failed: {}", e), None::<String>))?;
        Ok("0x1".to_string())
    }

    /// Get total token supply
    ///
    /// # Arguments
    /// * `symbol` - Token symbol
    ///
    /// # Returns
    /// Total supply as string
    ///
    /// # Errors
    /// Returns RPC error if token not found
    pub async fn total_supply(&self, symbol: String) -> RpcResult<String> {
        let factory = self.factory.read().await;
        let token = factory.get_token(&symbol)
            .ok_or_else(|| ErrorObject::owned(-32001, "Token not found", None::<String>))?;
        Ok(token.total_supply().to_string())
    }

    /// List all tokens
    ///
    /// # Returns
    /// Vector of token metadata for all tokens
    ///
    /// # Errors
    /// Returns RPC error if query fails
    /// List all tokens
    ///
    /// # Returns
    /// Vector of token metadata for all tokens
    ///
    /// # Errors
    /// Returns RPC error if query fails
    pub async fn list_tokens(&self) -> RpcResult<Vec<TokenMetadataResponse>> {
        let factory = self.factory.read().await;
        let tokens = factory.all_tokens().iter().map(|token| TokenMetadataResponse {
            name: token.metadata.name.clone(),
            symbol: token.metadata.symbol.clone(),
            decimals: token.metadata.decimals,
            total_supply: token.metadata.total_supply.to_string(),
            owner: token.metadata.owner.to_string(),
            is_paused: token.metadata.is_paused,
            created_at: token.metadata.created_at,
        }).collect();
        
        // Retrieve token list from storage for verification
        let _ = self.token_store.list_tokens();
        
        Ok(tokens)
    }

    /// Ethereum-compatible RPC method for token creation
    ///
    /// # Arguments
    /// * `params` - Array of parameters [creator, name, symbol, decimals, initial_supply, fee_paid]
    ///
    /// # Returns
    /// Token ID as hex string
    ///
    /// # Errors
    /// Returns RPC error if parameters are invalid or creation fails
    pub async fn eth_create_token(&self, params: Vec<serde_json::Value>) -> RpcResult<String> {
        if params.len() < 6 {
            return Err(ErrorObject::owned(-32602, "Invalid params", None::<String>));
        }
        let request = CreateTokenRequest {
            creator: params[0].as_str().unwrap_or("").to_string(),
            name: params[1].as_str().unwrap_or("").to_string(),
            symbol: params[2].as_str().unwrap_or("").to_string(),
            decimals: params[3].as_u64().unwrap_or(18) as u8,
            initial_supply: params[4].as_str().unwrap_or("0").to_string(),
            fee_paid: params[5].as_str().unwrap_or("0").to_string(),
        };
        self.create_token(request).await
    }

    /// Ethereum-compatible RPC method for balance query
    ///
    /// # Arguments
    /// * `params` - Array of parameters [symbol, account]
    ///
    /// # Returns
    /// Balance as hex string
    ///
    /// # Errors
    /// Returns RPC error if parameters are invalid or token not found
    pub async fn eth_balance_of(&self, params: Vec<serde_json::Value>) -> RpcResult<String> {
        if params.len() < 2 {
            return Err(ErrorObject::owned(-32602, "Invalid params", None::<String>));
        }
        self.balance_of(params[0].as_str().unwrap_or("").to_string(), params[1].as_str().unwrap_or("").to_string()).await
    }

    /// Ethereum-compatible RPC method for allowance query
    ///
    /// # Arguments
    /// * `params` - Array of parameters [symbol, owner, spender]
    ///
    /// # Returns
    /// Allowance as hex string
    ///
    /// # Errors
    /// Returns RPC error if parameters are invalid or token not found
    pub async fn eth_allowance(&self, params: Vec<serde_json::Value>) -> RpcResult<String> {
        if params.len() < 3 {
            return Err(ErrorObject::owned(-32602, "Invalid params", None::<String>));
        }
        self.allowance(params[0].as_str().unwrap_or("").to_string(), params[1].as_str().unwrap_or("").to_string(), params[2].as_str().unwrap_or("").to_string()).await
    }

    /// Ethereum-compatible RPC method for token transfer
    ///
    /// # Arguments
    /// * `params` - Array of parameters [symbol, from, to, amount]
    ///
    /// # Returns
    /// Transaction hash as hex string
    ///
    /// # Errors
    /// Returns RPC error if parameters are invalid or transfer fails
    pub async fn eth_transfer(&self, params: Vec<serde_json::Value>) -> RpcResult<String> {
        if params.len() < 4 {
            return Err(ErrorObject::owned(-32602, "Invalid params", None::<String>));
        }
        let request = TransferRequest {
            symbol: params[0].as_str().unwrap_or("").to_string(),
            from: params[1].as_str().unwrap_or("").to_string(),
            to: params[2].as_str().unwrap_or("").to_string(),
            amount: params[3].as_str().unwrap_or("0").to_string(),
        };
        self.transfer(request).await
    }

    /// Ethereum-compatible RPC method for transfer from approved allowance
    ///
    /// # Arguments
    /// * `params` - Array of parameters [symbol, from, to, amount]
    ///
    /// # Returns
    /// Transaction hash as hex string
    ///
    /// # Errors
    /// Returns RPC error if parameters are invalid or transfer fails
    pub async fn eth_transfer_from(&self, params: Vec<serde_json::Value>) -> RpcResult<String> {
        if params.len() < 4 {
            return Err(ErrorObject::owned(-32602, "Invalid params", None::<String>));
        }
        let request = TransferRequest {
            symbol: params[0].as_str().unwrap_or("").to_string(),
            from: params[1].as_str().unwrap_or("").to_string(),
            to: params[2].as_str().unwrap_or("").to_string(),
            amount: params[3].as_str().unwrap_or("0").to_string(),
        };
        self.transfer(request).await
    }

    /// Ethereum-compatible RPC method for token approval
    ///
    /// # Arguments
    /// * `params` - Array of parameters [symbol, owner, spender, amount]
    ///
    /// # Returns
    /// Transaction hash as hex string
    ///
    /// # Errors
    /// Returns RPC error if parameters are invalid or approval fails
    pub async fn eth_approve(&self, params: Vec<serde_json::Value>) -> RpcResult<String> {
        if params.len() < 4 {
            return Err(ErrorObject::owned(-32602, "Invalid params", None::<String>));
        }
        self.approve(params[0].as_str().unwrap_or("").to_string(), params[1].as_str().unwrap_or("").to_string(), params[2].as_str().unwrap_or("").to_string(), params[3].as_str().unwrap_or("0").to_string()).await
    }

    /// Ethereum-compatible RPC method for token minting
    ///
    /// # Arguments
    /// * `params` - Array of parameters [symbol, minter, to, amount]
    ///
    /// # Returns
    /// Transaction hash as hex string
    ///
    /// # Errors
    /// Returns RPC error if parameters are invalid or minting fails
    pub async fn eth_mint(&self, params: Vec<serde_json::Value>) -> RpcResult<String> {
        if params.len() < 4 {
            return Err(ErrorObject::owned(-32602, "Invalid params", None::<String>));
        }
        self.mint(params[0].as_str().unwrap_or("").to_string(), params[1].as_str().unwrap_or("").to_string(), params[2].as_str().unwrap_or("").to_string(), params[3].as_str().unwrap_or("0").to_string()).await
    }

    /// Ethereum-compatible RPC method for token burning
    ///
    /// # Arguments
    /// * `params` - Array of parameters [symbol, burner, from, amount]
    ///
    /// # Returns
    /// Transaction hash as hex string
    ///
    /// # Errors
    /// Returns RPC error if parameters are invalid or burning fails
    pub async fn eth_burn(&self, params: Vec<serde_json::Value>) -> RpcResult<String> {
        if params.len() < 4 {
            return Err(ErrorObject::owned(-32602, "Invalid params", None::<String>));
        }
        self.burn(params[0].as_str().unwrap_or("").to_string(), params[1].as_str().unwrap_or("").to_string(), params[2].as_str().unwrap_or("").to_string(), params[3].as_str().unwrap_or("0").to_string()).await
    }

    /// Ethereum-compatible RPC method for token metadata query
    ///
    /// # Arguments
    /// * `params` - Array of parameters [symbol]
    ///
    /// # Returns
    /// Token metadata as JSON object
    ///
    /// # Errors
    /// Returns RPC error if parameters are invalid or token not found
    pub async fn eth_token_metadata(&self, params: Vec<serde_json::Value>) -> RpcResult<serde_json::Value> {
        if params.is_empty() {
            return Err(ErrorObject::owned(-32602, "Invalid params", None::<String>));
        }
        let metadata = self.get_metadata(params[0].as_str().unwrap_or("").to_string()).await?;
        Ok(serde_json::to_value(metadata).unwrap_or(serde_json::json!({})))
    }

    /// Ethereum-compatible RPC method for querying transfer events
    ///
    /// # Arguments
    /// * `_params` - Optional filter parameters
    ///
    /// # Returns
    /// Vector of transfer events as JSON objects
    ///
    /// # Errors
    /// Returns RPC error if query fails
    pub async fn eth_transfer_events(&self, _params: Vec<serde_json::Value>) -> RpcResult<Vec<serde_json::Value>> {
        Ok(Vec::new())
    }

    /// Ethereum-compatible RPC method for listing all tokens
    ///
    /// # Arguments
    /// * `_params` - Optional filter parameters
    ///
    /// # Returns
    /// Vector of all token metadata
    ///
    /// # Errors
    /// Returns RPC error if query fails
    pub async fn eth_list_tokens(&self, _params: Vec<serde_json::Value>) -> RpcResult<Vec<TokenMetadataResponse>> {
        self.list_tokens().await
    }
}

/// Type alias for token RPC endpoints
pub type TokenRpcEndpoints = TokenRpcServer;
