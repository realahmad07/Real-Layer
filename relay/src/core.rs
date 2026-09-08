use ghost_layer_network::{PeerId, RelayResourceLimits, SessionId};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayLifecycle {
    Starting,
    Discovering,
    Ready,
    Draining,
    ShuttingDown,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayCoreError {
    InvalidTransition {
        state: RelayLifecycle,
        operation: &'static str,
    },
    PeerLimit,
    SessionLimit,
    FlowLimit,
}

impl fmt::Display for RelayCoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}
impl std::error::Error for RelayCoreError {}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayMetrics {
    pub active_peers: usize,
    pub active_sessions: usize,
    pub active_flows: usize,
    pub packets_accepted: u64,
    pub packets_rejected: u64,
    pub bytes_forwarded: u64,
    pub tcp_flows: u64,
    pub udp_flows: u64,
    pub dns_requests: u64,
    pub forwarding_errors: u64,
    pub session_errors: u64,
    pub connection_errors: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayOperationalState {
    pub peer_id: String,
    pub software_version: String,
    pub protocol_version: String,
    pub capabilities: Vec<String>,
    pub lifecycle: String,
    pub readiness: bool,
    pub uptime_secs: u64,
    pub active_sessions: usize,
    pub active_flows: usize,
    pub active_peers: usize,
    pub metrics: RelayMetrics,
}

pub struct RelayCore {
    peer_id: PeerId,
    software_version: String,
    protocol_version: String,
    capabilities: Vec<String>,
    limits: RelayResourceLimits,
    lifecycle: RelayLifecycle,
    started_at: Instant,
    peers: HashSet<PeerId>,
    sessions: HashSet<SessionId>,
    flows: HashSet<SessionId>,
    metrics: RelayMetrics,
}

pub type RelayRuntime = RelayCore;

impl RelayCore {
    pub fn new(
        peer_id: PeerId,
        software_version: String,
        protocol_version: String,
        capabilities: Vec<String>,
        limits: RelayResourceLimits,
    ) -> Result<Self, RelayCoreError> {
        Ok(Self {
            peer_id,
            software_version,
            protocol_version,
            capabilities,
            limits,
            lifecycle: RelayLifecycle::Starting,
            started_at: Instant::now(),
            peers: HashSet::new(),
            sessions: HashSet::new(),
            flows: HashSet::new(),
            metrics: RelayMetrics::default(),
        })
    }

    pub fn lifecycle(&self) -> RelayLifecycle {
        self.lifecycle
    }
    pub fn metrics(&self) -> RelayMetrics {
        self.metrics
    }
    pub fn metrics_mut(&mut self) -> &mut RelayMetrics {
        &mut self.metrics
    }
    pub fn is_ready(&self) -> bool {
        self.lifecycle == RelayLifecycle::Ready
    }

    pub fn transition(&mut self, next: RelayLifecycle) -> Result<(), RelayCoreError> {
        let valid = matches!(
            (self.lifecycle, next),
            (RelayLifecycle::Starting, RelayLifecycle::Discovering)
                | (RelayLifecycle::Discovering, RelayLifecycle::Ready)
                | (RelayLifecycle::Ready, RelayLifecycle::Draining)
                | (RelayLifecycle::Ready, RelayLifecycle::ShuttingDown)
                | (RelayLifecycle::Draining, RelayLifecycle::ShuttingDown)
                | (RelayLifecycle::ShuttingDown, RelayLifecycle::Stopped)
                | (RelayLifecycle::Draining, RelayLifecycle::Draining)
                | (RelayLifecycle::ShuttingDown, RelayLifecycle::ShuttingDown)
                | (RelayLifecycle::Stopped, RelayLifecycle::Stopped)
        );
        if !valid {
            return Err(RelayCoreError::InvalidTransition {
                state: self.lifecycle,
                operation: "lifecycle transition",
            });
        }
        self.lifecycle = next;
        Ok(())
    }

    pub fn admit_peer(&mut self, peer: PeerId) -> Result<(), RelayCoreError> {
        if self.peers.contains(&peer) {
            return Ok(());
        }
        if self.lifecycle != RelayLifecycle::Ready && self.lifecycle != RelayLifecycle::Discovering
        {
            return Err(RelayCoreError::InvalidTransition {
                state: self.lifecycle,
                operation: "admit peer",
            });
        }
        if self.peers.len() >= self.limits.maximum_connected_peers {
            return Err(RelayCoreError::PeerLimit);
        }
        self.peers.insert(peer);
        self.metrics.active_peers = self.peers.len();
        Ok(())
    }

