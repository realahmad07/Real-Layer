use crate::{
    classify_destination, decide, DataPlane, DataPlaneEnvelope, DataPlaneError, DestinationClass,
    MtuError, MtuPolicy, NetworkPacket, RoutingDecision, RoutingError,
};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineError {
    Mtu(MtuError),
    Routing(RoutingError),
    DataPlane(DataPlaneError),
    Dropped(DestinationClass),
    UnsupportedDestination,
}
impl fmt::Display for PipelineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for PipelineError {}
impl From<MtuError> for PipelineError {
    fn from(value: MtuError) -> Self {
        Self::Mtu(value)
    }
}
impl From<RoutingError> for PipelineError {
    fn from(value: RoutingError) -> Self {
        Self::Routing(value)
    }
}
impl From<DataPlaneError> for PipelineError {
    fn from(value: DataPlaneError) -> Self {
        Self::DataPlane(value)
    }
}

pub struct PacketPipeline<'channel> {
    mtu: MtuPolicy,
    data_plane: &'channel mut DataPlane<'channel>,
}
impl<'channel> PacketPipeline<'channel> {
    pub fn new(mtu: MtuPolicy, data_plane: &'channel mut DataPlane<'channel>) -> Self {
        Self { mtu, data_plane }
    }
    pub fn route(&self, bytes: Vec<u8>) -> Result<(NetworkPacket, RoutingDecision), PipelineError> {
        let packet = self.mtu.validate(bytes)?;
        let class = classify_destination(&packet)?;
        let decision = decide(class);
        match decision {
            RoutingDecision::Unsupported => Err(PipelineError::UnsupportedDestination),
            RoutingDecision::Drop => Err(PipelineError::Dropped(class)),
            _ => Ok((packet, decision)),
        }
    }
    pub fn send(&mut self, bytes: Vec<u8>) -> Result<DataPlaneEnvelope, PipelineError> {
        let (packet, decision) = self.route(bytes)?;
        if decision != RoutingDecision::ThroughGhostLayer {
            return Err(PipelineError::Dropped(classify_destination(&packet)?));
        }
        let frame = self.data_plane.send(packet.as_bytes())?;
        Ok(DataPlaneEnvelope {
            kind: crate::DATA_PLANE_KIND.to_owned(),
            session_id: self.data_plane.session_id(),
            frame,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        EncryptedChannel, RouteBinding, SessionInitiator, SessionResponder,
        CHANNEL_PROTOCOL_VERSION,
    };
    use libp2p::identity;

    fn channel_pair() -> (EncryptedChannel, EncryptedChannel) {
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
                2,
            )
            .unwrap(),
            EncryptedChannel::open(responder_session, CHANNEL_PROTOCOL_VERSION, 4096, 2).unwrap(),
        )
    }

    fn packet(destination: [u8; 4]) -> Vec<u8> {
        let mut bytes = vec![0u8; 20];
        bytes[0] = 0x45;
        bytes[16..20].copy_from_slice(&destination);
        bytes
    }

    #[test]
    fn pipeline_routes_public_packets_into_existing_data_plane() {
        let (mut sender, mut receiver) = channel_pair();
        let mtu = MtuPolicy::new(1500, 1500).unwrap();
        let mut sender_data_plane = DataPlane::open(&mut sender, 1500).unwrap();
        let mut pipeline = PacketPipeline::new(mtu, &mut sender_data_plane);
        let envelope = pipeline.send(packet([198, 51, 100, 1])).unwrap();
        let mut data_plane = DataPlane::open(&mut receiver, 1500).unwrap();
        assert_eq!(
            data_plane.receive(&envelope).unwrap().payload,
            packet([198, 51, 100, 1])
        );
        assert_eq!(
            pipeline.route(packet([127, 0, 0, 1])).unwrap().1,
            RoutingDecision::LocalSystem
        );
        assert!(matches!(
            pipeline.send(packet([127, 0, 0, 1])),
            Err(PipelineError::Dropped(DestinationClass::Loopback))
        ));
    }

    #[test]
    fn pipeline_rejects_invalid_and_oversized_packets_before_encryption() {
        let (mut sender, _) = channel_pair();
        let mut sender_data_plane = DataPlane::open(&mut sender, 20).unwrap();
        let mut pipeline =
            PacketPipeline::new(MtuPolicy::new(20, 20).unwrap(), &mut sender_data_plane);
        assert!(matches!(
            pipeline.send(vec![]),
            Err(PipelineError::Mtu(MtuError::Packet(
                crate::PacketError::Empty
            )))
        ));
        assert!(matches!(
            pipeline.send(vec![0x45; 21]),
            Err(PipelineError::Mtu(MtuError::Packet(
                crate::PacketError::Oversized { .. }
            )))
        ));
    }
}
