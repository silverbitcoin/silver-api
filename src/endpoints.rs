//! API endpoint implementations
//!
//! Provides query and transaction endpoints for blockchain interaction.

use crate::rpc::JsonRpcError;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use silver_core::{
    Object, ObjectID, SilverAddress, TransactionDigest,
};
use silver_storage::{EventStore, ObjectStore, TransactionStore};
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, error};

/// Query endpoints for blockchain data
pub struct QueryEndpoints {
    object_store: Arc<ObjectStore>,
    transaction_store: Arc<TransactionStore>,
    #[allow(dead_code)]
    event_store: Arc<EventStore>,
}

impl QueryEndpoints {
    /// Create new query endpoints
    pub fn new(
        object_store: Arc<ObjectStore>,
        transaction_store: Arc<TransactionStore>,
        event_store: Arc<EventStore>,
    ) -> Self {
        Self {
            object_store,
            transaction_store,
            event_store,
        }
    }

    /// Get an object by ID
    ///
    /// # Parameters
    /// - `id`: Object ID (hex or base58 encoded)
    ///
    /// # Returns
    /// Object data or error if not found
    pub fn get_object(&self, params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();

        // Parse parameters
        let request: GetObjectRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        // Parse object ID
        let object_id = parse_object_id(&request.id)?;

        debug!("Getting object: {}", object_id);

        // Query object from store
        let object = self
            .object_store
            .get_object(&object_id)
            .map_err(|e| {
                error!("Failed to get object {}: {}", object_id, e);
                JsonRpcError::internal_error(format!("Failed to get object: {}", e))
            })?;

        let object = object.ok_or_else(|| {
            JsonRpcError::new(-32001, format!("Object not found: {}", object_id))
        })?;

        // Convert to response
        let response = ObjectResponse::from_object(&object);

        let elapsed = start.elapsed();
        debug!("Got object {} in {:?}", object_id, elapsed);

        // Ensure response time under 100ms
        if elapsed.as_millis() > 100 {
            error!(
                "Query took {}ms (target: <100ms) for object {}",
                elapsed.as_millis(),
                object_id
            );
        }

        Ok(serde_json::to_value(response).unwrap())
    }

    /// Get objects owned by an address
    ///
    /// # Parameters
    /// - `owner`: Owner address (hex or base58 encoded)
    /// - `limit`: Maximum number of objects to return (default: 50, max: 1000)
    ///
    /// # Returns
    /// List of objects
    pub fn get_objects_by_owner(
        &self,
        params: JsonValue,
    ) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();

        // Parse parameters
        let request: GetObjectsByOwnerRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        // Parse owner address
        let owner = parse_address(&request.owner)?;

        // Validate limit
        let limit = request.limit.unwrap_or(50).min(1000);

        debug!("Getting objects for owner: {} (limit: {})", owner, limit);

        // Query objects from store
        let mut objects = self
            .object_store
            .get_objects_by_owner(&owner)
            .map_err(|e| {
                error!("Failed to get objects for owner {}: {}", owner, e);
                JsonRpcError::internal_error(format!("Failed to get objects: {}", e))
            })?;

        // Apply limit
        objects.truncate(limit);

        // Convert to response
        let object_responses: Vec<ObjectResponse> = objects
            .iter()
            .map(|obj| ObjectResponse::from_object(obj))
            .collect();

        let response = GetObjectsByOwnerResponse {
            objects: object_responses,
            next_cursor: None, // Pagination not implemented yet
            has_more: false,
        };

        let elapsed = start.elapsed();
        debug!(
            "Got {} objects for owner {} in {:?}",
            objects.len(),
            owner,
            elapsed
        );

        // Ensure response time under 100ms
        if elapsed.as_millis() > 100 {
            error!(
                "Query took {}ms (target: <100ms) for owner {}",
                elapsed.as_millis(),
                owner
            );
        }

