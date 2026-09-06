use anyhow::{Context, Result};
use futures::StreamExt;
use ghost_layer_network::{
    DataPlane, DataPlaneEnvelope, EncryptedChannel, HandshakeInit, NetworkEvent, NetworkNode,
    NodeConfig, NodeIdentity, SessionResponder, CHANNEL_PROTOCOL_VERSION, DATA_PLANE_KIND,
    DEFAULT_MAXIMUM_PAYLOAD_SIZE,
};
use ghost_layer_relay::{
    HealthClass, HealthThresholds, InMemoryRelayRegistry, RelayHealth, RelayHeartbeat,
    RelayMetadata, RelayMetadataRequest, RelayRegistry, RelayState, RelayStatus,
};
use std::collections::HashMap;
use std::time::SystemTime;
use tracing::{error, info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    init_logging();
    let config = NodeConfig::from_env().context("load relay configuration")?;
    let identity = NodeIdentity::load_or_generate(&config.identity_path)?;
    let peer_id = identity.peer_id();
    let mut node = NetworkNode::new(&identity, config.connection_timeout)?;
    node.listen(&config.listen_address)?;
    let mut state = RelayState::new(
        peer_id,
        config.advertised_address.clone(),
        config.software_version.clone(),
    );
    let mut registry = InMemoryRelayRegistry::default();
    let mut session_responder = SessionResponder::new(identity.keypair());
    let mut channels: HashMap<ghost_layer_network::SessionId, EncryptedChannel> = HashMap::new();
    registry.register(RelayMetadata::new(
        peer_id,
        config.protocol_version.clone(),
        config.software_version.clone(),
    ))?;
    let thresholds = HealthThresholds {
        heartbeat_freshness: config.heartbeat_freshness,
        degraded_latency: config.degraded_latency,
    };
    let mut heartbeat_timer = tokio::time::interval(config.heartbeat_interval);

    info!(%peer_id, role = "relay", "relay identity loaded");
    info!(environment = ?config.network_environment, "relay networking started");
    loop {
        tokio::select! {
            event = node.swarm.next() => {
                let Some(event) = event else { break };
                if let Some(event) = node.translate_event(event) {
                    state.apply_network_event(&event, SystemTime::now());
                    if let NetworkEvent::DiscoveryRequest { channel, payload, peer_id, .. } = event {
                        if let Ok(init) = serde_json::from_slice::<HandshakeInit>(&payload) {
                            if init.kind == "secure_session_init" {
                                if let Ok((response, session)) = session_responder.accept_init(init, peer_id, &config.protocol_version) {
                                    let session_id = session.session_id();
                                    channels.insert(session_id, EncryptedChannel::open(session, CHANNEL_PROTOCOL_VERSION, 4096, 8).map_err(|error| anyhow::anyhow!("open channel: {error}"))?);
                                    node.respond_to_discovery(channel, serde_json::to_vec(&response)?)?;
                                }
                            }
                        } else if let Ok(request) = serde_json::from_slice::<RelayMetadataRequest>(&payload) {
                            if request.kind == "metadata" && request.protocol_version == config.protocol_version {
                                let now = SystemTime::now();
                                let health = RelayHealth::evaluate(&state, thresholds, now);
                                let metadata = RelayMetadata::from_state(&state, config.protocol_version.clone(), health);
                                if let Ok(response) = serde_json::to_vec(&metadata) {
                                    node.respond_to_discovery(channel, response)?;
                                }
                            }
                        } else if let Ok(envelope) = serde_json::from_slice::<DataPlaneEnvelope>(&payload) {
                            if envelope.kind == DATA_PLANE_KIND {
                                if let Some(channel_state) = channels.get_mut(&envelope.session_id) {
                                    let mut data_plane = DataPlane::open(channel_state, DEFAULT_MAXIMUM_PAYLOAD_SIZE)
                                        .map_err(|error| anyhow::anyhow!("open data plane: {error}"))?;
                                    if let Ok(message) = data_plane.receive(&envelope) {
                                        let mut acknowledgement = b"ack: ".to_vec();
                                        acknowledgement.extend(message.payload);
                                        let response_frame = data_plane.send(&acknowledgement)
                                            .map_err(|error| anyhow::anyhow!("send data-plane acknowledgement: {error}"))?;
                                        let response = DataPlaneEnvelope { kind: DATA_PLANE_KIND.to_owned(), session_id: envelope.session_id, frame: response_frame };
                                        node.respond_to_discovery(channel, serde_json::to_vec(&response)?)?;
                                    }
                                } else {
                                    warn!("rejected channel frame for unknown session");
                                }
                            }
                        }
                    } else {
                        log_event(&event);
                    }
                }
            }
            _ = heartbeat_timer.tick() => {
                let now = SystemTime::now();
                let health = RelayHealth::evaluate(&state, thresholds, now);
                state.set_status(match health.class {
                    HealthClass::Healthy => RelayStatus::Online,
                    HealthClass::Degraded => RelayStatus::Degraded,
                    HealthClass::Unhealthy => RelayStatus::Offline,
                });
                let heartbeat = RelayHeartbeat::from_state(&mut state, now);
                log_heartbeat(&heartbeat, health);
            }
            result = tokio::signal::ctrl_c() => {
                if let Err(error) = result {
                    warn!(%error, "shutdown signal handler failed");
                }
                state.set_status(RelayStatus::ShuttingDown);
                info!(peer_id = %state.peer_id(), status = %state.status(), "relay shutting down");
                break;
            }
        }
    }
    Ok(())
}

fn init_logging() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();
}

fn log_event(event: &NetworkEvent) {
    match event {
        NetworkEvent::Listening { address } => info!(%address, "relay listening"),
        NetworkEvent::PeerConnected { peer_id } => info!(%peer_id, "connection established"),
        NetworkEvent::PeerDisconnected { peer_id } => info!(%peer_id, "connection closed"),
        NetworkEvent::PeerIdentified {
            peer_id,
            agent_version,
        } => info!(%peer_id, %agent_version, "peer identified"),
        NetworkEvent::PingResult { peer_id, result } => match result {
            Ok(latency) => info!(%peer_id, ?latency, "ping result"),
            Err(error) => warn!(%peer_id, %error, "ping failed"),
        },
        NetworkEvent::NetworkError { message } => error!(%message, "network error"),
        NetworkEvent::DiscoveryRequest { .. } | NetworkEvent::DiscoveryResponse { .. } => {}
    }
}

fn log_heartbeat(heartbeat: &RelayHeartbeat, health: RelayHealth) {
    let latency_ms = heartbeat
        .latency
        .map(|latency| latency.as_secs_f64() * 1000.0);
    info!(
        target: "relay_heartbeat",
        relay_heartbeat = true,
        peer_id = %heartbeat.peer_id,
        status = %heartbeat.status,
        uptime = ?heartbeat.uptime,
        peers = heartbeat.active_peer_count,
        latency_ms = ?latency_ms,
        health = ?health.class,
        "relay heartbeat"
    );
}
