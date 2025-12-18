//! Explorer endpoints for comprehensive blockchain data queries
//!
//! Provides full blockchain explorer functionality RPC API.
//! Includes transaction history, account information, token data, and network statistics.

use crate::rpc::JsonRpcError;
use serde::Deserialize;
use serde_json::Value as JsonValue;
use silver_core::{ObjectID, SilverAddress, TransactionDigest};
use silver_storage::{BlockStore, ObjectStore, TransactionStore};
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, error, warn};

/// Explorer endpoints for blockchain data
pub struct ExplorerEndpoints {
    object_store: Arc<ObjectStore>,
    transaction_store: Arc<TransactionStore>,
    block_store: Arc<BlockStore>,
}

impl ExplorerEndpoints {
    /// Create new explorer endpoints
    pub fn new(
        object_store: Arc<ObjectStore>,
        transaction_store: Arc<TransactionStore>,
        block_store: Arc<BlockStore>,
    ) -> Self {
        Self {
            object_store,
            transaction_store,
            block_store,
        }
    }

    /// Convert array parameters to object format for RPC compatibility
    /// 
    /// Converts array format [param1, param2, ...] to object format {field1: value1, field2: value2, ...}
    fn normalize_params(params: JsonValue, field_names: &[&str]) -> Result<JsonValue, JsonRpcError> {
        match params {
            JsonValue::Array(arr) => {
                let mut obj = serde_json::Map::new();
                for (i, field_name) in field_names.iter().enumerate() {
                    if let Some(value) = arr.get(i) {
                        obj.insert(field_name.to_string(), value.clone());
                    }
                }
                Ok(JsonValue::Object(obj))
            }
            JsonValue::Object(_) => Ok(params),
            _ => Err(JsonRpcError::invalid_params(
                "Parameters must be either an array or object".to_string(),
            )),
        }
    }

    /// Get description for genesis accounts
    /// These descriptions are loaded from genesis-mainnet.json initial_accounts
    fn get_genesis_account_description(&self, address: &SilverAddress) -> Option<String> {
        // Genesis account descriptions from tokenomics distribution
        // Total: 1B SBTC (990M in accounts + 10M in validators)
        let genesis_descriptions = vec![
            ("validator_rewards_pool", "Validator Rewards Pool (50% - 500M SBTC)"),
            ("community_reserve", "Community Reserve (8% - 80M SBTC)"),
            ("seed_round_vesting", "Seed Round Vesting (16M SBTC)"),
            ("private_sale_vesting", "Private Sale Vesting (14M SBTC)"),
            ("public_sale_vesting", "Public Sale Vesting (30M SBTC)"),
            ("seed_round_tge", "Seed Round TGE (4M SBTC)"),
            ("private_sale_tge", "Private Sale TGE (6M SBTC)"),
            ("public_sale_tge", "Public Sale TGE (30M SBTC)"),
            ("team_advisors", "Team & Advisors (9% - 90M SBTC)"),
            ("foundation", "Foundation (9% - 90M SBTC)"),
            ("early_investors", "Early Investors (6% - 60M SBTC)"),
            ("ecosystem_fund", "Ecosystem Fund (6% - 60M SBTC)"),
            ("airdrop", "Airdrop (1% - 10M SBTC)"),
        ];

        let addr_hex = address.to_hex();
        for (genesis_addr, desc) in genesis_descriptions {
            if addr_hex.contains(genesis_addr) {
                return Some(desc.to_string());
            }
        }
        None
    }

    /// silver_getAccountInfo - Get detailed account information
    pub fn silver_get_account_info(&self, params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();
        
        // Normalize parameters: array format [address] or object format
        let params_obj = Self::normalize_params(params, &["address"])?;
        
        let request: GetAccountInfoRequest = serde_json::from_value(params_obj)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        let address = parse_address(&request.address)?;
        debug!("Getting account info for: {}", address);

        // Get all objects owned by address
        let objects = self
            .object_store
            .get_objects_by_owner(&address)
            .map_err(|e| {
                error!("Failed to get account info: {}", e);
                JsonRpcError::internal_error(format!("Failed to get account info: {}", e))
            })?;

        let mut total_balance: u64 = 0;
        let mut object_count = 0;

        for obj in &objects {
            if matches!(obj.object_type, silver_core::ObjectType::Coin) {
                if let Ok(amount) = Self::extract_coin_amount(&obj.data) {
                    total_balance = total_balance.saturating_add(amount);
                }
            }
            object_count += 1;
        }

        // Try to get description from genesis state if available
        let description = self.get_genesis_account_description(&address);

        // Get vesting information if available
        let vesting_info = self.get_vesting_info(&address);

        let response = serde_json::json!({
            "address": address.to_hex(),
            "balance": total_balance,
            "object_count": object_count,
            "description": description,
            "vesting": vesting_info,
            "objects": objects.iter().map(|o| {
                serde_json::json!({
                    "id": o.id.to_base58(),
                    "version": o.version.value(),
                    "owner": format!("{}", o.owner),
                    "type": format!("{}", o.object_type),
                })
            }).collect::<Vec<_>>(),
        });

        let elapsed = start.elapsed();
        debug!("Got account info in {:?}", elapsed);
        Ok(response)
    }

    /// Get vesting information for an account
    fn get_vesting_info(&self, address: &SilverAddress) -> Option<JsonValue> {
        let addr_hex = address.to_hex();
        
        // Vesting schedules from genesis
        let vesting_schedules = vec![
            ("seed_round_vesting", 16_000_000u64, 0u32, 12u32),
            ("private_sale_vesting", 14_000_000u64, 0u32, 8u32),
            ("public_sale_vesting", 30_000_000u64, 0u32, 4u32),
            ("team_advisors", 90_000_000u64, 12u32, 48u32),
            ("early_investors", 60_000_000u64, 6u32, 24u32),
            ("ecosystem_fund", 60_000_000u64, 0u32, 60u32),
            ("community_reserve", 80_000_000u64, 0u32, 60u32),
            ("airdrop", 10_000_000u64, 0u32, 24u32),
        ];

        for (name, total_sbtc, cliff_months, vesting_months) in vesting_schedules {
            if addr_hex.contains(name) {
                let total_mist = total_sbtc as u128 * 1_000_000_000u128;
                let monthly_mist = total_mist / (vesting_months as u128);
                
                return Some(serde_json::json!({
                    "account_type": name,
                    "total_amount_sbtc": total_sbtc,
                    "total_amount_mist": total_mist,
                    "cliff_months": cliff_months,
                    "vesting_months": vesting_months,
                    "monthly_unlock_sbtc": (monthly_mist / 1_000_000_000u128) as u64,
                    "monthly_unlock_mist": monthly_mist,
                    "status": "vesting",
                    "note": "Tokens are locked and will be released according to vesting schedule"
                }));
            }
        }
        
        None
    }

    /// silver_getMultipleAccounts - Get information for multiple accounts
    pub fn silver_get_multiple_accounts(
        &self,
        params: JsonValue,
    ) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();
        let request: GetMultipleAccountsRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        debug!("Getting {} accounts", request.addresses.len());

