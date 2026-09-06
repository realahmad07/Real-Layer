# Ghost Layer Architecture

Ghost Layer is intended to provide censorship-resistant network access through independently operated relay nodes. The MVP foundation defines boundaries and shared interfaces; it does not claim anonymity, production security, or a complete VPN/tunneling implementation.

## Control Plane

The control plane manages node identity, discovery, registration, staking, health, availability, rewards, and session coordination. Relay operators remain independently responsible for their nodes. This phase implements the relay's local operational state and an in-memory registration boundary; decentralized discovery and settlement remain future work.

## Data Plane

The data plane carries only controlled application-level test messages through selected relay nodes over encrypted transport. User traffic and payload data must remain off-chain. Solana and MagicBlock must never be used as a data path for user traffic.

The repository intentionally does not implement packet forwarding, VPN behavior, tunneling, traffic obfuscation, or privacy-sensitive cryptography. Prompt 9 adds only bounded forwarding of a typed Ghost Layer application message between an Entry and Exit relay.

## Client

The Rust client will eventually load user configuration, discover eligible relays, establish sessions, and provide a local interface for future traffic handling. The current client only validates configuration and creates a placeholder transport configuration.

## Relay Node

The relay loads a persistent Ed25519 identity and starts the shared libp2p + QUIC swarm. Its `RelayState` owns strongly typed operational state: PeerId, uptime, status, listening and advertised addresses, active peers, heartbeat time, connection counters, measured Ping latency, and software version. `RelayState` consumes translated network events rather than depending on libp2p event types.

`RelayHealth` uses a deliberately simple classification. A stale heartbeat or loss of all peers after a successful connection is Unhealthy; latency above `GHOST_DEGRADED_LATENCY_MS` is Degraded; otherwise the relay is Healthy. This is an operational signal, not a reputation or production availability system.

The relay emits a `RelayHeartbeat` at `GHOST_HEARTBEAT_INTERVAL_SECS`. Each heartbeat contains the PeerId, timestamp, status, uptime, active peer count, latest successful latency, and software version. Heartbeats are logged as structured fields and never contain private keys, traffic, or payloads.

The flow is:

```text
libp2p
	↓
network events
	↓
RelayState
	↓
health / heartbeat
	↓
future MagicBlock coordination
```

Relay metadata advertises only currently implemented capabilities: QUIC and the relay node itself. One-hop and two-hop routing are intentionally absent from the capability list because routing is not implemented.

`RelayRegistry` is a trait with register, update, get, and remove operations. The current `InMemoryRelayRegistry` is dependency-free and is used as the local implementation. A future decentralized or Solana-backed registry should replace it without changing relay business logic:

```text
RelayRegistry
	↓
current: in-memory
future: decentralized/Solana-backed implementation
```

The future on-chain layer must carry control-plane metadata only. It must never carry user traffic or payload contents.

## Network Layer

The `network` crate owns configuration, persistent Ed25519 node identity, libp2p `PeerId` handling, QUIC transport setup, and the event abstraction consumed by the client and relay. An identity is generated once at `GHOST_IDENTITY_PATH` and loaded on later starts; private key bytes are never logged. libp2p supplies the authenticated QUIC transport and peer identity without custom cryptography.

Each swarm enables only Identify and Ping. Identify confirms the remote `PeerId` and implementation agent, while Ping provides a basic liveness/latency signal. Bootstrap peers are explicit multiaddrs in `GHOST_BOOTSTRAP_PEERS`; there is no public discovery service in this phase.

The client dials the configured relay multiaddr and maintains the asynchronous swarm. The relay listens and reports incoming/outgoing connections. Neither binary forwards user traffic, creates VPN tunnels, implements routing hops, or provides anonymity or metadata protection yet.

## Relay Discovery

Discovery currently operates through libp2p/local configuration and is not yet a decentralized economic registry. The network crate exposes a replaceable `DiscoveryService` interface and a configured-peer implementation. The client dials only the multiaddrs in `GHOST_BOOTSTRAP_PEERS`; it does not contact arbitrary internet hosts.

The discovery flow is:

```text
Client
	|
	v
DiscoveryService
	|
	v
Relay candidate list
	|
	v
Candidate ranking
	|
	v
Selected relay
	|
	v
Existing libp2p network
```

### Relay advertisement

