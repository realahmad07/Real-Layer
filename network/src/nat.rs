use crate::SessionId;
use std::collections::HashMap;
use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FlowKey {
    pub source: SocketAddr,
    pub destination: SocketAddr,
    pub session_id: SessionId,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NatMapping {
    pub source: SocketAddr,
    pub translated: SocketAddr,
    pub session_id: SessionId,
    pub expires_at: Instant,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NatError {
    Capacity,
    Duplicate,
    UnknownMapping,
    InvalidLifetime,
}
impl fmt::Display for NatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
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
        let translated = SocketAddr::new(self.translated_address, self.next_port);
        self.next_port = self.next_port.checked_add(1).unwrap_or(40000);
        let mapping = NatMapping {
            source: key.source,
            translated,
            session_id: key.session_id,
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
    pub fn lookup_reverse(&mut self, translated: SocketAddr) -> Result<FlowKey, NatError> {
        self.cleanup();
        self.reverse
            .get(&translated)
            .copied()
            .ok_or(NatError::UnknownMapping)
    }
    pub fn cleanup(&mut self) {
        let expired: Vec<_> = self
            .mappings
            .iter()
            .filter(|(_, value)| value.expires_at <= Instant::now())
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    fn key(port: u16) -> FlowKey {
        FlowKey {
            source: SocketAddr::from(([10, 0, 0, 1], port)),
            destination: SocketAddr::from(([198, 51, 100, 1], 443)),
            session_id: SessionId::generate(),
        }
    }
    #[test]
    fn mapping_lifecycle_and_capacity() {
        let mut table = NatTable::new(
            IpAddr::V4(std::net::Ipv4Addr::new(192, 0, 2, 1)),
            1,
            Duration::from_millis(20),
        )
        .unwrap();
        let first = key(1000);
        let mapping = table.create(first).unwrap();
        assert_eq!(table.lookup_forward(first), Some(mapping));
        assert_eq!(table.lookup_reverse(mapping.translated), Ok(first));
        assert_eq!(table.create(first), Err(NatError::Duplicate));
        assert_eq!(table.create(key(1001)), Err(NatError::Capacity));
        thread::sleep(Duration::from_millis(50));
        table.cleanup();
        assert_eq!(table.len(), 0);
        assert_eq!(
            table.lookup_reverse(mapping.translated),
            Err(NatError::UnknownMapping)
        );
    }
}
