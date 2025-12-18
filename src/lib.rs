//! # SilverBitcoin API
//!
//! JSON-RPC API gateway for blockchain interaction.
//!
//! This crate provides:
//! - JSON-RPC 2.0 server (HTTP and WebSocket)
//! - Query endpoints (objects, transactions, events)
//! - Transaction submission
//! - Rate limiting (100 req/s per IP)
//! - Batch request support (up to 50 requests)
//!
//! ## Example Usage
//!
//! ```no_run
//! use silver_api::{RpcServer, RpcConfig, QueryEndpoints, TransactionEndpoints};
//! use silver_storage::{ObjectStore, TransactionStore, EventStore, ParityDatabase};
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Initialize storage with ParityDB
//! let db = Arc::new(ParityDatabase::open("./data")?);
//! let object_store = Arc::new(ObjectStore::new(Arc::clone(&db)));
//! let transaction_store = Arc::new(TransactionStore::new(Arc::clone(&db)));
//! let event_store = Arc::new(EventStore::new(Arc::clone(&db)));
//!
//! // Create endpoints
//! let query_endpoints = Arc::new(QueryEndpoints::new(
//!     object_store,
//!     transaction_store.clone(),
//!     event_store,
//! ));
//! let transaction_endpoints = Arc::new(TransactionEndpoints::new(transaction_store));
//!
//! // Create and start RPC server
//! let config = RpcConfig::default();
//! let mut server = RpcServer::with_endpoints(config, query_endpoints, transaction_endpoints);
//! server.start().await?;
//! # Ok(())
//! # }
//! ```

#![warn(missing_docs, rust_2018_idioms)]
#![forbid(unsafe_code)]

pub mod admin_rpc;
pub mod bridge_endpoints;
pub mod dex_endpoints;
pub mod endpoints;
pub mod ethereum_complete;
pub mod explorer_endpoints;
pub mod performance;
pub mod rate_limit;
pub mod rpc;
pub mod subscriptions;
/// Token RPC methods for token operations (creation, transfer, approval, etc.)
pub mod token_rpc;
pub mod validator_endpoints;
pub mod wallet;

pub use admin_rpc::{AdminEndpoints, PeerInfo, PeerManager};
pub use bridge_endpoints::BridgeEndpoints;
pub use dex_endpoints::DexEndpoints;
pub use endpoints::{EthereumEndpoints, QueryEndpoints, TransactionEndpoints};
pub use ethereum_complete::{
    AccountStateManager, BlockHeader, BlockValidator, EthereumRpcHandler, FilterManager,
    GasCalculator, NonceManager, PendingTransaction, PendingTransactionPool, SubscriptionManager,
    TransactionReceipt, TransactionValidator,
};
pub use explorer_endpoints::ExplorerEndpoints;
pub use performance::{
    BatchProcessor, CacheStats, ConnectionPool, MemoryPool, PerformanceMetrics, PerformanceMonitor,
    PoolStats, QueryCache, QueryOptimizer,
};
pub use rate_limit::RateLimiter;
pub use rpc::{JsonRpcError, JsonRpcRequest, JsonRpcResponse, RpcConfig, RpcServer};
pub use subscriptions::{
    EventFilter, EventNotification, SubscribeRequest, SubscribeResponse, SubscriptionID,
    SubscriptionManager as SubscriptionManagerOld, UnsubscribeRequest,
};
pub use token_rpc::TokenRpcEndpoints;
pub use validator_endpoints::{
    handle_validator_rpc, AlertResponse, DelegationInfoResponse, HealthStatusResponse,
    PerformanceMetricsResponse, RewardClaimRequest, RewardClaimResponse, RewardHistoryResponse,
    StakeTransactionRequest, StakeTransactionResponse, ValidatorEndpoints, ValidatorInfoResponse,
};
pub use wallet::WalletEndpoints;