    pub fn remove_peer(&mut self, peer: &PeerId) {
        self.peers.remove(peer);
        self.metrics.active_peers = self.peers.len();
    }
    pub fn admit_session(&mut self, session: SessionId) -> Result<(), RelayCoreError> {
        if self.lifecycle != RelayLifecycle::Ready && self.lifecycle != RelayLifecycle::Discovering
        {
            return Err(RelayCoreError::InvalidTransition {
                state: self.lifecycle,
                operation: "admit session",
            });
        }
        if self.sessions.len() >= self.limits.maximum_active_sessions {
            return Err(RelayCoreError::SessionLimit);
        }
        self.sessions.insert(session);
        self.metrics.active_sessions = self.sessions.len();
        Ok(())
    }
    pub fn remove_session(&mut self, session: &SessionId) {
        self.sessions.remove(session);
        self.metrics.active_sessions = self.sessions.len();
        self.flows.remove(session);
        self.metrics.active_flows = self.flows.len();
    }
    pub fn admit_flow(&mut self, flow: SessionId, bytes: usize) -> Result<(), RelayCoreError> {
        if self.flows.contains(&flow) {
            return Ok(());
        }
        if self.flows.len() >= self.limits.maximum_active_flows {
            return Err(RelayCoreError::FlowLimit);
        }
        self.flows.insert(flow);
        self.metrics.active_flows = self.flows.len();
        self.metrics.packets_accepted += 1;
        self.metrics.bytes_forwarded = self.metrics.bytes_forwarded.saturating_add(bytes as u64);
        Ok(())
    }
    pub fn remove_flow(&mut self, flow: &SessionId) {
        self.flows.remove(flow);
        self.metrics.active_flows = self.flows.len();
    }
    pub fn reject_packet(&mut self) {
        self.metrics.packets_rejected += 1;
    }
    pub fn operational_state(&self) -> RelayOperationalState {
        RelayOperationalState {
            peer_id: self.peer_id.to_string(),
            software_version: self.software_version.clone(),
            protocol_version: self.protocol_version.clone(),
            capabilities: self.capabilities.clone(),
            lifecycle: format!("{:?}", self.lifecycle),
            readiness: self.is_ready(),
            uptime_secs: self.started_at.elapsed().as_secs(),
            active_sessions: self.sessions.len(),
            active_flows: self.flows.len(),
            active_peers: self.peers.len(),
            metrics: self.metrics,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    fn core() -> RelayCore {
        RelayCore::new(
            PeerId::random(),
            "test".into(),
            "1.0".into(),
            vec!["quic".into()],
            RelayResourceLimits {
                maximum_connected_peers: 1,
                maximum_active_sessions: 1,
                maximum_active_flows: 1,
                maximum_queued_packets: 1,
                maximum_exit_connections: 1,
                session_timeout: Duration::from_secs(1),
                flow_timeout: Duration::from_secs(1),
            },
        )
        .unwrap()
    }
    #[test]
    fn lifecycle_is_deterministic_and_shutdown_idempotent() {
        let mut core = core();
        core.transition(RelayLifecycle::Discovering).unwrap();
        core.transition(RelayLifecycle::Ready).unwrap();
        core.transition(RelayLifecycle::Draining).unwrap();
        core.transition(RelayLifecycle::ShuttingDown).unwrap();
        core.transition(RelayLifecycle::ShuttingDown).unwrap();
        core.transition(RelayLifecycle::Stopped).unwrap();
        core.transition(RelayLifecycle::Stopped).unwrap();
    }
    #[test]
    fn limits_reject_and_cleanup_resources() {
        let mut core = core();
        core.transition(RelayLifecycle::Discovering).unwrap();
        core.transition(RelayLifecycle::Ready).unwrap();
        let peer = PeerId::random();
        core.admit_peer(peer).unwrap();
        assert_eq!(
            core.admit_peer(PeerId::random()),
            Err(RelayCoreError::PeerLimit)
        );
        let session = SessionId::generate();
        core.admit_session(session).unwrap();
        assert_eq!(
            core.admit_session(SessionId::generate()),
            Err(RelayCoreError::SessionLimit)
        );
        core.admit_flow(session, 10).unwrap();
        assert_eq!(
            core.admit_flow(SessionId::generate(), 10),
            Err(RelayCoreError::FlowLimit)
        );
        core.remove_session(&session);
        assert_eq!(core.metrics().active_flows, 0);
    }
}