Relays answer the versioned `/ghost-layer/relay-discovery/1.0.0` libp2p request/response protocol. A request contains the expected protocol version; a compatible relay responds with serde JSON metadata containing its PeerId, protocol and software versions, QUIC transport, implemented capabilities, listening/advertised address, status, local health class, latest measured Ping latency, metadata timestamp, and heartbeat timestamp. The protocol carries metadata only, never user traffic, secrets, or payload contents.

Only `quic` and `relay` are advertised today. `one_hop` and `two_hop` are intentionally absent because route selection and forwarding are not implemented.

### Candidate validation and filtering

The client validates the protocol version, PeerId decoding, timestamp freshness, heartbeat freshness, multiaddr syntax, known transport values, and known capability values. It rejects Offline relays, stale heartbeats, unavailable required transports, and missing required capabilities. The default requirements are configurable with `GHOST_DISCOVERY_REQUIRED_TRANSPORT` and `GHOST_DISCOVERY_REQUIRED_CAPABILITIES`.

Relay health is self-reported and locally computed, not cryptographically attested. Metadata must therefore be treated as an operational hint rather than absolute truth or a reputation score.

### Candidate ranking

Eligible candidates are ranked deterministically by:

1. Health: Healthy, then Degraded, then Unhealthy.
2. Status: Online before other non-Offline statuses.
3. Measured latency: lower Ping latency first; missing latency is last.
4. Metadata freshness: newer timestamps first.
5. PeerId lexical order as a stable tie-breaker.

The ranking code is isolated in `RelayRanking` so a future decentralized reputation policy can replace it without changing discovery transport or relay state.

### Current limitations and future discovery

The current mechanism is configured/local libp2p discovery. It does not implement a DHT, Solana registry, MagicBlock coordination, cryptographic attestation, VPN tunneling, or Internet traffic forwarding. A future architecture may layer a Solana registry and MagicBlock coordination above discovery, but those systems must produce relay candidates from control-plane data only and must never carry user traffic.

## Route Selection

Route selection consumes the validated `RelayCandidate` list and produces metadata-only path descriptions. It does not establish a forwarding session and does not carry payloads.

A one-hop route is:

```text
Client -> Relay
```

A two-hop route explicitly distinguishes roles:

```text
Client -> Entry Relay -> Exit Relay
```

The Entry Relay is the first selected hop reached by the client. The Exit Relay is the final selected hop before future destination forwarding. Entry and Exit must have different PeerIds, and neither may equal the client PeerId.

`RouteSelector` accepts a client PeerId, discovered candidates, and a `RouteSelectionPolicy`. It requires Online status, Healthy health, usable QUIC addresses, the configured transport, and all configured capabilities. One-hop selection chooses the best eligible candidate. Two-hop selection chooses the best two distinct eligible candidates. Ranking is deterministic: Healthy before Degraded/Unhealthy, Online status, lower latency, fresher metadata, then lexical PeerId tie-breaking.

The route mode is configured with `GHOST_ROUTE_MODE=one-hop` or `GHOST_ROUTE_MODE=two-hop`; one-hop is the default. The client logs the selected route but does not connect application traffic through it.

Route selection remains metadata-only as a selection mechanism, but the selected two-hop route now feeds the controlled forwarding test described below. It does not authorize arbitrary destinations or traffic.

## Secure Session Establishment

Secure session establishment is implemented for protocol validation and future encrypted forwarding. User traffic forwarding and VPN tunneling are intentionally deferred.

The session layer is separate from long-term node identity:

```text
Persistent libp2p Ed25519 identity
	↓ signs/authenticates
Ephemeral X25519 key agreement
	↓ HKDF-SHA256 with protocol, SessionId, identities, and route context
Directional session keys
	↓ ChaCha20-Poly1305 for controlled session-level test data
Future encrypted forwarding
```

The existing libp2p PeerId identifies the expected remote. Each handshake carries the libp2p public key, derives its PeerId, and verifies a signature over the versioned session transcript. A response is rejected when its identity does not match the expected route hop or connected transport peer.

Handshake messages are versioned `HandshakeInit` and `HandshakeResponse` structures exchanged over the existing authenticated libp2p request/response channel. Their explicit message kinds prevent metadata requests from being interpreted as session handshakes. Messages contain protocol version, SessionId, explicit identities, ephemeral public keys, route binding, and authentication signatures. Private keys and derived session keys are never serialized or logged.

