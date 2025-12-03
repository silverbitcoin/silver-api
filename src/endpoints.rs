//! API endpoint implementations
//!
//! Provides query and transaction endpoints for blockchain interaction.

use crate::rpc::JsonRpcError;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use silver_core::{
    Object, ObjectID, ObjectType, SilverAddress, Transaction, TransactionDigest,
    MIN_FUEL_PRICE_MIST, MIST_PER_SBTC,
};
use silver_storage::{BlockStore, EventStore, ObjectStore, StoredTransaction, TransactionStore};
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, error, warn};

/// Query endpoints for blockchain data
pub struct QueryEndpoints {
    object_store: Arc<ObjectStore>,
    transaction_store: Arc<TransactionStore>,
    block_store: Arc<BlockStore>,
    #[allow(dead_code)]
    event_store: Arc<EventStore>,
}

impl QueryEndpoints {
    /// Create new query endpoints
    pub fn new(
        object_store: Arc<ObjectStore>,
        transaction_store: Arc<TransactionStore>,
        block_store: Arc<BlockStore>,
        event_store: Arc<EventStore>,
    ) -> Self {
        Self {
            object_store,
            transaction_store,
            block_store,
            event_store,
        }
    }

    /// Extract coin amount from serialized coin object data
    /// 
    /// Coin objects are serialized with the amount as the first 8 bytes in little-endian format.
    /// This properly deserializes the coin data to extract the actual amount.
    fn extract_coin_amount(data: &[u8]) -> Result<u64, String> {
        if data.len() < 8 {
            return Err("Coin data too short".to_string());
        }

        // Extract first 8 bytes as little-endian u64
        let mut amount_bytes = [0u8; 8];
        amount_bytes.copy_from_slice(&data[0..8]);
        Ok(u64::from_le_bytes(amount_bytes))
    }

    /// silver_getObject - Get an object by ID
    pub fn silver_get_object(&self, params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();

        let request: GetObjectRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        let object_id = parse_object_id(&request.id)?;

        debug!("Getting object: {}", object_id);

        let object = self.object_store.get_object(&object_id).map_err(|e| {
            error!("Failed to get object {}: {}", object_id, e);
            JsonRpcError::internal_error(format!("Failed to get object: {}", e))
        })?;

        let object = object
            .ok_or_else(|| JsonRpcError::new(-32001, format!("Object not found: {}", object_id)))?;

        let response = ObjectResponse::from_object(&object);

        let elapsed = start.elapsed();
        debug!("Got object {} in {:?}", object_id, elapsed);

        if elapsed.as_millis() > 100 {
            warn!(
                "Query took {}ms (target: <100ms) for object {}",
                elapsed.as_millis(),
                object_id
            );
        }

        Ok(serde_json::to_value(response).unwrap())
    }

    /// silver_getObjectsByOwner - Get objects owned by an address
    pub fn silver_get_objects_by_owner(
        &self,
        params: JsonValue,
    ) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();

        let request: GetObjectsByOwnerRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        let owner = parse_address(&request.owner)?;
        let limit = request.limit.unwrap_or(50).min(1000);

        debug!("Getting objects for owner: {}", owner);

        let objects = self
            .object_store
            .get_objects_by_owner(&owner)
            .map_err(|e| {
                error!("Failed to get objects for owner {}: {}", owner, e);
                JsonRpcError::internal_error(format!("Failed to get objects: {}", e))
            })?;

        let response: Vec<ObjectResponse> = objects
            .into_iter()
            .take(limit)
            .map(|obj| ObjectResponse::from_object(&obj))
            .collect();

        let elapsed = start.elapsed();
        debug!(
            "Got {} objects for owner {} in {:?}",
            response.len(),
            owner,
            elapsed
        );

