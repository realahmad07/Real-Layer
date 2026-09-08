use crate::{ForwardedPacket, ForwardingContext, ForwardingError, ForwardingMessage};
use ghost_layer_network::SessionId;
use std::collections::HashMap;

pub struct ForwardingManager {
    contexts: HashMap<SessionId, ForwardingContext>,
    maximum_flows: usize,
}

impl ForwardingManager {
    pub fn new(maximum_flows: usize) -> Result<Self, ForwardingError> {
        if maximum_flows == 0 {
            return Err(ForwardingError::Backpressure);
        }
        Ok(Self {
            contexts: HashMap::new(),
            maximum_flows,
        })
    }
    pub fn admit(&mut self, context: ForwardingContext) -> Result<(), ForwardingError> {
        if self.contexts.len() >= self.maximum_flows {
            return Err(ForwardingError::Backpressure);
        }
        self.contexts
            .entry(context.forwarding_id())
            .or_insert(context);
        Ok(())
    }
    pub fn validate(
        &self,
        message: &ForwardingMessage,
    ) -> Result<&ForwardingContext, ForwardingError> {
        let context = self
            .contexts
            .get(&message.forwarding_id)
            .ok_or(ForwardingError::InvalidForwardingContext)?;
        context.validate_message(message)?;
        Ok(context)
    }
    pub fn validate_packet(&self, packet: &ForwardedPacket) -> Result<(), ForwardingError> {
        self.contexts
            .get(&packet.flow_id)
            .ok_or(ForwardingError::InvalidForwardingContext)?
            .validate_packet(packet)
    }
    pub fn remove(&mut self, forwarding_id: &SessionId) {
        self.contexts.remove(forwarding_id);
    }
    pub fn len(&self) -> usize {
        self.contexts.len()
    }
    pub fn is_empty(&self) -> bool {
        self.contexts.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ghost_layer_network::{PeerId, RouteBinding, SessionId};
    #[test]
    fn forwarding_manager_limits_and_cleans_contexts() {
        let client = PeerId::random();
        let entry = PeerId::random();
        let exit = PeerId::random();
        let context = ForwardingContext::new(
            SessionId::generate(),
            client,
            entry,
            exit,
            RouteBinding::TwoHop { entry, exit },
        )
        .unwrap();
        let id = context.forwarding_id();
        let mut manager = ForwardingManager::new(1).unwrap();
        manager.admit(context).unwrap();
        assert_eq!(manager.len(), 1);
        manager.remove(&id);
        assert_eq!(manager.len(), 0);
    }
}