The route binding is validated before key derivation. A one-hop session binds the client to one relay. A two-hop session binds the client to distinct Entry and Exit PeerIds; the current MVP establishes the cryptographic session with the selected Entry relay while preserving the full two-hop context for future layered encryption. It does not forward packets from Entry to Exit.

Session state is explicit: `Negotiating -> Established -> Closed`, with failures rejected rather than silently transitioning. Responder SessionIds are tracked to reject duplicate handshakes. AEAD nonces are generated from a monotonic per-session counter, associated data is authenticated, malformed or modified ciphertext is rejected, and received counters cannot be replayed.

The cryptographic implementation uses established `x25519-dalek`, `hkdf` with SHA-256, `sha2`, `chacha20poly1305`, `rand`, and `zeroize` crates. No custom cryptographic primitives are implemented. This is an MVP session layer and is not a claim of production privacy or anonymity.

## Encrypted Session Channel

The encrypted channel is an application-level layer carried over the existing authenticated libp2p/QUIC request/response connection:

```text
libp2p
	↓
QUIC
	↓
existing request/response application carrier
	↓
EncryptedChannel
	↓
Encrypted data plane for controlled application payloads
```

`EncryptedChannel` owns the associated SessionId, authenticated peer, route context, lifecycle state, independent send/receive sequences, protocol version, maximum frame size, and bounded outbound queue. It consumes the already-established `SecureSession`; it does not derive another key set.

Each frame contains a length-prefixed serialized header and ciphertext. The header includes protocol version, SessionId, message type, sequence number, and ciphertext length. The header with its length field is bounded before parsing, and ciphertext length must match the remaining frame exactly. The supported message types are Ping, Pong, legacy SessionData, DataPlane, and Close. DataPlane is a distinct encrypted message type and is carried in a `DataPlaneEnvelope` whose kind is `data_plane`.

The header metadata is authenticated as AEAD associated data. `SecureSession` supplies the ChaCha20-Poly1305 key and its monotonic nonce construction; the channel maintains separate strictly increasing directional sequence numbers. Receive sequence numbers must arrive exactly in order, duplicates are rejected as replay, gaps are rejected, and the channel cannot wrap the sequence counter. A channel never accepts a frame for another SessionId or after closure.

Outbound buffering is bounded. A caller receives `Backpressure` when the configured pending-message capacity is full rather than allowing unbounded memory growth. Malformed, oversized, unsupported, unauthenticated, replayed, and incorrectly sequenced frames are rejected without exposing plaintext.

The `DataPlane` abstraction accepts only bounded application payloads, binds each envelope to its expected SessionId, and returns a structured message containing the session ID, channel sequence number, data type, and payload. It delegates encryption, framing, ordering, replay rejection, lifecycle, and backpressure to `EncryptedChannel`. The current client/relay flow establishes a secure session, sends `hello ghost layer`, and receives the controlled encrypted response `ack: hello ghost layer` before closing. No payload contents are logged.

The data-plane lifecycle is:

```text
application payload
	↓ size and session validation
DataPlane
	↓ DataPlane message
EncryptedChannel
	↓ AEAD frame with authenticated header
libp2p/QUIC request-response carrier
```

For a two-hop route, the channel preserves the Entry/Exit route binding in the underlying session context, but Entry-to-Exit forwarding and layered/onion forwarding are not implemented. Ghost Layer does not yet forward arbitrary Internet traffic.

Encrypted session channels and the data plane are currently used for controlled application-level protocol tests only. This is not a VPN: there is no TUN/TAP interface, OS routing, NAT, DNS forwarding, proxy, or arbitrary IP packet forwarding. Future TUN/TAP integration must feed a separate bounded adapter, and future Internet forwarding must sit beyond the current relay-terminating data-plane boundary.

## Multi-Hop Forwarding

Prompt 9 adds the first controlled multi-hop path:

```text
Client
	↓ client ↔ Entry SecureSession + EncryptedChannel
Entry Relay
	↓ Entry ↔ Exit SecureSession + EncryptedChannel
Exit Relay
	↓ controlled acknowledgement
Entry Relay
	↓
Client
```

`ForwardingContext` binds the client SessionId, client PeerId, Entry PeerId, Exit PeerId, selected `RouteBinding::TwoHop`, and a generated forwarding identity. Its explicit state machine is `Created -> Connecting -> Established -> Forwarding -> Closing -> Closed`. Invalid transitions, duplicate request/direction pairs, wrong peers, wrong sessions, malformed forwarding envelopes, and route mismatches fail closed.

