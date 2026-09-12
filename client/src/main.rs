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
    let client_state = Arc::new(RwLock::new(ClientState::default()));
    let _health_server = spawn_client_health_server(client_state.clone());
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
                                        let (envelope, protocol_payload) = if let Some(device) =
                                            tun_device.as_mut()
                                        {
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
                                            let payload = packet.as_bytes().to_vec();
                                            let envelope = pipeline.send(payload.clone()).map_err(
                                                |error| {
                                                    anyhow::anyhow!("route TUN packet: {error}")
                                                },
                                            )?;
                                            (envelope, Some(payload))
                                        } else {
                                            let payload = if config.udp_enabled {
                                                b"hello ghost layer udp".to_vec()
                                            } else if config.dns_enabled {
                                                dns_query("ghost-layer.test")
                                            } else if config.external_test_destination.is_some()
                                                && matches!(route, RouteBinding::TwoHop { .. })
                                            {
                                                b"ghost-layer-external-test".to_vec()
                                            } else {
                                                b"hello ghost layer".to_vec()
                                            };
                                            let frame = DataPlane::open(
                                                &mut channel,
                                                DEFAULT_MAXIMUM_PAYLOAD_SIZE,
                                            )
                                            .map_err(|error| {
                                                anyhow::anyhow!("open data plane: {error}")
                                            })?
                                            .send(&payload)
                                            .map_err(|error| {
                                                anyhow::anyhow!(
                                                    "encrypt data-plane payload: {error}"
                                                )
                                            })?;
                                            (
                                                DataPlaneEnvelope {
                                                    kind: DATA_PLANE_KIND.to_owned(),
                                                    session_id,
                                                    frame,
                                                },
                                                Some(payload.to_vec()),
                                            )
                                        };
                                        let request_payload =
                                            if matches!(route, RouteBinding::TwoHop { .. })
                                                && (config.udp_enabled || config.dns_enabled)
                                            {
                                                serde_json::to_vec(&build_protocol_request(
                                                    &config,
                                                    &route,
                                                    session_id,
                                                    peer_id,
                                                    protocol_payload.expect("protocol payload"),
                                                    envelope,
                                                )?)?
                                            } else {
                                                serde_json::to_vec(&envelope)?
                                            };
                                        node.request_discovery(peer, request_payload);
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
                                        Ok(message)
                                            if matches!(
                                                forwarded
                                                    .packet
                                                    .as_ref()
                                                    .map(|packet| packet.protocol),
                                                Some(
                                                    ForwardedProtocol::Udp | ForwardedProtocol::Dns
                                                )
                                            ) =>
                                        {
                                            if forwarded.packet.as_ref().is_some_and(|packet| {
                                                packet.protocol == ForwardedProtocol::Udp
                                            }) && message.payload
                                                != b"ack: hello ghost layer udp"
                                            {
                                                return Err(anyhow::anyhow!(
                                                    "unexpected UDP response payload"
                                                ));
                                            }
                                            if forwarded.packet.as_ref().is_some_and(|packet| {
                                                packet.protocol == ForwardedProtocol::Dns
                                            }) && !message
                                                .payload
                                                .windows(4)
                                                .any(|window| window == [127, 0, 0, 1])
                                            {
                                                return Err(anyhow::anyhow!(
                                                    "unexpected DNS response payload"
                                                ));
                                            }
                                            info!(session_id = %forwarded.client_session_id, protocol = ?forwarded.packet.as_ref().map(|packet| packet.protocol), response_size = message.payload.len(), "protocol response received through entry and exit");
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
                                                            exit_address: exit.address.to_string(),
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


use serde::Serialize;
use std::sync::{Arc, RwLock};
use std::io::{Read, Write};

#[derive(Serialize, Clone)]
pub struct ClientState {
    pub alive: bool,
    pub ready: bool,
    pub health: String,
    pub status: String,
    pub route: String,
}

impl Default for ClientState {
    fn default() -> Self {
        Self {
            alive: true,
            ready: false,
            health: "unhealthy".to_owned(),
            status: "disconnected".to_owned(),
            route: "none".to_owned(),
        }
    }
}

pub fn spawn_client_health_server(state: Arc<RwLock<ClientState>>) -> Result<std::thread::JoinHandle<()>> {
    let address = "127.0.0.1:8082".to_owned();
    let listener = std::net::TcpListener::bind(&address)
        .context("bind client health endpoint")?;
    listener.set_nonblocking(true).context("configure client health endpoint")?;
    Ok(std::thread::spawn(move || {
        tracing::info!(%address, "client health endpoint listening");
        let mut running = true;
        while running {
            match listener.accept() {
                Ok((mut stream, _addr)) => {
                    let state = state.clone();
                    std::thread::spawn(move || {
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(5000)));
                        let _ = stream.set_write_timeout(Some(std::time::Duration::from_millis(5000)));
                        let mut request = [0u8; 512];
                        let bytes_read = stream.read(&mut request).unwrap_or(0);
                        if bytes_read == 0 { return; }
                        let current_state = { state.read().unwrap().clone() };
                        let body = serde_json::to_string(&current_state).unwrap_or_else(|_| "{}".to_owned());
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        let _ = stream.write_all(response.as_bytes());
                        let _ = stream.shutdown(std::net::Shutdown::Both);
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    if !state.read().unwrap().alive {
                        running = false;
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "client health endpoint accept failed");
                    break;
                }
            }
        }
    }))
}
