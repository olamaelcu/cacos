//! Process-wide keypairs and shared sequencer state used across helpers
//! and HTTP handlers.

use crate::sequencer::Sequencer;
use secp256k1::{Keypair, Secp256k1, SecretKey};
use std::env;
use std::sync::{Arc, LazyLock, RwLock};

/// Signs service JWTs and is exposed as the PDS's repo signing key by the
/// server handlers.
pub static PDS_REPO_SIGNING_KEYPAIR: LazyLock<Keypair> = LazyLock::new(|| {
    let secp = Secp256k1::new();
    let private_key = env::var("PDS_REPO_SIGNING_KEY_K256_PRIVATE_KEY_HEX").unwrap();
    let secret_key = SecretKey::from_slice(&hex::decode(private_key.as_bytes()).unwrap()).unwrap();
    Keypair::from_secret_key(&secp, &secret_key)
});

/// Shared sequencer handle, mounted as poem state. The subscribe-repos
/// websocket handler reads this to sequence/commit/emit events. The
/// `Arc<RwLock<…>>` indirection makes `SharedSequencer: Clone` so it can be
/// handed to poem's `.data(...)` (which requires `Clone`) and still be
/// mutated concurrently from tests / background tasks via a second clone.
pub struct SharedSequencer {
    pub sequencer: Arc<RwLock<Sequencer>>,
}

impl SharedSequencer {
    pub fn new(sequencer: Sequencer) -> Self {
        Self {
            sequencer: Arc::new(RwLock::new(sequencer)),
        }
    }
}

impl Clone for SharedSequencer {
    fn clone(&self) -> Self {
        Self {
            sequencer: Arc::clone(&self.sequencer),
        }
    }
}
