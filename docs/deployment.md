# Ghost Layer Deployment

Ghost Layer is currently deployed as a controlled networking system. Blockchain coordination, staking, rewards, reputation, and DePIN settlement are intentionally not part of this deployment stage.

## Deployment levels

- **Local multi-container:** `docker/docker-compose.yml` runs three independent relay processes with separate persistent identity volumes. This is a reproducible local test, not a production deployment.
- **LAN multi-machine:** run the same relay binary on separate hosts, use each host's reachable QUIC multiaddr as `GHOST_ADVERTISED_ADDRESS`, and provision each identity independently.
- **VPS:** use the systemd example at `docker/ghost-layer-relay.service.example` with an environment file owned by the relay user.
- **Internet:** requires real hosts, firewall rules, stable advertised addresses, and an authorized test destination. It is not claimed by this repository unless executed and recorded.

No specific cloud provider is required. Any free-tier or low-cost Linux VM/container host that provides a stable public IPv4 address and inbound UDP is suitable. Do not assume country, region, or provider-specific behavior.

## Identity provisioning

Create one identity directory per relay and protect it from other users:

```sh
sudo install -d -o ghost-layer -g ghost-layer -m 0700 /var/lib/ghost-layer/identity
```

Set `GHOST_IDENTITY_PATH` to a file in that directory. The first start creates an Ed25519 identity; later starts load the same file and therefore preserve the PeerId. An invalid or unreadable identity fails startup. Back up the identity file securely because replacing it creates a different relay identity. Never commit it, log it, or bake it into an image.

## Configuration

Required operational values are `GHOST_LISTEN_ADDRESS`, `GHOST_ADVERTISED_ADDRESS`, and `GHOST_IDENTITY_PATH`. Use a QUIC multiaddr such as `/ip4/0.0.0.0/udp/7000/quic-v1` for binding and a separately reachable multiaddr for advertisement. Production configuration rejects loopback bind and advertisement addresses.

`GHOST_RELAY_ROLE` accepts `entry`, `exit`, or `both`. `both` is the default and preserves the current behavior. An entry-only relay does not advertise or accept exit forwarding; an exit-only relay does not initiate entry-to-exit sessions.

Set `GHOST_BOOTSTRAP_PEERS` to comma-separated full multiaddrs including `/p2p/<PeerId>`. Malformed addresses fail startup; unreachable peers are connection failures and do not stop an otherwise valid relay. Use bounded connection and heartbeat timeouts. `GHOST_HEALTH_LISTEN_ADDRESS` is optional and must be a loopback `IP:PORT`; it exposes `/live`, `/ready`, and `/health` for local supervision.

Exit operation is deny-by-default. Every TCP, UDP, or DNS destination must be an exact `IP:PORT` entry in `GHOST_ALLOWED_EXIT_DESTINATIONS`. Hostnames, wildcards, unspecified addresses, and implicit resolver fallback are rejected. Enable `GHOST_EXTERNAL_TEST_DESTINATION`, `GHOST_UDP_ENABLED`, or `GHOST_DNS_ENABLED` only with a matching controlled destination and the required NAT/DNS settings.

Resource limits include peers, sessions, flows, queued packets, exit connections, packet sizes, and session/flow timeouts. Rejections are controlled and do not open an unrestricted proxy.

## Docker Compose

From the repository root:

```sh
docker compose -f docker/docker-compose.yml up --build
```

The Compose example is a local staging topology. Relay A, B, and C use distinct UDP ports and distinct named identity volumes. The sample advertises loopback addresses intentionally and must not be copied to a public deployment. Bootstrap peer IDs are not hard-coded because identities are provisioned at first start; for a real multi-hop test, inspect each persisted PeerId and set full `/p2p/<PeerId>` bootstrap addresses in an operator-owned environment file.

The image is multi-stage, contains only the release relay binary and runtime files, runs as `ghost-layer`, and keeps identity state on a mounted volume. It does not contain source-controlled secrets.

### Single public relay

On the host, obtain its public address from the provider console or, where permitted, with `curl -4 -fsS https://api.ipify.org`. Then:

```sh
cp docker/relay.env.example docker/relay.env
# Edit docker/relay.env and replace GHOST_PUBLIC_IP with the real address.
docker compose --env-file docker/relay.env -f docker/docker-compose.single.yml build
docker compose --env-file docker/relay.env -f docker/docker-compose.single.yml up -d
docker compose --env-file docker/relay.env -f docker/docker-compose.single.yml ps
docker compose --env-file docker/relay.env -f docker/docker-compose.single.yml logs relay
```

