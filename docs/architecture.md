# Ghost Layer Architecture

Ghost Layer is intended to provide censorship-resistant network access through independently operated relay nodes. The MVP foundation defines boundaries and shared interfaces; it does not claim anonymity, production security, or a complete VPN/tunneling implementation.

## Control Plane

The control plane manages node identity, discovery, registration, staking, health, availability, rewards, and session coordination. Relay operators remain independently responsible for their nodes. This phase implements the relay's local operational state and an in-memory registration boundary; decentralized discovery and settlement remain future work.

## Data Plane

The future data plane will carry user traffic through selected relay nodes over encrypted transport. User traffic and payload data must remain off-chain. Solana and MagicBlock must never be used as a data path for user traffic.

The repository intentionally does not implement packet forwarding, VPN behavior, tunneling, traffic obfuscation, or privacy-sensitive cryptography in this phase.

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

The current mechanism is configured/local libp2p discovery. It does not implement a DHT, Solana registry, MagicBlock coordination, cryptographic attestation, route selection between multiple hops, VPN tunneling, or traffic forwarding. A future architecture may layer a Solana registry and MagicBlock coordination above discovery, but those systems must produce relay candidates from control-plane data only and must never carry user traffic.

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

**Route selection is metadata-only in this stage. User traffic forwarding will be implemented in a later stage after route selection and session establishment are validated.**

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
controlled Ping/Pong or SessionData messages
```

`EncryptedChannel` owns the associated SessionId, authenticated peer, route context, lifecycle state, independent send/receive sequences, protocol version, maximum frame size, and bounded outbound queue. It consumes the already-established `SecureSession`; it does not derive another key set.

Each frame contains a length-prefixed serialized header and ciphertext. The header includes protocol version, SessionId, message type, sequence number, and ciphertext length. The header with its length field is bounded before parsing, and ciphertext length must match the remaining frame exactly. The supported message types are Ping, Pong, SessionData, and Close. SessionData is limited to controlled protocol tests and is not connected to operating-system traffic.

The header metadata is authenticated as AEAD associated data. `SecureSession` supplies the ChaCha20-Poly1305 key and its monotonic nonce construction; the channel maintains separate strictly increasing directional sequence numbers. Receive sequence numbers must arrive exactly in order, duplicates are rejected as replay, gaps are rejected, and the channel cannot wrap the sequence counter. A channel never accepts a frame for another SessionId or after closure.

Outbound buffering is bounded. A caller receives `Backpressure` when the configured pending-message capacity is full rather than allowing unbounded memory growth. Malformed, oversized, unsupported, unauthenticated, replayed, and incorrectly sequenced frames are rejected without exposing plaintext.

The current client/relay flow opens this channel after session establishment, exchanges one authenticated Ping/Pong, and closes. For a two-hop route, the channel preserves the Entry/Exit route binding in the underlying session context, but Entry-to-Exit forwarding and layered/onion forwarding are not implemented.

Encrypted session channels are currently used for controlled protocol messages only. Real user traffic forwarding, VPN tunneling, TUN/TAP integration, and Entry-to-Exit forwarding are intentionally deferred.

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

The exact protocol, relay selection policy, metadata handling, failure behavior, and privacy properties remain TODOs. No routing implementation is included in this foundation.