`ForwardingMessage` is the only relay-to-relay application envelope. It contains forwarding identity, client session identity, the exact selected route, and a typed encrypted `DataPlaneEnvelope`; it is not a generic byte-forwarding API. Entry validates the client-bound channel, decrypts the controlled data-plane message, and re-encrypts only that typed application payload onto its authenticated relay-to-relay channel. Exit validates the forwarding context and Entry identity, processes the controlled test payload, and returns an acknowledgement over the same bounded encrypted channel. No private keys, session keys, or payload contents are logged.

The client-to-Entry and Entry-to-Exit channels are separate secure sessions with independent identities, AEAD keys, nonces, and sequence/replay state. The forwarding context prevents cross-session routing, while the route binding prevents an Entry from silently substituting an unauthorized Exit. Channel errors, backpressure, malformed frames, duplicate messages, closed sessions, and unavailable peers are surfaced as structured forwarding or data-plane errors; bounded integration timeouts terminate failed local flows.

The current multi-hop implementation forwards only the controlled `hello ghost layer` application message and returns `ack: hello ghost layer`. Ghost Layer still does not provide a VPN or Internet traffic forwarding. There is no TUN/TAP, OS routing, NAT, DNS, SOCKS/HTTP proxy, arbitrary IP packet handling, public Internet access, or Solana/MagicBlock coordination. Future forwarding adapters must preserve this context and state boundary.

## Restricted Exit Network Adapter

Prompt 13 adds a deliberately narrow outbound boundary after the Exit Packet Handler:

```text
Exit Relay
	↓ validated NetworkPacket and forwarding context
ExitPacketHandler
	↓ explicit DestinationPolicy
TcpExitNetworkAdapter
	↓ localhost-only test destination
local test server
```

`DestinationPolicy` accepts only literal `SocketAddr` values that are explicitly allowlisted. Loopback remains available through the development helper, while configured external IPv4/IPv6 addresses are accepted only when their exact IP and port appear in the allowlist. It rejects malformed addresses, port zero, and destinations that were not configured. It never resolves hostnames, accepts wildcards, falls back to unrestricted destinations, changes routes, or opens UDP connections.

`TcpExitNetworkAdapter` uses Rust TCP primitives with bounded request and response sizes, connection/read/write timeouts, and clean shutdown. The Exit Packet Handler invokes it only after validating the client session, selected route, Entry and Exit identities, relay session, forwarding state, and duplicate packet identity. The response is validated back into `NetworkPacket` before returning through Entry. The integration test uses a deterministic local TCP server as the explicitly configured destination; the same policy can hold a controlled external test address without adding an unrestricted proxy.

This adapter is a controlled test boundary, not a proxy. UDP, NAT, DNS, route installation, arbitrary sockets, unrestricted Internet forwarding, and production VPN behavior remain future work. The Exit Packet Handler and adapter intentionally have no capability to send traffic outside the explicit test allowlist. `GHOST_ALLOWED_EXIT_DESTINATIONS` configures literal `IP:PORT` entries and defaults to an empty list.

## Controlled External TCP Exit

Prompts 16-19 provide an opt-in controlled external TCP exit without enabling unrestricted Internet forwarding. Set `GHOST_EXTERNAL_TEST_DESTINATION` to one literal IPv4/IPv6 `IP:PORT` and include that exact normalized value in `GHOST_ALLOWED_EXIT_DESTINATIONS`. Configuration fails closed when the destination is absent from the allowlist. Hostnames, wildcards, CIDR ranges, arbitrary ports, implicit DNS, and automatic destination discovery are rejected.

The Entry relay forwards the authenticated encrypted payload to the selected Exit relay. The Exit validates the session, route, Entry and Exit peers, relay session, forwarding state, duplicate identity, packet bounds, and MTU through `ExitPacketHandler`, then calls the existing `TcpExitNetworkAdapter` with the exact configured destination. The bounded TCP response returns through the same encrypted Exit -> Entry -> Client forwarding path. No direct return socket is created.

The opt-in external test sends the bounded request `ghost-layer-external-test`. It requires an explicitly configured and allowlisted destination. If no destination is configured it reports `EXTERNAL INTERNET TEST SKIPPED — NO AUTHORIZED DESTINATION CONFIGURED`. If `GHOST_EXTERNAL_TEST_RESPONSE` is set, the received bytes must match it exactly; otherwise the test records the received byte count without assuming that an arbitrary TCP service echoes, speaks HTTP, or returns a fixed response. A connection or read failure is reported as an actual test failure. The Prompt 17A iPhone regression remains a separate real-runtime echo test at `192.168.1.3:9005`.

