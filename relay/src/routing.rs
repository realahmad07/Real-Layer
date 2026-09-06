use crate::{HealthClass, RelayCandidate, RelayRanking, RelayStatus};
use ghost_layer_network::PeerId;
use std::fmt;
use std::time::SystemTime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayHop {
    pub peer_id: PeerId,
    pub address: ghost_layer_network::Multiaddr,
    pub capabilities: Vec<String>,
    pub health: HealthClass,
    pub latency_ms: Option<u64>,
    pub protocol_version: String,
    pub software_version: String,
}

impl From<&RelayCandidate> for RelayHop {
    fn from(candidate: &RelayCandidate) -> Self {
        Self {
            peer_id: candidate.peer_id,
            address: candidate.address.clone(),
            capabilities: candidate.capabilities.clone(),
            health: candidate.health,
            latency_ms: candidate.latency.map(|latency| latency.as_millis() as u64),
            protocol_version: candidate.protocol_version.clone(),
            software_version: candidate.software_version.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    OneHop { relay: RelayHop },
    TwoHop { entry: RelayHop, exit: RelayHop },
}

impl Route {
    pub fn hops(&self) -> Vec<&RelayHop> {
        match self {
            Self::OneHop { relay } => vec![relay],
            Self::TwoHop { entry, exit } => vec![entry, exit],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteSelectionPolicy {
    pub mode: ghost_layer_network::RouteMode,
    pub required_transport: String,
    pub required_capabilities: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteSelectionError {
    NoEligibleRelay,
    InsufficientRelays { eligible: usize },
    InvalidCandidate { peer_id: PeerId, reason: String },
}

impl fmt::Display for RouteSelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoEligibleRelay => formatter.write_str("no eligible relay candidates"),
            Self::InsufficientRelays { eligible } => {
                write!(
                    formatter,
                    "two-hop route requires two eligible relays, found {eligible}"
                )
            }
            Self::InvalidCandidate { peer_id, reason } => {
                write!(formatter, "invalid route candidate {peer_id}: {reason}")
            }
        }
    }
}

impl std::error::Error for RouteSelectionError {}

pub struct RouteSelector;

impl RouteSelector {
    pub fn select(
        client_peer_id: PeerId,
        candidates: &[RelayCandidate],
        policy: &RouteSelectionPolicy,
        now: SystemTime,
    ) -> Result<Route, RouteSelectionError> {
        let mut eligible = candidates
            .iter()
            .filter(|candidate| candidate.peer_id != client_peer_id)
            .filter(|candidate| candidate.status == RelayStatus::Online)
            .filter(|candidate| candidate.health == HealthClass::Healthy)
            .filter(|candidate| candidate.address.to_string().contains("/quic-v1"))
            .filter(|candidate| {
                candidate
                    .capabilities
                    .iter()
                    .any(|capability| capability == &policy.required_transport)
            })
            .filter(|candidate| {
                policy
                    .required_capabilities
                    .iter()
                    .all(|required| candidate.capabilities.contains(required))
            })
            .cloned()
            .collect::<Vec<_>>();

        if eligible.is_empty() {
            return Err(RouteSelectionError::NoEligibleRelay);
        }
        eligible = RelayRanking::rank(eligible, now);

        match policy.mode {
            ghost_layer_network::RouteMode::OneHop => Ok(Route::OneHop {
                relay: RelayHop::from(&eligible[0]),
            }),
            ghost_layer_network::RouteMode::TwoHop => {
                if eligible.len() < 2 {
                    return Err(RouteSelectionError::InsufficientRelays {
                        eligible: eligible.len(),
                    });
                }
                let entry = RelayHop::from(&eligible[0]);
                let exit = RelayHop::from(&eligible[1]);
                if entry.peer_id == exit.peer_id {
                    return Err(RouteSelectionError::InsufficientRelays {
                        eligible: eligible.len(),
                    });
                }
                Ok(Route::TwoHop { entry, exit })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ghost_layer_network::PeerId;
    use std::time::{Duration, SystemTime};

    fn candidate(latency_ms: u64, status: RelayStatus, health: HealthClass) -> RelayCandidate {
        RelayCandidate {
            peer_id: PeerId::random(),
            address: "/ip4/127.0.0.1/udp/9000/quic-v1".parse().expect("address"),
            status,
            health,
            latency: Some(Duration::from_millis(latency_ms)),
            capabilities: vec!["quic".to_owned(), "relay".to_owned()],
            protocol_version: "1.0".to_owned(),
            software_version: "test".to_owned(),
            last_seen: SystemTime::now(),
        }
    }

    fn policy(mode: ghost_layer_network::RouteMode) -> RouteSelectionPolicy {
        RouteSelectionPolicy {
            mode,
            required_transport: "quic".to_owned(),
            required_capabilities: vec!["relay".to_owned()],
        }
    }

    #[test]
    fn one_hop_selects_lowest_latency_healthy_relay() {
        let candidates = vec![
            candidate(20, RelayStatus::Online, HealthClass::Healthy),
            candidate(3, RelayStatus::Online, HealthClass::Healthy),
        ];
        let route = RouteSelector::select(
            PeerId::random(),
            &candidates,
            &policy(ghost_layer_network::RouteMode::OneHop),
            SystemTime::now(),
        )
        .expect("route");
        match route {
            Route::OneHop { relay } => assert_eq!(relay.latency_ms, Some(3)),
            Route::TwoHop { .. } => panic!("wrong route"),
        }
    }

    #[test]
    fn two_hop_selects_distinct_best_two_relays() {
        let candidates = vec![
            candidate(20, RelayStatus::Online, HealthClass::Healthy),
            candidate(3, RelayStatus::Online, HealthClass::Healthy),
            candidate(10, RelayStatus::Online, HealthClass::Healthy),
        ];
        let route = RouteSelector::select(
            PeerId::random(),
            &candidates,
            &policy(ghost_layer_network::RouteMode::TwoHop),
            SystemTime::now(),
        )
        .expect("route");
        match route {
            Route::TwoHop { entry, exit } => {
                assert_ne!(entry.peer_id, exit.peer_id);
                assert_eq!(entry.latency_ms, Some(3));
                assert_eq!(exit.latency_ms, Some(10));
            }
            Route::OneHop { .. } => panic!("wrong route"),
        }
    }

    #[test]
    fn excludes_invalid_candidates_and_client_identity() {
        let client = PeerId::random();
        let mut client_candidate = candidate(1, RelayStatus::Online, HealthClass::Healthy);
        client_candidate.peer_id = client;
        let candidates = vec![
            client_candidate,
            candidate(2, RelayStatus::Offline, HealthClass::Healthy),
            candidate(3, RelayStatus::Online, HealthClass::Degraded),
        ];
        assert_eq!(
            RouteSelector::select(
                client,
                &candidates,
                &policy(ghost_layer_network::RouteMode::OneHop),
                SystemTime::now()
            ),
            Err(RouteSelectionError::NoEligibleRelay)
        );
    }

    #[test]
    fn two_hop_reports_insufficient_relays() {
        let candidates = vec![candidate(1, RelayStatus::Online, HealthClass::Healthy)];
        assert_eq!(
            RouteSelector::select(
                PeerId::random(),
                &candidates,
                &policy(ghost_layer_network::RouteMode::TwoHop),
                SystemTime::now()
            ),
            Err(RouteSelectionError::InsufficientRelays { eligible: 1 })
        );
    }

    #[test]
    fn repeated_selection_is_deterministic() {
        let candidates = vec![
            candidate(10, RelayStatus::Online, HealthClass::Healthy),
            candidate(10, RelayStatus::Online, HealthClass::Healthy),
        ];
        let client = PeerId::random();
        let now = SystemTime::now();
        let first = RouteSelector::select(
            client,
            &candidates,
            &policy(ghost_layer_network::RouteMode::OneHop),
            now,
        )
        .expect("route");
        let second = RouteSelector::select(
            client,
            &candidates,
            &policy(ghost_layer_network::RouteMode::OneHop),
            now,
        )
        .expect("route");
        assert_eq!(first, second);
    }
}
