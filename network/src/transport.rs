use crate::peer::NodeIdentity;
use crate::RelayDiscoveryCodec;
use anyhow::{Context, Result};
use libp2p::{
    identify, ping, request_response,
    swarm::{NetworkBehaviour, Swarm, SwarmEvent},
    Multiaddr, SwarmBuilder,
};
use std::time::Duration;

#[derive(NetworkBehaviour)]
#[behaviour(to_swarm = "BehaviourEvent")]
pub struct Behaviour {
    pub identify: identify::Behaviour,
    pub ping: ping::Behaviour,
    pub discovery: request_response::Behaviour<crate::RelayDiscoveryCodec>,
}

#[derive(Debug)]
pub enum BehaviourEvent {
    Identify(Box<identify::Event>),
    Ping(ping::Event),
    Discovery(request_response::Event<Vec<u8>, Vec<u8>>),
}

impl From<identify::Event> for BehaviourEvent {
    fn from(event: identify::Event) -> Self {
        Self::Identify(Box::new(event))
    }
}

impl From<ping::Event> for BehaviourEvent {
    fn from(event: ping::Event) -> Self {
        Self::Ping(event)
    }
}

impl From<request_response::Event<Vec<u8>, Vec<u8>>> for BehaviourEvent {
    fn from(event: request_response::Event<Vec<u8>, Vec<u8>>) -> Self {
        Self::Discovery(event)
    }
}

pub struct NetworkNode {
    pub swarm: Swarm<Behaviour>,
}

#[derive(Debug)]
pub enum NetworkEvent {
    PeerConnected {
        peer_id: libp2p::PeerId,
    },
    PeerDisconnected {
        peer_id: libp2p::PeerId,
    },
    PeerIdentified {
        peer_id: libp2p::PeerId,
        agent_version: String,
    },
    PingResult {
        peer_id: libp2p::PeerId,
        result: Result<Duration, String>,
    },
    DiscoveryRequest {
        peer_id: libp2p::PeerId,
        request_id: request_response::InboundRequestId,
        payload: Vec<u8>,
        channel: request_response::ResponseChannel<Vec<u8>>,
    },
    DiscoveryResponse {
        peer_id: libp2p::PeerId,
        request_id: request_response::OutboundRequestId,
        payload: Vec<u8>,
    },
    Listening {
        address: Multiaddr,
    },
    NetworkError {
        message: String,
    },
}

impl NetworkNode {
    pub fn new(identity: &NodeIdentity, connection_timeout: Duration) -> Result<Self> {
        let public_key = identity.keypair().public();
        let swarm = SwarmBuilder::with_existing_identity(identity.keypair().clone())
            .with_tokio()
            .with_quic()
            .with_behaviour(|_| Behaviour {
                identify: identify::Behaviour::new(identify::Config::new(
                    "/ghost-layer/identify/1.0.0".to_owned(),
                    public_key.clone(),
                )),
                ping: ping::Behaviour::new(ping::Config::new().with_interval(connection_timeout)),
                discovery: request_response::Behaviour::with_codec(
                    RelayDiscoveryCodec,
                    std::iter::once((
                        crate::RelayDiscoveryProtocol,
                        request_response::ProtocolSupport::Full,
                    )),
                    request_response::Config::default(),
                ),
            })
            .context("create libp2p behaviour")?
            .build();
        Ok(Self { swarm })
    }

    pub fn listen(&mut self, address: &str) -> Result<()> {
        let address: Multiaddr = address.parse().context("parse listen multiaddr")?;
        self.swarm.listen_on(address)?;
        Ok(())
    }

    pub fn dial(&mut self, address: &str) -> Result<()> {
        let address: Multiaddr = address.parse().context("parse peer multiaddr")?;
        self.swarm.dial(address)?;
        Ok(())
    }

    pub fn request_discovery(
        &mut self,
        peer_id: libp2p::PeerId,
        payload: Vec<u8>,
    ) -> crate::DiscoveryRequestId {
        self.swarm
            .behaviour_mut()
            .discovery
            .send_request(&peer_id, payload)
    }

    pub fn respond_to_discovery(
        &mut self,
        channel: request_response::ResponseChannel<Vec<u8>>,
        payload: Vec<u8>,
    ) -> Result<()> {
        self.swarm
            .behaviour_mut()
            .discovery
            .send_response(channel, payload)
            .map_err(|error| anyhow::anyhow!("send discovery response: {error:?}"))
    }

