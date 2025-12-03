//! JSON-RPC 2.0 server implementation
//!
//! Provides HTTP and WebSocket endpoints for blockchain interaction.

use crate::rate_limit::RateLimiter;
use axum::{
    extract::{ConnectInfo, State, WebSocketUpgrade},
    http::{header, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use jsonrpsee::server::ServerHandle;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::{net::SocketAddr, sync::Arc};

use tower_http::cors::{Any, CorsLayer};
use tracing::{debug, error, info, warn};

/// RPC server configuration
#[derive(Debug, Clone)]
pub struct RpcConfig {
    /// HTTP server bind address
    pub http_addr: SocketAddr,

    /// WebSocket server bind address  
    pub ws_addr: SocketAddr,

    /// Maximum request size in bytes (default 128KB)
    pub max_request_size: u32,

    /// Maximum response size in bytes (default 10MB)
    pub max_response_size: u32,

    /// Maximum concurrent connections
    pub max_connections: u32,

    /// Enable CORS
    pub enable_cors: bool,

    /// Rate limit per IP (requests per second)
    pub rate_limit_per_ip: u32,
}

impl Default for RpcConfig {
    fn default() -> Self {
        Self {
            http_addr: "127.0.0.1:9000".parse().unwrap(),
            ws_addr: "127.0.0.1:9001".parse().unwrap(),
            max_request_size: 128 * 1024,        // 128KB
            max_response_size: 10 * 1024 * 1024, // 10MB
            max_connections: 1000,
            enable_cors: true,
            rate_limit_per_ip: 100, // 100 req/s per IP
        }
    }
}

/// JSON-RPC 2.0 request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    /// JSON-RPC version (must be "2.0")
    pub jsonrpc: String,

    /// Method name
    pub method: String,

    /// Parameters (can be array or object)
    #[serde(default)]
    pub params: JsonValue,

    /// Request ID (can be string, number, or null)
    pub id: JsonValue,
}

/// JSON-RPC 2.0 response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    /// JSON-RPC version
    pub jsonrpc: String,

    /// Result (present on success)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<JsonValue>,

    /// Error (present on failure)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,

    /// Request ID
    pub id: JsonValue,
}

/// JSON-RPC 2.0 error
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    /// Error code
    pub code: i32,

    /// Error message
    pub message: String,

    /// Additional error data
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<JsonValue>,
}

impl JsonRpcError {
    /// Create a new error
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    /// Create error with data
    pub fn with_data(code: i32, message: impl Into<String>, data: JsonValue) -> Self {
        Self {
            code,
            message: message.into(),
            data: Some(data),
        }
    }

    /// Parse error (-32700)
    pub fn parse_error() -> Self {
        Self::new(-32700, "Parse error")
    }

    /// Invalid request (-32600)
    pub fn invalid_request() -> Self {
        Self::new(-32600, "Invalid request")
    }

    /// Method not found (-32601)
    pub fn method_not_found() -> Self {
        Self::new(-32601, "Method not found")
    }

    /// Invalid params (-32602)
    pub fn invalid_params(msg: impl Into<String>) -> Self {
        Self::new(-32602, msg)
    }

    /// Internal error (-32603)
    pub fn internal_error(msg: impl Into<String>) -> Self {
        Self::new(-32603, msg)
    }

    /// Rate limit exceeded (-32000)
    pub fn rate_limit_exceeded() -> Self {
        Self::new(-32000, "Rate limit exceeded")
    }
}

/// Batch JSON-RPC request
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BatchRequest {
    /// Single request
    Single(JsonRpcRequest),

    /// Batch of requests (max 50)
    Batch(Vec<JsonRpcRequest>),
}

/// Batch JSON-RPC response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BatchResponse {
    /// Single response
    Single(JsonRpcResponse),

    /// Batch of responses
    Batch(Vec<JsonRpcResponse>),
}

/// RPC server state shared across handlers
#[derive(Clone)]
pub struct RpcServerState {
    /// Rate limiter
    pub rate_limiter: Arc<RateLimiter>,

    /// Server configuration
    pub config: RpcConfig,

    /// Query endpoints
    pub query_endpoints: Option<Arc<crate::endpoints::QueryEndpoints>>,