        Ok(serde_json::to_value(response).unwrap())
    }

    /// silver_getTransaction - Get a transaction by digest
    pub fn silver_get_transaction(&self, params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();

        let request: GetTransactionRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        let digest = parse_transaction_digest(&request.digest)?;

        debug!("Getting transaction: {}", hex::encode(digest.0));

        let tx_data = self
            .transaction_store
            .get_transaction(&digest)
            .map_err(|e| {
                error!("Failed to get transaction: {}", e);
                JsonRpcError::internal_error(format!("Failed to get transaction: {}", e))
            })?;

        let tx_data = tx_data.ok_or_else(|| JsonRpcError::new(-32002, "Transaction not found"))?;

        let response = TransactionResponse::from_stored_transaction(&tx_data);

        let elapsed = start.elapsed();
        debug!("Got transaction in {:?}", elapsed);

        Ok(serde_json::to_value(response).unwrap())
    }

    /// silver_getBalance - Get balance of an address
    pub fn silver_get_balance(&self, params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();

        let request: GetBalanceRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        let address = parse_address(&request.address)?;

        debug!("Getting balance for: {}", address);

        // Get all objects owned by address
        let objects = self
            .object_store
            .get_objects_by_owner(&address)
            .map_err(|e| {
                error!("Failed to get balance: {}", e);
                JsonRpcError::internal_error(format!("Failed to get balance: {}", e))
            })?;

        // Sum up coin values by properly deserializing coin objects
        let mut total_balance: u128 = 0;
        for obj in objects {
            if matches!(obj.object_type, ObjectType::Coin) {
                // Deserialize coin object to extract amount
                match Self::extract_coin_amount(&obj.data) {
                    Ok(amount) => {
                        total_balance = total_balance.saturating_add(amount as u128);
                    }
                    Err(e) => {
                        warn!("Failed to extract coin amount: {}", e);
                        // Skip malformed coin objects
                        continue;
                    }
                }
            }
        }

        let response = serde_json::json!({
            "address": address.to_hex(),
            "balance_mist": total_balance,
            "balance_sbtc": format!("{:.9}", total_balance as f64 / MIST_PER_SBTC as f64),
        });
        
        debug!("Balance for {}: {} mist", address, total_balance);

        let elapsed = start.elapsed();
        debug!("Got balance in {:?}", elapsed);

        Ok(response)
    }

    /// silver_estimateGas - Estimate gas for a transaction
    pub fn silver_estimate_gas(&self, params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();

        let request: EstimateGasRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        debug!("Estimating gas for transaction");

        // Base gas cost for transaction overhead
        let mut estimated_gas = 1000u64;

        // Add gas per command (varies by command type)
        if let Some(commands) = &request.commands {
            for _cmd in commands {
                estimated_gas += 500; // Base command cost
            }
        }

        let response = serde_json::json!({
            "estimated_gas": estimated_gas,
            "min_fuel_price": MIN_FUEL_PRICE_MIST,
            "estimated_cost_mist": estimated_gas * MIN_FUEL_PRICE_MIST,
        });

        let elapsed = start.elapsed();
        debug!("Estimated gas in {:?}", elapsed);

        Ok(response)
    }

    /// silver_getLatestBlockNumber - Get the latest block number
    pub fn silver_get_latest_block_number(&self) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();
        debug!("Getting latest block number");

        let block_number = self
            .transaction_store
            .get_latest_block_number()
            .map_err(|e| {
                error!("Failed to get latest block number: {}", e);
                JsonRpcError::internal_error(format!("Failed to get latest block number: {}", e))
            })?;

        let response = serde_json::json!({
            "block_number": block_number,
        });

        let elapsed = start.elapsed();
        debug!("Got latest block number {} in {:?}", block_number, elapsed);

        Ok(response)
    }

    /// silver_getBlockByNumber - Get block by number
    pub fn silver_get_block_by_number(&self, params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();

        let request: GetBlockByNumberRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        let block_number = if request.block_number == "latest" {
            self.block_store
                .get_latest_block_number()
                .map_err(|e| {
                    error!("Failed to get latest block number: {}", e);
                    JsonRpcError::internal_error(format!(
                        "Failed to get latest block number: {}",
                        e
                    ))
                })?
        } else {
            request
                .block_number
                .parse::<u64>()
                .map_err(|e| JsonRpcError::invalid_params(format!("Invalid block number: {}", e)))?
        };

        debug!("Getting block: {}", block_number);

        // Try to get block from storage
        let block = self
            .block_store
            .get_block(block_number)
            .map_err(|e| {
                error!("Failed to get block: {}", e);
                JsonRpcError::internal_error(format!("Failed to get block: {}", e))
            })?;

        let response = match block {
            Some(block_data) => {
                serde_json::json!({
                    "number": block_data.number,
                    "hash": hex::encode(&block_data.hash),
                    "parent_hash": hex::encode(&block_data.parent_hash),
                    "timestamp": block_data.timestamp,
                    "transactions": block_data.transactions.iter()
                        .map(|tx| hex::encode(tx))
                        .collect::<Vec<_>>(),
                    "validator": hex::encode(&block_data.validator),
                    "gas_used": block_data.gas_used,
                    "gas_limit": block_data.gas_limit,
                })
            }
            None => {
                return Err(JsonRpcError::new(
                    -32001,
                    format!("Block {} not found", block_number),
                ));
            }
        };

        let elapsed = start.elapsed();
        debug!("Got block {} in {:?}", block_number, elapsed);

        Ok(response)
    }

    /// silver_getValidators - Get current validator set
    pub fn silver_get_validators(&self) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();
        debug!("Getting validators");

        let response = serde_json::json!({
            "validators": [],
            "count": 0,
        });

        let elapsed = start.elapsed();
        debug!("Got validators in {:?}", elapsed);

        Ok(response)
    }

    /// silver_getNetworkStats - Get network statistics
    pub fn silver_get_network_stats(&self) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();
        debug!("Getting network stats");

        let response = serde_json::json!({
            "tps": 0.0,
            "total_transactions": 0u64,
            "total_blocks": 0u64,
            "active_validators": 0usize,
            "network_health": "unknown",
        });

        let elapsed = start.elapsed();
        debug!("Got network stats in {:?}", elapsed);

        Ok(response)
    }

    /// silver_getTransactionsByAddress - Get transactions for an address
    pub fn silver_get_transactions_by_address(
        &self,
        params: JsonValue,
    ) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();

        let request: GetTransactionsByAddressRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        let address = parse_address(&request.address)?;
        let _limit = request.limit.unwrap_or(50).min(500);

        debug!("Getting transactions for address: {}", address);

        // Get transaction count to estimate how many to query
        let tx_count = self
            .transaction_store
            .get_transaction_count()
            .map_err(|e| {
                error!("Failed to get transaction count: {}", e);
                JsonRpcError::internal_error(format!("Failed to get transaction count: {}", e))
            })?;

        // In a real implementation, we would iterate through transactions
        // For now, return empty list as we don't have a full scan method
        let response: Vec<TransactionResponse> = Vec::new();

        // Note: A production implementation would need to:
        // 1. Implement a transaction iterator in TransactionStore
        // 2. Filter transactions by address (sender or recipient)
        if tx_count > 0 {
            debug!("Found {} total transactions, filtering for address {}", tx_count, address);
        }

        let elapsed = start.elapsed();
        debug!(
            "Got {} transactions for address {} in {:?}",
            response.len(),
            address,
            elapsed
        );

        Ok(serde_json::to_value(response).unwrap())
    }

    /// silver_getGasPrice - Get current gas price
    pub fn silver_get_gas_price(&self) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();
        debug!("Getting gas price");

        let response = serde_json::json!({
            "gas_price": MIN_FUEL_PRICE_MIST,
        });

        let elapsed = start.elapsed();
        debug!("Got gas price in {:?}", elapsed);

        Ok(response)
    }

    /// silver_getTransactionReceipt - Get transaction receipt
    pub fn silver_get_transaction_receipt(
        &self,
        params: JsonValue,
    ) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();

        let request: GetTransactionReceiptRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        let digest = parse_transaction_digest(&request.digest)?;

        debug!("Getting transaction receipt: {}", hex::encode(digest.0));

        let tx_data = self
            .transaction_store
            .get_transaction(&digest)
            .map_err(|e| {
                error!("Failed to get transaction receipt: {}", e);
                JsonRpcError::internal_error(format!("Failed to get transaction receipt: {}", e))
            })?;

        let tx_data =
            tx_data.ok_or_else(|| JsonRpcError::new(-32004, "Transaction receipt not found"))?;

        let response = TransactionReceiptResponse::from_stored_transaction(&tx_data);

        let elapsed = start.elapsed();
        debug!("Got transaction receipt in {:?}", elapsed);

        Ok(serde_json::to_value(response).unwrap())
    }

    /// silver_getTransactionByHash - Get transaction by hash
    pub fn silver_get_transaction_by_hash(
        &self,
        params: JsonValue,
    ) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();

        let request: GetTransactionByHashRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        let digest = parse_transaction_digest(&request.hash)?;

        debug!("Getting transaction by hash: {}", hex::encode(digest.0));

        let tx_data = self
            .transaction_store
            .get_transaction(&digest)
            .map_err(|e| {
                error!("Failed to get transaction: {}", e);
                JsonRpcError::internal_error(format!("Failed to get transaction: {}", e))
            })?;

        let tx_data = tx_data.ok_or_else(|| JsonRpcError::new(-32005, "Transaction not found"))?;

        let response = TransactionResponse::from_stored_transaction(&tx_data);

        let elapsed = start.elapsed();
        debug!("Got transaction in {:?}", elapsed);

        Ok(serde_json::to_value(response).unwrap())
    }

    /// silver_getCode - Get code for an address (for smart contracts)
    pub fn silver_get_code(&self, params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();

        let request: GetCodeRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        let address = parse_address(&request.address)?;

        debug!("Getting code for: {}", address);

        // Query object store for contract code at this address
        // In a real implementation, we would derive ObjectID from address
        // For now, return empty code as we need proper address-to-object mapping
        let code = match ObjectID::from_hex(&address.to_string()) {
            Ok(object_id) => {
                match self.object_store.get_object(&object_id) {
                    Ok(Some(_obj)) => {
                        // Extract code from object data
                        // Note: ObjectType::Contract may not exist, return empty for now
                        "0x".to_string()
                    }
                    Ok(None) => {
                        // Object not found, return empty code
                        "0x".to_string()
                    }
                    Err(e) => {
                        warn!("Failed to get code for address {}: {}", address, e);
                        "0x".to_string()
                    }
                }
            }
            Err(e) => {
                warn!("Failed to parse address {}: {}", address, e);
                "0x".to_string()
            }
        };

        let response = serde_json::json!({
            "address": address.to_hex(),
            "code": code,
        });

        let elapsed = start.elapsed();
        debug!("Got code in {:?}", elapsed);

        Ok(response)
    }

    /// silver_getTransactionCount - Get transaction count for an address
    pub fn silver_get_transaction_count(
        &self,
        params: JsonValue,
    ) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();

        let request: GetTransactionCountRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        let address = parse_address(&request.address)?;

        debug!("Getting transaction count for: {}", address);

        // Get approximate transaction count
        let count = match self.transaction_store.get_transaction_count() {
            Ok(total_count) => {
                // In a real implementation, we would filter by address
                // For now, return the total count as an approximation
                debug!("Total transactions in store: {}", total_count);
                total_count
            }
            Err(e) => {
                warn!("Failed to get transaction count: {}", e);
                0
            }
        };

        let response = serde_json::json!({
            "address": address.to_hex(),
            "count": count,
        });

        let elapsed = start.elapsed();
        debug!("Got transaction count in {:?}", elapsed);

        Ok(response)
    }

    /// silver_getEvents - Get events filtered by criteria
    pub fn silver_get_events(&self, params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();

        let request: GetEventsRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        debug!("Getting events with filter: {:?}", request.filter);

        // Query event store based on filter type
        let events: Vec<silver_storage::Event> = match &request.filter {
            Some(filter) => {
                // In a real implementation, we would filter by the provided criteria
                // For now, return empty list as we need to implement proper filtering
                debug!("Filtering events by: {:?}", filter);
                Vec::new()
            }
            None => {
                // Get all events (limited by count)
                match self.event_store.get_event_count() {
                    Ok(count) => {
                        debug!("Total events in store: {}", count);
                        Vec::new()
                    }
                    Err(e) => {
                        warn!("Failed to get event count: {}", e);
                        Vec::new()
                    }
                }
            }
        };

        let count = events.len();
        let response = serde_json::json!({
            "events": events,
            "count": count,
        });

        let elapsed = start.elapsed();
        debug!("Got {} events in {:?}", count, elapsed);

        Ok(response)
    }

    /// silver_getCheckpoint - Get checkpoint at specific sequence number
    pub fn silver_get_checkpoint(&self, params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();

        let request: GetCheckpointRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        debug!("Getting checkpoint: {}", request.sequence_number);

        let response = serde_json::json!({
            "sequence_number": request.sequence_number,
            "digest": "0x0000000000000000000000000000000000000000000000000000000000000000",
            "timestamp": 0u64,
        });

        let elapsed = start.elapsed();
        debug!("Got checkpoint in {:?}", elapsed);

        Ok(response)
    }

    /// silver_getLatestCheckpoint - Get latest checkpoint
    pub fn silver_get_latest_checkpoint(&self) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();
        debug!("Getting latest checkpoint");

        let response = serde_json::json!({
            "sequence_number": 0u64,
            "digest": "0x0000000000000000000000000000000000000000000000000000000000000000",
            "timestamp": 0u64,
        });

        let elapsed = start.elapsed();
        debug!("Got latest checkpoint in {:?}", elapsed);

        Ok(response)
    }

    /// silver_queryEvents - Query events with pagination
    pub fn silver_query_events(&self, params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();

        let request: QueryEventsRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        debug!("Querying events: {:?}", request);

        let response = serde_json::json!({
            "events": [],
            "next_cursor": null,
            "has_next_page": false,
        });

        let elapsed = start.elapsed();
        debug!("Queried events in {:?}", elapsed);

        Ok(response)
    }

    /// silver_getObjectsOwnedByAddress - Get all objects owned by address
    pub fn silver_get_objects_owned_by_address(
        &self,
        params: JsonValue,
    ) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();

        let request: GetObjectsOwnedByAddressRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        let address = parse_address(&request.address)?;
        let limit = request.limit.unwrap_or(50).min(1000);

        debug!("Getting objects owned by address: {}", address);

        let objects = self
            .object_store
            .get_objects_by_owner(&address)
            .map_err(|e| {
                error!("Failed to get objects: {}", e);
                JsonRpcError::internal_error(format!("Failed to get objects: {}", e))
            })?;

        let response: Vec<ObjectResponse> = objects
            .into_iter()
            .take(limit)
            .map(|obj| ObjectResponse::from_object(&obj))
            .collect();

        let elapsed = start.elapsed();
        debug!("Got {} objects in {:?}", response.len(), elapsed);

        Ok(serde_json::to_value(response).unwrap())
    }

    /// silver_getObjectsOwnedByObject - Get objects owned by another object
    pub fn silver_get_objects_owned_by_object(
        &self,
        params: JsonValue,
    ) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();

        let request: GetObjectsOwnedByObjectRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        let object_id = parse_object_id(&request.object_id)?;

        debug!("Getting objects owned by object: {}", object_id);

        // In a real implementation, we would query for objects owned by this object
        // For now, return empty list as we need to implement proper ownership tracking
        let response: Vec<ObjectResponse> = match self.object_store.get_object_count() {
            Ok(count) => {
                debug!("Total objects in store: {}", count);
                Vec::new()
            }
            Err(e) => {
                warn!("Failed to get object count: {}", e);
                Vec::new()
            }
        };

        let elapsed = start.elapsed();
        debug!("Got {} objects in {:?}", response.len(), elapsed);

        Ok(serde_json::to_value(response).unwrap())
    }

    /// silver_getObjectHistory - Get object version history
    pub fn silver_get_object_history(&self, params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();

        let request: GetObjectHistoryRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        let object_id = parse_object_id(&request.object_id)?;

        debug!("Getting object history: {}", object_id);

        let history = self
            .object_store
            .get_object_history(&object_id)
            .map_err(|e| {
                error!("Failed to get object history: {}", e);
                JsonRpcError::internal_error(format!("Failed to get object history: {}", e))
            })?;

        let response: Vec<ObjectResponse> = history
            .into_iter()
            .map(|obj| ObjectResponse::from_object(&obj))
            .collect();

        let elapsed = start.elapsed();
        debug!("Got {} versions in {:?}", response.len(), elapsed);

        Ok(serde_json::to_value(response).unwrap())
    }


}

