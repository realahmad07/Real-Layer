use crate::error::ConfigError;
use crate::os_routing::IpPrefix;
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
const OS_ROUTING_ENABLED: &str = "GHOST_OS_ROUTING_ENABLED";
const OS_ROUTES: &str = "GHOST_OS_ROUTES";
const DNS_SERVER: &str = "GHOST_DNS_SERVER";
const DNS_TIMEOUT_MS: &str = "GHOST_DNS_TIMEOUT_MS";
const UDP_ENABLED: &str = "GHOST_UDP_ENABLED";
const VPN_MODE: &str = "GHOST_VPN_MODE";
const MAX_CONNECTED_PEERS: &str = "GHOST_MAX_CONNECTED_PEERS";
const MAX_ACTIVE_SESSIONS: &str = "GHOST_MAX_ACTIVE_SESSIONS";
const MAX_ACTIVE_FLOWS: &str = "GHOST_MAX_ACTIVE_FLOWS";
const MAX_QUEUED_PACKETS: &str = "GHOST_MAX_QUEUED_PACKETS";
const MAX_EXIT_CONNECTIONS: &str = "GHOST_MAX_EXIT_CONNECTIONS";
const SESSION_TIMEOUT_SECS: &str = "GHOST_SESSION_TIMEOUT_SECS";
const FLOW_TIMEOUT_SECS: &str = "GHOST_FLOW_TIMEOUT_SECS";
const HEALTH_LISTEN_ADDRESS: &str = "GHOST_HEALTH_LISTEN_ADDRESS";
const RELAY_ROLE: &str = "GHOST_RELAY_ROLE";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayResourceLimits {
    pub maximum_connected_peers: usize,
    pub maximum_active_sessions: usize,
    pub maximum_active_flows: usize,
    pub maximum_queued_packets: usize,
    pub maximum_exit_connections: usize,
    pub session_timeout: Duration,
    pub flow_timeout: Duration,
}

impl RelayResourceLimits {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.maximum_connected_peers == 0 {
            return Err(ConfigError::Invalid {
                key: MAX_CONNECTED_PEERS,
                value: "must be non-zero".to_owned(),
            });
        }
        if self.maximum_active_sessions == 0 {
            return Err(ConfigError::Invalid {
                key: MAX_ACTIVE_SESSIONS,
                value: "must be non-zero".to_owned(),
            });
        }
        if self.maximum_active_flows == 0
            || self.maximum_queued_packets == 0
            || self.maximum_exit_connections == 0
        {
            return Err(ConfigError::Invalid {
                key: MAX_ACTIVE_FLOWS,
                value: "resource limits must be non-zero".to_owned(),
            });
        }
        if self.session_timeout.is_zero() || self.flow_timeout.is_zero() {
            return Err(ConfigError::Invalid {
                key: SESSION_TIMEOUT_SECS,
                value: "timeouts must be non-zero".to_owned(),
            });
        }
        Ok(())
    }
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RelayRole {
    Entry,
    Exit,
    Both,
}

