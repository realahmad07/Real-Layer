use crate::{PeerId, SessionId};
use std::collections::HashMap;
use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FlowKey {
    pub source: SocketAddr,
    pub destination: SocketAddr,
    pub session_id: SessionId,
    pub source_identity: PeerId,
    pub entry_peer: PeerId,
    pub exit_peer: PeerId,
    pub protocol: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NatMapping {
    pub source: SocketAddr,
    pub destination: SocketAddr,
    pub translated: SocketAddr,
    pub session_id: SessionId,
    pub source_identity: PeerId,
    pub entry_peer: PeerId,
    pub exit_peer: PeerId,
    pub protocol: u8,
    pub expires_at: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NatError {
    Capacity,
    Duplicate,
    UnknownMapping,
    BindingMismatch,
    PortExhausted,
    InvalidLifetime,
}

impl fmt::Display for NatError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for NatError {}

pub struct NatTable {
    mappings: HashMap<FlowKey, NatMapping>,
    reverse: HashMap<SocketAddr, FlowKey>,
    maximum_mappings: usize,
    lifetime: Duration,
    next_port: u16,
    translated_address: IpAddr,
}

impl NatTable {
    pub fn new(
        translated_address: IpAddr,
        maximum_mappings: usize,
        lifetime: Duration,
    ) -> Result<Self, NatError> {
        if maximum_mappings == 0 || lifetime.is_zero() {
            return Err(NatError::InvalidLifetime);
        }
        Ok(Self {
            mappings: HashMap::new(),
            reverse: HashMap::new(),
            maximum_mappings,
            lifetime,
            next_port: 40000,
            translated_address,
        })
    }

    pub fn create(&mut self, key: FlowKey) -> Result<NatMapping, NatError> {
        self.cleanup();
        if self.mappings.contains_key(&key) {
            return Err(NatError::Duplicate);
        }
        if self.mappings.len() >= self.maximum_mappings {
            return Err(NatError::Capacity);
        }
        let translated = self.allocate_port()?;
        let mapping = NatMapping {
            source: key.source,
            destination: key.destination,
            translated,
            session_id: key.session_id,
            source_identity: key.source_identity,
            entry_peer: key.entry_peer,
            exit_peer: key.exit_peer,
            protocol: key.protocol,
            expires_at: Instant::now() + self.lifetime,
        };
        self.mappings.insert(key, mapping);
        self.reverse.insert(translated, key);
        Ok(mapping)
    }

    pub fn lookup_forward(&mut self, key: FlowKey) -> Option<NatMapping> {
        self.cleanup();
        self.mappings.get(&key).copied()
    }

    pub fn lookup_reverse(
        &mut self,
        translated: SocketAddr,
        expected: FlowKey,
    ) -> Result<NatMapping, NatError> {
        self.cleanup();
        let key = self
            .reverse
            .get(&translated)
            .copied()
            .ok_or(NatError::UnknownMapping)?;
        if key != expected {
            return Err(NatError::BindingMismatch);
        }
        self.mappings
            .get(&key)
            .copied()
            .ok_or(NatError::UnknownMapping)
    }

    pub fn remove_session(&mut self, session_id: SessionId) {
        let keys: Vec<_> = self
            .mappings
            .keys()
            .filter(|key| key.session_id == session_id)
            .copied()
            .collect();
        for key in keys {
            if let Some(mapping) = self.mappings.remove(&key) {
                self.reverse.remove(&mapping.translated);
            }
        }
    }

    pub fn cleanup(&mut self) {
        let now = Instant::now();
        let expired: Vec<_> = self
            .mappings
            .iter()
            .filter(|(_, value)| value.expires_at <= now)
            .map(|(key, value)| (*key, value.translated))
            .collect();
        for (key, translated) in expired {
            self.mappings.remove(&key);
            self.reverse.remove(&translated);
        }
    }

    pub fn len(&self) -> usize {
        self.mappings.len()
    }

    pub fn is_empty(&self) -> bool {
        self.mappings.is_empty()
    }

    fn allocate_port(&mut self) -> Result<SocketAddr, NatError> {
        for _ in 0..=u16::MAX - 40000 {
            let port = self.next_port;
            self.next_port = if self.next_port == u16::MAX {
                40000
            } else {
                self.next_port + 1
            };
            let candidate = SocketAddr::new(self.translated_address, port);
            if !self.reverse.contains_key(&candidate) {
                return Ok(candidate);
            }
        }
        Err(NatError::PortExhausted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    fn key(session_id: SessionId, source_port: u16) -> FlowKey {
        FlowKey {
            source: SocketAddr::from(([10, 0, 0, 1], source_port)),
            destination: SocketAddr::from(([198, 51, 100, 1], 443)),
            session_id,
            source_identity: PeerId::random(),
            entry_peer: PeerId::random(),
            exit_peer: PeerId::random(),
            protocol: 6,
        }
    }

    #[test]
    fn mapping_binds_flow_and_rejects_wrong_return_context() {
        let mut table = NatTable::new(
            IpAddr::V4(std::net::Ipv4Addr::new(192, 0, 2, 1)),
            4,
            Duration::from_secs(1),
        )
        .unwrap();
        let first = key(SessionId::generate(), 1000);
        let mapping = table.create(first).unwrap();
        assert_eq!(table.lookup_forward(first), Some(mapping));
        assert_eq!(table.lookup_reverse(mapping.translated, first), Ok(mapping));
        let mut wrong = first;
        wrong.protocol = 17;
        assert_eq!(
            table.lookup_reverse(mapping.translated, wrong),
            Err(NatError::BindingMismatch)
        );
        assert_eq!(table.create(first), Err(NatError::Duplicate));
    }

    #[test]
    fn mapping_lifecycle_capacity_expiration_and_cleanup_work() {
        let session = SessionId::generate();
        let mut table = NatTable::new(
            IpAddr::V4(std::net::Ipv4Addr::new(192, 0, 2, 1)),
            1,
            Duration::from_millis(20),
        )
        .unwrap();
        let first = key(session, 1000);
        let mapping = table.create(first).unwrap();
        assert_eq!(
            table.create(key(SessionId::generate(), 1001)),
            Err(NatError::Capacity)
        );
        thread::sleep(Duration::from_millis(50));
        table.cleanup();
        assert!(table.is_empty());
        assert_eq!(
            table.lookup_reverse(mapping.translated, first),
            Err(NatError::UnknownMapping)
        );
        let replacement_key = key(session, 1002);
        let replacement = table.create(replacement_key).unwrap();
        table.remove_session(session);
        assert_eq!(table.lookup_forward(replacement_key), None);
        assert_eq!(
            table.lookup_reverse(replacement.translated, replacement_key),
            Err(NatError::UnknownMapping)
        );
    }
}
