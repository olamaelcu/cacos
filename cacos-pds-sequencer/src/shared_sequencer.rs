//! Shared sequencer state mounted as poem state.
//!
//! The subscribe-repos websocket handler reads this to
//! sequence/commit/emit events. The `Arc<RwLock<…>>` indirection makes
//! `SharedSequencer: Clone` so it can be handed to poem's `.data(...)`
//! (which requires `Clone`) and still be mutated concurrently from
//! tests / background tasks via a second clone.

use crate::Sequencer;
use std::sync::{Arc, RwLock};

/// Shared sequencer handle, mounted as poem state.
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
