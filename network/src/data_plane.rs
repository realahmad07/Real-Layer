use crate::{ChannelError, ChannelMessage, ChannelState, EncryptedChannel, SessionId};
use serde::{Deserialize, Serialize};
use std::fmt;

pub const DATA_PLANE_PROTOCOL_VERSION: &str = "1.0";
pub const DATA_PLANE_KIND: &str = "data_plane";
pub const DEFAULT_MAXIMUM_PAYLOAD_SIZE: usize = 2048;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataPlaneEnvelope {
    pub kind: String,
    pub session_id: SessionId,
    pub frame: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataPlaneMessageType {
    Data,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataPlaneMessage {
    pub session_id: SessionId,
    pub sequence_number: u64,
    pub message_type: DataPlaneMessageType,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataPlaneError {
    InvalidSession,
    ClosedSession,
    OversizedPayload,
    MalformedFrame,
    DuplicateMessage,
    SequenceViolation,
    UnavailableChannel,
    Backpressure,
    UnexpectedMessageType,
}

impl fmt::Display for DataPlaneError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for DataPlaneError {}

impl From<ChannelError> for DataPlaneError {
    fn from(error: ChannelError) -> Self {
        match error {
            ChannelError::InvalidSession
            | ChannelError::InvalidPeer
            | ChannelError::SessionInvalid => Self::InvalidSession,
            ChannelError::ChannelClosed => Self::ClosedSession,
            ChannelError::FrameTooLarge => Self::MalformedFrame,
            ChannelError::ReplayDetected => Self::DuplicateMessage,
            ChannelError::InvalidSequence => Self::SequenceViolation,
            ChannelError::Backpressure => Self::Backpressure,
            ChannelError::InvalidFrame
            | ChannelError::UnsupportedVersion
            | ChannelError::DecryptionFailed
            | ChannelError::AuthenticationFailed
            | ChannelError::EncryptionFailed
            | ChannelError::InvalidMessageType => Self::MalformedFrame,
        }
    }
}

pub struct DataPlane<'channel> {
    channel: &'channel mut EncryptedChannel,
    maximum_payload_size: usize,
}

impl<'channel> DataPlane<'channel> {
    pub fn open(
        channel: &'channel mut EncryptedChannel,
        maximum_payload_size: usize,
    ) -> Result<Self, DataPlaneError> {
        if channel.state() != ChannelState::Established || maximum_payload_size == 0 {
            return Err(DataPlaneError::UnavailableChannel);
        }
        Ok(Self {
            channel,
            maximum_payload_size,
        })
    }

    pub fn session_id(&self) -> SessionId {
        self.channel.session_id()
    }

    pub fn send(&mut self, payload: &[u8]) -> Result<Vec<u8>, DataPlaneError> {
        if payload.len() > self.maximum_payload_size {
            return Err(DataPlaneError::OversizedPayload);
        }
        self.channel
            .send(ChannelMessage::DataPlane(payload.to_vec()))
            .map_err(DataPlaneError::from)
    }

    pub fn receive(
        &mut self,
        envelope: &DataPlaneEnvelope,
    ) -> Result<DataPlaneMessage, DataPlaneError> {
        if envelope.kind != DATA_PLANE_KIND || envelope.session_id != self.session_id() {
            return Err(DataPlaneError::InvalidSession);
        }
        let payload = match self.channel.receive(&envelope.frame) {
            Ok(ChannelMessage::DataPlane(payload)) => payload,
            Ok(_) => return Err(DataPlaneError::UnexpectedMessageType),
            Err(error) => return Err(error.into()),
        };
        if payload.len() > self.maximum_payload_size {
            return Err(DataPlaneError::OversizedPayload);
        }
        Ok(DataPlaneMessage {
            session_id: self.session_id(),
            sequence_number: self.channel.receive_sequence() - 1,
            message_type: DataPlaneMessageType::Data,
            payload,
        })
    }

    pub fn close(&mut self) -> Result<(), DataPlaneError> {
        self.channel.close().map_err(DataPlaneError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RouteBinding, SessionInitiator, SessionResponder, CHANNEL_PROTOCOL_VERSION};
    use libp2p::identity;

    fn planes() -> (EncryptedChannel, EncryptedChannel) {
        let initiator_identity = identity::Keypair::generate_ed25519();
        let responder_identity = identity::Keypair::generate_ed25519();
        let peer = responder_identity.public().to_peer_id();
        let route = RouteBinding::OneHop { relay: peer };
        let initiator = SessionInitiator::new(&initiator_identity, peer, route, "1.0").unwrap();
        let init = initiator.build_init().unwrap();
        let mut responder = SessionResponder::new(&responder_identity);
        let (response, responder_session) = responder
            .accept_init(init, initiator_identity.public().to_peer_id(), "1.0")
            .unwrap();
        let initiator_session = initiator.complete(response).unwrap();
        (
            EncryptedChannel::open(initiator_session, CHANNEL_PROTOCOL_VERSION, 4096, 1).unwrap(),
            EncryptedChannel::open(responder_session, CHANNEL_PROTOCOL_VERSION, 4096, 1).unwrap(),
        )
    }

    #[test]
    fn data_plane_round_trip_binds_session_and_sequence() {
        let (mut sender, mut receiver) = planes();
        let session_id = sender.session_id();
        let frame = DataPlane::open(&mut sender, 32)
            .unwrap()
            .send(b"hello")
            .unwrap();
        let envelope = DataPlaneEnvelope {
            kind: DATA_PLANE_KIND.to_owned(),
            session_id,
            frame,
        };
        let message = DataPlane::open(&mut receiver, 32)
            .unwrap()
            .receive(&envelope)
            .unwrap();
        assert_eq!(message.payload, b"hello");
        assert_eq!(message.sequence_number, 0);
    }

    #[test]
    fn data_plane_rejects_limits_replay_wrong_kind_and_backpressure() {
        let (mut sender, mut receiver) = planes();
        assert_eq!(
            DataPlane::open(&mut sender, 4).unwrap().send(b"12345"),
            Err(DataPlaneError::OversizedPayload)
        );
        let session_id = sender.session_id();
        let frame = DataPlane::open(&mut sender, 32)
            .unwrap()
            .send(b"hello")
            .unwrap();
        let wrong = DataPlaneEnvelope {
            kind: "channel".to_owned(),
            session_id,
            frame: frame.clone(),
        };
        assert_eq!(
            DataPlane::open(&mut receiver, 32).unwrap().receive(&wrong),
            Err(DataPlaneError::InvalidSession)
        );
        let valid = DataPlaneEnvelope {
            kind: DATA_PLANE_KIND.to_owned(),
            session_id,
            frame,
        };
        let mut plane = DataPlane::open(&mut receiver, 32).unwrap();
        assert_eq!(plane.receive(&valid).unwrap().payload, b"hello");
        assert_eq!(plane.receive(&valid), Err(DataPlaneError::DuplicateMessage));
        let (mut limited, _) = planes();
        let mut plane = DataPlane::open(&mut limited, 32).unwrap();
        assert!(plane.send(b"one").is_ok());
        assert_eq!(plane.send(b"two"), Err(DataPlaneError::Backpressure));
    }

    #[test]
    fn data_plane_rejects_malformed_and_unexpected_frames_and_closes() {
        assert!(serde_json::from_slice::<DataPlaneEnvelope>(b"not-json").is_err());
        let (mut sender, mut receiver) = planes();
        let session_id = sender.session_id();
        let control_frame = sender
            .send(ChannelMessage::Ping)
            .expect("encrypt control frame");
        let control = DataPlaneEnvelope {
            kind: DATA_PLANE_KIND.to_owned(),
            session_id,
            frame: control_frame,
        };
        assert_eq!(
            DataPlane::open(&mut receiver, 32)
                .unwrap()
                .receive(&control),
            Err(DataPlaneError::UnexpectedMessageType)
        );
        sender.take_pending();
        receiver.close().unwrap();
        assert!(matches!(
            DataPlane::open(&mut receiver, 32),
            Err(DataPlaneError::UnavailableChannel)
        ));
    }
}
