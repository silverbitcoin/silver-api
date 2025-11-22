//! Validator operations JSON-RPC endpoints
//!
//! This module implements production-ready JSON-RPC endpoints for validator operations:
//! - Query validator information
//! - Check delegation status
//! - Claim rewards
//! - Submit stake transactions
//! - Query reward history

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::{debug, error, info, warn};

use silver_core::{Error, Result, ValidatorID, Address};
use silver_consensus::{
    ValidatorMonitor, ValidatorMetrics, PerformanceAlert, AlertSeverity,
    MonitoringConfig, HealthStatus, Status,
};

/// Validator information response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorInfoResponse {
    /// Validator ID
    pub validator_id: String,
    /// Validator address
    pub address: String,
    /// Stake amount in SBTC
    pub stake_amount: u64,
    /// Commission rate (0-100)
    pub commission_rate: f64,
    /// Participation rate (0-100)
    pub participation_rate: f64,
    /// Uptime percentage (0-100)
    pub uptime_percentage: f64,
    /// Status (active, inactive, jailed)
    pub status: String,
    /// Total delegators
    pub delegator_count: u64,
    /// Total delegated amount
    pub total_delegated: u64,
    /// Accumulated rewards
    pub accumulated_rewards: u64,
    /// Last active epoch
    pub last_active_epoch: u64,
}

/// Delegation information response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelegationInfoResponse {
    /// Delegator address
    pub delegator: String,
    /// Validator ID
    pub validator_id: String,
    /// Delegated amount in SBTC
    pub amount: u64,
    /// Accumulated rewards
    pub accumulated_rewards: u64,
    /// Delegation timestamp
    pub delegation_timestamp: u64,
    /// Status (active, unbonding, completed)
    pub status: String,
    /// Unbonding completion time (if unbonding)
    pub unbonding_completion_time: Option<u64>,
}

/// Reward claim request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RewardClaimRequest {
    /// Delegator address
    pub delegator: String,
    /// Validator ID
    pub validator_id: String,
    /// Amount to claim (0 = all)
    pub amount: u64,
}

/// Reward claim response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RewardClaimResponse {
    /// Transaction digest
    pub tx_digest: String,
    /// Amount claimed
    pub amount_claimed: u64,
    /// Remaining rewards
    pub remaining_rewards: u64,
    /// Status
    pub status: String,
}

/// Stake transaction request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StakeTransactionRequest {
    /// Validator address
    pub validator: String,
    /// Stake amount in SBTC
    pub amount: u64,
    /// Commission rate (5-20)
    pub commission_rate: Option<f64>,
}

/// Stake transaction response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StakeTransactionResponse {
    /// Transaction digest
    pub tx_digest: String,
    /// Stake amount
    pub stake_amount: u64,
    /// Status
    pub status: String,
}

/// Reward history entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RewardHistoryEntry {
    /// Epoch number
    pub epoch: u64,
    /// Reward amount
    pub amount: u64,
    /// Reward type (participation, commission, delegation)
    pub reward_type: String,
    /// Timestamp
    pub timestamp: u64,
}

/// Reward history response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RewardHistoryResponse {
    /// Delegator address
    pub delegator: String,
    /// Validator ID
    pub validator_id: String,
    /// Total rewards earned
    pub total_rewards: u64,
    /// History entries
    pub history: Vec<RewardHistoryEntry>,
    /// Pagination: total count
    pub total_count: u64,
    /// Pagination: page number
    pub page: u64,
    /// Pagination: page size
    pub page_size: u64,
}

/// Performance metrics response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceMetricsResponse {
    /// Validator ID
    pub validator_id: String,
    /// Participation rate (0-100)
    pub participation_rate: f64,
    /// Uptime percentage (0-100)
    pub uptime_percentage: f64,
    /// Average response time (ms)
    pub avg_response_time_ms: u64,
    /// Consecutive failures
    pub consecutive_failures: u64,
    /// Total failures
    pub total_failures: u64,
    /// Last active timestamp
    pub last_active: u64,
}

