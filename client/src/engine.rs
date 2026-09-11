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
    CandidateRequirements, ForwardedPacket, ForwardedProtocol, ForwardingMessage, RelayCandidate,
    RelayMetadataRequest, RelayRanking, Route, RouteSelectionPolicy, RouteSelector,
    FORWARDING_KIND,
};
use std::net::SocketAddr;
use std::time::SystemTime;
use tracing::{error, info, warn};

pub async fn run_client(mut tun_rx: Option<tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>>, tun_tx: Option<tokio::sync::mpsc::UnboundedSender<Vec<u8>>>) -> Result<()> {
    init_logging();
    let config = NodeConfig::from_env().context("load client configuration")?;
    let identity = NodeIdentity::load_or_generate(&config.identity_path)?;
    let peer_id = identity.peer_id();
    let mut node = NetworkNode::new(&identity, config.connection_timeout)?;
    node.listen(&config.listen_address)?;
    // os routing block removed
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


    let mut exit_peer: Option<ghost_layer_network::PeerId> = None;
    let mut current_session_id: Option<ghost_layer_network::SessionId> = None;

    loop {
        tokio::select! {
            Some(packet) = async { if let Some(rx) = tun_rx.as_mut() { rx.recv().await } else { futures::future::pending().await } } => {
                if let (Some(channel), Some(route), Some(exit)) = (active_channel.as_mut(), active_route.as_ref(), exit_peer.as_ref()) {
                    let session_id = current_session_id.unwrap();
                    let packet_length = packet.len();
                    let envelope = match DataPlane::open(channel, DEFAULT_MAXIMUM_PAYLOAD_SIZE) {
                        Ok(mut data_plane) => match MtuPolicy::new(config.tun.mtu, config.tun.maximum_packet_size) {
                            Ok(mtu) => match PacketPipeline::new(mtu, &mut data_plane).send(packet.clone()) {
                                Ok(envelope) => envelope,
                                Err(error) => {
                                    warn!(?error, "rejected Android TUN packet before relay");
                                    continue;
                                }
                            },
                            Err(error) => {
                                warn!(?error, "invalid Android TUN packet policy");
                                continue;
                            }
                        },
                        Err(error) => {
                            warn!(?error, "unable to open data plane for Android TUN packet");
                            continue;
                        }
                    };
                    {
                            let envelope = DataPlaneEnvelope {
                                kind: envelope.kind,
                                session_id,
                                frame: envelope.frame,
                            };
                            let message = ForwardingMessage {
                                kind: FORWARDING_KIND.to_owned(),
                                forwarding_id: session_id,
                                client_session_id: session_id,
                                client_peer: peer_id.to_string(),
                                route: route.clone(),
                                data: envelope,
                                packet: Some(ForwardedPacket {
                                    protocol: ForwardedProtocol::Ip,
                                    source: None,
                                    destination: "0.0.0.0:0".parse().unwrap(),
                                    flow_id: session_id,
                                    session_id,
                                    route_id: session_id,
                                    sequence: 1,
                                    payload: packet,
                                }),
                            };
                            if let Ok(payload) = serde_json::to_vec(&message) {
                                println!("VPN_PACKET_TX_TO_RELAY length={}", packet_length);
                                node.request_discovery(*exit, payload);
                            }
                    }
                }
            }
            Some(event) = node.swarm.next() => {
                if let Some(event) = node.translate_event(event) {
                    match event {
                        NetworkEvent::PeerConnected { peer_id: remote_peer } => {
                            let request = RelayMetadataRequest {
                                kind: "metadata".to_owned(),
                                protocol_version: config.protocol_version.clone(),
                            };
                            if let Ok(payload) = serde_json::to_vec(&request) {
                                node.request_discovery(remote_peer, payload);
                            }
                            log_event(&NetworkEvent::PeerConnected { peer_id: remote_peer });
                        }
                        NetworkEvent::DiscoveryResponse { peer_id: remote_peer, payload, .. } => {
                            if let Ok(response) = serde_json::from_slice::<HandshakeResponse>(&payload) {
                                if response.kind == "secure_session_response" {
                                    if let Some(initiator) = pending_session.take() {
                                        if let Ok(session) = initiator.complete(response) {
                                            current_session_id = Some(session.session_id());
                                            info!(session_id = %session.session_id(), peer = %session.peer_id(), route = ?session.route(), "secure session established");
                                            let route = session.route().clone();
                                            if let RouteBinding::TwoHop { exit, .. } = route {
                                                exit_peer = Some(exit);
                                            }
                                            active_route = Some(route);
                                            if let Ok(channel) = EncryptedChannel::open(session, CHANNEL_PROTOCOL_VERSION, 4096, 8) {
                                                active_channel = Some(channel);
                                            }
                                        }
                                    }
                                }
                            } else if let Ok(message) = serde_json::from_slice::<ForwardingMessage>(&payload) {
                                if message.kind == FORWARDING_KIND {
                                    if let Some(channel) = active_channel.as_mut() {
                                        if let Ok(mut data_plane) = DataPlane::open(channel, DEFAULT_MAXIMUM_PAYLOAD_SIZE) {
                                            if let Ok(received) = data_plane.receive(&message.data) {
                                                    println!("VPN_PACKET_RX_FROM_RELAY length={}", received.payload.len());
                                                if let Some(tx) = tun_tx.as_ref() {
                                                    let _ = tx.send(received.payload);
                                                        println!("VPN_PACKET_TX_TO_TUN");
                                                }
                                            }
                                        }
                                    }
                                }
                            } else if let Ok(metadata) = serde_json::from_slice::<ghost_layer_relay::RelayMetadata>(&payload) {
                                if let Ok(candidate) = RelayCandidate::from_metadata(&metadata, &requirements, SystemTime::now()) {
                                    candidates.push(candidate);
                                    let ranked = RelayRanking::rank(candidates.clone(), SystemTime::now());
                                    if let Ok(route) = RouteSelector::select(peer_id, &ranked, &route_policy, SystemTime::now()) {
                                        if pending_session.is_none() {
                                            let (first_peer, binding) = match &route {
                                                Route::OneHop { relay } => (relay.peer_id, RouteBinding::OneHop { relay: relay.peer_id }),
                                                Route::TwoHop { entry, exit } => (entry.peer_id, RouteBinding::TwoHop { entry: entry.peer_id, exit: exit.peer_id }),
                                            };
                                            if let Ok(initiator) = SessionInitiator::new(identity.keypair(), first_peer, binding, config.protocol_version.clone()) {
                                                if let Ok(init) = initiator.build_init() {
                                                    if let Ok(payload) = serde_json::to_vec(&init) {
                                                        node.request_discovery(first_peer, payload);
                                                        pending_session = Some(initiator);
                                                        info!(peer = %first_peer, "establishing secure session");
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        event => log_event(&event),
                    }
                }
            }
        }
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

fn build_protocol_request(
    config: &NodeConfig,
    route: &RouteBinding,
    session_id: ghost_layer_network::SessionId,
    client_peer: ghost_layer_network::PeerId,
    payload: Vec<u8>,
    envelope: DataPlaneEnvelope,
) -> Result<ForwardingMessage> {
    let RouteBinding::TwoHop { .. } = route else {
        return Err(anyhow::anyhow!(
            "protocol forwarding requires a two-hop route"
        ));
    };
    let (protocol, destination, source) = if config.udp_enabled {
        let destination = config
            .allowed_exit_destinations
            .first()
            .ok_or_else(|| anyhow::anyhow!("UDP forwarding requires an allowlisted destination"))?
            .parse::<SocketAddr>()?;
        (
            ForwardedProtocol::Udp,
            destination,
            Some(SocketAddr::from(([127, 0, 0, 1], 40000))),
        )
    } else if config.dns_enabled {
        (
            ForwardedProtocol::Dns,
            config
                .dns_server
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("DNS server is not configured"))?
                .parse::<SocketAddr>()?,
            None,
        )
    } else {
        return Err(anyhow::anyhow!("no protocol forwarding mode is enabled"));
    };
    Ok(ForwardingMessage {
        kind: FORWARDING_KIND.to_owned(),
        forwarding_id: session_id,
        client_session_id: session_id,
        client_peer: client_peer.to_string(),
        route: route.clone(),
        data: envelope,
        packet: Some(ForwardedPacket {
            protocol,
            source,
            destination,
            flow_id: session_id,
            session_id,
            route_id: session_id,
            sequence: 1,
            payload,
        }),
    })
}

fn dns_query(name: &str) -> Vec<u8> {
    let mut query = vec![0x12, 0x34, 0x01, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0];
    for label in name.split('.') {
        query.push(label.len() as u8);
        query.extend_from_slice(label.as_bytes());
    }
    query.extend_from_slice(&[0, 0, 1, 0, 1]);
    query
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
