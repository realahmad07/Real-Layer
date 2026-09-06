use crate::error::ConfigError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

const NODE_ID: &str = "GHOST_NODE_ID";
const LISTEN_ADDRESS: &str = "GHOST_LISTEN_ADDRESS";
const ADVERTISED_ADDRESS: &str = "GHOST_ADVERTISED_ADDRESS";
const NETWORK_ENVIRONMENT: &str = "GHOST_NETWORK_ENVIRONMENT";
const LOG_LEVEL: &str = "GHOST_LOG_LEVEL";
const SOLANA_RPC_URL: &str = "GHOST_SOLANA_RPC_URL";
const MAGICBLOCK_ENDPOINT: &str = "GHOST_MAGICBLOCK_ENDPOINT";
const IDENTITY_PATH: &str = "GHOST_IDENTITY_PATH";
const BOOTSTRAP_PEERS: &str = "GHOST_BOOTSTRAP_PEERS";
const CONNECTION_TIMEOUT_SECS: &str = "GHOST_CONNECTION_TIMEOUT_SECS";
const HEARTBEAT_INTERVAL_SECS: &str = "GHOST_HEARTBEAT_INTERVAL_SECS";
const HEARTBEAT_FRESHNESS_SECS: &str = "GHOST_HEARTBEAT_FRESHNESS_SECS";
const DEGRADED_LATENCY_MS: &str = "GHOST_DEGRADED_LATENCY_MS";
const SOFTWARE_VERSION: &str = "GHOST_SOFTWARE_VERSION";
const PROTOCOL_VERSION: &str = "GHOST_PROTOCOL_VERSION";
const DISCOVERY_REQUIRED_TRANSPORT: &str = "GHOST_DISCOVERY_REQUIRED_TRANSPORT";
const DISCOVERY_REQUIRED_CAPABILITIES: &str = "GHOST_DISCOVERY_REQUIRED_CAPABILITIES";
const ROUTE_MODE: &str = "GHOST_ROUTE_MODE";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RouteMode {
    OneHop,
    TwoHop,
}