    /// Transaction endpoints
    pub transaction_endpoints: Option<Arc<crate::endpoints::TransactionEndpoints>>,

    /// Explorer endpoints
    pub explorer_endpoints: Option<Arc<crate::explorer_endpoints::ExplorerEndpoints>>,

    /// Validator endpoints
    pub validator_endpoints: Option<Arc<crate::validator_endpoints::ValidatorEndpoints>>,

    /// Subscription manager
    pub subscription_manager: Option<Arc<crate::subscriptions::SubscriptionManager>>,
}

impl RpcServerState {
    /// Create new server state
    pub fn new(config: RpcConfig) -> Self {
        Self {
            rate_limiter: Arc::new(RateLimiter::new(config.rate_limit_per_ip)),
            config,
            query_endpoints: None,
            transaction_endpoints: None,
            explorer_endpoints: None,
            validator_endpoints: None,
            subscription_manager: None,
        }
    }

    /// Set query endpoints
    pub fn with_query_endpoints(
        mut self,
        endpoints: Arc<crate::endpoints::QueryEndpoints>,
    ) -> Self {
        self.query_endpoints = Some(endpoints);
        self
    }

    /// Set transaction endpoints
    pub fn with_transaction_endpoints(
        mut self,
        endpoints: Arc<crate::endpoints::TransactionEndpoints>,
    ) -> Self {
        self.transaction_endpoints = Some(endpoints);
        self
    }

    /// Set explorer endpoints
    pub fn with_explorer_endpoints(
        mut self,
        endpoints: Arc<crate::explorer_endpoints::ExplorerEndpoints>,
    ) -> Self {
        self.explorer_endpoints = Some(endpoints);
        self
    }

    /// Set validator endpoints
    pub fn with_validator_endpoints(
        mut self,
        endpoints: Arc<crate::validator_endpoints::ValidatorEndpoints>,
    ) -> Self {
        self.validator_endpoints = Some(endpoints);
        self
    }

    /// Set subscription manager
    pub fn with_subscription_manager(
        mut self,
        manager: Arc<crate::subscriptions::SubscriptionManager>,
    ) -> Self {
        self.subscription_manager = Some(manager);
        self
    }
}

/// Main RPC server
pub struct RpcServer {
    config: RpcConfig,
    state: Arc<RpcServerState>,
    http_handle: Option<ServerHandle>,
}

impl RpcServer {
    /// Create a new RPC server
    pub fn new(config: RpcConfig) -> Self {
        let state = Arc::new(RpcServerState::new(config.clone()));

        Self {
            config,
            state,
            http_handle: None,
        }
    }

    /// Create a new RPC server with endpoints
    pub fn with_endpoints(
        config: RpcConfig,
        query_endpoints: Arc<crate::endpoints::QueryEndpoints>,
        transaction_endpoints: Arc<crate::endpoints::TransactionEndpoints>,
    ) -> Self {
        let state = Arc::new(
            RpcServerState::new(config.clone())
                .with_query_endpoints(query_endpoints)
                .with_transaction_endpoints(transaction_endpoints),
        );

        Self {
            config,
            state,
            http_handle: None,
        }
    }

    /// Create a new RPC server with all endpoints
    pub fn with_all_endpoints(
        config: RpcConfig,
        query_endpoints: Arc<crate::endpoints::QueryEndpoints>,
        transaction_endpoints: Arc<crate::endpoints::TransactionEndpoints>,
        explorer_endpoints: Arc<crate::explorer_endpoints::ExplorerEndpoints>,
    ) -> Self {
        let state = Arc::new(
            RpcServerState::new(config.clone())
                .with_query_endpoints(query_endpoints)
                .with_transaction_endpoints(transaction_endpoints)
                .with_explorer_endpoints(explorer_endpoints),
        );

        Self {
            config,
            state,
            http_handle: None,
        }
    }

