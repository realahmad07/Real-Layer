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
const TUN_ENABLED: &str = "GHOST_TUN_ENABLED";
const TUN_INTERFACE_NAME: &str = "GHOST_TUN_INTERFACE_NAME";
const TUN_MTU: &str = "GHOST_TUN_MTU";
const TUN_MAXIMUM_PACKET_SIZE: &str = "GHOST_TUN_MAXIMUM_PACKET_SIZE";
const TUN_READ_BUFFER_LIMIT: &str = "GHOST_TUN_READ_BUFFER_LIMIT";
const TUN_WRITE_BUFFER_LIMIT: &str = "GHOST_TUN_WRITE_BUFFER_LIMIT";
const ALLOWED_EXIT_DESTINATIONS: &str = "GHOST_ALLOWED_EXIT_DESTINATIONS";
const NAT_ENABLED: &str = "GHOST_NAT_ENABLED";
const DNS_ENABLED: &str = "GHOST_DNS_ENABLED";
const EXTERNAL_TEST_DESTINATION: &str = "GHOST_EXTERNAL_TEST_DESTINATION";

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TunConfig {
    pub enabled: bool,
    pub interface_name: String,
    pub mtu: usize,
    pub maximum_packet_size: usize,
    pub read_buffer_limit: usize,
    pub write_buffer_limit: usize,
}

impl TunConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.interface_name.trim().is_empty() {
            return Err(ConfigError::Invalid {
                key: TUN_INTERFACE_NAME,
                value: self.interface_name.clone(),
            });
        }
        if self.mtu == 0 || self.mtu > u16::MAX as usize {
            return Err(ConfigError::Invalid {
                key: TUN_MTU,
                value: self.mtu.to_string(),
            });
        }
        if self.maximum_packet_size == 0 || self.maximum_packet_size > self.mtu {
            return Err(ConfigError::Invalid {
                key: TUN_MAXIMUM_PACKET_SIZE,
                value: self.maximum_packet_size.to_string(),
            });
        }
        if self.read_buffer_limit == 0 {
            return Err(ConfigError::Invalid {
                key: TUN_READ_BUFFER_LIMIT,
                value: self.read_buffer_limit.to_string(),
            });
        }
        if self.write_buffer_limit == 0 {
            return Err(ConfigError::Invalid {
                key: TUN_WRITE_BUFFER_LIMIT,
                value: self.write_buffer_limit.to_string(),
            });
        }
        Ok(())
    }
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
    pub tun: TunConfig,
    pub allowed_exit_destinations: Vec<String>,
    pub nat_enabled: bool,
    pub dns_enabled: bool,
    pub external_test_destination: Option<String>,
}

impl NodeConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let values: HashMap<String, String> = env::vars().collect();
        Self::from_map(&values)
    }

    pub fn from_map(values: &HashMap<String, String>) -> Result<Self, ConfigError> {
        let config = Self {
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
            tun: TunConfig {
                enabled: parse_bool_or_default(values, TUN_ENABLED, false)?,
                interface_name: optional(values, TUN_INTERFACE_NAME)
                    .unwrap_or_else(|| "ghost0".to_owned()),
                mtu: parse_usize_or_default(values, TUN_MTU, 1500)?,
                maximum_packet_size: parse_usize_or_default(values, TUN_MAXIMUM_PACKET_SIZE, 1500)?,
                read_buffer_limit: parse_usize_or_default(values, TUN_READ_BUFFER_LIMIT, 16)?,
                write_buffer_limit: parse_usize_or_default(values, TUN_WRITE_BUFFER_LIMIT, 16)?,
            },
            allowed_exit_destinations: parse_allowed_destinations(values)?,
            nat_enabled: parse_bool_or_default(values, NAT_ENABLED, false)?,
            dns_enabled: parse_bool_or_default(values, DNS_ENABLED, false)?,
            external_test_destination: parse_external_test_destination(values)?,
        };
        config.tun.validate()?;
        if let Some(destination) = &config.external_test_destination {
            if !config.allowed_exit_destinations.contains(destination) {
                return Err(ConfigError::Invalid {
                    key: EXTERNAL_TEST_DESTINATION,
                    value: destination.clone(),
                });
            }
        }
        Ok(config)
    }
}

fn parse_external_test_destination(
    values: &HashMap<String, String>,
) -> Result<Option<String>, ConfigError> {
    let Some(destination) = optional(values, EXTERNAL_TEST_DESTINATION) else {
        return Ok(None);
    };
    let address =
        destination
            .parse::<std::net::SocketAddr>()
            .map_err(|_| ConfigError::Invalid {
                key: EXTERNAL_TEST_DESTINATION,
                value: destination.clone(),
            })?;
    if address.port() == 0 || address.ip().is_unspecified() {
        return Err(ConfigError::Invalid {
            key: EXTERNAL_TEST_DESTINATION,
            value: destination,
        });
    }
    Ok(Some(address.to_string()))
}

