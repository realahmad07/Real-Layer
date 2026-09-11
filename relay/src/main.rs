use anyhow::{Context, Result};
use futures::StreamExt;
use ghost_layer_network::{
    DataPlane, DataPlaneEnvelope, DiscoveryRequestId, DiscoveryResponseChannel, DnsLimits,
    EncryptedChannel, FlowKey, HandshakeInit, HandshakeResponse, NatTable, NetworkEvent,
    NetworkNode, NodeConfig, NodeIdentity, PeerId, RouteBinding, SessionId, SessionInitiator,
    SessionResponder, UdpDnsResolver, CHANNEL_PROTOCOL_VERSION, DATA_PLANE_KIND,
    DEFAULT_MAXIMUM_PAYLOAD_SIZE,
};
use ghost_layer_relay::{
    DestinationPolicy, ExitForwardingBinding, ExitNetworkAdapter, ExitPacketHandler,
    ForwardedPacket, ForwardedProtocol, ForwardingContext, ForwardingDirection, ForwardingMessage,
    HealthClass, HealthThresholds, InMemoryRelayRegistry, RelayCore, RelayHealth, RelayHeartbeat,
    RelayLifecycle, RelayMetadata, RelayMetadataRequest, RelayRegistry, RelayState, RelayStatus,
    TcpAdapterConfig, TcpExitNetworkAdapter, UdpExitNetworkAdapter, FORWARDING_KIND,
};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::IpAddr;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
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
    let capabilities = relay_capabilities(&config);
    let mut core = RelayCore::new(
        peer_id,
        config.software_version.clone(),
        config.protocol_version.clone(),
        capabilities.clone(),
        config.resource_limits,
    )
    .map_err(|error| anyhow::anyhow!("initialize relay core: {error}"))?;
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
    let mut udp_nat = if config.nat_enabled {
        Some(
            NatTable::new(
                IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
                1024,
                Duration::from_secs(30),
            )
            .map_err(|error| anyhow::anyhow!("open UDP NAT table: {error}"))?,
        )
    } else {
        None
    };
    let mut exit_handler = ExitPacketHandler::open(
        config.tun.maximum_packet_size,
        config.tun.mtu,
        config.tun.write_buffer_limit,
    )
    .map_err(|error| anyhow::anyhow!("open exit packet handler: {error}"))?;
    let external_destination = config.external_test_destination.clone();
    let destination_policy =
        if external_destination.is_some() || config.udp_enabled || config.dns_enabled {
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
    let health_signals = HealthSignals::new();
    let health_thread = config
        .health_listen_address
        .as_deref()
        .map(|address| spawn_health_server(address, health_signals.clone()))
        .transpose()?;
    let shutdown_signal = shutdown_signal();
    tokio::pin!(shutdown_signal);

    info!(%peer_id, role = ?config.relay_role, "relay identity loaded");
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
                    match &event {
                        NetworkEvent::Listening { .. } if core.lifecycle() == RelayLifecycle::Starting => {
                            core.transition(RelayLifecycle::Discovering)
                                .map_err(|error| anyhow::anyhow!("relay lifecycle: {error}"))?;
                            core.transition(RelayLifecycle::Ready)
                                .map_err(|error| anyhow::anyhow!("relay lifecycle: {error}"))?;
                            health_signals.ready.store(true, Ordering::Release);
                            health_signals.healthy.store(true, Ordering::Release);
                        }
                        NetworkEvent::PeerConnected { peer_id } => {
                            if let Err(error) = core.admit_peer(*peer_id) {
                                core.reject_packet();
                                warn!(%peer_id, %error, "peer rejected by relay resource policy");
                            }
                        }
                        NetworkEvent::PeerDisconnected { peer_id } => core.remove_peer(peer_id),
                        _ => {}
                    }
                    if let NetworkEvent::DiscoveryRequest { channel, payload, peer_id, .. } = event {
                        if let Ok(init) = serde_json::from_slice::<HandshakeInit>(&payload) {
                            if init.kind == "secure_session_init" {
                                let client_session_id = init.session_id;
                                let route = init.route_context.clone();
                                let is_entry = matches!(
                                    &route,
                                    RouteBinding::TwoHop { entry, .. } if *entry == peer_id || *entry == identity.peer_id()
                                ) && matches!(&route, RouteBinding::TwoHop { entry, .. } if *entry == identity.peer_id())
                                    && !matches!(config.relay_role, ghost_layer_network::RelayRole::Exit);
                                let client_peer = peer_id;
                                if let Ok((response, session)) = session_responder.accept_init(init, peer_id, &config.protocol_version) {
                                    let session_id = session.session_id();
                                    if let Err(error) = core.admit_session(session_id) {
                                        core.metrics_mut().session_errors += 1;
                                        warn!(%session_id, %error, "session rejected by relay resource policy");
                                        continue;
                                    }
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
                                let metadata = RelayMetadata::from_state_with_capabilities(&state, config.protocol_version.clone(), health, capabilities.clone());
                                node.respond_to_discovery(channel, serde_json::to_vec(&metadata)?)?;
                            }
                        } else if let Ok(forwarded) = serde_json::from_slice::<ForwardingMessage>(&payload) {
                            if forwarded.kind == FORWARDING_KIND {
                                let is_client_protocol_request = matches!(
                                    &forwarded.route,
                                    RouteBinding::TwoHop { entry, .. } if *entry == identity.peer_id()
                                ) && client_forwarding.contains_key(&forwarded.data.session_id);
                                if is_client_protocol_request {
                                    let forwarding = client_forwarding
                                        .get_mut(&forwarded.data.session_id)
                                        .expect("client forwarding context");
                                    forwarding
                                        .context
                                        .validate_client(forwarded.data.session_id, peer_id)
                                        .map_err(|error| anyhow::anyhow!("validate client protocol request: {error}"))?;
                                    if forwarded.route != *forwarding.context.route()
                                        || forwarded.client_peer != forwarding.client_peer.to_string()
                                    {
                                        return Err(anyhow::anyhow!("invalid client protocol forwarding binding"));
                                    }
                                    let packet = forwarded
                                        .packet
                                        .as_ref()
                                        .ok_or_else(|| anyhow::anyhow!("client protocol metadata is missing"))?;
                                    if packet.session_id != forwarding.context.client_session_id()
                                        || packet.flow_id != forwarding.context.client_session_id()
                                        || packet.route_id != forwarding.context.client_session_id()
                                    {
                                        return Err(anyhow::anyhow!("invalid client protocol packet binding"));
                                    }
                                    let incoming = {
                                        let channel_state = channels
                                            .get_mut(&forwarded.data.session_id)
                                            .ok_or_else(|| anyhow::anyhow!("missing client channel"))?;
                                        DataPlane::open(channel_state, DEFAULT_MAXIMUM_PAYLOAD_SIZE)
                                            .map_err(|error| anyhow::anyhow!("open client data plane: {error}"))?
                                            .receive(&forwarded.data)
                                            .map_err(|error| anyhow::anyhow!("decrypt client protocol data: {error}"))?
                                    };
                                    if packet.payload != incoming.payload {
                                        return Err(anyhow::anyhow!("client protocol payload does not match encrypted payload"));
                                    }
                                    if let Err(error) = core.admit_flow(forwarded.forwarding_id, incoming.payload.len()) {
                                        core.reject_packet();
                                        return Err(anyhow::anyhow!("flow rejected by relay resource policy: {error}"));
                                    }
                                    forwarding
                                        .context
                                        .begin_forwarding(forwarding.context.forwarding_id(), ForwardingDirection::ClientToExit)
                                        .map_err(|error| anyhow::anyhow!("begin client protocol forwarding: {error}"))?;
                                    let relay_frame = {
                                        let channel_state = channels
                                            .get_mut(&forwarding.relay_session_id)
                                            .ok_or_else(|| anyhow::anyhow!("missing relay channel"))?;
                                        DataPlane::open(channel_state, DEFAULT_MAXIMUM_PAYLOAD_SIZE)
                                            .map_err(|error| anyhow::anyhow!("open relay data plane: {error}"))?
                                            .send(&incoming.payload)
                                            .map_err(|error| anyhow::anyhow!("encrypt relay data: {error}"))?
                                    };
                                    let mut normalized_packet = packet.clone();
                                    normalized_packet.flow_id = forwarding.context.forwarding_id();
                                    normalized_packet.session_id = forwarding.context.client_session_id();
                                    normalized_packet.route_id = forwarding.context.forwarding_id();
                                    let outbound_request = node.request_discovery(
                                        forwarding.context.exit_peer(),
                                        serde_json::to_vec(&ForwardingMessage {
                                            kind: FORWARDING_KIND.to_owned(),
                                            forwarding_id: forwarding.context.forwarding_id(),
                                            client_session_id: forwarding.context.client_session_id(),
                                            client_peer: forwarding.client_peer.to_string(),
                                            route: forwarding.context.route().clone(),
                                            data: DataPlaneEnvelope {
                                                kind: DATA_PLANE_KIND.to_owned(),
                                                session_id: forwarding.relay_session_id,
                                                frame: relay_frame,
                                            },
                                            packet: Some(normalized_packet),
                                        })?,
                                    );
                                    pending_client_forwards.insert(
                                        outbound_request,
                                        PendingClientForward {
                                            client_session_id: forwarded.data.session_id,
                                            response_channel: channel,
                                        },
                                    );
                                } else if matches!(config.relay_role, ghost_layer_network::RelayRole::Entry) {
                                    warn!(%peer_id, "rejected exit forwarding on entry-only relay");
                                } else if !matches!(&forwarded.route, RouteBinding::TwoHop { exit, .. } if *exit == identity.peer_id()) {
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
                                    let packet = forwarded.packet.as_ref().expect("validated packet metadata");
                                    context.accept_sequence(packet.sequence).map_err(|error| anyhow::anyhow!("validate forwarding sequence: {error}"))?;
                                    context.begin_forwarding(forwarded.forwarding_id, ForwardingDirection::ClientToExit).map_err(|error| anyhow::anyhow!("begin exit forwarding: {error}"))?;
                                    let incoming = {
                                        let channel_state = channels.get_mut(&forwarded.data.session_id).ok_or_else(|| anyhow::anyhow!("unknown exit relay session"))?;
                                        DataPlane::open(channel_state, DEFAULT_MAXIMUM_PAYLOAD_SIZE)
                                            .map_err(|error| anyhow::anyhow!("open exit data plane: {error}"))?
                                            .receive(&forwarded.data)
                                            .map_err(|error| anyhow::anyhow!("decrypt exit data: {error}"))?
                                    };
                                    if packet.payload != incoming.payload {
                                        return Err(anyhow::anyhow!("forwarded packet payload does not match encrypted payload"));
                                    }
                                    if let Err(error) = core.admit_flow(forwarded.forwarding_id, incoming.payload.len()) {
                                        core.reject_packet();
                                        return Err(anyhow::anyhow!("flow rejected by relay resource policy: {error}"));
                                    }
                                    let policy = destination_policy.as_ref();
                                    let binding = ExitForwardingBinding {
                                        relay_session_id: forwarded.data.session_id,
                                        entry_peer: peer_id,
                                        exit_peer: identity.peer_id(),
                                    };
                                    let (response_payload, destination_label) = match forwarded.packet.as_ref().map(|packet| packet.protocol) {
                                        Some(ForwardedProtocol::Udp) => {
                                            if !config.udp_enabled { return Err(anyhow::anyhow!("UDP runtime is disabled")); }
                                            let packet = forwarded.packet.as_ref().expect("protocol metadata");
                                            exit_handler.validate_bound_payload(forwarded.forwarding_id, &incoming.payload, context, binding)
                                                .map_err(|error| anyhow::anyhow!("validate UDP payload: {error}"))?;
                                            let source = packet.source.expect("validated UDP source");
                                            let nat = udp_nat.as_mut().ok_or_else(|| anyhow::anyhow!("UDP NAT is disabled"))?;
                                            let flow = FlowKey {
                                                flow_id: forwarded.forwarding_id,
                                                source,
                                                destination: packet.destination,
                                                session_id: packet.session_id,
                                                source_identity: context.client_peer(),
                                                entry_peer: context.entry_peer(),
                                                exit_peer: context.exit_peer(),
                                                protocol: 17,
                                            };
                                            let mapping = nat
                                                .create(flow)
                                                .map_err(|error| anyhow::anyhow!("create UDP NAT mapping: {error}"))?;
                                            let policy = policy.ok_or_else(|| anyhow::anyhow!("UDP destination policy is not configured"))?;
                                            let mut adapter = UdpExitNetworkAdapter::connect(&packet.destination.to_string(), policy, config.tun.maximum_packet_size, Duration::from_secs(3))
                                                .map_err(|error| anyhow::anyhow!("connect UDP exit adapter: {error}"))?;
                                            let response = adapter.exchange(&incoming.payload).map_err(|error| anyhow::anyhow!("UDP exchange: {error}"))?;
                                            adapter.close().map_err(|error| anyhow::anyhow!("close UDP adapter: {error}"))?;
                                            nat.lookup_reverse(mapping.translated, flow)
                                                .map_err(|error| anyhow::anyhow!("lookup UDP NAT return mapping: {error}"))?;
                                            (response, packet.destination.to_string())
                                        }
                                        Some(ForwardedProtocol::Dns) => {
                                            if !config.dns_enabled { return Err(anyhow::anyhow!("DNS runtime is disabled")); }
                                            let server = config.dns_server.as_deref().ok_or_else(|| anyhow::anyhow!("DNS server is not configured"))?.parse().map_err(|_| anyhow::anyhow!("invalid DNS server"))?;
                                            if packet.destination != server {
                                                return Err(anyhow::anyhow!("DNS destination does not match configured DNS server"));
                                            }
                                            exit_handler.validate_bound_payload(forwarded.forwarding_id, &incoming.payload, context, binding)
                                                .map_err(|error| anyhow::anyhow!("validate DNS payload: {error}"))?;
                                            let resolver = UdpDnsResolver::new(server, DnsLimits { maximum_request_size: config.tun.maximum_packet_size, maximum_response_size: config.tun.maximum_packet_size, timeout: config.dns_timeout }, true)
                                                .map_err(|error| anyhow::anyhow!("open DNS resolver: {error}"))?;
                                            (resolver.resolve_packet(&incoming.payload).map_err(|error| anyhow::anyhow!("DNS exchange: {error}"))?, server.to_string())
                                        }
                                        Some(ForwardedProtocol::Tcp) => {
                                            let destination = external_destination.as_deref().ok_or_else(|| anyhow::anyhow!("external exit destination is not configured"))?;
                                            if packet.destination != destination.parse().map_err(|_| anyhow::anyhow!("invalid configured forwarding destination"))? {
                                                return Err(anyhow::anyhow!("TCP destination does not match configured exit destination"));
                                            }
                                            let policy = policy.ok_or_else(|| anyhow::anyhow!("TCP destination policy is not configured"))?;
                                            let mut adapter = TcpExitNetworkAdapter::connect(destination, policy, TcpAdapterConfig {
                                                maximum_request_size: config.tun.maximum_packet_size,
                                                maximum_response_size: config.tun.maximum_packet_size,
                                                connection_timeout: Duration::from_secs(3),
                                                read_timeout: Duration::from_secs(3),
                                                write_timeout: Duration::from_secs(3),
                                            }).map_err(|error| anyhow::anyhow!("connect exit adapter: {error}"))?;
                                            let response = exit_handler.handle_bound_with_adapter_payload(forwarded.forwarding_id, &incoming.payload, context, binding, &mut adapter)
                                                .map_err(|error| anyhow::anyhow!("handle exit payload: {error}"))?;
                                            adapter.close().map_err(|error| anyhow::anyhow!("close exit adapter: {error}"))?;
                                            (response, destination.to_owned())
                                        }
                                        None => return Err(anyhow::anyhow!("forwarding protocol metadata is missing")),
                                    };
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
                                        packet: forwarded.packet.clone(),
                                    };
                                    node.respond_to_discovery(channel, serde_json::to_vec(&response)?)?;
                                    info!(forwarding_id = %response.forwarding_id, destination = %destination_label, "exit returned protocol response");
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
                                            packet: Some(ForwardedPacket {
                                                protocol: ForwardedProtocol::Tcp,
                                                source: None,
                                                destination: config
                                                    .external_test_destination
                                                    .as_deref()
                                                    .ok_or_else(|| anyhow::anyhow!("external exit destination is not configured"))?
                                                    .parse()
                                                    .map_err(|_| anyhow::anyhow!("invalid configured forwarding destination"))?,
                                                flow_id: forwarding.context.forwarding_id(),
                                                session_id: forwarding.context.client_session_id(),
                                                route_id: forwarding.context.forwarding_id(),
                                                sequence: 1,
                                                payload: incoming_payload.clone(),
                                            }),
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
                                packet: response.packet.clone(),
                            };
                            node.respond_to_discovery(pending.response_channel, serde_json::to_vec(&client_response)?)?;
                        }
                    } else {
                        log_event(&event);
                    }
                }
            }
            _ = heartbeat_timer.tick() => {
                if let Some(nat) = udp_nat.as_mut() {
                    nat.cleanup();
                }
                let now = SystemTime::now();
                let health = RelayHealth::evaluate(&state, thresholds, now);
                state.set_status(match health.class {
                    HealthClass::Healthy => RelayStatus::Online,
                    HealthClass::Degraded => RelayStatus::Degraded,
                    HealthClass::Unhealthy => RelayStatus::Offline,
                });
                health_signals.healthy.store(health.class != HealthClass::Unhealthy, Ordering::Release);
                let heartbeat = RelayHeartbeat::from_state(&mut state, now);
                log_heartbeat(&heartbeat, health);
            }
            _ = &mut shutdown_signal => {
                if core.lifecycle() == RelayLifecycle::Ready {
                    core.transition(RelayLifecycle::Draining)
                        .map_err(|error| anyhow::anyhow!("relay lifecycle: {error}"))?;
                }
                core.transition(RelayLifecycle::ShuttingDown)
                    .map_err(|error| anyhow::anyhow!("relay lifecycle: {error}"))?;
                state.set_status(RelayStatus::ShuttingDown);
                health_signals.ready.store(false, Ordering::Release);
                health_signals.healthy.store(false, Ordering::Release);
                health_signals.alive.store(false, Ordering::Release);
                if let Some(nat) = udp_nat.as_mut() {
                    for forwarding in client_forwarding.values() {
                        nat.remove_session(forwarding.context.client_session_id());
                    }
                    for context in exit_forwarding.values() {
                        nat.remove_session(context.client_session_id());
                    }
                }
                channels.clear();
                client_forwarding.clear();
                exit_forwarding.clear();
                core.transition(RelayLifecycle::Stopped)
                    .map_err(|error| anyhow::anyhow!("relay lifecycle: {error}"))?;
                info!(peer_id = %state.peer_id(), status = %state.status(), "relay shutting down");
                break;
            }
        }
    }
    if let Some(handle) = health_thread {
        let _ = handle.join();
    }
    Ok(())
}

