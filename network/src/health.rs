#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthStatus {
    Unknown,
    Healthy,
    Degraded,
    Unavailable,
}

impl HealthStatus {
    pub fn is_available(self) -> bool {
        matches!(self, Self::Healthy | Self::Degraded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_healthy_and_degraded_states_are_available() {
        assert!(HealthStatus::Healthy.is_available());
        assert!(HealthStatus::Degraded.is_available());
        assert!(!HealthStatus::Unknown.is_available());
        assert!(!HealthStatus::Unavailable.is_available());
    }
}
