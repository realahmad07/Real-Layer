use ghost_layer_network::Multiaddr;
fn main() {
    let s = "/p2p/12D3KooWJ3K6Xz8r4K17w47jZk7Z2g1D1";
    let addr: Result<Multiaddr, _> = s.parse();
    println!("{:?}", addr);
}
