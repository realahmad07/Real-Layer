//! Shared networking primitives for Ghost Layer.

pub mod channel;
pub mod config;
pub mod data_plane;
pub mod discovery;
pub mod dns;
pub mod error;
pub mod health;
pub mod mtu;
pub mod nat;
pub mod packet;
pub mod peer;
pub mod pipeline;
pub mod return_path;
pub mod routing;
pub mod session;
pub mod transport;
pub mod tun;

pub use channel::{
    ChannelEnvelope, ChannelError, ChannelMessage, ChannelState, EncryptedChannel,
    CHANNEL_PROTOCOL_VERSION,
};
pub use config::{LogLevel, NetworkEnvironment, NodeConfig, RouteMode, TunConfig};
pub use data_plane::{
    DataPlane, DataPlaneEnvelope, DataPlaneError, DataPlaneMessage, DataPlaneMessageType,
    DATA_PLANE_KIND, DATA_PLANE_PROTOCOL_VERSION, DEFAULT_MAXIMUM_PAYLOAD_SIZE,
};
pub use discovery::{
    ConfiguredPeerDiscovery, DiscoveryRequestId, DiscoveryResponseChannel, DiscoveryService,
    RelayDiscoveryCodec, RelayDiscoveryProtocol,
};
pub use dns::{DnsError, DnsLimits, DnsResolver, MockDnsResolver};
pub use error::ConfigError;
pub use health::HealthStatus;
pub use libp2p::Multiaddr;
pub use mtu::{MtuError, MtuPolicy};
pub use nat::{FlowKey, NatError, NatMapping, NatTable};
pub use packet::{NetworkPacket, PacketError, PacketType};
pub use peer::{Libp2pPeerId as PeerId, NodeIdentity};
pub use pipeline::{PacketPipeline, PipelineError};
pub use return_path::{ReturnPathError, ReturnPathKey, ReturnPathMapping, ReturnPathTable};
pub use routing::{classify_destination, decide, DestinationClass, RoutingDecision, RoutingError};
pub use session::{
    HandshakeInit, HandshakeResponse, RouteBinding, SecureSession, SessionError, SessionId,
    SessionInitiator, SessionResponder, SessionRole, SessionState,
};
pub use transport::{Behaviour, NetworkEvent, NetworkNode};
pub use tun::{TunDataPlane, TunPipelineError};
pub use tun::{TunDevice, TunError};
