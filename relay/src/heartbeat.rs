use crate::{RelayState, RelayStatus};
use ghost_layer_network::PeerId;
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayHeartbeat {
    pub peer_id: PeerId,
    pub timestamp: SystemTime,
    pub status: RelayStatus,
    pub uptime: Duration,
    pub active_peer_count: usize,
    pub latency: Option<Duration>,
    pub software_version: String,
}

impl RelayHeartbeat {
    pub fn from_state(state: &mut RelayState, timestamp: SystemTime) -> Self {
        state.mark_heartbeat(timestamp);
        Self {
            peer_id: *state.peer_id(),
            timestamp,
            status: state.status(),
            uptime: state.uptime(),
            active_peer_count: state.active_peer_count(),
            latency: state
                .latency_measurements()
                .filter_map(|measurement| measurement.round_trip)
                .last(),
            software_version: state.software_version().to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heartbeat_updates_timestamp_and_contains_state() {
        let peer_id = ghost_layer_network::PeerId::random();
        let mut state = RelayState::new(peer_id, None, "test".to_owned());
        let timestamp = SystemTime::UNIX_EPOCH + Duration::from_secs(42);
        let heartbeat = RelayHeartbeat::from_state(&mut state, timestamp);
        assert_eq!(heartbeat.peer_id, peer_id);
        assert_eq!(heartbeat.timestamp, timestamp);
        assert_eq!(state.last_heartbeat(), Some(timestamp));
    }
}
