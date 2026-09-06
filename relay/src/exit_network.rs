use std::fmt;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::str::FromStr;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitNetworkError {
    MalformedDestination,
    InvalidPort,
    DestinationNotAllowed,
    UnsupportedAddress,
    PayloadTooLarge,
    ResponseTooLarge,
    ConnectionTimeout,
    ReadTimeout,
    WriteTimeout,
    ConnectionFailed(String),
    ReadFailed(String),
    WriteFailed(String),
    Closed,
}

impl fmt::Display for ExitNetworkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for ExitNetworkError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DestinationPolicy {
    allowed: Vec<SocketAddr>,
}

impl DestinationPolicy {
    pub fn new(mut allowed: Vec<SocketAddr>) -> Self {
        allowed.sort_unstable();
        allowed.dedup();
        Self { allowed }
    }

    pub fn from_strings(destinations: &[String]) -> Result<Self, ExitNetworkError> {
        let mut allowed = Vec::with_capacity(destinations.len());
        for destination in destinations {
            let address = SocketAddr::from_str(destination)
                .map_err(|_| ExitNetworkError::MalformedDestination)?;
            if address.port() == 0 {
                return Err(ExitNetworkError::InvalidPort);
            }
            if address.ip().is_unspecified() {
                return Err(ExitNetworkError::UnsupportedAddress);
            }
            allowed.push(address);
        }
        Ok(Self::new(allowed))
    }

    pub fn localhost(port: u16) -> Self {
        Self::new(vec![SocketAddr::from(([127, 0, 0, 1], port))])
    }

    pub fn validate(&self, destination: &str) -> Result<SocketAddr, ExitNetworkError> {
        let address = SocketAddr::from_str(destination)
            .map_err(|_| ExitNetworkError::MalformedDestination)?;
        if address.port() == 0 {
            return Err(ExitNetworkError::InvalidPort);
        }
        if address.ip().is_unspecified() {
            return Err(ExitNetworkError::UnsupportedAddress);
        }
        if !self.allowed.contains(&address) {
            return Err(ExitNetworkError::DestinationNotAllowed);
        }
        Ok(address)
    }
}

pub trait ExitNetworkAdapter {
    fn send(&mut self, payload: &[u8]) -> Result<(), ExitNetworkError>;
    fn receive(&mut self) -> Result<Vec<u8>, ExitNetworkError>;
    fn close(&mut self) -> Result<(), ExitNetworkError>;

