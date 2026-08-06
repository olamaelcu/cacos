//! Shared types and row-mapping helpers for the blob module.

use crate::actor_store::db::ActorDb;
use crate::background::BackgroundQueue;
use crate::blobstore::{BlobStore, BoxedBlobStream};
use anyhow::Result;
use lexicon_cid::Cid;
use sea_orm::QueryResult;
use std::sync::Arc;

pub struct BlobMetadata {
    pub temp_key: String,
    pub size: i64,
    pub cid: Cid,
    pub mime_type: String,
    pub width: Option<i32>,
    pub height: Option<i32>,
}

pub struct BlobReader {
    /// Per-DID handle built via `OpenDALBlobStore::new(op, did)` and
    /// passed in by `ActorStore::read(did, blobstore)`. The shared
    /// `Operator` lives on the PDS process; this handle bakes the
    /// DID into every key.
    pub blobstore: Arc<dyn BlobStore<Stream = BoxedBlobStream>>,
    pub db: ActorDb,
    pub background_queue: BackgroundQueue,
}

pub struct ListMissingBlobsOpts {
    pub cursor: Option<String>,
    pub limit: u16,
}

pub struct ListBlobsOpts {
    pub since: Option<String>,
    pub cursor: Option<String>,
    pub limit: u16,
}

pub struct GetBlobOutput {
    pub size: i64,
    pub mime_type: Option<String>,
    /// Canonical stream type; downstream handlers `try_collect` to
    /// `Vec<Bytes>` then flatten to `Vec<u8>`.
    pub stream: BoxedBlobStream,
}

pub struct GetBlobMetadataOutput {
    pub size: i64,
    pub mime_type: Option<String>,
}

/// Mirror of a `blob` table row. A private struct (rather than the
/// Plan 01 entity) so the raw-SQL port never depends on entity
/// module paths; the explicit-column select below keeps the column
/// order fixed.
#[derive(Debug, Clone)]
pub struct BlobRow {
    pub cid: String,
    pub mime_type: String,
    pub size: i64,
    pub temp_key: Option<String>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub created_at: String,
    pub takedown_ref: Option<String>,
}

/// Columns in the exact order `row_to_blob` expects.
pub(super) const BLOB_SELECT: &str = "SELECT cid, \"mimeType\", size, \"tempKey\", width, height, \"createdAt\", \"takedownRef\" FROM blob";

pub(super) fn row_to_blob(row: &QueryResult) -> Result<BlobRow> {
    Ok(BlobRow {
        cid: row.try_get_by_index::<String>(0)?,
        mime_type: row.try_get_by_index::<String>(1)?,
        size: row.try_get_by_index::<i64>(2)?,
        temp_key: row.try_get_by_index::<Option<String>>(3)?,
        width: row.try_get_by_index::<Option<i32>>(4)?,
        height: row.try_get_by_index::<Option<i32>>(5)?,
        created_at: row.try_get_by_index::<String>(6)?,
        takedown_ref: row.try_get_by_index::<Option<String>>(7)?,
    })
}

impl BlobReader {
    /// `blobstore` is the per-DID handle built via
    /// `OpenDALBlobStore::new(op, did)` inside
    /// `ActorStore::read(did, blobstore)`. The handle bakes the DID
    /// into every key (`blocks/{did}/{cid}`, `tmp/{did}/{key}`,
    /// `quarantine/{did}/{cid}`) and overrides the `BlobStore` default
    /// `delete_all()` to return `Some(future)` that wipes the
    /// actor's three prefixes in one operation.
    pub fn new(
        blobstore: Arc<dyn BlobStore<Stream = BoxedBlobStream>>,
        db: ActorDb,
        background_queue: BackgroundQueue,
    ) -> Self {
        BlobReader {
            blobstore,
            db,
            background_queue,
        }
    }
}
