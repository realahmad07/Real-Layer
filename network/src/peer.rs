use anyhow::{Context, Result};
use libp2p::{identity, PeerId};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

pub use libp2p::PeerId as Libp2pPeerId;

/// Persistent Ed25519 identity for a Ghost Layer node.
pub struct NodeIdentity {
    keypair: identity::Keypair,
}

impl NodeIdentity {
    pub fn load_or_generate(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if path.exists() {
            let bytes =
                fs::read(path).with_context(|| format!("read identity from {}", path.display()))?;
            let keypair = identity::Keypair::from_protobuf_encoding(&bytes)
                .context("decode persisted libp2p identity")?;
            return Ok(Self { keypair });
        }

        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)
                .with_context(|| format!("create identity directory {}", parent.display()))?;
        }
        let keypair = identity::Keypair::generate_ed25519();
        let bytes = keypair
            .to_protobuf_encoding()
            .context("encode libp2p identity")?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .with_context(|| format!("create identity at {}", path.display()))?;
        file.write_all(&bytes)
            .context("write persisted libp2p identity")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        }
        Ok(Self { keypair })
    }

    pub fn keypair(&self) -> &identity::Keypair {
        &self.keypair
    }

    pub fn peer_id(&self) -> PeerId {
        self.keypair.public().to_peer_id()
    }
}

#[cfg(test)]
mod tests {
    use super::NodeIdentity;

    #[test]
    fn identity_persists_across_loads() {
        let path = std::env::temp_dir().join(format!("ghost-layer-{}.key", std::process::id()));
        let first = NodeIdentity::load_or_generate(&path).expect("generate identity");
        let first_peer = first.peer_id();
        drop(first);
        let second = NodeIdentity::load_or_generate(&path).expect("load identity");
        assert_eq!(first_peer, second.peer_id());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn invalid_identity_fails_without_generating_a_replacement() {
        let path =
            std::env::temp_dir().join(format!("ghost-layer-invalid-{}.key", std::process::id()));
        std::fs::write(&path, b"not-a-libp2p-key").expect("write invalid identity");
        assert!(NodeIdentity::load_or_generate(&path).is_err());
        let _ = std::fs::remove_file(path);
    }
}