#[derive(Clone)]
struct HealthSignals {
    alive: Arc<AtomicBool>,
    ready: Arc<AtomicBool>,
    healthy: Arc<AtomicBool>,
}

impl HealthSignals {
    fn new() -> Self {
        Self {
            alive: Arc::new(AtomicBool::new(true)),
            ready: Arc::new(AtomicBool::new(false)),
            healthy: Arc::new(AtomicBool::new(false)),
        }
    }
}

fn http_response(status_code: u16, body: &str) -> Vec<u8> {
    let body_bytes = body.as_bytes();
    let status_text = match status_code {
        200 => "OK",
        404 => "Not Found",
        503 => "Service Unavailable",
        _ => "OK",
    };

    let mut response = Vec::new();
    response.extend_from_slice(format!("HTTP/1.1 {status_code} {status_text}\r\n").as_bytes());
    response.extend_from_slice(b"Content-Type: application/json\r\n");
    response.extend_from_slice(format!("Content-Length: {}\r\n", body_bytes.len()).as_bytes());
    response.extend_from_slice(b"Connection: close\r\n\r\n");
    response.extend_from_slice(body_bytes);
    response
}

fn spawn_health_server(address: &str, signals: HealthSignals) -> Result<thread::JoinHandle<()>> {
    let address = address.to_owned();
    let listener = std::net::TcpListener::bind(&address)
        .with_context(|| format!("bind health endpoint at {address}"))?;
    listener
        .set_nonblocking(true)
        .context("configure health endpoint")?;
    Ok(thread::spawn(move || {
        info!(%address, "health endpoint listening");
        while signals.alive.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let mut request = [0u8; 512];
                    let _ = stream.read(&mut request);
                    let request = String::from_utf8_lossy(&request);
                    let path = request.split_whitespace().nth(1).unwrap_or("/");
                    let (status, body) = match path {
                        "/live" => (200, r#"{"alive":true}"#),
                        "/ready" if signals.ready.load(Ordering::Acquire) => {
                            (200, r#"{"ready":true}"#)
                        }
                        "/ready" => (503, r#"{"ready":false}"#),
                        "/health" if signals.healthy.load(Ordering::Acquire) => {
                            (200, r#"{"health":"healthy"}"#)
                        }
                        "/health" => (503, r#"{"health":"unhealthy"}"#),
                        _ => (404, r#"{"error":"not_found"}"#),
                    };
                    let response = http_response(status, body);
                    let _ = stream.write_all(&response);
                    let _ = stream.shutdown(std::net::Shutdown::Both);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(100));
                }
                Err(error) => {
                    warn!(%error, "health endpoint accept failed");
                    break;
                }
            }
        }
    }))
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut terminate = signal(SignalKind::terminate()).expect("install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = terminate.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_health_response_is_valid_http() {
        let response = http_response(200, "{\"alive\":true}");
        let text = String::from_utf8(response).expect("http response should be utf-8");

        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(text.contains("Content-Type: application/json"));
        assert!(text.contains("Content-Length: 16"));
        assert!(text.contains("{\"alive\":true}"));
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

fn relay_capabilities(config: &NodeConfig) -> Vec<String> {
    let mut capabilities = vec![
        "quic".to_owned(),
        "relay".to_owned(),
        "health_reporting".to_owned(),
    ];
    match config.route_mode {
        ghost_layer_network::RouteMode::OneHop => capabilities.push("one_hop".to_owned()),
        ghost_layer_network::RouteMode::TwoHop => capabilities.push("two_hop".to_owned()),
    }
    match config.relay_role {
        ghost_layer_network::RelayRole::Entry => capabilities.push("entry_role".to_owned()),
        ghost_layer_network::RelayRole::Exit => capabilities.push("exit_role".to_owned()),
        ghost_layer_network::RelayRole::Both => {
            capabilities.push("entry_role".to_owned());
            capabilities.push("exit_role".to_owned());
        }
    }
    if config.external_test_destination.is_some()
        && !matches!(config.relay_role, ghost_layer_network::RelayRole::Entry)
    {
        capabilities.push("tcp_exit".to_owned());
    }
    if config.udp_enabled && !matches!(config.relay_role, ghost_layer_network::RelayRole::Entry) {
        capabilities.push("udp_exit".to_owned());
    }
    if config.dns_enabled && !matches!(config.relay_role, ghost_layer_network::RelayRole::Entry) {
        capabilities.push("dns_forwarding".to_owned());
    }
    if config.tun.enabled {
        capabilities.push("tun_boundary".to_owned());
    }
    capabilities
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
