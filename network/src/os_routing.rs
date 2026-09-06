use std::collections::HashSet;
use std::fmt;
use std::net::IpAddr;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct IpPrefix {
    pub address: IpAddr,
    pub prefix_length: u8,
}

impl IpPrefix {
    pub fn new(address: IpAddr, prefix_length: u8) -> Result<Self, RouteError> {
        let maximum = if address.is_ipv4() { 32 } else { 128 };
        if prefix_length == 0 || prefix_length > maximum {
            return Err(RouteError::DefaultRouteRejected);
        }
        let network = match address {
            IpAddr::V4(address) => {
                let mask = if prefix_length == 32 {
                    u32::MAX
                } else {
                    u32::MAX << (32 - prefix_length)
                };
                IpAddr::V4(std::net::Ipv4Addr::from(u32::from(address) & mask))
            }
            IpAddr::V6(address) => {
                let mut bytes = address.octets();
                let full_bytes = usize::from(prefix_length / 8);
                let remaining_bits = prefix_length % 8;
                if full_bytes < bytes.len() {
                    if remaining_bits != 0 {
                        bytes[full_bytes] &= 0xff << (8 - remaining_bits);
                    }
                    let first_zero = full_bytes + usize::from(remaining_bits != 0);
                    bytes[first_zero..].fill(0);
                }
                IpAddr::V6(std::net::Ipv6Addr::from(bytes))
            }
        };
        Ok(Self {
            address: network,
            prefix_length,
        })
    }

    pub fn contains(&self, address: IpAddr) -> bool {
        if self.address.is_ipv4() != address.is_ipv4() {
            return false;
        }
        Self::new(address, self.prefix_length)
            .map(|candidate| candidate.address == self.address)
            .unwrap_or(false)
    }
}

impl fmt::Display for IpPrefix {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}/{}", self.address, self.prefix_length)
    }
}

