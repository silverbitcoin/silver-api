//! DEX RPC Endpoints - Production-ready JSON-RPC methods for DEX operations
//!
//! Provides JSON-RPC endpoints for:
//! - Pool creation and management
//! - Liquidity provision (add/remove)
//! - Token swaps
//! - Price queries
//! - Pool information and statistics

use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use silver_execution::DEXState;

/// DEX RPC endpoints
pub struct DexEndpoints {
    /// DEX state manager
    dex_state: Arc<RwLock<DEXState>>,
}

impl DexEndpoints {
    /// Create new DEX endpoints
    pub fn new(owner: String, fee_collector: String, protocol_fee: u64) -> Self {
        let dex_state = DEXState::new(owner, fee_collector, protocol_fee);
        Self {
            dex_state: Arc::new(RwLock::new(dex_state)),
        }
    }

    /// Create a new liquidity pool
    pub async fn create_pool(&self, params: Value) -> Result<Value, String> {
        debug!("DEX: create_pool called with params: {:?}", params);

        let params_array = params.as_array().ok_or("Invalid params format")?;
        if params_array.is_empty() {
            return Err("Missing parameters".to_string());
        }

        let param = &params_array[0];
        let creator = param
            .get("creator")
            .and_then(|v| v.as_str())
            .ok_or("Missing creator")?
            .to_string();
        let token_a = param
            .get("token_a")
            .and_then(|v| v.as_str())
            .ok_or("Missing token_a")?
            .to_string();
        let token_b = param
            .get("token_b")
            .and_then(|v| v.as_str())
            .ok_or("Missing token_b")?
            .to_string();
        let fee_percentage = param
            .get("fee_percentage")
            .and_then(|v| v.as_u64())
            .ok_or("Missing fee_percentage")?;

        let mut dex = self.dex_state.write().await;
        match dex.create_pool(creator, token_a, token_b, fee_percentage) {
            Ok(pool_id) => {
                info!("DEX: Pool created with ID: {}", pool_id);
                Ok(json!({
                    "pool_id": pool_id,
                    "status": "success"
                }))
            }
            Err(e) => {
                warn!("DEX: Failed to create pool: {}", e);
                Err(format!("Failed to create pool: {}", e))
            }
        }
    }

    /// Add liquidity to a pool
    pub async fn add_liquidity(&self, params: Value) -> Result<Value, String> {
        debug!("DEX: add_liquidity called with params: {:?}", params);

        let params_array = params.as_array().ok_or("Invalid params format")?;
        if params_array.is_empty() {
            return Err("Missing parameters".to_string());
        }

        let param = &params_array[0];
        let provider = param
            .get("provider")
            .and_then(|v| v.as_str())
            .ok_or("Missing provider")?
            .to_string();
        let pool_id = param
            .get("pool_id")
            .and_then(|v| v.as_u64())
            .ok_or("Missing pool_id")?;
        let amount_a = param
            .get("amount_a")
            .and_then(|v| v.as_str())
            .ok_or("Missing amount_a")?
            .parse::<u128>()
            .map_err(|_| "Invalid amount_a")?;
        let amount_b = param
            .get("amount_b")
            .and_then(|v| v.as_str())
            .ok_or("Missing amount_b")?
            .parse::<u128>()
            .map_err(|_| "Invalid amount_b")?;

        let mut dex = self.dex_state.write().await;
        match dex.add_liquidity(provider, pool_id, amount_a, amount_b) {
            Ok(shares) => {
                info!("DEX: Liquidity added, shares: {}", shares);
                Ok(json!({
                    "shares": shares.to_string(),
                    "status": "success"
                }))
            }
            Err(e) => {
                warn!("DEX: Failed to add liquidity: {}", e);
                Err(format!("Failed to add liquidity: {}", e))
            }
        }
    }

    /// Remove liquidity from a pool
    pub async fn remove_liquidity(&self, params: Value) -> Result<Value, String> {
        debug!("DEX: remove_liquidity called with params: {:?}", params);

        let params_array = params.as_array().ok_or("Invalid params format")?;
        if params_array.is_empty() {
            return Err("Missing parameters".to_string());
        }

        let param = &params_array[0];
        let provider = param
            .get("provider")
            .and_then(|v| v.as_str())
            .ok_or("Missing provider")?
            .to_string();
        let pool_id = param
            .get("pool_id")
            .and_then(|v| v.as_u64())
            .ok_or("Missing pool_id")?;
        let shares = param
            .get("shares")
            .and_then(|v| v.as_str())
            .ok_or("Missing shares")?
            .parse::<u128>()
            .map_err(|_| "Invalid shares")?;

        let mut dex = self.dex_state.write().await;
        match dex.remove_liquidity(provider, pool_id, shares) {
            Ok((amount_a, amount_b)) => {
                info!(
                    "DEX: Liquidity removed, amount_a: {}, amount_b: {}",
                    amount_a, amount_b
                );
                Ok(json!({
                    "amount_a": amount_a.to_string(),
                    "amount_b": amount_b.to_string(),
                    "status": "success"
                }))
            }
            Err(e) => {
                warn!("DEX: Failed to remove liquidity: {}", e);
                Err(format!("Failed to remove liquidity: {}", e))
            }
        }
    }

