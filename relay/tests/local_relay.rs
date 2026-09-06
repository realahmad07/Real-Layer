use futures::StreamExt;
use ghost_layer_network::{NetworkEvent, NetworkNode, NodeIdentity};
use ghost_layer_relay::{RelayHeartbeat, RelayState};
use std::time::{Duration, SystemTime};

#[tokio::test]
async fn local_peers_update_relay_state_and_heartbeat() {
    let directory =
        std::env::temp_dir().join(format!("ghost-layer-relay-test-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("create test directory");
    let identity_a = NodeIdentity::load_or_generate(directory.join("a.key")).expect("identity a");
    let identity_b = NodeIdentity::load_or_generate(directory.join("b.key")).expect("identity b");
    let mut node_a = NetworkNode::new(&identity_a, Duration::from_millis(50)).expect("node a");
    let mut node_b = NetworkNode::new(&identity_b, Duration::from_millis(50)).expect("node b");
    node_a
        .listen("/ip4/127.0.0.1/udp/0/quic-v1")
        .expect("listen a");
    node_b
        .listen("/ip4/127.0.0.1/udp/0/quic-v1")
        .expect("listen b");

    let address_b = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(event) = node_b.swarm.next().await {
                if let Some(NetworkEvent::Listening { address }) = node_b.translate_event(event) {
                    break address;
                }
            }
        }
    })
    .await
    .expect("listen event");
    node_a
        .dial(&format!("{address_b}/p2p/{}", identity_b.peer_id()))
        .expect("dial b");

    let mut state = RelayState::new(identity_a.peer_id(), None, "test".to_owned());
    let mut connected = false;
    let mut identified = false;
    let mut latency_recorded = false;
    tokio::time::timeout(Duration::from_secs(10), async {
        while !(connected && identified && latency_recorded) {
            tokio::select! {
                Some(event) = node_a.swarm.next() => if let Some(event) = node_a.translate_event(event) {
                    state.apply_network_event(&event, SystemTime::now());
                    match event {
                        NetworkEvent::PeerConnected { peer_id } if peer_id == identity_b.peer_id() => connected = true,
                        NetworkEvent::PeerIdentified { peer_id, .. } if peer_id == identity_b.peer_id() => identified = true,
                        NetworkEvent::PingResult { peer_id, result } if peer_id == identity_b.peer_id() && result.is_ok() => latency_recorded = true,
                        _ => {}
                    }
                },
                Some(event) = node_b.swarm.next() => { let _ = node_b.translate_event(event); },
            }
        }
    }).await.expect("connect, identify, and ping");

    let heartbeat = RelayHeartbeat::from_state(&mut state, SystemTime::now());
    assert_eq!(heartbeat.active_peer_count, 1);
    assert!(heartbeat.latency.is_some());
    assert!(state.last_heartbeat().is_some());
    assert_eq!(state.total_successful_connections(), 1);
    let _ = std::fs::remove_dir_all(directory);
}