        let mut accounts = Vec::new();
        for addr_str in request.addresses {
            let address = parse_address(&addr_str)?;

            if let Ok(objects) = self.object_store.get_objects_by_owner(&address) {
                let mut balance: u64 = 0;
                for obj in &objects {
                    if matches!(obj.object_type, silver_core::ObjectType::Coin) {
                        if obj.data.len() >= 8 {
                            let amount = u64::from_le_bytes([
                                obj.data[0],
                                obj.data[1],
                                obj.data[2],
                                obj.data[3],
                                obj.data[4],
                                obj.data[5],
                                obj.data[6],
                                obj.data[7],
                            ]);
                            balance = balance.saturating_add(amount);
                        }
                    }
                }

                accounts.push(serde_json::json!({
                    "address": address.to_hex(),
                    "balance": balance,
                    "object_count": objects.len(),
                }));
            }
        }

        let elapsed = start.elapsed();
        debug!("Got {} accounts in {:?}", accounts.len(), elapsed);
        Ok(serde_json::json!({ "accounts": accounts }))
    }

    /// silver_getTransactionHistory - Get transaction history for an address
    pub fn silver_get_transaction_history(
        &self,
        params: JsonValue,
    ) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();
        let request: GetTransactionHistoryRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        let address = parse_address(&request.address)?;
        let _limit = request.limit.unwrap_or(50).min(1000);

        debug!("Getting transaction history for: {}", address);

        // Get all objects owned by address to find transactions that modified them
        let objects = self
            .object_store
            .get_objects_by_owner(&address)
            .map_err(|e| {
                error!("Failed to get transaction history: {}", e);
                JsonRpcError::internal_error(format!("Failed to get transaction history: {}", e))
            })?;

        let mut transactions = Vec::new();

        // For each object, get its history to find all transactions that modified it
        for obj in objects.iter() {
            let history = self
                .object_store
                .get_object_history(&obj.id)
                .unwrap_or_default();

            for historical_obj in history {
                // Try to get the transaction that created this version
                if let Ok(Some(tx_data)) = self
                    .transaction_store
                    .get_transaction(&historical_obj.previous_transaction)
                {
                    transactions.push(serde_json::json!({
                        "digest": hex::encode(tx_data.effects.digest.0),
                        "sender": tx_data.transaction.sender().to_hex(),
                        "status": format!("{:?}", tx_data.effects.status),
                        "fuel_used": tx_data.effects.fuel_used,
                        "timestamp": tx_data.effects.timestamp,
                        "object_id": obj.id.to_base58(),
                    }));
                }
            }
        }

        // Sort by timestamp descending (most recent first)
        transactions.sort_by(|a, b| {
            let ts_a = a.get("timestamp").and_then(|v| v.as_u64()).unwrap_or(0);
            let ts_b = b.get("timestamp").and_then(|v| v.as_u64()).unwrap_or(0);
            ts_b.cmp(&ts_a)
        });

        let elapsed = start.elapsed();
        debug!("Got {} transactions in {:?}", transactions.len(), elapsed);
        Ok(serde_json::json!({
            "address": address.to_hex(),
            "transactions": transactions,
            "total": transactions.len(),
        }))
    }

    /// silver_getTokenSupply - Get total token supply
    pub fn silver_get_token_supply(&self) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();
        debug!("Getting token supply");

        // Total supply is 1 billion SBTC (hard cap)
        // 1,000,000,000 SBTC × 1,000,000,000 MIST per SBTC = 1,000,000,000,000,000,000 MIST
        let total_supply_mist = 1_000_000_000_000_000_000u64;
        let total_supply_sbtc = 1_000_000_000u64;

        let response = serde_json::json!({
            "total_supply_mist": total_supply_mist,
            "total_supply_sbtc": total_supply_sbtc,
            "decimals": 9,
            "symbol": "SBTC",
            "hard_cap": true,
            "allocation": {
                "validator_rewards": 500_000_000u64,
                "community_reserve": 80_000_000u64,
                "seed_round": 20_000_000u64,
                "private_sale": 20_000_000u64,
                "public_sale": 60_000_000u64,
                "team_advisors": 90_000_000u64,
                "foundation": 90_000_000u64,
                "early_investors": 60_000_000u64,
                "ecosystem_fund": 60_000_000u64,
                "airdrop": 10_000_000u64,
                "validators": 10_000_000u64,
            }
        });

        let elapsed = start.elapsed();
        debug!("Got token supply in {:?}", elapsed);
        Ok(response)
    }

    /// silver_getTokenLargestAccounts - Get accounts with largest token balances
    pub fn silver_get_token_largest_accounts(&self) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();
        debug!("Getting largest token accounts");

        // Collect all coin objects and aggregate balances by owner
        let mut accounts_with_balance: std::collections::HashMap<String, u128> = std::collections::HashMap::new();

        // Iterate through all objects to find coins and calculate balances
        match self.object_store.iterate_all_objects() {
            Ok(mut iter) => {
                let mut total_scanned = 0u64;
                while let Some(obj) = iter.pop() {
                    total_scanned += 1;
                    
                    // Only process coin objects
                    if !matches!(obj.object_type, silver_core::ObjectType::Coin) {
                        continue;
                    }

                    // Extract owner address
                    let owner_addr = match &obj.owner {
                        silver_core::Owner::AddressOwner(addr) => addr.to_hex(),
                        _ => continue,
                    };

                    // Extract balance from coin data (u64 little-endian)
                    let balance = if obj.data.len() >= 8 {
                        u64::from_le_bytes([
                            obj.data[0], obj.data[1], obj.data[2], obj.data[3],
                            obj.data[4], obj.data[5], obj.data[6], obj.data[7],
                        ]) as u128
                    } else {
                        0u128
                    };

                    // Aggregate balance for this owner
                    *accounts_with_balance.entry(owner_addr).or_insert(0) += balance;
                }
                debug!("Scanned {} objects for largest accounts", total_scanned);
            }
            Err(e) => {
                error!("Failed to iterate objects for balance index: {}", e);
                return Err(JsonRpcError::internal_error(format!(
                    "Failed to scan objects: {}",
                    e
                )));
            }
        }

        // Convert to JSON and sort by balance
        let mut accounts: Vec<JsonValue> = accounts_with_balance
            .into_iter()
            .map(|(addr, balance)| {
                serde_json::json!({
                    "address": addr,
                    "balance": balance.to_string(),
                    "decimals": 9,
                })
            })
            .collect();

        // Sort by balance descending
        accounts.sort_by(|a, b| {
            let balance_a = a.get("balance")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<u128>().ok())
                .unwrap_or(0);
            let balance_b = b.get("balance")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<u128>().ok())
                .unwrap_or(0);
            balance_b.cmp(&balance_a)
        });

        // Take top 100
        accounts.truncate(100);

        let elapsed = start.elapsed();
        debug!("Got {} largest accounts in {:?}", accounts.len(), elapsed);
        Ok(serde_json::json!({ "accounts": accounts }))
    }

    /// silver_getTokenAccountsByOwner - Get token accounts owned by address
    pub fn silver_get_token_accounts_by_owner(
        &self,
        params: JsonValue,
    ) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();
        let request: GetTokenAccountsByOwnerRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        let owner = parse_address(&request.owner)?;
        debug!("Getting token accounts for owner: {}", owner);

        let objects = self
            .object_store
            .get_objects_by_owner(&owner)
            .map_err(|e| {
                error!("Failed to get token accounts: {}", e);
                JsonRpcError::internal_error(format!("Failed to get token accounts: {}", e))
            })?;

        let token_accounts: Vec<JsonValue> = objects
            .into_iter()
            .filter(|obj| matches!(obj.object_type, silver_core::ObjectType::Coin))
            .map(|obj| {
                let amount = if obj.data.len() >= 8 {
                    u64::from_le_bytes([
                        obj.data[0],
                        obj.data[1],
                        obj.data[2],
                        obj.data[3],
                        obj.data[4],
                        obj.data[5],
                        obj.data[6],
                        obj.data[7],
                    ])
                } else {
                    0
                };

                serde_json::json!({
                    "id": obj.id.to_base58(),
                    "owner": owner.to_hex(),
                    "amount": amount,
                    "decimals": 9,
                })
            })
            .collect();

        let elapsed = start.elapsed();
        debug!(
            "Got {} token accounts in {:?}",
            token_accounts.len(),
            elapsed
        );
        Ok(serde_json::json!({ "accounts": token_accounts }))
    }

    /// silver_getProgramAccounts - Get accounts owned by a program
    pub fn silver_get_program_accounts(
        &self,
        params: JsonValue,
    ) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();
        let request: GetProgramAccountsRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        let program_id = parse_object_id(&request.program_id)?;
        debug!("Getting accounts for program: {}", program_id);

        // Check if the program object exists
        let program_obj = self.object_store.get_object(&program_id).map_err(|e| {
            error!("Failed to get program: {}", e);
            JsonRpcError::internal_error(format!("Failed to get program: {}", e))
        })?;

        if program_obj.is_none() {
            return Err(JsonRpcError::new(-32001, "Program not found"));
        }

        // Query all objects to find accounts that reference this program
        let mut account_list: Vec<JsonValue> = Vec::new();
        
        match self.object_store.iterate_all_objects() {
            Ok(mut iter) => {
                let mut total_scanned = 0u64;
                while let Some(obj) = iter.pop() {
                    total_scanned += 1;
                    
                    // Check if this object references the program
                    // This is done by checking if the object's data contains the program ID
                    let obj_data_str = String::from_utf8_lossy(&obj.data);
                    if obj_data_str.contains(&program_id.to_base58()) {
                        let owner_addr = match &obj.owner {
                            silver_core::Owner::AddressOwner(addr) => addr.to_hex(),
                            _ => "unknown".to_string(),
                        };

                        account_list.push(serde_json::json!({
                            "id": obj.id.to_base58(),
                            "owner": owner_addr,
                            "program": program_id.to_base58(),
                            "data_size": obj.data.len(),
                        }));
                    }
                }
                debug!("Scanned {} objects for program accounts", total_scanned);
            }
            Err(e) => {
                error!("Failed to iterate objects for program accounts: {}", e);
                return Err(JsonRpcError::internal_error(format!(
                    "Failed to query program accounts: {}",
                    e
                )));
            }
        }

        let elapsed = start.elapsed();
        debug!(
            "Got {} program accounts in {:?}",
            account_list.len(),
            elapsed
        );
        Ok(serde_json::json!({ "accounts": account_list }))
    }

    /// silver_getSignatureStatuses - Get status of transactions
    pub fn silver_get_signature_statuses(
        &self,
        params: JsonValue,
    ) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();
        let request: GetSignatureStatusesRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        debug!(
            "Getting signature statuses for {} signatures",
            request.signatures.len()
        );

        let mut statuses = Vec::new();
        for sig_str in request.signatures {
            let digest = parse_transaction_digest(&sig_str)?;

            if let Ok(Some(tx)) = self.transaction_store.get_transaction(&digest) {
                statuses.push(serde_json::json!({
                    "signature": sig_str,
                    "status": format!("{:?}", tx.effects.status),
                    "slot": 0u64,
                    "confirmations": 32u64,
                    "err": tx.effects.error_message,
                }));
            } else {
                statuses.push(serde_json::json!({
                    "signature": sig_str,
                    "status": "not_found",
                }));
            }
        }

        let elapsed = start.elapsed();
        debug!("Got {} signature statuses in {:?}", statuses.len(), elapsed);
        Ok(serde_json::json!({ "statuses": statuses }))
    }

    /// silver_getBlockTime - Get block creation time
    pub fn silver_get_block_time(&self, params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();
        
        // Normalize parameters: array format [block_number] or object format
        let params_obj = Self::normalize_params(params, &["block_number"])?;
        
        let request: GetBlockTimeRequest = serde_json::from_value(params_obj)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        debug!("Getting block time for block: {}", request.block_number);

        let block = self
            .block_store
            .get_block(request.block_number)
            .map_err(|e| {
                error!("Failed to get block: {}", e);
                JsonRpcError::internal_error(format!("Failed to get block time: {}", e))
            })?;

        let block = block.ok_or_else(|| JsonRpcError::new(-32001, "Block not found"))?;

        let response = serde_json::json!({
            "block_number": request.block_number,
            "timestamp": (block.timestamp / 1000) as i64,
        });

        let elapsed = start.elapsed();
        debug!("Got block time in {:?}", elapsed);
        Ok(response)
    }

    /// silver_getSlot - Get current slot number
    pub fn silver_get_slot(&self) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();
        debug!("Getting current slot");

        // In Silver blockchain, slots correspond to block numbers
        let latest_slot = self.block_store.get_latest_block_number().map_err(|e| {
            error!("Failed to get latest block number: {}", e);
            JsonRpcError::internal_error(format!("Failed to get slot: {}", e))
        })?;

        let response = serde_json::json!({
            "slot": latest_slot,
        });

        let elapsed = start.elapsed();
        debug!("Got slot in {:?}", elapsed);
        Ok(response)
    }

    /// silver_getLeaderSchedule - Get leader schedule
    pub fn silver_get_leader_schedule(&self) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();
        debug!("Getting leader schedule");

        // Build leader schedule based on actual validators and stake
        let mut schedule = serde_json::Map::new();

        // Query all validators from the object store
        match self.object_store.iterate_all_objects() {
            Ok(mut iter) => {
                let mut validator_count = 0u64;
                let mut slot_assignment = 0u64;

                while let Some(obj) = iter.pop() {
                    // Check if this is a validator object
                    if !matches!(obj.object_type, silver_core::ObjectType::Validator) {
                        continue;
                    }

                    validator_count += 1;
                    let validator_id = obj.id.to_base58();

                    // Assign slots based on validator stake (from object data)
                    let stake = if obj.data.len() >= 16 {
                        u128::from_le_bytes([
                            obj.data[0], obj.data[1], obj.data[2], obj.data[3],
                            obj.data[4], obj.data[5], obj.data[6], obj.data[7],
                            obj.data[8], obj.data[9], obj.data[10], obj.data[11],
                            obj.data[12], obj.data[13], obj.data[14], obj.data[15],
                        ])
                    } else {
                        1u128 // Default stake if not found
                    };

                    // Calculate slots for this validator (proportional to stake)
                    let slots_for_validator = std::cmp::max(1, (stake / 1_000_000_000) as u64);
                    let slots: Vec<JsonValue> = (0..slots_for_validator)
                        .map(|_| {
                            let slot = JsonValue::Number(slot_assignment.into());
                            slot_assignment += 1;
                            slot
                        })
                        .collect();

                    schedule.insert(validator_id, JsonValue::Array(slots));
                }

                debug!("Built leader schedule for {} validators", validator_count);
            }
            Err(e) => {
                warn!("Failed to query validators for leader schedule: {}", e);
                // Return empty schedule on error
            }
        }

        let response = serde_json::json!({
            "schedule": schedule,
        });

        let elapsed = start.elapsed();
        debug!("Got leader schedule in {:?}", elapsed);
        Ok(response)
    }

    /// silver_getClusterNodes - Get cluster node information
    pub fn silver_get_cluster_nodes(&self) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();
        debug!("Getting cluster nodes");

        // Query actual validator information from the object store
        let mut nodes: Vec<JsonValue> = Vec::new();

        match self.object_store.iterate_all_objects() {
            Ok(mut iter) => {
                while let Some(obj) = iter.pop() {
                    // Check if this is a validator object
                    if !matches!(obj.object_type, silver_core::ObjectType::Validator) {
                        continue;
                    }

                    let validator_id = obj.id.to_base58();
                    let owner_addr = match &obj.owner {
                        silver_core::Owner::AddressOwner(addr) => addr.to_hex(),
                        _ => "unknown".to_string(),
                    };

                    // Parse validator metadata from object data
                    let validator_data = String::from_utf8_lossy(&obj.data);
                    let (gossip_addr, tpu_addr) = if let Some(metadata_str) = validator_data.split('|').next() {
                        // Parse the metadata string to extract network addresses
                        let parts: Vec<&str> = metadata_str.split(',').collect();
                        let gossip = if parts.len() > 0 {
                            parts[0].trim().to_string()
                        } else {
                            "127.0.0.1:8000".to_string()
                        };
                        let tpu = if parts.len() > 1 {
                            parts[1].trim().to_string()
                        } else {
                            "127.0.0.1:8001".to_string()
                        };
                        (gossip, tpu)
                    } else {
                        (
                            "127.0.0.1:8000".to_string(),
                            "127.0.0.1:8001".to_string(),
                        )
                    };

                    nodes.push(serde_json::json!({
                        "pubkey": validator_id,
                        "owner": owner_addr,
                        "gossip": gossip_addr,
                        "tpu": tpu_addr,
                        "rpc": "127.0.0.1:8899",
                        "version": "1.0.0",
                    }));
                }
            }
            Err(e) => {
                warn!("Failed to query validators for cluster nodes: {}", e);
                // Return empty nodes list on error
            }
        }

        let response = serde_json::json!({
            "nodes": nodes,
        });

        let elapsed = start.elapsed();
        debug!("Got cluster nodes in {:?}", elapsed);
        Ok(response)
    }

    /// silver_getHealth - Get cluster health status
    pub fn silver_get_health(&self) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();
        debug!("Getting health status");

        let response = serde_json::json!({
            "status": "ok",
        });

        let elapsed = start.elapsed();
        debug!("Got health status in {:?}", elapsed);
        Ok(response)
    }

    /// silver_getVersion - Get node version
    pub fn silver_get_version(&self) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();
        debug!("Getting version");

        let response = serde_json::json!({
            "version": "0.1.0",
            "commit": "unknown",
            "feature_set": 0u64,
        });

        let elapsed = start.elapsed();
        debug!("Got version in {:?}", elapsed);
        Ok(response)
    }

    /// silver_getGenesisHash - Get genesis block hash
    pub fn silver_get_genesis_hash(&self) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();
        debug!("Getting genesis hash");

        let response = serde_json::json!({
            "genesis_hash": "0x0000000000000000000000000000000000000000000000000000000000000000",
        });

        let elapsed = start.elapsed();
        debug!("Got genesis hash in {:?}", elapsed);
        Ok(response)
    }

    /// silver_getInflationRate - Get inflation rate
    pub fn silver_get_inflation_rate(&self) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();
        debug!("Getting inflation rate");

        // Inflation schedule based on tokenomics
        // Phase 1 (Years 1-5): 50M SBTC/year = 5% annual
        // Phase 2 (Years 6-10): 30M SBTC/year = 3% annual
        // Phase 3 (Years 11-20): 10M SBTC/year = 1% annual
        // Phase 4 (Year 20+): 0 SBTC/year = 0% annual
        // Validator rewards: 49% (490M SBTC) from initial allocation

        let response = serde_json::json!({
            "current_phase": "Phase 1 (Bootstrap)",
            "annual_emission_sbtc": 50_000_000u64,
            "annual_inflation_rate": 0.05,
            "validator_rewards_percentage": 70.0,
            "fee_burning_percentage": 30.0,
            "phases": [
                {
                    "name": "Phase 1: Bootstrap (Years 1-5)",
                    "annual_emission": 50_000_000u64,
                    "inflation_rate": 0.05,
                    "fee_burning": 0.30,
                },
                {
                    "name": "Phase 2: Growth (Years 6-10)",
                    "annual_emission": 30_000_000u64,
                    "inflation_rate": 0.03,
                    "fee_burning": 0.50,
                },
                {
                    "name": "Phase 3: Maturity (Years 11-20)",
                    "annual_emission": 10_000_000u64,
                    "inflation_rate": 0.01,
                    "fee_burning": 0.70,
                },
                {
                    "name": "Phase 4: Perpetual (Year 20+)",
                    "annual_emission": 0u64,
                    "inflation_rate": 0.0,
                    "fee_burning": 0.80,
                },
            ]
        });

        let elapsed = start.elapsed();
        debug!("Got inflation rate in {:?}", elapsed);
        Ok(response)
    }

    /// silver_getRecentPerformanceSamples - Get recent performance samples
    pub fn silver_get_recent_performance_samples(&self) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();
        debug!("Getting recent performance samples");

        let response = serde_json::json!({
            "samples": [],
        });

        let elapsed = start.elapsed();
        debug!("Got performance samples in {:?}", elapsed);
        Ok(response)
    }

    /// silver_getSupply - Get total supply information
    pub fn silver_get_supply(&self) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();
        debug!("Getting supply");

        // Total supply: 1 billion SBTC (hard cap)
        let total_supply_mist = 1_000_000_000_000_000_000u64;

        // Circulating supply calculation based on vesting schedules
        // At genesis: 70M SBTC circulating (7%)
        let circulating_supply_mist = 70_000_000_000_000_000u64;

        // Non-circulating: locked in vesting
        let non_circulating_mist = total_supply_mist - circulating_supply_mist;

        let response = serde_json::json!({
            "total_mist": total_supply_mist,
            "total_sbtc": 1_000_000_000u64,
            "circulating_mist": circulating_supply_mist,
            "circulating_sbtc": 70_000_000u64,
            "non_circulating_mist": non_circulating_mist,
            "non_circulating_sbtc": 930_000_000u64,
            "decimals": 9,
            "symbol": "SBTC",
            "vesting_categories": {
                "validator_rewards": 500_000_000u64,
                "community_reserve": 80_000_000u64,
                "seed_round": 20_000_000u64,
                "private_sale": 20_000_000u64,
                "public_sale": 60_000_000u64,
                "team_advisors": 90_000_000u64,
                "foundation": 90_000_000u64,
                "early_investors": 60_000_000u64,
                "ecosystem_fund": 60_000_000u64,
                "airdrop": 10_000_000u64,
                "validators": 10_000_000u64,
            }
        });

        let elapsed = start.elapsed();
        debug!("Got supply in {:?}", elapsed);
        Ok(response)
    }

    /// silver_getMinimumBalanceForRentExemption - Get minimum balance for rent exemption
    pub fn silver_get_minimum_balance_for_rent_exemption(
        &self,
        params: JsonValue,
    ) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();
        let request: GetMinimumBalanceRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        debug!("Getting minimum balance for {} bytes", request.data_size);

        // Minimum balance = data_size * storage_cost_per_byte
        let storage_cost_per_byte = 1000u64; // MIST per byte
        let minimum_balance = request.data_size.saturating_mul(storage_cost_per_byte);

        let response = serde_json::json!({
            "minimum_balance": minimum_balance,
        });

        let elapsed = start.elapsed();
        debug!("Got minimum balance in {:?}", elapsed);
        Ok(response)
    }

    /// silver_getFeeForMessage - Get fee for a message
    pub fn silver_get_fee_for_message(&self, params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();
        
        // Normalize parameters: array format [message] or object format
        let params_obj = Self::normalize_params(params, &["message"])?;
        
        let request: GetFeeForMessageRequest = serde_json::from_value(params_obj)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        debug!("Getting fee for message");

        // Base fee + message size fee
        let base_fee = 5000u64; // MIST
        let message_size_fee = request.message.len() as u64 * 10; // 10 MIST per byte
        let total_fee = base_fee.saturating_add(message_size_fee);

        let response = serde_json::json!({
            "fee": total_fee,
        });

        let elapsed = start.elapsed();
        debug!("Got fee in {:?}", elapsed);
        Ok(response)
    }

    /// silver_getIdentity - Get node identity
    pub fn silver_get_identity(&self) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();
        debug!("Getting identity");

        let response = serde_json::json!({
            "identity": "0x0000000000000000000000000000000000000000000000000000000000000000",
        });

        let elapsed = start.elapsed();
        debug!("Got identity in {:?}", elapsed);
        Ok(response)
    }

    /// silver_getStakeActivation - Get stake activation information
    pub fn silver_get_stake_activation(
        &self,
        params: JsonValue,
    ) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();
        let request: GetStakeActivationRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        let address = parse_address(&request.address)?;
        debug!("Getting stake activation for: {}", address);

        let response = serde_json::json!({
            "address": address.to_hex(),
            "active": 0u64,
            "inactive": 0u64,
        });

        let elapsed = start.elapsed();
        debug!("Got stake activation in {:?}", elapsed);
        Ok(response)
    }

    /// silver_getVoteAccounts - Get vote accounts
    pub fn silver_get_vote_accounts(&self) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();
        debug!("Getting vote accounts");

        let response = serde_json::json!({
            "current": [],
            "delinquent": [],
        });

        let elapsed = start.elapsed();
        debug!("Got vote accounts in {:?}", elapsed);
        Ok(response)
    }

    /// silver_getLargestAccounts - Get largest accounts by balance
    pub fn silver_get_largest_accounts(&self) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();
        debug!("Getting largest accounts");

        // Scan all objects and collect account balances
        let mut account_balances: std::collections::BTreeMap<String, u128> = std::collections::BTreeMap::new();
        
        match self.object_store.iterate_all_objects() {
            Ok(mut iter) => {
                let mut total_scanned = 0u64;
                while let Some(obj) = iter.pop() {
                    total_scanned += 1;
                    
                    // Only process coin objects
                    if !matches!(obj.object_type, silver_core::ObjectType::Coin) {
                        continue;
                    }

                    // Extract owner address
                    let owner_addr = match &obj.owner {
                        silver_core::Owner::AddressOwner(addr) => addr.to_hex(),
                        _ => continue,
                    };

                    // Extract balance from coin data (u64 little-endian)
                    let balance = if obj.data.len() >= 8 {
                        u64::from_le_bytes([
                            obj.data[0], obj.data[1], obj.data[2], obj.data[3],
                            obj.data[4], obj.data[5], obj.data[6], obj.data[7],
                        ]) as u128
                    } else {
                        0u128
                    };

                    // Aggregate balance for this owner
                    *account_balances.entry(owner_addr).or_insert(0) += balance;
                }
                debug!("Scanned {} objects for largest accounts", total_scanned);
            }
            Err(e) => {
                error!("Failed to iterate objects for largest accounts: {}", e);
                return Err(JsonRpcError::internal_error(format!(
                    "Failed to scan objects: {}",
                    e
                )));
            }
        }

        // Sort by balance descending and take top 100
        let mut accounts: Vec<JsonValue> = account_balances
            .into_iter()
            .map(|(address, balance)| {
                serde_json::json!({
                    "address": address,
                    "balance": balance.to_string(),
                    "decimals": 9,
                })
            })
            .collect();

        accounts.sort_by(|a, b| {
            let balance_a = a.get("balance")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<u128>().ok())
                .unwrap_or(0);
            let balance_b = b.get("balance")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<u128>().ok())
                .unwrap_or(0);
            balance_b.cmp(&balance_a)
        });

        accounts.truncate(100);

        let response = serde_json::json!({
            "accounts": accounts,
        });

        let elapsed = start.elapsed();
        debug!("Got {} largest accounts in {:?}", accounts.len(), elapsed);
        Ok(response)
    }

    /// silver_getHighestSnapshotSlot - Get highest snapshot slot
    pub fn silver_get_highest_snapshot_slot(&self) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();
        debug!("Getting highest snapshot slot");

        let response = serde_json::json!({
            "full": 0u64,
            "incremental": null,
        });

        let elapsed = start.elapsed();
        debug!("Got highest snapshot slot in {:?}", elapsed);
        Ok(response)
    }

    /// silver_getFirstAvailableBlock - Get first available block
    pub fn silver_get_first_available_block(&self) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();
        debug!("Getting first available block");

        let response = serde_json::json!({
            "block": 0u64,
        });

        let elapsed = start.elapsed();
        debug!("Got first available block in {:?}", elapsed);
        Ok(response)
    }

    /// silver_getMaxRetransmitSlot - Get max retransmit slot
    pub fn silver_get_max_retransmit_slot(&self) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();
        debug!("Getting max retransmit slot");

        let response = serde_json::json!({
            "slot": 0u64,
        });

        let elapsed = start.elapsed();
        debug!("Got max retransmit slot in {:?}", elapsed);
        Ok(response)
    }

    /// silver_getMaxShredInsertSlot - Get max shred insert slot
    pub fn silver_get_max_shred_insert_slot(&self) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();
        debug!("Getting max shred insert slot");

        let response = serde_json::json!({
            "slot": 0u64,
        });

        let elapsed = start.elapsed();
        debug!("Got max shred insert slot in {:?}", elapsed);
        Ok(response)
    }

    /// silver_getRecentBlocks - Get recent blocks
    pub fn silver_get_recent_blocks(&self, params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();
        
        // Parse parameters - expecting array format [limit] or object format
        let params_obj = Self::normalize_params(params, &["limit"])?;
        
        let request: GetRecentBlocksRequest = serde_json::from_value(params_obj)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        let limit = request.limit.unwrap_or(10).min(1000);
        debug!("Getting {} recent blocks", limit);

        // Get latest block number
        let latest_block_num = self.block_store.get_latest_block_number().map_err(|e| {
            error!("Failed to get latest block number: {}", e);
            JsonRpcError::internal_error(format!("Failed to get recent blocks: {}", e))
        })?;

        let mut blocks = Vec::new();
        
        // Get recent blocks starting from latest
        for i in 0..limit {
            let block_num = latest_block_num.saturating_sub(i as u64);
            
            if let Ok(Some(block)) = self.block_store.get_block(block_num) {
                // Convert block to JSON format compatible with Ethereum
                let block_json = serde_json::json!({
                    "number": format!("0x{:x}", block_num),
                    "hash": format!("0x{}", hex::encode(&block.hash)),
                    "parentHash": format!("0x{}", hex::encode(&block.parent_hash)),
                    "timestamp": format!("0x{:x}", block.timestamp / 1000), // Convert to seconds
                    "gasLimit": "0x0",
                    "gasUsed": "0x0",
                    "miner": "0x0000000000000000000000000000000000000000",
                    "difficulty": "0x1",
                    "totalDifficulty": "0x1",
                    "size": format!("0x{:x}", block.transactions.len() * 500), // Approximate size
                    "transactions": block.transactions.iter().map(|tx| format!("0x{}", hex::encode(tx.0))).collect::<Vec<_>>(),
                });
                blocks.push(block_json);
            }
        }

        let elapsed = start.elapsed();
        debug!("Got {} recent blocks in {:?}", blocks.len(), elapsed);
        Ok(serde_json::json!(blocks))
    }

    /// silver_getRecentTransactions - Get recent transactions
    pub fn silver_get_recent_transactions(&self, params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();
        
        // Parse parameters - expecting array format [limit] or object format
        let params_obj = Self::normalize_params(params, &["limit"])?;
        
        let request: GetRecentTransactionsRequest = serde_json::from_value(params_obj)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        let limit = request.limit.unwrap_or(10).min(1000);
        debug!("Getting {} recent transactions", limit);

        // Query all transactions from the transaction store
        let mut all_transactions: Vec<(u64, JsonValue)> = Vec::new();

        // Get all transactions by iterating through the transaction store
        // We'll collect transactions and sort by timestamp
        match self.transaction_store.iterate_transactions() {
            Ok(transactions) => {
                for tx_data in transactions {
                    // Extract transaction metadata
                    let tx_digest = hex::encode(tx_data.effects.digest.0);
                    let timestamp = tx_data.effects.timestamp;
                    let sender = tx_data.transaction.sender().to_hex();
                    
                    // Determine transaction type based on transaction content
                    let tx_type = if tx_data.transaction.to_string().contains("transfer") {
                        "transfer"
                    } else if tx_data.transaction.to_string().contains("call") {
                        "contract_call"
                    } else if tx_data.transaction.to_string().contains("publish") {
                        "publish"
                    } else {
                        "unknown"
                    };

                    // Get transaction status
                    let status = format!("{:?}", tx_data.effects.status);
                    let gas_used = tx_data.effects.fuel_used;

                    // Build transaction JSON
                    let tx_json = serde_json::json!({
                        "digest": tx_digest,
                        "sender": sender,
                        "type": tx_type,
                        "status": status,
                        "gas_used": gas_used,
                        "timestamp": timestamp,
                    });

                    all_transactions.push((timestamp, tx_json));
                }
                debug!("Retrieved {} transactions from store", all_transactions.len());
            }
            Err(e) => {
                warn!("Failed to get transactions from store: {}", e);
                // Continue with empty transactions list
            }
        }

        // Sort by timestamp descending (most recent first)
        all_transactions.sort_by(|a, b| b.0.cmp(&a.0));

        // Take the most recent transactions up to limit
        let transactions: Vec<JsonValue> = all_transactions
            .into_iter()
            .take(limit)
            .map(|(_, tx)| tx)
            .collect();

        let elapsed = start.elapsed();
        debug!("Got {} recent transactions in {:?}", transactions.len(), elapsed);
        Ok(serde_json::json!(transactions))
    }

    /// silver_getTokenInfo - Get token information
    pub fn silver_get_token_info(&self, params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();
        
        // Parse parameters - expecting array format [token_id] or object format
        let params_obj = Self::normalize_params(params, &["token_id"])?;
        
        let request: GetTokenInfoRequest = serde_json::from_value(params_obj)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        debug!("Getting token info for: {}", request.token_id);

        // For SBTC, we return fixed token information since it's the native token
        let response = serde_json::json!({
            "token_id": request.token_id,
            "name": "SilverBitcoin",
            "symbol": "SBTC",
            "decimals": 9,
            "total_supply": "1000000000000000000", // 1 billion SBTC in MIST
            "circulating_supply": "70000000000000000", // 70 million SBTC in MIST
            "contract_address": "0x0000000000000000000000000000000000000000",
            "type": "native",
        });

        let elapsed = start.elapsed();
        debug!("Got token info in {:?}", elapsed);
        Ok(response)
    }

    /// silver_getTokenHolders - Get token holders
    pub fn silver_get_token_holders(&self, params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();
        
        // Parse parameters - expecting array format [token_id, limit] or object format
        let params_obj = Self::normalize_params(params, &["token_id", "limit"])?;
        
        let request: GetTokenHoldersRequest = serde_json::from_value(params_obj)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        let limit = request.limit.unwrap_or(50).min(1000);
        debug!("Getting token holders for: {} with limit: {}", request.token_id, limit);

        // Get largest accounts as token holders (since SBTC is the native token)
        let mut account_balances: std::collections::BTreeMap<String, u128> = std::collections::BTreeMap::new();
        
        match self.object_store.iterate_all_objects() {
            Ok(mut iter) => {
                let mut total_scanned = 0u64;
                while let Some(obj) = iter.pop() {
                    total_scanned += 1;
                    
                    // Only process coin objects
                    if !matches!(obj.object_type, silver_core::ObjectType::Coin) {
                        continue;
                    }

                    // Extract owner address
                    let owner_addr = match &obj.owner {
                        silver_core::Owner::AddressOwner(addr) => addr.to_hex(),
                        _ => continue,
                    };

                    // Extract balance from coin data (u64 little-endian)
                    let balance = if obj.data.len() >= 8 {
                        u64::from_le_bytes([
                            obj.data[0], obj.data[1], obj.data[2], obj.data[3],
                            obj.data[4], obj.data[5], obj.data[6], obj.data[7],
                        ]) as u128
                    } else {
                        0u128
                    };

                    // Aggregate balance for this owner
                    *account_balances.entry(owner_addr).or_insert(0) += balance;
                }
                debug!("Scanned {} objects for token holders", total_scanned);
            }
            Err(e) => {
                error!("Failed to iterate objects for token holders: {}", e);
                return Err(JsonRpcError::internal_error(format!(
                    "Failed to scan objects: {}",
                    e
                )));
            }
        }

        // Sort by balance descending and take top N
        let mut holders: Vec<JsonValue> = account_balances
            .into_iter()
            .map(|(address, balance)| {
                serde_json::json!({
                    "address": address,
                    "balance": balance.to_string(),
                    "decimals": 9,
                })
            })
            .collect();

        holders.sort_by(|a, b| {
            let balance_a = a.get("balance")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<u128>().ok())
                .unwrap_or(0);
            let balance_b = b.get("balance")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<u128>().ok())
                .unwrap_or(0);
            balance_b.cmp(&balance_a)
        });

        holders.truncate(limit);

        let response = serde_json::json!({
            "token_id": request.token_id,
            "holders": holders,
        });

        let elapsed = start.elapsed();
        debug!("Got {} token holders in {:?}", holders.len(), elapsed);
        Ok(response)
    }

    /// silver_getTokenTransactions - Get token transactions
    pub fn silver_get_token_transactions(&self, params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();
        
        // Parse parameters - expecting array format [token_id, limit] or object format
        let params_obj = Self::normalize_params(params, &["token_id", "limit"])?;
        
        let request: GetTokenTransactionsRequest = serde_json::from_value(params_obj)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        let limit = request.limit.unwrap_or(50).min(1000);
        debug!("Getting token transactions for: {} with limit: {}", request.token_id, limit);

        // Query all transactions that involve the specified token
        let mut all_transactions: Vec<(u64, JsonValue)> = Vec::new();

        // Get all transactions from the transaction store
        match self.transaction_store.iterate_transactions() {
            Ok(transactions) => {
                for tx_data in transactions {
                    // Convert transaction to string to check if it involves the token
                    let tx_str = tx_data.transaction.to_string();
                    
                    // Filter transactions that involve the specified token
                    // Check if transaction data contains the token ID
                    if !tx_str.contains(&request.token_id) {
                        continue;
                    }

                    // Extract transaction metadata
                    let tx_digest = hex::encode(tx_data.effects.digest.0);
                    let timestamp = tx_data.effects.timestamp;
                    let sender = tx_data.transaction.sender().to_hex();
                    
                    // Determine transaction type
                    let tx_type = if tx_str.contains("transfer") {
                        "transfer"
                    } else if tx_str.contains("mint") {
                        "mint"
                    } else if tx_str.contains("burn") {
                        "burn"
                    } else if tx_str.contains("swap") {
                        "swap"
                    } else {
                        "unknown"
                    };

                    // Get transaction status
                    let status = format!("{:?}", tx_data.effects.status);
                    let gas_used = tx_data.effects.fuel_used;

                    // Extract recipient from transaction if available
                    let recipient = if let Some(pos) = tx_str.find("to:") {
                        tx_str[pos + 3..]
                            .split(',')
                            .next()
                            .map(|s| s.trim().to_string())
                            .unwrap_or_else(|| "unknown".to_string())
                    } else {
                        "unknown".to_string()
                    };

                    // Extract amount if available
                    let amount = if let Some(pos) = tx_str.find("amount:") {
                        tx_str[pos + 7..]
                            .split(',')
                            .next()
                            .and_then(|s| s.trim().parse::<u128>().ok())
                            .unwrap_or(0u128)
                    } else {
                        0u128
                    };

                    // Build transaction JSON
                    let tx_json = serde_json::json!({
                        "digest": tx_digest,
                        "sender": sender,
                        "recipient": recipient,
                        "type": tx_type,
                        "status": status,
                        "amount": amount.to_string(),
                        "token_id": request.token_id,
                        "timestamp": timestamp,
                        "gas_used": gas_used,
                    });

                    all_transactions.push((timestamp, tx_json));
                }
                debug!("Retrieved {} token transactions from store", all_transactions.len());
            }
            Err(e) => {
                warn!("Failed to get transactions from store: {}", e);
                // Continue with empty transactions list
            }
        }

        // Sort by timestamp descending (most recent first)
        all_transactions.sort_by(|a, b| b.0.cmp(&a.0));

        // Take the most recent transactions up to limit
        let transactions: Vec<JsonValue> = all_transactions
            .into_iter()
            .take(limit)
            .map(|(_, tx)| tx)
            .collect();

        let response = serde_json::json!({
            "token_id": request.token_id,
            "transactions": transactions,
            "total": transactions.len(),
        });

        let elapsed = start.elapsed();
        debug!("Got {} token transactions in {:?}", transactions.len(), elapsed);
        Ok(response)
    }

    /// silver_queryEvents - Query events with pagination
    pub fn silver_query_events(&self, params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();

        // Normalize parameters: array format [filter, limit, cursor] or object format
        let params_obj = Self::normalize_params(params, &["filter", "limit", "cursor"])?;

        let request: QueryEventsRequest = serde_json::from_value(params_obj)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        debug!("Querying events with filter: {:?}", request.filter);

        // Query events from event store with pagination
        let limit = request.limit.unwrap_or(50).min(1000);
        let cursor_offset = request
            .cursor
            .as_ref()
            .and_then(|c| c.strip_prefix("cursor_").and_then(|s| s.parse::<usize>().ok()))
            .unwrap_or(0);

        // Query all events from the event store
        let mut all_events: Vec<(u64, JsonValue)> = Vec::new();

        // Iterate through all objects to find events
        match self.object_store.iterate_all_objects() {
            Ok(mut iter) => {
                while let Some(obj) = iter.pop() {
                    // Check if this is an event object
                    if !matches!(obj.object_type, silver_core::ObjectType::Event) {
                        continue;
                    }

                    // Parse event data
                    let event_data = String::from_utf8_lossy(&obj.data);
                    
                    // Extract timestamp from object version (used as event timestamp)
                    let timestamp = obj.version.value() as u64;

                    // Apply filter if provided
                    let should_include = if let Some(filter) = &request.filter {
                        // Filter by event type if specified
                        if let Some(event_type) = filter.get("event_type").and_then(|v| v.as_str()) {
                            event_data.contains(event_type)
                        } else {
                            true
                        }
                    } else {
                        true
                    };

                    if should_include {
                        let event_json = serde_json::json!({
                            "id": obj.id.to_base58(),
                            "type": "event",
                            "data": event_data.to_string(),
                            "timestamp": timestamp,
                            "sender": match &obj.owner {
                                silver_core::Owner::AddressOwner(addr) => addr.to_hex(),
                                _ => "unknown".to_string(),
                            },
                        });
                        all_events.push((timestamp, event_json));
                    }
                }
            }
            Err(e) => {
                warn!("Failed to iterate objects for events: {}", e);
                // Continue with empty events list
            }
        }

        // Sort by timestamp descending (most recent first)
        all_events.sort_by(|a, b| b.0.cmp(&a.0));

        // Get total count before consuming the vector
        let total_count = all_events.len();

        // Apply pagination
        let paginated_events: Vec<JsonValue> = all_events
            .into_iter()
            .skip(cursor_offset)
            .take(limit)
            .map(|(_, event)| event)
            .collect();

        let has_next_page = cursor_offset + limit < total_count;
        let next_cursor = if has_next_page {
            Some(format!("cursor_{}", cursor_offset + limit))
        } else {
            None
        };

        let response = serde_json::json!({
            "events": paginated_events,
            "next_cursor": next_cursor,
            "has_next_page": has_next_page,
            "count": paginated_events.len(),
            "total": total_count,
        });

        let elapsed = start.elapsed();
        debug!("Queried {} events in {:?}", paginated_events.len(), elapsed);

        Ok(response)
    }

    /// Extract coin amount from object data
    /// 
    /// Parses the coin object structure to extract the amount value.
    /// Coin objects store the amount as the first 8 bytes in little-endian format.
    /// 
    /// # Arguments
    /// * `data` - The serialized coin object data
    /// 
    /// # Returns
    /// The coin amount in MIST (smallest unit), or 0 if parsing fails
    #[allow(dead_code)]
    fn extract_coin_amount(data: &[u8]) -> Result<u64, String> {
        // Coin objects have amount stored in first 8 bytes as little-endian u64
        if data.len() >= 8 {
            let mut bytes = [0u8; 8];
            bytes.copy_from_slice(&data[..8]);
            let amount = u64::from_le_bytes(bytes);
            Ok(amount)
        } else {
            // Invalid coin object - too small
            Err("Coin object data too small".to_string())
        }
    }
}

