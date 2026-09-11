import re

with open('client/src/engine.rs', 'r', encoding='utf-8') as f:
    code = f.read()

# Replace async fn main with run_client
code = code.replace(
    'async fn main() -> Result<()> {',
    'pub async fn run_client(mut tun_rx: Option<tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>>, tun_tx: Option<tokio::sync::mpsc::UnboundedSender<Vec<u8>>>) -> Result<()> {'
)

# Remove tun_device = open_configured_tun
code = re.sub(
    r'    let mut tun_device = open_configured_tun\(&config\.tun\)\?;\n    if tun_device\.is_some\(\) \{\n        info!\("Windows TUN device opened"\);\n    \}\n',
    '',
    code
)
code = re.sub(
    r'    if tun_device\.is_some\(\) \{\n        info!\(interface = %config\.tun\.interface_name, "Windows TUN device opened"\);\n    \}\n',
    '',
    code
)
code = re.sub(
    r'    let mut tun_device = open_configured_tun\(&config\.tun\)\?;\n',
    '',
    code
)


# Remove route_manager
code = re.sub(r'    let mut route_manager = WindowsRouteManager::new\(\);\n', '', code)
code = re.sub(r'    if config\.os_routing_enabled \{\n        route_manager.*?\}\n', '', code, flags=re.DOTALL)

# Remove tun_device.close() at the end
code = re.sub(r'    if let Some\(device\) = tun_device\.as_mut\(\) \{\n        device.*?\}\n', '', code, flags=re.DOTALL)

# The loop to replace
loop_start = code.find('    while let Some(event) = node.swarm.next().await {')
loop_end = code.find('    Ok(())\n}')

if loop_start != -1 and loop_end != -1:
    new_loop = """
    let mut exit_peer: Option<ghost_layer_network::PeerId> = None;
    let mut current_session_id: Option<ghost_layer_network::SessionId> = None;

    loop {
        tokio::select! {
            Some(packet) = async { if let Some(rx) = tun_rx.as_mut() { rx.recv().await } else { futures::future::pending().await } } => {
                if let (Some(channel), Some(route), Some(exit)) = (active_channel.as_mut(), active_route.as_ref(), exit_peer.as_ref()) {
                    if let Ok(mut data_plane) = DataPlane::open(channel, DEFAULT_MAXIMUM_PAYLOAD_SIZE) {
                        if let Ok(frame) = data_plane.send(&packet) {
                            let session_id = current_session_id.unwrap();
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
                                node.request_discovery(*exit, payload);
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
                                                if let Some(tx) = tun_tx.as_ref() {
                                                    let _ = tx.send(received.payload);
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
"""
    code = code[:loop_start] + new_loop + code[loop_end:]

with open('client/src/engine.rs', 'w', encoding='utf-8') as f:
    f.write(code)
