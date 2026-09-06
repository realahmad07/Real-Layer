use super::{TunDevice, TunError};
use crate::{
    DataPlane, DataPlaneEnvelope, DataPlaneError, NetworkPacket, PacketError, DATA_PLANE_KIND,
};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TunPipelineError {
    Tun(TunError),
    DataPlane(DataPlaneError),
    InvalidPacket(PacketError),
}

impl fmt::Display for TunPipelineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for TunPipelineError {}

impl From<TunError> for TunPipelineError {
    fn from(error: TunError) -> Self {
        Self::Tun(error)
    }
}

impl From<DataPlaneError> for TunPipelineError {
    fn from(error: DataPlaneError) -> Self {
        Self::DataPlane(error)
    }
}

impl<'channel, 'device, Device> TunDataPlane<'channel, 'device, Device>
where
    Device: TunDevice + ?Sized,
{
    pub fn new(
        device: &'device mut Device,
        data_plane: &'device mut DataPlane<'channel>,
        maximum_packet_size: usize,
        mtu: usize,
    ) -> Result<Self, TunPipelineError> {
        if maximum_packet_size == 0 || mtu == 0 {
            return Err(TunPipelineError::InvalidPacket(PacketError::InvalidLimit));
        }
        Ok(Self {
            device,
            data_plane,
            maximum_packet_size,
            mtu,
        })
    }

    pub fn read_frame(&mut self) -> Result<Option<DataPlaneEnvelope>, TunPipelineError> {
        let Some(packet) = self.device.read_packet()? else {
            return Ok(None);
        };
        let frame = self.data_plane.send(packet.as_bytes())?;
        Ok(Some(DataPlaneEnvelope {
            kind: DATA_PLANE_KIND.to_owned(),
            session_id: self.data_plane.session_id(),
            frame,
        }))
    }

    pub fn write_frame(
        &mut self,
        envelope: &DataPlaneEnvelope,
    ) -> Result<NetworkPacket, TunPipelineError> {
        let message = self.data_plane.receive(envelope)?;
        let packet = NetworkPacket::new(message.payload, self.maximum_packet_size, self.mtu)
            .map_err(TunPipelineError::InvalidPacket)?;
        self.device.write_packet(packet.clone())?;
        Ok(packet)
    }
}

pub struct TunDataPlane<'channel, 'device, Device: TunDevice + ?Sized> {
    device: &'device mut Device,
    data_plane: &'device mut DataPlane<'channel>,
    maximum_packet_size: usize,
    mtu: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tun::mock::MockTunDevice;
    use crate::{
        EncryptedChannel, RouteBinding, SessionInitiator, SessionResponder,
        CHANNEL_PROTOCOL_VERSION,
    };
    use libp2p::identity;

    fn channels() -> (EncryptedChannel, EncryptedChannel) {
        let initiator_identity = identity::Keypair::generate_ed25519();
        let responder_identity = identity::Keypair::generate_ed25519();
        let peer = responder_identity.public().to_peer_id();
        let initiator = SessionInitiator::new(
            &initiator_identity,
            peer,
            RouteBinding::OneHop { relay: peer },
            "1.0",
        )
        .unwrap();
        let init = initiator.build_init().unwrap();
        let mut responder = SessionResponder::new(&responder_identity);
        let (response, responder_session) = responder
            .accept_init(init, initiator_identity.public().to_peer_id(), "1.0")
            .unwrap();
        (
            EncryptedChannel::open(
                initiator.complete(response).unwrap(),
                CHANNEL_PROTOCOL_VERSION,
                4096,
                4,
            )
            .unwrap(),
            EncryptedChannel::open(responder_session, CHANNEL_PROTOCOL_VERSION, 4096, 4).unwrap(),
        )
    }

    fn packet_bytes() -> Vec<u8> {
        let mut packet = vec![0u8; 20];
        packet[0] = 0x45;
        packet[19] = 1;
        packet
    }

    fn config() -> crate::TunConfig {
        crate::TunConfig {
            enabled: false,
            interface_name: "mock0".to_owned(),
            mtu: 1500,
            maximum_packet_size: 1500,
            read_buffer_limit: 4,
            write_buffer_limit: 4,
        }
    }

    #[test]
    fn moves_valid_packet_from_tun_through_data_plane() {
        let (mut sender, _) = channels();
        let mut device = MockTunDevice::open(config());
        let bytes = packet_bytes();
        device.inject_packet(bytes.clone()).unwrap();
        let session_id = sender.session_id();
        let mut plane = DataPlane::open(&mut sender, 1500).unwrap();
        let envelope = TunDataPlane::new(&mut device, &mut plane, 1500, 1500)
            .unwrap()
            .read_frame()
            .unwrap()
            .unwrap();
        assert_eq!(envelope.kind, DATA_PLANE_KIND);
        assert_eq!(envelope.session_id, session_id);
        assert!(!envelope.frame.is_empty());
        assert_eq!(device.read_packet().unwrap(), None);
    }

    #[test]
    fn writes_data_plane_response_to_tun_without_changing_bytes() {
        let (mut sender, mut receiver) = channels();
        let mut source = MockTunDevice::open(config());
        source.inject_packet(packet_bytes()).unwrap();
        let sender_session_id = sender.session_id();
        let mut sender_plane = DataPlane::open(&mut sender, 1500).unwrap();
        let envelope = TunDataPlane::new(&mut source, &mut sender_plane, 1500, 1500)
            .unwrap()
            .read_frame()
            .unwrap()
            .unwrap();
        let mut receiver_plane = DataPlane::open(&mut receiver, 1500).unwrap();
        let message = receiver_plane.receive(&envelope).unwrap();
        assert_eq!(message.payload, packet_bytes());
        let response_frame = receiver_plane.send(&message.payload).unwrap();
        let response = DataPlaneEnvelope {
            kind: DATA_PLANE_KIND.to_owned(),
            session_id: sender_session_id,
            frame: response_frame,
        };
        let mut destination = MockTunDevice::open(config());
        let mut response_plane = DataPlane::open(&mut sender, 1500).unwrap();
        let packet = TunDataPlane::new(&mut destination, &mut response_plane, 1500, 1500)
            .unwrap()
            .write_frame(&response)
            .unwrap();
        assert_eq!(packet.as_bytes(), packet_bytes());
        assert_eq!(
            destination.take_written().unwrap().unwrap().as_bytes(),
            packet_bytes()
        );
    }
}
