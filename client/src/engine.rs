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


    let mut exit_peer: Option<ghost_layer_network::PeerId> = None;
    let mut current_session_id: Option<ghost_layer_network::SessionId> = None;

    loop {
        tokio::select! {
            Some(packet) = async { if let Some(rx) = tun_rx.as_mut() { rx.recv().await } else { futures::future::pending().await } } => {
                if let (Some(channel), Some(route), Some(exit)) = (active_channel.as_mut(), active_route.as_ref(), exit_peer.as_ref()) {
                    let session_id = current_session_id.expect("session ID must be present if channel is active");
                    if let Some((dns_dest, src_port, dns_payload)) = extract_dns_payload(&packet) {
                        match DataPlane::open(channel, DEFAULT_MAXIMUM_PAYLOAD_SIZE) {
                            Ok(mut data_plane) => {
                                match data_plane.send(&dns_payload) {
                                    Ok(frame) => {
                                        let envelope = DataPlaneEnvelope {
                                            kind: DATA_PLANE_KIND.to_owned(),
                                            session_id,
                                            frame,
                                        };
                                        let message = ForwardingMessage {
                                            kind: FORWARDING_KIND.to_owned(),
                                            forwarding_id: session_id,
                                            client_session_id: session_id,
                                            client_peer: peer_id.to_string(),
                                            route: route.clone(),
                                            data: envelope,
                                            packet: Some(ForwardedPacket {
                                                protocol: ForwardedProtocol::Dns,
                                                source: Some(std::net::SocketAddr::new(
                                                    std::net::IpAddr::V4(std::net::Ipv4Addr::new(10, 8, 0, 1)),
                                                    src_port,
                                                )),
                                                destination: dns_dest,
                                                flow_id: session_id,
                                                session_id,
                                                route_id: session_id,
                                                sequence: 1,
                                                payload: dns_payload,
                                            }),
                                        };
                                        if let Ok(payload) = serde_json::to_vec(&message) {
                                            println!("VPN_PACKET_TX_TO_RELAY length={} protocol=dns dest={}", packet.len(), dns_dest);
                                            node.request_discovery(*exit, payload);
                                        }
                                    }
                                    Err(error) => {
                                        warn!(?error, "failed to encrypt DNS payload for relay");
                                    }
                                }
                            }
                            Err(error) => {
                                warn!(?error, "unable to open data plane for DNS packet");
                            }
                        }
                    } else {
                        // Non-DNS packet: send as ForwardedProtocol::Ip
                        if packet.len() >= 20 {
                            let dst_ip = std::net::Ipv4Addr::new(packet[16], packet[17], packet[18], packet[19]);
                            if let Ok(mut data_plane) = DataPlane::open(channel, DEFAULT_MAXIMUM_PAYLOAD_SIZE) {
                                if let Ok(frame) = data_plane.send(&packet) {
                                    let envelope = DataPlaneEnvelope {
                                        kind: DATA_PLANE_KIND.to_owned(),
                                        session_id,
                                        frame,
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
                                            source: Some(std::net::SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::new(10, 8, 0, 1)), 0)),
                                            destination: std::net::SocketAddr::new(std::net::IpAddr::V4(dst_ip), 0),
                                            flow_id: session_id,
                                            session_id,
                                            route_id: session_id,
                                            sequence: 1,
                                            payload: packet.clone(),
                                        }),
                                    };
                                    if let Ok(payload) = serde_json::to_vec(&message) {
                                        println!("VPN_PACKET_TX_TO_RELAY length={} protocol=ip dest={}", packet.len(), dst_ip);
                                        node.request_discovery(*exit, payload);
                                    }
                                }
                            }
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
                                            match &route {
                                                RouteBinding::TwoHop { exit, .. } => {
                                                    exit_peer = Some(*exit);
                                                }
                                                RouteBinding::OneHop { relay } => {
                                                    exit_peer = Some(*relay);
                                                }
                                            }
                                            active_route = Some(route.clone());
                                            if let Ok(channel) = EncryptedChannel::open(session, CHANNEL_PROTOCOL_VERSION, 4096, 8) {
                                                active_channel = Some(channel);
                                                if let Ok(mut state) = client_state.write() {
                                                    state.ready = true;
                                                    state.health = "healthy".to_owned();
                                                    state.status = "online".to_owned();
                                                    state.route = format!("{:?}", route);
                                                }
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
                                                    // Wrap DNS response bytes back into a valid IP/UDP packet
                                                    // so Android's IP stack accepts it.
                                                    let tun_packet = if let Some(fwd_packet) = message.packet.as_ref() {
                                                        if matches!(fwd_packet.protocol, ForwardedProtocol::Dns) {
                                                            let dst_port = fwd_packet.source
                                                                .map(|s| s.port())
                                                                .unwrap_or(12345);
                                                            let dns_server_ip = match fwd_packet.destination.ip() {
                                                                std::net::IpAddr::V4(ip) => ip.octets(),
                                                                _ => [8u8, 8, 8, 8],
                                                            };
                                                            build_dns_response_packet(
                                                                &received.payload,
                                                                dns_server_ip,
                                                                [10u8, 8, 0, 1],
                                                                53,
                                                                dst_port,
                                                            )
                                                        } else {
                                                            received.payload
                                                        }
                                                    } else {
                                                        received.payload
                                                    };
                                                    let _ = tx.send(tun_packet);
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
                                                Route::TwoHop { entry, exit } => (entry.peer_id, RouteBinding::TwoHop { entry: entry.peer_id, exit: exit.peer_id, exit_address: exit.address.to_string() }),
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
                        NetworkEvent::PeerDisconnected { peer_id: disconnected_peer } => {
                            if Some(disconnected_peer) == exit_peer || active_route.as_ref().is_some_and(|r| match r {
                                RouteBinding::OneHop { relay } => *relay == disconnected_peer,
                                RouteBinding::TwoHop { entry, .. } => *entry == disconnected_peer,
                            }) {
                                active_channel = None;
                                active_route = None;
                                current_session_id = None;
                                exit_peer = None;
                                if let Ok(mut state) = client_state.write() {
                                    state.ready = false;
                                    state.health = "unhealthy".to_owned();
                                    state.status = "disconnected".to_owned();
                                    state.route = "none".to_owned();
                                }
                            }
                            log_event(&NetworkEvent::PeerDisconnected { peer_id: disconnected_peer });
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

/// Extract a DNS query from a raw IPv4 TUN packet.
/// Returns `Some((dns_server_dest, client_source_port, raw_dns_bytes))` for IPv4/UDP packets
/// destined for port 53, or `None` for all other packets.
fn extract_dns_payload(packet: &[u8]) -> Option<(std::net::SocketAddr, u16, Vec<u8>)> {
    if packet.len() < 28 {
        return None; // too short for IPv4 (20) + UDP (8)
    }
    if packet[0] >> 4 != 4 {
        return None; // not IPv4
    }
    if packet[9] != 17 {
        return None; // protocol not UDP
    }
    let ip_header_len = ((packet[0] & 0x0f) as usize) * 4;
    if packet.len() < ip_header_len + 8 {
        return None;
    }
    let src_port = u16::from_be_bytes([packet[ip_header_len], packet[ip_header_len + 1]]);
    let dst_port = u16::from_be_bytes([packet[ip_header_len + 2], packet[ip_header_len + 3]]);
    if dst_port != 53 {
        return None; // not DNS
    }
    let dst_ip = std::net::Ipv4Addr::new(packet[16], packet[17], packet[18], packet[19]);
    let dns_payload = packet[ip_header_len + 8..].to_vec();
    Some((
        std::net::SocketAddr::new(std::net::IpAddr::V4(dst_ip), 53),
        src_port,
        dns_payload,
    ))
}

/// Build a minimal IPv4/UDP packet wrapping `dns_response` bytes.
/// Used to reconstruct the return packet that Android's resolver expects on the TUN.
fn build_dns_response_packet(
    dns_response: &[u8],
    src_ip: [u8; 4],
    dst_ip: [u8; 4],
    src_port: u16,
    dst_port: u16,
) -> Vec<u8> {
    let udp_payload_len = 8 + dns_response.len();
    let ip_total_len = 20 + udp_payload_len;
    let mut packet = vec![0u8; ip_total_len];

    // IPv4 header
    packet[0] = 0x45; // version=4, IHL=5
    packet[1] = 0x00;
    packet[2] = (ip_total_len >> 8) as u8;
    packet[3] = ip_total_len as u8;
    packet[4] = 0x00;
    packet[5] = 0x00;
    packet[6] = 0x40;
    packet[7] = 0x00; // flags: DF
    packet[8] = 64;   // TTL
    packet[9] = 17;   // protocol: UDP
    packet[12..16].copy_from_slice(&src_ip);
    packet[16..20].copy_from_slice(&dst_ip);
    let checksum = ipv4_checksum(&packet[0..20]);
    packet[10] = (checksum >> 8) as u8;
    packet[11] = checksum as u8;

    // UDP header at offset 20
    packet[20] = (src_port >> 8) as u8;
    packet[21] = src_port as u8;
    packet[22] = (dst_port >> 8) as u8;
    packet[23] = dst_port as u8;
    packet[24] = (udp_payload_len >> 8) as u8;
    packet[25] = udp_payload_len as u8;
    packet[26] = 0x00;
    packet[27] = 0x00; // UDP checksum disabled

    packet[28..].copy_from_slice(dns_response);
    packet
}

fn ipv4_checksum(header: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    for chunk in header.chunks(2) {
        let word = if chunk.len() == 2 {
            ((chunk[0] as u32) << 8) | (chunk[1] as u32)
        } else {
            (chunk[0] as u32) << 8
        };
        sum += word;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
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
