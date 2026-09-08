use crate::{DestinationPolicy, ExitNetworkError};
use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::time::Duration;

pub struct UdpExitNetworkAdapter {
    socket: UdpSocket,
    destination: SocketAddr,
    maximum_datagram_size: usize,
    closed: bool,
}

impl UdpExitNetworkAdapter {
    pub fn connect(
        destination: &str,
        policy: &DestinationPolicy,
        maximum_datagram_size: usize,
        timeout: Duration,
    ) -> Result<Self, ExitNetworkError> {
        if maximum_datagram_size == 0 || timeout.is_zero() {
            return Err(ExitNetworkError::PayloadTooLarge);
        }
        let destination = policy
            .validate_udp(destination)
            .map_err(|_| ExitNetworkError::DestinationNotAllowed)?;
        let socket = UdpSocket::bind(if destination.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        })
        .map_err(|error| ExitNetworkError::ConnectionFailed(error.to_string()))?;
        socket
            .connect(destination)
            .map_err(|error| ExitNetworkError::ConnectionFailed(error.to_string()))?;
        socket
            .set_read_timeout(Some(timeout))
            .map_err(|error| ExitNetworkError::ConnectionFailed(error.to_string()))?;
        socket
            .set_write_timeout(Some(timeout))
            .map_err(|error| ExitNetworkError::ConnectionFailed(error.to_string()))?;
        Ok(Self {
            socket,
            destination,
            maximum_datagram_size,
            closed: false,
        })
    }

    pub fn send(&mut self, payload: &[u8]) -> Result<(), ExitNetworkError> {
        if self.closed {
            return Err(ExitNetworkError::Closed);
        }
        if payload.len() > self.maximum_datagram_size {
            return Err(ExitNetworkError::PayloadTooLarge);
        }
        let written = self.socket.send(payload).map_err(map_io)?;
        if written != payload.len() {
            return Err(ExitNetworkError::WriteFailed(
                "short datagram write".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn receive(&mut self) -> Result<Vec<u8>, ExitNetworkError> {
        if self.closed {
            return Err(ExitNetworkError::Closed);
        }
        let mut buffer = vec![0u8; self.maximum_datagram_size + 1];
        let size = self.socket.recv(&mut buffer).map_err(map_io)?;
        if size > self.maximum_datagram_size {
            return Err(ExitNetworkError::ResponseTooLarge);
        }
        buffer.truncate(size);
        Ok(buffer)
    }

    pub fn exchange(&mut self, payload: &[u8]) -> Result<Vec<u8>, ExitNetworkError> {
        self.send(payload)?;
        self.receive()
    }

    pub fn close(&mut self) -> Result<(), ExitNetworkError> {
        if self.closed {
            return Err(ExitNetworkError::Closed);
        }
        self.closed = true;
        Ok(())
    }

    pub fn destination(&self) -> SocketAddr {
        self.destination
    }
}

fn map_io(error: io::Error) -> ExitNetworkError {
    match error.kind() {
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => ExitNetworkError::ReadTimeout,
        _ => ExitNetworkError::ReadFailed(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DestinationPolicy;
    use std::thread;

    #[test]
    fn exchanges_bounded_allowlisted_datagram() {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let address = socket.local_addr().unwrap();
        let server = thread::spawn(move || {
            let mut buffer = [0u8; 32];
            let (size, peer) = socket.recv_from(&mut buffer).unwrap();
            assert_eq!(&buffer[..size], b"hello");
            socket.send_to(b"ack", peer).unwrap();
        });
        let policy = DestinationPolicy::from_strings(&[address.to_string()]).unwrap();
        let mut adapter = UdpExitNetworkAdapter::connect(
            &address.to_string(),
            &policy,
            32,
            Duration::from_secs(1),
        )
        .unwrap();
        assert_eq!(adapter.exchange(b"hello").unwrap(), b"ack");
        adapter.close().unwrap();
        server.join().unwrap();
    }
}