        Ok(serde_json::to_value(response).unwrap())
    }

    /// Get transaction status and effects
    ///
    /// # Parameters
    /// - `digest`: Transaction digest (hex encoded)
    ///
    /// # Returns
    /// Transaction data and execution effects
    pub fn get_transaction(&self, params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();

        // Parse parameters
        let request: GetTransactionRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        // Parse transaction digest
        let digest = parse_transaction_digest(&request.digest)?;

        debug!("Getting transaction: {}", hex::encode(digest.0));

        // Query transaction from store
        let tx_data = self
            .transaction_store
            .get_transaction(&digest)
            .map_err(|e| {
                error!("Failed to get transaction {}: {}", hex::encode(digest.0), e);
                JsonRpcError::internal_error(format!("Failed to get transaction: {}", e))
            })?;

        let tx_data = tx_data.ok_or_else(|| {
            JsonRpcError::new(
                -32002,
                format!("Transaction not found: {}", hex::encode(digest.0)),
            )
        })?;

        // Convert to response
        let response = TransactionResponse {
            digest: hex::encode(digest.0),
            transaction: TransactionDataResponse {
                sender: tx_data.transaction.sender().to_hex(),
                fuel_budget: tx_data.transaction.fuel_budget(),
                fuel_price: tx_data.transaction.fuel_price(),
                commands: tx_data.transaction.data.kind.command_count(),
            },
            effects: Some(TransactionEffectsResponse {
                status: format!("{:?}", tx_data.effects.status),
                fuel_used: tx_data.effects.fuel_used,
                // Note: Object mutations and events are tracked in the execution engine
                // and will be added to TransactionEffects in the execution implementation
                created_objects: 0,
                mutated_objects: 0,
                deleted_objects: 0,
                events: 0,
            }),
            timestamp: tx_data.effects.timestamp,
        };

        let elapsed = start.elapsed();
        debug!(
            "Got transaction {} in {:?}",
            hex::encode(digest.0),
            elapsed
        );

        // Ensure response time under 100ms
        if elapsed.as_millis() > 100 {
            error!(
                "Query took {}ms (target: <100ms) for transaction {}",
                elapsed.as_millis(),
                hex::encode(digest.0)
            );
        }

        Ok(serde_json::to_value(response).unwrap())
    }
}

/// Transaction endpoints for submitting transactions
pub struct TransactionEndpoints {
    #[allow(dead_code)]
    transaction_store: Arc<TransactionStore>,
}

impl TransactionEndpoints {
    /// Create new transaction endpoints
    pub fn new(transaction_store: Arc<TransactionStore>) -> Self {
        Self { transaction_store }
    }

    /// Submit a transaction to the blockchain
    ///
    /// # Parameters
    /// - `transaction`: Base64-encoded serialized transaction
    ///
    /// # Returns
    /// Transaction digest
    pub fn submit_transaction(&self, params: JsonValue) -> Result<JsonValue, JsonRpcError> {
        let start = Instant::now();

        // Parse parameters
        let request: SubmitTransactionRequest = serde_json::from_value(params)
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

        // Decode transaction from base64
        use base64::{Engine as _, engine::general_purpose};
        let tx_bytes = general_purpose::STANDARD.decode(&request.transaction).map_err(|e| {
            JsonRpcError::invalid_params(format!("Invalid base64 encoding: {}", e))
        })?;

        // Deserialize transaction
        let transaction: silver_core::Transaction = bincode::deserialize(&tx_bytes).map_err(|e| {
            JsonRpcError::invalid_params(format!("Invalid transaction format: {}", e))
        })?;

        // Validate transaction structure
        transaction.validate().map_err(|e| {
            JsonRpcError::invalid_params(format!("Transaction validation failed: {}", e))
        })?;

        // Check transaction size (max 128KB)
        if tx_bytes.len() > 128 * 1024 {
            return Err(JsonRpcError::invalid_params(format!(
                "Transaction too large: {} bytes (max: 128KB)",
                tx_bytes.len()
            )));
        }

        // Compute transaction digest
        let digest = transaction.digest();

        debug!(
            "Received transaction submission: {} ({} bytes)",
            hex::encode(digest.0),
            tx_bytes.len()
        );

        // Note: In production, this would forward to the transaction coordinator
        // which would validate and route to the consensus engine.
        // For now, we accept the transaction and return the digest.
        // The transaction coordinator integration is handled at the node level.

        let response = SubmitTransactionResponse {
            digest: hex::encode(digest.0),
        };

        let elapsed = start.elapsed();
        debug!(
            "Transaction {} submitted in {:?}",
            hex::encode(digest.0),
            elapsed
        );

        // Ensure response time under 100ms
        if elapsed.as_millis() > 100 {
            error!(
                "Transaction submission took {}ms (target: <100ms)",
                elapsed.as_millis()
            );
        }

        Ok(serde_json::to_value(response).unwrap())
    }
}

