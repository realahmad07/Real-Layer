# Ghost Layer

Ghost Layer is a decentralized privacy-routing network project. This repository contains the initial production-oriented foundation for Rust clients, relay nodes, shared networking primitives, and future Solana and MagicBlock coordination components.

This repository does not claim anonymity or unrestricted Internet access. The current deployment supports controlled, encrypted application-level forwarding and exact destination allowlists. User traffic must remain off-chain; future on-chain components coordinate network state, not payloads.

Ghost Layer is currently deployed as a controlled networking system. Blockchain coordination, staking, rewards, reputation, and DePIN settlement are intentionally not part of this deployment stage.

## Workspace

- `client/`: future user-side Rust client
- `relay/`: future decentralized Rust relay node
- `network/`: shared peer, configuration, health, and transport abstractions
- `programs/`: future Solana/Anchor programs
- `coordination/`: future MagicBlock coordination components
- `dashboard/`: future React and TypeScript dashboard
- `tests/`: integration and end-to-end test space
- `docs/`: architecture and design documentation
- `docs/deployment.md`: Docker, LAN, VPS, identity, health, and security operations
- `docker/`: multi-stage relay image, Compose topology, and systemd example

## Development

The workspace uses stable Rust. After installing Rust, run:

```text
cargo check
cargo test
```

Configuration is supplied through environment variables. See `.env.example` for the non-secret shape of the configuration.

For deployment procedures and explicit limitations, see [docs/deployment.md](docs/deployment.md). A local Compose run is not evidence of a VPS or Internet deployment.