impl FromStr for RelayRole {
    type Err = ConfigError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "entry" => Ok(Self::Entry),
            "exit" => Ok(Self::Exit),
            "both" | "entry_exit" | "entry-exit" => Ok(Self::Both),
            _ => Err(ConfigError::Invalid {
                key: RELAY_ROLE,
                value: value.to_owned(),
            }),
        }
    }
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
    pub os_routing_enabled: bool,
    pub os_routes: Vec<IpPrefix>,
    pub dns_server: Option<String>,
    pub dns_timeout: Duration,
    pub udp_enabled: bool,
    pub vpn_mode: bool,
    pub resource_limits: RelayResourceLimits,
    pub health_listen_address: Option<String>,
    pub relay_role: RelayRole,
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
            os_routing_enabled: parse_bool_or_default(values, OS_ROUTING_ENABLED, false)?,
            os_routes: parse_os_routes(values)?,
            dns_server: parse_dns_server(values)?,
            dns_timeout: Duration::from_millis(parse_u64_or_default(values, DNS_TIMEOUT_MS, 1000)?),
            udp_enabled: parse_bool_or_default(values, UDP_ENABLED, false)?,
            vpn_mode: parse_bool_or_default(values, VPN_MODE, false)?,
            resource_limits: RelayResourceLimits {
                maximum_connected_peers: parse_usize_or_default(values, MAX_CONNECTED_PEERS, 128)?,
                maximum_active_sessions: parse_usize_or_default(values, MAX_ACTIVE_SESSIONS, 256)?,
                maximum_active_flows: parse_usize_or_default(values, MAX_ACTIVE_FLOWS, 256)?,
                maximum_queued_packets: parse_usize_or_default(values, MAX_QUEUED_PACKETS, 1024)?,
                maximum_exit_connections: parse_usize_or_default(values, MAX_EXIT_CONNECTIONS, 64)?,
                session_timeout: Duration::from_secs(parse_u64_or_default(
                    values,
                    SESSION_TIMEOUT_SECS,
                    300,
                )?),
                flow_timeout: Duration::from_secs(parse_u64_or_default(
                    values,
                    FLOW_TIMEOUT_SECS,
                    60,
                )?),
            },
            health_listen_address: parse_health_listen_address(values)?,
            relay_role: parse_or_default(values, RELAY_ROLE, "both")?,
        };
        config.tun.validate()?;
        config.resource_limits.validate()?;
        if let Some(destination) = &config.external_test_destination {
            if !config.allowed_exit_destinations.contains(destination) {
                return Err(ConfigError::Invalid {
                    key: EXTERNAL_TEST_DESTINATION,
                    value: destination.clone(),
                });
            }
        }
        if config.os_routing_enabled && config.os_routes.is_empty() {
            return Err(ConfigError::Invalid {
                key: OS_ROUTES,
                value: "at least one explicit route is required when OS routing is enabled"
                    .to_owned(),
            });
        }
        if config.dns_enabled && config.dns_server.is_none() {
            return Err(ConfigError::Invalid {
                key: DNS_SERVER,
                value: "an explicit IP:PORT DNS server is required when DNS is enabled".to_owned(),
            });
        }
        if config.dns_enabled
            && !config
                .dns_server
                .as_ref()
                .is_some_and(|server| config.allowed_exit_destinations.contains(server))
        {
            return Err(ConfigError::Invalid {
                key: DNS_SERVER,
                value: "DNS server must be an exact allowed exit destination".to_owned(),
            });
        }
        if config.dns_timeout.is_zero() {
            return Err(ConfigError::Invalid {
                key: DNS_TIMEOUT_MS,
                value: "timeout must be non-zero".to_owned(),
            });
        }
        if config.udp_enabled && !config.nat_enabled {
            return Err(ConfigError::Invalid {
                key: UDP_ENABLED,
                value: "UDP runtime requires GHOST_NAT_ENABLED=true".to_owned(),
            });
        }
        if config.network_environment == NetworkEnvironment::Production {
            if validate_quic_multiaddr(&config.listen_address, LISTEN_ADDRESS)? {
                return Err(ConfigError::Invalid {
                    key: LISTEN_ADDRESS,
                    value: "production relays must not bind to loopback".to_owned(),
                });
            }
            if config.advertised_address.is_none() {
                return Err(ConfigError::Missing {
                    key: ADVERTISED_ADDRESS,
                });
            }
            if let Some(address) = &config.advertised_address {
                if validate_quic_multiaddr(address, ADVERTISED_ADDRESS)? {
                    return Err(ConfigError::Invalid {
                        key: ADVERTISED_ADDRESS,
                        value: "production relays must not advertise loopback".to_owned(),
                    });
                }
            }
            if (config.external_test_destination.is_some()
                || config.udp_enabled
                || config.dns_enabled)
                && config.allowed_exit_destinations.is_empty()
            {
                return Err(ConfigError::Missing {
                    key: ALLOWED_EXIT_DESTINATIONS,
                });
            }
            validate_production_bootstrap_peers(&config.bootstrap_peers)?;
        }
        if config.vpn_mode
            && (!config.tun.enabled || !config.os_routing_enabled || config.os_routes.is_empty())
        {
            return Err(ConfigError::Invalid {
                key: VPN_MODE,
                value: "VPN mode requires enabled TUN and explicit OS routes".to_owned(),
            });
        }
        Ok(config)
    }
}

fn validate_quic_multiaddr(address: &str, key: &'static str) -> Result<bool, ConfigError> {
    let multiaddr = address
        .parse::<libp2p::Multiaddr>()
        .map_err(|_| ConfigError::Invalid {
            key,
            value: address.to_owned(),
        })?;
    let rendered = multiaddr.to_string();
    if !rendered.contains("/udp/") || !rendered.contains("/quic-v1") {
        return Err(ConfigError::Invalid {
            key,
            value: "listen address must be a QUIC multiaddr".to_owned(),
        });
    }
    Ok(rendered.contains("/ip4/127.0.0.1/") || rendered.contains("/ip6/::1/"))
}

