use crate::{NetworkPacket, PacketType};
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DestinationClass {
    Loopback,
    Private,
    LinkLocal,
    Multicast,
    Broadcast,
    Public,
    Unsupported,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutingDecision {
    ThroughGhostLayer,
    LocalSystem,
    Drop,
    Unsupported,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutingError {
    MalformedPacket,
    UnsupportedPacket,
}
impl fmt::Display for RoutingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for RoutingError {}

pub fn classify_destination(packet: &NetworkPacket) -> Result<DestinationClass, RoutingError> {
    let bytes = packet.as_bytes();
    let address = match packet.packet_type() {
        PacketType::Ipv4 => {
            let header_length = usize::from(bytes[0] & 0x0f) * 4;
            if bytes.len() < 20 || header_length > bytes.len() {
                return Err(RoutingError::MalformedPacket);
            }
            IpAddr::V4(Ipv4Addr::new(bytes[16], bytes[17], bytes[18], bytes[19]))
        }
        PacketType::Ipv6 => {
            if bytes.len() < 40 {
                return Err(RoutingError::MalformedPacket);
            }
            let mut octets = [0u8; 16];
            octets.copy_from_slice(&bytes[24..40]);
            IpAddr::V6(Ipv6Addr::from(octets))
        }
    };
    Ok(match address {
        IpAddr::V4(value) if value.is_loopback() => DestinationClass::Loopback,
        IpAddr::V4(value) if value.is_private() => DestinationClass::Private,
        IpAddr::V4(value) if value.is_link_local() => DestinationClass::LinkLocal,
        IpAddr::V4(value) if value.is_multicast() => DestinationClass::Multicast,
        IpAddr::V4(value) if value == Ipv4Addr::BROADCAST => DestinationClass::Broadcast,
        IpAddr::V4(value) if value.is_unspecified() => DestinationClass::Unsupported,
        IpAddr::V6(value) if value.is_loopback() => DestinationClass::Loopback,
        IpAddr::V6(value) if (value.segments()[0] & 0xfe00) == 0xfc00 => DestinationClass::Private,
        IpAddr::V6(value) if (value.segments()[0] & 0xffc0) == 0xfe80 => {
            DestinationClass::LinkLocal
        }
        IpAddr::V6(value) if value.is_multicast() => DestinationClass::Multicast,
        IpAddr::V6(value) if value.is_unspecified() => DestinationClass::Unsupported,
        _ => DestinationClass::Public,
    })
}

pub fn decide(class: DestinationClass) -> RoutingDecision {
    match class {
        DestinationClass::Public => RoutingDecision::ThroughGhostLayer,
        DestinationClass::Loopback | DestinationClass::Private | DestinationClass::LinkLocal => {
            RoutingDecision::LocalSystem
        }
        DestinationClass::Multicast | DestinationClass::Broadcast => RoutingDecision::Drop,
        DestinationClass::Unsupported => RoutingDecision::Unsupported,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ipv4(destination: [u8; 4]) -> NetworkPacket {
        let mut bytes = vec![0u8; 20];
        bytes[0] = 0x45;
        bytes[16..20].copy_from_slice(&destination);
        NetworkPacket::new(bytes, 1500, 1500).unwrap()
    }
    fn ipv6(destination: [u8; 16]) -> NetworkPacket {
        let mut bytes = vec![0u8; 40];
        bytes[0] = 0x60;
        bytes[24..40].copy_from_slice(&destination);
        NetworkPacket::new(bytes, 1500, 1500).unwrap()
    }
    #[test]
    fn classifies_ipv4_destinations() {
        assert_eq!(
            classify_destination(&ipv4([127, 0, 0, 1])),
            Ok(DestinationClass::Loopback)
        );
        assert_eq!(
            classify_destination(&ipv4([10, 0, 0, 1])),
            Ok(DestinationClass::Private)
        );
        assert_eq!(
            classify_destination(&ipv4([169, 254, 1, 1])),
            Ok(DestinationClass::LinkLocal)
        );
        assert_eq!(
            classify_destination(&ipv4([224, 0, 0, 1])),
            Ok(DestinationClass::Multicast)
        );
        assert_eq!(
            classify_destination(&ipv4([255, 255, 255, 255])),
            Ok(DestinationClass::Broadcast)
        );
        assert_eq!(
            classify_destination(&ipv4([198, 51, 100, 1])),
            Ok(DestinationClass::Public)
        );
    }
    #[test]
    fn classifies_ipv6_and_decisions() {
        assert_eq!(
            classify_destination(&ipv6([0; 16])),
            Ok(DestinationClass::Unsupported)
        );
        assert_eq!(
            classify_destination(&ipv6([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1])),
            Ok(DestinationClass::Loopback)
        );
        assert_eq!(
            classify_destination(&ipv6([0xff; 16])),
            Ok(DestinationClass::Multicast)
        );
        assert_eq!(
            decide(DestinationClass::Public),
            RoutingDecision::ThroughGhostLayer
        );
        assert_eq!(
            decide(DestinationClass::Private),
            RoutingDecision::LocalSystem
        );
        assert_eq!(decide(DestinationClass::Multicast), RoutingDecision::Drop);
    }
}
