# Event Subscription System Guide

This guide explains how to use the SilverBitcoin event subscription system for real-time blockchain event notifications.

## Overview

The event subscription system provides:
- **WebSocket-based subscriptions** for real-time event delivery
- **Flexible filtering** by sender, event type, object type, object ID, or transaction
- **Up to 10 active subscriptions per connection**
- **Sub-500ms event delivery** after transaction finalization
- **30+ day event retention** in persistent storage

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    Event Flow                                │
├─────────────────────────────────────────────────────────────┤
│                                                               │
│  1. Transaction Execution                                    │
│     ┌──────────────────┐                                    │
│     │ TransactionExecutor │ ──► Emits events during execution│
│     └──────────────────┘                                    │
│              │                                                │
│              ▼                                                │
│  2. Event Emission                                           │
│     ┌──────────────────┐                                    │
│     │  EventEmitter    │ ──► Persists to EventStore         │
│     └──────────────────┘                                    │
│              │                                                │
│              ▼                                                │
│  3. Event Broadcasting                                       │
│     ┌──────────────────┐                                    │
│     │ SubscriptionMgr  │ ──► Broadcasts to subscribers      │
│     └──────────────────┘                                    │
│              │                                                │
│              ▼                                                │
│  4. Event Delivery                                           │
│     ┌──────────────────┐                                    │
│     │ WebSocket Client │ ◄── Receives filtered events       │
│     └──────────────────┘                                    │
│                                                               │
└─────────────────────────────────────────────────────────────┘
```

## Components

### 1. EventEmitter

Handles event emission from transaction execution:

```rust
use silver_execution::EventEmitter;
use silver_storage::EventStore;
use std::sync::Arc;

// Create event emitter
let event_store = Arc::new(EventStore::new(db));
let emitter = EventEmitter::new(event_store);

// Emit events after transaction execution
let event_ids = emitter.emit_transaction_events(
    transaction_digest,
    &execution_result,
)?;

// Batch emit for multiple transactions
let event_ids = emitter.emit_batch_events(&transactions)?;
```

### 2. SubscriptionManager

Manages WebSocket subscriptions and event routing:

```rust
use silver_api::SubscriptionManager;

// Create subscription manager
let subscription_manager = Arc::new(SubscriptionManager::new());

// Handle WebSocket connection
subscription_manager.handle_connection(socket, addr).await;

// Broadcast event to all subscribers
subscription_manager.broadcast_event(event, sender).await;
```

### 3. EventFilter

Filters events based on criteria:

```rust
use silver_api::EventFilter;

// Filter by sender
let filter = EventFilter::new()
    .with_sender(sender_address);

// Filter by event type
let filter = EventFilter::new()
    .with_event_type("ObjectCreated".to_string());

// Filter by object ID
let filter = EventFilter::new()
    .with_object_id(object_id);

// Multiple filters
let filter = EventFilter::new()
    .with_sender(sender_address)
    .with_event_type("TransferObjects".to_string())
    .with_object_id(object_id);
