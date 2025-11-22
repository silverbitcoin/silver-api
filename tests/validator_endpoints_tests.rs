use silver_api::{
    ValidatorEndpoints, ValidatorInfoResponse, DelegationInfoResponse,
    RewardClaimRequest, StakeTransactionRequest, handle_validator_rpc,
};
use serde_json::json;

#[tokio::test]
async fn test_validator_endpoints_creation() {
    let endpoints = ValidatorEndpoints::default();
    let health = endpoints.get_health_status().await;
    assert!(health.is_ok());
}

#[tokio::test]
async fn test_register_and_query_validator() {
    let endpoints = ValidatorEndpoints::default();

    // Register validator
    assert!(endpoints.register_validator("validator1").await.is_ok());

    // Record some snapshots
    for _ in 0..100 {
        assert!(endpoints.record_snapshot("validator1", true, 50).await.is_ok());
    }

    // Query validator info
    let info = endpoints.get_validator_info("validator1").await;
    assert!(info.is_ok());

    let validator_info = info.unwrap();
    assert_eq!(validator_info.validator_id, "validator1");
    assert_eq!(validator_info.participation_rate, 100.0);
}

#[tokio::test]
async fn test_multiple_validators() {
    let endpoints = ValidatorEndpoints::default();

    // Register multiple validators
    for i in 1..=5 {
        let validator_id = format!("validator{}", i);
        assert!(endpoints.register_validator(&validator_id).await.is_ok());

        // Record snapshots with different participation rates
        let participation = 100 - (i as u64 * 10);
        for _ in 0..participation {
            assert!(endpoints
                .record_snapshot(&validator_id, true, 50)
                .await
                .is_ok());
        }
        for _ in 0..(100 - participation) {
            assert!(endpoints
                .record_snapshot(&validator_id, false, 0)
                .await
                .is_ok());
        }
    }

    // Get all validators
    let validators = endpoints.get_all_validators().await;
    assert!(validators.is_ok());
    assert_eq!(validators.unwrap().len(), 5);
}

#[tokio::test]
async fn test_performance_metrics() {
    let endpoints = ValidatorEndpoints::default();

    endpoints.register_validator("validator1").await.unwrap();

    // Record snapshots with varying response times
    endpoints.record_snapshot("validator1", true, 100).await.unwrap();
    endpoints.record_snapshot("validator1", true, 200).await.unwrap();
    endpoints.record_snapshot("validator1", true, 150).await.unwrap();

    let metrics = endpoints.get_performance_metrics("validator1").await;
    assert!(metrics.is_ok());

    let perf = metrics.unwrap();
    assert_eq!(perf.avg_response_time_ms, 150);
}

#[tokio::test]
async fn test_health_status() {
    let endpoints = ValidatorEndpoints::default();

    // Register validators with different health states
    endpoints.register_validator("validator1").await.unwrap();
    endpoints.register_validator("validator2").await.unwrap();

    // Healthy validator
    for _ in 0..100 {
        endpoints.record_snapshot("validator1", true, 50).await.unwrap();
    }

    // Degraded validator
    for _ in 0..85 {
        endpoints.record_snapshot("validator2", true, 50).await.unwrap();
    }
    for _ in 0..15 {
        endpoints.record_snapshot("validator2", false, 0).await.unwrap();
    }

    let health = endpoints.get_health_status().await;
    assert!(health.is_ok());

    let status = health.unwrap();
    assert_eq!(status.total_validators, 2);
    assert!(status.average_participation > 0.0);
}

#[tokio::test]
async fn test_claim_rewards() {
    let endpoints = ValidatorEndpoints::default();

    let request = RewardClaimRequest {
        delegator: "delegator1".to_string(),
        validator_id: "validator1".to_string(),
        amount: 1000,
    };

    let response = endpoints.claim_rewards(request).await;
    assert!(response.is_ok());

    let claim = response.unwrap();
    assert_eq!(claim.amount_claimed, 1000);
    assert_eq!(claim.status, "pending");
    assert!(!claim.tx_digest.is_empty());
}

#[tokio::test]
async fn test_submit_stake() {
    let endpoints = ValidatorEndpoints::default();

    let request = StakeTransactionRequest {
        validator: "validator1".to_string(),
        amount: 50000,
        commission_rate: Some(10.0),
    };

    let response = endpoints.submit_stake(request).await;
    assert!(response.is_ok());

    let stake = response.unwrap();
    assert_eq!(stake.stake_amount, 50000);
    assert_eq!(stake.status, "pending");
    assert!(!stake.tx_digest.is_empty());
}

#[tokio::test]
async fn test_submit_stake_validation() {
    let endpoints = ValidatorEndpoints::default();

    // Test zero amount
    let request = StakeTransactionRequest {
        validator: "validator1".to_string(),
        amount: 0,
        commission_rate: None,
    };

    let response = endpoints.submit_stake(request).await;
    assert!(response.is_err());
}

