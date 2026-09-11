use ghost_layer_network::{
    packet::NetworkPacket,
    tun::{TunDevice, TunError},
    TunConfig,
};
use std::fs::File;
use std::io::{Read, Write};

pub struct AndroidTunDevice {
    file: File,
    config: TunConfig,
}

impl AndroidTunDevice {
    pub fn from_file(file: File, config: TunConfig) -> Self {
        Self {
            file,
            config,
        }
    }
}

impl TunDevice for AndroidTunDevice {
    fn read_packet(&mut self) -> Result<Option<NetworkPacket>, TunError> {
        let mut buf = vec![0u8; 65535];
        match self.file.read(&mut buf) {
            Ok(n) if n > 0 => {
                buf.truncate(n);
                match NetworkPacket::new(buf, self.config.maximum_packet_size, self.config.mtu) {
                    Ok(packet) => Ok(Some(packet)),
                    Err(e) => Err(TunError::InvalidPacket(e)),
                }
            }
            Ok(_) => Ok(None),
            Err(e) => Err(TunError::ReadFailed(e.to_string())),
        }
    }

    fn write_packet(&mut self, packet: NetworkPacket) -> Result<(), TunError> {
        match self.file.write_all(packet.as_bytes()) {
            Ok(_) => Ok(()),
            Err(e) => Err(TunError::WriteFailed(e.to_string())),
        }
    }

    fn close(&mut self) -> Result<(), TunError> {
        Ok(())
    }
}
