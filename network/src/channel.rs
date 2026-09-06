use crate::{PeerId, RouteBinding, SecureSession, SessionError, SessionId, SessionState};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fmt;

pub const CHANNEL_PROTOCOL_VERSION: &str = "1.0";
const FRAME_OVERHEAD_LIMIT: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChannelMessage {
    Ping,
    Pong,
    SessionData(Vec<u8>),
    Close,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelEnvelope {
    pub kind: String,
    pub session_id: SessionId,
    pub frame: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelState {
    Opening,
    Established,
    Closing,
    Closed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct FrameHeader {
    protocol_version: String,
    session_id: SessionId,
    message_type: MessageType,
    sequence_number: u64,
    ciphertext_length: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum MessageType {
    Ping,
    Pong,
    SessionData,
    Close,
}

impl From<&ChannelMessage> for MessageType {
    fn from(message: &ChannelMessage) -> Self {
        match message {
            ChannelMessage::Ping => Self::Ping,
            ChannelMessage::Pong => Self::Pong,
            ChannelMessage::SessionData(_) => Self::SessionData,
            ChannelMessage::Close => Self::Close,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelError {
    InvalidFrame,
    FrameTooLarge,
    InvalidSession,
    InvalidPeer,
    UnsupportedVersion,
    InvalidMessageType,
    ReplayDetected,
    InvalidSequence,
    AuthenticationFailed,
    EncryptionFailed,
    DecryptionFailed,
    ChannelClosed,
    Backpressure,
    SessionInvalid,
}

impl fmt::Display for ChannelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}
impl std::error::Error for ChannelError {}

impl From<SessionError> for ChannelError {
    fn from(error: SessionError) -> Self {
        match error {
            SessionError::InvalidState { .. } => Self::SessionInvalid,
            SessionError::DecryptionFailed => Self::DecryptionFailed,
            SessionError::EncryptionFailed => Self::EncryptionFailed,
            _ => Self::AuthenticationFailed,
        }
    }
}

#[derive(Debug)]
pub struct EncryptedChannel {
    session: SecureSession,
    state: ChannelState,
    protocol_version: String,
    maximum_frame_size: usize,
    maximum_pending_messages: usize,
    pending_outbound: VecDeque<Vec<u8>>,
    send_sequence: u64,
    receive_sequence: u64,
}

impl EncryptedChannel {
    pub fn open(
        session: SecureSession,
        protocol_version: impl Into<String>,
        maximum_frame_size: usize,
        maximum_pending_messages: usize,
    ) -> Result<Self, ChannelError> {
        if session.state() != SessionState::Established
            || maximum_frame_size <= FRAME_OVERHEAD_LIMIT
            || maximum_pending_messages == 0
        {
            return Err(ChannelError::SessionInvalid);
        }
        Ok(Self {
            session,
            state: ChannelState::Established,
            protocol_version: protocol_version.into(),
            maximum_frame_size,
            maximum_pending_messages,
            pending_outbound: VecDeque::new(),
            send_sequence: 0,
            receive_sequence: 0,
        })
    }

    pub fn session_id(&self) -> SessionId {
        self.session.session_id()
    }
    pub fn peer_id(&self) -> PeerId {
        self.session.peer_id()
    }
    pub fn route(&self) -> &RouteBinding {
        self.session.route()
    }
    pub fn state(&self) -> ChannelState {
        self.state
    }
    pub fn send_sequence(&self) -> u64 {
        self.send_sequence
    }
    pub fn receive_sequence(&self) -> u64 {
        self.receive_sequence
    }

    pub fn send(&mut self, message: ChannelMessage) -> Result<Vec<u8>, ChannelError> {
        self.ensure_established()?;
        let sequence = self.send_sequence;
        self.send_sequence = self
            .send_sequence
            .checked_add(1)
            .ok_or(ChannelError::InvalidSequence)?;
        let message_type = MessageType::from(&message);
        let plaintext =
            serde_json::to_vec(&message).map_err(|_| ChannelError::InvalidMessageType)?;
        let header = FrameHeader {
            protocol_version: self.protocol_version.clone(),
            session_id: self.session_id(),
            message_type,
            sequence_number: sequence,
            ciphertext_length: 0,
        };
        let associated_data =
            serde_json::to_vec(&header).map_err(|_| ChannelError::InvalidFrame)?;
        let ciphertext = self.session.encrypt(&plaintext, &associated_data)?;
        let header = FrameHeader {
            ciphertext_length: ciphertext.len() as u32,
            ..header
        };
        let header_bytes = serde_json::to_vec(&header).map_err(|_| ChannelError::InvalidFrame)?;
        let mut frame = (header_bytes.len() as u32).to_be_bytes().to_vec();
        frame.extend(header_bytes);
        frame.extend(ciphertext);
        if frame.len() > self.maximum_frame_size {
            self.send_sequence = sequence;
            return Err(ChannelError::FrameTooLarge);
        }
        if self.pending_outbound.len() >= self.maximum_pending_messages {
            self.send_sequence = sequence;
            return Err(ChannelError::Backpressure);
        }
        self.pending_outbound.push_back(frame.clone());
        if message_type == MessageType::Close {
            self.state = ChannelState::Closing;
        }
        Ok(frame)
    }

    pub fn take_pending(&mut self) -> Option<Vec<u8>> {
        self.pending_outbound.pop_front()
    }

    pub fn receive(&mut self, frame: &[u8]) -> Result<ChannelMessage, ChannelError> {
        self.ensure_established()?;
        if frame.len() > self.maximum_frame_size || frame.len() < 4 {
            return Err(ChannelError::FrameTooLarge);
        }
        let header_len = u32::from_be_bytes(
            frame[..4]
                .try_into()
                .map_err(|_| ChannelError::InvalidFrame)?,
        ) as usize;
        if header_len == 0 || header_len > self.maximum_frame_size || frame.len() < 4 + header_len {
            return Err(ChannelError::InvalidFrame);
        }
        let header: FrameHeader = serde_json::from_slice(&frame[4..4 + header_len])
            .map_err(|_| ChannelError::InvalidFrame)?;
        if header.protocol_version != self.protocol_version {
            return Err(ChannelError::UnsupportedVersion);
        }
        if header.session_id != self.session_id() {
            return Err(ChannelError::InvalidSession);
        }
        if header.sequence_number < self.receive_sequence {
            return Err(ChannelError::ReplayDetected);
        }
        if header.sequence_number != self.receive_sequence {
            return Err(ChannelError::InvalidSequence);
        }
        let ciphertext = &frame[4 + header_len..];
        if ciphertext.len() != header.ciphertext_length as usize {
            return Err(ChannelError::InvalidFrame);
        }
        let associated_data = serde_json::to_vec(&FrameHeader {
            ciphertext_length: 0,
            ..header.clone()
        })
        .map_err(|_| ChannelError::InvalidFrame)?;
        let plaintext = self.session.decrypt(ciphertext, &associated_data)?;
        let message: ChannelMessage =
            serde_json::from_slice(&plaintext).map_err(|_| ChannelError::InvalidMessageType)?;
        if MessageType::from(&message) != header.message_type {
            return Err(ChannelError::InvalidMessageType);
        }
        self.receive_sequence = self
            .receive_sequence
            .checked_add(1)
            .ok_or(ChannelError::InvalidSequence)?;
        if matches!(message, ChannelMessage::Close) {
            self.state = ChannelState::Closing;
        }
        Ok(message)
    }

    pub fn close(&mut self) -> Result<(), ChannelError> {
        match self.state {
            ChannelState::Established | ChannelState::Closing => {
                self.state = ChannelState::Closed;
                self.session.close().map_err(ChannelError::from)
            }
            ChannelState::Closed | ChannelState::Failed => Err(ChannelError::ChannelClosed),
            ChannelState::Opening => Err(ChannelError::SessionInvalid),
        }
    }

    pub fn mark_remote_closed(&mut self) {
        self.state = ChannelState::Closed;
    }

    fn ensure_established(&self) -> Result<(), ChannelError> {
        if self.state == ChannelState::Established {
            Ok(())
        } else {
            Err(ChannelError::ChannelClosed)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RouteBinding, SessionInitiator, SessionResponder};
    use libp2p::identity;

    fn channels() -> (EncryptedChannel, EncryptedChannel) {
        let initiator_identity = identity::Keypair::generate_ed25519();
        let responder_identity = identity::Keypair::generate_ed25519();
        let peer = responder_identity.public().to_peer_id();
        let route = RouteBinding::OneHop { relay: peer };
        let initiator =
            SessionInitiator::new(&initiator_identity, peer, route.clone(), "1.0").unwrap();
        let init = initiator.build_init().unwrap();
        let mut responder = SessionResponder::new(&responder_identity);
        let (response, responder_session) = responder
            .accept_init(init, initiator_identity.public().to_peer_id(), "1.0")
            .unwrap();
        let initiator_session = initiator.complete(response).unwrap();
        (
            EncryptedChannel::open(initiator_session, CHANNEL_PROTOCOL_VERSION, 4096, 2).unwrap(),
            EncryptedChannel::open(responder_session, CHANNEL_PROTOCOL_VERSION, 4096, 2).unwrap(),
        )
    }

    #[test]
    fn encrypted_messages_exchange_and_sequences_are_directional() {
        let (mut sender, mut receiver) = channels();
        let frame = sender.send(ChannelMessage::Ping).unwrap();
        assert_eq!(receiver.receive(&frame).unwrap(), ChannelMessage::Ping);
        let response = receiver.send(ChannelMessage::Pong).unwrap();
        assert_eq!(sender.receive(&response).unwrap(), ChannelMessage::Pong);
        assert_eq!(sender.send_sequence(), 1);
        assert_eq!(receiver.send_sequence(), 1);
    }

    #[test]
    fn tampering_replay_wrong_session_and_invalid_lifecycle_are_rejected() {
        let (mut sender, mut receiver) = channels();
        let frame = sender
            .send(ChannelMessage::SessionData(b"test".to_vec()))
            .unwrap();
        let mut modified = frame.clone();
        *modified.last_mut().unwrap() ^= 1;
        assert!(matches!(
            receiver.receive(&modified),
            Err(ChannelError::DecryptionFailed)
        ));
        assert_eq!(
            receiver.receive(&frame).unwrap(),
            ChannelMessage::SessionData(b"test".to_vec())
        );
        assert_eq!(receiver.receive(&frame), Err(ChannelError::ReplayDetected));
        receiver.close().unwrap();
        assert_eq!(
            receiver.send(ChannelMessage::Ping),
            Err(ChannelError::ChannelClosed)
        );
        assert_eq!(receiver.receive(&frame), Err(ChannelError::ChannelClosed));
    }

    #[test]
    fn malformed_oversized_and_out_of_order_frames_are_rejected() {
        let (mut sender, mut receiver) = channels();
        let mut malformed = vec![0, 0, 0, 2, b'{', b'}'];
        assert_eq!(
            receiver.receive(&malformed),
            Err(ChannelError::InvalidFrame)
        );
        let oversized = vec![0u8; 4097];
        assert_eq!(
            receiver.receive(&oversized),
            Err(ChannelError::FrameTooLarge)
        );
        let first = sender.send(ChannelMessage::Ping).unwrap();
        let second = sender.send(ChannelMessage::Pong).unwrap();
        assert_eq!(
            receiver.receive(&second),
            Err(ChannelError::InvalidSequence)
        );
        assert_eq!(receiver.receive(&first).unwrap(), ChannelMessage::Ping);
        malformed.clear();
    }

    #[test]
    fn session_binding_and_backpressure_are_enforced() {
        let (mut sender, mut receiver) = channels();
        let first = sender.send(ChannelMessage::Ping).unwrap();
        let second = sender.send(ChannelMessage::Pong).unwrap();
        assert_eq!(
            sender.send(ChannelMessage::Close),
            Err(ChannelError::Backpressure)
        );
        assert_eq!(receiver.receive(&first).unwrap(), ChannelMessage::Ping);
        let (mut other_sender, _) = channels();
        let wrong_session = other_sender.send(ChannelMessage::Ping).unwrap();
        assert_eq!(
            receiver.receive(&wrong_session),
            Err(ChannelError::InvalidSession)
        );
        assert_eq!(receiver.receive(&second).unwrap(), ChannelMessage::Pong);
    }

    #[test]
    fn debug_does_not_expose_channel_keys() {
        let (channel, _) = channels();
        let debug = format!("{channel:?}");
        assert!(!debug.contains("send_key"));
        assert!(!debug.contains("receive_key"));
    }
}
