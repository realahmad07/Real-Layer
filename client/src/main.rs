use anyhow::{Context, Result};
use futures::StreamExt;
use ghost_layer_network::{
    ConfiguredPeerDiscovery, DataPlane, DataPlaneEnvelope, DataPlaneMessageType, DiscoveryService,
    EncryptedChannel, HandshakeResponse, MtuPolicy, NetworkEvent, NetworkNode, NodeConfig,
    NodeIdentity, PacketPipeline, RouteBinding, RouteManager, RoutingPolicy, SessionInitiator,
    TunDataPlane, WindowsRouteManager, CHANNEL_PROTOCOL_VERSION, DATA_PLANE_KIND,
    DEFAULT_MAXIMUM_PAYLOAD_SIZE,
};
use ghost_layer_relay::{
    CandidateRequirements, ForwardingMessage, RelayCandidate, RelayMetadataRequest, RelayRanking,
    Route, RouteSelectionPolicy, RouteSelector, FORWARDING_KIND,
};
use std::time::SystemTime;
use tracing::{error, info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    init_logging();
    let config = NodeConfig::from_env().context("load client configuration")?;
    let identity = NodeIdentity::load_or_generate(&config.identity_path)?;
    let peer_id = identity.peer_id();
    let mut node = NetworkNode::new(&identity, config.connection_timeout)?;
    node.listen(&config.listen_address)?;
    let mut tun_device = open_configured_tun(&config.tun)?;
    if tun_device.is_some() {
        info!(interface = %config.tun.interface_name, "Windows TUN device opened");
    }
    let mut route_manager = WindowsRouteManager::new();
    if config.os_routing_enabled {
        if tun_device.is_none() {
            return Err(anyhow::anyhow!(
                "OS routing requires an available Ghost Layer TUN device"
            ));
        }
        route_manager
            .install(
                &RoutingPolicy::Explicit(config.os_routes.clone()),
                &config.tun.interface_name,
            )
            .map_err(|error| anyhow::anyhow!("install Ghost Layer OS route: {error}"))?;
        info!(routes = ?config.os_routes, interface = %config.tun.interface_name, "Ghost Layer OS routes installed");
    }
    let discovery = ConfiguredPeerDiscovery::from_addresses(&config.bootstrap_peers)?;
    let requirements = CandidateRequirements {
        protocol_version: config.protocol_version.clone(),
        required_transport: config.discovery_required_transport.clone(),
        required_capabilities: config.discovery_required_capabilities.clone(),
        heartbeat_freshness: config.heartbeat_freshness,
    };
    let route_policy = RouteSelectionPolicy {
        mode: config.route_mode,
        required_transport: requirements.required_transport.clone(),
        required_capabilities: requirements.required_capabilities.clone(),
    };
    let mut candidates = Vec::new();
    let mut pending_session: Option<SessionInitiator> = None;
    let mut active_channel: Option<EncryptedChannel> = None;
    let mut active_route: Option<RouteBinding> = None;

    info!(%peer_id, role = "client", "client identity loaded");
    if discovery.known_peers().is_empty() {
        warn!("no bootstrap peer configured; client will only listen");
    } else {
        for address in discovery.known_peers() {
            info!(address = %address, "dialing configured relay");
            node.dial(&address.to_string())?;
        }
    }

    while let Some(event) = node.swarm.next().await {
        if let Some(event) = node.translate_event(event) {
            match event {
                NetworkEvent::PeerConnected {
                    peer_id: remote_peer,
                } => {
                    let request = RelayMetadataRequest {
                        kind: "metadata".to_owned(),
                        protocol_version: config.protocol_version.clone(),
                    };
                    node.request_discovery(remote_peer, serde_json::to_vec(&request)?);
                    log_event(&NetworkEvent::PeerConnected {
                        peer_id: remote_peer,
                    });
                }
                NetworkEvent::DiscoveryResponse {
                    peer_id: remote_peer,
                    payload,
                    ..
                } => {
                    if let Ok(response) = serde_json::from_slice::<HandshakeResponse>(&payload) {
                        if response.kind == "secure_session_response" {
                            if let Some(initiator) = pending_session.take() {
                                match initiator.complete(response) {
                                    Ok(session) => {
                                        info!(session_id = %session.session_id(), peer = %session.peer_id(), route = ?session.route(), "secure session established");
                                        let session_id = session.session_id();
                                        let peer = session.peer_id();
                                        let route = session.route().clone();
                                        let mut channel = EncryptedChannel::open(
                                            session,
                                            CHANNEL_PROTOCOL_VERSION,
                                            4096,
                                            8,
                                        )
                                        .map_err(|error| {
                                            anyhow::anyhow!("open encrypted channel: {error}")
                                        })?;
                                        let envelope = if let Some(device) = tun_device.as_mut() {
                                            let mut data_plane = DataPlane::open(
                                                &mut channel,
                                                DEFAULT_MAXIMUM_PAYLOAD_SIZE,
                                            )
                                            .map_err(|error| {
                                                anyhow::anyhow!("open TUN data plane: {error}")
                                            })?;
                                            let Some(packet) =
                                                device.read_packet().map_err(|error| {
                                                    anyhow::anyhow!("read TUN packet: {error}")
                                                })?
                                            else {
                                                return Err(anyhow::anyhow!(
                                                    "TUN device returned no packet for controlled bridge"
                                                ));
                                            };
                                            let mut pipeline = PacketPipeline::new(
                                                MtuPolicy::new(
                                                    config.tun.mtu,
                                                    config.tun.maximum_packet_size,
                                                )
                                                .map_err(|error| {
                                                    anyhow::anyhow!(
                                                        "configure TUN MTU policy: {error}"
                                                    )
                                                })?,
                                                &mut data_plane,
                                            );
                                            pipeline.send(packet.as_bytes().to_vec()).map_err(
                                                |error| {
                                                    anyhow::anyhow!("route TUN packet: {error}")
                                                },
                                            )?
                                        } else {
                                            let payload =
                                                if config.external_test_destination.is_some()
                                                    && matches!(route, RouteBinding::TwoHop { .. })
                                                {
                                                    b"ghost-layer-external-test".as_slice()
                                                } else {
                                                    b"hello ghost layer".as_slice()
                                                };
                                            let frame = DataPlane::open(
                                                &mut channel,
                                                DEFAULT_MAXIMUM_PAYLOAD_SIZE,
                                            )
                                            .map_err(|error| {
                                                anyhow::anyhow!("open data plane: {error}")
                                            })?
                                            .send(payload)
                                            .map_err(|error| {
                                                anyhow::anyhow!(
                                                    "encrypt data-plane payload: {error}"
                                                )
                                            })?;
                                            DataPlaneEnvelope {
                                                kind: DATA_PLANE_KIND.to_owned(),
                                                session_id,
                                                frame,
                                            }
                                        };
                                        node.request_discovery(
                                            peer,
                                            serde_json::to_vec(&envelope)?,
                                        );
                                        active_channel = Some(channel);
                                        active_route = Some(route);
                                        info!(session_id = %session_id, "encrypted channel established; sending protocol Ping");
                                    }
                                    Err(error) => {
                                        warn!(peer_id = %remote_peer, ?error, "secure session establishment failed")
                                    }
                                }
                            }
                        }
                    } else if let Ok(forwarded) =
                        serde_json::from_slice::<ForwardingMessage>(&payload)
                    {
                        if forwarded.kind == FORWARDING_KIND {
                            if let (Some(channel), Some(route)) =
                                (active_channel.as_mut(), active_route.as_ref())
                            {
                                if forwarded.client_session_id == channel.session_id()
                                    && forwarded.client_peer == peer_id.to_string()
                                    && forwarded.route == *route
                                    && forwarded.data.session_id == channel.session_id()
                                {
                                    if let Some(device) = tun_device.as_mut() {
                                        let mut data_plane =
                                            DataPlane::open(channel, DEFAULT_MAXIMUM_PAYLOAD_SIZE)
                                                .map_err(|error| {
                                                    anyhow::anyhow!(
                                                        "open TUN response data plane: {error}"
                                                    )
                                                })?;
                                        TunDataPlane::new(
                                            device.as_mut(),
                                            &mut data_plane,
                                            config.tun.maximum_packet_size,
                                            config.tun.mtu,
                                        )
                                        .map_err(|error| {
                                            anyhow::anyhow!("open TUN response bridge: {error}")
                                        })?
                                        .write_frame(&forwarded.data)
                                        .map_err(
                                            |error| anyhow::anyhow!("write packet to TUN: {error}"),
                                        )?;
                                        info!(session_id = %forwarded.client_session_id, "TUN packet returned through entry and exit");
                                        channel.close().map_err(|error| {
                                            anyhow::anyhow!("close encrypted channel: {error}")
                                        })?;
                                        break;
                                    }
                                    match DataPlane::open(channel, DEFAULT_MAXIMUM_PAYLOAD_SIZE)
                                        .and_then(|mut plane| plane.receive(&forwarded.data))
                                    {
                                        Ok(message)
                                            if message.payload == b"ghost-layer-external-test" =>
                                        {
                                            info!(session_id = %forwarded.client_session_id, "external TCP echo received through entry and exit");
                                            channel.close().map_err(|error| {
                                                anyhow::anyhow!("close encrypted channel: {error}")
                                            })?;
                                            break;
                                        }
                                        Ok(message) => {
                                            warn!(?message, "unexpected external TCP response")
                                        }
                                        Err(error) => {
                                            warn!(?error, "rejected external TCP response")
                                        }
                                    }
                                } else {
                                    warn!("rejected forwarding response binding");
                                }
                            }
                        }
                    } else if let Ok(envelope) =
                        serde_json::from_slice::<DataPlaneEnvelope>(&payload)
                    {
                        if envelope.kind == DATA_PLANE_KIND {
                            if let Some(channel) = active_channel.as_mut() {
                                match DataPlane::open(channel, DEFAULT_MAXIMUM_PAYLOAD_SIZE)
                                    .and_then(|mut plane| plane.receive(&envelope))
                                {
                                    Ok(message)
                                        if message.message_type == DataPlaneMessageType::Data
                                            && message.payload.starts_with(b"ack: ") =>
                                    {
                                        info!(session_id = %envelope.session_id, "encrypted data-plane acknowledgement received");
                                        channel.close().map_err(|error| {
                                            anyhow::anyhow!("close encrypted channel: {error}")
                                        })?;
                                        info!(session_id = %envelope.session_id, "encrypted channel closed");
                                        break;
                                    }
                                    Ok(message) => warn!(?message, "unexpected channel response"),
                                    Err(error) => {
                                        warn!(?error, "rejected encrypted channel response")
                                    }
                                }
                            }
                        }
                    } else {
                        match serde_json::from_slice(&payload) {
                            Ok(metadata) => match RelayCandidate::from_metadata(
                                &metadata,
                                &requirements,
                                SystemTime::now(),
                            ) {
                                Ok(candidate) => {
                                    info!(peer_id = %candidate.peer_id, status = %candidate.status, health = ?candidate.health, latency_ms = ?candidate.latency.map(|latency| latency.as_millis()), capabilities = ?candidate.capabilities, "discovered relay");
                                    candidates.push(candidate);
                                    let ranked =
                                        RelayRanking::rank(candidates.clone(), SystemTime::now());
                                    if let Some(selected) = ranked.first() {
                                        info!(peer_id = %selected.peer_id, "relay candidate selected");
                                    }
                                    match RouteSelector::select(
                                        peer_id,
                                        &ranked,
                                        &route_policy,
                                        SystemTime::now(),
                                    ) {
                                        Ok(route) => {
                                            log_route(&route);
                                            if pending_session.is_none() {
                                                let (first_peer, binding) = match &route {
                                                    Route::OneHop { relay } => (
                                                        relay.peer_id,
                                                        RouteBinding::OneHop {
                                                            relay: relay.peer_id,
                                                        },
                                                    ),
                                                    Route::TwoHop { entry, exit } => (
                                                        entry.peer_id,
                                                        RouteBinding::TwoHop {
                                                            entry: entry.peer_id,
                                                            exit: exit.peer_id,
                                                        },
                                                    ),
                                                };
                                                let initiator = SessionInitiator::new(
                                                    identity.keypair(),
                                                    first_peer,
                                                    binding,
                                                    config.protocol_version.clone(),
                                                )?;
                                                let init = initiator.build_init()?;
                                                node.request_discovery(
                                                    first_peer,
                                                    serde_json::to_vec(&init)?,
                                                );
                                                pending_session = Some(initiator);
                                                info!(peer = %first_peer, "establishing secure session");
                                            }
                                        }
                                        Err(error) => warn!(
                                            ?error,
                                            "no route selected from discovered relays"
                                        ),
                                    }
                                }
                                Err(error) => {
                                    warn!(peer_id = %remote_peer, ?error, "rejected relay metadata")
                                }
                            },
                            Err(error) => {
                                warn!(peer_id = %remote_peer, %error, "invalid relay metadata response")
                            }
                        }
                    }
                }
                event => log_event(&event),
            }
        }
    }
    if config.os_routing_enabled {
        route_manager
            .remove_owned()
            .map_err(|error| anyhow::anyhow!("remove Ghost Layer OS routes: {error}"))?;
    }
    if let Some(device) = tun_device.as_mut() {
        device
            .close()
            .map_err(|error| anyhow::anyhow!("close TUN device: {error}"))?;
    }
    Ok(())
}

fn open_configured_tun(
    config: &ghost_layer_network::TunConfig,
) -> Result<Option<Box<dyn ghost_layer_network::TunDevice>>> {
    if !config.enabled {
        return Ok(None);
    }
    #[cfg(windows)]
    {
        let device = ghost_layer_network::tun::windows::WindowsTunDevice::open(config.clone())
            .map_err(|error| anyhow::anyhow!("open Windows TUN device: {error}"))?;
        Ok(Some(Box::new(device)))
    }
    #[cfg(not(windows))]
    {
        let _ = config;
        Err(anyhow::anyhow!(
            "Windows TUN support is unavailable on this platform"
        ))
    }
}

fn log_route(route: &Route) {
    match route {
        Route::OneHop { relay } => {
            info!(relay = %relay.peer_id, latency_ms = ?relay.latency_ms, "selected one-hop route")
        }
        Route::TwoHop { entry, exit } => {
            info!(entry = %entry.peer_id, entry_latency_ms = ?entry.latency_ms, exit = %exit.peer_id, exit_latency_ms = ?exit.latency_ms, "selected two-hop route")
        }
    }
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
        NetworkEvent::Listening { address } => info!(%address, "client listening"),
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
