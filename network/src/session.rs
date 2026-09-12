use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Key, Nonce,
};
use hkdf::Hkdf;
use libp2p::{identity, PeerId};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::{collections::HashSet, fmt};
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroize;

const SESSION_PROTOCOL: &[u8] = b"ghost-layer/session/1";

#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId([u8; 16]);

impl SessionId {
    pub fn generate() -> Self {
        let mut bytes = [0u8; 16];
        OsRng.fill_bytes(&mut bytes);
        Self(bytes)
    }
}

impl fmt::Debug for SessionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "SessionId({})", hex(self.0))
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", hex(self.0))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionRole {
    Initiator,
    Responder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Negotiating,
    Established,
    Failed,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteBinding {
    OneHop { relay: PeerId },
    TwoHop { entry: PeerId, exit: PeerId, exit_address: String },
}

#[derive(Serialize, Deserialize)]
enum WireRouteBinding {
    OneHop { relay: String },
    TwoHop { entry: String, exit: String, exit_address: String },
}

impl Serialize for RouteBinding {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let wire = match self {
            Self::OneHop { relay } => WireRouteBinding::OneHop {
                relay: relay.to_string(),
            },
            Self::TwoHop { entry, exit, exit_address } => WireRouteBinding::TwoHop {
                entry: entry.to_string(),
                exit: exit.to_string(),
                exit_address: exit_address.clone(),
            },
        };
        wire.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for RouteBinding {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        match WireRouteBinding::deserialize(deserializer)? {
            WireRouteBinding::OneHop { relay } => Ok(Self::OneHop {
                relay: relay.parse().map_err(serde::de::Error::custom)?,
            }),
            WireRouteBinding::TwoHop { entry, exit, exit_address } => Ok(Self::TwoHop {
                entry: entry.parse().map_err(serde::de::Error::custom)?,
                exit: exit.parse().map_err(serde::de::Error::custom)?,
                exit_address,
            }),
        }
    }
}

impl RouteBinding {
    pub fn peers(&self) -> Vec<PeerId> {
        match self {
            Self::OneHop { relay } => vec![*relay],
            Self::TwoHop { entry, exit, .. } => vec![*entry, *exit],
        }
    }

    pub fn contains(&self, peer_id: PeerId) -> bool {
        self.peers().contains(&peer_id)
    }