fn validate_production_bootstrap_peers(peers: &[String]) -> Result<(), ConfigError> {
    let mut unique = std::collections::HashSet::with_capacity(peers.len());
    for peer in peers {
        let multiaddr = peer
            .parse::<libp2p::Multiaddr>()
            .map_err(|_| ConfigError::Invalid {
                key: BOOTSTRAP_PEERS,
                value: peer.clone(),
            })?;
        if !multiaddr.to_string().contains("/p2p/") || !unique.insert(peer) {
            return Err(ConfigError::Invalid {
                key: BOOTSTRAP_PEERS,
                value: peer.clone(),
            });
        }
    }
    Ok(())
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

fn parse_os_routes(values: &HashMap<String, String>) -> Result<Vec<IpPrefix>, ConfigError> {
    let Some(raw) = optional(values, OS_ROUTES) else {
        return Ok(Vec::new());
    };
    raw.split(',')
        .map(str::trim)
        .filter(|route| !route.is_empty())
        .map(|route| {
            route.parse::<IpPrefix>().map_err(|_| ConfigError::Invalid {
                key: OS_ROUTES,
                value: route.to_owned(),
            })
        })
        .collect()
}

fn parse_dns_server(values: &HashMap<String, String>) -> Result<Option<String>, ConfigError> {
    let Some(server) = optional(values, DNS_SERVER) else {
        return Ok(None);
    };
    let address = server
        .parse::<std::net::SocketAddr>()
        .map_err(|_| ConfigError::Invalid {
            key: DNS_SERVER,
            value: server.clone(),
        })?;
    if address.port() == 0 || address.ip().is_unspecified() {
        return Err(ConfigError::Invalid {
            key: DNS_SERVER,
            value: server,
        });
    }
    Ok(Some(address.to_string()))
}

fn parse_health_listen_address(
    values: &HashMap<String, String>,
) -> Result<Option<String>, ConfigError> {
    let Some(address) = optional(values, HEALTH_LISTEN_ADDRESS) else {
        return Ok(None);
    };
    let parsed = address
        .parse::<std::net::SocketAddr>()
        .map_err(|_| ConfigError::Invalid {
            key: HEALTH_LISTEN_ADDRESS,
            value: address.clone(),
        })?;
    if parsed.port() == 0 || !parsed.ip().is_loopback() {
        return Err(ConfigError::Invalid {
            key: HEALTH_LISTEN_ADDRESS,
            value: "health endpoint must use a loopback address and non-zero port".to_owned(),
        });
    }
    Ok(Some(parsed.to_string()))
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
        assert!(!config.os_routing_enabled);
        assert!(config.os_routes.is_empty());
        assert_eq!(config.resource_limits.maximum_connected_peers, 128);
        assert_eq!(config.resource_limits.maximum_active_sessions, 256);
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
    fn rejects_zero_resource_limits() {
        let mut values = complete_values();
        values.insert(MAX_ACTIVE_SESSIONS.to_owned(), "0".to_owned());
        assert!(NodeConfig::from_map(&values).is_err());
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
    fn parses_controlled_os_routes_and_rejects_default_routes() {
        let mut values = complete_values();
        values.insert(OS_ROUTING_ENABLED.to_owned(), "true".to_owned());
        values.insert(OS_ROUTES.to_owned(), "192.168.1.3/32".to_owned());
        let config = NodeConfig::from_map(&values).expect("valid controlled route");
        assert!(config.os_routing_enabled);
        assert_eq!(config.os_routes[0].to_string(), "192.168.1.3/32");

        values.insert(OS_ROUTES.to_owned(), "0.0.0.0/0".to_owned());
        assert!(matches!(
            NodeConfig::from_map(&values),
            Err(ConfigError::Invalid { key: OS_ROUTES, .. })
        ));
    }

    #[test]
    fn validates_explicit_dns_and_vpn_mode_requirements() {
        let mut values = complete_values();
        values.insert(DNS_ENABLED.to_owned(), "true".to_owned());
        assert!(matches!(
            NodeConfig::from_map(&values),
            Err(ConfigError::Invalid {
                key: DNS_SERVER,
                ..
            })
        ));
        values.insert(DNS_SERVER.to_owned(), "192.0.2.53:53".to_owned());
        values.insert(
            ALLOWED_EXIT_DESTINATIONS.to_owned(),
            "192.0.2.53:53".to_owned(),
        );
        values.insert(VPN_MODE.to_owned(), "true".to_owned());
        assert!(matches!(
            NodeConfig::from_map(&values),
            Err(ConfigError::Invalid { key: VPN_MODE, .. })
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
    fn production_requires_non_loopback_bind_advertisement_and_exit_policy() {
        let mut values = complete_values();
        values.insert(NETWORK_ENVIRONMENT.to_owned(), "production".to_owned());
        values.insert(
            LISTEN_ADDRESS.to_owned(),
            "/ip4/127.0.0.1/udp/7000/quic-v1".to_owned(),
        );
        assert!(matches!(
            NodeConfig::from_map(&values),
            Err(ConfigError::Invalid {
                key: LISTEN_ADDRESS,
                ..
            })
        ));

        values.insert(
            LISTEN_ADDRESS.to_owned(),
            "/ip4/0.0.0.0/udp/7000/quic-v1".to_owned(),
        );
        assert!(matches!(
            NodeConfig::from_map(&values),
            Err(ConfigError::Missing {
                key: ADVERTISED_ADDRESS
            })
        ));

        values.insert(
            ADVERTISED_ADDRESS.to_owned(),
            "/ip4/203.0.113.10/udp/7000/quic-v1".to_owned(),
        );
        values.insert(UDP_ENABLED.to_owned(), "true".to_owned());
        values.insert(NAT_ENABLED.to_owned(), "true".to_owned());
        assert!(matches!(
            NodeConfig::from_map(&values),
            Err(ConfigError::Missing {
                key: ALLOWED_EXIT_DESTINATIONS
            })
        ));
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
    fn parses_relay_role() {
        let mut values = complete_values();
        values.insert(RELAY_ROLE.to_owned(), "exit".to_owned());
        let config = NodeConfig::from_map(&values).expect("valid relay role");
        assert_eq!(config.relay_role, RelayRole::Exit);
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