```

## WebSocket Protocol

### Connection

Connect to the WebSocket endpoint:

```
ws://localhost:9001/
```

### Subscribe to Events

Send a subscription request:

```json
{
  "method": "silver_subscribeEvents",
  "filter": {
    "sender": "0x1234...",
    "event_type": "ObjectCreated",
    "object_type": "Coin",
    "object_id": "0xabcd...",
    "transaction_digest": "0x5678..."
  }
}
```

Response:

```json
{
  "subscription_id": 1,
  "message": "Subscription created successfully"
}
```

### Receive Events

Events are pushed to the client as they occur:

```json
{
  "subscription_id": 1,
  "event_id": 12345,
  "transaction_digest": "0x5678...",
  "event_type": "ObjectCreated",
  "sender": "0x1234...",
  "object_id": "0xabcd...",
  "object_type": "Coin",
  "data": "0x1a2b3c...",
  "timestamp": 1699564800000
}
```

### Unsubscribe

Cancel a subscription:

```json
{
  "method": "silver_unsubscribe",
  "subscription_id": 1
}
```

Response:

```json
{
  "message": "Subscription removed successfully"
}
```

## Event Types

The following event types are supported:

- `ObjectCreated` - New object created
- `ObjectModified` - Object state changed
- `ObjectDeleted` - Object deleted
- `ObjectTransferred` - Object ownership transferred
- `ObjectShared` - Object made shared
- `ObjectFrozen` - Object made immutable
- `CoinSplit` - Coin split into multiple coins
- `CoinMerged` - Multiple coins merged
- `ModulePublished` - Smart contract module published
- `FunctionCalled` - Smart contract function called
- `Custom(name)` - Custom event from smart contract

## Usage Examples

### Example 1: Monitor All Transfers

```javascript
const ws = new WebSocket('ws://localhost:9001/');

ws.onopen = () => {
  // Subscribe to all transfer events
  ws.send(JSON.stringify({
    method: 'silver_subscribeEvents',
    filter: {
      event_type: 'ObjectTransferred'
    }
  }));
};

ws.onmessage = (event) => {
  const data = JSON.parse(event.data);
  
  if (data.subscription_id) {
    console.log('Subscribed:', data.subscription_id);
  } else if (data.event_type === 'ObjectTransferred') {
    console.log('Transfer event:', data);
  }
};
```

### Example 2: Monitor Specific Address

```javascript
const ws = new WebSocket('ws://localhost:9001/');

ws.onopen = () => {
  // Subscribe to events from specific sender
  ws.send(JSON.stringify({
    method: 'silver_subscribeEvents',
    filter: {
      sender: '0x1234567890abcdef...'
    }
  }));
};

ws.onmessage = (event) => {
  const data = JSON.parse(event.data);
  console.log('Event from address:', data);
};
```

### Example 3: Monitor Specific Object

```javascript
const ws = new WebSocket('ws://localhost:9001/');

ws.onopen = () => {
  // Subscribe to events for specific object
  ws.send(JSON.stringify({
    method: 'silver_subscribeEvents',
    filter: {
      object_id: '0xabcdef1234567890...'
    }
  }));
};

ws.onmessage = (event) => {
  const data = JSON.parse(event.data);
  console.log('Event for object:', data);
};
```

### Example 4: Multiple Subscriptions

```javascript
const ws = new WebSocket('ws://localhost:9001/');
const subscriptions = [];

ws.onopen = () => {
  // Subscribe to transfers
  ws.send(JSON.stringify({
    method: 'silver_subscribeEvents',
    filter: { event_type: 'ObjectTransferred' }
  }));
  
  // Subscribe to coin operations
  ws.send(JSON.stringify({
    method: 'silver_subscribeEvents',
    filter: { event_type: 'CoinSplit' }
  }));
  
  // Subscribe to specific address
  ws.send(JSON.stringify({
    method: 'silver_subscribeEvents',
    filter: { sender: '0x1234...' }
  }));
};

ws.onmessage = (event) => {
  const data = JSON.parse(event.data);
  
  if (data.subscription_id) {
    subscriptions.push(data.subscription_id);
    console.log('Subscription created:', data.subscription_id);
  } else {
    console.log('Event received:', data);
  }
};