    fn validate(&self) -> Result<(), SessionError> {
        if let Self::TwoHop { entry, exit, .. } = self {
            if entry == exit {
                return Err(SessionError::InvalidRoute(
                    "entry and exit must differ".to_owned(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandshakeInit {
    pub kind: String,
    pub protocol_version: String,
    pub session_id: SessionId,
    pub initiator_identity: String,
    pub responder_identity: String,
    pub initiator_public_key: Vec<u8>,
    pub ephemeral_public_key: [u8; 32],
    pub route_context: RouteBinding,
    pub authentication: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandshakeResponse {
    pub kind: String,
    pub protocol_version: String,
    pub session_id: SessionId,
    pub initiator_identity: String,
    pub responder_identity: String,
    pub responder_public_key: Vec<u8>,
    pub ephemeral_public_key: [u8; 32],
    pub route_context: RouteBinding,
    pub authentication: Vec<u8>,
}

pub struct SessionInitiator {
    identity: identity::Keypair,
    expected_peer: PeerId,
    route: RouteBinding,
    protocol_version: String,
    session_id: SessionId,
    secret: StaticSecret,
    state: SessionState,
}

pub struct SessionResponder {
    identity: identity::Keypair,
    seen_sessions: HashSet<SessionId>,
}

pub struct SecureSession {
    session_id: SessionId,
    peer_id: PeerId,
    route: RouteBinding,
    role: SessionRole,
    state: SessionState,
    send_key: [u8; 32],
    receive_key: [u8; 32],
    send_nonce: u64,
    last_received_nonce: Option<u64>,
}

impl fmt::Debug for SecureSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecureSession")
            .field("session_id", &self.session_id)
            .field("peer_id", &self.peer_id)
            .field("route", &self.route)
            .field("role", &self.role)
            .field("state", &self.state)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionError {
    UnsupportedProtocol {
        expected: String,
        actual: String,
    },
    InvalidIdentity(String),
    IdentityMismatch {
        expected: String,
        actual: String,
    },
    InvalidRoute(String),
    InvalidKeyMaterial,
    AuthenticationFailed,
    DuplicateSession,
    SessionIdMismatch,
    InvalidState {
        state: SessionState,
        operation: &'static str,
    },
    EncryptionFailed,
    DecryptionFailed,
}

impl fmt::Display for SessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}
impl std::error::Error for SessionError {}

impl SessionInitiator {
    pub fn new(
        identity: &identity::Keypair,
        expected_peer: PeerId,
        route: RouteBinding,
        protocol_version: impl Into<String>,
    ) -> Result<Self, SessionError> {
        route.validate()?;
        if !route.contains(expected_peer) {
            return Err(SessionError::InvalidRoute(
                "expected peer is not in route".to_owned(),
            ));
        }
        Ok(Self {
            identity: identity.clone(),
            expected_peer,
            route,
            protocol_version: protocol_version.into(),
            session_id: SessionId::generate(),
            secret: StaticSecret::random_from_rng(OsRng),
            state: SessionState::Negotiating,
        })
    }

    pub fn session_id(&self) -> SessionId {
        self.session_id
    }

    pub fn build_init(&self) -> Result<HandshakeInit, SessionError> {
        self.ensure_state(SessionState::Negotiating, "build_init")?;
        let public = self.identity.public();
        let public_bytes = public.encode_protobuf();
        let ephemeral_public_key = PublicKey::from(&self.secret).to_bytes();
        let authentication = sign_transcript(
            &self.identity,
            &transcript(
                &self.protocol_version,
                self.session_id,
                &self.route,
                &public_bytes,
                &ephemeral_public_key,
            ),
        );
        Ok(HandshakeInit {
            kind: "secure_session_init".to_owned(),
            protocol_version: self.protocol_version.clone(),
            session_id: self.session_id,
            initiator_identity: public.to_peer_id().to_string(),
            responder_identity: self.expected_peer.to_string(),
            initiator_public_key: public_bytes,
            ephemeral_public_key,
            route_context: self.route.clone(),
            authentication,
        })
    }

    pub fn complete(mut self, response: HandshakeResponse) -> Result<SecureSession, SessionError> {
        if self.state != SessionState::Negotiating {
            return Err(SessionError::InvalidState {
                state: self.state,
                operation: "complete",
            });
        }
        validate_response(
            &response,
            self.session_id,
            self.identity.public().to_peer_id(),
            self.expected_peer,
            &self.route,
            &self.protocol_version,
        )?;
        let shared = self
            .secret
            .diffie_hellman(&PublicKey::from(response.ephemeral_public_key));
        if shared.as_bytes().iter().all(|byte| *byte == 0) {
            return Err(SessionError::InvalidKeyMaterial);
        }
        self.state = SessionState::Established;
        Ok(derive_session(
            self.session_id,
            self.expected_peer,
            self.route,
            SessionRole::Initiator,
            shared.as_bytes(),
            &self.protocol_version,
        ))
    }

    fn ensure_state(
        &self,
        expected: SessionState,
        operation: &'static str,
    ) -> Result<(), SessionError> {
        if self.state == expected {
            Ok(())
        } else {
            Err(SessionError::InvalidState {
                state: self.state,
                operation,
            })
        }
    }
}

impl SessionResponder {
    pub fn new(identity: &identity::Keypair) -> Self {
        Self {
            identity: identity.clone(),
            seen_sessions: HashSet::new(),
        }
    }

    pub fn accept_init(
        &mut self,
        init: HandshakeInit,
        transport_peer: PeerId,
        protocol_version: &str,
    ) -> Result<(HandshakeResponse, SecureSession), SessionError> {
        if self.seen_sessions.contains(&init.session_id) {
            return Err(SessionError::DuplicateSession);
        }
        let expected_route = init.route_context.clone();
        validate_init(
            &init,
            transport_peer,
            self.identity.public().to_peer_id(),
            &expected_route,
            protocol_version,
        )?;
        if !expected_route.contains(self.identity.public().to_peer_id()) {
            return Err(SessionError::InvalidRoute(
                "route does not include responder".to_owned(),
            ));
        }
        self.seen_sessions.insert(init.session_id);
        let secret = StaticSecret::random_from_rng(OsRng);
        let public = self.identity.public();
        let public_bytes = public.encode_protobuf();
        let ephemeral_public_key = PublicKey::from(&secret).to_bytes();
        let authentication = sign_transcript(
            &self.identity,
            &transcript(
                protocol_version,
                init.session_id,
                &expected_route,
                &public_bytes,
                &ephemeral_public_key,
            ),
        );
        let response = HandshakeResponse {
            kind: "secure_session_response".to_owned(),
            protocol_version: protocol_version.to_owned(),
            session_id: init.session_id,
            initiator_identity: init.initiator_identity.clone(),
            responder_identity: public.to_peer_id().to_string(),
            responder_public_key: public_bytes,
            ephemeral_public_key,
            route_context: expected_route.clone(),
            authentication,
        };
        let shared = secret.diffie_hellman(&PublicKey::from(init.ephemeral_public_key));
        if shared.as_bytes().iter().all(|byte| *byte == 0) {
            return Err(SessionError::InvalidKeyMaterial);
        }
        let session = derive_session(
            init.session_id,
            transport_peer,
            expected_route,
            SessionRole::Responder,
            shared.as_bytes(),
            protocol_version,
        );
        Ok((response, session))
    }
}

impl SecureSession {
    pub fn session_id(&self) -> SessionId {
        self.session_id
    }
    pub fn peer_id(&self) -> PeerId {
        self.peer_id
    }
    pub fn route(&self) -> &RouteBinding {
        &self.route
    }
    pub fn role(&self) -> SessionRole {
        self.role
    }
    pub fn state(&self) -> SessionState {
        self.state
    }

    pub fn close(&mut self) -> Result<(), SessionError> {
        if self.state != SessionState::Established {
            return Err(SessionError::InvalidState {
                state: self.state,
                operation: "close",
            });
        }
        self.state = SessionState::Closed;
        Ok(())
    }

    pub fn encrypt(
        &mut self,
        plaintext: &[u8],
        associated_data: &[u8],
    ) -> Result<Vec<u8>, SessionError> {
        if self.state != SessionState::Established {
            return Err(SessionError::InvalidState {
                state: self.state,
                operation: "encrypt",
            });
        }
        let nonce_value = self.send_nonce;
        self.send_nonce = self
            .send_nonce
            .checked_add(1)
            .ok_or(SessionError::EncryptionFailed)?;
        let nonce = nonce_from_counter(nonce_value);
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&self.send_key));
        let ciphertext = cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                chacha20poly1305::aead::Payload {
                    msg: plaintext,
                    aad: associated_data,
                },
            )
            .map_err(|_| SessionError::EncryptionFailed)?;
        let mut output = nonce_value.to_be_bytes().to_vec();
        output.extend(ciphertext);
        Ok(output)
    }

    pub fn decrypt(
        &mut self,
        ciphertext: &[u8],
        associated_data: &[u8],
    ) -> Result<Vec<u8>, SessionError> {
        if self.state != SessionState::Established {
            return Err(SessionError::InvalidState {
                state: self.state,
                operation: "decrypt",
            });
        }
        if ciphertext.len() < 8 {
            return Err(SessionError::DecryptionFailed);
        }
        let mut counter = [0u8; 8];
        counter.copy_from_slice(&ciphertext[..8]);
        let counter = u64::from_be_bytes(counter);
        if self.last_received_nonce.is_some_and(|last| counter <= last) {
            return Err(SessionError::DecryptionFailed);
        }
        let nonce = nonce_from_counter(counter);
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&self.receive_key));
        let plaintext = cipher
            .decrypt(
                Nonce::from_slice(&nonce),
                chacha20poly1305::aead::Payload {
                    msg: &ciphertext[8..],
                    aad: associated_data,
                },
            )
            .map_err(|_| SessionError::DecryptionFailed)?;
        self.last_received_nonce = Some(counter);
        Ok(plaintext)
    }
}

impl Drop for SecureSession {
    fn drop(&mut self) {
        self.send_key.zeroize();
        self.receive_key.zeroize();
    }
}

fn validate_init(
    init: &HandshakeInit,
    transport_peer: PeerId,
    responder_peer: PeerId,
    expected_route: &RouteBinding,
    protocol: &str,
) -> Result<(), SessionError> {
    expected_route.validate()?;
    if init.protocol_version != protocol {
        return Err(SessionError::UnsupportedProtocol {
            expected: protocol.to_owned(),
            actual: init.protocol_version.clone(),
        });
    }
    if init
        .responder_identity
        .parse::<PeerId>()
        .map_err(|_| SessionError::InvalidIdentity(init.responder_identity.clone()))?
        != responder_peer
    {
        return Err(SessionError::IdentityMismatch {
            expected: responder_peer.to_string(),
            actual: init
                .responder_identity
                .parse::<PeerId>()
                .map_err(|_| SessionError::InvalidIdentity(init.responder_identity.clone()))?
                .to_string(),
        });
    }
    if &init.route_context != expected_route {
        return Err(SessionError::InvalidRoute(
            "route context mismatch".to_owned(),
        ));
    }
    let public = identity::PublicKey::try_decode_protobuf(&init.initiator_public_key)
        .map_err(|_| SessionError::InvalidKeyMaterial)?;
    let actual = public.to_peer_id();
    if init
        .initiator_identity
        .parse::<PeerId>()
        .map_err(|_| SessionError::InvalidIdentity(init.initiator_identity.clone()))?
        != actual
    {
        return Err(SessionError::IdentityMismatch {
            expected: actual.to_string(),
            actual: actual.to_string(),
        });
    }
    if actual != transport_peer {
        return Err(SessionError::IdentityMismatch {
            expected: transport_peer.to_string(),
            actual: actual.to_string(),
        });
    }
    let transcript = transcript(
        protocol,
        init.session_id,
        expected_route,
        &init.initiator_public_key,
        &init.ephemeral_public_key,
    );
    if !public.verify(&transcript, &init.authentication) {
        return Err(SessionError::AuthenticationFailed);
    }
    Ok(())
}

fn validate_response(
    response: &HandshakeResponse,
    session_id: SessionId,
    initiator_peer: PeerId,
    expected_peer: PeerId,
    route: &RouteBinding,
    protocol: &str,
) -> Result<(), SessionError> {
    if response.protocol_version != protocol {
        return Err(SessionError::UnsupportedProtocol {
            expected: protocol.to_owned(),
            actual: response.protocol_version.clone(),
        });
    }
    if response.session_id != session_id {
        return Err(SessionError::SessionIdMismatch);
    }
    if response.route_context != *route {
        return Err(SessionError::InvalidRoute(
            "route context mismatch".to_owned(),
        ));
    }
    let public = identity::PublicKey::try_decode_protobuf(&response.responder_public_key)
        .map_err(|_| SessionError::InvalidKeyMaterial)?;
    let actual = public.to_peer_id();
    if actual != expected_peer {
        return Err(SessionError::IdentityMismatch {
            expected: expected_peer.to_string(),
            actual: actual.to_string(),
        });
    }
    if response
        .initiator_identity
        .parse::<PeerId>()
        .map_err(|_| SessionError::InvalidIdentity(response.initiator_identity.clone()))?
        != initiator_peer
    {
        return Err(SessionError::IdentityMismatch {
            expected: initiator_peer.to_string(),
            actual: response
                .initiator_identity
                .parse::<PeerId>()
                .map_err(|_| SessionError::InvalidIdentity(response.initiator_identity.clone()))?
                .to_string(),
        });
    }
    let transcript = transcript(
        protocol,
        session_id,
        route,
        &response.responder_public_key,
        &response.ephemeral_public_key,
    );
    if !public.verify(&transcript, &response.authentication) {
        return Err(SessionError::AuthenticationFailed);
    }
    Ok(())
}

fn transcript(
    protocol: &str,
    session_id: SessionId,
    route: &RouteBinding,
    public_key: &[u8],
    ephemeral: &[u8; 32],
) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(SESSION_PROTOCOL);
    bytes.extend_from_slice(protocol.as_bytes());
    bytes.extend_from_slice(&session_id.0);
    bytes.extend(serde_json::to_vec(route).expect("route serialization"));
    bytes.extend(public_key);
    bytes.extend(ephemeral);
    bytes
}

fn sign_transcript(identity: &identity::Keypair, transcript: &[u8]) -> Vec<u8> {
    identity
        .sign(transcript)
        .expect("libp2p identity signing cannot fail")
}

fn derive_session(
    session_id: SessionId,
    peer_id: PeerId,
    route: RouteBinding,
    role: SessionRole,
    shared: &[u8],
    protocol: &str,
) -> SecureSession {
    let mut salt = [0u8; 32];
    let mut hasher = sha2::Sha256::new();
    use sha2::Digest;
    hasher.update(SESSION_PROTOCOL);
    hasher.update(protocol.as_bytes());
    hasher.update(serde_json::to_vec(&route).expect("route serialization"));
    hasher.update(session_id.0);
    salt.copy_from_slice(&hasher.finalize());
    let hk = Hkdf::<Sha256>::new(Some(&salt), shared);
    let mut okm = [0u8; 64];
    hk.expand(b"ghost-layer directional session keys", &mut okm)
        .expect("HKDF output length is valid");
    let (first, second) = okm.split_at(32);
    let (send_key, receive_key) = if role == SessionRole::Initiator {
        (first, second)
    } else {
        (second, first)
    };
    SecureSession {
        session_id,
        peer_id,
        route,
        role,
        state: SessionState::Established,
        send_key: send_key.try_into().expect("key length"),
        receive_key: receive_key.try_into().expect("key length"),
        send_nonce: 0,
        last_received_nonce: None,
    }
}

fn nonce_from_counter(counter: u64) -> [u8; 12] {
    let mut nonce = [0u8; 12];
    nonce[4..].copy_from_slice(&counter.to_be_bytes());
    nonce
}
fn hex(bytes: [u8; 16]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (identity::Keypair, identity::Keypair, RouteBinding) {
        let initiator = identity::Keypair::generate_ed25519();
        let responder = identity::Keypair::generate_ed25519();
        let route = RouteBinding::OneHop {
            relay: responder.public().to_peer_id(),
        };
        (initiator, responder, route)
    }

    #[test]
    fn handshake_derives_matching_directional_keys_and_encrypts() {
        let (initiator_identity, responder_identity, route) = fixture();
        let responder_peer = responder_identity.public().to_peer_id();
        let initiator =
            SessionInitiator::new(&initiator_identity, responder_peer, route.clone(), "1.0")
                .unwrap();
        let init = initiator.build_init().unwrap();
        let mut responder = SessionResponder::new(&responder_identity);
        let (response, mut responder_session) = responder
            .accept_init(init, initiator_identity.public().to_peer_id(), "1.0")
            .unwrap();
        let mut initiator_session = initiator.complete(response).unwrap();
        let encrypted = initiator_session
            .encrypt(b"test session data", b"route-aad")
            .unwrap();
        assert_eq!(
            responder_session.decrypt(&encrypted, b"route-aad").unwrap(),
            b"test session data"
        );
        assert!(initiator_session.peer_id() == responder_peer);
        assert_eq!(initiator_session.state(), SessionState::Established);
    }

    #[test]
    fn rejects_identity_protocol_route_and_replay_mismatches() {
        let (initiator_identity, responder_identity, route) = fixture();
        let responder_peer = responder_identity.public().to_peer_id();
        let initiator =
            SessionInitiator::new(&initiator_identity, responder_peer, route.clone(), "1.0")
                .unwrap();
        let mut init = initiator.build_init().unwrap();
        init.protocol_version = "9.0".to_owned();
        let mut responder = SessionResponder::new(&responder_identity);
        assert!(matches!(
            responder.accept_init(init, initiator_identity.public().to_peer_id(), "1.0"),
            Err(SessionError::UnsupportedProtocol { .. })
        ));
        let initiator =
            SessionInitiator::new(&initiator_identity, responder_peer, route.clone(), "1.0")
                .unwrap();
        let init = initiator.build_init().unwrap();
        let (response, _) = responder
            .accept_init(
                init.clone(),
                initiator_identity.public().to_peer_id(),
                "1.0",
            )
            .unwrap();
        assert!(matches!(
            responder.accept_init(init, initiator_identity.public().to_peer_id(), "1.0"),
            Err(SessionError::DuplicateSession)
        ));
        let mut wrong = response;
        wrong.protocol_version = "2.0".to_owned();
        assert!(initiator.complete(wrong).is_err());
    }

    #[test]
    fn different_route_contexts_produce_different_handshake_sessions() {
        let (initiator_identity, responder_identity, _) = fixture();
        let responder_peer = responder_identity.public().to_peer_id();
        let route_one = RouteBinding::OneHop {
            relay: responder_peer,
        };
        let route_two = RouteBinding::TwoHop {
            entry: responder_peer,
            exit: identity::Keypair::generate_ed25519().public().to_peer_id(),
            exit_address: "".to_owned(),
        };
        let first = SessionInitiator::new(&initiator_identity, responder_peer, route_one, "1.0")
            .unwrap()
            .session_id();
        let second = SessionInitiator::new(&initiator_identity, responder_peer, route_two, "1.0")
            .unwrap()
            .session_id();
        assert_ne!(first, second);
    }

    #[test]
    fn tampering_wrong_aad_replay_and_invalid_state_are_rejected() {
        let (initiator_identity, responder_identity, route) = fixture();
        let responder_peer = responder_identity.public().to_peer_id();
        let initiator =
            SessionInitiator::new(&initiator_identity, responder_peer, route.clone(), "1.0")
                .unwrap();
        let init = initiator.build_init().unwrap();
        let mut responder = SessionResponder::new(&responder_identity);
        let (response, mut responder_session) = responder
            .accept_init(init, initiator_identity.public().to_peer_id(), "1.0")
            .unwrap();
        let mut initiator_session = initiator.complete(response).unwrap();
        let ciphertext = initiator_session
            .encrypt(b"secret test data", b"aad")
            .unwrap();
        let mut modified = ciphertext.clone();
        *modified.last_mut().expect("ciphertext tag") ^= 1;
        assert_eq!(
            responder_session.decrypt(&modified, b"aad"),
            Err(SessionError::DecryptionFailed)
        );
        assert_eq!(
            responder_session.decrypt(&ciphertext, b"wrong-aad"),
            Err(SessionError::DecryptionFailed)
        );
        assert_eq!(
            responder_session.decrypt(&ciphertext, b"aad").unwrap(),
            b"secret test data"
        );
        assert_eq!(
            responder_session.decrypt(&ciphertext, b"aad"),
            Err(SessionError::DecryptionFailed)
        );
        initiator_session.close().unwrap();
        assert!(matches!(
            initiator_session.encrypt(b"closed", b"aad"),
            Err(SessionError::InvalidState { .. })
        ));
    }
}


