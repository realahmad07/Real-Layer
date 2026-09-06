use anyhow::{Context, Result};
use futures::StreamExt;
use ghost_layer_network::{
    DataPlane, DataPlaneEnvelope, DiscoveryRequestId, DiscoveryResponseChannel, EncryptedChannel,
    HandshakeInit, HandshakeResponse, NetworkEvent, NetworkNode, NodeConfig, NodeIdentity, PeerId,
    RouteBinding, SessionId, SessionInitiator, SessionResponder, CHANNEL_PROTOCOL_VERSION,
    DATA_PLANE_KIND, DEFAULT_MAXIMUM_PAYLOAD_SIZE,
};
use ghost_layer_relay::{
    DestinationPolicy, ExitForwardingBinding, ExitNetworkAdapter, ExitPacketHandler,
    ForwardingContext, ForwardingDirection, ForwardingMessage, HealthClass, HealthThresholds,
    InMemoryRelayRegistry, RelayHealth, RelayHeartbeat, RelayMetadata, RelayMetadataRequest,
    RelayRegistry, RelayState, RelayStatus, TcpAdapterConfig, TcpExitNetworkAdapter,
    FORWARDING_KIND,
};
use std::collections::HashMap;
use std::time::Duration;
use std::time::SystemTime;
use tracing::{error, info, warn};

struct PendingRelayHandshake {
    initiator: SessionInitiator,
    client_session_id: SessionId,
    client_peer: PeerId,
    route: RouteBinding,
    client_response: HandshakeResponse,
    client_response_channel: Option<DiscoveryResponseChannel>,
}

struct ClientForwarding {
    client_peer: PeerId,
    context: ForwardingContext,
    relay_session_id: SessionId,
}