// ============================================================================
// Request Types
// ============================================================================

/// Request to get account information
#[derive(Debug, Deserialize)]
pub struct GetAccountInfoRequest {
    /// Account address to query
    pub address: String,
}

/// Request to get multiple accounts information
#[derive(Debug, Deserialize)]
pub struct GetMultipleAccountsRequest {
    /// List of account addresses to query
    pub addresses: Vec<String>,
}

/// Request to get transaction history for an address
#[derive(Debug, Deserialize)]
pub struct GetTransactionHistoryRequest {
    /// Account address to query
    pub address: String,
    /// Maximum number of transactions to return
    pub limit: Option<usize>,
}

/// Request to get token accounts owned by an address
#[derive(Debug, Deserialize)]
pub struct GetTokenAccountsByOwnerRequest {
    /// Owner address
    pub owner: String,
}

/// Request to get accounts owned by a program
#[derive(Debug, Deserialize)]
pub struct GetProgramAccountsRequest {
    /// Program ID
    pub program_id: String,
}

/// Request to get signature statuses
#[derive(Debug, Deserialize)]
pub struct GetSignatureStatusesRequest {
    /// List of transaction signatures to check
    pub signatures: Vec<String>,
}

/// Request to get block time
#[derive(Debug, Deserialize)]
pub struct GetBlockTimeRequest {
    /// Block number
    pub block_number: u64,
}

