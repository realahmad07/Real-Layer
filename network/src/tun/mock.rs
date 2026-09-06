use super::{TunDevice, TunError};
use crate::config::TunConfig;
use crate::packet::NetworkPacket;
use std::collections::VecDeque;

pub struct MockTunDevice {
    config: TunConfig,
    inbound: VecDeque<NetworkPacket>,
    outbound: VecDeque<NetworkPacket>,
    closed: bool,
}

impl MockTunDevice {
    pub fn open(config: TunConfig) -> Self {
        Self {
            inbound: VecDeque::with_capacity(config.read_buffer_limit),
            outbound: VecDeque::with_capacity(config.write_buffer_limit),
            config,
            closed: false,
        }
    }

    pub fn inject_packet(&mut self, bytes: Vec<u8>) -> Result<(), TunError> {
        if self.closed {
            return Err(TunError::Closed);
        }
        if self.inbound.len() >= self.config.read_buffer_limit {
            return Err(TunError::QueueFull);
        }
        let packet = NetworkPacket::new(bytes, self.config.maximum_packet_size, self.config.mtu)
            .map_err(TunError::InvalidPacket)?;
        self.inbound.push_back(packet);
        Ok(())
    }

    pub fn take_written(&mut self) -> Result<Option<NetworkPacket>, TunError> {
        if self.closed {
            return Err(TunError::Closed);
        }
        Ok(self.outbound.pop_front())
    }
}

impl TunDevice for MockTunDevice {
    fn read_packet(&mut self) -> Result<Option<NetworkPacket>, TunError> {
        if self.closed {
            return Err(TunError::Closed);
        }
        Ok(self.inbound.pop_front())
    }

    fn write_packet(&mut self, packet: NetworkPacket) -> Result<(), TunError> {
        if self.closed {
            return Err(TunError::Closed);
        }
        if packet.len() > self.config.mtu || packet.len() > self.config.maximum_packet_size {
            return Err(TunError::InvalidPacket(
                crate::packet::PacketError::Oversized {
                    size: packet.len(),
                    maximum: self.config.maximum_packet_size,
                },
            ));
        }
        if self.outbound.len() >= self.config.write_buffer_limit {
            return Err(TunError::QueueFull);
        }
        self.outbound.push_back(packet);
        Ok(())
    }

    fn close(&mut self) -> Result<(), TunError> {
        if self.closed {
            return Err(TunError::Closed);
        }
        self.closed = true;
        self.inbound.clear();
        self.outbound.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TunConfig;
    use crate::packet::PacketError;

    fn config() -> TunConfig {
        TunConfig {
            enabled: false,
            interface_name: "mock0".to_owned(),
            mtu: 1500,
            maximum_packet_size: 1500,
            read_buffer_limit: 1,
            write_buffer_limit: 1,
        }
    }

    fn packet() -> Vec<u8> {
        let mut packet = vec![0u8; 20];
        packet[0] = 0x45;
        packet
    }

    #[test]
    fn reads_writes_and_bounds_queues() {
        let mut device = MockTunDevice::open(config());
        device.inject_packet(packet()).unwrap();
        assert_eq!(device.inject_packet(packet()), Err(TunError::QueueFull));
        assert_eq!(device.read_packet().unwrap().unwrap().len(), 20);
        let network_packet = NetworkPacket::new(packet(), 1500, 1500).unwrap();
        device.write_packet(network_packet.clone()).unwrap();
        assert_eq!(
            device.write_packet(network_packet),
            Err(TunError::QueueFull)
        );
        assert_eq!(device.take_written().unwrap().unwrap().len(), 20);
    }

    #[test]
    fn rejects_invalid_input_and_closed_operations() {
        let mut device = MockTunDevice::open(config());
        assert!(matches!(
            device.inject_packet(vec![]),
            Err(TunError::InvalidPacket(PacketError::Empty))
        ));
        device.close().unwrap();
        assert_eq!(device.read_packet(), Err(TunError::Closed));
        assert_eq!(device.inject_packet(packet()), Err(TunError::Closed));
    }
}
