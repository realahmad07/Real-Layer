use ghost_layer_network::NodeIdentity;

fn main() {
    let a = NodeIdentity::load_or_generate("target/relay-acceptance.key").unwrap();
    let b = NodeIdentity::load_or_generate("target/relay-test.key").unwrap();

    println!("ENTRY_PEER_ID={}", a.peer_id());
    println!("EXIT_PEER_ID={}", b.peer_id());
}
