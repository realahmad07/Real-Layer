FROM rust:1.80-bookworm AS builder
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY client/Cargo.toml client/Cargo.toml
COPY network/Cargo.toml network/Cargo.toml
COPY relay/Cargo.toml relay/Cargo.toml
COPY client client
COPY network network
COPY relay relay
RUN cargo build --release --bin ghost-layer-relay

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install --no-install-recommends --yes wget \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --home-dir /var/lib/ghost-layer --shell /usr/sbin/nologin ghost-layer
COPY --from=builder /src/target/release/ghost-layer-relay /usr/local/bin/ghost-layer-relay
COPY docker/entrypoint.sh /usr/local/bin/ghost-layer-entrypoint
RUN chmod 0555 /usr/local/bin/ghost-layer-entrypoint
RUN mkdir -p /var/lib/ghost-layer/identity && chown -R ghost-layer:ghost-layer /var/lib/ghost-layer
USER ghost-layer
WORKDIR /var/lib/ghost-layer
ENV GHOST_NETWORK_ENVIRONMENT=production \
    GHOST_IDENTITY_PATH=/var/lib/ghost-layer/identity/relay.key \
    GHOST_HEALTH_LISTEN_ADDRESS=127.0.0.1:8080 \
    GHOST_LOG_LEVEL=info
EXPOSE 7000/udp
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 CMD wget -q -O - http://127.0.0.1:8080/ready >/dev/null || exit 1
ENTRYPOINT ["/usr/local/bin/ghost-layer-entrypoint"]
