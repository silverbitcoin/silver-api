//! JSON-RPC 2.0 server implementation
//!
//! Provides HTTP and WebSocket endpoints for blockchain interaction.

use crate::rate_limit::RateLimiter;
use axum::{
    extract::{ConnectInfo, State, WebSocketUpgrade},
    http::{header, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, options},
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

    /// CORS allowed origins (comma-separated, use * for all)
    pub cors_origins: String,

    /// CORS allowed methods (comma-separated)
    pub cors_methods: String,

    /// CORS allowed headers (comma-separated)
    pub cors_headers: String,

    /// CORS max age in seconds
    pub cors_max_age: u64,

    /// Rate limit per IP (requests per second)
    pub rate_limit_per_ip: u32,
}

impl Default for RpcConfig {
    fn default() -> Self {
        // These are hardcoded valid addresses, so parsing will always succeed
        // But we use expect() with a clear message for production safety
        Self {
            http_addr: "127.0.0.1:9000".parse()
                .expect("Invalid default HTTP address"),
            ws_addr: "127.0.0.1:9001".parse()
                .expect("Invalid default WebSocket address"),
            max_request_size: 128 * 1024,        // 128KB
            max_response_size: 10 * 1024 * 1024, // 10MB
            max_connections: 1000,
            enable_cors: true,
            cors_origins: "*".to_string(),
            cors_methods: "GET,POST,OPTIONS,PUT,DELETE".to_string(),
            cors_headers: "Content-Type,Authorization,X-Requested-With".to_string(),
            cors_max_age: 3600,
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

impl std::fmt::Display for JsonRpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for JsonRpcError {}

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
/// RPC server state shared across handlers
#[derive(Clone)]
pub struct RpcServerState {
    /// Rate limiter for request throttling
    pub rate_limiter: Arc<RateLimiter>,

    /// Server configuration
    pub config: RpcConfig,

    /// Query endpoints for blockchain data retrieval
    pub query_endpoints: Option<Arc<crate::endpoints::QueryEndpoints>>,

    /// Transaction endpoints for transaction submission
    pub transaction_endpoints: Option<Arc<crate::endpoints::TransactionEndpoints>>,

    /// Explorer endpoints for blockchain exploration
    pub explorer_endpoints: Option<Arc<crate::explorer_endpoints::ExplorerEndpoints>>,

    /// Validator endpoints for validator operations
    pub validator_endpoints: Option<Arc<crate::validator_endpoints::ValidatorEndpoints>>,

    /// Token RPC endpoints for token operations
    pub token_endpoints: Option<Arc<crate::token_rpc::TokenRpcEndpoints>>,

    /// DEX endpoints for DEX operations
    pub dex_endpoints: Option<Arc<crate::dex_endpoints::DexEndpoints>>,

    /// Bridge endpoints for bridge operations
    pub bridge_endpoints: Option<Arc<crate::bridge_endpoints::BridgeEndpoints>>,

    /// Subscription manager for WebSocket subscriptions
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
            token_endpoints: None,
            dex_endpoints: None,
            bridge_endpoints: None,
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

    /// Set token RPC endpoints
    pub fn with_token_endpoints(
        mut self,
        endpoints: Arc<crate::token_rpc::TokenRpcEndpoints>,
    ) -> Self {
        self.token_endpoints = Some(endpoints);
        self
    }

    /// Set DEX endpoints
    pub fn with_dex_endpoints(
        mut self,
        endpoints: Arc<crate::dex_endpoints::DexEndpoints>,
    ) -> Self {
        self.dex_endpoints = Some(endpoints);
        self
    }

    /// Set Bridge endpoints
    pub fn with_bridge_endpoints(
        mut self,
        endpoints: Arc<crate::bridge_endpoints::BridgeEndpoints>,
    ) -> Self {
        self.bridge_endpoints = Some(endpoints);
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
    /// Server configuration
    config: RpcConfig,
    /// Shared server state
    pub state: Arc<RpcServerState>,
    /// HTTP server handle
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

        // Build CORS layer with real configuration
        let cors = if self.config.enable_cors {
            // Parse allowed methods from config
            let methods: Vec<Method> = self.config.cors_methods
                .split(',')
                .filter_map(|m| match m.trim().to_uppercase().as_str() {
                    "GET" => Some(Method::GET),
                    "POST" => Some(Method::POST),
                    "PUT" => Some(Method::PUT),
                    "DELETE" => Some(Method::DELETE),
                    "OPTIONS" => Some(Method::OPTIONS),
                    "PATCH" => Some(Method::PATCH),
                    "HEAD" => Some(Method::HEAD),
                    _ => None,
                })
                .collect();

            // Parse allowed headers from config
            let headers: Vec<header::HeaderName> = self.config.cors_headers
                .split(',')
                .filter_map(|h| header::HeaderName::from_bytes(h.trim().as_bytes()).ok())
                .collect();

            let mut cors_layer = CorsLayer::new()
                .allow_origin(Any)
                .max_age(std::time::Duration::from_secs(self.config.cors_max_age));

            // Add methods
            if !methods.is_empty() {
                cors_layer = cors_layer.allow_methods(methods);
            } else {
                cors_layer = cors_layer.allow_methods([
                    Method::GET,
                    Method::POST,
                    Method::OPTIONS,
                    Method::PUT,
                    Method::DELETE,
                ]);
            }

            // Add headers
            if !headers.is_empty() {
                cors_layer = cors_layer.allow_headers(headers);
            } else {
                cors_layer = cors_layer.allow_headers([
                    header::CONTENT_TYPE,
                    header::AUTHORIZATION,
                ]);
            }

            // Add expose headers for responses
            cors_layer = cors_layer.expose_headers([
                header::CONTENT_TYPE,
                header::CONTENT_LENGTH,
            ]);

            info!("CORS enabled with origins: {}", self.config.cors_origins);
            info!("CORS methods: {}", self.config.cors_methods);
            info!("CORS headers: {}", self.config.cors_headers);
            info!("CORS max age: {} seconds", self.config.cors_max_age);

            cors_layer
        } else {
            info!("CORS disabled");
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
                .allow_headers([header::CONTENT_TYPE])
        };

        // Build router - CORS layer applied ONCE
        let app = Router::new()
            .route("/", post(handle_json_rpc))
            .route("/", options(handle_cors_preflight))
            .route("/health", get(handle_health))
            .route("/health", options(handle_cors_preflight))
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

        // Build CORS layer with real configuration
        let cors = if self.config.enable_cors {
            // Parse allowed methods from config
            let methods: Vec<Method> = self.config.cors_methods
                .split(',')
                .filter_map(|m| match m.trim().to_uppercase().as_str() {
                    "GET" => Some(Method::GET),
                    "POST" => Some(Method::POST),
                    "PUT" => Some(Method::PUT),
                    "DELETE" => Some(Method::DELETE),
                    "OPTIONS" => Some(Method::OPTIONS),
                    "PATCH" => Some(Method::PATCH),
                    "HEAD" => Some(Method::HEAD),
                    _ => None,
                })
                .collect();

            // Parse allowed headers from config
            let mut headers: Vec<header::HeaderName> = self.config.cors_headers
                .split(',')
                .filter_map(|h| header::HeaderName::from_bytes(h.trim().as_bytes()).ok())
                .collect();

            // Add WebSocket-specific headers
            if let Ok(ws_protocol) = header::HeaderName::from_bytes(b"sec-websocket-protocol") {
                headers.push(ws_protocol);
            }
            if let Ok(ws_key) = header::HeaderName::from_bytes(b"sec-websocket-key") {
                headers.push(ws_key);
            }
            if let Ok(ws_version) = header::HeaderName::from_bytes(b"sec-websocket-version") {
                headers.push(ws_version);
            }

            let mut cors_layer = CorsLayer::new()
                .allow_origin(Any)
                .max_age(std::time::Duration::from_secs(self.config.cors_max_age));

            // Add methods (WebSocket uses GET and OPTIONS)
            if !methods.is_empty() {
                cors_layer = cors_layer.allow_methods(methods);
            } else {
                cors_layer = cors_layer.allow_methods([Method::GET, Method::OPTIONS]);
            }

            // Add headers
            if !headers.is_empty() {
                cors_layer = cors_layer.allow_headers(headers);
            } else {
                cors_layer = cors_layer.allow_headers([
                    header::CONTENT_TYPE,
                    header::AUTHORIZATION,
                ]);
            }

            // Add expose headers for responses
            cors_layer = cors_layer.expose_headers([
                header::CONTENT_TYPE,
                header::CONTENT_LENGTH,
            ]);

            info!("WebSocket CORS enabled with origins: {}", self.config.cors_origins);
            info!("WebSocket CORS methods: {}", self.config.cors_methods);
            info!("WebSocket CORS headers: {}", self.config.cors_headers);

            cors_layer
        } else {
            info!("WebSocket CORS disabled");
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([Method::GET, Method::OPTIONS])
                .allow_headers([header::CONTENT_TYPE])
        };

        // Build router - CORS layer applied ONCE
        let app = Router::new()
            .route("/", get(handle_websocket_upgrade))
            .route("/", options(handle_cors_preflight))
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

        // Query methods - Account Info
        "silver_getAccountInfo" => {
            if let Some(ref endpoints) = state.query_endpoints {
                endpoints.silver_get_account_info(request.params)
            } else {
                Err(JsonRpcError::internal_error(
                    "Query endpoints not initialized",
                ))
            }
        }
        "silver_getTransactionHistory" => {
            if let Some(ref endpoints) = state.query_endpoints {
                endpoints.silver_get_transaction_history(request.params)
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
        "silver_getMultipleAccounts" => {
            if let Some(ref endpoints) = state.explorer_endpoints {
                endpoints.silver_get_multiple_accounts(request.params)
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
        "silver_getRecentBlocks" => {
            if let Some(ref endpoints) = state.explorer_endpoints {
                endpoints.silver_get_recent_blocks(request.params)
            } else {
                Err(JsonRpcError::internal_error(
                    "Explorer endpoints not initialized",
                ))
            }
        }
        "silver_getRecentTransactions" => {
            if let Some(ref endpoints) = state.explorer_endpoints {
                endpoints.silver_get_recent_transactions(request.params)
            } else {
                Err(JsonRpcError::internal_error(
                    "Explorer endpoints not initialized",
                ))
            }
        }
        "silver_getTokenInfo" => {
            if let Some(ref endpoints) = state.explorer_endpoints {
                endpoints.silver_get_token_info(request.params)
            } else {
                Err(JsonRpcError::internal_error(
                    "Explorer endpoints not initialized",
                ))
            }
        }
        "silver_getTokenHolders" => {
            if let Some(ref endpoints) = state.explorer_endpoints {
                endpoints.silver_get_token_holders(request.params)
            } else {
                Err(JsonRpcError::internal_error(
                    "Explorer endpoints not initialized",
                ))
            }
        }
        "silver_getTokenTransactions" => {
            if let Some(ref endpoints) = state.explorer_endpoints {
                endpoints.silver_get_token_transactions(request.params)
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

        // Ethereum-compatible methods
        "eth_chainId" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_chain_id()
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_networkId" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_network_id()
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_blockNumber" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_block_number()
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_gasPrice" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_gas_price()
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_getBalance" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_get_balance(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_getCode" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_get_code(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_getTransactionCount" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_get_transaction_count(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_sendTransaction" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_send_transaction(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_sendRawTransaction" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_send_raw_transaction(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_getTransactionByHash" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_get_transaction_by_hash(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_getTransactionReceipt" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_get_transaction_receipt(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_getBlockByNumber" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_get_block_by_number(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_getBlockByHash" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_get_block_by_hash(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_estimateGas" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_estimate_gas(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_call" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_call(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_accounts" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_accounts()
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_coinbase" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_coinbase()
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_mining" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_mining()
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_hashrate" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_hashrate()
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_syncing" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_syncing()
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_getStorageAt" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_get_storage_at(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_getLogs" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_get_logs(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_newFilter" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_new_filter(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_newBlockFilter" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_new_block_filter()
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_newPendingTransactionFilter" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_new_pending_transaction_filter()
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_uninstallFilter" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_uninstall_filter(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_getFilterChanges" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_get_filter_changes(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_getFilterLogs" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_get_filter_logs(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "web3_clientVersion" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.web3_client_version()
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "web3_sha3" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.web3_sha3(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }

        // Additional Ethereum methods
        "eth_protocolVersion" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_protocol_version()
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_getTransactionByBlockNumberAndIndex" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_get_transaction_by_block_number_and_index(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_getTransactionByBlockHashAndIndex" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_get_transaction_by_block_hash_and_index(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_getUncleByBlockNumberAndIndex" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_get_uncle_by_block_number_and_index(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_getUncleByBlockHashAndIndex" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_get_uncle_by_block_hash_and_index(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_getUncleCountByBlockNumber" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_get_uncle_count_by_block_number(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_getUncleCountByBlockHash" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_get_uncle_count_by_block_hash(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_sign" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_sign(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_signTransaction" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_sign_transaction(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_signTypedData" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_sign_typed_data(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_requestAccounts" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_request_accounts()
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_subscribe" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_subscribe(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_unsubscribe" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_unsubscribe(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_getCompilers" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_get_compilers()
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_compileSolidity" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_compile_solidity(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_compileLLL" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_compile_lll(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_compileSerp" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_compile_serp(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }

        // Token RPC methods
        "eth_createToken" => {
            if let Some(ref endpoints) = state.token_endpoints {
                let params = match request.params {
                    serde_json::Value::Array(arr) => arr,
                    _ => vec![],
                };
                match endpoints.eth_create_token(params).await {
                    Ok(result) => Ok(serde_json::json!(result)),
                    Err(e) => Err(JsonRpcError::internal_error(format!("Token error: {}", e))),
                }
            } else {
                Err(JsonRpcError::internal_error("Token endpoints not initialized"))
            }
        }
        "eth_balanceOf" => {
            if let Some(ref endpoints) = state.token_endpoints {
                let params = match request.params {
                    serde_json::Value::Array(arr) => arr,
                    _ => vec![],
                };
                match endpoints.eth_balance_of(params).await {
                    Ok(result) => Ok(serde_json::json!(result)),
                    Err(e) => Err(JsonRpcError::internal_error(format!("Token error: {}", e))),
                }
            } else {
                Err(JsonRpcError::internal_error("Token endpoints not initialized"))
            }
        }
        "eth_allowance" => {
            if let Some(ref endpoints) = state.token_endpoints {
                let params = match request.params {
                    serde_json::Value::Array(arr) => arr,
                    _ => vec![],
                };
                match endpoints.eth_allowance(params).await {
                    Ok(result) => Ok(serde_json::json!(result)),
                    Err(e) => Err(JsonRpcError::internal_error(format!("Token error: {}", e))),
                }
            } else {
                Err(JsonRpcError::internal_error("Token endpoints not initialized"))
            }
        }
        "eth_transfer" => {
            if let Some(ref endpoints) = state.token_endpoints {
                let params = match request.params {
                    serde_json::Value::Array(arr) => arr,
                    _ => vec![],
                };
                match endpoints.eth_transfer(params).await {
                    Ok(result) => Ok(serde_json::json!(result)),
                    Err(e) => Err(JsonRpcError::internal_error(format!("Token error: {}", e))),
                }
            } else {
                Err(JsonRpcError::internal_error("Token endpoints not initialized"))
            }
        }
        "eth_transferFrom" => {
            if let Some(ref endpoints) = state.token_endpoints {
                let params = match request.params {
                    serde_json::Value::Array(arr) => arr,
                    _ => vec![],
                };
                match endpoints.eth_transfer_from(params).await {
                    Ok(result) => Ok(serde_json::json!(result)),
                    Err(e) => Err(JsonRpcError::internal_error(format!("Token error: {}", e))),
                }
            } else {
                Err(JsonRpcError::internal_error("Token endpoints not initialized"))
            }
        }
        "eth_approve" => {
            if let Some(ref endpoints) = state.token_endpoints {
                let params = match request.params {
                    serde_json::Value::Array(arr) => arr,
                    _ => vec![],
                };
                match endpoints.eth_approve(params).await {
                    Ok(result) => Ok(serde_json::json!(result)),
                    Err(e) => Err(JsonRpcError::internal_error(format!("Token error: {}", e))),
                }
            } else {
                Err(JsonRpcError::internal_error("Token endpoints not initialized"))
            }
        }
        "eth_mint" => {
            if let Some(ref endpoints) = state.token_endpoints {
                let params = match request.params {
                    serde_json::Value::Array(arr) => arr,
                    _ => vec![],
                };
                match endpoints.eth_mint(params).await {
                    Ok(result) => Ok(serde_json::json!(result)),
                    Err(e) => Err(JsonRpcError::internal_error(format!("Token error: {}", e))),
                }
            } else {
                Err(JsonRpcError::internal_error("Token endpoints not initialized"))
            }
        }
        "eth_burn" => {
            if let Some(ref endpoints) = state.token_endpoints {
                let params = match request.params {
                    serde_json::Value::Array(arr) => arr,
                    _ => vec![],
                };
                match endpoints.eth_burn(params).await {
                    Ok(result) => Ok(serde_json::json!(result)),
                    Err(e) => Err(JsonRpcError::internal_error(format!("Token error: {}", e))),
                }
            } else {
                Err(JsonRpcError::internal_error("Token endpoints not initialized"))
            }
        }
        "eth_tokenMetadata" => {
            if let Some(ref endpoints) = state.token_endpoints {
                let params = match request.params {
                    serde_json::Value::Array(arr) => arr,
                    _ => vec![],
                };
                match endpoints.eth_token_metadata(params).await {
                    Ok(result) => Ok(result),
                    Err(e) => Err(JsonRpcError::internal_error(format!("Token error: {}", e))),
                }
            } else {
                Err(JsonRpcError::internal_error("Token endpoints not initialized"))
            }
        }
        "eth_transferEvents" => {
            if let Some(ref endpoints) = state.token_endpoints {
                let params = match request.params {
                    serde_json::Value::Array(arr) => arr,
                    _ => vec![],
                };
                match endpoints.eth_transfer_events(params).await {
                    Ok(result) => Ok(serde_json::json!(result)),
                    Err(e) => Err(JsonRpcError::internal_error(format!("Token error: {}", e))),
                }
            } else {
                Err(JsonRpcError::internal_error("Token endpoints not initialized"))
            }
        }
        "eth_listTokens" => {
            if let Some(ref endpoints) = state.token_endpoints {
                let params = match request.params {
                    serde_json::Value::Array(arr) => arr,
                    _ => vec![],
                };
                match endpoints.eth_list_tokens(params).await {
                    Ok(result) => Ok(serde_json::json!(result)),
                    Err(e) => Err(JsonRpcError::internal_error(format!("Token error: {}", e))),
                }
            } else {
                Err(JsonRpcError::internal_error("Token endpoints not initialized"))
            }
        }

        // ====================================================================
        // NETWORK METHODS (net_*)
        // ====================================================================
        "net_version" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.net_version()
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "net_listening" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.net_listening()
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "net_peerCount" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.net_peer_count()
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }

        // ====================================================================
        // STATE/ACCOUNT METHODS
        // ====================================================================
        "eth_getAccount" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_get_account(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_getProof" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_get_proof(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_getStorageProof" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_get_storage_proof(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }

        // ====================================================================
        // BLOCK METHODS
        // ====================================================================
        "eth_getBlockReceipts" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_get_block_receipts(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_blockHash" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_block_hash(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }

        // ====================================================================
        // DEBUG METHODS
        // ====================================================================
        "debug_traceTransaction" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.debug_trace_transaction(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "debug_traceBlockByNumber" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.debug_trace_block_by_number(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "debug_traceBlockByHash" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.debug_trace_block_by_hash(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }

        // ====================================================================
        // ADVANCED METHODS
        // ====================================================================
        "eth_createAccessList" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_create_access_list(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_pendingTransactions" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_pending_transactions()
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_maxPriorityFeePerGas" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_max_priority_fee_per_gas()
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_feeHistory" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_fee_history(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }

        // ====================================================================
        // MISSING BLOCK TRANSACTION COUNT METHODS
        // ====================================================================
        "eth_getBlockTransactionCountByHash" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_get_block_transaction_count_by_hash(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }
        "eth_getBlockTransactionCountByNumber" => {
            if let Some(ref query_endpoints) = state.query_endpoints {
                if let Some(ref tx_endpoints) = state.transaction_endpoints {
                    let eth_endpoints = crate::endpoints::EthereumEndpoints::new(
                        Arc::new(query_endpoints.as_ref().clone()),
                        Arc::new(tx_endpoints.as_ref().clone()),
                    );
                    eth_endpoints.eth_get_block_transaction_count_by_number(request.params)
                } else {
                    Err(JsonRpcError::internal_error("Transaction endpoints not initialized"))
                }
            } else {
                Err(JsonRpcError::internal_error("Query endpoints not initialized"))
            }
        }

        // ====================================================================
        // WALLET MANAGEMENT METHODS
        // ====================================================================

        // Mnemonic-based wallet
        "wallet_generateWallet" => {
            let wallet_endpoints = crate::wallet::WalletEndpoints::new();
            wallet_endpoints.generate_wallet(request.params)
        }
        "wallet_importWallet" => {
            let wallet_endpoints = crate::wallet::WalletEndpoints::new();
            wallet_endpoints.import_wallet(request.params)
        }
        "wallet_deriveAddresses" => {
            let wallet_endpoints = crate::wallet::WalletEndpoints::new();
            wallet_endpoints.derive_addresses(request.params)
        }
        "wallet_validateMnemonic" => {
            let wallet_endpoints = crate::wallet::WalletEndpoints::new();
            wallet_endpoints.validate_mnemonic(request.params)
        }
        "wallet_getDerivationPaths" => {
            let wallet_endpoints = crate::wallet::WalletEndpoints::new();
            wallet_endpoints.get_derivation_paths(request.params)
        }

        // Private key import
        "wallet_importPrivateKey" => {
            let wallet_endpoints = crate::wallet::WalletEndpoints::new();
            wallet_endpoints.import_private_key(request.params)
        }
        "wallet_importPrivateKeyBytes" => {
            let wallet_endpoints = crate::wallet::WalletEndpoints::new();
            wallet_endpoints.import_private_key_bytes(request.params)
        }
        "wallet_importEthereumPrivateKey" => {
            let wallet_endpoints = crate::wallet::WalletEndpoints::new();
            wallet_endpoints.import_ethereum_private_key(request.params)
        }

        // Keystore import
        "wallet_importKeystore" => {
            let wallet_endpoints = crate::wallet::WalletEndpoints::new();
            wallet_endpoints.import_keystore(request.params)
        }

        // Wallet encryption
        "wallet_encryptWallet" => {
            let wallet_endpoints = crate::wallet::WalletEndpoints::new();
            wallet_endpoints.encrypt_wallet(request.params)
        }
        "wallet_decryptWallet" => {
            let wallet_endpoints = crate::wallet::WalletEndpoints::new();
            wallet_endpoints.decrypt_wallet(request.params)
        }

        // Wallet management
        "wallet_listWallets" => {
            let wallet_endpoints = crate::wallet::WalletEndpoints::new();
            wallet_endpoints.list_wallets(request.params)
        }
        "wallet_getWallet" => {
            let wallet_endpoints = crate::wallet::WalletEndpoints::new();
            wallet_endpoints.get_wallet(request.params)
        }
        "wallet_deleteWallet" => {
            let wallet_endpoints = crate::wallet::WalletEndpoints::new();
            wallet_endpoints.delete_wallet(request.params)
        }
        "wallet_renameWallet" => {
            let wallet_endpoints = crate::wallet::WalletEndpoints::new();
            wallet_endpoints.rename_wallet(request.params)
        }
        "wallet_setDefaultWallet" => {
            let wallet_endpoints = crate::wallet::WalletEndpoints::new();
            wallet_endpoints.set_default_wallet(request.params)
        }
        "wallet_getDefaultWallet" => {
            let wallet_endpoints = crate::wallet::WalletEndpoints::new();
            wallet_endpoints.get_default_wallet(request.params)
        }

        // Account derivation
        "wallet_deriveAccounts" => {
            let wallet_endpoints = crate::wallet::WalletEndpoints::new();
            wallet_endpoints.derive_accounts(request.params)
        }

        // Token methods
        "token_create" => {
            if let Some(ref token_endpoints) = state.token_endpoints {
                let params_array = match request.params {
                    JsonValue::Array(arr) => arr,
                    JsonValue::Object(obj) => vec![JsonValue::Object(obj)],
                    _ => vec![],
                };
                
                if params_array.is_empty() {
                    Err(JsonRpcError::invalid_params("Missing parameters"))
                } else {
                    match serde_json::from_value::<crate::token_rpc::CreateTokenRequest>(params_array[0].clone()) {
                        Ok(req) => {
                            let result = token_endpoints.create_token(req).await;
                            result.map(|v| JsonValue::String(v))
                                .map_err(|e| JsonRpcError::internal_error(e.message().to_string()))
                        }
                        Err(e) => Err(JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))
                    }
                }
            } else {
                Err(JsonRpcError::internal_error("Token endpoints not initialized"))
            }
        }
        "token_transfer" => {
            if let Some(ref token_endpoints) = state.token_endpoints {
                let params_array = match request.params {
                    JsonValue::Array(arr) => arr,
                    JsonValue::Object(obj) => vec![JsonValue::Object(obj)],
                    _ => vec![],
                };
                
                if params_array.is_empty() {
                    Err(JsonRpcError::invalid_params("Missing parameters"))
                } else {
                    match serde_json::from_value::<crate::token_rpc::TransferRequest>(params_array[0].clone()) {
                        Ok(req) => {
                            let result = token_endpoints.transfer(req).await;
                            result.map(|v| JsonValue::String(v))
                                .map_err(|e| JsonRpcError::internal_error(e.message().to_string()))
                        }
                        Err(e) => Err(JsonRpcError::invalid_params(format!("Invalid parameters: {}", e)))
                    }
                }
            } else {
                Err(JsonRpcError::internal_error("Token endpoints not initialized"))
            }
        }
        "token_balanceOf" => {
            if let Some(ref token_endpoints) = state.token_endpoints {
                let params_array = match request.params {
                    JsonValue::Array(arr) => arr,
                    _ => vec![],
                };
                
                if params_array.len() < 2 {
                    Err(JsonRpcError::invalid_params("Missing parameters"))
                } else {
                    let symbol = params_array[0].as_str().unwrap_or("").to_string();
                    let account = params_array[1].as_str().unwrap_or("").to_string();
                    
                    let result = token_endpoints.balance_of(symbol, account).await;
                    result.map(|v| JsonValue::String(v))
                        .map_err(|e| JsonRpcError::internal_error(e.message().to_string()))
                }
            } else {
                Err(JsonRpcError::internal_error("Token endpoints not initialized"))
            }
        }
        "token_metadata" => {
            if let Some(ref token_endpoints) = state.token_endpoints {
                let params_array = match request.params {
                    JsonValue::Array(arr) => arr,
                    _ => vec![],
                };
                
                if params_array.is_empty() {
                    Err(JsonRpcError::invalid_params("Missing parameters"))
                } else {
                    let symbol = params_array[0].as_str().unwrap_or("").to_string();
                    
                    let result = token_endpoints.get_metadata(symbol).await;
                    result.map(|v| serde_json::to_value(v).unwrap_or(JsonValue::Null))
                        .map_err(|e| JsonRpcError::internal_error(e.message().to_string()))
                }
            } else {
                Err(JsonRpcError::internal_error("Token endpoints not initialized"))
            }
        }
        "token_approve" => {
            if let Some(ref token_endpoints) = state.token_endpoints {
                let params_array = match request.params {
                    JsonValue::Array(arr) => arr,
                    _ => vec![],
                };
                
                if params_array.len() < 4 {
                    Err(JsonRpcError::invalid_params("Missing parameters"))
                } else {
                    let symbol = params_array[0].as_str().unwrap_or("").to_string();
                    let owner = params_array[1].as_str().unwrap_or("").to_string();
                    let spender = params_array[2].as_str().unwrap_or("").to_string();
                    let amount = params_array[3].as_str().unwrap_or("0").to_string();
                    
                    let result = token_endpoints.approve(symbol, owner, spender, amount).await;
                    result.map(|v| JsonValue::String(v))
                        .map_err(|e| JsonRpcError::internal_error(e.message().to_string()))
                }
            } else {
                Err(JsonRpcError::internal_error("Token endpoints not initialized"))
            }
        }
        "token_allowance" => {
            if let Some(ref token_endpoints) = state.token_endpoints {
                let params_array = match request.params {
                    JsonValue::Array(arr) => arr,
                    _ => vec![],
                };
                
                if params_array.len() < 3 {
                    Err(JsonRpcError::invalid_params("Missing parameters"))
                } else {
                    let symbol = params_array[0].as_str().unwrap_or("").to_string();
                    let owner = params_array[1].as_str().unwrap_or("").to_string();
                    let spender = params_array[2].as_str().unwrap_or("").to_string();
                    
                    let result = token_endpoints.allowance(symbol, owner, spender).await;
                    result.map(|v| JsonValue::String(v))
                        .map_err(|e| JsonRpcError::internal_error(e.message().to_string()))
                }
            } else {
                Err(JsonRpcError::internal_error("Token endpoints not initialized"))
            }
        }
        "token_mint" => {
            if let Some(ref token_endpoints) = state.token_endpoints {
                let params_array = match request.params {
                    JsonValue::Array(arr) => arr,
                    _ => vec![],
                };
                
                if params_array.len() < 4 {
                    Err(JsonRpcError::invalid_params("Missing parameters"))
                } else {
                    let symbol = params_array[0].as_str().unwrap_or("").to_string();
                    let minter = params_array[1].as_str().unwrap_or("").to_string();
                    let to = params_array[2].as_str().unwrap_or("").to_string();
                    let amount = params_array[3].as_str().unwrap_or("0").to_string();
                    
                    let result = token_endpoints.mint(symbol, minter, to, amount).await;
                    result.map(|v| JsonValue::String(v))
                        .map_err(|e| JsonRpcError::internal_error(e.message().to_string()))
                }
            } else {
                Err(JsonRpcError::internal_error("Token endpoints not initialized"))
            }
        }
        "token_burn" => {
            if let Some(ref token_endpoints) = state.token_endpoints {
                let params_array = match request.params {
                    JsonValue::Array(arr) => arr,
                    _ => vec![],
                };
                
                if params_array.len() < 4 {
                    Err(JsonRpcError::invalid_params("Missing parameters"))
                } else {
                    let symbol = params_array[0].as_str().unwrap_or("").to_string();
                    let burner = params_array[1].as_str().unwrap_or("").to_string();
                    let from = params_array[2].as_str().unwrap_or("").to_string();
                    let amount = params_array[3].as_str().unwrap_or("0").to_string();
                    
                    let result = token_endpoints.burn(symbol, burner, from, amount).await;
                    result.map(|v| JsonValue::String(v))
                        .map_err(|e| JsonRpcError::internal_error(e.message().to_string()))
                }
            } else {
                Err(JsonRpcError::internal_error("Token endpoints not initialized"))
            }
        }
        "token_list" => {
            if let Some(ref token_endpoints) = state.token_endpoints {
                let result = token_endpoints.list_tokens().await;
                result.map(|v| serde_json::to_value(v).unwrap_or(JsonValue::Null))
                    .map_err(|e| JsonRpcError::internal_error(e.message().to_string()))
            } else {
                Err(JsonRpcError::internal_error("Token endpoints not initialized"))
            }
        }
        "token_totalSupply" => {
            if let Some(ref token_endpoints) = state.token_endpoints {
                let params_array = match request.params {
                    JsonValue::Array(arr) => arr,
                    _ => vec![],
                };
                
                if params_array.is_empty() {
                    Err(JsonRpcError::invalid_params("Missing parameters"))
                } else {
                    let symbol = params_array[0].as_str().unwrap_or("").to_string();
                    
                    let result = token_endpoints.total_supply(symbol).await;
                    result.map(|v| JsonValue::String(v))
                        .map_err(|e| JsonRpcError::internal_error(e.message().to_string()))
                }
            } else {
                Err(JsonRpcError::internal_error("Token endpoints not initialized"))
            }
        }

        // DEX methods
        "dex_createPool" => {
            if let Some(ref dex_endpoints) = state.dex_endpoints {
                match dex_endpoints.create_pool(request.params).await {
                    Ok(result) => Ok(result),
                    Err(e) => Err(JsonRpcError::internal_error(e)),
                }
            } else {
                Err(JsonRpcError::internal_error("DEX endpoints not initialized"))
            }
        }
        "dex_addLiquidity" => {
            if let Some(ref dex_endpoints) = state.dex_endpoints {
                match dex_endpoints.add_liquidity(request.params).await {
                    Ok(result) => Ok(result),
                    Err(e) => Err(JsonRpcError::internal_error(e)),
                }
            } else {
                Err(JsonRpcError::internal_error("DEX endpoints not initialized"))
            }
        }
        "dex_removeLiquidity" => {
            if let Some(ref dex_endpoints) = state.dex_endpoints {
                match dex_endpoints.remove_liquidity(request.params).await {
                    Ok(result) => Ok(result),
                    Err(e) => Err(JsonRpcError::internal_error(e)),
                }
            } else {
                Err(JsonRpcError::internal_error("DEX endpoints not initialized"))
            }
        }
        "dex_swap" => {
            if let Some(ref dex_endpoints) = state.dex_endpoints {
                match dex_endpoints.swap(request.params).await {
                    Ok(result) => Ok(result),
                    Err(e) => Err(JsonRpcError::internal_error(e)),
                }
            } else {
                Err(JsonRpcError::internal_error("DEX endpoints not initialized"))
            }
        }
        "dex_getPrice" => {
            if let Some(ref dex_endpoints) = state.dex_endpoints {
                match dex_endpoints.get_price(request.params).await {
                    Ok(result) => Ok(result),
                    Err(e) => Err(JsonRpcError::internal_error(e)),
                }
            } else {
                Err(JsonRpcError::internal_error("DEX endpoints not initialized"))
            }
        }
        "dex_getPool" => {
            if let Some(ref dex_endpoints) = state.dex_endpoints {
                match dex_endpoints.get_pool(request.params).await {
                    Ok(result) => Ok(result),
                    Err(e) => Err(JsonRpcError::internal_error(e)),
                }
            } else {
                Err(JsonRpcError::internal_error("DEX endpoints not initialized"))
            }
        }
        "dex_listPools" => {
            if let Some(ref dex_endpoints) = state.dex_endpoints {
                match dex_endpoints.list_pools(request.params).await {
                    Ok(result) => Ok(result),
                    Err(e) => Err(JsonRpcError::internal_error(e)),
                }
            } else {
                Err(JsonRpcError::internal_error("DEX endpoints not initialized"))
            }
        }
        "dex_getStats" => {
            if let Some(ref dex_endpoints) = state.dex_endpoints {
                match dex_endpoints.get_stats(request.params).await {
                    Ok(result) => Ok(result),
                    Err(e) => Err(JsonRpcError::internal_error(e)),
                }
            } else {
                Err(JsonRpcError::internal_error("DEX endpoints not initialized"))
            }
        }

        // Bridge methods
        "bridge_registerValidator" => {
            if let Some(ref bridge_endpoints) = state.bridge_endpoints {
                match bridge_endpoints.register_validator(request.params).await {
                    Ok(result) => Ok(result),
                    Err(e) => Err(JsonRpcError::internal_error(e)),
                }
            } else {
                Err(JsonRpcError::internal_error("Bridge endpoints not initialized"))
            }
        }
        "bridge_unregisterValidator" => {
            if let Some(ref bridge_endpoints) = state.bridge_endpoints {
                match bridge_endpoints.unregister_validator(request.params).await {
                    Ok(result) => Ok(result),
                    Err(e) => Err(JsonRpcError::internal_error(e)),
                }
            } else {
                Err(JsonRpcError::internal_error("Bridge endpoints not initialized"))
            }
        }
        "bridge_initiate" => {
            if let Some(ref bridge_endpoints) = state.bridge_endpoints {
                match bridge_endpoints.initiate(request.params).await {
                    Ok(result) => Ok(result),
                    Err(e) => Err(JsonRpcError::internal_error(e)),
                }
            } else {
                Err(JsonRpcError::internal_error("Bridge endpoints not initialized"))
            }
        }
        "bridge_confirm" => {
            if let Some(ref bridge_endpoints) = state.bridge_endpoints {
                match bridge_endpoints.confirm(request.params).await {
                    Ok(result) => Ok(result),
                    Err(e) => Err(JsonRpcError::internal_error(e)),
                }
            } else {
                Err(JsonRpcError::internal_error("Bridge endpoints not initialized"))
            }
        }
        "bridge_execute" => {
            if let Some(ref bridge_endpoints) = state.bridge_endpoints {
                match bridge_endpoints.execute(request.params).await {
                    Ok(result) => Ok(result),
                    Err(e) => Err(JsonRpcError::internal_error(e)),
                }
            } else {
                Err(JsonRpcError::internal_error("Bridge endpoints not initialized"))
            }
        }
        "bridge_cancel" => {
            if let Some(ref bridge_endpoints) = state.bridge_endpoints {
                match bridge_endpoints.cancel(request.params).await {
                    Ok(result) => Ok(result),
                    Err(e) => Err(JsonRpcError::internal_error(e)),
                }
            } else {
                Err(JsonRpcError::internal_error("Bridge endpoints not initialized"))
            }
        }
        "bridge_getStatus" => {
            if let Some(ref bridge_endpoints) = state.bridge_endpoints {
                match bridge_endpoints.get_status(request.params).await {
                    Ok(result) => Ok(result),
                    Err(e) => Err(JsonRpcError::internal_error(e)),
                }
            } else {
                Err(JsonRpcError::internal_error("Bridge endpoints not initialized"))
            }
        }
        "bridge_getValidator" => {
            if let Some(ref bridge_endpoints) = state.bridge_endpoints {
                match bridge_endpoints.get_validator(request.params).await {
                    Ok(result) => Ok(result),
                    Err(e) => Err(JsonRpcError::internal_error(e)),
                }
            } else {
                Err(JsonRpcError::internal_error("Bridge endpoints not initialized"))
            }
        }
        "bridge_listValidators" => {
            if let Some(ref bridge_endpoints) = state.bridge_endpoints {
                match bridge_endpoints.list_validators(request.params).await {
                    Ok(result) => Ok(result),
                    Err(e) => Err(JsonRpcError::internal_error(e)),
                }
            } else {
                Err(JsonRpcError::internal_error("Bridge endpoints not initialized"))
            }
        }
        "bridge_getStats" => {
            if let Some(ref bridge_endpoints) = state.bridge_endpoints {
                match bridge_endpoints.get_stats(request.params).await {
                    Ok(result) => Ok(result),
                    Err(e) => Err(JsonRpcError::internal_error(e)),
                }
            } else {
                Err(JsonRpcError::internal_error("Bridge endpoints not initialized"))
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

/// Handle CORS preflight requests (OPTIONS)
async fn handle_cors_preflight() -> impl IntoResponse {
    // The CORS layer handles the actual headers, we just need to return 200 OK
    (StatusCode::OK, "")
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
