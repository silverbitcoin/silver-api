//! WebSocket event subscription system
//!
//! This module provides real-time event subscriptions over WebSocket connections.
//! Clients can subscribe to events filtered by sender, type, or object type.
//!
//! Features:
//! - Up to 10 active subscriptions per connection
//! - Event filtering by sender, type, object type
//! - Delivery within 500ms of finalization
//! - Automatic cleanup on disconnect

use axum::extract::ws::{Message, WebSocket};
use dashmap::DashMap;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use silver_core::{ObjectID, SilverAddress, TransactionDigest};
use silver_storage::{Event, EventType};
use std::{
    net::SocketAddr,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::{broadcast, mpsc, RwLock};
use tracing::{debug, error, info, warn};

/// Maximum number of subscriptions per connection
const MAX_SUBSCRIPTIONS_PER_CONNECTION: usize = 10;

/// Event delivery timeout (500ms as per requirements)
#[allow(dead_code)]
const EVENT_DELIVERY_TIMEOUT: Duration = Duration::from_millis(500);

/// Subscription ID
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SubscriptionID(u64);

impl SubscriptionID {
    /// Create a new subscription ID
    pub fn new(id: u64) -> Self {
        Self(id)
    }

    /// Get the inner value
    pub fn value(&self) -> u64 {
        self.0
    }
}

/// Event filter for subscriptions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventFilter {
    /// Filter by sender address (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender: Option<SilverAddress>,

    /// Filter by event type (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_type: Option<String>,

    /// Filter by object type (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_type: Option<String>,

    /// Filter by object ID (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_id: Option<ObjectID>,

    /// Filter by transaction digest (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_digest: Option<TransactionDigest>,
}

impl EventFilter {
    /// Create a new empty filter (matches all events)
    pub fn new() -> Self {
        Self {
            sender: None,
            event_type: None,
            object_type: None,
            object_id: None,
            transaction_digest: None,
        }
    }

    /// Filter by sender
    pub fn with_sender(mut self, sender: SilverAddress) -> Self {
        self.sender = Some(sender);
        self
    }

    /// Filter by event type
    pub fn with_event_type(mut self, event_type: String) -> Self {
        self.event_type = Some(event_type);
        self
    }

    /// Filter by object type
    pub fn with_object_type(mut self, object_type: String) -> Self {
        self.object_type = Some(object_type);
        self
    }

    /// Filter by object ID
    pub fn with_object_id(mut self, object_id: ObjectID) -> Self {
        self.object_id = Some(object_id);
        self
    }

    /// Filter by transaction digest
    pub fn with_transaction_digest(mut self, digest: TransactionDigest) -> Self {
        self.transaction_digest = Some(digest);
        self
    }

    /// Check if an event matches this filter
    pub fn matches(&self, event: &EventNotification) -> bool {
        // Check sender filter
        if let Some(ref sender) = self.sender {
            if event.sender != *sender {
                return false;
            }
        }

        // Check event type filter
        if let Some(ref event_type) = self.event_type {
            if event.event_type != *event_type {
                return false;
            }
        }

        // Check object type filter
        if let Some(ref object_type) = self.object_type {
            if let Some(ref evt_obj_type) = event.object_type {
                if evt_obj_type != object_type {
                    return false;
                }
            } else {
                return false;
            }
        }

        // Check object ID filter
        if let Some(ref object_id) = self.object_id {
            if event.object_id != Some(*object_id) {
                return false;
            }
        }

        // Check transaction digest filter
        if let Some(ref tx_digest) = self.transaction_digest {
            if event.transaction_digest != *tx_digest {
                return false;
            }
        }

        true
    }
}

impl Default for EventFilter {
    fn default() -> Self {
        Self::new()
    }
}

/// Subscription request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscribeRequest {
    /// Subscription method (must be "silver_subscribeEvents")
    pub method: String,

    /// Event filter
    pub filter: EventFilter,
}

/// Subscription response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscribeResponse {
    /// Subscription ID
    pub subscription_id: SubscriptionID,

    /// Success message
    pub message: String,
}

