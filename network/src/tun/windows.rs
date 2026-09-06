use super::{TunDevice, TunError};
use crate::config::TunConfig;
use crate::packet::NetworkPacket;
use std::io::{Read, Write};

pub struct WindowsTunDevice {
    device: tun::Device,
    config: TunConfig,
    closed: bool,
}

impl WindowsTunDevice {
    pub fn open(config: TunConfig) -> Result<Self, TunError> {
        config
            .validate()
            .map_err(|error| TunError::InvalidConfiguration(error.to_string()))?;
        if !config.enabled {
            return Err(TunError::DeviceUnavailable);
        }
        let mut device_config = tun::Configuration::default();
        device_config
            .tun_name(&config.interface_name)
            .mtu(config.mtu as u16)
            .layer(tun::Layer::L3)
            .up();
        let device = tun::create(&device_config).map_err(|error| {
            TunError::DeviceInitializationFailed(format!(
                "Wintun driver or interface '{}' is unavailable: {error}",
                config.interface_name
            ))
        })?;
        Ok(Self {
            device,
            config,
            closed: false,
        })
    }
}

impl TunDevice for WindowsTunDevice {
    fn read_packet(&mut self) -> Result<Option<NetworkPacket>, TunError> {
        if self.closed {
            return Err(TunError::Closed);
        }
        let mut buffer = vec![0u8; self.config.maximum_packet_size];
        let size = self
            .device
            .read(&mut buffer)
            .map_err(|error| TunError::ReadFailed(error.to_string()))?;
        if size == 0 {
            return Ok(None);
        }
        buffer.truncate(size);
        NetworkPacket::new(buffer, self.config.maximum_packet_size, self.config.mtu)
            .map(Some)
            .map_err(TunError::InvalidPacket)
    }

    fn write_packet(&mut self, packet: NetworkPacket) -> Result<(), TunError> {
        if self.closed {
            return Err(TunError::Closed);
        }
        if packet.len() > self.config.maximum_packet_size || packet.len() > self.config.mtu {
            return Err(TunError::InvalidPacket(
                crate::packet::PacketError::Oversized {
                    size: packet.len(),
                    maximum: self.config.maximum_packet_size,
                },
            ));
        }
        let written = self
            .device
            .write(packet.as_bytes())
            .map_err(|error| TunError::WriteFailed(error.to_string()))?;
        if written != packet.len() {
            return Err(TunError::WriteFailed("short device write".to_owned()));
        }
        Ok(())
    }

    fn close(&mut self) -> Result<(), TunError> {
        if self.closed {
            return Err(TunError::Closed);
        }
        self.closed = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> TunConfig {
        TunConfig {
            enabled: false,
            interface_name: "ghost-test".to_owned(),
            mtu: 1500,
            maximum_packet_size: 1500,
            read_buffer_limit: 4,
            write_buffer_limit: 4,
        }
    }

    #[test]
    fn disabled_configuration_does_not_open_a_device() {
        assert!(matches!(
            WindowsTunDevice::open(config()),
            Err(TunError::DeviceUnavailable)
        ));
    }

    #[test]
    fn invalid_configuration_is_rejected_before_driver_access() {
        let mut invalid = config();
        invalid.mtu = 0;
        assert!(matches!(
            WindowsTunDevice::open(invalid),
            Err(TunError::InvalidConfiguration(_))
        ));
    }
}
