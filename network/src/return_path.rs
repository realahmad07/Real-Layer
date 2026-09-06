use crate::SessionId;
use std::collections::HashMap;
use std::fmt;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReturnPathKey {
    pub session_id: SessionId,
    pub packet_id: SessionId,
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
    InvalidLifetime,
}
impl fmt::Display for ReturnPathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for ReturnPathError {}

pub struct ReturnPathTable {
    entries: HashMap<SessionId, ReturnPathMapping>,
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
            capacity,
            lifetime,
        })
    }
    pub fn insert(&mut self, key: ReturnPathKey) -> Result<(), ReturnPathError> {
        self.cleanup();
        if self.entries.contains_key(&key.packet_id) {
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
    pub fn lookup(&mut self, packet_id: SessionId) -> Result<ReturnPathMapping, ReturnPathError> {
        self.cleanup();
        self.entries
            .get(&packet_id)
            .copied()
            .ok_or(ReturnPathError::Unknown)
    }
    pub fn cleanup(&mut self) {
        let now = Instant::now();
        self.entries.retain(|_, value| value.expires_at > now);
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

    #[test]
    fn return_path_mapping_is_bounded_and_expires() {
        let mut table = ReturnPathTable::new(1, Duration::from_millis(20)).unwrap();
        let key = ReturnPathKey {
            session_id: SessionId::generate(),
            packet_id: SessionId::generate(),
        };
        table.insert(key).unwrap();
        assert_eq!(table.lookup(key.packet_id).unwrap().key, key);
        assert_eq!(table.insert(key), Err(ReturnPathError::Duplicate));
        assert_eq!(
            table.insert(ReturnPathKey {
                session_id: SessionId::generate(),
                packet_id: SessionId::generate()
            }),
            Err(ReturnPathError::Capacity)
        );
        std::thread::sleep(Duration::from_millis(50));
        table.cleanup();
        assert_eq!(table.len(), 0);
        assert_eq!(table.lookup(key.packet_id), Err(ReturnPathError::Unknown));
    }
}
