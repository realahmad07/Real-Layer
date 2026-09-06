//! Shared networking primitives for Ghost Layer.

pub mod channel;
pub mod config;
pub mod discovery;
pub mod error;
pub mod health;
pub mod peer;
pub mod session;
pub mod transport;

pub use channel::{
    ChannelEnvelope, ChannelError, ChannelMessage, ChannelState, EncryptedChannel,
    CHANNEL_PROTOCOL_VERSION,
};
pub use config::{LogLevel, NetworkEnvironment, NodeConfig, RouteMode};
pub use discovery::{
    ConfiguredPeerDiscovery, DiscoveryRequestId, DiscoveryService, RelayDiscoveryCodec,
    RelayDiscoveryProtocol,
};
pub use error::ConfigError;
pub use health::HealthStatus;
pub use libp2p::Multiaddr;
pub use peer::{Libp2pPeerId as PeerId, NodeIdentity};
pub use session::{
    HandshakeInit, HandshakeResponse, RouteBinding, SecureSession, SessionError, SessionId,
    SessionInitiator, SessionResponder, SessionRole, SessionState,
};
pub use transport::{Behaviour, NetworkEvent, NetworkNode};
