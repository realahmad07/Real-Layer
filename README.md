# Ghost Layer

Ghost Layer is a decentralized privacy-routing network project. This repository contains the initial production-oriented foundation for Rust clients, relay nodes, shared networking primitives, and future Solana and MagicBlock coordination components.

This repository does not yet implement a VPN, tunneling system, anonymity guarantees, or production security. User traffic must remain off-chain; future on-chain components coordinate network state, not payloads.

## Workspace

- `client/`: future user-side Rust client
- `relay/`: future decentralized Rust relay node
- `network/`: shared peer, configuration, health, and transport abstractions
- `programs/`: future Solana/Anchor programs
- `coordination/`: future MagicBlock coordination components
- `dashboard/`: future React and TypeScript dashboard
- `tests/`: integration and end-to-end test space
- `docs/`: architecture and design documentation

## Development

The workspace uses stable Rust. After installing Rust, run:

```text
cargo check
cargo test
```

Configuration is supplied through environment variables. See `.env.example` for the non-secret shape of the configuration.