#[tokio::test]
async fn test_reward_history() {
    let endpoints = ValidatorEndpoints::default();

    let history = endpoints
        .get_reward_history("delegator1", "validator1", 0, 10)
        .await;
    assert!(history.is_ok());

    let resp = history.unwrap();
    assert_eq!(resp.delegator, "delegator1");
    assert_eq!(resp.validator_id, "validator1");
    assert_eq!(resp.page, 0);
    assert_eq!(resp.page_size, 10);
}

#[tokio::test]
async fn test_reward_history_pagination() {
    let endpoints = ValidatorEndpoints::default();

    // Test with large page size (should be capped at 100)
    let history = endpoints
        .get_reward_history("delegator1", "validator1", 0, 500)
        .await;
    assert!(history.is_ok());

    let resp = history.unwrap();
    assert!(resp.page_size <= 100);
}

#[tokio::test]
async fn test_delegation_status() {
    let endpoints = ValidatorEndpoints::default();

    let status = endpoints
        .get_delegation_status("delegator1", "validator1")
        .await;
    assert!(status.is_ok());

    let delegation = status.unwrap();
    assert_eq!(delegation.delegator, "delegator1");
    assert_eq!(delegation.validator_id, "validator1");
}

#[tokio::test]
async fn test_get_delegations() {
    let endpoints = ValidatorEndpoints::default();

    let delegations = endpoints.get_delegations("delegator1").await;
    assert!(delegations.is_ok());
}

#[tokio::test]
async fn test_get_validator_delegations() {
    let endpoints = ValidatorEndpoints::default();

    let delegations = endpoints.get_validator_delegations("validator1").await;
    assert!(delegations.is_ok());
}

#[tokio::test]
async fn test_get_active_alerts() {
    let endpoints = ValidatorEndpoints::default();

    endpoints.register_validator("validator1").await.unwrap();

    // Record poor performance to trigger alerts
    for _ in 0..85 {
        endpoints.record_snapshot("validator1", true, 50).await.unwrap();
    }
    for _ in 0..15 {
        endpoints.record_snapshot("validator1", false, 0).await.unwrap();
    }

    let alerts = endpoints.get_active_alerts(None).await;
    assert!(alerts.is_ok());
}

#[tokio::test]
async fn test_get_active_alerts_for_validator() {
    let endpoints = ValidatorEndpoints::default();

    endpoints.register_validator("validator1").await.unwrap();

    let alerts = endpoints.get_active_alerts(Some("validator1")).await;
    assert!(alerts.is_ok());
}

#[tokio::test]
async fn test_get_critical_validators() {
    let endpoints = ValidatorEndpoints::default();

    endpoints.register_validator("validator1").await.unwrap();

    // Record poor performance
    for _ in 0..85 {
        endpoints.record_snapshot("validator1", true, 50).await.unwrap();
    }
    for _ in 0..15 {
        endpoints.record_snapshot("validator1", false, 0).await.unwrap();
    }

    let critical = endpoints.get_critical_validators().await;
    assert!(critical.is_ok());
}

#[tokio::test]
async fn test_get_warning_validators() {
    let endpoints = ValidatorEndpoints::default();

    endpoints.register_validator("validator1").await.unwrap();

    // Record moderate performance degradation
    for _ in 0..93 {
        endpoints.record_snapshot("validator1", true, 50).await.unwrap();
    }
    for _ in 0..7 {
        endpoints.record_snapshot("validator1", false, 0).await.unwrap();
    }

    let warnings = endpoints.get_warning_validators().await;
    assert!(warnings.is_ok());
}