/// Transaction endpoints for submitting transactions
pub struct TransactionEndpoints {
    transaction_store: Arc<TransactionStore>,
}

impl TransactionEndpoints {
    /// Create new transaction endpoints
    pub fn new(transaction_store: Arc<TransactionStore>) -> Self {
        Self { transaction_store }
    }

    /// silver_submitTransaction - Submit a transaction to the blockchain
    pub fn silver_submit_transaction(&self, params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();

        let request: SubmitTransactionRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        debug!("Submitting transaction");

        // Parse and validate transaction
        let tx: Transaction = serde_json::from_value(request.transaction)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid transaction: {}", e)))?;

        tx.validate().map_err(|e| {
            JsonRpcError::invalid_params(format!("Transaction validation failed: {}", e))
        })?;

        let digest = tx.digest();

        // Create execution effects (pending status)
        let effects = silver_storage::TransactionEffects {
            digest,
            status: silver_storage::ExecutionStatus::Pending,
            fuel_used: 0,
            error_message: None,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        };

        // Store transaction
        self.transaction_store
            .store_transaction(&tx, effects)
            .map_err(|e| {
                error!("Failed to submit transaction: {}", e);
                JsonRpcError::internal_error(format!("Failed to submit transaction: {}", e))
            })?;

        let response = serde_json::json!({
            "digest": hex::encode(digest.0),
            "status": "pending",
        });

        let elapsed = start.elapsed();
        debug!("Submitted transaction in {:?}", elapsed);

        Ok(response)
    }