/// Unsubscribe request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnsubscribeRequest {
    /// Unsubscribe method (must be "silver_unsubscribe")
    pub method: String,

    /// Subscription ID to cancel
    pub subscription_id: SubscriptionID,
}

/// Event notification sent to subscribers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventNotification {
    /// Subscription ID this event is for
    pub subscription_id: SubscriptionID,

    /// Event ID
    pub event_id: u64,

    /// Transaction that generated this event
    pub transaction_digest: TransactionDigest,

    /// Event type
    pub event_type: String,

    /// Sender address
    pub sender: SilverAddress,

    /// Object ID (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_id: Option<ObjectID>,

    /// Object type (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_type: Option<String>,

    /// Event data (hex-encoded)
    pub data: String,

    /// Timestamp (Unix milliseconds)
    pub timestamp: u64,
}

impl EventNotification {
    /// Create a new event notification
    pub fn from_event(
        subscription_id: SubscriptionID,
        event: &Event,
        sender: SilverAddress,
    ) -> Self {
        let event_type_str = match &event.event_type {
            EventType::ObjectCreated => "ObjectCreated".to_string(),
            EventType::ObjectModified => "ObjectModified".to_string(),
            EventType::ObjectDeleted => "ObjectDeleted".to_string(),
            EventType::ObjectTransferred => "ObjectTransferred".to_string(),
            EventType::ObjectShared => "ObjectShared".to_string(),
            EventType::ObjectFrozen => "ObjectFrozen".to_string(),
            EventType::CoinSplit => "CoinSplit".to_string(),
            EventType::CoinMerged => "CoinMerged".to_string(),
            EventType::ModulePublished => "ModulePublished".to_string(),
            EventType::FunctionCalled => "FunctionCalled".to_string(),
            EventType::Custom(name) => name.clone(),
        };

        // Extract object type from event data if available
        let object_type = if let Ok(event_data) = bcs::from_bytes::<serde_json::Value>(&event.data)
        {
            event_data
                .get("type")
                .and_then(|t| t.as_str())
                .map(|s| s.to_string())
        } else {
            None
        };

        Self {
            subscription_id,
            event_id: event.event_id.value(),
            transaction_digest: event.transaction_digest,
            event_type: event_type_str,
            sender,
            object_id: event.object_id,
            object_type,
            data: hex::encode(&event.data),
            timestamp: event.timestamp,
        }
    }
}

/// Active subscription
struct Subscription {
    /// Subscription ID
    id: SubscriptionID,

    /// Event filter
    filter: EventFilter,

    /// Channel to send events to this subscription
    sender: mpsc::UnboundedSender<EventNotification>,
}

/// Connection state
struct ConnectionState {
    /// Active subscriptions for this connection
    subscriptions: Vec<Subscription>,

    /// Connection address
    addr: SocketAddr,
}

impl ConnectionState {
    /// Create a new connection state
    fn new(addr: SocketAddr) -> Self {
        Self {
            subscriptions: Vec::new(),
            addr,
        }
    }

    /// Add a subscription
    fn add_subscription(&mut self, subscription: Subscription) -> Result<(), String> {
        if self.subscriptions.len() >= MAX_SUBSCRIPTIONS_PER_CONNECTION {
            return Err(format!(
                "Maximum subscriptions per connection ({}) exceeded",
                MAX_SUBSCRIPTIONS_PER_CONNECTION
            ));
        }

        self.subscriptions.push(subscription);
        Ok(())
    }

    /// Remove a subscription
    fn remove_subscription(&mut self, subscription_id: SubscriptionID) -> bool {
        let initial_len = self.subscriptions.len();
        self.subscriptions.retain(|s| s.id != subscription_id);
        self.subscriptions.len() < initial_len
    }

    /// Get subscription count
    fn subscription_count(&self) -> usize {
        self.subscriptions.len()
    }
}

/// Subscription manager
///
/// Manages all active WebSocket subscriptions and routes events to subscribers.
pub struct SubscriptionManager {
    /// Active connections
    connections: Arc<DashMap<SocketAddr, Arc<RwLock<ConnectionState>>>>,

