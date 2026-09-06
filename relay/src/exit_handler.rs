use crate::{ExitNetworkAdapter, ExitNetworkError};
use crate::{ForwardingContext, ForwardingError, ForwardingState};
use ghost_layer_network::{NetworkPacket, PacketError, PeerId, SessionId};
use std::collections::HashSet;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitPacketHandlerError {
    Closed,
    QueueFull,
    InvalidPacket(PacketError),
    InvalidSession,
    InvalidRoute,
    WrongEntry,
    WrongExit,
    InvalidForwardingState,
    DuplicatePacket,
    Shutdown,
    Network(ExitNetworkError),
}

impl fmt::Display for ExitPacketHandlerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for ExitPacketHandlerError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitForwardingBinding {
    pub relay_session_id: SessionId,
    pub entry_peer: PeerId,
    pub exit_peer: PeerId,
}

pub struct ExitPacketHandler {
    maximum_packet_size: usize,
    mtu: usize,
    maximum_pending_responses: usize,
    pending_responses: Vec<NetworkPacket>,
    handled_packets: HashSet<SessionId>,
    closed: bool,
}

impl ExitPacketHandler {
    pub fn open(
        maximum_packet_size: usize,
        mtu: usize,
        maximum_pending_responses: usize,
    ) -> Result<Self, ExitPacketHandlerError> {
        if maximum_packet_size == 0 || mtu == 0 || maximum_pending_responses == 0 {
            return Err(ExitPacketHandlerError::InvalidPacket(
                PacketError::InvalidLimit,
            ));
        }
        Ok(Self {
            maximum_packet_size,
            mtu,
            maximum_pending_responses,
            pending_responses: Vec::new(),
            handled_packets: HashSet::new(),
            closed: false,
        })
    }

    pub fn handle(
        &mut self,
        packet: NetworkPacket,
    ) -> Result<NetworkPacket, ExitPacketHandlerError> {
        self.ensure_open()?;
        let packet = self.validate_packet(packet)?;
        self.enqueue_response(packet.clone())?;
        self.pending_responses
            .pop()
            .ok_or(ExitPacketHandlerError::Shutdown)
    }

    pub fn handle_bound(
        &mut self,
        packet_id: SessionId,
        packet: NetworkPacket,
        context: &ForwardingContext,
        relay_session_id: SessionId,
        entry_peer: PeerId,
        exit_peer: PeerId,
    ) -> Result<NetworkPacket, ExitPacketHandlerError> {
        self.ensure_open()?;
        if context.state() != ForwardingState::Forwarding {
            return Err(ExitPacketHandlerError::InvalidForwardingState);
        }
        context
            .validate_entry(entry_peer)
            .map_err(map_context_error)?;
        context
            .validate_exit(exit_peer)
            .map_err(map_context_error)?;
        context
            .validate_relay_session(relay_session_id)
            .map_err(map_context_error)?;
        if !self.handled_packets.insert(packet_id) {
            return Err(ExitPacketHandlerError::DuplicatePacket);
        }
        self.handle(packet)
    }

    pub fn handle_bound_with_adapter<A: ExitNetworkAdapter>(
        &mut self,
        packet_id: SessionId,
        packet: NetworkPacket,
        context: &ForwardingContext,
        binding: ExitForwardingBinding,
        adapter: &mut A,
    ) -> Result<NetworkPacket, ExitPacketHandlerError> {
        self.ensure_open()?;
        if context.state() != ForwardingState::Forwarding {
            return Err(ExitPacketHandlerError::InvalidForwardingState);
        }
        context
            .validate_entry(binding.entry_peer)
            .map_err(map_context_error)?;
        context
            .validate_exit(binding.exit_peer)
            .map_err(map_context_error)?;
        context
            .validate_relay_session(binding.relay_session_id)
            .map_err(map_context_error)?;
        if !self.handled_packets.insert(packet_id) {
            return Err(ExitPacketHandlerError::DuplicatePacket);
        }
        let response = adapter
            .exchange(packet.as_bytes())
            .map_err(ExitPacketHandlerError::Network)?;
        let response = NetworkPacket::new(response, self.maximum_packet_size, self.mtu)
            .map_err(ExitPacketHandlerError::InvalidPacket)?;
        self.enqueue_response(response.clone())?;
        self.pending_responses
            .pop()
            .ok_or(ExitPacketHandlerError::Shutdown)
    }

    pub fn handle_bound_with_adapter_payload<A: ExitNetworkAdapter>(
        &mut self,
        packet_id: SessionId,
        payload: &[u8],
        context: &ForwardingContext,
        binding: ExitForwardingBinding,
        adapter: &mut A,
    ) -> Result<Vec<u8>, ExitPacketHandlerError> {
        self.ensure_open()?;
        if context.state() != ForwardingState::Forwarding {
            return Err(ExitPacketHandlerError::InvalidForwardingState);
        }
        context
            .validate_entry(binding.entry_peer)
            .map_err(map_context_error)?;
        context
            .validate_exit(binding.exit_peer)
            .map_err(map_context_error)?;
        context
            .validate_relay_session(binding.relay_session_id)
            .map_err(map_context_error)?;
        if !self.handled_packets.insert(packet_id) {
            return Err(ExitPacketHandlerError::DuplicatePacket);
        }
        adapter
            .exchange(payload)
            .map_err(ExitPacketHandlerError::Network)
    }

