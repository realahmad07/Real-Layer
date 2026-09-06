use futures::StreamExt;
use ghost_layer_network::{NetworkEvent, NetworkNode, NodeIdentity, PeerId};
use ghost_layer_relay::{
    CandidateRequirements, HealthClass, RelayCandidate, RelayMetadata, RelayMetadataRequest,
    RelayRanking, RelayStatus, Route, RouteSelectionPolicy, RouteSelector,
};
use std::time::{Duration, SystemTime};

struct TestRelay {
    peer_id: PeerId,
    node: NetworkNode,
    metadata: RelayMetadata,
    address: String,
}

#[tokio::test]
async fn client_discovers_filters_and_ranks_three_local_relays() {
    let directory =
        std::env::temp_dir().join(format!("ghost-layer-discovery-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("create test directory");
    let mut relays = Vec::new();
    for index in 0..3 {
        let identity = NodeIdentity::load_or_generate(directory.join(format!("relay-{index}.key")))
            .expect("relay identity");
        let mut node = NetworkNode::new(&identity, Duration::from_millis(50)).expect("relay node");
        node.listen("/ip4/127.0.0.1/udp/0/quic-v1")
            .expect("relay listen");
        let address = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(event) = node.swarm.next().await {
                    if let Some(NetworkEvent::Listening { address }) = node.translate_event(event) {
                        break address;
                    }
                }
            }
        })
        .await
        .expect("relay listen event");
        let mut metadata =
            RelayMetadata::new(identity.peer_id(), "1.0".to_owned(), "test".to_owned());
        metadata.listening_address = Some(address.to_string());
        metadata.status = if index == 2 {
            RelayStatus::Offline
        } else {
            RelayStatus::Online
        };
        metadata.health = HealthClass::Healthy;
        metadata.latency_ms = Some([30, 10, 3][index]);
        metadata.last_heartbeat = Some(SystemTime::now());
        relays.push(TestRelay {
            peer_id: identity.peer_id(),
            node,
            metadata,
            address: address.to_string(),
        });
    }

    let known_relays: Vec<_> = relays
        .iter()
        .map(|relay| (relay.peer_id, relay.address.clone()))
        .collect();
    for relay in relays {
        let mut node = relay.node;
        let metadata = relay.metadata;
        tokio::spawn(async move {
            while let Some(event) = node.swarm.next().await {
                if let Some(NetworkEvent::DiscoveryRequest {
                    channel, payload, ..
                }) = node.translate_event(event)
                {
                    let request: RelayMetadataRequest =
                        serde_json::from_slice(&payload).expect("metadata request");
                    assert_eq!(request.protocol_version, "1.0");
                    let response = serde_json::to_vec(&metadata).expect("metadata response");
                    node.respond_to_discovery(channel, response)
                        .expect("send metadata response");
                }
            }
        });
    }

    let client_identity =
        NodeIdentity::load_or_generate(directory.join("client.key")).expect("client identity");
    let mut client =
        NetworkNode::new(&client_identity, Duration::from_millis(50)).expect("client node");
    client
        .listen("/ip4/127.0.0.1/udp/0/quic-v1")
        .expect("client listen");
    for (peer_id, address) in &known_relays {
        client
            .dial(&format!("{address}/p2p/{peer_id}"))
            .expect("dial relay");
    }

    let requirements = CandidateRequirements {
        protocol_version: "1.0".to_owned(),
        required_transport: "quic".to_owned(),
        required_capabilities: vec!["relay".to_owned()],
        heartbeat_freshness: Duration::from_secs(30),
    };
    let mut responses = 0;
    let mut candidates = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), async {
        while responses < 3 {
            if let Some(event) = client.swarm.next().await {
                if let Some(event) = client.translate_event(event) {
                    match event {
                        NetworkEvent::PeerConnected { peer_id } => {
                            let request = RelayMetadataRequest {
                                kind: "metadata".to_owned(),
                                protocol_version: "1.0".to_owned(),
                            };
                            client.request_discovery(
                                peer_id,
                                serde_json::to_vec(&request).expect("request encoding"),
                            );
                        }
                        NetworkEvent::DiscoveryResponse { payload, .. } => {
                            responses += 1;
                            let metadata: RelayMetadata =
                                serde_json::from_slice(&payload).expect("metadata response");
                            if let Ok(candidate) = RelayCandidate::from_metadata(
                                &metadata,
                                &requirements,
                                SystemTime::now(),
                            ) {
                                candidates.push(candidate);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    })
    .await
    .expect("all three relay responses");

    assert_eq!(responses, 3);
    assert_eq!(candidates.len(), 2);
    let ranked = RelayRanking::rank(candidates, SystemTime::now());
    assert_eq!(ranked[0].latency, Some(Duration::from_millis(10)));
    assert_eq!(ranked[0].status, RelayStatus::Online);
    let client_peer = PeerId::random();
    let policy = RouteSelectionPolicy {
        mode: ghost_layer_network::RouteMode::OneHop,
        required_transport: "quic".to_owned(),
        required_capabilities: vec!["relay".to_owned()],
    };
    let one_hop = RouteSelector::select(client_peer, &ranked, &policy, SystemTime::now())
        .expect("one-hop route");
    assert!(matches!(one_hop, Route::OneHop { .. }));
    let two_hop_policy = RouteSelectionPolicy {
        mode: ghost_layer_network::RouteMode::TwoHop,
        ..policy
    };
    let first_two_hop =
        RouteSelector::select(client_peer, &ranked, &two_hop_policy, SystemTime::now())
            .expect("two-hop route");
    let second_two_hop =
        RouteSelector::select(client_peer, &ranked, &two_hop_policy, SystemTime::now())
            .expect("deterministic two-hop route");
    match (&first_two_hop, &second_two_hop) {
        (
            Route::TwoHop {
                entry: first_entry,
                exit: first_exit,
            },
            Route::TwoHop {
                entry: second_entry,
                exit: second_exit,
            },
        ) => {
            assert_ne!(first_entry.peer_id, first_exit.peer_id);
            assert_eq!(first_entry.peer_id, second_entry.peer_id);
            assert_eq!(first_exit.peer_id, second_exit.peer_id);
        }
        _ => panic!("expected two-hop routes"),
    }
    let _ = std::fs::remove_dir_all(directory);
}
