# Docker deployment

`Dockerfile` builds the relay in a Rust builder stage and runs only the release binary as the non-root `ghost-layer` user. `docker-compose.yml` demonstrates three independent local relay processes with separate identity volumes and UDP ports.

For one generic public Linux host, copy `docker/relay.env.example` to an operator-controlled `.env` file, replace `GHOST_PUBLIC_IP` with the host's public IPv4 address, then run:

```sh
docker compose --env-file docker/relay.env -f docker/docker-compose.single.yml build
docker compose --env-file docker/relay.env -f docker/docker-compose.single.yml up -d
docker compose --env-file docker/relay.env -f docker/docker-compose.single.yml ps
docker compose --env-file docker/relay.env -f docker/docker-compose.single.yml logs -f relay
```

Open only the configured UDP QUIC port, normally `7000/udp`, in the cloud firewall and host firewall. The health endpoint remains loopback-only. This single-relay mode supports client discovery, authenticated encrypted one-hop sessions, and controlled exit features when explicitly configured; two-hop forwarding requires a second independently provisioned relay.

Run from the repository root:

```sh
docker compose -f docker/docker-compose.yml up --build
```

The Compose file is a local staging test. Replace its loopback advertised addresses, provision real PeerIds, and provide full bootstrap multiaddrs before using separate hosts. See [deployment.md](../docs/deployment.md) for VPS, identity, firewall, health, and security procedures.