    pub fn enqueue_response(
        &mut self,
        packet: NetworkPacket,
    ) -> Result<(), ExitPacketHandlerError> {
        self.ensure_open()?;
        let packet = self.validate_packet(packet)?;
        if self.pending_responses.len() >= self.maximum_pending_responses {
            return Err(ExitPacketHandlerError::QueueFull);
        }
        self.pending_responses.push(packet);
        Ok(())
    }

    pub fn take_response(&mut self) -> Result<Option<NetworkPacket>, ExitPacketHandlerError> {
        self.ensure_open()?;
        Ok(self.pending_responses.pop())
    }

    pub fn close(&mut self) -> Result<(), ExitPacketHandlerError> {
        if self.closed {
            return Err(ExitPacketHandlerError::Closed);
        }
        self.closed = true;
        self.pending_responses.clear();
        self.handled_packets.clear();
        Ok(())
    }

    fn validate_packet(
        &self,
        packet: NetworkPacket,
    ) -> Result<NetworkPacket, ExitPacketHandlerError> {
        NetworkPacket::new(
            packet.as_bytes().to_vec(),
            self.maximum_packet_size,
            self.mtu,
        )
        .map_err(ExitPacketHandlerError::InvalidPacket)
    }

    fn ensure_open(&self) -> Result<(), ExitPacketHandlerError> {
        if self.closed {
            Err(ExitPacketHandlerError::Closed)
        } else {
            Ok(())
        }
    }
}

fn map_context_error(error: ForwardingError) -> ExitPacketHandlerError {
    match error {
        ForwardingError::WrongEntryRelay => ExitPacketHandlerError::WrongEntry,
        ForwardingError::WrongExitRelay => ExitPacketHandlerError::WrongExit,
        ForwardingError::WrongSessionBinding => ExitPacketHandlerError::InvalidSession,
        ForwardingError::InvalidRoute => ExitPacketHandlerError::InvalidRoute,
        _ => ExitPacketHandlerError::InvalidSession,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ghost_layer_network::RouteBinding;

    fn packet() -> NetworkPacket {
        let mut bytes = vec![0u8; 20];
        bytes[0] = 0x45;
        NetworkPacket::new(bytes, 1500, 1500).unwrap()
    }

    fn context() -> (ForwardingContext, SessionId, PeerId, PeerId) {
        let client = PeerId::random();
        let entry = PeerId::random();
        let exit = PeerId::random();
        let relay_session = SessionId::generate();
        let mut context = ForwardingContext::new(
            SessionId::generate(),
            client,
            entry,
            exit,
            RouteBinding::TwoHop { entry, exit },
        )
        .unwrap();
        context.transition_connecting().unwrap();
        context.transition_established().unwrap();
        context.bind_relay_session(relay_session).unwrap();
        context
            .begin_forwarding(
                SessionId::generate(),
                crate::ForwardingDirection::ClientToExit,
            )
            .unwrap();
        (context, relay_session, entry, exit)
    }

    #[test]
    fn handler_echoes_valid_packets_deterministically() {
        let mut handler = ExitPacketHandler::open(1500, 1500, 1).unwrap();
        let input = packet();
        let response = handler.handle(input.clone()).unwrap();
        assert_eq!(response, input);
    }

    #[test]
    fn handler_rejects_queue_closed_and_bound_failures() {
        let mut handler = ExitPacketHandler::open(1500, 1500, 1).unwrap();
        handler.enqueue_response(packet()).unwrap();
        assert_eq!(
            handler.enqueue_response(packet()),
            Err(ExitPacketHandlerError::QueueFull)
        );
        let (context, relay_session, _, exit) = context();
        assert_eq!(
            handler.handle_bound(
                SessionId::generate(),
                packet(),
                &context,
                relay_session,
                PeerId::random(),
                exit
            ),
            Err(ExitPacketHandlerError::WrongEntry)
        );
        handler.close().unwrap();
        assert_eq!(
            handler.handle(packet()),
            Err(ExitPacketHandlerError::Closed)
        );
    }

    #[test]
    fn handler_rejects_invalid_packet_and_duplicate_id() {
        let mut handler = ExitPacketHandler::open(1500, 1500, 1).unwrap();
        let (context, relay_session, entry, exit) = context();
        let packet_id = SessionId::generate();
        let first = handler
            .handle_bound(packet_id, packet(), &context, relay_session, entry, exit)
            .unwrap();
        assert_eq!(first.len(), 20);
        assert_eq!(
            handler.handle_bound(packet_id, packet(), &context, relay_session, entry, exit),
            Err(ExitPacketHandlerError::DuplicatePacket)
        );
        let invalid = NetworkPacket::new(vec![0x70; 20], 1500, 1500).unwrap_err();
        assert_eq!(invalid, PacketError::UnsupportedType { version: 7 });
    }
}
