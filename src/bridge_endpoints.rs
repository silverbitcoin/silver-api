//! Bridge RPC Endpoints - Production-ready JSON-RPC methods for bridge operations
//!
//! Provides JSON-RPC endpoints for:
//! - Validator registration and management
//! - Bridge transaction initiation
//! - Transaction confirmation by validators
//! - Transaction execution
//! - Bridge statistics and monitoring

use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use silver_execution::{BridgeState, BridgeTransactionStatus};

/// Bridge RPC endpoints
pub struct BridgeEndpoints {
    /// Bridge state manager
    bridge_state: Arc<RwLock<BridgeState>>,
}

impl BridgeEndpoints {
    /// Create new Bridge endpoints
    pub fn new(owner: String, fee_collector: String, min_stake: u128, fee_percentage: u64) -> Self {
        let bridge_state = BridgeState::new(owner, fee_collector, min_stake, fee_percentage);
        Self {
            bridge_state: Arc::new(RwLock::new(bridge_state)),
        }
    }

    /// Register a bridge validator
    pub async fn register_validator(&self, params: Value) -> Result<Value, String> {
        debug!("Bridge: register_validator called with params: {:?}", params);

        let params_array = params.as_array().ok_or("Invalid params format")?;
        if params_array.is_empty() {
            return Err("Missing parameters".to_string());
        }

        let param = &params_array[0];
        let validator = param
            .get("validator")
            .and_then(|v| v.as_str())
            .ok_or("Missing validator")?
            .to_string();
        let stake = param
            .get("stake")
            .and_then(|v| v.as_str())
            .ok_or("Missing stake")?
            .parse::<u128>()
            .map_err(|_| "Invalid stake")?;

        let mut bridge = self.bridge_state.write().await;
        match bridge.register_validator(validator.clone(), stake) {
            Ok(_) => {
                info!("Bridge: Validator registered: {}", validator);
                Ok(json!({
                    "validator": validator,
                    "stake": stake.to_string(),
                    "status": "registered"
                }))
            }
            Err(e) => {
                warn!("Bridge: Failed to register validator: {}", e);
                Err(format!("Failed to register validator: {}", e))
            }
        }
    }

    /// Unregister a bridge validator
    pub async fn unregister_validator(&self, params: Value) -> Result<Value, String> {
        debug!("Bridge: unregister_validator called with params: {:?}", params);

        let params_array = params.as_array().ok_or("Invalid params format")?;
        if params_array.is_empty() {
            return Err("Missing parameters".to_string());
        }

        let param = &params_array[0];
        let validator = param
            .get("validator")
            .and_then(|v| v.as_str())
            .ok_or("Missing validator")?
            .to_string();

        let mut bridge = self.bridge_state.write().await;
        match bridge.unregister_validator(validator.clone()) {
            Ok(_) => {
                info!("Bridge: Validator unregistered: {}", validator);
                Ok(json!({
                    "validator": validator,
                    "status": "unregistered"
                }))
            }
            Err(e) => {
                warn!("Bridge: Failed to unregister validator: {}", e);
                Err(format!("Failed to unregister validator: {}", e))
            }
        }
    }

    /// Initiate a bridge transaction
    pub async fn initiate(&self, params: Value) -> Result<Value, String> {
        debug!("Bridge: initiate called with params: {:?}", params);

        let params_array = params.as_array().ok_or("Invalid params format")?;
        if params_array.is_empty() {
            return Err("Missing parameters".to_string());
        }

        let param = &params_array[0];
        let source_chain = param
            .get("source_chain")
            .and_then(|v| v.as_str())
            .ok_or("Missing source_chain")?
            .to_string();
        let dest_chain = param
            .get("dest_chain")
            .and_then(|v| v.as_str())
            .ok_or("Missing dest_chain")?
            .to_string();
        let user = param
            .get("user")
            .and_then(|v| v.as_str())
            .ok_or("Missing user")?
            .to_string();
        let token = param
            .get("token")
            .and_then(|v| v.as_str())
            .ok_or("Missing token")?
            .to_string();
        let amount = param
            .get("amount")
            .and_then(|v| v.as_str())
            .ok_or("Missing amount")?
            .parse::<u128>()
            .map_err(|_| "Invalid amount")?;

        let mut bridge = self.bridge_state.write().await;
        match bridge.initiate_bridge_transaction(source_chain, dest_chain, user, token, amount) {
            Ok(tx_id) => {
                info!("Bridge: Transaction initiated with ID: {}", tx_id);
                Ok(json!({
                    "tx_id": tx_id,
                    "status": "pending"
                }))
            }
            Err(e) => {
                warn!("Bridge: Failed to initiate transaction: {}", e);
                Err(format!("Failed to initiate transaction: {}", e))
            }
        }
    }

