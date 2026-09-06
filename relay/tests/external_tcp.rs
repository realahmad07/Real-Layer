use ghost_layer_relay::{
    DestinationPolicy, ExitNetworkAdapter, TcpAdapterConfig, TcpExitNetworkAdapter,
};
use std::time::Duration;

#[test]
fn opt_in_external_tcp_destination_round_trip() {
    let Ok(destination) = std::env::var("GHOST_EXTERNAL_TEST_DESTINATION") else {
        println!("EXTERNAL INTERNET TEST SKIPPED — NO AUTHORIZED DESTINATION CONFIGURED");
        return;
    };
    let allowlist = std::env::var("GHOST_ALLOWED_EXIT_DESTINATIONS")
        .expect("external destination requires GHOST_ALLOWED_EXIT_DESTINATIONS");
    let destinations: Vec<String> = allowlist
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect();
    let policy = DestinationPolicy::from_strings(&destinations)
        .expect("external allowlist must contain literal IP:PORT values");
    policy
        .validate(&destination)
        .expect("external test destination must be explicitly allowlisted");
    let mut adapter = TcpExitNetworkAdapter::connect(
        &destination,
        &policy,
        TcpAdapterConfig {
            maximum_request_size: 64,
            maximum_response_size: 256,
            connection_timeout: Duration::from_secs(3),
            read_timeout: Duration::from_secs(3),
            write_timeout: Duration::from_secs(3),
        },
    )
    .expect("connect to approved external TCP test endpoint");
    let response = adapter
        .exchange(b"ghost-layer-external-test")
        .expect("external TCP test request/response");
    if let Ok(expected) = std::env::var("GHOST_EXTERNAL_TEST_RESPONSE") {
        assert_eq!(response, expected.as_bytes());
    } else {
        println!(
            "authorized external TCP response received: {} bytes",
            response.len()
        );
    }
    adapter.close().expect("close external TCP adapter");
}