struct PendingClientForward {
    client_session_id: SessionId,
    response_channel: DiscoveryResponseChannel,
}

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
    let mut pending_relay_handshakes: HashMap<SessionId, PendingRelayHandshake> = HashMap::new();
    let mut client_forwarding: HashMap<SessionId, ClientForwarding> = HashMap::new();
    let mut pending_client_forwards: HashMap<DiscoveryRequestId, PendingClientForward> =
        HashMap::new();
    let mut exit_forwarding: HashMap<SessionId, ForwardingContext> = HashMap::new();
    let mut exit_handler = ExitPacketHandler::open(
        config.tun.maximum_packet_size,
        config.tun.mtu,
        config.tun.write_buffer_limit,
    )
    .map_err(|error| anyhow::anyhow!("open exit packet handler: {error}"))?;
    let external_destination = config.external_test_destination.clone();
    let destination_policy = if external_destination.is_some() {
        Some(
            DestinationPolicy::from_strings(&config.allowed_exit_destinations)
                .map_err(|error| anyhow::anyhow!("load exit destination policy: {error}"))?,
        )
    } else {
        None
    };
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
    for address in &config.bootstrap_peers {
        node.dial(address)?;
        info!(address, "dialing configured relay peer");
    }
    loop {
        tokio::select! {
            event = node.swarm.next() => {
                let Some(event) = event else { break };
                if let Some(event) = node.translate_event(event) {
                    state.apply_network_event(&event, SystemTime::now());
                    if let NetworkEvent::DiscoveryRequest { channel, payload, peer_id, .. } = event {
                        if let Ok(init) = serde_json::from_slice::<HandshakeInit>(&payload) {
                            if init.kind == "secure_session_init" {
                                let client_session_id = init.session_id;
                                let route = init.route_context.clone();
                                let is_entry = matches!(
                                    &route,
                                    RouteBinding::TwoHop { entry, .. } if *entry == peer_id || *entry == identity.peer_id()
                                ) && matches!(&route, RouteBinding::TwoHop { entry, .. } if *entry == identity.peer_id());
                                let client_peer = peer_id;
                                if let Ok((response, session)) = session_responder.accept_init(init, peer_id, &config.protocol_version) {
                                    let session_id = session.session_id();
                                    channels.insert(session_id, EncryptedChannel::open(session, CHANNEL_PROTOCOL_VERSION, 4096, 8).map_err(|error| anyhow::anyhow!("open channel: {error}"))?);
                                    if is_entry {
                                        if let RouteBinding::TwoHop { exit, .. } = route.clone() {
                                            let relay_route = RouteBinding::OneHop { relay: exit };
                                            let initiator = SessionInitiator::new(
                                                identity.keypair(),
                                                exit,
                                                relay_route,
                                                config.protocol_version.clone(),
                                            )?;
                                            let relay_session_id = initiator.session_id();
                                            let relay_init = initiator.build_init()?;
                                            pending_relay_handshakes.insert(
                                                relay_session_id,
                                                PendingRelayHandshake {
                                                    initiator,
                                                    client_session_id,
                                                    client_peer,
                                                    route,
                                                    client_response: response,
                                                    client_response_channel: Some(channel),
                                                },
                                            );
                                            node.request_discovery(exit, serde_json::to_vec(&relay_init)?);
                                            info!(%client_session_id, %exit, "entry establishing secure session with exit");
                                        }
                                    } else {
                                        node.respond_to_discovery(channel, serde_json::to_vec(&response)?)?;
                                    }
                                }
                            }
                        } else if let Ok(request) = serde_json::from_slice::<RelayMetadataRequest>(&payload) {
                            if request.kind == "metadata" && request.protocol_version == config.protocol_version {
                                let now = SystemTime::now();
                                let health = RelayHealth::evaluate(&state, thresholds, now);
                                let metadata = RelayMetadata::from_state(&state, config.protocol_version.clone(), health);
                                node.respond_to_discovery(channel, serde_json::to_vec(&metadata)?)?;
                            }
                        } else if let Ok(forwarded) = serde_json::from_slice::<ForwardingMessage>(&payload) {
                            if forwarded.kind == FORWARDING_KIND {
                                if !matches!(&forwarded.route, RouteBinding::TwoHop { exit, .. } if *exit == identity.peer_id()) {
                                    warn!(%peer_id, "rejected forwarding message for a different exit");
                                } else if forwarded.client_peer.parse::<PeerId>().ok().is_none() {
                                    warn!("rejected forwarding message with malformed client peer");
                                } else {
                                    let client_peer = forwarded.client_peer.parse::<PeerId>().expect("validated client peer");
                                    if let std::collections::hash_map::Entry::Vacant(vacant_entry) =
                                        exit_forwarding.entry(forwarded.forwarding_id)
                                    {
                                        let RouteBinding::TwoHop {
                                            entry: route_entry,
                                            exit,
                                        } = forwarded.route.clone() else { unreachable!() };
                                        let mut context = ForwardingContext::with_forwarding_id(
                                            forwarded.forwarding_id,
                                            forwarded.client_session_id,
                                            client_peer,
                                            route_entry,
                                            exit,
                                            forwarded.route.clone(),
                                        )?;
                                        context.transition_connecting()?;
                                        context.transition_established()?;
                                        context.bind_relay_session(forwarded.data.session_id)?;
                                        vacant_entry.insert(context);
                                    }
                                    let context = exit_forwarding.get_mut(&forwarded.forwarding_id).expect("exit forwarding context");
                                    context.validate_message(&forwarded).map_err(|error| anyhow::anyhow!("validate forwarding message: {error}"))?;
                                    context.validate_entry(peer_id).map_err(|error| anyhow::anyhow!("validate entry peer: {error}"))?;
                                    context.validate_relay_session(forwarded.data.session_id).map_err(|error| anyhow::anyhow!("validate relay session: {error}"))?;
                                    context.begin_forwarding(forwarded.forwarding_id, ForwardingDirection::ClientToExit).map_err(|error| anyhow::anyhow!("begin exit forwarding: {error}"))?;
                                    let incoming = {
                                        let channel_state = channels.get_mut(&forwarded.data.session_id).ok_or_else(|| anyhow::anyhow!("unknown exit relay session"))?;
                                        DataPlane::open(channel_state, DEFAULT_MAXIMUM_PAYLOAD_SIZE)
                                            .map_err(|error| anyhow::anyhow!("open exit data plane: {error}"))?
                                            .receive(&forwarded.data)
                                            .map_err(|error| anyhow::anyhow!("decrypt exit data: {error}"))?
                                    };
                                    let destination = external_destination.as_deref().ok_or_else(|| anyhow::anyhow!("external exit destination is not configured"))?;
                                    let policy = destination_policy.as_ref().expect("destination policy for configured destination");
                                    let mut adapter = TcpExitNetworkAdapter::connect(
                                        destination,
                                        policy,
                                        TcpAdapterConfig {
                                            maximum_request_size: config.tun.maximum_packet_size,
                                            maximum_response_size: config.tun.maximum_packet_size,
                                            connection_timeout: Duration::from_secs(3),
                                            read_timeout: Duration::from_secs(3),
                                            write_timeout: Duration::from_secs(3),
                                        },
                                    ).map_err(|error| anyhow::anyhow!("connect exit adapter: {error}"))?;
                                    let response_payload = exit_handler
                                        .handle_bound_with_adapter_payload(
                                            forwarded.forwarding_id,
                                            &incoming.payload,
                                            context,
                                            ExitForwardingBinding {
                                                relay_session_id: forwarded.data.session_id,
                                                entry_peer: peer_id,
                                                exit_peer: identity.peer_id(),
                                            },
                                            &mut adapter,
                                        )
                                        .map_err(|error| anyhow::anyhow!("handle exit payload: {error}"))?;
                                    adapter.close().map_err(|error| anyhow::anyhow!("close exit adapter: {error}"))?;
                                    let response_frame = {
                                        let channel_state = channels.get_mut(&forwarded.data.session_id).expect("exit relay session");
                                        DataPlane::open(channel_state, DEFAULT_MAXIMUM_PAYLOAD_SIZE)
                                            .map_err(|error| anyhow::anyhow!("open exit response data plane: {error}"))?
                                            .send(&response_payload)
                                            .map_err(|error| anyhow::anyhow!("encrypt exit response: {error}"))?
                                    };
                                    let response = ForwardingMessage {
                                        kind: FORWARDING_KIND.to_owned(),
                                        forwarding_id: forwarded.forwarding_id,
                                        client_session_id: forwarded.client_session_id,
                                        client_peer: forwarded.client_peer,
                                        route: forwarded.route,
                                        data: DataPlaneEnvelope { kind: DATA_PLANE_KIND.to_owned(), session_id: forwarded.data.session_id, frame: response_frame },
                                    };
                                    node.respond_to_discovery(channel, serde_json::to_vec(&response)?)?;
                                    info!(forwarding_id = %response.forwarding_id, destination, "exit returned external TCP response");
                                }
                            }
                        } else if let Ok(envelope) = serde_json::from_slice::<DataPlaneEnvelope>(&payload) {
                            if envelope.kind == DATA_PLANE_KIND {
                                if client_forwarding.contains_key(&envelope.session_id) {
                                    let incoming_payload = {
                                        let channel_state = channels.get_mut(&envelope.session_id).ok_or_else(|| anyhow::anyhow!("missing client channel"))?;
                                        DataPlane::open(channel_state, DEFAULT_MAXIMUM_PAYLOAD_SIZE)
                                            .map_err(|error| anyhow::anyhow!("open client data plane: {error}"))?
                                            .receive(&envelope)
                                            .map_err(|error| anyhow::anyhow!("decrypt client data: {error}"))?
                                            .payload
                                    };
                                    let forwarding = client_forwarding.get_mut(&envelope.session_id).expect("client forwarding state");
                                    if forwarding.client_peer != peer_id {
                                        warn!(%peer_id, "rejected client data from an unbound peer");
                                    } else {
                                        forwarding.context.begin_forwarding(forwarding.context.forwarding_id(), ForwardingDirection::ClientToExit)
                                            .map_err(|error| anyhow::anyhow!("begin client forwarding: {error}"))?;
                                        let relay_frame = {
                                            let channel_state = channels.get_mut(&forwarding.relay_session_id).ok_or_else(|| anyhow::anyhow!("missing relay channel"))?;
                                            DataPlane::open(channel_state, DEFAULT_MAXIMUM_PAYLOAD_SIZE)
                                                .map_err(|error| anyhow::anyhow!("open relay data plane: {error}"))?
                                                .send(&incoming_payload)
                                                .map_err(|error| anyhow::anyhow!("encrypt relay data: {error}"))?
                                        };
                                        let forwarded = ForwardingMessage {
                                            kind: FORWARDING_KIND.to_owned(),
                                            forwarding_id: forwarding.context.forwarding_id(),
                                            client_session_id: forwarding.context.client_session_id(),
                                            client_peer: forwarding.client_peer.to_string(),
                                            route: forwarding.context.route().clone(),
                                            data: DataPlaneEnvelope { kind: DATA_PLANE_KIND.to_owned(), session_id: forwarding.relay_session_id, frame: relay_frame },
                                        };
                                        let outbound_request = node.request_discovery(
                                            forwarding.context.exit_peer(),
                                            serde_json::to_vec(&forwarded)?,
                                        );
                                        pending_client_forwards.insert(
                                            outbound_request,
                                            PendingClientForward {
                                                client_session_id: envelope.session_id,
                                                response_channel: channel,
                                            },
                                        );
                                        info!(forwarding_id = %forwarded.forwarding_id, "entry forwarded encrypted data to exit");
                                    }
                                } else if let Some(channel_state) = channels.get_mut(&envelope.session_id) {
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
                    } else if let NetworkEvent::DiscoveryResponse { peer_id, request_id, payload } = event {
                        if let Ok(response) = serde_json::from_slice::<HandshakeResponse>(&payload) {
                            if let Some(pending) = pending_relay_handshakes.remove(&response.session_id) {
                                if peer_id != pending.route.peers().into_iter().last().expect("relay route peer") {
                                    warn!(%peer_id, "rejected relay handshake response from unexpected exit");
                                } else {
                                    let session = pending.initiator.complete(response)?;
                                    let relay_session_id = session.session_id();
                                    channels.insert(relay_session_id, EncryptedChannel::open(session, CHANNEL_PROTOCOL_VERSION, 4096, 8).map_err(|error| anyhow::anyhow!("open relay channel: {error}"))?);
                                    let RouteBinding::TwoHop { entry, exit } = pending.route.clone() else { unreachable!() };
                                    let mut context = ForwardingContext::new(pending.client_session_id, pending.client_peer, entry, exit, pending.route)?;
                                    context.transition_connecting()?;
                                    context.transition_established()?;
                                    context.bind_relay_session(relay_session_id)?;
                                    client_forwarding.insert(pending.client_session_id, ClientForwarding { client_peer: pending.client_peer, context, relay_session_id });
                                    if let Some(client_response_channel) = pending.client_response_channel {
                                        node.respond_to_discovery(client_response_channel, serde_json::to_vec(&pending.client_response)?)?;
                                    }
                                    info!(%relay_session_id, %pending.client_session_id, "entry-to-exit secure session established");
                                }
                            }
                        } else if let Some(pending) = pending_client_forwards.remove(&request_id) {
                            let response: ForwardingMessage = serde_json::from_slice(&payload).map_err(|error| anyhow::anyhow!("decode exit forwarding response: {error}"))?;
                            let forwarding = client_forwarding.get_mut(&pending.client_session_id).ok_or_else(|| anyhow::anyhow!("missing client forwarding context"))?;
                            forwarding.context.validate_message(&response).map_err(|error| anyhow::anyhow!("validate exit response: {error}"))?;
                            forwarding.context.validate_exit(peer_id).map_err(|error| anyhow::anyhow!("validate exit response peer: {error}"))?;
                            forwarding.context.validate_relay_session(response.data.session_id).map_err(|error| anyhow::anyhow!("validate exit response session: {error}"))?;
                            forwarding.context.begin_forwarding(response.forwarding_id, ForwardingDirection::ExitToClient).map_err(|error| anyhow::anyhow!("begin return forwarding: {error}"))?;
                            let incoming = {
                                let channel_state = channels.get_mut(&response.data.session_id).ok_or_else(|| anyhow::anyhow!("missing entry relay channel"))?;
                                DataPlane::open(channel_state, DEFAULT_MAXIMUM_PAYLOAD_SIZE)
                                    .map_err(|error| anyhow::anyhow!("open entry response data plane: {error}"))?
                                    .receive(&response.data)
                                    .map_err(|error| anyhow::anyhow!("decrypt exit response: {error}"))?
                            };
                            let client_frame = {
                                let channel_state = channels.get_mut(&pending.client_session_id).ok_or_else(|| anyhow::anyhow!("missing client channel"))?;
                                DataPlane::open(channel_state, DEFAULT_MAXIMUM_PAYLOAD_SIZE)
                                    .map_err(|error| anyhow::anyhow!("open client response data plane: {error}"))?
                                    .send(&incoming.payload)
                                    .map_err(|error| anyhow::anyhow!("encrypt client response: {error}"))?
                            };
                            let client_response = ForwardingMessage {
                                kind: FORWARDING_KIND.to_owned(),
                                forwarding_id: response.forwarding_id,
                                client_session_id: pending.client_session_id,
                                client_peer: forwarding.client_peer.to_string(),
                                route: response.route,
                                data: DataPlaneEnvelope { kind: DATA_PLANE_KIND.to_owned(), session_id: pending.client_session_id, frame: client_frame },
                            };
                            node.respond_to_discovery(pending.response_channel, serde_json::to_vec(&client_response)?)?;
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
