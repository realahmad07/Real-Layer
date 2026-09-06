use crate::{NetworkPacket, PacketError};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MtuPolicy {
    pub tunnel_mtu: usize,
    pub maximum_packet_size: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MtuError {
    InvalidConfiguration,
    Packet(PacketError),
}

impl fmt::Display for MtuError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}
impl std::error::Error for MtuError {}

impl MtuPolicy {
    pub fn new(tunnel_mtu: usize, maximum_packet_size: usize) -> Result<Self, MtuError> {
        if tunnel_mtu == 0 || maximum_packet_size == 0 || maximum_packet_size > tunnel_mtu {
            return Err(MtuError::InvalidConfiguration);
        }
        Ok(Self {
            tunnel_mtu,
            maximum_packet_size,
        })
    }

    pub fn validate(&self, bytes: Vec<u8>) -> Result<NetworkPacket, MtuError> {
        NetworkPacket::new(bytes, self.maximum_packet_size, self.tunnel_mtu)
            .map_err(MtuError::Packet)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn packet(size: usize) -> Vec<u8> {
        let mut bytes = vec![0u8; size];
        bytes[0] = 0x45;
        bytes
    }
    #[test]
    fn validates_exact_below_oversized_and_empty_packets() {
        let policy = MtuPolicy::new(64, 64).unwrap();
        assert_eq!(policy.validate(packet(64)).unwrap().len(), 64);
        assert_eq!(policy.validate(packet(20)).unwrap().len(), 20);
        assert_eq!(
            policy.validate(packet(65)),
            Err(MtuError::Packet(PacketError::Oversized {
                size: 65,
                maximum: 64
            }))
        );
        assert_eq!(
            policy.validate(Vec::new()),
            Err(MtuError::Packet(PacketError::Empty))
        );
    }
    #[test]
    fn rejects_invalid_configuration() {
        assert_eq!(MtuPolicy::new(0, 1), Err(MtuError::InvalidConfiguration));
        assert_eq!(MtuPolicy::new(10, 11), Err(MtuError::InvalidConfiguration));
    }
}
