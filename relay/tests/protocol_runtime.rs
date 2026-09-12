use ghost_layer_network::{NodeIdentity, PeerId};
use std::net::{SocketAddr, UdpSocket};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn binary(name: &str) -> PathBuf {
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target")
        .join("debug")
        .join(format!("{name}{suffix}"))
}

fn multiaddr(port: u16, peer: PeerId) -> String {
    format!("/ip4/127.0.0.1/udp/{port}/quic-v1/p2p/{peer}")
}

#[allow(clippy::too_many_arguments)]
fn spawn_relay(
    binary: &PathBuf,
    listen_port: u16,
    identity_path: &PathBuf,
    bootstrap: &str,
    destination: SocketAddr,
    udp: bool,
    dns: Option<SocketAddr>,
    external: Option<SocketAddr>,
) -> Child {
    let mut command = Command::new(binary);
    command
        .env(
            "GHOST_LISTEN_ADDRESS",
            format!("/ip4/127.0.0.1/udp/{listen_port}/quic-v1"),
        )
        .env("GHOST_IDENTITY_PATH", identity_path)
        .env("GHOST_BOOTSTRAP_PEERS", bootstrap)
        .env("GHOST_NETWORK_ENVIRONMENT", "development")
        .env("GHOST_UDP_ENABLED", udp.to_string())
        .env("GHOST_NAT_ENABLED", udp.to_string())
        .env("GHOST_DNS_ENABLED", dns.is_some().to_string())
        .env("GHOST_ALLOWED_EXIT_DESTINATIONS", destination.to_string())
        .env(
            "GHOST_DNS_SERVER",
            dns.map(|address| address.to_string()).unwrap_or_default(),
        )
        .env(
            "GHOST_EXTERNAL_TEST_DESTINATION",
            external
                .map(|address| address.to_string())
                .unwrap_or_default(),
        )
        .env("RUST_LOG", "warn")
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    command.spawn().expect("spawn relay")
}

fn wait_for_client(mut client: Child, relays: &mut [Child]) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(status) = client.try_wait().expect("poll client") {
            assert!(status.success(), "client exited with {status}");
            break;
        }
        if Instant::now() >= deadline {
            let _ = client.kill();
            let _ = client.wait();
            for relay in relays.iter_mut() {
                let _ = relay.kill();
                let _ = relay.wait();
            }
            panic!("client runtime timed out");
        }
        thread::sleep(Duration::from_millis(100));
    }
    for relay in relays {
        let _ = relay.kill();
        let _ = relay.wait();
    }
}

fn dns_response(request: &[u8]) -> Vec<u8> {
    let mut response = request[..12].to_vec();
    response[2] = 0x81;
    response[3] = 0x80;
    response[6] = 0;
    response[7] = 1;
    response.extend_from_slice(&request[12..]);
    response.extend_from_slice(&[0xc0, 0x0c, 0, 1, 0, 1, 0, 0, 0, 30, 0, 4, 127, 0, 0, 1]);
    response
}