    fn exchange(&mut self, payload: &[u8]) -> Result<Vec<u8>, ExitNetworkError> {
        self.send(payload)?;
        self.receive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TcpAdapterConfig {
    pub maximum_request_size: usize,
    pub maximum_response_size: usize,
    pub connection_timeout: Duration,
    pub read_timeout: Duration,
    pub write_timeout: Duration,
}

impl TcpAdapterConfig {
    pub fn validate(&self) -> Result<(), ExitNetworkError> {
        if self.maximum_request_size == 0 || self.maximum_response_size == 0 {
            return Err(ExitNetworkError::PayloadTooLarge);
        }
        if self.connection_timeout.is_zero()
            || self.read_timeout.is_zero()
            || self.write_timeout.is_zero()
        {
            return Err(ExitNetworkError::ConnectionTimeout);
        }
        Ok(())
    }
}

pub struct TcpExitNetworkAdapter {
    stream: TcpStream,
    config: TcpAdapterConfig,
    closed: bool,
}

impl TcpExitNetworkAdapter {
    pub fn connect(
        destination: &str,
        policy: &DestinationPolicy,
        config: TcpAdapterConfig,
    ) -> Result<Self, ExitNetworkError> {
        config.validate()?;
        let address = policy.validate(destination)?;
        let stream =
            TcpStream::connect_timeout(&address, config.connection_timeout).map_err(|error| {
                if error.kind() == std::io::ErrorKind::TimedOut {
                    ExitNetworkError::ConnectionTimeout
                } else {
                    ExitNetworkError::ConnectionFailed(error.to_string())
                }
            })?;
        stream
            .set_read_timeout(Some(config.read_timeout))
            .map_err(|error| ExitNetworkError::ConnectionFailed(error.to_string()))?;
        stream
            .set_write_timeout(Some(config.write_timeout))
            .map_err(|error| ExitNetworkError::ConnectionFailed(error.to_string()))?;
        Ok(Self {
            stream,
            config,
            closed: false,
        })
    }
}

impl ExitNetworkAdapter for TcpExitNetworkAdapter {
    fn send(&mut self, payload: &[u8]) -> Result<(), ExitNetworkError> {
        if self.closed {
            return Err(ExitNetworkError::Closed);
        }
        if payload.len() > self.config.maximum_request_size {
            return Err(ExitNetworkError::PayloadTooLarge);
        }
        self.stream.write_all(payload).map_err(|error| {
            if error.kind() == std::io::ErrorKind::TimedOut {
                ExitNetworkError::WriteTimeout
            } else {
                ExitNetworkError::WriteFailed(error.to_string())
            }
        })
    }

    fn receive(&mut self) -> Result<Vec<u8>, ExitNetworkError> {
        if self.closed {
            return Err(ExitNetworkError::Closed);
        }
        let mut response = vec![0u8; self.config.maximum_response_size + 1];
        let size = self.stream.read(&mut response).map_err(|error| {
            if error.kind() == std::io::ErrorKind::TimedOut {
                ExitNetworkError::ReadTimeout
            } else {
                ExitNetworkError::ReadFailed(error.to_string())
            }
        })?;
        if size > self.config.maximum_response_size {
            return Err(ExitNetworkError::ResponseTooLarge);
        }
        response.truncate(size);
        Ok(response)
    }

    fn close(&mut self) -> Result<(), ExitNetworkError> {
        if self.closed {
            return Err(ExitNetworkError::Closed);
        }
        self.stream
            .shutdown(std::net::Shutdown::Both)
            .map_err(|error| ExitNetworkError::ConnectionFailed(error.to_string()))?;
        self.closed = true;
        Ok(())
    }
}

impl TcpExitNetworkAdapter {
    pub fn exchange(&mut self, payload: &[u8]) -> Result<Vec<u8>, ExitNetworkError> {
        self.send(payload)?;
        self.receive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::net::TcpListener;
    use std::thread;

    fn config() -> TcpAdapterConfig {
        TcpAdapterConfig {
            maximum_request_size: 64,
            maximum_response_size: 64,
            connection_timeout: Duration::from_secs(1),
            read_timeout: Duration::from_secs(1),
            write_timeout: Duration::from_secs(1),
        }
    }

    #[test]
    fn policy_allows_only_explicit_loopback_destination() {
        let policy = DestinationPolicy::localhost(9000);
        assert_eq!(
            policy.validate("127.0.0.1:9000").unwrap(),
            SocketAddr::from(([127, 0, 0, 1], 9000))
        );
        assert_eq!(
            policy.validate("127.0.0.1:9001"),
            Err(ExitNetworkError::DestinationNotAllowed)
        );
        assert_eq!(
            policy.validate("10.0.0.1:9000"),
            Err(ExitNetworkError::DestinationNotAllowed)
        );
        assert_eq!(
            policy.validate("127.0.0.1:0"),
            Err(ExitNetworkError::InvalidPort)
        );
        assert_eq!(
            policy.validate("localhost:9000"),
            Err(ExitNetworkError::MalformedDestination)
        );
        let external = DestinationPolicy::from_strings(&["192.0.2.10:9000".to_owned()]).unwrap();
        assert_eq!(
            external.validate("192.0.2.10:9000").unwrap(),
            "192.0.2.10:9000".parse().unwrap()
        );
        assert_eq!(
            external.validate("192.0.2.11:9000"),
            Err(ExitNetworkError::DestinationNotAllowed)
        );
        assert_eq!(
            DestinationPolicy::from_strings(&[
                "192.0.2.10:9000".to_owned(),
                "192.0.2.10:9000".to_owned()
            ])
            .unwrap()
            .allowed
            .len(),
            1
        );
        assert_eq!(
            DestinationPolicy::from_strings(&["0.0.0.0:9000".to_owned()]),
            Err(ExitNetworkError::UnsupportedAddress)
        );
        assert_eq!(
            DestinationPolicy::from_strings(&[])
                .unwrap()
                .validate("192.0.2.10:9000"),
            Err(ExitNetworkError::DestinationNotAllowed)
        );
    }

    #[test]
    fn tcp_adapter_exchanges_bounded_local_payload_and_closes() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 64];
            let size = stream.read(&mut request).unwrap();
            assert_eq!(&request[..size], b"hello");
            stream.write_all(b"ghost-layer-test-ack").unwrap();
        });
        let policy = DestinationPolicy::localhost(address.port());
        let mut adapter =
            TcpExitNetworkAdapter::connect(&address.to_string(), &policy, config()).unwrap();
        assert_eq!(adapter.exchange(b"hello").unwrap(), b"ghost-layer-test-ack");
        adapter.close().unwrap();
        assert_eq!(adapter.close(), Err(ExitNetworkError::Closed));
        server.join().unwrap();
    }

    #[test]
    fn tcp_adapter_rejects_large_payload_and_invalid_configuration() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let _ = listener.accept();
        });
        let mut small = config();
        small.maximum_request_size = 2;
        let policy = DestinationPolicy::localhost(address.port());
        let mut adapter =
            TcpExitNetworkAdapter::connect(&address.to_string(), &policy, small).unwrap();
        assert_eq!(
            adapter.send(b"too-large"),
            Err(ExitNetworkError::PayloadTooLarge)
        );
        assert_eq!(
            TcpAdapterConfig {
                maximum_request_size: 0,
                ..config()
            }
            .validate(),
            Err(ExitNetworkError::PayloadTooLarge)
        );
        adapter.close().unwrap();
        server.join().unwrap();
    }

    #[test]
    fn tcp_adapter_rejects_large_response() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 7];
            stream.read_exact(&mut request).unwrap();
            stream.write_all(&[7u8; 65]).unwrap();
        });
        let mut limited = config();
        limited.maximum_response_size = 64;
        let policy = DestinationPolicy::localhost(address.port());
        let mut adapter =
            TcpExitNetworkAdapter::connect(&address.to_string(), &policy, limited).unwrap();
        adapter.send(b"request").unwrap();
        assert_eq!(adapter.receive(), Err(ExitNetworkError::ResponseTooLarge));
        adapter.close().unwrap();
        server.join().unwrap();
    }

    #[test]
    fn tcp_adapter_reports_unavailable_connection() {
        let policy = DestinationPolicy::localhost(1);
        let result = TcpExitNetworkAdapter::connect(
            "127.0.0.1:1",
            &policy,
            TcpAdapterConfig {
                connection_timeout: Duration::from_millis(100),
                ..config()
            },
        );
        assert!(matches!(
            result,
            Err(ExitNetworkError::ConnectionFailed(_)) | Err(ExitNetworkError::ConnectionTimeout)
        ));
    }
}