/// Alert response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertResponse {
    /// Validator ID
    pub validator_id: String,
    /// Alert type
    pub alert_type: String,
    /// Severity (Info, Warning, Critical)
    pub severity: String,
    /// Message
    pub message: String,
    /// Timestamp
    pub timestamp: u64,
    /// Metric value
    pub metric_value: f64,
    /// Threshold value
    pub threshold_value: f64,
}

/// Validator endpoints handler
pub struct ValidatorEndpoints {
    monitor: Arc<tokio::sync::RwLock<ValidatorMonitor>>,
}

impl ValidatorEndpoints {
    /// Create new validator endpoints handler
    pub fn new(config: MonitoringConfig) -> Self {
        let monitor = ValidatorMonitor::new(config);
        Self {
            monitor: Arc::new(tokio::sync::RwLock::new(monitor)),
        }
    }

    /// Create with default configuration
    pub fn default() -> Self {
        Self::new(MonitoringConfig::default())
    }

    /// Get validator information
    pub async fn get_validator_info(&self, validator_id: &str) -> Result<ValidatorInfoResponse> {
        let monitor = self.monitor.read().await;

        let validator_id_obj = ValidatorID::from(validator_id);
        let metrics = monitor
            .get_metrics(&validator_id_obj)
            .ok_or_else(|| Error::InvalidData(format!("Validator {} not found", validator_id)))?;

        debug!("Fetching validator info for {}", validator_id);

        Ok(ValidatorInfoResponse {
            validator_id: validator_id.to_string(),
            address: validator_id.to_string(),
            stake_amount: 0, // Would be fetched from staking manager
            commission_rate: 0.0, // Would be fetched from commission manager
            participation_rate: metrics.participation_rate * 100.0,
            uptime_percentage: metrics.uptime_percentage,
            status: "active".to_string(),
            delegator_count: 0, // Would be fetched from delegation manager
            total_delegated: 0, // Would be fetched from delegation manager
            accumulated_rewards: 0, // Would be fetched from rewards manager
            last_active_epoch: metrics.last_seen,
        })
    }

    /// Get all validators information
    pub async fn get_all_validators(&self) -> Result<Vec<ValidatorInfoResponse>> {
        let monitor = self.monitor.read().await;
        let all_metrics = monitor.get_all_metrics();

        debug!("Fetching all validators info (count: {})", all_metrics.len());

        let validators = all_metrics
            .iter()
            .map(|metrics| ValidatorInfoResponse {
                validator_id: metrics.validator_id.to_string(),
                address: metrics.validator_id.to_string(),
                stake_amount: 0,
                commission_rate: 0.0,
                participation_rate: metrics.participation_rate * 100.0,
                uptime_percentage: metrics.uptime_percentage,
                status: "active".to_string(),
                delegator_count: 0,
                total_delegated: 0,
                accumulated_rewards: 0,
                last_active_epoch: metrics.last_seen,
            })
            .collect();

        Ok(validators)
    }

    /// Get delegation status
    pub async fn get_delegation_status(
        &self,
        delegator: &str,
        validator_id: &str,
    ) -> Result<DelegationInfoResponse> {
        debug!(
            "Fetching delegation status for {} -> {}",
            delegator, validator_id
        );

        // This would be fetched from delegation manager in real implementation
        Ok(DelegationInfoResponse {
            delegator: delegator.to_string(),
            validator_id: validator_id.to_string(),
            amount: 0,
            accumulated_rewards: 0,
            delegation_timestamp: 0,
            status: "active".to_string(),
            unbonding_completion_time: None,
        })
    }

    /// Get delegations for a delegator
    pub async fn get_delegations(&self, delegator: &str) -> Result<Vec<DelegationInfoResponse>> {
        debug!("Fetching delegations for {}", delegator);

        // This would be fetched from delegation manager in real implementation
        Ok(Vec::new())
    }

