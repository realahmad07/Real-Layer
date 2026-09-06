use crate::packet::NetworkPacket;
use std::fmt;

pub mod bridge;
pub mod mock;

#[cfg(windows)]
pub mod windows;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TunError {
    Closed,
    QueueFull,
    InvalidPacket(crate::packet::PacketError),
    DriverUnavailable,
    DeviceUnavailable,
    DeviceInitializationFailed(String),
    InvalidConfiguration(String),
    ReadFailed(String),
    WriteFailed(String),
    Shutdown,
}

impl fmt::Display for TunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for TunError {}

pub trait TunDevice {
    fn read_packet(&mut self) -> Result<Option<NetworkPacket>, TunError>;
    fn write_packet(&mut self, packet: NetworkPacket) -> Result<(), TunError>;
    fn close(&mut self) -> Result<(), TunError>;
}

pub use bridge::{TunDataPlane, TunPipelineError};