impl FromStr for RouteMode {
    type Err = ConfigError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "one-hop" | "one_hop" => Ok(Self::OneHop),
            "two-hop" | "two_hop" => Ok(Self::TwoHop),
            _ => Err(ConfigError::Invalid {
                key: ROUTE_MODE,
                value: value.to_owned(),
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetworkEnvironment {
    Development,
    Staging,
    Production,
}

impl FromStr for NetworkEnvironment {
    type Err = ConfigError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "development" | "dev" => Ok(Self::Development),
            "staging" | "stage" => Ok(Self::Staging),
            "production" | "prod" => Ok(Self::Production),
            _ => Err(ConfigError::Invalid {
                key: NETWORK_ENVIRONMENT,
                value: value.to_owned(),
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl FromStr for LogLevel {
    type Err = ConfigError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "error" => Ok(Self::Error),
            "warn" | "warning" => Ok(Self::Warn),
            "info" => Ok(Self::Info),
            "debug" => Ok(Self::Debug),
            "trace" => Ok(Self::Trace),
            _ => Err(ConfigError::Invalid {
                key: LOG_LEVEL,
                value: value.to_owned(),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeConfig {
    pub node_id: String,
    pub listen_address: String,
    pub advertised_address: Option<String>,
    pub network_environment: NetworkEnvironment,
    pub log_level: LogLevel,
    pub solana_rpc_url: String,
    pub magicblock_endpoint: Option<String>,
    pub identity_path: PathBuf,
    pub bootstrap_peers: Vec<String>,
    pub connection_timeout: Duration,
    pub heartbeat_interval: Duration,
    pub heartbeat_freshness: Duration,
    pub degraded_latency: Duration,
    pub software_version: String,
    pub protocol_version: String,
    pub discovery_required_transport: String,
    pub discovery_required_capabilities: Vec<String>,
    pub route_mode: RouteMode,
}

impl NodeConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let values: HashMap<String, String> = env::vars().collect();
        Self::from_map(&values)
    }

    pub fn from_map(values: &HashMap<String, String>) -> Result<Self, ConfigError> {
        Ok(Self {
            node_id: optional(values, NODE_ID).unwrap_or_else(|| "node".to_owned()),
            listen_address: required(values, LISTEN_ADDRESS)?,
            advertised_address: optional(values, ADVERTISED_ADDRESS),
            network_environment: parse_or_default(values, NETWORK_ENVIRONMENT, "development")?,
            log_level: parse_or_default(values, LOG_LEVEL, "info")?,
            solana_rpc_url: optional(values, SOLANA_RPC_URL).unwrap_or_default(),
            magicblock_endpoint: optional(values, MAGICBLOCK_ENDPOINT),
            identity_path: PathBuf::from(
                optional(values, IDENTITY_PATH).unwrap_or_else(|| "./ghost-layer.key".to_owned()),
            ),
            bootstrap_peers: optional(values, BOOTSTRAP_PEERS)
                .map(|peers| {
                    peers
                        .split(',')
                        .map(str::trim)
                        .filter(|peer| !peer.is_empty())
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            connection_timeout: Duration::from_secs(parse_u64_or_default(
                values,
                CONNECTION_TIMEOUT_SECS,
                10,
            )?),
            heartbeat_interval: Duration::from_secs(parse_u64_or_default(
                values,
                HEARTBEAT_INTERVAL_SECS,
                10,
            )?),
            heartbeat_freshness: Duration::from_secs(parse_u64_or_default(
                values,
                HEARTBEAT_FRESHNESS_SECS,
                30,
            )?),
            degraded_latency: Duration::from_millis(parse_u64_or_default(
                values,
                DEGRADED_LATENCY_MS,
                500,
            )?),
            software_version: optional(values, SOFTWARE_VERSION)
                .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_owned()),
            protocol_version: optional(values, PROTOCOL_VERSION)
                .unwrap_or_else(|| "1.0".to_owned()),
            discovery_required_transport: optional(values, DISCOVERY_REQUIRED_TRANSPORT)
                .unwrap_or_else(|| "quic".to_owned()),
            discovery_required_capabilities: optional(values, DISCOVERY_REQUIRED_CAPABILITIES)
                .map(|capabilities| {
                    capabilities
                        .split(',')
                        .map(str::trim)
                        .filter(|capability| !capability.is_empty())
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_else(|| vec!["relay".to_owned()]),
            route_mode: parse_or_default(values, ROUTE_MODE, "one-hop")?,
        })
    }
}

fn parse_u64_or_default(
    values: &HashMap<String, String>,
    key: &'static str,
    default: u64,
) -> Result<u64, ConfigError> {
    match values.get(key) {
        None => Ok(default),
        Some(value) => value.parse().map_err(|_| ConfigError::Invalid {
            key,
            value: value.clone(),
        }),
    }
}

fn required(values: &HashMap<String, String>, key: &'static str) -> Result<String, ConfigError> {
    values
        .get(key)
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .ok_or(ConfigError::Missing { key })
}

fn optional(values: &HashMap<String, String>, key: &str) -> Option<String> {
    values
        .get(key)
        .filter(|value| !value.trim().is_empty())
        .cloned()
}

fn parse_or_default<T>(
    values: &HashMap<String, String>,
    key: &'static str,
    default: &str,
) -> Result<T, ConfigError>
where
    T: FromStr<Err = ConfigError>,
{
    values
        .get(key)
        .map(String::as_str)
        .unwrap_or(default)
        .parse()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete_values() -> HashMap<String, String> {
        HashMap::from([
            (NODE_ID.to_owned(), "node-alpha".to_owned()),
            (LISTEN_ADDRESS.to_owned(), "0.0.0.0:7000".to_owned()),
            (
                SOLANA_RPC_URL.to_owned(),
                "https://api.devnet.solana.com".to_owned(),
            ),
        ])
    }

    #[test]
    fn parses_required_values_and_defaults() {
        let config = NodeConfig::from_map(&complete_values()).expect("valid config");

        assert_eq!(config.node_id, "node-alpha");
        assert_eq!(config.network_environment, NetworkEnvironment::Development);
        assert_eq!(config.log_level, LogLevel::Info);
        assert_eq!(config.magicblock_endpoint, None);
    }

    #[test]
    fn parses_optional_and_explicit_values() {
        let mut values = complete_values();
        values.insert(
            ADVERTISED_ADDRESS.to_owned(),
            "relay.example:7000".to_owned(),
        );
        values.insert(NETWORK_ENVIRONMENT.to_owned(), "staging".to_owned());
        values.insert(LOG_LEVEL.to_owned(), "debug".to_owned());
        values.insert(
            MAGICBLOCK_ENDPOINT.to_owned(),
            "https://magicblock.example".to_owned(),
        );

        let config = NodeConfig::from_map(&values).expect("valid config");

        assert_eq!(
            config.advertised_address.as_deref(),
            Some("relay.example:7000")
        );
        assert_eq!(config.network_environment, NetworkEnvironment::Staging);
        assert_eq!(config.log_level, LogLevel::Debug);
        assert_eq!(
            config.magicblock_endpoint.as_deref(),
            Some("https://magicblock.example")
        );
    }

    #[test]
    fn rejects_missing_required_values() {
        let error = NodeConfig::from_map(&HashMap::new()).expect_err("config should be invalid");
        assert_eq!(
            error,
            ConfigError::Missing {
                key: LISTEN_ADDRESS
            }
        );
    }

    #[test]
    fn parses_network_values() {
        let mut values = complete_values();
        values.insert(IDENTITY_PATH.to_owned(), "./node.key".to_owned());
        values.insert(BOOTSTRAP_PEERS.to_owned(), "peer-a, peer-b".to_owned());
        values.insert(CONNECTION_TIMEOUT_SECS.to_owned(), "15".to_owned());
        let config = NodeConfig::from_map(&values).expect("valid network config");

        assert_eq!(config.identity_path, PathBuf::from("./node.key"));
        assert_eq!(config.bootstrap_peers, vec!["peer-a", "peer-b"]);
        assert_eq!(config.connection_timeout, Duration::from_secs(15));
    }

    #[test]
    fn parses_route_mode_and_rejects_invalid_values() {
        let mut values = complete_values();
        values.insert(ROUTE_MODE.to_owned(), "two-hop".to_owned());
        let config = NodeConfig::from_map(&values).expect("valid route mode");
        assert_eq!(config.route_mode, RouteMode::TwoHop);

        values.insert(ROUTE_MODE.to_owned(), "three-hop".to_owned());
        assert!(matches!(
            NodeConfig::from_map(&values),
            Err(ConfigError::Invalid {
                key: ROUTE_MODE,
                ..
            })
        ));
    }
}