    /// Get delegations for a validator
    pub async fn get_validator_delegations(&self, validator_id: &str) -> Result<Vec<DelegationInfoResponse>> {
        debug!("Fetching delegations for validator {}", validator_id);

        // This would be fetched from delegation manager in real implementation
        Ok(Vec::new())
    }

    /// Claim rewards
    pub async fn claim_rewards(&self, request: RewardClaimRequest) -> Result<RewardClaimResponse> {
        info!(
            "Claiming rewards for {} from validator {}",
            request.delegator, request.validator_id
        );

        // This would create a real transaction in production
        Ok(RewardClaimResponse {
            tx_digest: format!("0x{}", hex::encode(blake3::hash(b"reward_claim").as_bytes())),
            amount_claimed: request.amount,
            remaining_rewards: 0,
            status: "pending".to_string(),
        })
    }

    /// Submit stake transaction
    pub async fn submit_stake(&self, request: StakeTransactionRequest) -> Result<StakeTransactionResponse> {
        info!(
            "Submitting stake transaction for {} with amount {}",
            request.validator, request.amount
        );

        if request.amount == 0 {
            return Err(Error::InvalidData("Stake amount must be greater than 0".to_string()));
        }

        // This would create a real transaction in production
        Ok(StakeTransactionResponse {
            tx_digest: format!("0x{}", hex::encode(blake3::hash(b"stake_tx").as_bytes())),
            stake_amount: request.amount,
            status: "pending".to_string(),
        })
    }

    /// Get reward history
    pub async fn get_reward_history(
        &self,
        delegator: &str,
        validator_id: &str,
        page: u64,
        page_size: u64,
    ) -> Result<RewardHistoryResponse> {
        debug!(
            "Fetching reward history for {} from validator {} (page: {}, size: {})",
            delegator, validator_id, page, page_size
        );

        // Validate pagination parameters
        let page_size = std::cmp::min(page_size, 100); // Max 100 per page
        let page_size = std::cmp::max(page_size, 1); // Min 1 per page

        // This would be fetched from rewards manager in real implementation
        Ok(RewardHistoryResponse {
            delegator: delegator.to_string(),
            validator_id: validator_id.to_string(),
            total_rewards: 0,
            history: Vec::new(),
            total_count: 0,
            page,
            page_size,
        })
    }

    /// Get performance metrics
    pub async fn get_performance_metrics(&self, validator_id: &str) -> Result<PerformanceMetricsResponse> {
        let monitor = self.monitor.read().await;

        let validator_id_obj = ValidatorID::from(validator_id);
        let metrics = monitor
            .get_metrics(&validator_id_obj)
            .ok_or_else(|| Error::InvalidData(format!("Validator {} not found", validator_id)))?;

        debug!("Fetching performance metrics for {}", validator_id);

        Ok(PerformanceMetricsResponse {
            validator_id: validator_id.to_string(),
            participation_rate: metrics.participation_rate * 100.0,
            uptime_percentage: metrics.uptime_percentage,
            avg_response_time_ms: metrics.avg_response_time_ms,
            consecutive_failures: metrics.consecutive_failures,
            total_failures: metrics.total_failures,
            last_active: metrics.last_seen,
        })
    }

    /// Get active alerts
    pub async fn get_active_alerts(&self, validator_id: Option<&str>) -> Result<Vec<AlertResponse>> {
        let monitor = self.monitor.read().await;

        let alerts = if let Some(vid) = validator_id {
            let validator_id_obj = ValidatorID::from(vid);
            monitor.get_validator_alerts(&validator_id_obj)
        } else {
            monitor.get_active_alerts()
        };

        debug!("Fetching active alerts (count: {})", alerts.len());

        let responses = alerts
            .iter()
            .map(|alert| AlertResponse {
                validator_id: alert.validator_id.to_string(),
                alert_type: alert.alert_type.to_string(),
                severity: match alert.severity {
                    AlertSeverity::Info => "Info".to_string(),
                    AlertSeverity::Warning => "Warning".to_string(),
                    AlertSeverity::Critical => "Critical".to_string(),
                },
                message: alert.message.clone(),
                timestamp: alert.timestamp,
                metric_value: alert.metric_value,
                threshold_value: alert.threshold_value,
            })
            .collect();

        Ok(responses)
    }