    /// Create a new RPC server with all endpoints including validators
    pub fn with_all_endpoints_and_validators(
        config: RpcConfig,
        query_endpoints: Arc<crate::endpoints::QueryEndpoints>,
        transaction_endpoints: Arc<crate::endpoints::TransactionEndpoints>,
        explorer_endpoints: Arc<crate::explorer_endpoints::ExplorerEndpoints>,
        validator_endpoints: Arc<crate::validator_endpoints::ValidatorEndpoints>,
    ) -> Self {
        let state = Arc::new(
            RpcServerState::new(config.clone())
                .with_query_endpoints(query_endpoints)
                .with_transaction_endpoints(transaction_endpoints)
                .with_explorer_endpoints(explorer_endpoints)
                .with_validator_endpoints(validator_endpoints),
        );

        Self {
            config,
            state,
            http_handle: None,
        }
    }

    /// Start the HTTP server
    pub async fn start_http(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        info!("Starting HTTP JSON-RPC server on {}", self.config.http_addr);

        // Build CORS layer
        let cors = if self.config.enable_cors {
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
                .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION])
                .max_age(std::time::Duration::from_secs(3600))
        } else {
            CorsLayer::permissive()
        };

        // Build router
        let app = Router::new()
            .route("/", post(handle_json_rpc))
            .route("/health", get(handle_health))
            .layer(cors)
            .with_state(self.state.clone());

        // Start server
        let listener = tokio::net::TcpListener::bind(self.config.http_addr).await?;
        info!("HTTP server listening on {}", self.config.http_addr);

        tokio::spawn(async move {
            if let Err(e) = axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            {
                error!("HTTP server error: {}", e);
            }
        });

        Ok(())
    }

    /// Start the WebSocket server
    pub async fn start_websocket(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        info!(
            "Starting WebSocket JSON-RPC server on {}",
            self.config.ws_addr
        );

        // Build CORS layer
        let cors = if self.config.enable_cors {
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([Method::GET, Method::OPTIONS])
                .allow_headers([
                    header::CONTENT_TYPE,
                    header::AUTHORIZATION,
                    header::SEC_WEBSOCKET_PROTOCOL,
                ])
                .max_age(std::time::Duration::from_secs(3600))
        } else {
            CorsLayer::permissive()
        };

        // Build router
        let app = Router::new()
            .route("/", get(handle_websocket_upgrade))
            .layer(cors)
            .with_state(self.state.clone());

        // Start server
        let listener = tokio::net::TcpListener::bind(self.config.ws_addr).await?;
        info!("WebSocket server listening on {}", self.config.ws_addr);

        tokio::spawn(async move {
            if let Err(e) = axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            {
                error!("WebSocket server error: {}", e);
            }
        });

        Ok(())
    }

    /// Start both HTTP and WebSocket servers
    pub async fn start(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.start_http().await?;
        self.start_websocket().await?;
        Ok(())
    }

    /// Stop the server
    pub async fn stop(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(handle) = self.http_handle.take() {
            handle.stop()?;
        }
        Ok(())
    }

    /// Get the HTTP address
    pub fn http_addr(&self) -> SocketAddr {
        self.config.http_addr
    }

    /// Get the WebSocket address
    pub fn ws_addr(&self) -> SocketAddr {
        self.config.ws_addr
    }
}

/// Handle JSON-RPC requests over HTTP
async fn handle_json_rpc(
    State(state): State<Arc<RpcServerState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(request): Json<BatchRequest>,
) -> Response {
    // Check rate limit
    if !state.rate_limiter.check_rate_limit(addr.ip()).await {
        warn!("Rate limit exceeded for {}", addr.ip());
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                result: None,
                error: Some(JsonRpcError::rate_limit_exceeded()),
                id: JsonValue::Null,
            }),
        )
            .into_response();
    }

    // Process request
    let response = match request {
        BatchRequest::Single(req) => {
            let resp = process_single_request(req, &state).await;
            BatchResponse::Single(resp)
        }
        BatchRequest::Batch(reqs) => {
            // Validate batch size (max 50)
            if reqs.len() > 50 {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(JsonRpcResponse {
                        jsonrpc: "2.0".to_string(),
                        result: None,
                        error: Some(JsonRpcError::invalid_request()),
                        id: JsonValue::Null,
                    }),
                )
                    .into_response();
            }

            // Process all requests
            let mut responses = Vec::new();
            for req in reqs {
                responses.push(process_single_request(req, &state).await);
            }
            BatchResponse::Batch(responses)
        }
    };

    Json(response).into_response()
}