/// Request to get minimum balance for rent exemption
#[derive(Debug, Deserialize)]
pub struct GetMinimumBalanceRequest {
    /// Data size in bytes
    pub data_size: u64,
}

/// Request to get fee for a message
#[derive(Debug, Deserialize)]
pub struct GetFeeForMessageRequest {
    /// Message to calculate fee for
    pub message: String,
}

/// Request to get stake activation information
#[derive(Debug, Deserialize)]
pub struct GetStakeActivationRequest {
    /// Stake account address
    pub address: String,
}

/// Request to query events with pagination
#[derive(Debug, Deserialize)]
pub struct QueryEventsRequest {
    /// Event filter criteria
    #[serde(default)]
    pub filter: Option<JsonValue>,
    /// Pagination cursor
    #[serde(default)]
    pub cursor: Option<String>,
    /// Maximum number of events to return
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Request to get recent blocks
#[derive(Debug, Deserialize)]
pub struct GetRecentBlocksRequest {
    /// Maximum number of blocks to return
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Request to get recent transactions
#[derive(Debug, Deserialize)]
pub struct GetRecentTransactionsRequest {
    /// Maximum number of transactions to return
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Request to get token information
#[derive(Debug, Deserialize)]
pub struct GetTokenInfoRequest {
    /// Token ID
    pub token_id: String,
}

/// Request to get token holders
#[derive(Debug, Deserialize)]
pub struct GetTokenHoldersRequest {
    /// Token ID
    pub token_id: String,
    /// Maximum number of holders to return
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Request to get token transactions
#[derive(Debug, Deserialize)]
pub struct GetTokenTransactionsRequest {
    /// Token ID
    pub token_id: String,
    /// Maximum number of transactions to return
    #[serde(default)]
    pub limit: Option<usize>,
}

// ============================================================================
// Helper Functions
// ============================================================================

fn parse_address(addr: &str) -> Result<SilverAddress, JsonRpcError> {
    if addr.starts_with("0x") {
        SilverAddress::from_hex(addr)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid address: {}", e)))
    } else {
        SilverAddress::from_base58(addr)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid address: {}", e)))
    }
}

fn parse_object_id(id: &str) -> Result<ObjectID, JsonRpcError> {
    if id.starts_with("0x") {
        ObjectID::from_hex(id)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid object ID: {}", e)))
    } else {
        ObjectID::from_base58(id)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid object ID: {}", e)))
    }
}

fn parse_transaction_digest(digest: &str) -> Result<TransactionDigest, JsonRpcError> {
    let digest_str = digest.trim_start_matches("0x");
    let bytes = hex::decode(digest_str)
        .map_err(|e| JsonRpcError::invalid_params(format!("Invalid digest: {}", e)))?;

    if bytes.len() != 64 {
        return Err(JsonRpcError::invalid_params(format!(
            "Digest must be 64 bytes, got {}",
            bytes.len()
        )));
    }

    let mut arr = [0u8; 64];
    arr.copy_from_slice(&bytes);
    Ok(TransactionDigest::new(arr))
}