    pub fn translate_event(&self, event: SwarmEvent<BehaviourEvent>) -> Option<NetworkEvent> {
        match event {
            SwarmEvent::NewListenAddr { address, .. } => Some(NetworkEvent::Listening { address }),
            SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                Some(NetworkEvent::PeerConnected { peer_id })
            }
            SwarmEvent::ConnectionClosed { peer_id, .. } => {
                Some(NetworkEvent::PeerDisconnected { peer_id })
            }
            SwarmEvent::Behaviour(BehaviourEvent::Identify(event)) => match *event {
                identify::Event::Received { peer_id, info, .. } => {
                    Some(NetworkEvent::PeerIdentified {
                        peer_id,
                        agent_version: info.agent_version,
                    })
                }
                _ => None,
            },
            SwarmEvent::Behaviour(BehaviourEvent::Ping(ping::Event { peer, result, .. })) => {
                Some(NetworkEvent::PingResult {
                    peer_id: peer,
                    result: result.map_err(|error| error.to_string()),
                })
            }
            SwarmEvent::Behaviour(BehaviourEvent::Discovery(
                request_response::Event::Message { peer, message, .. },
            )) => match message {
                request_response::Message::Request {
                    request_id,
                    request,
                    channel,
                } => Some(NetworkEvent::DiscoveryRequest {
                    peer_id: peer,
                    request_id,
                    payload: request,
                    channel,
                }),
                request_response::Message::Response {
                    request_id,
                    response,
                } => Some(NetworkEvent::DiscoveryResponse {
                    peer_id: peer,
                    request_id,
                    payload: response,
                }),
            },
            SwarmEvent::OutgoingConnectionError { error, .. } => Some(NetworkEvent::NetworkError {
                message: error.to_string(),
            }),
            SwarmEvent::IncomingConnectionError { error, .. } => Some(NetworkEvent::NetworkError {
                message: error.to_string(),
            }),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{NetworkEvent, NetworkNode};
    use crate::peer::NodeIdentity;
    use futures::StreamExt;
    use std::time::Duration;

    #[tokio::test]
    async fn two_local_quic_peers_connect_and_ping() {
        let directory =
            std::env::temp_dir().join(format!("ghost-layer-test-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("create test directory");
        let identity_a =
            NodeIdentity::load_or_generate(directory.join("a.key")).expect("identity a");
        let identity_b =
            NodeIdentity::load_or_generate(directory.join("b.key")).expect("identity b");
        let mut peer_a = NetworkNode::new(&identity_a, Duration::from_millis(100)).expect("node a");
        let mut peer_b = NetworkNode::new(&identity_b, Duration::from_millis(100)).expect("node b");
        peer_a
            .listen("/ip4/127.0.0.1/udp/0/quic-v1")
            .expect("listen a");
        peer_b
            .listen("/ip4/127.0.0.1/udp/0/quic-v1")
            .expect("listen b");

        let listen_b = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(event) = peer_b.swarm.next().await {
                    if let Some(NetworkEvent::Listening { address }) = peer_b.translate_event(event)
                    {
                        break address;
                    }
                }
            }
        })
        .await
        .expect("peer b listen event");
        peer_a
            .dial(&format!("{listen_b}/p2p/{}", identity_b.peer_id()))
            .expect("dial b");

        let mut connected = false;
        let mut identified = false;
        let mut pinged = false;
        tokio::time::timeout(Duration::from_secs(10), async {
            while !(connected && identified && pinged) {
                tokio::select! {
                    Some(event) = peer_a.swarm.next() => if let Some(event) = peer_a.translate_event(event) {
                        match event {
                            NetworkEvent::PeerConnected { peer_id } if peer_id == identity_b.peer_id() => connected = true,
                            NetworkEvent::PeerIdentified { peer_id, .. } if peer_id == identity_b.peer_id() => identified = true,
                            NetworkEvent::PingResult { peer_id, result } if peer_id == identity_b.peer_id() && result.is_ok() => pinged = true,
                            _ => {}
                        }
                    },
                    Some(event) = peer_b.swarm.next() => { let _ = peer_b.translate_event(event); },
                }
            }
        }).await.expect("peers connect, identify, and ping");

        let _ = std::fs::remove_dir_all(directory);
    }
}
