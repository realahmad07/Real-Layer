use ghost_layer_network::{NetworkEvent, PeerId};
use std::collections::{HashSet, VecDeque};
use std::fmt;
use std::time::{Duration, SystemTime};

const MAX_LATENCY_SAMPLES: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RelayStatus {
    Starting,
    Online,
    Degraded,
    Offline,
    ShuttingDown,
}

impl fmt::Display for RelayStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Starting => "starting",
            Self::Online => "online",
            Self::Degraded => "degraded",
            Self::Offline => "offline",
            Self::ShuttingDown => "shutting_down",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LatencyMeasurement {
    pub peer_id: PeerId,
    pub timestamp: SystemTime,
    pub round_trip: Option<Duration>,
    pub success: bool,
}

#[derive(Debug)]
pub struct RelayState {
    peer_id: PeerId,
    started_at: SystemTime,
    status: RelayStatus,
    listening_address: Option<String>,
    advertised_address: Option<String>,
    active_peers: HashSet<PeerId>,
    last_heartbeat: Option<SystemTime>,
    total_successful_connections: u64,
    total_failed_connections: u64,
    latency_measurements: VecDeque<LatencyMeasurement>,
    software_version: String,
}

impl RelayState {
    pub fn new(
        peer_id: PeerId,
        advertised_address: Option<String>,
        software_version: String,
    ) -> Self {
        Self {
            peer_id,
            started_at: SystemTime::now(),
            status: RelayStatus::Starting,
            listening_address: None,
            advertised_address,
            active_peers: HashSet::new(),
            last_heartbeat: None,
            total_successful_connections: 0,
            total_failed_connections: 0,
            latency_measurements: VecDeque::with_capacity(MAX_LATENCY_SAMPLES),
            software_version,
        }
    }

    pub fn peer_id(&self) -> &PeerId {
        &self.peer_id
    }
    pub fn status(&self) -> RelayStatus {
        self.status
    }
    pub fn uptime(&self) -> Duration {
        self.started_at.elapsed().unwrap_or_default()
    }
    pub fn listening_address(&self) -> Option<&str> {
        self.listening_address.as_deref()
    }
    pub fn advertised_address(&self) -> Option<&str> {
        self.advertised_address.as_deref()
    }
    pub fn active_peer_count(&self) -> usize {
        self.active_peers.len()
    }
    pub fn last_heartbeat(&self) -> Option<SystemTime> {
        self.last_heartbeat
    }
    pub fn total_successful_connections(&self) -> u64 {
        self.total_successful_connections
    }
    pub fn total_failed_connections(&self) -> u64 {
        self.total_failed_connections
    }
    pub fn latency_measurements(&self) -> impl Iterator<Item = &LatencyMeasurement> {
        self.latency_measurements.iter()
    }
    pub fn software_version(&self) -> &str {
        &self.software_version
    }

    pub fn mark_heartbeat(&mut self, timestamp: SystemTime) {
        self.last_heartbeat = Some(timestamp);
    }
    pub fn set_status(&mut self, status: RelayStatus) {
        self.status = status;
    }

    pub fn record_latency(&mut self, measurement: LatencyMeasurement) {
        if self.latency_measurements.len() == MAX_LATENCY_SAMPLES {
            self.latency_measurements.pop_front();
        }
        self.latency_measurements.push_back(measurement);
    }

    pub fn apply_network_event(&mut self, event: &NetworkEvent, timestamp: SystemTime) {
        match event {
            NetworkEvent::Listening { address } => {
                self.listening_address = Some(address.to_string());
                self.status = RelayStatus::Online;
            }
            NetworkEvent::PeerConnected { peer_id } => {
                if self.active_peers.insert(*peer_id) {
                    self.total_successful_connections += 1;
                }
                self.status = RelayStatus::Online;
            }
            NetworkEvent::PeerDisconnected { peer_id } => {
                self.active_peers.remove(peer_id);
            }
            NetworkEvent::PingResult { peer_id, result } => {
                self.record_latency(LatencyMeasurement {
                    peer_id: *peer_id,
                    timestamp,
                    round_trip: result.as_ref().ok().copied(),
                    success: result.is_ok(),
                });
                if result.is_err() {
                    self.total_failed_connections += 1;
                }
            }
            NetworkEvent::NetworkError { .. } => self.total_failed_connections += 1,
            NetworkEvent::PeerIdentified { .. }
            | NetworkEvent::DiscoveryRequest { .. }
            | NetworkEvent::DiscoveryResponse { .. } => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ghost_layer_network::PeerId;

    fn peer() -> PeerId {
        PeerId::random()
    }

    #[test]
    fn state_transitions_and_records_latency() {
        let peer_id = peer();
        let mut state = RelayState::new(peer_id, None, "test".to_owned());
        assert_eq!(state.status(), RelayStatus::Starting);
        state.apply_network_event(
            &NetworkEvent::Listening {
                address: "/ip4/127.0.0.1/udp/1/quic-v1".parse().unwrap(),
            },
            SystemTime::now(),
        );
        assert_eq!(state.status(), RelayStatus::Online);
        state.apply_network_event(&NetworkEvent::PeerConnected { peer_id }, SystemTime::now());
        state.apply_network_event(
            &NetworkEvent::PingResult {
                peer_id,
                result: Ok(Duration::from_millis(12)),
            },
            SystemTime::now(),
        );
        assert_eq!(state.active_peer_count(), 1);
        assert_eq!(state.latency_measurements().count(), 1);
        assert_eq!(state.total_successful_connections(), 1);
    }
}