    /// silver_dryRunTransaction - Dry run a transaction without submitting
    pub fn silver_dry_run_transaction(&self, params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();

        let request: DryRunTransactionRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        debug!("Dry running transaction");

        let tx: Transaction = serde_json::from_value(request.transaction)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid transaction: {}", e)))?;

        tx.validate().map_err(|e| {
            JsonRpcError::invalid_params(format!("Transaction validation failed: {}", e))
        })?;

        // Execute transaction in dry-run mode (no state changes)
        // In a real implementation, we would execute the transaction
        // For now, return a simulated result
        let execution_result = serde_json::json!({
            "status": "success",
            "gas_used": 1000,
            "effects": {
                "created": [],
                "modified": [],
                "deleted": [],
            },
            "return_values": [],
        });

        let elapsed = start.elapsed();
        debug!("Dry run completed in {:?}", elapsed);

        Ok(execution_result)
    }
}

// ============================================================================
// Request/Response Types
// ============================================================================

/// Request to get an object by ID
#[derive(Debug, Deserialize)]
struct GetObjectRequest {
    /// Object ID (hex or base58)
    id: String,
}

/// Request to get objects owned by an address
#[derive(Debug, Deserialize)]
struct GetObjectsByOwnerRequest {
    /// Owner address
    owner: String,
    /// Maximum number of objects to return
    limit: Option<usize>,
}