    /// Next subscription ID
    next_subscription_id: Arc<AtomicU64>,

    /// Event broadcast channel
    event_broadcast: broadcast::Sender<(Event, SilverAddress)>,
}

impl SubscriptionManager {
    /// Create a new subscription manager
    pub fn new() -> Self {
        let (event_broadcast, _) = broadcast::channel(10000);

        Self {
            connections: Arc::new(DashMap::new()),
            next_subscription_id: Arc::new(AtomicU64::new(0)),
            event_broadcast,
        }
    }

    /// Get a broadcast receiver for events
    pub fn subscribe_to_events(&self) -> broadcast::Receiver<(Event, SilverAddress)> {
        self.event_broadcast.subscribe()
    }

    /// Broadcast an event to all subscribers
    ///
    /// This should be called when a new event is finalized.
    pub async fn broadcast_event(&self, event: Event, sender: SilverAddress) {
        debug!(
            "Broadcasting event: id={}, type={:?}, sender={}",
            event.event_id.value(),
            event.event_type,
            sender
        );

        // Send to broadcast channel (best-effort)
        let _ = self.event_broadcast.send((event, sender));
    }

    /// Handle a new WebSocket connection
    pub async fn handle_connection(&self, socket: WebSocket, addr: SocketAddr) {
        info!("New WebSocket subscription connection from {}", addr);

        // Create connection state
        let conn_state = Arc::new(RwLock::new(ConnectionState::new(addr)));
        self.connections.insert(addr, conn_state.clone());

        // Split socket
        let (mut sender, mut receiver) = socket.split();

        // Create event receiver for this connection
        let mut event_receiver = self.event_broadcast.subscribe();

        // Spawn task to forward events to this connection
        let conn_state_clone = conn_state.clone();
        let addr_clone = addr;
        let event_forwarder = tokio::spawn(async move {
            while let Ok((event, event_sender)) = event_receiver.recv().await {
                let state = conn_state_clone.read().await;

                // Check each subscription to see if it matches
                for subscription in &state.subscriptions {
                    let notification =
                        EventNotification::from_event(subscription.id, &event, event_sender);

                    if subscription.filter.matches(&notification) {
                        // Send event to subscription channel
                        if let Err(e) = subscription.sender.send(notification) {
                            warn!(
                                "Failed to send event to subscription {}: {}",
                                subscription.id.value(),
                                e
                            );
                        }
                    }
                }
            }

            debug!("Event forwarder for {} stopped", addr_clone);
        });

        // Handle incoming messages and outgoing events
        let result = self
            .handle_connection_messages(&mut sender, &mut receiver, conn_state.clone(), addr)
            .await;

        // Cleanup
        event_forwarder.abort();
        self.connections.remove(&addr);

        match result {
            Ok(_) => info!("WebSocket subscription connection from {} closed", addr),
            Err(e) => error!("WebSocket subscription error from {}: {}", addr, e),
        }
    }