    /// Confirm a bridge transaction
    pub async fn confirm(&self, params: Value) -> Result<Value, String> {
        debug!("Bridge: confirm called with params: {:?}", params);

        let params_array = params.as_array().ok_or("Invalid params format")?;
        if params_array.is_empty() {
            return Err("Missing parameters".to_string());
        }

        let param = &params_array[0];
        let tx_id = param
            .get("tx_id")
            .and_then(|v| v.as_u64())
            .ok_or("Missing tx_id")?;
        let validator = param
            .get("validator")
            .and_then(|v| v.as_str())
            .ok_or("Missing validator")?
            .to_string();

        let mut bridge = self.bridge_state.write().await;
        match bridge.confirm_bridge_transaction(tx_id, validator.clone()) {
            Ok(_) => {
                let tx = bridge.get_transaction(tx_id).ok();
                let confirmations = tx.as_ref().map(|t| t.confirmations).unwrap_or(0);
                info!(
                    "Bridge: Transaction {} confirmed by {}, confirmations: {}",
                    tx_id, validator, confirmations
                );
                Ok(json!({
                    "tx_id": tx_id,
                    "validator": validator,
                    "confirmations": confirmations,
                    "status": "confirmed"
                }))
            }
            Err(e) => {
                warn!("Bridge: Failed to confirm transaction: {}", e);
                Err(format!("Failed to confirm transaction: {}", e))
            }
        }
    }

    /// Execute a confirmed bridge transaction
    pub async fn execute(&self, params: Value) -> Result<Value, String> {
        debug!("Bridge: execute called with params: {:?}", params);

        let params_array = params.as_array().ok_or("Invalid params format")?;
        if params_array.is_empty() {
            return Err("Missing parameters".to_string());
        }

        let param = &params_array[0];
        let tx_id = param
            .get("tx_id")
            .and_then(|v| v.as_u64())
            .ok_or("Missing tx_id")?;

        let mut bridge = self.bridge_state.write().await;
        match bridge.execute_bridge_transaction(tx_id) {
            Ok(_) => {
                info!("Bridge: Transaction {} executed", tx_id);
                Ok(json!({
                    "tx_id": tx_id,
                    "status": "executed"
                }))
            }
            Err(e) => {
                warn!("Bridge: Failed to execute transaction: {}", e);
                Err(format!("Failed to execute transaction: {}", e))
            }
        }
    }

    /// Cancel a bridge transaction
    pub async fn cancel(&self, params: Value) -> Result<Value, String> {
        debug!("Bridge: cancel called with params: {:?}", params);

        let params_array = params.as_array().ok_or("Invalid params format")?;
        if params_array.is_empty() {
            return Err("Missing parameters".to_string());
        }

        let param = &params_array[0];
        let tx_id = param
            .get("tx_id")
            .and_then(|v| v.as_u64())
            .ok_or("Missing tx_id")?;

        let mut bridge = self.bridge_state.write().await;
        match bridge.cancel_bridge_transaction(tx_id) {
            Ok(_) => {
                info!("Bridge: Transaction {} cancelled", tx_id);
                Ok(json!({
                    "tx_id": tx_id,
                    "status": "cancelled"
                }))
            }
            Err(e) => {
                warn!("Bridge: Failed to cancel transaction: {}", e);
                Err(format!("Failed to cancel transaction: {}", e))
            }
        }
    }

