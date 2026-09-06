use std::fmt;

pub const IPV4_VERSION: u8 = 4;
pub const IPV6_VERSION: u8 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacketType {
    Ipv4,
    Ipv6,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkPacket {
    bytes: Vec<u8>,
    packet_type: PacketType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PacketError {
    Empty,
    Oversized { size: usize, maximum: usize },
    ExceedsMtu { size: usize, mtu: usize },
    Malformed,
    UnsupportedType { version: u8 },
    InvalidLimit,
}

impl fmt::Display for PacketError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for PacketError {}

impl NetworkPacket {
    pub fn new(bytes: Vec<u8>, maximum_size: usize, mtu: usize) -> Result<Self, PacketError> {
        if maximum_size == 0 || mtu == 0 {
            return Err(PacketError::InvalidLimit);
        }
        let size = bytes.len();
        if size == 0 {
            return Err(PacketError::Empty);
        }
        if size > maximum_size {
            return Err(PacketError::Oversized {
                size,
                maximum: maximum_size,
            });
        }
        if size > mtu {
            return Err(PacketError::ExceedsMtu { size, mtu });
        }
        let version = bytes[0] >> 4;
        let packet_type = match version {
            IPV4_VERSION => {
                let header_length = usize::from(bytes[0] & 0x0f) * 4;
                if header_length < 20 || size < header_length {
                    return Err(PacketError::Malformed);
                }
                PacketType::Ipv4
            }
            IPV6_VERSION => {
                if size < 40 {
                    return Err(PacketError::Malformed);
                }
                PacketType::Ipv6
            }
            version => return Err(PacketError::UnsupportedType { version }),
        };
        Ok(Self { bytes, packet_type })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn packet_type(&self) -> PacketType {
        self.packet_type
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ipv4() -> Vec<u8> {
        let mut packet = vec![0u8; 20];
        packet[0] = 0x45;
        packet
    }

    #[test]
    fn validates_packet_types_and_limits() {
        let packet = NetworkPacket::new(ipv4(), 1500, 1500).unwrap();
        assert_eq!(packet.packet_type(), PacketType::Ipv4);
        assert_eq!(packet.as_bytes(), ipv4());
        assert_eq!(
            NetworkPacket::new(vec![], 1500, 1500),
            Err(PacketError::Empty)
        );
        assert_eq!(
            NetworkPacket::new(vec![0x70; 20], 1500, 1500),
            Err(PacketError::UnsupportedType { version: 7 })
        );
    }

    #[test]
    fn identifies_ipv6_and_rejects_malformed_or_oversized_packets() {
        assert_eq!(
            NetworkPacket::new(vec![0x60; 40], 1500, 1500)
                .unwrap()
                .packet_type(),
            PacketType::Ipv6
        );
        assert_eq!(
            NetworkPacket::new(vec![0x45; 19], 1500, 1500),
            Err(PacketError::Malformed)
        );
        assert_eq!(
            NetworkPacket::new(vec![0x45; 1501], 1500, 1500),
            Err(PacketError::Oversized {
                size: 1501,
                maximum: 1500
            })
        );
        assert_eq!(
            NetworkPacket::new(ipv4(), 1500, 19),
            Err(PacketError::ExceedsMtu { size: 20, mtu: 19 })
        );
    }
}