    /// Perform a token swap
    pub async fn swap(&self, params: Value) -> Result<Value, String> {
        debug!("DEX: swap called with params: {:?}", params);

        let params_array = params.as_array().ok_or("Invalid params format")?;
        if params_array.is_empty() {
            return Err("Missing parameters".to_string());
        }

        let param = &params_array[0];
        let swapper = param
            .get("swapper")
            .and_then(|v| v.as_str())
            .ok_or("Missing swapper")?
            .to_string();
        let pool_id = param
            .get("pool_id")
            .and_then(|v| v.as_u64())
            .ok_or("Missing pool_id")?;
        let token_in = param
            .get("token_in")
            .and_then(|v| v.as_str())
            .ok_or("Missing token_in")?
            .to_string();
        let amount_in = param
            .get("amount_in")
            .and_then(|v| v.as_str())
            .ok_or("Missing amount_in")?
            .parse::<u128>()
            .map_err(|_| "Invalid amount_in")?;
        let min_amount_out = param
            .get("min_amount_out")
            .and_then(|v| v.as_str())
            .ok_or("Missing min_amount_out")?
            .parse::<u128>()
            .map_err(|_| "Invalid min_amount_out")?;

        let mut dex = self.dex_state.write().await;
        match dex.swap(swapper, pool_id, token_in, amount_in, min_amount_out) {
            Ok(amount_out) => {
                info!("DEX: Swap executed, amount_out: {}", amount_out);
                Ok(json!({
                    "amount_out": amount_out.to_string(),
                    "status": "success"
                }))
            }
            Err(e) => {
                warn!("DEX: Swap failed: {}", e);
                Err(format!("Swap failed: {}", e))
            }
        }
    }

    /// Get price of token_out in terms of token_in
    pub async fn get_price(&self, params: Value) -> Result<Value, String> {
        debug!("DEX: get_price called with params: {:?}", params);

        let params_array = params.as_array().ok_or("Invalid params format")?;
        if params_array.is_empty() {
            return Err("Missing parameters".to_string());
        }

        let param = &params_array[0];
        let pool_id = param
            .get("pool_id")
            .and_then(|v| v.as_u64())
            .ok_or("Missing pool_id")?;
        let token_in = param
            .get("token_in")
            .and_then(|v| v.as_str())
            .ok_or("Missing token_in")?
            .to_string();
        let amount_in = param
            .get("amount_in")
            .and_then(|v| v.as_str())
            .ok_or("Missing amount_in")?
            .parse::<u128>()
            .map_err(|_| "Invalid amount_in")?;

        let dex = self.dex_state.read().await;
        match dex.get_price(pool_id, token_in, amount_in) {
            Ok(price) => {
                debug!("DEX: Price calculated: {}", price);
                Ok(json!({
                    "price": price.to_string(),
                    "status": "success"
                }))
            }
            Err(e) => {
                warn!("DEX: Failed to get price: {}", e);
                Err(format!("Failed to get price: {}", e))
            }
        }
    }

    /// Get pool information
    pub async fn get_pool(&self, params: Value) -> Result<Value, String> {
        debug!("DEX: get_pool called with params: {:?}", params);

        let params_array = params.as_array().ok_or("Invalid params format")?;
        if params_array.is_empty() {
            return Err("Missing parameters".to_string());
        }

        let param = &params_array[0];
        let pool_id = param
            .get("pool_id")
            .and_then(|v| v.as_u64())
            .ok_or("Missing pool_id")?;

        let dex = self.dex_state.read().await;
        match dex.get_pool(pool_id) {
            Ok(pool) => {
                debug!("DEX: Pool retrieved: {:?}", pool);
                Ok(json!({
                    "pool_id": pool.pool_id,
                    "token_a": pool.token_a,
                    "token_b": pool.token_b,
                    "reserve_a": pool.reserve_a.to_string(),
                    "reserve_b": pool.reserve_b.to_string(),
                    "total_shares": pool.total_shares.to_string(),
                    "fee_percentage": pool.fee_percentage,
                    "is_active": pool.is_active,
                    "created_at": pool.created_at,
                    "status": "success"
                }))
            }
            Err(e) => {
                warn!("DEX: Failed to get pool: {}", e);
                Err(format!("Failed to get pool: {}", e))
            }
        }
    }

    /// List all pools
    pub async fn list_pools(&self, _params: Value) -> Result<Value, String> {
        debug!("DEX: list_pools called");

        let dex = self.dex_state.read().await;
        let pools = dex.list_pools();

        let pools_json: Vec<Value> = pools
            .iter()
            .map(|pool| {
                json!({
                    "pool_id": pool.pool_id,
                    "token_a": pool.token_a,
                    "token_b": pool.token_b,
                    "reserve_a": pool.reserve_a.to_string(),
                    "reserve_b": pool.reserve_b.to_string(),
                    "total_shares": pool.total_shares.to_string(),
                    "fee_percentage": pool.fee_percentage,
                    "is_active": pool.is_active,
                })
            })
            .collect();

        info!("DEX: Listed {} pools", pools_json.len());
        Ok(json!({
            "pools": pools_json,
            "count": pools_json.len(),
            "status": "success"
        }))
    }

    /// Get DEX statistics
    pub async fn get_stats(&self, _params: Value) -> Result<Value, String> {
        debug!("DEX: get_stats called");

        let dex = self.dex_state.read().await;
        let stats = dex.get_stats();

        info!("DEX: Stats retrieved");
        Ok(json!({
            "total_pools": stats.total_pools,
            "total_positions": stats.total_positions,
            "total_swaps": stats.total_swaps,
            "total_volume": stats.total_volume.to_string(),
            "total_fees": stats.total_fees.to_string(),
            "is_paused": stats.is_paused,
            "status": "success"
        }))
    }
}