/// Request to get a transaction by digest
#[derive(Debug, Deserialize)]
struct GetTransactionRequest {
    /// Transaction digest (hex encoded)
    digest: String,
}

/// Request to get balance of an address
#[derive(Debug, Deserialize)]
struct GetBalanceRequest {
    /// Account address
    address: String,
}

/// Request to estimate gas for a transaction
#[derive(Debug, Deserialize)]
struct EstimateGasRequest {
    /// Sender address (optional) - used for validation in real implementation
    #[allow(dead_code)]
    sender: Option<String>,
    /// Transaction commands (optional)
    commands: Option<Vec<JsonValue>>,
}

/// Request to submit a transaction
#[derive(Debug, Deserialize)]
struct SubmitTransactionRequest {
    /// Signed transaction data
    transaction: JsonValue,
}

/// Request to dry run a transaction
#[derive(Debug, Deserialize)]
struct DryRunTransactionRequest {
    /// Transaction to simulate
    transaction: JsonValue,
}

/// Request to get block by number
#[derive(Debug, Deserialize)]
struct GetBlockByNumberRequest {
    /// Block number or "latest"
    block_number: String,
}

/// Request to get transactions by address
#[derive(Debug, Deserialize)]
struct GetTransactionsByAddressRequest {
    /// Account address
    address: String,
    /// Maximum number of transactions
    limit: Option<usize>,
}

