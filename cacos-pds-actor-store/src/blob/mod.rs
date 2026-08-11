//! Blob metadata lifecycle: the `blob` / `record_blob` rows plus the
//! blobstore state transitions.
//!
//! Every row in `blob` (and the `record_blob` join) is the metadata
//! half of an atproto blob; the bytes themselves live in the
//! `OpenDALBlobStore` handed to [`BlobReader::new`] as a per-DID
//! handle. [`BlobReader`] mediates the lifecycle: upload + track
//! (untethered buffer in temp storage), `verify_blob_and_make_permanent`
//! (commit bytes to permanent storage and link by CID), takedown
//! (`quarantine` moves the blob out of the active namespace), and
//! dereference / list / missing-blob queries over the per-actor DB.
//!
//! ## Layout
//!
//! `mod.rs` is the re-export hub. The `impl BlobReader` blocks are
//! split by concern:
//! - [`types`] — shared structs + the row mapping helpers +
//!   [`BlobReader::new`].
//! - [`read`] — single-row lookups, counts, and paged listing
//!   (`get_blob`, `list_blobs`, `list_missing_blobs`, ...).
//! - [`write`] — upload/track/promote/associate plus the write-batch
//!   dereference/GC pass (`process_write_blobs`,
//!   `delete_dereferenced_blobs`). Also hosts the free helpers
//!   [`verify_blob`], [`accepted_mime`], [`sha256_stream`].
//! - [`takedown`] — the takedown/quarantine transactor and its status
//!   reporter.
//!
//! SQL text and error messages are preserved verbatim (including the
//! historic "takendown" typo); only the rusqlite `?1` numbered
//! placeholders become sea-orm positional `?`.

pub mod read;
pub mod takedown;
pub mod types;
pub mod write;

#[cfg(test)]
pub mod tests;

pub use types::{
    BlobMetadata, BlobReader, GetBlobMetadataOutput, GetBlobOutput, ListBlobsOpts,
    ListMissingBlobsOpts,
};
pub use write::{accepted_mime, sha256_stream, verify_blob};