fn parse_allowed_destinations(
    values: &HashMap<String, String>,
) -> Result<Vec<String>, ConfigError> {
    let Some(raw) = optional(values, ALLOWED_EXIT_DESTINATIONS) else {
        return Ok(Vec::new());
    };
    raw.split(',')
        .map(str::trim)
        .filter(|destination| !destination.is_empty())
        .map(|destination| {
            let address =
                destination
                    .parse::<std::net::SocketAddr>()
                    .map_err(|_| ConfigError::Invalid {
                        key: ALLOWED_EXIT_DESTINATIONS,
                        value: destination.to_owned(),
                    })?;
            if address.port() == 0 {
                return Err(ConfigError::Invalid {
                    key: ALLOWED_EXIT_DESTINATIONS,
                    value: destination.to_owned(),
                });
            }
            Ok(address.to_string())
        })
        .collect()
}

fn parse_bool_or_default(
    values: &HashMap<String, String>,
    key: &'static str,
    default: bool,
) -> Result<bool, ConfigError> {
    match values.get(key) {
        None => Ok(default),
        Some(value) => match value.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Ok(true),
            "0" | "false" | "no" | "off" => Ok(false),
            _ => Err(ConfigError::Invalid {
                key,
                value: value.clone(),
            }),
        },
    }
}

fn parse_usize_or_default(
    values: &HashMap<String, String>,
    key: &'static str,
    default: usize,
) -> Result<usize, ConfigError> {
    match values.get(key) {
        None => Ok(default),
        Some(value) => value.parse().map_err(|_| ConfigError::Invalid {
            key,
            value: value.clone(),
        }),
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
        assert!(!config.tun.enabled);
        assert_eq!(config.tun.mtu, 1500);
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
    fn parses_tun_values_and_rejects_invalid_limits() {
        let mut values = complete_values();
        values.insert(TUN_ENABLED.to_owned(), "true".to_owned());
        values.insert(TUN_INTERFACE_NAME.to_owned(), "ghost-test".to_owned());
        values.insert(TUN_MTU.to_owned(), "1400".to_owned());
        values.insert(TUN_MAXIMUM_PACKET_SIZE.to_owned(), "1400".to_owned());
        values.insert(TUN_READ_BUFFER_LIMIT.to_owned(), "4".to_owned());
        values.insert(TUN_WRITE_BUFFER_LIMIT.to_owned(), "5".to_owned());
        let config = NodeConfig::from_map(&values).expect("valid tun config");
        assert_eq!(config.tun.interface_name, "ghost-test");
        assert_eq!(config.tun.mtu, 1400);
        assert_eq!(config.tun.read_buffer_limit, 4);
        assert_eq!(config.tun.write_buffer_limit, 5);

        values.insert(TUN_MTU.to_owned(), "invalid".to_owned());
        assert!(matches!(
            NodeConfig::from_map(&values),
            Err(ConfigError::Invalid { key: TUN_MTU, .. })
        ));
    }

    #[test]
    fn parses_explicit_exit_destinations_without_hostname_resolution() {
        let mut values = complete_values();
        values.insert(
            ALLOWED_EXIT_DESTINATIONS.to_owned(),
            "192.0.2.10:9000, 127.0.0.1:9001, 192.0.2.10:9000".to_owned(),
        );
        let config = NodeConfig::from_map(&values).expect("valid destinations");
        assert_eq!(
            config.allowed_exit_destinations,
            vec![
                "192.0.2.10:9000".to_owned(),
                "127.0.0.1:9001".to_owned(),
                "192.0.2.10:9000".to_owned()
            ]
        );
        values.insert(
            ALLOWED_EXIT_DESTINATIONS.to_owned(),
            "example.test:9000".to_owned(),
        );
        assert!(matches!(
            NodeConfig::from_map(&values),
            Err(ConfigError::Invalid {
                key: ALLOWED_EXIT_DESTINATIONS,
                ..
            })
        ));
    }

    #[test]
    fn parses_literal_external_test_destination() {
        let mut values = complete_values();
        values.insert(
            EXTERNAL_TEST_DESTINATION.to_owned(),
            "192.0.2.10:9000".to_owned(),
        );
        values.insert(
            ALLOWED_EXIT_DESTINATIONS.to_owned(),
            "192.0.2.10:9000".to_owned(),
        );
        let config = NodeConfig::from_map(&values).expect("valid external test destination");
        assert_eq!(
            config.external_test_destination.as_deref(),
            Some("192.0.2.10:9000")
        );
        values.insert(
            ALLOWED_EXIT_DESTINATIONS.to_owned(),
            "192.0.2.11:9000".to_owned(),
        );
        assert!(matches!(
            NodeConfig::from_map(&values),
            Err(ConfigError::Invalid {
                key: EXTERNAL_TEST_DESTINATION,
                ..
            })
        ));
        values.insert(
            EXTERNAL_TEST_DESTINATION.to_owned(),
            "test.example:9000".to_owned(),
        );
        assert!(matches!(
            NodeConfig::from_map(&values),
            Err(ConfigError::Invalid {
                key: EXTERNAL_TEST_DESTINATION,
                ..
            })
        ));
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
