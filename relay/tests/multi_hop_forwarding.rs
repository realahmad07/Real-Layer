use futures::StreamExt;
use ghost_layer_network::{
    DataPlane, DataPlaneEnvelope, EncryptedChannel, HandshakeInit, HandshakeResponse, NetworkEvent,
    NetworkNode, NodeIdentity, PeerId, RouteBinding, SecureSession, SessionInitiator,
    SessionResponder, TunConfig, TunDataPlane, CHANNEL_PROTOCOL_VERSION, DATA_PLANE_KIND,
    DEFAULT_MAXIMUM_PAYLOAD_SIZE,
};
use ghost_layer_relay::{
    DestinationPolicy, ExitForwardingBinding, ExitNetworkAdapter, ExitPacketHandler,
    ForwardingContext, ForwardingDirection, ForwardingMessage, ForwardingState, RelayCandidate,
    Route, RouteSelectionPolicy, RouteSelector, TcpAdapterConfig, TcpExitNetworkAdapter,
    FORWARDING_KIND,
};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, SystemTime};

async fn listening_address(node: &mut NetworkNode) -> String {
    loop {
        if let Some(event) = node.swarm.next().await {
            if let Some(NetworkEvent::Listening { address }) = node.translate_event(event) {
                return address.to_string();
            }
        }
    }
}

async fn establish_session(
    initiator_node: &mut NetworkNode,
    responder_node: &mut NetworkNode,
    initiator_identity: &NodeIdentity,
    responder_identity: &NodeIdentity,
    route: RouteBinding,
) -> (SecureSession, SecureSession) {
    let responder_peer = responder_identity.peer_id();
    let initiator =
        SessionInitiator::new(initiator_identity.keypair(), responder_peer, route, "1.0")
            .expect("create session initiator");
    let init = initiator.build_init().expect("build session init");
    let mut response: Option<HandshakeResponse> = None;
    let mut pending_init = Some(init);
    let mut responder = SessionResponder::new(responder_identity.keypair());
    let mut responder_session = None;

    tokio::time::timeout(Duration::from_secs(10), async {
        while response.is_none() {
            tokio::select! {
                Some(event) = initiator_node.swarm.next() => if let Some(event) = initiator_node.translate_event(event) {
                    match event {
                        NetworkEvent::PeerConnected { peer_id } if peer_id == responder_peer => {
                            initiator_node.request_discovery(peer_id, serde_json::to_vec(&pending_init.take().expect("single init")).expect("encode init"));
                        }
                        NetworkEvent::DiscoveryResponse { payload, .. } => {
                            response = Some(serde_json::from_slice(&payload).expect("decode session response"));
                        }
                        _ => {}
                    }
                },
                Some(event) = responder_node.swarm.next() => if let Some(NetworkEvent::DiscoveryRequest { peer_id, channel, payload, .. }) = responder_node.translate_event(event) {
                    let init: HandshakeInit = serde_json::from_slice(&payload).expect("decode session init");
                    let (response_message, session) = responder.accept_init(init, peer_id, "1.0").expect("accept session init");
                    responder_session = Some(session);
                    responder_node.respond_to_discovery(channel, serde_json::to_vec(&response_message).expect("encode session response")).expect("send session response");
                },
            }
        }
    })
    .await
    .expect("session establishment timeout");

    (
        initiator
            .complete(response.expect("session response"))
            .expect("complete session"),
        responder_session.expect("responder session"),
    )
}

fn candidate(peer_id: PeerId, address: String, latency_ms: u64) -> RelayCandidate {
    RelayCandidate {
        peer_id,
        address: address.parse().expect("relay address"),
        status: ghost_layer_relay::RelayStatus::Online,
        health: ghost_layer_relay::HealthClass::Healthy,
        latency: Some(Duration::from_millis(latency_ms)),
        capabilities: vec!["quic".to_owned(), "relay".to_owned()],
        protocol_version: "1.0".to_owned(),
        software_version: "test".to_owned(),
        last_seen: SystemTime::now(),
    }
}

