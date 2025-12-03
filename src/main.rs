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
use silver_storage::{BlockStore, EventStore, ObjectStore, RocksDatabase, TransactionStore};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::info;
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

    /// Database directory path
    #[arg(long, default_value = "./data")]
    db: PathBuf,

    /// Maximum concurrent connections
    #[arg(long, default_value = "1000")]
    max_connections: u32,

    /// Rate limit per IP (requests per second)
    #[arg(long, default_value = "100")]
    rate_limit: u32,

    /// Enable CORS
    #[arg(long, default_value = "false")]
    enable_cors: bool,

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

    // Parse addresses
    let http_addr: SocketAddr = args.http.parse()?;
    let ws_addr: SocketAddr = args.ws.parse()?;

    // Create database directory if it doesn't exist
    std::fs::create_dir_all(&args.db)?;

    // Initialize storage
    info!("Initializing database...");
    let db = Arc::new(RocksDatabase::open(&args.db)?);
    let object_store = Arc::new(ObjectStore::new(Arc::clone(&db)));
    let transaction_store = Arc::new(TransactionStore::new(Arc::clone(&db)));
    let event_store = Arc::new(EventStore::new(Arc::clone(&db)));
    let block_store = Arc::new(BlockStore::new(Arc::clone(&db)));

    info!("Database initialized successfully");

    // Initialize genesis block if it doesn't exist
    if let Ok(None) = block_store.get_block(0) {
        info!("Genesis block not found, creating genesis block 0");
        let genesis_block = silver_storage::Block::new(
            0,
            vec![0u8; 32],
            vec![0u8; 32],
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
            vec![],
            vec![0u8; 32],
            0,
            0,
        );
        block_store.store_block(&genesis_block)?;
        info!("Genesis block 0 created and stored successfully");
    } else {
        info!("Genesis block 0 already exists");
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

    // Create RPC server configuration
    let config = RpcConfig {
        http_addr,
        ws_addr,
        max_request_size: 128 * 1024,        // 128KB
        max_response_size: 10 * 1024 * 1024, // 10MB
        max_connections: args.max_connections,
        enable_cors: args.enable_cors,
        rate_limit_per_ip: args.rate_limit,
    };

    // Create validator endpoints
    let validator_endpoints = Arc::new(silver_api::ValidatorEndpoints::new());

    // Create and start RPC server with all endpoints including validators
    let mut server = RpcServer::with_all_endpoints_and_validators(
        config,
        query_endpoints,
        transaction_endpoints,
        explorer_endpoints,
        validator_endpoints,
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