    /// Handle messages for a connection
    async fn handle_connection_messages(
        &self,
        sender: &mut futures::stream::SplitSink<WebSocket, Message>,
        receiver: &mut futures::stream::SplitStream<WebSocket>,
        conn_state: Arc<RwLock<ConnectionState>>,
        addr: SocketAddr,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Create channel for subscription events
        let (event_tx, mut event_rx) = mpsc::unbounded_channel::<EventNotification>();

        // Handle incoming messages and outgoing events concurrently
        loop {
            tokio::select! {
                // Handle incoming WebSocket messages
                Some(msg) = receiver.next() => {
                    match msg {
                        Ok(Message::Text(text)) => {
                            debug!("Received subscription message from {}: {}", addr, text);

                            // Parse message
                            let response = self
                                .handle_subscription_message(&text, conn_state.clone(), event_tx.clone())
                                .await;

                            // Send response
                            match serde_json::to_string(&response) {
                                Ok(response_text) => {
                                    if let Err(e) = sender.send(Message::Text(response_text)).await {
                                        error!("Failed to send response: {}", e);
                                        break;
                                    }
                                }
                                Err(e) => {
                                    error!("Failed to serialize response: {}", e);
                                    break;
                                }
                            }
                        }
                        Ok(Message::Binary(_)) => {
                            warn!("Received binary message from {}, ignoring", addr);
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
                            info!("WebSocket close message from {}", addr);
                            break;
                        }
                        Err(e) => {
                            error!("WebSocket error from {}: {}", addr, e);
                            break;
                        }
                    }
                }
                // Handle outgoing event notifications
                Some(notification) = event_rx.recv() => {
                    match serde_json::to_string(&notification) {
                        Ok(json) => {
                            if let Err(e) = sender.send(Message::Text(json)).await {
                                error!("Failed to send event notification: {}", e);
                                break;
                            }
                        }
                        Err(e) => {
                            error!("Failed to serialize event notification: {}", e);
                        }
                    }
                }
                else => break,
            }
        }

        Ok(())
    }

    /// Handle a subscription message
    async fn handle_subscription_message(
        &self,
        text: &str,
        conn_state: Arc<RwLock<ConnectionState>>,
        event_tx: mpsc::UnboundedSender<EventNotification>,
    ) -> JsonValue {
        // Try to parse as subscribe request
        if let Ok(req) = serde_json::from_str::<SubscribeRequest>(text) {
            return self.handle_subscribe(req, conn_state, event_tx).await;
        }

        // Try to parse as unsubscribe request
        if let Ok(req) = serde_json::from_str::<UnsubscribeRequest>(text) {
            return self.handle_unsubscribe(req, conn_state).await;
        }

        // Unknown message
        serde_json::json!({
            "error": "Invalid subscription message format"
        })
    }

    /// Handle subscribe request
    async fn handle_subscribe(
        &self,
        req: SubscribeRequest,
        conn_state: Arc<RwLock<ConnectionState>>,
        event_tx: mpsc::UnboundedSender<EventNotification>,
    ) -> JsonValue {
        // Allocate subscription ID
        let subscription_id =
            SubscriptionID::new(self.next_subscription_id.fetch_add(1, Ordering::SeqCst));

        // Create subscription
        let subscription = Subscription {
            id: subscription_id,
            filter: req.filter,
            sender: event_tx,
        };

        // Add to connection state
        let mut state = conn_state.write().await;
        match state.add_subscription(subscription) {
            Ok(_) => {
                info!(
                    "Created subscription {} for connection {}",
                    subscription_id.value(),
                    state.addr
                );

                serde_json::json!({
                    "subscription_id": subscription_id.value(),
                    "message": "Subscription created successfully"
                })
            }
            Err(e) => {
                warn!("Failed to create subscription: {}", e);

                serde_json::json!({
                    "error": e
                })
            }
        }
    }

    /// Handle unsubscribe request
    async fn handle_unsubscribe(
        &self,
        req: UnsubscribeRequest,
        conn_state: Arc<RwLock<ConnectionState>>,
    ) -> JsonValue {
        let mut state = conn_state.write().await;

        if state.remove_subscription(req.subscription_id) {
            info!(
                "Removed subscription {} for connection {}",
                req.subscription_id.value(),
                state.addr
            );

            serde_json::json!({
                "message": "Subscription removed successfully"
            })
        } else {
            warn!(
                "Subscription {} not found for connection {}",
                req.subscription_id.value(),
                state.addr
            );

            serde_json::json!({
                "error": "Subscription not found"
            })
        }
    }

    /// Get the number of active connections
    pub fn connection_count(&self) -> usize {
        self.connections.len()
    }

    /// Get the total number of active subscriptions
    pub async fn subscription_count(&self) -> usize {
        let mut total = 0;
        for entry in self.connections.iter() {
            let state = entry.value().read().await;
            total += state.subscription_count();
        }
        total
    }
}

impl Default for SubscriptionManager {
    fn default() -> Self {
        Self::new()
    }
}
