use ghost_layer_network::{PeerId, RouteBinding, SessionId};
use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct SessionBinding {
    pub session_id: SessionId,
    pub authenticated_peer: PeerId,
    pub route: RouteBinding,
    pub entry_peer: PeerId,
    pub exit_peer: PeerId,
    pub created_at: Instant,
    pub last_activity: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionManagerError {
    Limit,
    Duplicate,
    Unknown,
}

pub struct SessionManager {
    sessions: HashMap<SessionId, SessionBinding>,
    maximum_sessions: usize,
    timeout: Duration,
}

impl SessionManager {
    pub fn new(maximum_sessions: usize, timeout: Duration) -> Result<Self, SessionManagerError> {
        if maximum_sessions == 0 || timeout.is_zero() {
            return Err(SessionManagerError::Limit);
        }
        Ok(Self {
            sessions: HashMap::new(),
            maximum_sessions,
            timeout,
        })
    }
    pub fn insert(&mut self, binding: SessionBinding) -> Result<(), SessionManagerError> {
        self.cleanup();
        if self.sessions.contains_key(&binding.session_id) {
            return Err(SessionManagerError::Duplicate);
        }
        if self.sessions.len() >= self.maximum_sessions {
            return Err(SessionManagerError::Limit);
        }
        self.sessions.insert(binding.session_id, binding);
        Ok(())
    }
    pub fn get(&self, session_id: &SessionId) -> Option<&SessionBinding> {
        self.sessions.get(session_id)
    }
    pub fn remove(&mut self, session_id: &SessionId) -> Result<(), SessionManagerError> {
        self.sessions
            .remove(session_id)
            .map(|_| ())
            .ok_or(SessionManagerError::Unknown)
    }
    pub fn cleanup(&mut self) {
        let timeout = self.timeout;
        let now = Instant::now();
        self.sessions
            .retain(|_, session| now.duration_since(session.last_activity) <= timeout);
    }
    pub fn len(&self) -> usize {
        self.sessions.len()
    }
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn session_manager_enforces_limit_duplicate_and_lookup() {
        let mut manager = SessionManager::new(1, Duration::from_secs(1)).unwrap();
        let entry = PeerId::random();
        let exit = PeerId::random();
        let binding = SessionBinding {
            session_id: SessionId::generate(),
            authenticated_peer: PeerId::random(),
            route: RouteBinding::TwoHop { entry, exit },
            entry_peer: entry,
            exit_peer: exit,
            created_at: Instant::now(),
            last_activity: Instant::now(),
        };
        let id = binding.session_id;
        manager.insert(binding.clone()).unwrap();
        assert_eq!(
            manager.insert(binding.clone()),
            Err(SessionManagerError::Duplicate)
        );
        assert!(manager.get(&id).is_some());
        assert_eq!(
            manager.insert(SessionBinding {
                session_id: SessionId::generate(),
                ..binding
            }),
            Err(SessionManagerError::Limit)
        );
    }
}