This is an application-level controlled TCP exit, not a proxy or production VPN. TUN packets may reach this boundary only through the existing bounded packet, routing, encrypted session, and multi-hop forwarding layers. NAT, DNS interception, OS routing, default-route installation, UDP, transparent proxying, arbitrary destination forwarding, and unrestricted Internet access remain disabled.

## VPN Networking Foundations

Prompt 15 adds platform-independent models needed before a production data plane, without enabling those production behaviors. `PacketPipeline` applies the following bounded sequence:

```text
NetworkPacket
	↓ packet and MTU validation
destination classification
	↓ deterministic RoutingDecision
optional future NAT boundary
	↓ existing DataPlane
encrypted session / multi-hop forwarding
```

The destination classifier reads only IPv4/IPv6 packet headers and never resolves hostnames. Loopback, private/unique-local, and link-local addresses are classified for local/system handling; multicast and IPv4 broadcast are dropped by the default policy; public addresses are eligible for the Ghost Layer path; unspecified or malformed destinations are unsupported. These classifications are policy inputs, not permission to alter the operating system routing table.

`MtuPolicy` owns the configured tunnel MTU and maximum packet size. It rejects empty, malformed, and oversized packets, preserves bytes within the limit, and does not fragment or truncate. `NatTable` and `ReturnPathTable` are bounded in-memory state models with deterministic allocation, reverse lookup, expiration, duplicate detection, and cleanup. They do not modify OS NAT state, expose the machine as a gateway, or forward return traffic.

`DnsResolver` is an explicit mock-capable interface with request/response bounds and a timeout setting. `MockDnsResolver` is disabled unless explicitly enabled in its construction and is used only for deterministic tests. Ghost Layer does not intercept DNS, modify system DNS, or resolve arbitrary hostnames automatically. `GHOST_NAT_ENABLED` and `GHOST_DNS_ENABLED` default to false.

NAT is NOT active. DNS interception is NOT active. OS routing is NOT modified by default, and default-route installation is rejected. UDP is NOT implemented. Unrestricted Internet forwarding is NOT implemented. Production VPN behavior is NOT implemented. The existing controlled TCP allowlist and Client -> Entry -> Exit localhost test remain the only outbound path.

## TUN/TAP Boundary

Prompt 10 adds a platform-independent `TunDevice` boundary without enabling system-wide routing:

```text
TunDevice
	↓ bounded read
NetworkPacket
	↓ IPv4/IPv6 boundary validation and MTU check
TunDataPlane
	↓ existing DataPlane and encrypted session
controlled multi-hop test path
```

`NetworkPacket` owns validated bytes, identifies only IPv4 or IPv6, rejects empty, malformed, unknown-version, over-limit, and over-MTU input, and does not implement an IP stack or fragmentation. `TunConfig` supplies disabled-by-default interface settings, MTU, maximum packet size, and bounded read/write queue limits.

`MockTunDevice` is the deterministic implementation used by tests. It supports packet injection, reads, writes, queue-full errors, invalid/oversized packet rejection, and clean closure. `TunDataPlane` adapts this device to the existing encrypted `DataPlane`; it does not create a new encryption or forwarding path. The three-node test sends a controlled minimal IPv4 packet from Mock TUN through Client -> Entry -> Exit and writes the returned bytes to a destination Mock TUN.

The Windows implementation uses the maintained `tun` Rust crate (`0.8.14`) and its Wintun backend. When `GHOST_TUN_ENABLED=true`, `WindowsTunDevice::open` requests the configured named L3 interface, then reads bounded packets, validates them as `NetworkPacket`, enforces MTU and maximum size, writes validated return packets, and closes through the existing `TunDevice` boundary. It does not install or download a driver. If Wintun or the named interface is unavailable, opening fails with a structured error identifying the required driver/interface; no driver functionality is faked. The current environment has not executed a real-device test because no supported driver is installed. This TUN capability is not system-wide VPN routing: Ghost Layer does not add routes, capture arbitrary traffic, change interface metrics, or install a default route. NAT, DNS interception/tunneling, UDP, proxying, fragmentation, and public Internet access remain disabled.

## Controlled Windows OS Routing

Prompt 21 adds a disabled-by-default OS route boundary without enabling full VPN mode:

