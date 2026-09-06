use anyhow::{Context, Result};
use async_trait::async_trait;
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use libp2p::request_response;
use libp2p::Multiaddr;
use std::io;

pub const RELAY_DISCOVERY_PROTOCOL: &str = "/ghost-layer/relay-discovery/1.0.0";
pub type DiscoveryRequestId = request_response::OutboundRequestId;

pub trait DiscoveryService {
    fn known_peers(&self) -> &[Multiaddr];
}

#[derive(Debug, Clone, Default)]
pub struct ConfiguredPeerDiscovery {
    peers: Vec<Multiaddr>,
}

impl ConfiguredPeerDiscovery {
    pub fn from_addresses<I, S>(addresses: I) -> Result<Self>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let peers = addresses
            .into_iter()
            .map(|address| {
                address
                    .as_ref()
                    .parse()
                    .with_context(|| format!("invalid discovery address: {}", address.as_ref()))
            })
            .collect::<Result<Vec<Multiaddr>>>()?;
        Ok(Self { peers })
    }
}

impl DiscoveryService for ConfiguredPeerDiscovery {
    fn known_peers(&self) -> &[Multiaddr] {
        &self.peers
    }
}

#[derive(Debug, Clone, Default)]
pub struct RelayDiscoveryProtocol;

impl AsRef<str> for RelayDiscoveryProtocol {
    fn as_ref(&self) -> &str {
        RELAY_DISCOVERY_PROTOCOL
    }
}

#[derive(Debug, Clone, Default)]
pub struct RelayDiscoveryCodec;

#[async_trait]
impl request_response::Codec for RelayDiscoveryCodec {
    type Protocol = RelayDiscoveryProtocol;
    type Request = Vec<u8>;
    type Response = Vec<u8>;

    async fn read_request<T>(&mut self, _: &Self::Protocol, io: &mut T) -> io::Result<Self::Request>
    where
        T: AsyncRead + Unpin + Send,
    {
        let mut bytes = Vec::new();
        io.read_to_end(&mut bytes).await?;
        Ok(bytes)
    }

    async fn read_response<T>(
        &mut self,
        _: &Self::Protocol,
        io: &mut T,
    ) -> io::Result<Self::Response>
    where
        T: AsyncRead + Unpin + Send,
    {
        let mut bytes = Vec::new();
        io.read_to_end(&mut bytes).await?;
        Ok(bytes)
    }

    async fn write_request<T>(
        &mut self,
        _: &Self::Protocol,
        io: &mut T,
        request: Self::Request,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        io.write_all(&request).await?;
        io.close().await
    }

    async fn write_response<T>(
        &mut self,
        _: &Self::Protocol,
        io: &mut T,
        response: Self::Response,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        io.write_all(&response).await?;
        io.close().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_is_versioned() {
        assert_eq!(
            RelayDiscoveryProtocol.as_ref(),
            "/ghost-layer/relay-discovery/1.0.0"
        );
    }
}
