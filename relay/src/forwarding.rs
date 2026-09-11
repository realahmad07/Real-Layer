use ghost_layer_network::{DataPlaneEnvelope, PeerId, RouteBinding, SessionId};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::net::SocketAddr;

pub const FORWARDING_KIND: &str = "ghost_layer_forwarding";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ForwardedProtocol {
    Tcp,
    Udp,
    Dns,
    Ip,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForwardedPacket {
    pub protocol: ForwardedProtocol,
    pub source: Option<SocketAddr>,
    pub destination: SocketAddr,
    pub flow_id: SessionId,
    pub session_id: SessionId,
    pub route_id: SessionId,
    pub sequence: u64,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hop {
    Entry,
    Exit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForwardingDirection {
    ClientToExit,
    ExitToClient,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForwardingState {
    Created,
    Connecting,
    Established,
    Forwarding,
    Closing,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForwardingMessage {
    pub kind: String,
    pub forwarding_id: SessionId,
    pub client_session_id: SessionId,
    pub client_peer: String,
    pub route: RouteBinding,
    pub data: DataPlaneEnvelope,
    #[serde(default)]
    pub packet: Option<ForwardedPacket>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForwardingError {
    InvalidState {
        state: ForwardingState,
        operation: &'static str,
    },
    InvalidRoute,
    InvalidForwardingContext,
    WrongEntryRelay,
    WrongExitRelay,
    WrongSessionBinding,
    DuplicateForwardingRequest,
    ClosedContext,
    MalformedForwardingMessage,
    OversizedPayload,
    ExitUnavailable,
    EntrySessionClosed,
    ExitSessionClosed,
    AuthenticationFailed,
    SequenceViolation,
    UnsupportedProtocol,
    ForwardingTimeout,
    Backpressure,
    UnexpectedProtocolMessage,
    Shutdown,
}

impl fmt::Display for ForwardingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for ForwardingError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiHopSession {
    pub client_session_id: SessionId,
    pub relay_session_id: SessionId,
    pub client_peer: PeerId,
    pub entry_peer: PeerId,
    pub exit_peer: PeerId,
    pub route: RouteBinding,
}

pub struct ForwardingContext {
    forwarding_id: SessionId,
    client_session_id: SessionId,
    client_peer: PeerId,
    entry_peer: PeerId,
    exit_peer: PeerId,
    route: RouteBinding,
    state: ForwardingState,
    relay_session_id: Option<SessionId>,
    last_request: Option<(SessionId, ForwardingDirection)>,
    last_sequence: Option<u64>,
}

impl ForwardingContext {
    pub fn new(
        client_session_id: SessionId,
        client_peer: PeerId,
        entry_peer: PeerId,
        exit_peer: PeerId,
        route: RouteBinding,
    ) -> Result<Self, ForwardingError> {
        Self::with_forwarding_id(
            SessionId::generate(),
            client_session_id,
            client_peer,
            entry_peer,
            exit_peer,
            route,
        )
    }

    pub fn with_forwarding_id(
        forwarding_id: SessionId,
        client_session_id: SessionId,
        client_peer: PeerId,
        entry_peer: PeerId,
        exit_peer: PeerId,
        route: RouteBinding,
    ) -> Result<Self, ForwardingError> {
        match route {
            RouteBinding::TwoHop { entry, exit }
                if entry == entry_peer && exit == exit_peer && entry != exit => {}
            _ => return Err(ForwardingError::InvalidRoute),
        }
        if client_peer == entry_peer || client_peer == exit_peer {
            return Err(ForwardingError::InvalidRoute);
        }
        Ok(Self {
            forwarding_id,
            client_session_id,
            client_peer,
            entry_peer,
            exit_peer,
            route,
            state: ForwardingState::Created,
            relay_session_id: None,
            last_request: None,
            last_sequence: None,
        })
    }

    pub fn forwarding_id(&self) -> SessionId {
        self.forwarding_id
    }

    pub fn client_session_id(&self) -> SessionId {
        self.client_session_id
    }

    pub fn client_peer(&self) -> PeerId {
        self.client_peer
    }

    pub fn entry_peer(&self) -> PeerId {
        self.entry_peer
    }

    pub fn exit_peer(&self) -> PeerId {
        self.exit_peer
    }

    pub fn route(&self) -> &RouteBinding {
        &self.route
    }

    pub fn state(&self) -> ForwardingState {
        self.state
    }

    pub fn bind_relay_session(&mut self, session_id: SessionId) -> Result<(), ForwardingError> {
        if self.state != ForwardingState::Established {
            return Err(ForwardingError::InvalidState {
                state: self.state,
                operation: "bind relay session",
            });
        }
        self.relay_session_id = Some(session_id);
        Ok(())
    }

    pub fn transition_connecting(&mut self) -> Result<(), ForwardingError> {
        self.transition(
            ForwardingState::Created,
            ForwardingState::Connecting,
            "connect",
        )
    }

    pub fn transition_established(&mut self) -> Result<(), ForwardingError> {
        self.transition(
            ForwardingState::Connecting,
            ForwardingState::Established,
            "establish",
        )
    }

    pub fn begin_forwarding(
        &mut self,
        request_id: SessionId,
        direction: ForwardingDirection,
    ) -> Result<(), ForwardingError> {
        if matches!(
            self.state,
            ForwardingState::Closing | ForwardingState::Closed
        ) {
            return Err(ForwardingError::ClosedContext);
        }
        if !matches!(
            self.state,
            ForwardingState::Established | ForwardingState::Forwarding
        ) {
            return Err(ForwardingError::InvalidState {
                state: self.state,
                operation: "forward",
            });
        }
        if self.last_request == Some((request_id, direction)) {
            return Err(ForwardingError::DuplicateForwardingRequest);
        }
        self.last_request = Some((request_id, direction));
        self.state = ForwardingState::Forwarding;
        Ok(())
    }

    pub fn transition_closing(&mut self) -> Result<(), ForwardingError> {
        if matches!(
            self.state,
            ForwardingState::Created | ForwardingState::Closed
        ) {
            return Err(ForwardingError::InvalidState {
                state: self.state,
                operation: "close",
            });
        }
        self.state = ForwardingState::Closing;
        Ok(())
    }

    pub fn transition_closed(&mut self) -> Result<(), ForwardingError> {
        if self.state != ForwardingState::Closing {
            return Err(ForwardingError::InvalidState {
                state: self.state,
                operation: "finish close",
            });
        }
        self.state = ForwardingState::Closed;
        Ok(())
    }

    pub fn validate_client(
        &self,
        session_id: SessionId,
        peer: PeerId,
    ) -> Result<(), ForwardingError> {
        if session_id != self.client_session_id {
            return Err(ForwardingError::WrongSessionBinding);
        }
        if peer != self.client_peer {
            return Err(ForwardingError::WrongEntryRelay);
        }
        Ok(())
    }

    pub fn validate_entry(&self, peer: PeerId) -> Result<(), ForwardingError> {
        if peer != self.entry_peer {
            return Err(ForwardingError::WrongEntryRelay);
        }
        Ok(())
    }

    pub fn validate_exit(&self, peer: PeerId) -> Result<(), ForwardingError> {
        if peer != self.exit_peer {
            return Err(ForwardingError::WrongExitRelay);
        }
        Ok(())
    }

    pub fn validate_message(&self, message: &ForwardingMessage) -> Result<(), ForwardingError> {
        if message.kind != FORWARDING_KIND {
            return Err(ForwardingError::MalformedForwardingMessage);
        }
        if message.forwarding_id != self.forwarding_id
            || message.client_session_id != self.client_session_id
            || message.client_peer.parse::<PeerId>().ok() != Some(self.client_peer)
            || message.route != self.route
        {
            return Err(ForwardingError::InvalidForwardingContext);
        }
        if message.data.kind != ghost_layer_network::DATA_PLANE_KIND {
            return Err(ForwardingError::UnexpectedProtocolMessage);
        }
        let packet = message
            .packet
            .as_ref()
            .ok_or(ForwardingError::MalformedForwardingMessage)?;
        self.validate_packet(packet)?;
        Ok(())
    }

    pub fn validate_packet(&self, packet: &ForwardedPacket) -> Result<(), ForwardingError> {
        if packet.session_id != self.client_session_id
            || packet.route_id != self.forwarding_id
            || packet.flow_id != self.forwarding_id
            || packet.payload.len() > ghost_layer_network::DEFAULT_MAXIMUM_PAYLOAD_SIZE
        {
            return Err(ForwardingError::InvalidForwardingContext);
        }
        if packet.sequence == 0 {
            return Err(ForwardingError::SequenceViolation);
        }
        if packet.destination.port() == 0 {
            return Err(ForwardingError::MalformedForwardingMessage);
        }
        if matches!(packet.protocol, ForwardedProtocol::Udp) && packet.source.is_none() {
            return Err(ForwardingError::MalformedForwardingMessage);
        }
        Ok(())
    }

    pub fn accept_sequence(&mut self, sequence: u64) -> Result<(), ForwardingError> {
        if sequence == 0 || self.last_sequence.is_some_and(|last| sequence <= last) {
            return Err(ForwardingError::SequenceViolation);
        }
        self.last_sequence = Some(sequence);
        Ok(())
    }

    pub fn validate_relay_session(&self, session_id: SessionId) -> Result<(), ForwardingError> {
        if self.relay_session_id != Some(session_id) {
            return Err(ForwardingError::WrongSessionBinding);
        }
        Ok(())
    }

    fn transition(
        &mut self,
        expected: ForwardingState,
        next: ForwardingState,
        operation: &'static str,
    ) -> Result<(), ForwardingError> {
        if self.state != expected {
            return Err(ForwardingError::InvalidState {
                state: self.state,
                operation,
            });
        }
        self.state = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> ForwardingContext {
        let client = PeerId::random();
        let entry = PeerId::random();
        let exit = PeerId::random();
        ForwardingContext::new(
            SessionId::generate(),
            client,
            entry,
            exit,
            RouteBinding::TwoHop { entry, exit },
        )
        .unwrap()
    }

    #[test]
    fn context_enforces_route_and_state_transitions() {
        let mut context = context();
        assert_eq!(context.state(), ForwardingState::Created);
        assert!(matches!(
            context.transition_established(),
            Err(ForwardingError::InvalidState { .. })
        ));
        context.transition_connecting().unwrap();
        context.transition_established().unwrap();
        let request = SessionId::generate();
        context
            .begin_forwarding(request, ForwardingDirection::ClientToExit)
            .unwrap();
        assert_eq!(
            context.begin_forwarding(request, ForwardingDirection::ClientToExit),
            Err(ForwardingError::DuplicateForwardingRequest)
        );
        context
            .begin_forwarding(request, ForwardingDirection::ExitToClient)
            .unwrap();
        context.transition_closing().unwrap();
        context.transition_closed().unwrap();
        assert_eq!(
            context.begin_forwarding(SessionId::generate(), ForwardingDirection::ClientToExit),
            Err(ForwardingError::ClosedContext)
        );
    }

    #[test]
    fn context_rejects_wrong_peers_sessions_and_routes() {
        let client = PeerId::random();
        let entry = PeerId::random();
        let exit = PeerId::random();
        let session = SessionId::generate();
        assert!(matches!(
            ForwardingContext::new(
                session,
                client,
                entry,
                exit,
                RouteBinding::TwoHop {
                    entry,
                    exit: PeerId::random(),
                },
            ),
            Err(ForwardingError::InvalidRoute)
        ));
        let context = context();
        assert_eq!(
            context.validate_client(SessionId::generate(), context.entry_peer()),
            Err(ForwardingError::WrongSessionBinding)
        );
        assert_eq!(
            context.validate_exit(PeerId::random()),
            Err(ForwardingError::WrongExitRelay)
        );
    }

    #[test]
    fn forwarding_message_requires_context_and_data_plane_kind() {
        let context = context();
        let packet = ForwardedPacket {
            protocol: ForwardedProtocol::Tcp,
            source: None,
            destination: "192.168.1.3:9005".parse().unwrap(),
            flow_id: context.forwarding_id(),
            session_id: context.client_session_id(),
            route_id: context.forwarding_id(),
            sequence: 1,
            payload: b"hello".to_vec(),
        };
        let message = ForwardingMessage {
            kind: FORWARDING_KIND.to_owned(),
            forwarding_id: context.forwarding_id(),
            client_session_id: context.client_session_id(),
            client_peer: context.client_peer().to_string(),
            route: context.route().clone(),
            data: DataPlaneEnvelope {
                kind: ghost_layer_network::DATA_PLANE_KIND.to_owned(),
                session_id: SessionId::generate(),
                frame: vec![1, 2, 3],
            },
            packet: Some(packet),
        };
        assert!(context.validate_message(&message).is_ok());
        assert_eq!(
            context.validate_exit(context.entry_peer()),
            Err(ForwardingError::WrongExitRelay)
        );
        let mut malformed = message;
        malformed.kind = "other".to_owned();
        assert_eq!(
            context.validate_message(&malformed),
            Err(ForwardingError::MalformedForwardingMessage)
        );
    }

    #[test]
    fn protocol_aware_packet_round_trips_and_binding_is_checked() {
        let context = context();
        let packet = ForwardedPacket {
            protocol: ForwardedProtocol::Tcp,
            source: None,
            destination: "192.168.1.3:9005".parse().unwrap(),
            flow_id: context.forwarding_id(),
            session_id: context.client_session_id(),
            route_id: context.forwarding_id(),
            sequence: 1,
            payload: b"ghost-layer-external-test".to_vec(),
        };
        let encoded = serde_json::to_vec(&packet).unwrap();
        let decoded: ForwardedPacket = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded, packet);
        let message = ForwardingMessage {
            kind: FORWARDING_KIND.to_owned(),
            forwarding_id: context.forwarding_id(),
            client_session_id: context.client_session_id(),
            client_peer: context.client_peer().to_string(),
            route: context.route().clone(),
            data: DataPlaneEnvelope {
                kind: ghost_layer_network::DATA_PLANE_KIND.to_owned(),
                session_id: SessionId::generate(),
                frame: vec![1],
            },
            packet: Some(packet),
        };
        assert!(context.validate_message(&message).is_ok());
        let mut wrong = message;
        wrong.packet.as_mut().unwrap().session_id = SessionId::generate();
        assert_eq!(
            context.validate_message(&wrong),
            Err(ForwardingError::InvalidForwardingContext)
        );
    }
}
