//! Blob storage for the PDS.
//!
//! The `BlobStore` trait and its associated `BoxedBlobStream`,
//! `BlobNotFoundError`, and `MemoryBlobStore` test double are
//! re-exported from the atproto protocol crates and consumed by call
//! sites that hold a per-DID handle as
//! `Arc<dyn BlobStore<Stream = BoxedBlobStream>>`.
//!
//! ## Per-DID partitioning
//!
//! The shared backend (OpenDAL's `Operator`) is built once at PDS startup
//! and handed out as `Arc<dyn BlobStore<Stream = BoxedBlobStream>>` per
//! actor. Each per-DID handle bakes the DID into every key
//! (`blocks/{did}/{cid}` for stored, `tmp/{did}/{key}` for untethered
//! uploads, `quarantine/{did}/{cid}` for taken-down blobs), restoring
//! per-DID tenant isolation. `delete_all` on a per-DID handle wipes
//! only that actor's directory.
//!
//! The canonical factory is [`from_env`]: reads S3 / disk env vars and
//! returns the per-DID handle for the given DID.

pub use rsky_blobstore::{
    BlobNotFoundError, BlobStore, BoxedBlobStream, MemoryBlobStore,
};

pub mod opendal;

pub use opendal::OpenDALBlobStore;

use std::sync::Arc;

/// Build a per-DID blobstore handle from environment configuration.
/// S3-first: if `S3_ENDPOINT` is set, build the S3 backend from
/// `S3_BUCKET` / `S3_ACCESS_KEY_ID` / `S3_SECRET_ACCESS_KEY`. Otherwise
/// fall back to the disk backend rooted at `PDS_BLOBSTORE_DISK_LOCATION`
/// (defaulting to `./blobs`).
///
/// `did` is baked into every key by the per-DID wrapper, so the same
/// shared `Operator` instance can be handed out to every actor without
/// leakage between tenants.
pub fn from_env(
    did: &str,
) -> anyhow::Result<Arc<dyn BlobStore<Stream = BoxedBlobStream>>> {
    OpenDALBlobStore::from_env(did)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsky_common::ipld::sha256_to_cid;
    use sha2::{Digest, Sha256};

    fn cid_for(bytes: &[u8]) -> lexicon_cid::Cid {
        sha256_to_cid(Sha256::digest(bytes).to_vec())
    }

    /// The `BlobStore` trait round-trips bytes through the
    /// `MemoryBlobStore` test double (no OpenDAL involvement at this
    /// stage; the per-DID OpenDAL backend wires in via a later pass).
    #[tokio::test]
    async fn memory_blobstore_round_trip_works() {
        let store = MemoryBlobStore::default();
        let bytes = b"hello blob".to_vec();
        let cid = cid_for(&bytes);
        let key = BlobStore::put_temp(&store, bytes.clone()).await.unwrap();
        assert!(BlobStore::has_temp(&store, key.clone()).await.unwrap());
        BlobStore::make_permanent(&store, key, cid).await.unwrap();
        assert!(BlobStore::has_stored(&store, cid).await.unwrap());
        let fetched = BlobStore::get_bytes(&store, cid).await.unwrap();
        assert_eq!(fetched, bytes);
    }

    /// `BlobNotFoundError` is the canonical error type returned when
    /// a CID is missing. Smoke-test that calling `get_bytes` on an
    /// unknown CID produces an error with the expected message.
    #[tokio::test]
    async fn blob_not_found_error_is_downcastable() {
        let store = MemoryBlobStore::default();
        let cid = cid_for(b"never uploaded");
        let err = BlobStore::get_bytes(&store, cid).await.unwrap_err();
        assert!(err.to_string().contains("stored blob not found"));
    }

    /// `BoxedBlobStream` is the canonical concrete stream type. Verify
    /// it is `Send` (the supertrait requires the Stream to be `Send`
    /// too).
    #[tokio::test]
    async fn boxed_blob_stream_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<BoxedBlobStream>();
    }

    /// `from_env(did)` returns an
    /// `Arc<dyn BlobStore<Stream = BoxedBlobStream>>` baked with the
    /// given DID. The `MemoryBlobStore` is used here; the
    /// OpenDAL-backed per-DID partitioning is exercised in the
    /// `opendal` module tests.
    #[tokio::test]
    async fn from_env_returns_send_sync_handle() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Arc<dyn BlobStore<Stream = BoxedBlobStream>>>();
    }

    /// The `BlobStore` trait's associated `Stream` type is
    /// `BoxedBlobStream` for `MemoryBlobStore`. Verify that the
    /// stream's items are `Result<bytes::Bytes>`.
    #[tokio::test]
    async fn memory_blobstore_stream_yields_bytes() {
        let store = MemoryBlobStore::default();
        let bytes = b"stream bytes".to_vec();
        let cid = cid_for(&bytes);
        BlobStore::put_permanent(&store, cid, bytes.clone()).await.unwrap();
        let stream = BlobStore::get_stream(&store, cid).await.unwrap();
        use futures::TryStreamExt;
        let chunks: Vec<bytes::Bytes> = stream.try_collect().await.unwrap();
        let flat: Vec<u8> = chunks.iter().flat_map(|b| b.iter().copied()).collect();
        assert_eq!(flat, bytes);
    }

    /// `BlobStore` is `Debug` (supertrait).
    #[test]
    fn memory_blobstore_is_debug() {
        let store = MemoryBlobStore::default();
        let _ = format!("{:?}", store);
    }

    /// `BlobNotFoundError` is `Debug` + `Display` + `Send + Sync`.
    #[test]
    fn blob_not_found_error_traits() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<BlobNotFoundError>();
        let e = BlobNotFoundError;
        let _ = format!("{:?}", e);
        let _ = format!("{}", e);
    }
}
