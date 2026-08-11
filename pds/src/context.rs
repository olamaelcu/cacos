//! Shared sequencer state used across helpers and HTTP handlers.
//!
//! The service-signing keypairs (`PDS_REPO_SIGNING_KEYPAIR`,
//! `PDS_PLC_ROTATION_KEYPAIR`, `PDS_JWT_KEYPAIR`) live in
//! `cacos_pds_account::auth::*` since Plan 00 Step 3.

use crate::sequencer::Sequencer;
use std::sync::{Arc, RwLock};

/// Shared sequencer handle, mounted as poem state. The subscribe-repos
/// websocket handler reads this to sequence/commit/emit events. The
/// `Arc<RwLock<…>>` indirection makes `SharedSequencer: Clone` so it can be
/// handed to poem's `.data(...)` (which requires `Clone`) and still be
/// mutated concurrently from tests / background tasks via a second clone.
///
/// Step 4's `cacos-pds-sequencer` extraction will move this type out of
/// `cacos-pds` entirely.
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
