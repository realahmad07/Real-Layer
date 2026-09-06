use std::collections::HashMap;
use std::fmt;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DnsError {
    Disabled,
    MalformedRequest,
    UnknownHost,
    RequestTooLarge,
    ResponseTooLarge,
    Timeout,
}
impl fmt::Display for DnsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for DnsError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DnsLimits {
    pub maximum_request_size: usize,
    pub maximum_response_size: usize,
    pub timeout: Duration,
}

pub trait DnsResolver {
    fn resolve(&self, request: &[u8]) -> Result<Vec<u8>, DnsError>;
}

pub struct MockDnsResolver {
    records: HashMap<String, Vec<u8>>,
    limits: DnsLimits,
    enabled: bool,
    timed_out: bool,
}
impl MockDnsResolver {
    pub fn new(
        records: HashMap<String, Vec<u8>>,
        limits: DnsLimits,
        enabled: bool,
    ) -> Result<Self, DnsError> {
        if limits.maximum_request_size == 0
            || limits.maximum_response_size == 0
            || limits.timeout.is_zero()
        {
            return Err(DnsError::Timeout);
        }
        Ok(Self {
            records,
            limits,
            enabled,
            timed_out: false,
        })
    }

    pub fn with_timeout(mut self, timed_out: bool) -> Self {
        self.timed_out = timed_out;
        self
    }
}
impl DnsResolver for MockDnsResolver {
    fn resolve(&self, request: &[u8]) -> Result<Vec<u8>, DnsError> {
        if !self.enabled {
            return Err(DnsError::Disabled);
        }
        if self.timed_out {
            return Err(DnsError::Timeout);
        }
        if request.is_empty() {
            return Err(DnsError::MalformedRequest);
        }
        if request.len() > self.limits.maximum_request_size {
            return Err(DnsError::RequestTooLarge);
        }
        let name = std::str::from_utf8(request).map_err(|_| DnsError::MalformedRequest)?;
        let response = self.records.get(name).ok_or(DnsError::UnknownHost)?;
        if response.len() > self.limits.maximum_response_size {
            return Err(DnsError::ResponseTooLarge);
        }
        Ok(response.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn resolver(enabled: bool) -> MockDnsResolver {
        MockDnsResolver::new(
            HashMap::from([(String::from("test.local"), vec![127, 0, 0, 1])]),
            DnsLimits {
                maximum_request_size: 32,
                maximum_response_size: 8,
                timeout: Duration::from_secs(1),
            },
            enabled,
        )
        .unwrap()
    }
    #[test]
    fn mock_dns_is_bounded_and_explicit() {
        let dns = resolver(true);
        assert_eq!(dns.resolve(b"test.local").unwrap(), vec![127, 0, 0, 1]);
        assert_eq!(dns.resolve(b"missing"), Err(DnsError::UnknownHost));
        assert_eq!(dns.resolve(b""), Err(DnsError::MalformedRequest));
        assert_eq!(
            resolver(false).resolve(b"test.local"),
            Err(DnsError::Disabled)
        );
    }
    #[test]
    fn mock_dns_rejects_limits() {
        let dns = resolver(true);
        assert_eq!(dns.resolve(&[b'x'; 33]), Err(DnsError::RequestTooLarge));
        let large = MockDnsResolver::new(
            HashMap::from([(String::from("large"), vec![1; 9])]),
            DnsLimits {
                maximum_request_size: 32,
                maximum_response_size: 8,
                timeout: Duration::from_secs(1),
            },
            true,
        )
        .unwrap();
        assert_eq!(large.resolve(b"large"), Err(DnsError::ResponseTooLarge));
    }
}
