use crate::{HealthClass, RelayMetadata, RelayStatus};
use ghost_layer_network::{Multiaddr, PeerId};
use std::cmp::Ordering;
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayCandidate {
    pub peer_id: PeerId,
    pub address: Multiaddr,
    pub status: RelayStatus,
    pub health: HealthClass,
    pub latency: Option<Duration>,
    pub capabilities: Vec<String>,
    pub protocol_version: String,
    pub software_version: String,
    pub last_seen: SystemTime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateRequirements {
    pub protocol_version: String,
    pub required_transport: String,
    pub required_capabilities: Vec<String>,
    pub heartbeat_freshness: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayCandidateError {
    InvalidMetadata(String),
    Ineligible(String),
}

impl RelayCandidate {
    pub fn from_metadata(
        metadata: &RelayMetadata,
        requirements: &CandidateRequirements,
        now: SystemTime,
    ) -> Result<Self, RelayCandidateError> {
        metadata
            .validate(
                &requirements.protocol_version,
                now,
                requirements.heartbeat_freshness,
            )
            .map_err(RelayCandidateError::InvalidMetadata)?;
        let address_text = metadata
            .advertised_address
            .as_deref()
            .or(metadata.listening_address.as_deref())
            .ok_or_else(|| RelayCandidateError::InvalidMetadata("missing address".to_owned()))?;
        let candidate = Self {
            peer_id: metadata.peer_id,
            address: address_text
                .parse()
                .map_err(|_| RelayCandidateError::InvalidMetadata("invalid address".to_owned()))?,
            status: metadata.status,
            health: metadata.health,
            latency: metadata.latency_ms.map(Duration::from_millis),
            capabilities: metadata.capabilities.clone(),
            protocol_version: metadata.protocol_version.clone(),
            software_version: metadata.software_version.clone(),
            last_seen: metadata.timestamp,
        };
        if candidate.status == RelayStatus::Offline {
            return Err(RelayCandidateError::Ineligible(
                "relay is offline".to_owned(),
            ));
        }
        if !metadata
            .supported_transports
            .iter()
            .any(|transport| transport == &requirements.required_transport)
        {
            return Err(RelayCandidateError::Ineligible(
                "required transport is unavailable".to_owned(),
            ));
        }
        if let Some(missing) = requirements
            .required_capabilities
            .iter()
            .find(|capability| !candidate.capabilities.contains(capability))
        {
            return Err(RelayCandidateError::Ineligible(format!(
                "required capability is unavailable: {missing}"
            )));
        }
        Ok(candidate)
    }
}

pub struct RelayRanking;

impl RelayRanking {
    pub fn rank(mut candidates: Vec<RelayCandidate>, now: SystemTime) -> Vec<RelayCandidate> {
        candidates.sort_by(|left, right| Self::compare(left, right, now));
        candidates
    }

    fn compare(left: &RelayCandidate, right: &RelayCandidate, now: SystemTime) -> Ordering {
        health_rank(right.health)
            .cmp(&health_rank(left.health))
            .then_with(|| status_rank(right.status).cmp(&status_rank(left.status)))
            .then_with(|| latency_rank(left.latency).cmp(&latency_rank(right.latency)))
            .then_with(|| {
                freshness_rank(right.last_seen, now).cmp(&freshness_rank(left.last_seen, now))
            })
            .then_with(|| left.peer_id.to_string().cmp(&right.peer_id.to_string()))
    }
}

fn status_rank(status: RelayStatus) -> u8 {
    matches!(status, RelayStatus::Online) as u8
}

fn health_rank(health: HealthClass) -> u8 {
    match health {
        HealthClass::Healthy => 2,
        HealthClass::Degraded => 1,
        HealthClass::Unhealthy => 0,
    }
}

fn latency_rank(latency: Option<Duration>) -> Duration {
    latency.unwrap_or(Duration::from_secs(u64::MAX))
}

fn freshness_rank(timestamp: SystemTime, now: SystemTime) -> Duration {
    now.duration_since(timestamp)
        .unwrap_or(Duration::from_secs(u64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata(peer_id: PeerId, latency_ms: u64) -> RelayMetadata {
        let mut metadata = RelayMetadata::new(peer_id, "1.0".to_owned(), "test".to_owned());
        metadata.listening_address = Some("/ip4/127.0.0.1/udp/9000/quic-v1".to_owned());
        metadata.status = RelayStatus::Online;
        metadata.latency_ms = Some(latency_ms);
        metadata.last_heartbeat = Some(metadata.timestamp);
        metadata
    }

    #[test]
    fn filters_stale_offline_and_missing_capability_candidates() {
        let now = SystemTime::now();
        let requirements = CandidateRequirements {
            protocol_version: "1.0".to_owned(),
            required_transport: "quic".to_owned(),
            required_capabilities: vec!["relay".to_owned()],
            heartbeat_freshness: Duration::from_secs(30),
        };
        let mut offline = metadata(PeerId::random(), 1);
        offline.status = RelayStatus::Offline;
        assert!(RelayCandidate::from_metadata(&offline, &requirements, now).is_err());
        let mut missing = metadata(PeerId::random(), 1);
        missing.capabilities.clear();
        assert!(RelayCandidate::from_metadata(&missing, &requirements, now).is_err());
        let mut stale = metadata(PeerId::random(), 1);
        stale.timestamp = now - Duration::from_secs(60);
        stale.last_heartbeat = Some(stale.timestamp);
        assert!(RelayCandidate::from_metadata(&stale, &requirements, now).is_err());
    }

    #[test]
    fn ranks_latency_and_ties_deterministically() {
        let requirements = CandidateRequirements {
            protocol_version: "1.0".to_owned(),
            required_transport: "quic".to_owned(),
            required_capabilities: vec!["relay".to_owned()],
            heartbeat_freshness: Duration::from_secs(30),
        };
        let fast_metadata = metadata(PeerId::random(), 3);
        let slow_metadata = metadata(PeerId::random(), 20);
        let now = SystemTime::now();
        let fast = RelayCandidate::from_metadata(&fast_metadata, &requirements, now).unwrap();
        let slow = RelayCandidate::from_metadata(&slow_metadata, &requirements, now).unwrap();
        let ranked = RelayRanking::rank(vec![slow, fast], now);
        assert_eq!(ranked[0].latency, Some(Duration::from_millis(3)));
    }
}
