//! OpenDAL-backed [`BlobStore`] implementation.
//!
//! See the per-subagent implementation (Subagent 3) for the full impl.
//! This stub exists so the re-export in `mod.rs` resolves during the
//! module's initial scaffolding pass.

use std::fmt::Debug;

/// Shared OpenDAL-backed blobstore factory + per-DID handle wrapper.
///
/// This is the type that `mod.rs::from_env` returns per-DID. The full
/// impl lands in Subagent 3 of Plan 04; this stub exists so `mod.rs`
/// compiles and the dependency graph resolves end-to-end.
#[derive(Debug)]
pub struct OpenDALBlobStore {
    _placeholder: (),
}

impl OpenDALBlobStore {
    /// Placeholder until Subagent 3 lands. Returns an in-memory store
    /// so Subagent 2's compile check can run end-to-end.
    pub fn from_env(
        _did: &str,
    ) -> anyhow::Result<
        std::sync::Arc<
            dyn crate::blobstore::BlobStore<Stream = crate::blobstore::BoxedBlobStream>,
        >,
    > {
        Ok(std::sync::Arc::new(
            crate::blobstore::MemoryBlobStore::default(),
        ))
    }
}