/// Request to get transaction receipt
#[derive(Debug, Deserialize)]
struct GetTransactionReceiptRequest {
    /// Transaction digest
    digest: String,
}

/// Request to get transaction by hash
#[derive(Debug, Deserialize)]
struct GetTransactionByHashRequest {
    /// Transaction hash
    hash: String,
}

/// Request to get code for an address
#[derive(Debug, Deserialize)]
struct GetCodeRequest {
    /// Account address
    address: String,
}

/// Request to get transaction count
#[derive(Debug, Deserialize)]
struct GetTransactionCountRequest {
    /// Account address
    address: String,
}

/// Request to get events
#[derive(Debug, Deserialize)]
struct GetEventsRequest {
    /// Event filter criteria
    filter: Option<JsonValue>,
}

/// Request to get checkpoint
#[derive(Debug, Deserialize)]
struct GetCheckpointRequest {
    /// Checkpoint sequence number
    sequence_number: u64,
}

/// Request to query events with pagination
#[derive(Debug, Deserialize)]
struct QueryEventsRequest {
    /// Event filter criteria - used for filtering in real implementation
    #[serde(default)]
    #[allow(dead_code)]
    filter: Option<JsonValue>,
    /// Pagination cursor - used for pagination in real implementation
    #[serde(default)]
    #[allow(dead_code)]
    cursor: Option<String>,
    /// Maximum number of events to return - used for limiting results
    #[serde(default)]
    #[allow(dead_code)]
    limit: Option<usize>,
}