fn run_runtime(udp: bool) {
    let run_id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    let base_port = 20000 + (run_id % 1000) as u16;
    let directory = std::env::temp_dir().join(format!("ghost-layer-runtime-{run_id}"));
    std::fs::create_dir_all(&directory).expect("create runtime directory");
    let client_key = directory.join("client.key");
    let entry_key = directory.join("entry.key");
    let exit_key = directory.join("exit.key");
    NodeIdentity::load_or_generate(&client_key).expect("client identity");
    let entry_identity = NodeIdentity::load_or_generate(&entry_key).expect("entry identity");
    let exit_identity = NodeIdentity::load_or_generate(&exit_key).expect("exit identity");
    let server = UdpSocket::bind("127.0.0.1:0").expect("bind protocol server");
    server
        .set_read_timeout(Some(Duration::from_secs(15)))
        .expect("set server timeout");
    let server_address = server.local_addr().expect("server address");
    let server_thread = thread::spawn(move || {
        let mut request = [0u8; 2048];
        let (size, peer) = server
            .recv_from(&mut request)
            .expect("receive protocol request");
        let response = if udp {
            assert_eq!(&request[..size], b"hello ghost layer udp");
            b"ack: hello ghost layer udp".to_vec()
        } else {
            dns_response(&request[..size])
        };
        server
            .send_to(&response, peer)
            .expect("send protocol response");
    });
    let relay_binary = binary("ghost-layer-relay");
    let client_binary = binary("ghost-layer-client");
    assert!(relay_binary.exists(), "relay binary is not built");
    assert!(client_binary.exists(), "client binary is not built");
    let exit_port = base_port + 2;
    let entry_port = base_port + 1;
    let client_port = base_port + 3;
    let exit_bootstrap = multiaddr(exit_port, exit_identity.peer_id());
    let entry_bootstrap = multiaddr(entry_port, entry_identity.peer_id());
    let dns = (!udp).then_some(server_address);
    let exit = spawn_relay(
        &relay_binary,
        exit_port,
        &exit_key,
        "",
        server_address,
        udp,
        dns,
        None,
    );
    let entry = spawn_relay(
        &relay_binary,
        entry_port,
        &entry_key,
        &exit_bootstrap,
        server_address,
        udp,
        dns,
        None,
    );
    let client = Command::new(&client_binary)
        .env(
            "GHOST_LISTEN_ADDRESS",
            format!("/ip4/127.0.0.1/udp/{client_port}/quic-v1"),
        )
        .env("GHOST_IDENTITY_PATH", &client_key)
        .env(
            "GHOST_BOOTSTRAP_PEERS",
            format!("{entry_bootstrap},{exit_bootstrap}"),
        )
        .env("GHOST_NETWORK_ENVIRONMENT", "development")
        .env("GHOST_ROUTE_MODE", "two-hop")
        .env("GHOST_UDP_ENABLED", udp.to_string())
        .env("GHOST_NAT_ENABLED", udp.to_string())
        .env("GHOST_DNS_ENABLED", (!udp).to_string())
        .env(
            "GHOST_DNS_SERVER",
            if udp {
                "".to_owned()
            } else {
                server_address.to_string()
            },
        )
        .env(
            "GHOST_ALLOWED_EXIT_DESTINATIONS",
            server_address.to_string(),
        )
        .env("RUST_LOG", "warn")
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn client");
    wait_for_client(client, &mut [entry, exit]);
    server_thread.join().expect("join protocol server");
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn client_entry_exit_udp_runtime_round_trip() {
    run_runtime(true);
}

#[test]
fn client_entry_exit_dns_runtime_round_trip() {
    run_runtime(false);
}

#[test]
fn client_entry_exit_tcp_runtime_round_trip() {
    let destination: SocketAddr = "8.8.8.8:53".parse().unwrap();
    if std::net::TcpStream::connect_timeout(&destination, Duration::from_millis(500)).is_err() {
        println!("IPHONE TCP RUNTIME SKIPPED — 8.8.8.8:53 UNAVAILABLE");
        return;
    }
    let run_id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    let base_port = 21000 + (run_id % 1000) as u16;
    let directory = std::env::temp_dir().join(format!("ghost-layer-tcp-runtime-{run_id}"));
    std::fs::create_dir_all(&directory).expect("create TCP runtime directory");
    let client_key = directory.join("client.key");
    let entry_key = directory.join("entry.key");
    let exit_key = directory.join("exit.key");
    let entry_identity = NodeIdentity::load_or_generate(&entry_key).expect("entry identity");
    let exit_identity = NodeIdentity::load_or_generate(&exit_key).expect("exit identity");
    NodeIdentity::load_or_generate(&client_key).expect("client identity");
    let relay_binary = binary("ghost-layer-relay");
    let client_binary = binary("ghost-layer-client");
    let exit_port = base_port + 2;
    let entry_port = base_port + 1;
    let exit_bootstrap = multiaddr(exit_port, exit_identity.peer_id());
    let entry_bootstrap = multiaddr(entry_port, entry_identity.peer_id());
    let exit = spawn_relay(
        &relay_binary,
        exit_port,
        &exit_key,
        "",
        destination,
        false,
        None,
        Some(destination),
    );
    let entry = spawn_relay(
        &relay_binary,
        entry_port,
        &entry_key,
        &exit_bootstrap,
        destination,
        false,
        None,
        Some(destination),
    );
    let client = Command::new(&client_binary)
        .env(
            "GHOST_LISTEN_ADDRESS",
            format!("/ip4/127.0.0.1/udp/{}/quic-v1", base_port + 3),
        )
        .env("GHOST_IDENTITY_PATH", &client_key)
        .env(
            "GHOST_BOOTSTRAP_PEERS",
            format!("{entry_bootstrap},{exit_bootstrap}"),
        )
        .env("GHOST_NETWORK_ENVIRONMENT", "development")
        .env("GHOST_ROUTE_MODE", "two-hop")
        .env("GHOST_EXTERNAL_TEST_DESTINATION", destination.to_string())
        .env("GHOST_ALLOWED_EXIT_DESTINATIONS", destination.to_string())
        .env("RUST_LOG", "warn")
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn TCP client");
    wait_for_client(client, &mut [entry, exit]);
    let _ = std::fs::remove_dir_all(directory);
}

