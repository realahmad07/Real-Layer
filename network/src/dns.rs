use std::collections::HashMap;
use std::fmt;
use std::net::{SocketAddr, UdpSocket};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DnsError {
    Disabled,
    MalformedRequest,
    UnknownHost,
    RequestTooLarge,
    ResponseTooLarge,
    Timeout,
    TransactionMismatch,
    InvalidResponse,
    ServerFailure,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DnsRecordType {
    A = 1,
    Aaaa = 28,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DnsQuery {
    pub transaction_id: u16,
    pub record_type: DnsRecordType,
}

impl DnsQuery {
    pub fn parse(packet: &[u8], maximum_size: usize) -> Result<Self, DnsError> {
        if packet.len() < 12 {
            return Err(DnsError::MalformedRequest);
        }
        if packet.len() > maximum_size {
            return Err(DnsError::RequestTooLarge);
        }
        if packet[2] & 0x80 != 0 || packet[4] != 0 || packet[5] != 1 {
            return Err(DnsError::MalformedRequest);
        }
        let mut index = 12;
        while index < packet.len() && packet[index] != 0 {
            let length = usize::from(packet[index]);
            if length == 0 || length > 63 || index + length + 1 >= packet.len() {
                return Err(DnsError::MalformedRequest);
            }
            index += length + 1;
        }
        if index + 4 >= packet.len() {
            return Err(DnsError::MalformedRequest);
        }
        let record_type = match u16::from_be_bytes([packet[index + 1], packet[index + 2]]) {
            1 => DnsRecordType::A,
            28 => DnsRecordType::Aaaa,
            _ => return Err(DnsError::MalformedRequest),
        };
        Ok(Self {
            transaction_id: u16::from_be_bytes([packet[0], packet[1]]),
            record_type,
        })
    }
}

pub struct UdpDnsResolver {
    server: SocketAddr,
    limits: DnsLimits,
    enabled: bool,
}

impl UdpDnsResolver {
    pub fn new(server: SocketAddr, limits: DnsLimits, enabled: bool) -> Result<Self, DnsError> {
        if limits.maximum_request_size == 0
            || limits.maximum_response_size == 0
            || limits.timeout.is_zero()
        {
            return Err(DnsError::Timeout);
        }
        if server.port() == 0 || server.ip().is_unspecified() {
            return Err(DnsError::MalformedRequest);
        }
        Ok(Self {
            server,
            limits,
            enabled,
        })
    }

    pub fn resolve_packet(&self, request: &[u8]) -> Result<Vec<u8>, DnsError> {
        if !self.enabled {
            return Err(DnsError::Disabled);
        }
        let query = DnsQuery::parse(request, self.limits.maximum_request_size)?;
        let socket = UdpSocket::bind(if self.server.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        })
        .map_err(|_| DnsError::Timeout)?;
        socket
            .set_read_timeout(Some(self.limits.timeout))
            .map_err(|_| DnsError::Timeout)?;
        socket
            .set_write_timeout(Some(self.limits.timeout))
            .map_err(|_| DnsError::Timeout)?;
        socket
            .send_to(request, self.server)
            .map_err(|_| DnsError::Timeout)?;
        let mut response = vec![0u8; self.limits.maximum_response_size + 1];
        let size = socket.recv(&mut response).map_err(|_| DnsError::Timeout)?;
        if size > self.limits.maximum_response_size {
            return Err(DnsError::ResponseTooLarge);
        }
        response.truncate(size);
        if response.len() < 12
            || u16::from_be_bytes([response[0], response[1]]) != query.transaction_id
        {
            return Err(DnsError::TransactionMismatch);
        }
        if response[2] & 0x80 == 0 {
            return Err(DnsError::InvalidResponse);
        }
        match response[3] & 0x0f {
            0 | 3 => Ok(response),
            2 => Err(DnsError::ServerFailure),
            _ => Err(DnsError::InvalidResponse),
        }
    }
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

    #[test]
    fn parses_bounded_a_and_aaaa_queries_and_rejects_malformed_packets() {
        let mut a = vec![0x12, 0x34, 0x01, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0];
        a.extend([1, b'a', 0]);
        a.extend([0, 1, 0, 1]);
        assert_eq!(
            DnsQuery::parse(&a, 512).unwrap().record_type,
            DnsRecordType::A
        );
        let mut aaaa = a.clone();
        aaaa[2] = 0x01;
        let type_index = aaaa.len() - 4;
        aaaa[type_index] = 0;
        aaaa[type_index + 1] = 28;
        assert_eq!(
            DnsQuery::parse(&aaaa, 512).unwrap().record_type,
            DnsRecordType::Aaaa
        );
        assert_eq!(DnsQuery::parse(&[], 512), Err(DnsError::MalformedRequest));
        assert_eq!(
            DnsQuery::parse(&[0; 513], 512),
            Err(DnsError::RequestTooLarge)
        );
    }

    fn wire_query() -> Vec<u8> {
        let mut query = vec![0x12, 0x34, 0x01, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0];
        query.extend_from_slice(&[4, b't', b'e', b's', b't', 0, 0, 1, 0, 1]);
        query
    }

    fn resolver_with_response(response: Vec<u8>) -> Result<Vec<u8>, DnsError> {
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            let mut request = [0u8; 512];
            let (size, peer) = server.recv_from(&mut request).unwrap();
            let response = if response.is_empty() {
                request[..size].to_vec()
            } else {
                response
            };
            server.send_to(&response, peer).unwrap();
        });
        let resolver = UdpDnsResolver::new(
            address,
            DnsLimits {
                maximum_request_size: 512,
                maximum_response_size: 512,
                timeout: Duration::from_secs(1),
            },
            true,
        )?;
        let result = resolver.resolve_packet(&wire_query());
        thread.join().unwrap();
        result
    }

    #[test]
    fn udp_dns_runtime_accepts_nxdomain_without_local_fallback() {
        let mut response = wire_query();
        response[2] = 0x81;
        response[3] = 0x83;
        assert_eq!(resolver_with_response(response).unwrap()[3] & 0x0f, 3);
    }

    #[test]
    fn udp_dns_runtime_rejects_malformed_response() {
        let mut response = vec![0; 12];
        response[0] = 0x12;
        response[1] = 0x34;
        assert_eq!(
            resolver_with_response(response),
            Err(DnsError::InvalidResponse)
        );
    }

    #[test]
    fn udp_dns_runtime_rejects_transaction_mismatch() {
        let mut response = wire_query();
        response[0] = 0xab;
        assert_eq!(
            resolver_with_response(response),
            Err(DnsError::TransactionMismatch)
        );
    }
}