```text
explicit GHOST_OS_ROUTES
	↓ non-default IpPrefix policy
Windows route manager
	↓ named Ghost Layer TUN interface only
existing TUN -> NetworkPacket -> MTU/routing -> encrypted data plane
```

`GHOST_OS_ROUTING_ENABLED` defaults to `false`. When enabled, `GHOST_OS_ROUTES` must contain one or more explicit IPv4/IPv6 prefixes such as `192.168.1.3/32`; `0.0.0.0/0`, `::/0`, malformed prefixes, and empty route sets are rejected. The client opens the configured TUN device before attempting route installation, and route installation fails closed when the TUN is unavailable.

The platform-independent `RoutingPolicy` and `RouteManager` boundaries are tested with a deterministic mock manager. The Windows implementation uses `netsh` only for explicitly configured prefixes and the configured TUN interface. It records routes successfully added by the current process, skips pre-existing matching routes, rolls back partial installation failures, and removes only its owned routes during normal shutdown. It never changes the default route, unrelated routes, interface metrics, firewall rules, proxy settings, or system-wide traffic capture. Crash recovery is limited to the ownership state available to the running process; operators should verify route state after an abnormal termination.

This is controlled route plumbing, not a production VPN. A real Windows routing test requires an installed supported Wintun driver and a narrow explicitly configured route. In the current environment, the required driver is unavailable, so the real routing test is skipped. NAT remains disabled, DNS interception remains disabled, UDP remains unimplemented, and unrestricted Internet forwarding remains disabled.

### Windows manual TUN test

Mode A is the default Mock TUN path and requires no driver. Mode B is the Windows real-device path and requires the Wintun driver/runtime appropriate to the installed `tun` crate. Verify the installation using the Wintun distribution or Windows device-management tools before starting Ghost Layer; do not modify route tables or firewall settings.

For a controlled manual check, set `GHOST_TUN_ENABLED=true`, choose `GHOST_TUN_INTERFACE_NAME`, and set `GHOST_TUN_MTU` and `GHOST_TUN_MAXIMUM_PACKET_SIZE` to matching bounded values. Start the client with its normal development relay configuration. A successful open is reported through the structured client/adapter lifecycle; a missing driver reports device initialization failure and exits without pretending that a TUN exists. Inject only a controlled IPv4/IPv6 test packet through the named interface, verify that it reaches the existing packet-validation/data-plane boundary, and verify a controlled response is written back. Shut down with the normal process signal so the adapter is closed. This procedure does not route ordinary Windows traffic and does not require Internet connectivity.

### Local two-node run

From the repository root, start the relay in one terminal:

```powershell
$env:GHOST_LISTEN_ADDRESS = "/ip4/127.0.0.1/udp/9000/quic-v1"
$env:GHOST_IDENTITY_PATH = "$PWD/relay.key"
$env:GHOST_NETWORK_ENVIRONMENT = "development"
cargo run -p ghost-layer-relay
```

Copy the relay `PeerId` from its log. In a second terminal, use that value in the `/p2p/` suffix:

```powershell
$env:GHOST_LISTEN_ADDRESS = "/ip4/127.0.0.1/udp/9001/quic-v1"
$env:GHOST_IDENTITY_PATH = "$PWD/client.key"
$env:GHOST_BOOTSTRAP_PEERS = "/ip4/127.0.0.1/udp/9000/quic-v1/p2p/<RELAY_PEER_ID>"
$env:GHOST_NETWORK_ENVIRONMENT = "development"
cargo run -p ghost-layer-client
```

The client log should show `connection established`, `peer identified`, and `ping result`. The relay log should show the corresponding connection and identification events.

## Solana Layer

Future Solana/Anchor programs will provide decentralized network registration, staking, rewards, and settlement. Programs should store and settle control-plane state only. They must not receive or persist user traffic or payload data.

## MagicBlock Layer

MagicBlock Ephemeral Rollups are planned for fast coordination of relay heartbeats, health, availability, latency, and routing/session coordination. These ephemeral state updates complement, rather than replace, durable Solana settlement and registration.

## Future Routing

The MVP direction supports future 1-hop and 2-hop routing:

- 1-hop routing: a client selects one relay for a session.
- 2-hop routing: a client selects an entry relay and a separate exit relay.

The exact production protocol, relay selection policy, metadata handling, failure behavior, and privacy properties remain future work. The current implementation supports only the controlled, local two-hop application test documented above.