    /// Get bridge transaction status
    pub async fn get_status(&self, params: Value) -> Result<Value, String> {
        debug!("Bridge: get_status called with params: {:?}", params);

        let params_array = params.as_array().ok_or("Invalid params format")?;
        if params_array.is_empty() {
            return Err("Missing parameters".to_string());
        }

        let param = &params_array[0];
        let tx_id = param
            .get("tx_id")
            .and_then(|v| v.as_u64())
            .ok_or("Missing tx_id")?;

        let bridge = self.bridge_state.read().await;
        match bridge.get_transaction(tx_id) {
            Ok(tx) => {
                let status_str = match tx.status {
                    BridgeTransactionStatus::Pending => "pending",
                    BridgeTransactionStatus::Confirmed => "confirmed",
                    BridgeTransactionStatus::Executed => "executed",
                    BridgeTransactionStatus::Failed => "failed",
                    BridgeTransactionStatus::Cancelled => "cancelled",
                };

                debug!("Bridge: Transaction {} status: {}", tx_id, status_str);
                Ok(json!({
                    "tx_id": tx_id,
                    "status": status_str,
                    "confirmations": tx.confirmations,
                    "amount": tx.amount.to_string(),
                    "fee": tx.fee.to_string(),
                    "source_chain": tx.source_chain,
                    "dest_chain": tx.dest_chain,
                }))
            }
            Err(e) => {
                warn!("Bridge: Failed to get transaction status: {}", e);
                Err(format!("Failed to get transaction status: {}", e))
            }
        }
    }

    /// Get validator information
    pub async fn get_validator(&self, params: Value) -> Result<Value, String> {
        debug!("Bridge: get_validator called with params: {:?}", params);

        let params_array = params.as_array().ok_or("Invalid params format")?;
        if params_array.is_empty() {
            return Err("Missing parameters".to_string());
        }

        let param = &params_array[0];
        let validator = param
            .get("validator")
            .and_then(|v| v.as_str())
            .ok_or("Missing validator")?
            .to_string();

        let bridge = self.bridge_state.read().await;
        match bridge.get_validator(validator) {
            Ok(val) => {
                debug!("Bridge: Validator info retrieved");
                Ok(json!({
                    "address": val.address,
                    "stake": val.stake.to_string(),
                    "is_active": val.is_active,
                    "confirmed_count": val.confirmed_count,
                    "failed_count": val.failed_count,
                    "is_slashed": val.is_slashed,
                    "status": "success"
                }))
            }
            Err(e) => {
                warn!("Bridge: Failed to get validator: {}", e);
                Err(format!("Failed to get validator: {}", e))
            }
        }
    }

    /// List all validators
    pub async fn list_validators(&self, _params: Value) -> Result<Value, String> {
        debug!("Bridge: list_validators called");

        let bridge = self.bridge_state.read().await;
        let validators = bridge.list_validators();

        let validators_json: Vec<Value> = validators
            .iter()
            .map(|val| {
                json!({
                    "address": val.address,
                    "stake": val.stake.to_string(),
                    "is_active": val.is_active,
                    "confirmed_count": val.confirmed_count,
                    "failed_count": val.failed_count,
                    "is_slashed": val.is_slashed,
                })
            })
            .collect();

        info!("Bridge: Listed {} validators", validators_json.len());
        Ok(json!({
            "validators": validators_json,
            "count": validators_json.len(),
            "status": "success"
        }))
    }

    /// Get bridge statistics
    pub async fn get_stats(&self, _params: Value) -> Result<Value, String> {
        debug!("Bridge: get_stats called");

        let bridge = self.bridge_state.read().await;
        let stats = bridge.get_stats();

        info!("Bridge: Stats retrieved");
        Ok(json!({
            "total_validators": stats.total_validators,
            "active_validators": stats.active_validators,
            "total_transactions": stats.total_transactions,
            "pending_transactions": stats.pending_transactions,
            "confirmed_transactions": stats.confirmed_transactions,
            "executed_transactions": stats.executed_transactions,
            "total_volume": stats.total_volume.to_string(),
            "total_fees": stats.total_fees.to_string(),
            "is_paused": stats.is_paused,
            "status": "success"
        }))
    }
}
