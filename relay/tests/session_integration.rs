use futures::StreamExt;
use ghost_layer_network::{
    DataPlane, DataPlaneEnvelope, DataPlaneMessageType, EncryptedChannel, HandshakeInit,
    HandshakeResponse, NetworkEvent, NetworkNode, NodeIdentity, RouteBinding, SessionInitiator,
    SessionResponder, CHANNEL_PROTOCOL_VERSION, DATA_PLANE_KIND, DEFAULT_MAXIMUM_PAYLOAD_SIZE,
};
use std::time::Duration;

#[tokio::test]
async fn local_quic_session_binds_one_and_two_hop_routes() {
    let directory =
        std::env::temp_dir().join(format!("ghost-layer-session-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("session test directory");
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
    entry
        .listen("/ip4/127.0.0.1/udp/0/quic-v1")
        .expect("entry listen");
    let entry_address = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(event) = entry.swarm.next().await {
                if let Some(NetworkEvent::Listening { address }) = entry.translate_event(event) {
                    break address;
                }
            }
        }
    })
    .await
    .expect("entry listen event");
    client
        .listen("/ip4/127.0.0.1/udp/0/quic-v1")
        .expect("client listen");
    client
        .dial(&format!("{entry_address}/p2p/{}", entry_identity.peer_id()))
        .expect("dial entry");

    let route = RouteBinding::TwoHop {
        entry: entry_identity.peer_id(),
        exit: exit_identity.peer_id(),
    };
    let initiator = SessionInitiator::new(
        client_identity.keypair(),
        entry_identity.peer_id(),
        route.clone(),
        "1.0",
    )
    .expect("route-bound initiator");
    let init = initiator.build_init().expect("handshake init");
    let mut responder = SessionResponder::new(entry_identity.keypair());
    let mut response: Option<HandshakeResponse> = None;
    let mut pending_init: Option<HandshakeInit> = Some(init);
    let mut responder_channel: Option<EncryptedChannel> = None;

    tokio::time::timeout(Duration::from_secs(10), async {
        while response.is_none() {
            tokio::select! {
                Some(event) = client.swarm.next() => if let Some(event) = client.translate_event(event) {
                    if let NetworkEvent::PeerConnected { peer_id } = event {
                        let init = pending_init.take().expect("single session init");
                        client.request_discovery(peer_id, serde_json::to_vec(&init).expect("encode init"));
                    } else if let NetworkEvent::DiscoveryResponse { payload, .. } = event {
                        response = Some(serde_json::from_slice(&payload).expect("decode response"));
                    }
                },
                Some(event) = entry.swarm.next() => if let Some(NetworkEvent::DiscoveryRequest { peer_id, channel, payload, .. }) = entry.translate_event(event) {
                    let init: HandshakeInit = serde_json::from_slice(&payload).expect("decode init");
                    let (response_message, responder_session) = responder.accept_init(init, peer_id, "1.0").expect("accept authenticated init");
                    responder_channel = Some(EncryptedChannel::open(responder_session, CHANNEL_PROTOCOL_VERSION, 4096, 4).expect("open responder channel"));
                    entry.respond_to_discovery(channel, serde_json::to_vec(&response_message).expect("encode response")).expect("send response");
                },
            }
        }
    }).await.expect("session handshake timeout");

    let session = initiator
        .complete(response.expect("response"))
        .expect("complete session");
    assert_eq!(session.route(), &route);
    assert_eq!(session.peer_id(), entry_identity.peer_id());
    assert_eq!(
        session.state(),
        ghost_layer_network::SessionState::Established
    );
    assert_ne!(
        route,
        RouteBinding::OneHop {
            relay: entry_identity.peer_id()
        }
    );
    let mut client_channel = EncryptedChannel::open(session, CHANNEL_PROTOCOL_VERSION, 4096, 4)
        .expect("open client channel");
    let application_payload = b"hello ghost layer";
    let frame = DataPlane::open(&mut client_channel, DEFAULT_MAXIMUM_PAYLOAD_SIZE)
        .expect("open client data plane")
        .send(application_payload)
        .expect("encrypt ping");
    let session_id = client_channel.session_id();
    client.request_discovery(
        entry_identity.peer_id(),
        serde_json::to_vec(&DataPlaneEnvelope {
            kind: DATA_PLANE_KIND.to_owned(),
            session_id,
            frame,
        })
        .expect("encode data-plane payload"),
    );
    let mut acknowledgement_received = false;
    tokio::time::timeout(Duration::from_secs(10), async {
        while !acknowledgement_received {
            tokio::select! {
                Some(event) = client.swarm.next() => if let Some(NetworkEvent::DiscoveryResponse { payload, .. }) = client.translate_event(event) {
                    let envelope: DataPlaneEnvelope = serde_json::from_slice(&payload).expect("decode acknowledgement envelope");
                    assert_eq!(envelope.session_id, session_id);
                    let message = DataPlane::open(&mut client_channel, DEFAULT_MAXIMUM_PAYLOAD_SIZE)
                        .expect("open client data plane")
                        .receive(&envelope)
                        .expect("decrypt acknowledgement");
                    assert_eq!(message.message_type, DataPlaneMessageType::Data);
                    assert_eq!(message.payload, b"ack: hello ghost layer");
                    acknowledgement_received = true;
                },
                Some(event) = entry.swarm.next() => if let Some(NetworkEvent::DiscoveryRequest { channel, payload, .. }) = entry.translate_event(event) {
                    let envelope: DataPlaneEnvelope = serde_json::from_slice(&payload).expect("decode data-plane payload");
                    let responder_channel = responder_channel.as_mut().expect("responder channel");
                    let mut data_plane = DataPlane::open(responder_channel, DEFAULT_MAXIMUM_PAYLOAD_SIZE)
                        .expect("open responder data plane");
                    let message = data_plane.receive(&envelope).expect("decrypt data-plane payload");
                    assert_eq!(message.payload, application_payload);
                    let mut acknowledgement = b"ack: ".to_vec();
                    acknowledgement.extend(message.payload);
                    let response_frame = data_plane.send(&acknowledgement).expect("encrypt acknowledgement");
                    entry.respond_to_discovery(channel, serde_json::to_vec(&DataPlaneEnvelope { kind: DATA_PLANE_KIND.to_owned(), session_id: envelope.session_id, frame: response_frame }).expect("encode acknowledgement")).expect("send acknowledgement");
                },
            }
        }
    }).await.expect("encrypted ping-pong timeout");
    assert!(acknowledgement_received);
    client_channel.close().expect("close client channel");
    let _ = std::fs::remove_dir_all(directory);
}
