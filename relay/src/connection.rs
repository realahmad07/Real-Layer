use ghost_layer_network::PeerId;
use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Connecting,
    Ready,
    Closing,
    Closed,
}

#[derive(Debug, Clone)]
pub struct ConnectionRecord {
    pub peer_id: PeerId,
    pub state: ConnectionState,
    pub created_at: Instant,
    pub last_activity: Instant,
    pub protocol_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionManagerError {
    Limit,
    Duplicate,
}

pub struct ConnectionManager {
    records: HashMap<PeerId, ConnectionRecord>,
    maximum_connections: usize,
}

impl ConnectionManager {
    pub fn new(maximum_connections: usize) -> Result<Self, ConnectionManagerError> {
        if maximum_connections == 0 {
            return Err(ConnectionManagerError::Limit);
        }
        Ok(Self {
            records: HashMap::new(),
            maximum_connections,
        })
    }
    pub fn establish(
        &mut self,
        peer_id: PeerId,
        protocol_version: String,
    ) -> Result<(), ConnectionManagerError> {
        if self.records.contains_key(&peer_id) {
            return Err(ConnectionManagerError::Duplicate);
        }
        if self.records.len() >= self.maximum_connections {
            return Err(ConnectionManagerError::Limit);
        }
        let now = Instant::now();
        self.records.insert(
            peer_id,
            ConnectionRecord {
                peer_id,
                state: ConnectionState::Ready,
                created_at: now,
                last_activity: now,
                protocol_version,
            },
        );
        Ok(())
    }
    pub fn touch(&mut self, peer_id: &PeerId) -> bool {
        self.records
            .get_mut(peer_id)
            .map(|record| {
                record.last_activity = Instant::now();
            })
            .is_some()
    }
    pub fn close(&mut self, peer_id: &PeerId) -> bool {
        self.records.remove(peer_id).is_some()
    }
    pub fn cleanup_idle(&mut self, idle_for: Duration) -> usize {
        let now = Instant::now();
        let before = self.records.len();
        self.records
            .retain(|_, record| now.duration_since(record.last_activity) <= idle_for);
        before - self.records.len()
    }
    pub fn len(&self) -> usize {
        self.records.len()
    }
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn connection_limit_duplicate_and_cleanup_are_deterministic() {
        let mut manager = ConnectionManager::new(1).unwrap();
        let peer = PeerId::random();
        manager.establish(peer, "1.0".into()).unwrap();
        assert_eq!(
            manager.establish(peer, "1.0".into()),
            Err(ConnectionManagerError::Duplicate)
        );
        assert_eq!(
            manager.establish(PeerId::random(), "1.0".into()),
            Err(ConnectionManagerError::Limit)
        );
        assert_eq!(manager.cleanup_idle(Duration::ZERO), 1);
    }
}