impl FromStr for IpPrefix {
    type Err = RouteError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (address, prefix_length) = value.split_once('/').ok_or(RouteError::InvalidPrefix)?;
        let address = address
            .parse::<IpAddr>()
            .map_err(|_| RouteError::InvalidPrefix)?;
        let prefix_length = prefix_length
            .parse::<u8>()
            .map_err(|_| RouteError::InvalidPrefix)?;
        Self::new(address, prefix_length)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OwnedRoute {
    pub destination: IpPrefix,
    pub interface_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutingPolicy {
    Disabled,
    Explicit(Vec<IpPrefix>),
    ControlledTest(IpPrefix),
}

impl RoutingPolicy {
    pub fn routes(&self, interface_name: &str) -> Result<Vec<OwnedRoute>, RouteError> {
        if interface_name.trim().is_empty() {
            return Err(RouteError::InvalidInterface);
        }
        let prefixes = match self {
            Self::Disabled => return Ok(Vec::new()),
            Self::Explicit(prefixes) => prefixes.clone(),
            Self::ControlledTest(prefix) => vec![*prefix],
        };
        let mut routes = Vec::with_capacity(prefixes.len());
        for destination in prefixes {
            if routes
                .iter()
                .any(|route: &OwnedRoute| route.destination == destination)
            {
                return Err(RouteError::DuplicateRoute);
            }
            routes.push(OwnedRoute {
                destination,
                interface_name: interface_name.to_owned(),
            });
        }
        Ok(routes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteError {
    InvalidPrefix,
    DefaultRouteRejected,
    InvalidInterface,
    DuplicateRoute,
    RouteUnavailable(String),
    RouteOperationFailed(String),
}

impl fmt::Display for RouteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for RouteError {}

pub trait RouteManager {
    fn inspect(&mut self, route: &OwnedRoute) -> Result<bool, RouteError>;
    fn add(&mut self, route: &OwnedRoute) -> Result<(), RouteError>;
    fn remove(&mut self, route: &OwnedRoute) -> Result<(), RouteError>;

    fn install(&mut self, policy: &RoutingPolicy, interface_name: &str) -> Result<(), RouteError> {
        let routes = policy.routes(interface_name)?;
        let mut added = Vec::new();
        for route in &routes {
            if self.inspect(route)? {
                continue;
            }
            if let Err(error) = self.add(route) {
                for installed in added.iter().rev() {
                    let _ = self.remove(installed);
                }
                return Err(error);
            }
            added.push(route.clone());
        }
        Ok(())
    }

    fn remove_owned(&mut self) -> Result<(), RouteError>;
}

#[derive(Debug, Default)]
pub struct MockRouteManager {
    routes: HashSet<OwnedRoute>,
    owned_routes: HashSet<OwnedRoute>,
    available: bool,
}

impl MockRouteManager {
    pub fn new() -> Self {
        Self {
            available: true,
            ..Self::default()
        }
    }

    pub fn set_available(&mut self, available: bool) {
        self.available = available;
    }

    pub fn seed(&mut self, route: OwnedRoute) {
        self.routes.insert(route);
    }

    pub fn contains(&self, route: &OwnedRoute) -> bool {
        self.routes.contains(route)
    }
}

impl RouteManager for MockRouteManager {
    fn inspect(&mut self, route: &OwnedRoute) -> Result<bool, RouteError> {
        if !self.available {
            return Err(RouteError::RouteUnavailable(
                "mock route manager unavailable".to_owned(),
            ));
        }
        Ok(self.routes.contains(route))
    }

    fn add(&mut self, route: &OwnedRoute) -> Result<(), RouteError> {
        if !self.available {
            return Err(RouteError::RouteUnavailable(
                "mock route manager unavailable".to_owned(),
            ));
        }
        if !self.routes.insert(route.clone()) {
            return Err(RouteError::DuplicateRoute);
        }
        self.owned_routes.insert(route.clone());
        Ok(())
    }

    fn remove(&mut self, route: &OwnedRoute) -> Result<(), RouteError> {
        if self.owned_routes.remove(route) {
            self.routes.remove(route);
        }
        Ok(())
    }

    fn remove_owned(&mut self) -> Result<(), RouteError> {
        for route in self.owned_routes.drain() {
            self.routes.remove(&route);
        }
        Ok(())
    }
}

#[cfg(windows)]
#[derive(Default)]
pub struct WindowsRouteManager {
    owned_routes: HashSet<OwnedRoute>,
}

#[cfg(windows)]
impl WindowsRouteManager {
    pub fn new() -> Self {
        Self {
            owned_routes: HashSet::new(),
        }
    }

    fn family(route: &OwnedRoute) -> &'static str {
        if route.destination.address.is_ipv4() {
            "ipv4"
        } else {
            "ipv6"
        }
    }

    fn run_netsh(arguments: &[String]) -> Result<std::process::Output, RouteError> {
        std::process::Command::new("netsh")
            .args(arguments)
            .output()
            .map_err(|error| RouteError::RouteOperationFailed(error.to_string()))
    }
}

#[cfg(windows)]
impl RouteManager for WindowsRouteManager {
    fn inspect(&mut self, route: &OwnedRoute) -> Result<bool, RouteError> {
        let output = Self::run_netsh(&[
            "interface".to_owned(),
            Self::family(route).to_owned(),
            "show".to_owned(),
            "route".to_owned(),
        ])?;
        if !output.status.success() {
            return Err(RouteError::RouteOperationFailed(
                String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            ));
        }
        let prefix = route.destination.to_string();
        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .any(|line| line.contains(&prefix) && line.contains(&route.interface_name)))
    }

    fn add(&mut self, route: &OwnedRoute) -> Result<(), RouteError> {
        let output = Self::run_netsh(&[
            "interface".to_owned(),
            Self::family(route).to_owned(),
            "add".to_owned(),
            "route".to_owned(),
            format!("prefix={}", route.destination),
            format!("interface={}", route.interface_name),
            "store=active".to_owned(),
        ])?;
        if !output.status.success() {
            return Err(RouteError::RouteOperationFailed(
                String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            ));
        }
        self.owned_routes.insert(route.clone());
        Ok(())
    }

    fn remove(&mut self, route: &OwnedRoute) -> Result<(), RouteError> {
        if !self.owned_routes.remove(route) {
            return Ok(());
        }
        let output = Self::run_netsh(&[
            "interface".to_owned(),
            Self::family(route).to_owned(),
            "delete".to_owned(),
            "route".to_owned(),
            format!("prefix={}", route.destination),
            format!("interface={}", route.interface_name),
        ])?;
        if !output.status.success() {
            return Err(RouteError::RouteOperationFailed(
                String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            ));
        }
        Ok(())
    }

    fn remove_owned(&mut self) -> Result<(), RouteError> {
        let routes: Vec<_> = self.owned_routes.iter().cloned().collect();
        for route in routes {
            self.remove(&route)?;
        }
        Ok(())
    }
}

#[cfg(not(windows))]
#[derive(Default)]
pub struct WindowsRouteManager;

#[cfg(not(windows))]
impl WindowsRouteManager {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(not(windows))]
impl RouteManager for WindowsRouteManager {
    fn inspect(&mut self, _route: &OwnedRoute) -> Result<bool, RouteError> {
        Err(RouteError::RouteUnavailable(
            "Windows route management is unavailable on this platform".to_owned(),
        ))
    }

    fn add(&mut self, _route: &OwnedRoute) -> Result<(), RouteError> {
        Err(RouteError::RouteUnavailable(
            "Windows route management is unavailable on this platform".to_owned(),
        ))
    }

    fn remove(&mut self, _route: &OwnedRoute) -> Result<(), RouteError> {
        Ok(())
    }

    fn remove_owned(&mut self) -> Result<(), RouteError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(value: &str) -> IpPrefix {
        value.parse().unwrap()
    }

    #[test]
    fn rejects_default_and_invalid_prefixes() {
        assert_eq!(
            "0.0.0.0/0".parse::<IpPrefix>(),
            Err(RouteError::DefaultRouteRejected)
        );
        assert_eq!(
            "::/0".parse::<IpPrefix>(),
            Err(RouteError::DefaultRouteRejected)
        );
        assert_eq!(
            "192.168.1.1".parse::<IpPrefix>(),
            Err(RouteError::InvalidPrefix)
        );
        assert_eq!(
            "192.168.1.1/33".parse::<IpPrefix>(),
            Err(RouteError::DefaultRouteRejected)
        );
    }

    #[test]
    fn canonicalizes_and_matches_prefixes() {
        let prefix = route("192.168.1.3/24");
        assert_eq!(prefix.to_string(), "192.168.1.0/24");
        assert!(prefix.contains("192.168.1.99".parse().unwrap()));
        assert!(!prefix.contains("192.168.2.1".parse().unwrap()));
    }

    #[test]
    fn disabled_policy_does_not_change_routes() {
        let mut manager = MockRouteManager::new();
        manager.install(&RoutingPolicy::Disabled, "ghost0").unwrap();
        assert!(!manager.contains(&OwnedRoute {
            destination: route("192.168.1.3/32"),
            interface_name: "ghost0".to_owned(),
        }));
    }

    #[test]
    fn installs_only_owned_routes_and_cleans_them_up() {
        let mut manager = MockRouteManager::new();
        let existing = OwnedRoute {
            destination: route("192.168.1.3/32"),
            interface_name: "ghost0".to_owned(),
        };
        manager.seed(existing.clone());
        manager
            .install(
                &RoutingPolicy::Explicit(vec![existing.destination, route("198.51.100.7/32")]),
                "ghost0",
            )
            .unwrap();
        assert!(manager.contains(&existing));
        let owned = OwnedRoute {
            destination: route("198.51.100.7/32"),
            interface_name: "ghost0".to_owned(),
        };
        assert!(manager.contains(&owned));
        manager.remove_owned().unwrap();
        assert!(manager.contains(&existing));
        assert!(!manager.contains(&owned));
    }

    #[test]
    fn unavailable_manager_fails_without_routes() {
        let mut manager = MockRouteManager::new();
        manager.set_available(false);
        assert!(matches!(
            manager.install(
                &RoutingPolicy::ControlledTest(route("192.168.1.3/32")),
                "ghost0"
            ),
            Err(RouteError::RouteUnavailable(_))
        ));
    }
}