#[tokio::test]
async fn client_entry_exit_forwards_controlled_data_plane_message() {
    let directory =
        std::env::temp_dir().join(format!("ghost-layer-multi-hop-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("create test directory");
    let client_identity =
        NodeIdentity::load_or_generate(directory.join("client.key")).expect("client identity");
    let entry_identity =
        NodeIdentity::load_or_generate(directory.join("entry.key")).expect("entry identity");
    let exit_identity =
        NodeIdentity::load_or_generate(directory.join("exit.key")).expect("exit identity");
    let mut client =
        NetworkNode::new(&client_identity, Duration::from_millis(50)).expect("client node");
    let mut entry =
        NetworkNode::new(&entry_identity, Duration::from_millis(50)).expect("entry node");
    let mut exit = NetworkNode::new(&exit_identity, Duration::from_millis(50)).expect("exit node");
    client
        .listen("/ip4/127.0.0.1/udp/0/quic-v1")
        .expect("client listen");
    entry
        .listen("/ip4/127.0.0.1/udp/0/quic-v1")
        .expect("entry listen");
    exit.listen("/ip4/127.0.0.1/udp/0/quic-v1")
        .expect("exit listen");
    let entry_address = listening_address(&mut entry).await;
    let exit_address = listening_address(&mut exit).await;
    client
        .dial(&format!("{entry_address}/p2p/{}", entry_identity.peer_id()))
        .expect("dial entry");
    entry
        .dial(&format!("{exit_address}/p2p/{}", exit_identity.peer_id()))
        .expect("dial exit");

    let route = RouteSelector::select(
        client_identity.peer_id(),
        &[
            candidate(entry_identity.peer_id(), entry_address.clone(), 3),
            candidate(exit_identity.peer_id(), exit_address, 10),
        ],
        &RouteSelectionPolicy {
            mode: ghost_layer_network::RouteMode::TwoHop,
            required_transport: "quic".to_owned(),
            required_capabilities: vec!["relay".to_owned()],
        },
        SystemTime::now(),
    )
    .expect("select two-hop route");
    let (entry_hop, exit_hop) = match &route {
        Route::TwoHop { entry, exit } => (entry, exit),
        Route::OneHop { .. } => panic!("expected two-hop route"),
    };
    let route_binding = RouteBinding::TwoHop {
        entry: entry_hop.peer_id,
        exit: exit_hop.peer_id,
    };
    let (client_session, entry_client_session) = establish_session(
        &mut client,
        &mut entry,
        &client_identity,
        &entry_identity,
        route_binding.clone(),
    )
    .await;
    let (relay_session, exit_session) = establish_session(
        &mut entry,
        &mut exit,
        &entry_identity,
        &exit_identity,
        RouteBinding::OneHop {
            relay: exit_identity.peer_id(),
        },
    )
    .await;
    let client_session_id = client_session.session_id();
    let relay_session_id = relay_session.session_id();
    let mut client_channel =
        EncryptedChannel::open(client_session, CHANNEL_PROTOCOL_VERSION, 4096, 8)
            .expect("client channel");
    let mut entry_client_channel =
        EncryptedChannel::open(entry_client_session, CHANNEL_PROTOCOL_VERSION, 4096, 8)
            .expect("entry channel");
    let mut entry_exit_channel =
        EncryptedChannel::open(relay_session, CHANNEL_PROTOCOL_VERSION, 4096, 8)
            .expect("relay channel");
    let mut exit_channel = EncryptedChannel::open(exit_session, CHANNEL_PROTOCOL_VERSION, 4096, 8)
        .expect("exit channel");
    let mut context = ForwardingContext::new(
        client_session_id,
        client_identity.peer_id(),
        entry_identity.peer_id(),
        exit_identity.peer_id(),
        route_binding,
    )
    .expect("forwarding context");
    context.transition_connecting().expect("context connecting");
    context
        .transition_established()
        .expect("context established");
    context
        .bind_relay_session(relay_session_id)
        .expect("bind relay session");
    assert_eq!(context.state(), ForwardingState::Established);
    let mut exit_handler = ExitPacketHandler::open(1500, 1500, 2).expect("open exit handler");
    let test_server = TcpListener::bind("127.0.0.1:0").expect("bind local test server");
    let test_server_address = test_server.local_addr().expect("test server address");
    let test_server_thread = thread::spawn(move || {
        let (mut stream, _) = test_server.accept().expect("accept test connection");
        let mut request = [0u8; 1500];
        let size = stream.read(&mut request).expect("read test request");
        assert_eq!(size, 20);
        assert_eq!(request[0] >> 4, 4);
        let mut response = vec![0u8; 20];
        response[0] = 0x45;
        response[19] = 2;
        stream.write_all(&response).expect("write test response");
    });

    let request_id = ghost_layer_network::SessionId::generate();
    let mut source_tun = ghost_layer_network::tun::mock::MockTunDevice::open(TunConfig {
        enabled: false,
        interface_name: "source".to_owned(),
        mtu: 1500,
        maximum_packet_size: 1500,
        read_buffer_limit: 2,
        write_buffer_limit: 2,
    });
    let mut destination_tun = ghost_layer_network::tun::mock::MockTunDevice::open(TunConfig {
        enabled: false,
        interface_name: "destination".to_owned(),
        mtu: 1500,
        maximum_packet_size: 1500,
        read_buffer_limit: 2,
        write_buffer_limit: 2,
    });
    let mut packet = vec![0u8; 20];
    packet[0] = 0x45;
    packet[19] = 1;
    source_tun
        .inject_packet(packet.clone())
        .expect("inject controlled packet");
    let client_envelope = {
        let mut client_data_plane =
            DataPlane::open(&mut client_channel, 1500).expect("client data plane");
        TunDataPlane::new(&mut source_tun, &mut client_data_plane, 1500, 1500)
            .expect("client TUN bridge")
            .read_frame()
            .expect("read client packet")
            .expect("client packet")
    };
    client.request_discovery(
        entry_identity.peer_id(),
        serde_json::to_vec(&client_envelope).expect("encode client data"),
    );

    let mut acknowledged = false;
    let mut client_response_channel = None;
    tokio::time::timeout(Duration::from_secs(10), async {
        while !acknowledged {
            tokio::select! {
                Some(event) = client.swarm.next() => if let Some(NetworkEvent::DiscoveryResponse { payload, .. }) = client.translate_event(event) {
                    let message: ForwardingMessage = serde_json::from_slice(&payload).expect("decode entry response");
                    context.validate_message(&message).expect("validate response context");
                    assert_eq!(message.data.session_id, client_session_id);
                    let response = {
                        let mut client_data_plane = DataPlane::open(&mut client_channel, 1500)
                            .expect("client response data plane");
                        TunDataPlane::new(&mut destination_tun, &mut client_data_plane, 1500, 1500)
                            .expect("client TUN response bridge")
                            .write_frame(&message.data)
                            .expect("write client response")
                    };
                    let mut expected_response = packet.clone();
                    expected_response[19] = 2;
                    assert_eq!(response.as_bytes(), expected_response.as_slice());
                    acknowledged = true;
                },
                Some(event) = entry.swarm.next() => if let Some(event) = entry.translate_event(event) {
                    match event {
                        NetworkEvent::DiscoveryRequest { peer_id, channel, payload, .. } => {
                            let client_data: DataPlaneEnvelope = serde_json::from_slice(&payload).expect("decode client data");
                            context.validate_client(client_data.session_id, peer_id).expect("validate client binding");
                            let incoming = DataPlane::open(&mut entry_client_channel, DEFAULT_MAXIMUM_PAYLOAD_SIZE).expect("entry client data plane").receive(&client_data).expect("decrypt entry data");
                            context.begin_forwarding(request_id, ForwardingDirection::ClientToExit).expect("begin client to exit");
                            client_response_channel = Some(channel);
                            let relay_frame = DataPlane::open(&mut entry_exit_channel, DEFAULT_MAXIMUM_PAYLOAD_SIZE).expect("entry relay data plane").send(&incoming.payload).expect("encrypt relay data");
                            let forwarded = ForwardingMessage {
                                kind: FORWARDING_KIND.to_owned(),
                                forwarding_id: context.forwarding_id(),
                                client_session_id: context.client_session_id(),
                                client_peer: client_identity.peer_id().to_string(),
                                route: context.route().clone(),
                                data: DataPlaneEnvelope { kind: DATA_PLANE_KIND.to_owned(), session_id: relay_session_id, frame: relay_frame },
                            };
                            entry.request_discovery(exit_identity.peer_id(), serde_json::to_vec(&forwarded).expect("encode forwarding request"));
                        }
                        NetworkEvent::DiscoveryResponse { payload, .. } => {
                            let response: ForwardingMessage = serde_json::from_slice(&payload).expect("decode exit response");
                            context.validate_message(&response).expect("validate exit response");
                            context.validate_exit(exit_identity.peer_id()).expect("validate exit peer");
                            context.validate_relay_session(response.data.session_id).expect("validate relay response session");
                            let incoming = DataPlane::open(&mut entry_exit_channel, DEFAULT_MAXIMUM_PAYLOAD_SIZE).expect("entry response data plane").receive(&response.data).expect("decrypt exit response");
                            context.begin_forwarding(request_id, ForwardingDirection::ExitToClient).expect("begin exit to client");
                            let client_frame = DataPlane::open(&mut entry_client_channel, DEFAULT_MAXIMUM_PAYLOAD_SIZE).expect("entry client response data plane").send(&incoming.payload).expect("encrypt entry response");
                            entry.respond_to_discovery(client_response_channel.take().expect("client response channel"), serde_json::to_vec(&ForwardingMessage {
                                kind: FORWARDING_KIND.to_owned(),
                                forwarding_id: context.forwarding_id(),
                                client_session_id: context.client_session_id(),
                                client_peer: client_identity.peer_id().to_string(),
                                route: context.route().clone(),
                                data: DataPlaneEnvelope { kind: DATA_PLANE_KIND.to_owned(), session_id: client_session_id, frame: client_frame },
                            }).expect("encode client response")).expect("send client response");
                        }
                        _ => {}
                    }
                },
                Some(event) = exit.swarm.next() => if let Some(NetworkEvent::DiscoveryRequest { peer_id, channel, payload, .. }) = exit.translate_event(event) {
                    let forwarded: ForwardingMessage = serde_json::from_slice(&payload).expect("decode forwarding request");
                    context.validate_message(&forwarded).expect("validate exit request");
                    context.validate_entry(peer_id).expect("validate entry peer");
                    context.validate_relay_session(forwarded.data.session_id).expect("validate relay request session");
                    let incoming = DataPlane::open(&mut exit_channel, DEFAULT_MAXIMUM_PAYLOAD_SIZE).expect("exit data plane").receive(&forwarded.data).expect("decrypt exit data");
                    let incoming_packet = ghost_layer_network::NetworkPacket::new(incoming.payload, 1500, 1500).expect("validate exit packet");
                    assert_eq!(incoming_packet.as_bytes(), packet.as_slice());
                    let mut adapter = TcpExitNetworkAdapter::connect(
                        &test_server_address.to_string(),
                        &DestinationPolicy::from_strings(&[test_server_address.to_string()])
                            .expect("configure approved exit destination"),
                        TcpAdapterConfig {
                            maximum_request_size: 1500,
                            maximum_response_size: 1500,
                            connection_timeout: Duration::from_secs(1),
                            read_timeout: Duration::from_secs(1),
                            write_timeout: Duration::from_secs(1),
                        },
                    ).expect("connect restricted exit adapter");
                    let response_packet = exit_handler.handle_bound_with_adapter(request_id, incoming_packet, &context, ExitForwardingBinding { relay_session_id, entry_peer: peer_id, exit_peer: exit_identity.peer_id() }, &mut adapter).expect("handle exit packet through local adapter");
                    adapter.close().expect("close exit adapter");
                    let response_frame = DataPlane::open(&mut exit_channel, DEFAULT_MAXIMUM_PAYLOAD_SIZE).expect("exit response data plane").send(response_packet.as_bytes()).expect("encrypt exit response");
                    exit.respond_to_discovery(channel, serde_json::to_vec(&ForwardingMessage {
                        kind: FORWARDING_KIND.to_owned(),
                        forwarding_id: context.forwarding_id(),
                        client_session_id: context.client_session_id(),
                        client_peer: client_identity.peer_id().to_string(),
                        route: context.route().clone(),
                        data: DataPlaneEnvelope { kind: DATA_PLANE_KIND.to_owned(), session_id: relay_session_id, frame: response_frame },
                    }).expect("encode exit response")).expect("send exit response");
                },
            }
        }
    }).await.expect("multi-hop forwarding timeout");
    assert!(acknowledged);
    context.transition_closing().expect("context closing");
    context.transition_closed().expect("context closed");
    client_channel.close().expect("close client channel");
    entry_client_channel
        .close()
        .expect("close entry client channel");
    entry_exit_channel.close().expect("close relay channel");
    exit_channel.close().expect("close exit channel");
    test_server_thread.join().expect("join test server");
    let _ = std::fs::remove_dir_all(directory);
}