/// Request to get objects owned by address
#[derive(Debug, Deserialize)]
struct GetObjectsOwnedByAddressRequest {
    /// Owner address
    address: String,
    /// Maximum number of objects
    limit: Option<usize>,
}

/// Request to get objects owned by object
#[derive(Debug, Deserialize)]
struct GetObjectsOwnedByObjectRequest {
    /// Parent object ID
    object_id: String,
}

/// Request to get object history
#[derive(Debug, Deserialize)]
struct GetObjectHistoryRequest {
    /// Object ID
    object_id: String,
}

/// Response containing object information
#[derive(Debug, Serialize)]
struct ObjectResponse {
    /// Object ID (base58)
    id: String,
    /// Object version number
    version: u64,
    /// Object owner
    owner: String,
    /// Object type
    object_type: String,
    /// Size of object data in bytes
    data_size: usize,
    /// Digest of previous transaction
    previous_transaction: String,
    /// Storage rebate amount
    storage_rebate: u64,
}

impl ObjectResponse {
    fn from_object(obj: &Object) -> Self {
        Self {
            id: obj.id.to_base58(),
            version: obj.version.value(),
            owner: format!("{}", obj.owner),
            object_type: format!("{}", obj.object_type),
            data_size: obj.data.len(),
            previous_transaction: hex::encode(obj.previous_transaction.0),
            storage_rebate: obj.storage_rebate,
        }
    }
}

/// Response containing transaction information
#[derive(Debug, Serialize)]
struct TransactionResponse {
    /// Transaction digest (hex)
    digest: String,
    /// Sender address
    sender: String,
    /// Fuel budget
    fuel_budget: u64,
    /// Fuel price per unit
    fuel_price: u64,
    /// Execution status
    status: String,
    /// Fuel used
    fuel_used: u64,
    /// Execution timestamp
    timestamp: u64,
    /// Error message if execution failed
    error_message: Option<String>,
}

impl TransactionResponse {
    fn from_stored_transaction(tx_data: &StoredTransaction) -> Self {
        Self {
            digest: hex::encode(tx_data.effects.digest.0),
            sender: tx_data.transaction.sender().to_hex(),
            fuel_budget: tx_data.transaction.fuel_budget(),
            fuel_price: tx_data.transaction.fuel_price(),
            status: format!("{:?}", tx_data.effects.status),
            fuel_used: tx_data.effects.fuel_used,
            timestamp: tx_data.effects.timestamp,
            error_message: tx_data.effects.error_message.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
struct TransactionReceiptResponse {
    digest: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    sender: Option<String>,
    status: String,
    fuel_used: u64,
    timestamp: u64,
    error_message: Option<String>,
}

impl TransactionReceiptResponse {
    fn from_stored_transaction(tx_data: &StoredTransaction) -> Self {
        Self {
            digest: hex::encode(tx_data.effects.digest.0),
            sender: Some(tx_data.transaction.sender().to_hex()),
            status: format!("{:?}", tx_data.effects.status),
            fuel_used: tx_data.effects.fuel_used,
            timestamp: tx_data.effects.timestamp,
            error_message: tx_data.effects.error_message.clone(),
        }
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

fn parse_object_id(id: &str) -> Result<ObjectID, JsonRpcError> {
    // Try hex first
    if id.starts_with("0x") {
        ObjectID::from_hex(id)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid object ID: {}", e)))
    } else {
        // Try base58
        ObjectID::from_base58(id)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid object ID: {}", e)))
    }
}

fn parse_address(addr: &str) -> Result<SilverAddress, JsonRpcError> {
    // Try hex first
    if addr.starts_with("0x") {
        SilverAddress::from_hex(addr)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid address: {}", e)))
    } else {
        // Try base58
        SilverAddress::from_base58(addr)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid address: {}", e)))
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

// ============================================================================
// Tests
// ============================================================================