// Later: unsubscribe from all
function unsubscribeAll() {
  subscriptions.forEach(id => {
    ws.send(JSON.stringify({
      method: 'silver_unsubscribe',
      subscription_id: id
    }));
  });
}
```

## Integration with Node

To integrate the event subscription system into a SilverBitcoin node:

```rust
use silver_api::{RpcServer, RpcConfig, SubscriptionManager};
use silver_execution::EventEmitter;
use silver_storage::{EventStore, RocksDatabase};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize storage
    let db = Arc::new(RocksDatabase::open("./data")?);
    let event_store = Arc::new(EventStore::new(Arc::clone(&db)));
    
    // Create event emitter
    let event_emitter = Arc::new(EventEmitter::new(Arc::clone(&event_store)));
    
    // Create subscription manager
    let subscription_manager = Arc::new(SubscriptionManager::new());
    
    // Create RPC server with subscription support
    let config = RpcConfig::default();
    let mut server = RpcServer::new(config);
    
    // Start servers
    server.start().await?;
    
    // After transaction execution:
    // 1. Emit events to storage
    let event_ids = event_emitter.emit_transaction_events(
        transaction_digest,
        &execution_result,
    )?;
    
    // 2. Broadcast to subscribers
    for event in &execution_result.events {
        let storage_event = event_store.get_event(event_ids[0])?;
        if let Some(evt) = storage_event {
            subscription_manager.broadcast_event(evt, event.sender).await;
        }
    }
    
    Ok(())
}
```

## Performance Considerations

### Event Delivery Latency

- **Target**: < 500ms from finalization to delivery
- **Typical**: 50-200ms depending on network conditions
- **Factors**: Network latency, subscription count, event size

### Subscription Limits

- **Per connection**: 10 active subscriptions
- **Reason**: Prevent resource exhaustion
- **Workaround**: Use multiple connections if needed

### Event Retention

- **Default**: 30 days
- **Configurable**: Set custom retention period
- **Storage**: Events stored in RocksDB with indexes
- **Pruning**: Automatic cleanup of old events

### Scalability

- **Broadcast channel**: 10,000 event buffer
- **Concurrent connections**: Limited by system resources
- **Event throughput**: Handles 10,000+ events/second

## Error Handling

### Connection Errors

```javascript
ws.onerror = (error) => {
  console.error('WebSocket error:', error);
};

ws.onclose = (event) => {
  console.log('Connection closed:', event.code, event.reason);
  // Implement reconnection logic
};
```

### Rate Limiting

If rate limit is exceeded (100 req/s per IP):

```
HTTP 429 Too Many Requests
```

### Invalid Subscription

```json
{
  "error": "Maximum subscriptions per connection (10) exceeded"
}
```

## Best Practices

1. **Filter Wisely**: Use specific filters to reduce unnecessary events
2. **Handle Reconnection**: Implement automatic reconnection with exponential backoff
3. **Buffer Events**: Handle bursts of events with client-side buffering
4. **Unsubscribe**: Clean up subscriptions when no longer needed
5. **Monitor Latency**: Track event delivery times for performance monitoring
6. **Error Recovery**: Implement robust error handling and recovery

## Testing

### Unit Tests

```bash
cargo test --package silver-api subscriptions
cargo test --package silver-execution event_emitter
```

### Integration Tests

```bash
cargo test --package silver-api --test integration
```

### Manual Testing

Use `websocat` for manual testing:

```bash
# Connect to WebSocket
websocat ws://localhost:9001/

# Send subscription request
{"method":"silver_subscribeEvents","filter":{"event_type":"ObjectCreated"}}

# Observe events
```

## Troubleshooting

### Events Not Received

1. Check WebSocket connection is established
2. Verify subscription was created successfully
3. Check event filter matches expected events
4. Ensure transactions are being executed and finalized

### High Latency

1. Check network conditions
2. Verify node is not overloaded
3. Reduce number of subscriptions
4. Use more specific filters

### Connection Drops

1. Implement reconnection logic
2. Check firewall/proxy settings
3. Verify WebSocket timeout settings
4. Monitor server logs for errors

## API Reference

See the full API documentation:

```bash
cargo doc --package silver-api --open
```

## Support

For issues or questions:
- GitHub Issues: https://github.com/silverbitcoin/silverbitcoin-blockchain/issues
- Documentation: https://docs.silverbitcoin.org
- Community: https://discord.gg/silverbitcoin