/// Process a single JSON-RPC request
async fn process_single_request(
    request: JsonRpcRequest,
    state: &Arc<RpcServerState>,
) -> JsonRpcResponse {
    // Validate JSON-RPC version
    if request.jsonrpc != "2.0" {
        return JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            result: None,
            error: Some(JsonRpcError::invalid_request()),
            id: request.id,
        };
    }

    // Route to appropriate handler
    let result = match request.method.as_str() {
        // Query methods - Objects
        "silver_getObject" => {
            if let Some(ref endpoints) = state.query_endpoints {
                endpoints.silver_get_object(request.params)
            } else {
                Err(JsonRpcError::internal_error(
                    "Query endpoints not initialized",
                ))
            }
        }
        "silver_getObjectsByOwner" => {
            if let Some(ref endpoints) = state.query_endpoints {
                endpoints.silver_get_objects_by_owner(request.params)
            } else {
                Err(JsonRpcError::internal_error(
                    "Query endpoints not initialized",
                ))
            }
        }

        // Query methods - Transactions
        "silver_getTransaction" => {
            if let Some(ref endpoints) = state.query_endpoints {
                endpoints.silver_get_transaction(request.params)
            } else {
                Err(JsonRpcError::internal_error(
                    "Query endpoints not initialized",
                ))
            }
        }

        // Query methods - Balance and Account
        "silver_getBalance" => {
            if let Some(ref endpoints) = state.query_endpoints {
                endpoints.silver_get_balance(request.params)
            } else {
                Err(JsonRpcError::internal_error(
                    "Query endpoints not initialized",
                ))
            }
        }

        // Query methods - Gas estimation
        "silver_estimateGas" => {
            if let Some(ref endpoints) = state.query_endpoints {
                endpoints.silver_estimate_gas(request.params)
            } else {
                Err(JsonRpcError::internal_error(
                    "Query endpoints not initialized",
                ))
            }
        }

        // Query methods - Blocks
        "silver_getLatestBlockNumber" => {
            if let Some(ref endpoints) = state.query_endpoints {
                endpoints.silver_get_latest_block_number()
            } else {
                Err(JsonRpcError::internal_error(
                    "Query endpoints not initialized",
                ))
            }
        }
        "silver_getBlockByNumber" => {
            if let Some(ref endpoints) = state.query_endpoints {
                endpoints.silver_get_block_by_number(request.params)
            } else {
                Err(JsonRpcError::internal_error(
                    "Query endpoints not initialized",
                ))
            }
        }

        // Query methods - Validators and Network
        "silver_getValidators" => {
            if let Some(ref endpoints) = state.query_endpoints {
                endpoints.silver_get_validators()
            } else {
                Err(JsonRpcError::internal_error(
                    "Query endpoints not initialized",
                ))
            }
        }
        "silver_getNetworkStats" => {
            if let Some(ref endpoints) = state.query_endpoints {
                endpoints.silver_get_network_stats()
            } else {
                Err(JsonRpcError::internal_error(
                    "Query endpoints not initialized",
                ))
            }
        }

        // Query methods - Transaction queries
        "silver_getTransactionsByAddress" => {
            if let Some(ref endpoints) = state.query_endpoints {
                endpoints.silver_get_transactions_by_address(request.params)
            } else {
                Err(JsonRpcError::internal_error(
                    "Query endpoints not initialized",
                ))
            }
        }
        "silver_getGasPrice" => {
            if let Some(ref endpoints) = state.query_endpoints {
                endpoints.silver_get_gas_price()
            } else {
                Err(JsonRpcError::internal_error(
                    "Query endpoints not initialized",
                ))
            }
        }
        "silver_getTransactionReceipt" => {
            if let Some(ref endpoints) = state.query_endpoints {
                endpoints.silver_get_transaction_receipt(request.params)
            } else {
                Err(JsonRpcError::internal_error(
                    "Query endpoints not initialized",
                ))
            }
        }
        "silver_getTransactionByHash" => {
            if let Some(ref endpoints) = state.query_endpoints {
                endpoints.silver_get_transaction_by_hash(request.params)
            } else {
                Err(JsonRpcError::internal_error(
                    "Query endpoints not initialized",
                ))
            }
        }

        // Query methods - Code and Account
        "silver_getCode" => {
            if let Some(ref endpoints) = state.query_endpoints {
                endpoints.silver_get_code(request.params)
            } else {
                Err(JsonRpcError::internal_error(
                    "Query endpoints not initialized",
                ))
            }
        }
        "silver_getTransactionCount" => {
            if let Some(ref endpoints) = state.query_endpoints {
                endpoints.silver_get_transaction_count(request.params)
            } else {
                Err(JsonRpcError::internal_error(
                    "Query endpoints not initialized",
                ))
            }
        }

        // Query methods - Events
        "silver_getEvents" => {
            if let Some(ref endpoints) = state.query_endpoints {
                endpoints.silver_get_events(request.params)
            } else {
                Err(JsonRpcError::internal_error(
                    "Query endpoints not initialized",
                ))
            }
        }
        "silver_queryEvents" => {
            if let Some(ref endpoints) = state.query_endpoints {
                endpoints.silver_query_events(request.params)
            } else {
                Err(JsonRpcError::internal_error(
                    "Query endpoints not initialized",
                ))
            }
        }

        // Query methods - Checkpoints
        "silver_getCheckpoint" => {
            if let Some(ref endpoints) = state.query_endpoints {
                endpoints.silver_get_checkpoint(request.params)
            } else {
                Err(JsonRpcError::internal_error(
                    "Query endpoints not initialized",
                ))
            }
        }
        "silver_getLatestCheckpoint" => {
            if let Some(ref endpoints) = state.query_endpoints {
                endpoints.silver_get_latest_checkpoint()
            } else {
                Err(JsonRpcError::internal_error(
                    "Query endpoints not initialized",
                ))
            }
        }

        // Query methods - Object ownership
        "silver_getObjectsOwnedByAddress" => {
            if let Some(ref endpoints) = state.query_endpoints {
                endpoints.silver_get_objects_owned_by_address(request.params)
            } else {
                Err(JsonRpcError::internal_error(
                    "Query endpoints not initialized",
                ))
            }
        }
        "silver_getObjectsOwnedByObject" => {
            if let Some(ref endpoints) = state.query_endpoints {
                endpoints.silver_get_objects_owned_by_object(request.params)
            } else {
                Err(JsonRpcError::internal_error(
                    "Query endpoints not initialized",
                ))
            }
        }

        // Query methods - Object history
        "silver_getObjectHistory" => {
            if let Some(ref endpoints) = state.query_endpoints {
                endpoints.silver_get_object_history(request.params)
            } else {
                Err(JsonRpcError::internal_error(
                    "Query endpoints not initialized",
                ))
            }
        }

        // Transaction submission
        "silver_submitTransaction" => {
            if let Some(ref endpoints) = state.transaction_endpoints {
                endpoints.silver_submit_transaction(request.params)
            } else {
                Err(JsonRpcError::internal_error(
                    "Transaction endpoints not initialized",
                ))
            }
        }
        "silver_dryRunTransaction" => {
            if let Some(ref endpoints) = state.transaction_endpoints {
                endpoints.silver_dry_run_transaction(request.params)
            } else {
                Err(JsonRpcError::internal_error(
                    "Transaction endpoints not initialized",
                ))
            }
        }

        // Explorer methods - Account information
        "silver_getAccountInfo" => {
            if let Some(ref endpoints) = state.explorer_endpoints {
                endpoints.silver_get_account_info(request.params)
            } else {
                Err(JsonRpcError::internal_error(
                    "Explorer endpoints not initialized",
                ))
            }
        }
        "silver_getMultipleAccounts" => {
            if let Some(ref endpoints) = state.explorer_endpoints {
                endpoints.silver_get_multiple_accounts(request.params)
            } else {
                Err(JsonRpcError::internal_error(
                    "Explorer endpoints not initialized",
                ))
            }
        }
        "silver_getTransactionHistory" => {
            if let Some(ref endpoints) = state.explorer_endpoints {
                endpoints.silver_get_transaction_history(request.params)
            } else {
                Err(JsonRpcError::internal_error(
                    "Explorer endpoints not initialized",
                ))
            }
        }

        // Explorer methods - Token information
        "silver_getTokenSupply" => {
            if let Some(ref endpoints) = state.explorer_endpoints {
                endpoints.silver_get_token_supply()
            } else {
                Err(JsonRpcError::internal_error(
                    "Explorer endpoints not initialized",
                ))
            }
        }
        "silver_getTokenLargestAccounts" => {
            if let Some(ref endpoints) = state.explorer_endpoints {
                endpoints.silver_get_token_largest_accounts()
            } else {
                Err(JsonRpcError::internal_error(
                    "Explorer endpoints not initialized",
                ))
            }
        }
        "silver_getTokenAccountsByOwner" => {
            if let Some(ref endpoints) = state.explorer_endpoints {
                endpoints.silver_get_token_accounts_by_owner(request.params)
            } else {
                Err(JsonRpcError::internal_error(
                    "Explorer endpoints not initialized",
                ))
            }
        }

        // Explorer methods - Program accounts
        "silver_getProgramAccounts" => {
            if let Some(ref endpoints) = state.explorer_endpoints {
                endpoints.silver_get_program_accounts(request.params)
            } else {
                Err(JsonRpcError::internal_error(
                    "Explorer endpoints not initialized",
                ))
            }
        }

        // Explorer methods - Transaction status
        "silver_getSignatureStatuses" => {
            if let Some(ref endpoints) = state.explorer_endpoints {
                endpoints.silver_get_signature_statuses(request.params)
            } else {
                Err(JsonRpcError::internal_error(
                    "Explorer endpoints not initialized",
                ))
            }
        }

        // Explorer methods - Block information
        "silver_getBlockTime" => {
            if let Some(ref endpoints) = state.explorer_endpoints {
                endpoints.silver_get_block_time(request.params)
            } else {
                Err(JsonRpcError::internal_error(
                    "Explorer endpoints not initialized",
                ))
            }
        }

        // Explorer methods - Slot information
        "silver_getSlot" => {
            if let Some(ref endpoints) = state.explorer_endpoints {
                endpoints.silver_get_slot()
            } else {
                Err(JsonRpcError::internal_error(
                    "Explorer endpoints not initialized",
                ))
            }
        }
        "silver_getLeaderSchedule" => {
            if let Some(ref endpoints) = state.explorer_endpoints {
                endpoints.silver_get_leader_schedule()
            } else {
                Err(JsonRpcError::internal_error(
                    "Explorer endpoints not initialized",
                ))
            }
        }

        // Explorer methods - Cluster information
        "silver_getClusterNodes" => {
            if let Some(ref endpoints) = state.explorer_endpoints {
                endpoints.silver_get_cluster_nodes()
            } else {
                Err(JsonRpcError::internal_error(
                    "Explorer endpoints not initialized",
                ))
            }
        }
        "silver_getHealth" => {
            if let Some(ref endpoints) = state.explorer_endpoints {
                endpoints.silver_get_health()
            } else {
                Err(JsonRpcError::internal_error(
                    "Explorer endpoints not initialized",
                ))
            }
        }
        "silver_getVersion" => {
            if let Some(ref endpoints) = state.explorer_endpoints {
                endpoints.silver_get_version()
            } else {
                Err(JsonRpcError::internal_error(
                    "Explorer endpoints not initialized",
                ))
            }
        }
        "silver_getGenesisHash" => {
            if let Some(ref endpoints) = state.explorer_endpoints {
                endpoints.silver_get_genesis_hash()
            } else {
                Err(JsonRpcError::internal_error(
                    "Explorer endpoints not initialized",
                ))
            }
        }

        // Explorer methods - Inflation and supply
        "silver_getInflationRate" => {
            if let Some(ref endpoints) = state.explorer_endpoints {
                endpoints.silver_get_inflation_rate()
            } else {
                Err(JsonRpcError::internal_error(
                    "Explorer endpoints not initialized",
                ))
            }
        }
        "silver_getRecentPerformanceSamples" => {
            if let Some(ref endpoints) = state.explorer_endpoints {
                endpoints.silver_get_recent_performance_samples()
            } else {
                Err(JsonRpcError::internal_error(
                    "Explorer endpoints not initialized",
                ))
            }
        }
        "silver_getSupply" => {
            if let Some(ref endpoints) = state.explorer_endpoints {
                endpoints.silver_get_supply()
            } else {
                Err(JsonRpcError::internal_error(
                    "Explorer endpoints not initialized",
                ))
            }
        }

        // Explorer methods - Rent and fees
        "silver_getMinimumBalanceForRentExemption" => {
            if let Some(ref endpoints) = state.explorer_endpoints {
                endpoints.silver_get_minimum_balance_for_rent_exemption(request.params)
            } else {
                Err(JsonRpcError::internal_error(
                    "Explorer endpoints not initialized",
                ))
            }
        }
        "silver_getFeeForMessage" => {
            if let Some(ref endpoints) = state.explorer_endpoints {
                endpoints.silver_get_fee_for_message(request.params)
            } else {
                Err(JsonRpcError::internal_error(
                    "Explorer endpoints not initialized",
                ))
            }
        }

        // Explorer methods - Identity and stake
        "silver_getIdentity" => {
            if let Some(ref endpoints) = state.explorer_endpoints {
                endpoints.silver_get_identity()
            } else {
                Err(JsonRpcError::internal_error(
                    "Explorer endpoints not initialized",
                ))
            }
        }
        "silver_getStakeActivation" => {
            if let Some(ref endpoints) = state.explorer_endpoints {
                endpoints.silver_get_stake_activation(request.params)
            } else {
                Err(JsonRpcError::internal_error(
                    "Explorer endpoints not initialized",
                ))
            }
        }
        "silver_getVoteAccounts" => {
            if let Some(ref endpoints) = state.explorer_endpoints {
                endpoints.silver_get_vote_accounts()
            } else {
                Err(JsonRpcError::internal_error(
                    "Explorer endpoints not initialized",
                ))
            }
        }

        // Explorer methods - Accounts and snapshots
        "silver_getLargestAccounts" => {
            if let Some(ref endpoints) = state.explorer_endpoints {
                endpoints.silver_get_largest_accounts()
            } else {
                Err(JsonRpcError::internal_error(
                    "Explorer endpoints not initialized",
                ))
            }
        }
        "silver_getHighestSnapshotSlot" => {
            if let Some(ref endpoints) = state.explorer_endpoints {
                endpoints.silver_get_highest_snapshot_slot()
            } else {
                Err(JsonRpcError::internal_error(
                    "Explorer endpoints not initialized",
                ))
            }
        }
        "silver_getFirstAvailableBlock" => {
            if let Some(ref endpoints) = state.explorer_endpoints {
                endpoints.silver_get_first_available_block()
            } else {
                Err(JsonRpcError::internal_error(
                    "Explorer endpoints not initialized",
                ))
            }
        }
        "silver_getMaxRetransmitSlot" => {
            if let Some(ref endpoints) = state.explorer_endpoints {
                endpoints.silver_get_max_retransmit_slot()
            } else {
                Err(JsonRpcError::internal_error(
                    "Explorer endpoints not initialized",
                ))
            }
        }
        "silver_getMaxShredInsertSlot" => {
            if let Some(ref endpoints) = state.explorer_endpoints {
                endpoints.silver_get_max_shred_insert_slot()
            } else {
                Err(JsonRpcError::internal_error(
                    "Explorer endpoints not initialized",
                ))
            }
        }

        // Validator methods - Information and monitoring
        "validator_getInfo"
        | "validator_getAllValidators"
        | "validator_getDelegationStatus"
        | "validator_getDelegations"
        | "validator_getValidatorDelegations"
        | "validator_claimRewards"
        | "validator_submitStake"
        | "validator_getRewardHistory"
        | "validator_getPerformanceMetrics"
        | "validator_getActiveAlerts"
        | "validator_getCriticalValidators"
        | "validator_getWarningValidators"
        | "validator_getHealthStatus" => {
            if let Some(ref endpoints) = state.validator_endpoints {
                // Convert params to array for validator endpoint handler
                let params_array: Vec<JsonValue> = match &request.params {
                    JsonValue::Array(arr) => arr.clone(),
                    JsonValue::Object(obj) => vec![JsonValue::Object(obj.clone())],
                    other => vec![other.clone()],
                };

                // Call validator endpoint handler
                match crate::validator_endpoints::handle_validator_rpc(
                    endpoints,
                    request.method.as_str(),
                    &params_array,
                )
                .await
                {
                    Ok(result) => Ok(result),
                    Err(e) => Err(JsonRpcError::internal_error(format!(
                        "Validator error: {}",
                        e
                    ))),
                }
            } else {
                Err(JsonRpcError::internal_error(
                    "Validator endpoints not initialized",
                ))
            }
        }

        _ => Err(JsonRpcError::method_not_found()),
    };

    match result {
        Ok(value) => JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            result: Some(value),
            error: None,
            id: request.id,
        },
        Err(error) => JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            result: None,
            error: Some(error),
            id: request.id,
        },
    }
}