#[tokio::test]
async fn test_rpc_handler_get_info() {
    let endpoints = ValidatorEndpoints::default();
    endpoints.register_validator("validator1").await.unwrap();

    let params = vec![serde_json::Value::String("validator1".to_string())];
    let result = handle_validator_rpc(&endpoints, "validator_getInfo", &params).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_rpc_handler_get_all_validators() {
    let endpoints = ValidatorEndpoints::default();
    endpoints.register_validator("validator1").await.unwrap();
    endpoints.register_validator("validator2").await.unwrap();

    let result = handle_validator_rpc(&endpoints, "validator_getAllValidators", &[]).await;
    assert!(result.is_ok());

    let value = result.unwrap();
    assert!(value.is_array());
}

#[tokio::test]
async fn test_rpc_handler_get_delegation_status() {
    let endpoints = ValidatorEndpoints::default();

    let params = vec![
        serde_json::Value::String("delegator1".to_string()),
        serde_json::Value::String("validator1".to_string()),
    ];
    let result = handle_validator_rpc(&endpoints, "validator_getDelegationStatus", &params).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_rpc_handler_get_delegations() {
    let endpoints = ValidatorEndpoints::default();

    let params = vec![serde_json::Value::String("delegator1".to_string())];
    let result = handle_validator_rpc(&endpoints, "validator_getDelegations", &params).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_rpc_handler_get_validator_delegations() {
    let endpoints = ValidatorEndpoints::default();

    let params = vec![serde_json::Value::String("validator1".to_string())];
    let result = handle_validator_rpc(&endpoints, "validator_getValidatorDelegations", &params).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_rpc_handler_claim_rewards() {
    let endpoints = ValidatorEndpoints::default();

    let request = json!({
        "delegator": "delegator1",
        "validator_id": "validator1",
        "amount": 1000
    });

    let params = vec![request];
    let result = handle_validator_rpc(&endpoints, "validator_claimRewards", &params).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_rpc_handler_submit_stake() {
    let endpoints = ValidatorEndpoints::default();

    let request = json!({
        "validator": "validator1",
        "amount": 50000,
        "commission_rate": 10.0
    });

    let params = vec![request];
    let result = handle_validator_rpc(&endpoints, "validator_submitStake", &params).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_rpc_handler_get_reward_history() {
    let endpoints = ValidatorEndpoints::default();

    let params = vec![
        serde_json::Value::String("delegator1".to_string()),
        serde_json::Value::String("validator1".to_string()),
        serde_json::Value::Number(0.into()),
        serde_json::Value::Number(10.into()),
    ];
    let result = handle_validator_rpc(&endpoints, "validator_getRewardHistory", &params).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_rpc_handler_get_performance_metrics() {
    let endpoints = ValidatorEndpoints::default();
    endpoints.register_validator("validator1").await.unwrap();

    let params = vec![serde_json::Value::String("validator1".to_string())];
    let result = handle_validator_rpc(&endpoints, "validator_getPerformanceMetrics", &params).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_rpc_handler_get_active_alerts() {
    let endpoints = ValidatorEndpoints::default();

    let result = handle_validator_rpc(&endpoints, "validator_getActiveAlerts", &[]).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_rpc_handler_get_critical_validators() {
    let endpoints = ValidatorEndpoints::default();

    let result = handle_validator_rpc(&endpoints, "validator_getCriticalValidators", &[]).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_rpc_handler_get_warning_validators() {
    let endpoints = ValidatorEndpoints::default();

    let result = handle_validator_rpc(&endpoints, "validator_getWarningValidators", &[]).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_rpc_handler_get_health_status() {
    let endpoints = ValidatorEndpoints::default();

    let result = handle_validator_rpc(&endpoints, "validator_getHealthStatus", &[]).await;
    assert!(result.is_ok());

    let value = result.unwrap();
    assert!(value.is_object());
}

#[tokio::test]
async fn test_rpc_handler_unknown_method() {
    let endpoints = ValidatorEndpoints::default();

    let result = handle_validator_rpc(&endpoints, "unknown_method", &[]).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_rpc_handler_missing_parameters() {
    let endpoints = ValidatorEndpoints::default();

    let result = handle_validator_rpc(&endpoints, "validator_getInfo", &[]).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_concurrent_operations() {
    let endpoints = std::sync::Arc::new(ValidatorEndpoints::default());

    let mut handles = vec![];

    // Spawn multiple concurrent operations
    for i in 0..10 {
        let endpoints_clone = std::sync::Arc::clone(&endpoints);
        let handle = tokio::spawn(async move {
            let validator_id = format!("validator{}", i);
            endpoints_clone.register_validator(&validator_id).await.unwrap();

            for _ in 0..10 {
                endpoints_clone
                    .record_snapshot(&validator_id, true, 50)
                    .await
                    .unwrap();
            }

            endpoints_clone.get_validator_info(&validator_id).await
        });

        handles.push(handle);
    }

    // Wait for all operations to complete
    for handle in handles {
        let result = handle.await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_ok());
    }
}

#[tokio::test]
async fn test_validator_info_response_structure() {
    let endpoints = ValidatorEndpoints::default();
    endpoints.register_validator("validator1").await.unwrap();

    for _ in 0..100 {
        endpoints.record_snapshot("validator1", true, 50).await.unwrap();
    }

    let info = endpoints.get_validator_info("validator1").await.unwrap();

    assert!(!info.validator_id.is_empty());
    assert!(!info.address.is_empty());
    assert!(info.participation_rate >= 0.0 && info.participation_rate <= 100.0);
    assert!(info.uptime_percentage >= 0.0 && info.uptime_percentage <= 100.0);
}

#[tokio::test]
async fn test_health_status_response_structure() {
    let endpoints = ValidatorEndpoints::default();
    endpoints.register_validator("validator1").await.unwrap();

    let health = endpoints.get_health_status().await.unwrap();

    assert!(!health.status.is_empty());
    assert!(health.total_validators > 0);
    assert!(health.average_participation >= 0.0 && health.average_participation <= 100.0);
    assert!(health.average_uptime >= 0.0 && health.average_uptime <= 100.0);
}