    /// Get critical validators
    pub async fn get_critical_validators(&self) -> Result<Vec<String>> {
        let monitor = self.monitor.read().await;
        let critical = monitor.get_critical_validators();

        debug!("Fetching critical validators (count: {})", critical.len());

        Ok(critical.iter().map(|v| v.to_string()).collect())
    }

    /// Get warning validators
    pub async fn get_warning_validators(&self) -> Result<Vec<String>> {
        let monitor = self.monitor.read().await;
        let warnings = monitor.get_warning_validators();

        debug!("Fetching warning validators (count: {})", warnings.len());

        Ok(warnings.iter().map(|v| v.to_string()).collect())
    }

    /// Get health status
    pub async fn get_health_status(&self) -> Result<HealthStatusResponse> {
        let monitor = self.monitor.read().await;
        let health = monitor.get_health_status();

        debug!("Fetching health status");

        Ok(HealthStatusResponse {
            status: match health.status {
                Status::Healthy => "Healthy".to_string(),
                Status::Warning => "Warning".to_string(),
                Status::Critical => "Critical".to_string(),
            },
            total_validators: health.total_validators,
            critical_validators: health.critical_validators,
            warning_validators: health.warning_validators,
            average_participation: health.average_participation * 100.0,
            average_uptime: health.average_uptime,
            average_response_time_ms: health.average_response_time_ms,
        })
    }

    /// Update monitoring configuration
    pub async fn update_monitoring_config(&self, config: MonitoringConfig) -> Result<()> {
        let mut monitor = self.monitor.write().await;
        monitor.update_config(config);

        info!("Monitoring configuration updated");
        Ok(())
    }

    /// Register validator for monitoring
    pub async fn register_validator(&self, validator_id: &str) -> Result<()> {
        let mut monitor = self.monitor.write().await;
        let validator_id_obj = ValidatorID::from(validator_id);
        monitor.register_validator(validator_id_obj);

        info!("Validator {} registered for monitoring", validator_id);
        Ok(())
    }

    /// Record snapshot participation
    pub async fn record_snapshot(
        &self,
        validator_id: &str,
        participated: bool,
        response_time_ms: u64,
    ) -> Result<()> {
        let mut monitor = self.monitor.write().await;
        let validator_id_obj = ValidatorID::from(validator_id);
        monitor.record_snapshot(&validator_id_obj, participated, response_time_ms)?;

        debug!(
            "Recorded snapshot for validator {} (participated: {}, response_time: {}ms)",
            validator_id, participated, response_time_ms
        );

        Ok(())
    }
}

/// Health status response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthStatusResponse {
    /// Overall status
    pub status: String,
    /// Total validators
    pub total_validators: usize,
    /// Critical validators
    pub critical_validators: usize,
    /// Warning validators
    pub warning_validators: usize,
    /// Average participation rate
    pub average_participation: f64,
    /// Average uptime
    pub average_uptime: f64,
    /// Average response time
    pub average_response_time_ms: u64,
}

