use crate::{PeerId, SessionId};
use std::collections::HashMap;
use std::fmt;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReturnPathKey {
    pub session_id: SessionId,
    pub packet_id: SessionId,
    pub source_identity: PeerId,
    pub entry_peer: PeerId,
    pub exit_peer: PeerId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReturnPathMapping {
    pub key: ReturnPathKey,
    pub expires_at: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReturnPathError {
    Capacity,
    Duplicate,
    Unknown,
    BindingMismatch,
    Replayed,
    InvalidLifetime,
}

impl fmt::Display for ReturnPathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for ReturnPathError {}

pub struct ReturnPathTable {
    entries: HashMap<SessionId, ReturnPathMapping>,
    consumed: HashMap<SessionId, Instant>,
    capacity: usize,
    lifetime: Duration,
}

impl ReturnPathTable {
    pub fn new(capacity: usize, lifetime: Duration) -> Result<Self, ReturnPathError> {
        if capacity == 0 || lifetime.is_zero() {
            return Err(ReturnPathError::InvalidLifetime);
        }
        Ok(Self {
            entries: HashMap::new(),
            consumed: HashMap::new(),
            capacity,
            lifetime,
        })
    }

    pub fn insert(&mut self, key: ReturnPathKey) -> Result<(), ReturnPathError> {
        self.cleanup();
        if self.entries.contains_key(&key.packet_id) || self.consumed.contains_key(&key.packet_id) {
            return Err(ReturnPathError::Duplicate);
        }
        if self.entries.len() >= self.capacity {
            return Err(ReturnPathError::Capacity);
        }
        self.entries.insert(
            key.packet_id,
            ReturnPathMapping {
                key,
                expires_at: Instant::now() + self.lifetime,
            },
        );
        Ok(())
    }

    pub fn lookup(
        &mut self,
        packet_id: SessionId,
        expected: ReturnPathKey,
    ) -> Result<ReturnPathMapping, ReturnPathError> {
        self.cleanup();
        let mapping = self.entries.get(&packet_id).copied().ok_or_else(|| {
            if self.consumed.contains_key(&packet_id) {
                ReturnPathError::Replayed
            } else {
                ReturnPathError::Unknown
            }
        })?;
        if mapping.key != expected {
            return Err(ReturnPathError::BindingMismatch);
        }
        Ok(mapping)
    }

    pub fn consume(
        &mut self,
        packet_id: SessionId,
        expected: ReturnPathKey,
    ) -> Result<ReturnPathMapping, ReturnPathError> {
        let mapping = self.lookup(packet_id, expected)?;
        self.entries.remove(&packet_id);
        self.consumed
            .insert(packet_id, Instant::now() + self.lifetime);
        Ok(mapping)
    }

    pub fn remove_session(&mut self, session_id: SessionId) {
        self.entries
            .retain(|_, mapping| mapping.key.session_id != session_id);
    }

    pub fn cleanup(&mut self) {
        let now = Instant::now();
        self.entries.retain(|_, value| value.expires_at > now);
        self.consumed.retain(|_, expires_at| *expires_at > now);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    fn key(session_id: SessionId) -> ReturnPathKey {
        ReturnPathKey {
            session_id,
            packet_id: SessionId::generate(),
            source_identity: PeerId::random(),
            entry_peer: PeerId::random(),
            exit_peer: PeerId::random(),
        }
    }

    #[test]
    fn mapping_validates_bindings_and_rejects_replay() {
        let mut table = ReturnPathTable::new(2, Duration::from_secs(1)).unwrap();
        let original = key(SessionId::generate());
        table.insert(original).unwrap();
        let mut wrong = original;
        wrong.exit_peer = PeerId::random();
        assert_eq!(
            table.lookup(original.packet_id, wrong),
            Err(ReturnPathError::BindingMismatch)
        );
        assert_eq!(
            table.consume(original.packet_id, original).unwrap().key,
            original
        );
        assert_eq!(
            table.consume(original.packet_id, original),
            Err(ReturnPathError::Replayed)
        );
    }

    #[test]
    fn mapping_is_bounded_expires_and_cleans_by_session() {
        let mut table = ReturnPathTable::new(1, Duration::from_millis(20)).unwrap();
        let first = key(SessionId::generate());
        table.insert(first).unwrap();
        assert_eq!(
            table.insert(key(SessionId::generate())),
            Err(ReturnPathError::Capacity)
        );
        thread::sleep(Duration::from_millis(50));
        table.cleanup();
        assert!(table.is_empty());
        let second = key(SessionId::generate());
        table.insert(second).unwrap();
        table.remove_session(second.session_id);
        assert!(table.is_empty());
    }
}
