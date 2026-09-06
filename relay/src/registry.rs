use crate::RelayMetadata;
use ghost_layer_network::PeerId;
use std::collections::HashMap;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    AlreadyRegistered,
    NotFound,
}

impl fmt::Display for RegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyRegistered => formatter.write_str("relay is already registered"),
            Self::NotFound => formatter.write_str("relay is not registered"),
        }
    }
}

impl std::error::Error for RegistryError {}

pub trait RelayRegistry {
    fn register(&mut self, metadata: RelayMetadata) -> Result<(), RegistryError>;
    fn update(&mut self, metadata: RelayMetadata) -> Result<(), RegistryError>;
    fn get(&self, peer_id: &PeerId) -> Option<&RelayMetadata>;
    fn remove(&mut self, peer_id: &PeerId) -> Result<RelayMetadata, RegistryError>;
}

#[derive(Debug, Default)]
pub struct InMemoryRelayRegistry {
    relays: HashMap<PeerId, RelayMetadata>,
}

impl RelayRegistry for InMemoryRelayRegistry {
    fn register(&mut self, metadata: RelayMetadata) -> Result<(), RegistryError> {
        if self.relays.contains_key(&metadata.peer_id) {
            return Err(RegistryError::AlreadyRegistered);
        }
        self.relays.insert(metadata.peer_id, metadata);
        Ok(())
    }

    fn update(&mut self, metadata: RelayMetadata) -> Result<(), RegistryError> {
        if !self.relays.contains_key(&metadata.peer_id) {
            return Err(RegistryError::NotFound);
        }
        self.relays.insert(metadata.peer_id, metadata);
        Ok(())
    }

    fn get(&self, peer_id: &PeerId) -> Option<&RelayMetadata> {
        self.relays.get(peer_id)
    }

    fn remove(&mut self, peer_id: &PeerId) -> Result<RelayMetadata, RegistryError> {
        self.relays.remove(peer_id).ok_or(RegistryError::NotFound)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_memory_registry_supports_lifecycle() {
        let peer_id = ghost_layer_network::PeerId::random();
        let metadata = RelayMetadata::new(peer_id, "1.0".to_owned(), "test".to_owned());
        let mut registry = InMemoryRelayRegistry::default();
        registry.register(metadata.clone()).expect("register");
        assert_eq!(registry.get(&peer_id), Some(&metadata));
        assert_eq!(
            registry.register(metadata.clone()),
            Err(RegistryError::AlreadyRegistered)
        );
        registry
            .update(RelayMetadata {
                software_version: "updated".to_owned(),
                ..metadata
            })
            .expect("update");
        assert_eq!(
            registry.remove(&peer_id).expect("remove").software_version,
            "updated"
        );
    }
}