/// Handle health check endpoint
async fn handle_health() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
        "service": "silverbitcoin-rpc",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// Handle WebSocket upgrade
async fn handle_websocket_upgrade(
    ws: WebSocketUpgrade,
    State(state): State<Arc<RpcServerState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> Response {
    // Check rate limit
    if !state.rate_limiter.check_rate_limit(addr.ip()).await {
        warn!(
            "Rate limit exceeded for WebSocket connection from {}",
            addr.ip()
        );
        return (StatusCode::TOO_MANY_REQUESTS, "Rate limit exceeded").into_response();
    }

    debug!("WebSocket connection from {}", addr);

    // Check if subscription manager is available
    if let Some(ref subscription_manager) = state.subscription_manager {
        let manager = Arc::clone(subscription_manager);
        // Upgrade to WebSocket and handle with subscription manager
        ws.on_upgrade(move |socket| async move {
            manager.handle_connection(socket, addr).await;
        })
    } else {
        // Fallback to legacy WebSocket handler for JSON-RPC
        ws.on_upgrade(move |socket| handle_websocket(socket, state, addr))
    }
}

/// Handle WebSocket connection
async fn handle_websocket(
    socket: axum::extract::ws::WebSocket,
    state: Arc<RpcServerState>,
    addr: SocketAddr,
) {
    use axum::extract::ws::Message;
    use futures::{SinkExt, StreamExt};

    let (mut sender, mut receiver) = socket.split();

    info!("WebSocket connection established from {}", addr);

    // Handle incoming messages
    while let Some(msg) = receiver.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                debug!("Received WebSocket message from {}: {}", addr, text);

                // Parse JSON-RPC request
                let request: Result<BatchRequest, _> = serde_json::from_str(&text);

                let response = match request {
                    Ok(BatchRequest::Single(req)) => {
                        let resp = process_single_request(req, &state).await;
                        BatchResponse::Single(resp)
                    }
                    Ok(BatchRequest::Batch(reqs)) => {
                        if reqs.len() > 50 {
                            BatchResponse::Single(JsonRpcResponse {
                                jsonrpc: "2.0".to_string(),
                                result: None,
                                error: Some(JsonRpcError::invalid_request()),
                                id: JsonValue::Null,
                            })
                        } else {
                            let mut responses = Vec::new();
                            for req in reqs {
                                responses.push(process_single_request(req, &state).await);
                            }
                            BatchResponse::Batch(responses)
                        }
                    }
                    Err(e) => {
                        warn!("Failed to parse WebSocket message: {}", e);
                        BatchResponse::Single(JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            result: None,
                            error: Some(JsonRpcError::parse_error()),
                            id: JsonValue::Null,
                        })
                    }
                };

                // Send response
                let response_text = serde_json::to_string(&response).unwrap();
                if let Err(e) = sender.send(Message::Text(response_text)).await {
                    error!("Failed to send WebSocket response: {}", e);
                    break;
                }
            }
            Ok(Message::Binary(_)) => {
                warn!("Received binary WebSocket message from {}, ignoring", addr);
            }
            Ok(Message::Ping(data)) => {
                if let Err(e) = sender.send(Message::Pong(data)).await {
                    error!("Failed to send pong: {}", e);
                    break;
                }
            }
            Ok(Message::Pong(_)) => {
                // Ignore pong messages
            }
            Ok(Message::Close(_)) => {
                info!("WebSocket connection closed by client {}", addr);
                break;
            }
            Err(e) => {
                error!("WebSocket error from {}: {}", addr, e);
                break;
            }
        }
    }

    info!("WebSocket connection closed from {}", addr);
}