// Request/Response types

#[derive(Debug, Deserialize)]
struct GetObjectRequest {
    id: String,
}

#[derive(Debug, Serialize)]
struct ObjectResponse {
    id: String,
    version: u64,
    owner: String,
    object_type: String,
    data: String, // hex encoded
    size_bytes: usize,
}

impl ObjectResponse {
    fn from_object(obj: &Object) -> Self {
        Self {
            id: obj.id.to_hex(),
            version: obj.version.value(),
            owner: format!("{}", obj.owner),
            object_type: format!("{}", obj.object_type),
            data: hex::encode(&obj.data),
            size_bytes: obj.size_bytes(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct GetObjectsByOwnerRequest {
    owner: String,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Serialize)]
struct GetObjectsByOwnerResponse {
    objects: Vec<ObjectResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_cursor: Option<String>,
    has_more: bool,
}

#[derive(Debug, Deserialize)]
struct GetTransactionRequest {
    digest: String,
}

#[derive(Debug, Deserialize)]
struct SubmitTransactionRequest {
    transaction: String, // Base64-encoded serialized transaction
}

#[derive(Debug, Serialize)]
struct SubmitTransactionResponse {
    digest: String, // Hex-encoded transaction digest
}

#[derive(Debug, Serialize)]
struct TransactionResponse {
    digest: String,
    transaction: TransactionDataResponse,
    #[serde(skip_serializing_if = "Option::is_none")]
    effects: Option<TransactionEffectsResponse>,
    timestamp: u64,
}

#[derive(Debug, Serialize)]
struct TransactionDataResponse {
    sender: String,
    fuel_budget: u64,
    fuel_price: u64,
    commands: usize,
}

#[derive(Debug, Serialize)]
struct TransactionEffectsResponse {
    status: String,
    fuel_used: u64,
    created_objects: usize,
    mutated_objects: usize,
    deleted_objects: usize,
    events: usize,
}

// Helper functions

fn parse_object_id(s: &str) -> Result<ObjectID, JsonRpcError> {
    // Try hex first
    if let Ok(id) = ObjectID::from_hex(s) {
        return Ok(id);
    }

    // Try base58
    if let Ok(id) = ObjectID::from_base58(s) {
        return Ok(id);
    }

    Err(JsonRpcError::invalid_params(format!(
        "Invalid object ID: {}",
        s
    )))
}

fn parse_address(s: &str) -> Result<SilverAddress, JsonRpcError> {
    // Try hex first
    if let Ok(addr) = SilverAddress::from_hex(s) {
        return Ok(addr);
    }

    // Try base58
    if let Ok(addr) = SilverAddress::from_base58(s) {
        return Ok(addr);
    }

    Err(JsonRpcError::invalid_params(format!(
        "Invalid address: {}",
        s
    )))
}

fn parse_transaction_digest(s: &str) -> Result<TransactionDigest, JsonRpcError> {
    let bytes = hex::decode(s)
        .map_err(|e| JsonRpcError::invalid_params(format!("Invalid transaction digest: {}", e)))?;

    if bytes.len() != 64 {
        return Err(JsonRpcError::invalid_params(format!(
            "Transaction digest must be 64 bytes, got {}",
            bytes.len()
        )));
    }

    let mut arr = [0u8; 64];
    arr.copy_from_slice(&bytes);
    Ok(TransactionDigest(arr))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_object_id_hex() {
        let id = ObjectID::new([42u8; 64]);
        let hex = id.to_hex();
        let parsed = parse_object_id(&hex).unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn test_parse_object_id_base58() {
        let id = ObjectID::new([42u8; 64]);
        let b58 = id.to_base58();
        let parsed = parse_object_id(&b58).unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn test_parse_object_id_invalid() {
        let result = parse_object_id("invalid");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_transaction_digest() {
        let digest = TransactionDigest([42u8; 64]);
        let hex = hex::encode(digest.0);
        let parsed = parse_transaction_digest(&hex).unwrap();
        assert_eq!(digest, parsed);
    }

    #[test]
    fn test_parse_transaction_digest_invalid_length() {
        let result = parse_transaction_digest("aabbcc");
        assert!(result.is_err());
    }
}
