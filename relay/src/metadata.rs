use crate::{HealthClass, RelayHealth, RelayState, RelayStatus};
use ghost_layer_network::{Multiaddr, PeerId};
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayMetadataRequest {
    pub kind: String,
    pub protocol_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayMetadata {
    #[serde(with = "peer_id_serde")]
    pub peer_id: PeerId,
    pub protocol_version: String,
    pub software_version: String,
    pub supported_transports: Vec<String>,
    pub capabilities: Vec<String>,
    pub listening_address: Option<String>,
    pub advertised_address: Option<String>,
    pub status: RelayStatus,
    pub health: HealthClass,
    pub latency_ms: Option<u64>,
    #[serde(with = "unix_timestamp")]
    pub timestamp: SystemTime,
    #[serde(default, with = "optional_unix_timestamp")]
    pub last_heartbeat: Option<SystemTime>,
}

impl RelayMetadata {
    pub fn new(peer_id: PeerId, protocol_version: String, software_version: String) -> Self {
        Self {
            peer_id,
            protocol_version,
            software_version,
            supported_transports: vec!["quic".to_owned()],
            capabilities: vec!["quic".to_owned(), "relay".to_owned()],
            listening_address: None,
            advertised_address: None,
            status: RelayStatus::Starting,
            health: HealthClass::Healthy,
            latency_ms: None,
            timestamp: SystemTime::now(),
            last_heartbeat: None,
        }
    }

    pub fn from_state(state: &RelayState, protocol_version: String, health: RelayHealth) -> Self {
        Self::from_state_with_capabilities(
            state,
            protocol_version,
            health,
            vec!["quic".to_owned(), "relay".to_owned()],
        )
    }

    pub fn from_state_with_capabilities(
        state: &RelayState,
        protocol_version: String,
        health: RelayHealth,
        capabilities: Vec<String>,
    ) -> Self {
        Self {
            peer_id: *state.peer_id(),
            protocol_version,
            software_version: state.software_version().to_owned(),
            supported_transports: vec!["quic".to_owned()],
            capabilities,
            listening_address: state.listening_address().map(str::to_owned),
            advertised_address: state.advertised_address().map(str::to_owned),
            status: state.status(),
            health: health.class,
            latency_ms: health.latency.map(|latency| latency.as_millis() as u64),
            timestamp: SystemTime::now(),
            last_heartbeat: state.last_heartbeat(),
        }
    }

    pub fn validate(
        &self,
        expected_protocol: &str,
        now: SystemTime,
        freshness: Duration,
    ) -> Result<(), String> {
        if self.protocol_version != expected_protocol {
            return Err(format!(
                "unsupported protocol version: {}",
                self.protocol_version
            ));
        }
        if now
            .duration_since(self.timestamp)
            .map_err(|_| "metadata timestamp is in the future")?
            > freshness
        {
            return Err("metadata timestamp is stale".to_owned());
        }
        if self.last_heartbeat.is_some_and(|timestamp| {
            now.duration_since(timestamp)
                .map_or(true, |age| age > freshness)
        }) {
            return Err("relay heartbeat is stale".to_owned());
        }
        let address = self
            .advertised_address
            .as_deref()
            .or(self.listening_address.as_deref())
            .ok_or_else(|| "relay has no address".to_owned())?;
        if address.parse::<Multiaddr>().is_err() {
            return Err("relay address is invalid".to_owned());
        }
        if self
            .supported_transports
            .iter()
            .any(|transport| transport != "quic")
        {
            return Err("relay advertised an unknown transport".to_owned());
        }
        if self.capabilities.iter().any(|capability| {
            !matches!(
                capability.as_str(),
                "quic"
                    | "relay"
                    | "one_hop"
                    | "two_hop"
                    | "entry_role"
                    | "exit_role"
                    | "tcp_exit"
                    | "udp_exit"
                    | "dns_forwarding"
                    | "tun_boundary"
                    | "health_reporting"
            )
        }) {
            return Err("relay advertised an unknown capability".to_owned());
        }
        Ok(())
    }
}

mod peer_id_serde {
    use ghost_layer_network::PeerId;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(peer_id: &PeerId, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&peer_id.to_string())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<PeerId, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

mod unix_timestamp {
    use super::*;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(timestamp: &SystemTime, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        timestamp
            .duration_since(UNIX_EPOCH)
            .map_err(serde::ser::Error::custom)?
            .as_secs()
            .serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<SystemTime, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(UNIX_EPOCH + Duration::from_secs(u64::deserialize(deserializer)?))
    }
}

mod optional_unix_timestamp {
    use super::*;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(timestamp: &Option<SystemTime>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        timestamp
            .map(|value| {
                value
                    .duration_since(UNIX_EPOCH)
                    .map(|duration| duration.as_secs())
            })
            .transpose()
            .map_err(serde::ser::Error::custom)?
            .serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<SystemTime>, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Option::<u64>::deserialize(deserializer)?
            .map(|value| UNIX_EPOCH + Duration::from_secs(value)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_round_trips_and_validates() {
        let mut metadata =
            RelayMetadata::new(PeerId::random(), "1.0".to_owned(), "test".to_owned());
        metadata.timestamp = UNIX_EPOCH
            + Duration::from_secs(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("current time")
                    .as_secs(),
            );
        metadata.listening_address = Some("/ip4/127.0.0.1/udp/9000/quic-v1".to_owned());
        metadata.last_heartbeat = Some(metadata.timestamp);
        let encoded = serde_json::to_vec(&metadata).expect("serialize metadata");
        let decoded: RelayMetadata =
            serde_json::from_slice(&encoded).expect("deserialize metadata");
        decoded
            .validate("1.0", SystemTime::now(), Duration::from_secs(30))
            .expect("valid metadata");
        assert_eq!(decoded, metadata);
    }
}