Required cloud firewall rule: allow inbound UDP `GHOST_QUIC_PORT` (default `7000`) to the host. Do not expose TCP `8080`; the readiness endpoint is bound to container loopback. `restart: unless-stopped` restarts the relay after process failure, while the mounted identity volume preserves its PeerId.

## Linux VPS

1. Install a supported Rust-built release binary, or build it with `cargo build --release --bin ghost-layer-relay` on a build host.
2. Create a dedicated `ghost-layer` user and `/var/lib/ghost-layer/identity` with mode `0700`.
3. Install the binary as `/usr/local/bin/ghost-layer-relay` and the example unit as `/etc/systemd/system/ghost-layer-relay.service`.
4. Create `/etc/ghost-layer/relay.env`, owned by `root:ghost-layer` with mode `0640`, containing placeholders replaced by the operator.
5. Allow only the configured UDP QUIC port in the firewall. Do not expose the loopback health endpoint.
6. Run `systemctl daemon-reload` and `systemctl enable --now ghost-layer-relay`.
7. Verify `journalctl -u ghost-layer-relay`, the logged PeerId, `curl http://127.0.0.1:<health-port>/live`, `/ready`, and `/health`, then verify bootstrap connectivity and discovery metadata.
8. Stop with `systemctl stop ghost-layer-relay` and confirm a clean draining shutdown.

**REAL VPS DEPLOYMENT: NOT EXECUTED - NO DEPLOYMENT HOST CONFIGURED.**

## Client connection and verification

After the relay first starts, read its stable `peer_id` from logs:

```sh
docker compose --env-file docker/relay.env -f docker/docker-compose.single.yml logs relay | grep 'relay identity loaded'
```

On the client machine, set `GHOST_BOOTSTRAP_PEERS` to the advertised address with that PeerId, for example `/ip4/<PUBLIC_IP>/udp/7000/quic-v1/p2p/<PEER_ID>`, set `GHOST_ROUTE_MODE=one-hop`, and run the existing client binary:

```sh
cargo build --release --bin ghost-layer-client
GHOST_LISTEN_ADDRESS=/ip4/0.0.0.0/udp/0/quic-v1 \
GHOST_ADVERTISED_ADDRESS= \
GHOST_BOOTSTRAP_PEERS=/ip4/<PUBLIC_IP>/udp/7000/quic-v1/p2p/<PEER_ID> \
GHOST_ROUTE_MODE=one-hop \
./target/release/ghost-layer-client
```

Verify `connection established`, `discovered relay`, `secure session established`, and `protocol response received` in the client log. These prove transport connectivity, discovery metadata, authenticated encryption, and the return path. For TCP, configure one exact controlled `IP:PORT` in both `GHOST_ALLOWED_EXIT_DESTINATIONS` and `GHOST_EXTERNAL_TEST_DESTINATION`; for UDP, also enable NAT and `GHOST_UDP_ENABLED`; for DNS, configure one exact `GHOST_DNS_SERVER` and enable `GHOST_DNS_ENABLED`. No feature is enabled by default and no arbitrary Internet destination is accepted.

Verify health locally on the host/container with `wget -qO- http://127.0.0.1:8080/live`, `/ready`, and `/health`. Restart with `docker compose ... restart relay`, confirm the same PeerId in logs, and verify readiness returns after the process restarts. Stop with `docker compose ... stop relay` and confirm the client fails closed rather than bypassing the relay.

## Controlled validation

Use a test server controlled by the operator, never an arbitrary public destination. Configure its exact `IP:PORT` in the exit allowlist and run the client through a real Entry and Exit relay. Record the Entry/Exit PeerIds, route binding, exact response, and the absence of a direct Exit-to-Client path. Repeat after restarting each relay and with the bootstrap peer unavailable.

TCP is implemented for the controlled allowlisted exchange. UDP is implemented only with the bounded NAT path and an exact allowlist. DNS is implemented only against an explicitly configured IP endpoint and must not fall back to a local resolver. Windows TUN and OS routing remain opt-in; no default route, wildcard route, firewall rule, NAT setup, or DNS interception is installed automatically.

**REAL WINDOWS TUN TEST: SKIPPED - SUPPORTED TUN DRIVER NOT AVAILABLE.**

## Security and failure handling

Keep identity files private, run as non-root, restrict inbound ports, avoid public management endpoints, and treat environment files as secret-bearing configuration. Logs contain operational metadata only; they must not contain private keys, session keys, credentials, DNS contents, or payloads. Back up identity files securely and test recovery before replacing a relay.

Relay restart, peer timeout, destination rejection, malformed messages, and resource exhaustion should fail closed, clean sessions/flows, and avoid process panics. The current relay loop is the operational boundary; production operators should monitor process exit, systemd restart events, health responses, and structured logs.