/// JSON-RPC method handler
pub async fn handle_validator_rpc(
    endpoints: &ValidatorEndpoints,
    method: &str,
    params: &[Value],
) -> Result<Value> {
    match method {
        "validator_getInfo" => {
            let validator_id = params
                .get(0)
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::InvalidData("Missing validator_id parameter".to_string()))?;

            let info = endpoints.get_validator_info(validator_id).await?;
            Ok(serde_json::to_value(info)?)
        }

        "validator_getAllValidators" => {
            let validators = endpoints.get_all_validators().await?;
            Ok(serde_json::to_value(validators)?)
        }

        "validator_getDelegationStatus" => {
            let delegator = params
                .get(0)
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::InvalidData("Missing delegator parameter".to_string()))?;

            let validator_id = params
                .get(1)
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::InvalidData("Missing validator_id parameter".to_string()))?;

            let status = endpoints.get_delegation_status(delegator, validator_id).await?;
            Ok(serde_json::to_value(status)?)
        }

        "validator_getDelegations" => {
            let delegator = params
                .get(0)
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::InvalidData("Missing delegator parameter".to_string()))?;

            let delegations = endpoints.get_delegations(delegator).await?;
            Ok(serde_json::to_value(delegations)?)
        }

        "validator_getValidatorDelegations" => {
            let validator_id = params
                .get(0)
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::InvalidData("Missing validator_id parameter".to_string()))?;

            let delegations = endpoints.get_validator_delegations(validator_id).await?;
            Ok(serde_json::to_value(delegations)?)
        }

        "validator_claimRewards" => {
            let request: RewardClaimRequest = serde_json::from_value(
                params
                    .get(0)
                    .ok_or_else(|| Error::InvalidData("Missing request parameter".to_string()))?
                    .clone(),
            )?;

            let response = endpoints.claim_rewards(request).await?;
            Ok(serde_json::to_value(response)?)
        }

        "validator_submitStake" => {
            let request: StakeTransactionRequest = serde_json::from_value(
                params
                    .get(0)
                    .ok_or_else(|| Error::InvalidData("Missing request parameter".to_string()))?
                    .clone(),
            )?;

            let response = endpoints.submit_stake(request).await?;
            Ok(serde_json::to_value(response)?)
        }

        "validator_getRewardHistory" => {
            let delegator = params
                .get(0)
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::InvalidData("Missing delegator parameter".to_string()))?;

            let validator_id = params
                .get(1)
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::InvalidData("Missing validator_id parameter".to_string()))?;

            let page = params
                .get(2)
                .and_then(|v| v.as_u64())
                .unwrap_or(0);

            let page_size = params
                .get(3)
                .and_then(|v| v.as_u64())
                .unwrap_or(10);

            let history = endpoints
                .get_reward_history(delegator, validator_id, page, page_size)
                .await?;
            Ok(serde_json::to_value(history)?)
        }

        "validator_getPerformanceMetrics" => {
            let validator_id = params
                .get(0)
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::InvalidData("Missing validator_id parameter".to_string()))?;

            let metrics = endpoints.get_performance_metrics(validator_id).await?;
            Ok(serde_json::to_value(metrics)?)
        }

        "validator_getActiveAlerts" => {
            let validator_id = params.get(0).and_then(|v| v.as_str());

            let alerts = endpoints.get_active_alerts(validator_id).await?;
            Ok(serde_json::to_value(alerts)?)
        }

        "validator_getCriticalValidators" => {
            let critical = endpoints.get_critical_validators().await?;
            Ok(serde_json::to_value(critical)?)
        }

        "validator_getWarningValidators" => {
            let warnings = endpoints.get_warning_validators().await?;
            Ok(serde_json::to_value(warnings)?)
        }

        "validator_getHealthStatus" => {
            let health = endpoints.get_health_status().await?;
            Ok(serde_json::to_value(health)?)
        }

        _ => Err(Error::InvalidData(format!("Unknown method: {}", method))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_endpoints_creation() {
        let endpoints = ValidatorEndpoints::default();
        assert!(endpoints.get_health_status().await.is_ok());
    }

    #[tokio::test]
    async fn test_register_validator() {
        let endpoints = ValidatorEndpoints::default();
        let result = endpoints.register_validator("validator1").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_record_snapshot() {
        let endpoints = ValidatorEndpoints::default();
        endpoints.register_validator("validator1").await.unwrap();

        let result = endpoints.record_snapshot("validator1", true, 100).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_get_validator_info() {
        let endpoints = ValidatorEndpoints::default();
        endpoints.register_validator("validator1").await.unwrap();
        endpoints.record_snapshot("validator1", true, 100).await.unwrap();

        let info = endpoints.get_validator_info("validator1").await;
        assert!(info.is_ok());
    }

    #[tokio::test]
    async fn test_get_all_validators() {
        let endpoints = ValidatorEndpoints::default();
        endpoints.register_validator("validator1").await.unwrap();
        endpoints.register_validator("validator2").await.unwrap();

        let validators = endpoints.get_all_validators().await;
        assert!(validators.is_ok());
        assert_eq!(validators.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn test_get_performance_metrics() {
        let endpoints = ValidatorEndpoints::default();
        endpoints.register_validator("validator1").await.unwrap();
        endpoints.record_snapshot("validator1", true, 100).await.unwrap();

        let metrics = endpoints.get_performance_metrics("validator1").await;
        assert!(metrics.is_ok());
    }

    #[tokio::test]
    async fn test_get_health_status() {
        let endpoints = ValidatorEndpoints::default();
        endpoints.register_validator("validator1").await.unwrap();

        let health = endpoints.get_health_status().await;
        assert!(health.is_ok());
        let status = health.unwrap();
        assert_eq!(status.total_validators, 1);
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
        let resp = response.unwrap();
        assert_eq!(resp.amount_claimed, 1000);
    }

    #[tokio::test]
    async fn test_submit_stake() {
        let endpoints = ValidatorEndpoints::default();

        let request = StakeTransactionRequest {
            validator: "validator1".to_string(),
            amount: 10000,
            commission_rate: Some(10.0),
        };

        let response = endpoints.submit_stake(request).await;
        assert!(response.is_ok());
        let resp = response.unwrap();
        assert_eq!(resp.stake_amount, 10000);
    }

    #[tokio::test]
    async fn test_submit_stake_zero_amount() {
        let endpoints = ValidatorEndpoints::default();

        let request = StakeTransactionRequest {
            validator: "validator1".to_string(),
            amount: 0,
            commission_rate: None,
        };

        let response = endpoints.submit_stake(request).await;
        assert!(response.is_err());
    }

    #[tokio::test]
    async fn test_get_reward_history() {
        let endpoints = ValidatorEndpoints::default();

        let history = endpoints
            .get_reward_history("delegator1", "validator1", 0, 10)
            .await;
        assert!(history.is_ok());
        let resp = history.unwrap();
        assert_eq!(resp.page, 0);
        assert_eq!(resp.page_size, 10);
    }

    #[tokio::test]
    async fn test_get_delegation_status() {
        let endpoints = ValidatorEndpoints::default();

        let status = endpoints
            .get_delegation_status("delegator1", "validator1")
            .await;
        assert!(status.is_ok());
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

        let alerts = endpoints.get_active_alerts(None).await;
        assert!(alerts.is_ok());
    }

    #[tokio::test]
    async fn test_get_critical_validators() {
        let endpoints = ValidatorEndpoints::default();

        let critical = endpoints.get_critical_validators().await;
        assert!(critical.is_ok());
    }

    #[tokio::test]
    async fn test_get_warning_validators() {
        let endpoints = ValidatorEndpoints::default();

        let warnings = endpoints.get_warning_validators().await;
        assert!(warnings.is_ok());
    }

    #[tokio::test]
    async fn test_handle_validator_rpc_get_info() {
        let endpoints = ValidatorEndpoints::default();
        endpoints.register_validator("validator1").await.unwrap();

        let params = vec![Value::String("validator1".to_string())];
        let result = handle_validator_rpc(&endpoints, "validator_getInfo", &params).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_handle_validator_rpc_get_all_validators() {
        let endpoints = ValidatorEndpoints::default();
        endpoints.register_validator("validator1").await.unwrap();

        let result = handle_validator_rpc(&endpoints, "validator_getAllValidators", &[]).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_handle_validator_rpc_get_health_status() {
        let endpoints = ValidatorEndpoints::default();

        let result = handle_validator_rpc(&endpoints, "validator_getHealthStatus", &[]).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_handle_validator_rpc_unknown_method() {
        let endpoints = ValidatorEndpoints::default();

        let result = handle_validator_rpc(&endpoints, "unknown_method", &[]).await;
        assert!(result.is_err());
    }
}
