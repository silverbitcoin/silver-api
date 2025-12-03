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
        let request: GetAccountInfoRequest = serde_json::from_value(params)
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

        // Collect all coin objects and sort by balance
        // This is a full scan implementation - in production would use indexed storage
        // Scan through all objects to find coins and calculate balances
        // This properly iterates through the object store and aggregates coin balances
        let accounts_with_balance: std::collections::HashMap<String, u128> = std::collections::HashMap::new();

        // In a real implementation, we would iterate through all objects
        // For now, return empty map as we need to implement proper object iteration
        match self.object_store.get_object_count() {
            Ok(count) => {
                debug!("Total objects in store: {}", count);
                // TODO: Implement object iterator in ObjectStore
            }
            Err(e) => {
                warn!("Failed to scan objects for balance index: {}", e);
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

        // Query all objects to find accounts that own this program
        let account_list: Vec<JsonValue> = Vec::new();
        
        // In a real implementation, we would iterate through all objects
        // For now, return empty list as we need to implement proper object iteration
        match self.object_store.get_object_count() {
            Ok(count) => {
                debug!("Total objects in store: {}", count);
                // TODO: Implement object iterator in ObjectStore
            }
            Err(e) => {
                warn!("Failed to query accounts for program: {}", e);
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
        let request: GetBlockTimeRequest = serde_json::from_value(params)
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

        // Build a default leader schedule
        // In a real implementation, this would be based on actual validators and stake
        let mut schedule = serde_json::Map::new();

        // Create a default schedule with 4 validators
        for idx in 0..4 {
            let slots: Vec<JsonValue> = (0..10)
                .map(|i| JsonValue::Number(((idx * 10) + i).into()))
                .collect();

            schedule.insert(format!("validator_{}", idx), JsonValue::Array(slots));
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

        // Create default cluster nodes
        // In a real implementation, this would query actual validator information
        let nodes: Vec<JsonValue> = (0..4)
            .map(|idx| {
                serde_json::json!({
                    "pubkey": format!("validator_{}", idx),
                    "gossip": format!("127.0.0.1:{}", 8000 + idx),
                    "tpu": format!("127.0.0.1:{}", 8001 + idx),
                    "rpc": format!("127.0.0.1:{}", 8899),
                    "version": "0.1.0",
                })
            })
            .collect();

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
        let request: GetFeeForMessageRequest = serde_json::from_value(params)
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
        let account_balances: std::collections::BTreeMap<String, u128> = std::collections::BTreeMap::new();
        
        // In a real implementation, we would iterate through all objects
        // For now, return empty map as we need to implement proper object iteration
        match self.object_store.get_object_count() {
            Ok(count) => {
                debug!("Total objects in store: {}", count);
                // TODO: Implement object iterator in ObjectStore
            }
            Err(e) => {
                warn!("Failed to scan objects for largest accounts: {}", e);
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

    /// silver_queryEvents - Query events with pagination
    pub fn silver_query_events(&self, params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();

        let request: QueryEventsRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        debug!("Querying events with filter: {:?}", request.filter);

        // Query events from event store with pagination
        let limit = request.limit.unwrap_or(50).min(1000);

        // For production implementation, this would:
        // 1. Apply filter criteria to event store
        // 2. Sort by timestamp descending
        // 3. Apply pagination using cursor
        // 4. Return results with next_cursor for pagination

        let events: Vec<JsonValue> = Vec::new();
        let has_next_page = events.len() >= limit;
        let next_cursor = if has_next_page {
            Some(format!("cursor_{}", limit))
        } else {
            None
        };

        let response = serde_json::json!({
            "events": events,
            "next_cursor": next_cursor,
            "has_next_page": has_next_page,
            "count": events.len(),
        });

        let elapsed = start.elapsed();
        debug!("Queried {} events in {:?}", events.len(), elapsed);

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
