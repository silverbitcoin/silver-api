//! SilverBitcoin RPC Server
//!
//! Standalone RPC server for blockchain interaction.
//!
//! Usage:
//! ```bash
//! cargo run --release -- --http 127.0.0.1:9000 --ws 127.0.0.1:9001 --db ./data
//! ```

use clap::Parser;
use silver_api::{ExplorerEndpoints, QueryEndpoints, RpcConfig, RpcServer, TransactionEndpoints};
use silver_storage::{BlockStore, EventStore, ObjectStore, ParityDatabase, TransactionStore};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{info, warn};
use tracing_subscriber;

#[derive(Parser, Debug)]
#[command(name = "SilverBitcoin RPC Server")]
#[command(about = "JSON-RPC 2.0 API server for SilverBitcoin blockchain", long_about = None)]
struct Args {
    /// HTTP server bind address
    #[arg(long, default_value = "127.0.0.1:9000")]
    http: String,

    /// WebSocket server bind address
    #[arg(long, default_value = "127.0.0.1:9001")]
    ws: String,

    /// Database directory path (should be synced RPC database, not validator's database)
    /// PRODUCTION: RPC server reads from synced copy of Validator 2's database
    /// - Validator 2 writes to: data/mainnet/db/validator2/
    /// - sync-rpc-db.sh copies to: data/mainnet/db/rpc/
    /// - RPC server reads from: data/mainnet/db/rpc/
    #[arg(long, default_value = "./data/mainnet/db/rpc")]
    db: PathBuf,

    /// Maximum concurrent connections
    #[arg(long, default_value = "1000")]
    max_connections: u32,

    /// Rate limit per IP (requests per second)
    #[arg(long, default_value = "100")]
    rate_limit: u32,

    /// Enable CORS
    #[arg(long, default_value = "true")]
    enable_cors: bool,

    /// CORS allowed origins (comma-separated, use * for all)
    #[arg(long, default_value = "*")]
    cors_origins: String,

    /// CORS allowed methods (comma-separated)
    #[arg(long, default_value = "GET,POST,OPTIONS,PUT,DELETE")]
    cors_methods: String,

    /// CORS allowed headers (comma-separated)
    #[arg(long, default_value = "Content-Type,Authorization,X-Requested-With")]
    cors_headers: String,

    /// CORS max age in seconds
    #[arg(long, default_value = "3600")]
    cors_max_age: u64,

    /// Log level (trace, debug, info, warn, error)
    #[arg(long, default_value = "info")]
    log_level: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    // Initialize tracing
    init_tracing(&args.log_level)?;

    info!("Starting SilverBitcoin RPC Server");
    info!("HTTP: {}", args.http);
    info!("WebSocket: {}", args.ws);
    info!("Database: {}", args.db.display());
    info!("CORS enabled: {}", args.enable_cors);
    if args.enable_cors {
        info!("CORS origins: {}", args.cors_origins);
        info!("CORS methods: {}", args.cors_methods);
        info!("CORS headers: {}", args.cors_headers);
        info!("CORS max age: {} seconds", args.cors_max_age);
    }

    // Parse addresses
    let http_addr: SocketAddr = args.http.parse()?;
    let ws_addr: SocketAddr = args.ws.parse()?;

    // Create database directory if it doesn't exist
    std::fs::create_dir_all(&args.db)?;

    // Initialize storage
    info!("Initializing ParityDB database...");
    let db = Arc::new(ParityDatabase::open(&args.db)?);
    let object_store = Arc::new(ObjectStore::new(Arc::clone(&db)));
    let transaction_store = Arc::new(TransactionStore::new(Arc::clone(&db)));
    let event_store = Arc::new(EventStore::new());
    let block_store = Arc::new(BlockStore::new(Arc::clone(&db)));
    let token_store = Arc::new(silver_storage::TokenStore::new());

    info!("Database initialized successfully");

    // RPC server is read-only - do not create or modify blocks
    // Blocks are created by the Node and stored in the shared database
    // Verify that at least genesis block exists
    match block_store.get_block(0) {
        Ok(Some(_)) => {
            info!("Genesis block 0 found in database");
        }
        Ok(None) => {
            warn!("Genesis block 0 not found - Node may not have initialized the database yet");
        }
        Err(e) => {
            warn!("Error checking genesis block: {}", e);
        }
    }

    // Create endpoints
    let query_endpoints = Arc::new(QueryEndpoints::new(
        Arc::clone(&object_store),
        Arc::clone(&transaction_store),
        Arc::clone(&block_store),
        Arc::clone(&event_store),
    ));
    let transaction_endpoints = Arc::new(TransactionEndpoints::new(Arc::clone(&transaction_store)));
    let explorer_endpoints = Arc::new(ExplorerEndpoints::new(
        Arc::clone(&object_store),
        Arc::clone(&transaction_store),
        Arc::clone(&block_store),
    ));
    let token_endpoints = Arc::new(silver_api::TokenRpcEndpoints::new(Arc::clone(&token_store)));

    // Create RPC server configuration
    let config = RpcConfig {
        http_addr,
        ws_addr,
        max_request_size: 128 * 1024,        // 128KB
        max_response_size: 10 * 1024 * 1024, // 10MB
        max_connections: args.max_connections,
        enable_cors: args.enable_cors,
        cors_origins: args.cors_origins,
        cors_methods: args.cors_methods,
        cors_headers: args.cors_headers,
        cors_max_age: args.cors_max_age,
        rate_limit_per_ip: args.rate_limit,
    };

    // Create validator endpoints
    let validator_endpoints = Arc::new(silver_api::ValidatorEndpoints::new());

    // Create and start RPC server with all endpoints including validators and tokens
    let mut server = RpcServer::with_all_endpoints_and_validators(
        config,
        query_endpoints,
        transaction_endpoints,
        explorer_endpoints,
        validator_endpoints,
    );
    server.state = Arc::new(
        (*server.state).clone().with_token_endpoints(token_endpoints)
    );

    info!("Starting RPC servers...");
    server.start().await?;

    info!("✓ RPC Server started successfully");
    info!("HTTP endpoint: http://{}", args.http);
    info!("WebSocket endpoint: ws://{}", args.ws);
    info!("Press Ctrl+C to stop");

    // Keep the server running
    tokio::signal::ctrl_c().await?;
    info!("Shutting down...");

    Ok(())
}

/// Initialize tracing/logging
fn init_tracing(log_level: &str) -> Result<(), Box<dyn std::error::Error>> {
    let level = match log_level.to_lowercase().as_str() {
        "trace" => tracing::Level::TRACE,
        "debug" => tracing::Level::DEBUG,
        "info" => tracing::Level::INFO,
        "warn" => tracing::Level::WARN,
        "error" => tracing::Level::ERROR,
        _ => tracing::Level::INFO,
    };

    tracing_subscriber::fmt()
        .with_max_level(level)
        .with_target(true)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true)
        .init();

    Ok(())
}
