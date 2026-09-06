use crate::{RelayState, RelayStatus};
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum HealthClass {
    Healthy,
    Degraded,
    Unhealthy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HealthThresholds {
    pub heartbeat_freshness: Duration,
    pub degraded_latency: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelayHealth {
    pub class: HealthClass,
    pub latency: Option<Duration>,
    pub heartbeat_fresh: bool,
}

impl RelayHealth {
    pub fn evaluate(state: &RelayState, thresholds: HealthThresholds, now: SystemTime) -> Self {
        let heartbeat_fresh = state.last_heartbeat().is_some_and(|timestamp| {
            now.duration_since(timestamp).unwrap_or_default() <= thresholds.heartbeat_freshness
        });
        let latency = state
            .latency_measurements()
            .filter_map(|measurement| measurement.round_trip)
            .last();
        let connectivity_ok =
            state.active_peer_count() > 0 || state.total_successful_connections() == 0;
        let class = if matches!(
            state.status(),
            RelayStatus::Offline | RelayStatus::ShuttingDown
        ) || !heartbeat_fresh && state.last_heartbeat().is_some()
            || !connectivity_ok
        {
            HealthClass::Unhealthy
        } else if latency.is_some_and(|value| value > thresholds.degraded_latency)
            || state.status() == RelayStatus::Degraded
        {
            HealthClass::Degraded
        } else {
            HealthClass::Healthy
        };
        Self {
            class,
            latency,
            heartbeat_fresh,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_heartbeat_is_unhealthy() {
        let peer_id = ghost_layer_network::PeerId::random();
        let mut state = RelayState::new(peer_id, None, "test".to_owned());
        let now = SystemTime::now();
        state.mark_heartbeat(now - Duration::from_secs(60));
        let health = RelayHealth::evaluate(
            &state,
            HealthThresholds {
                heartbeat_freshness: Duration::from_secs(10),
                degraded_latency: Duration::from_secs(1),
            },
            now,
        );
        assert_eq!(health.class, HealthClass::Unhealthy);
    }
}
