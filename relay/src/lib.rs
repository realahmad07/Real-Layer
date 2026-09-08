mod candidate;
mod connection;
mod core;
mod exit_handler;
mod exit_network;
mod forwarding;
mod forwarding_manager;
mod health;
mod heartbeat;
mod metadata;
mod registry;
mod routing;
mod session_manager;
mod state;
mod udp;

pub use candidate::{CandidateRequirements, RelayCandidate, RelayCandidateError, RelayRanking};
pub use connection::{
    ConnectionManager, ConnectionManagerError, ConnectionRecord, ConnectionState,
};
pub use core::{
    RelayCore, RelayCoreError, RelayLifecycle, RelayMetrics, RelayOperationalState, RelayRuntime,
};
pub use exit_handler::{ExitForwardingBinding, ExitPacketHandler, ExitPacketHandlerError};
pub use exit_network::{
    DestinationPolicy, ExitNetworkAdapter, ExitNetworkError, TcpAdapterConfig,
    TcpExitNetworkAdapter,
};
pub use forwarding::{
    ForwardedPacket, ForwardedProtocol, ForwardingContext, ForwardingDirection, ForwardingError,
    ForwardingMessage, ForwardingState, Hop, MultiHopSession, FORWARDING_KIND,
};
pub use forwarding_manager::ForwardingManager;
pub use health::{HealthClass, HealthThresholds, RelayHealth};
pub use heartbeat::RelayHeartbeat;
pub use metadata::{RelayMetadata, RelayMetadataRequest};
pub use registry::{InMemoryRelayRegistry, RegistryError, RelayRegistry};
pub use routing::{RelayHop, Route, RouteSelectionError, RouteSelectionPolicy, RouteSelector};
pub use session_manager::{SessionBinding, SessionManager, SessionManagerError};
pub use state::{LatencyMeasurement, RelayState, RelayStatus};
pub use udp::UdpExitNetworkAdapter;
