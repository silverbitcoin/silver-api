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
//! use silver_storage::{ObjectStore, TransactionStore, EventStore, RocksDatabase};
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Initialize storage
//! let db = Arc::new(RocksDatabase::open("./data")?);
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

pub mod rpc;
pub mod endpoints;
pub mod rate_limit;
pub mod subscriptions;

pub use rpc::{RpcServer, RpcConfig, JsonRpcRequest, JsonRpcResponse, JsonRpcError};
pub use endpoints::{QueryEndpoints, TransactionEndpoints};
pub use rate_limit::RateLimiter;
pub use subscriptions::{
    SubscriptionManager, EventFilter, EventNotification, SubscriptionID,
    SubscribeRequest, SubscribeResponse, UnsubscribeRequest,
};
