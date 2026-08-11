//! OpenDAL-backed [`BlobStore`] implementation.
//!
//! One shared OpenDAL [`Operator`](opendal::Operator) is built at PDS startup
//! (S3, filesystem, or in-memory backend) and handed out to every actor as a
//! per-DID `OpenDALBlobStore`. The DID is baked into every key the handle
//! emits, so a single `Operator` can serve every tenant while keeping their
//! blobs isolated to their own subtrees.
//!
//! ## Key layout
//!
//! Every blob key carries the owning DID as a path component:
//!
//! - `tmp/{did}/{key}` — untethered upload buffer; `key` is a 40-char hex
//!   string minted by `put_temp` and returned to the caller for later
//!   `make_permanent`.
//! - `blocks/{did}/{cid}` — committed blob, addressable by CID.
//! - `quarantine/{did}/{cid}` — takedown staging area; `quarantine` renames a
//!   blob from `blocks/` into here and `unquarantine` reverses it.
//!
//! `delete_all` on a per-DID handle wipes only that actor's three prefixes.
//!
//! ## Streaming reads
//!
//! [`get_stream`](BlobStore::get_stream) materializes the OpenDAL
//! [`Reader`](opendal::Reader) into a
//! [`futures::AsyncRead`](futures::AsyncRead) via
//! [`into_futures_async_read`](opendal::Reader::into_futures_async_read) and
//! then drives a `futures::stream::poll_fn` over `AsyncRead::poll_read`. This
//! avoids pulling in `tokio-util` for a single `ReaderStream` adapter and keeps
//! the trait `Stream` item shape (`Result<bytes::Bytes>`).

use std::pin::Pin;
use std::sync::Arc;
use std::task::Poll;

use anyhow::{Context, Result};
use bytes::Bytes;
use futures::future::BoxFuture;
use futures::io::AsyncRead;
use futures::stream::{StreamExt, TryStreamExt};
use lexicon_cid::Cid;
pub use opendal::Operator;
use opendal::{ErrorKind, services};
use rand::RngCore;

use crate::{BlobNotFoundError, BlobStore, BoxedBlobStream};

/// Chunk size for `delete_many` error counting. OpenDAL batches deletes
/// server-side for S3; for local FS each call is one syscall.
const DELETE_MANY_CHUNK_SIZE: usize = 500;

/// Metric name registered with the `metrics` facade for each blobstore
/// operation. The host process's Prometheus recorder is responsible
/// for emitting samples with HELP lines; this crate does not install a
/// recorder itself. The constant strings here must match the names
/// used by the PDS observability module's `describe()` calls.
pub const BLOB_OPS_TOTAL: &str = "cacos_blob_ops_total";
pub const BLOB_PUT_BYTES: &str = "cacos_blob_put_bytes";
pub const BLOB_GET_BYTES: &str = "cacos_blob_get_bytes";

/// Local stub for the PDS-level `timed()` helper. The full PDS
/// observability integration (tracing-timing percentiles, the
/// `cacos.stage` span, stage histograms) lives in the caller; here we
/// only need the wrapped future's output preserved. The blobstore
/// still emits per-operation counters and byte-size histograms via
/// the `metrics` facade, so those metrics are visible to whatever
/// recorder the host process installs.
async fn timed<T>(_stage: &'static str, fut: impl std::future::Future<Output = T>) -> T {
    fut.await
}

/// Per-DID handle over a shared OpenDAL [`Operator`](opendal::Operator).
///
/// `op` is built once (typically via [`OpenDALBlobStore::from_env_operator`])
/// and cloned cheaply (the inner state is `Arc`-wrapped inside OpenDAL).
/// Each per-DID instance only ever writes keys under its own `{did}/` subtree,
/// so multiple actors can share the same backend safely.
#[derive(Debug, Clone)]
pub struct OpenDALBlobStore {
    /// The shared backend (S3 / FS / Memory).
    pub op: Operator,
    /// The owning actor's DID, baked into every key emitted by this handle.
    pub did: String,
}

impl OpenDALBlobStore {
    /// Wrap an already-built [`Operator`](opendal::Operator) with a DID.
    pub fn new(operator: Operator, did: String) -> Self {
        Self { op: operator, did }
    }

    /// Build an in-memory store suitable for tests.
    pub fn new_memory(did: &str) -> Result<Self> {
        let op = Operator::new(services::Memory::default())
            .context("build memory operator")?
            .finish();
        Ok(Self::new(op, did.to_owned()))
    }

    /// Build a filesystem-backed store rooted at `location`.
    pub fn new_disk(location: impl AsRef<std::path::Path>, did: &str) -> Result<Self> {
        let op = Operator::new(
            services::Fs::default().root(location.as_ref().to_string_lossy().as_ref()),
        )
        .context("build fs operator")?
        .finish();
        Ok(Self::new(op, did.to_owned()))
    }

    /// Build an S3-backed store. `endpoint` may be AWS or any S3-compatible
    /// endpoint (MinIO, R2, ...).
    pub fn new_s3(
        endpoint: &str,
        bucket: &str,
        access_key_id: &str,
        secret_access_key: &str,
        did: &str,
    ) -> Result<Self> {
        let op = Operator::new(
            services::S3::default()
                .endpoint(endpoint)
                .bucket(bucket)
                .access_key_id(access_key_id)
                .secret_access_key(secret_access_key),
        )
        .context("build s3 operator")?
        .finish();
        Ok(Self::new(op, did.to_owned()))
    }

    /// Build a per-DID handle from the PDS environment.
    ///
    /// S3-first: when `S3_ENDPOINT` is set, the S3 backend is wired up from
    /// `S3_BUCKET` / `S3_ACCESS_KEY_ID` / `S3_SECRET_ACCESS_KEY`. Otherwise a
    /// filesystem backend rooted at `PDS_BLOBSTORE_DISK_LOCATION` (defaulting
    /// to `./blobs`) is used.
    pub fn from_env(did: &str) -> Result<Arc<dyn BlobStore<Stream = BoxedBlobStream>>> {
        let op = Self::from_env_operator()?;
        Ok(Arc::new(Self::new(op, did.to_owned())))
    }

    /// Same env-reading as [`from_env`](Self::from_env), but returns the bare
    /// [`Operator`](opendal::Operator) (no DID wrapping). Callers that need to
    /// mint many per-DID handles from the same backend reuse the same operator
    /// via `Arc`/`OnceLock` instead of re-reading the environment per actor.
    pub fn from_env_operator() -> Result<Operator> {
        let op = if let Ok(endpoint) = std::env::var("S3_ENDPOINT") {
            let bucket =
                std::env::var("S3_BUCKET").context("S3_ENDPOINT is set but S3_BUCKET is not")?;
            let access_key_id = std::env::var("S3_ACCESS_KEY_ID")
                .context("S3_ENDPOINT is set but S3_ACCESS_KEY_ID is not")?;
            let secret_access_key = std::env::var("S3_SECRET_ACCESS_KEY")
                .context("S3_ENDPOINT is set but S3_SECRET_ACCESS_KEY is not")?;
            Operator::new(
                services::S3::default()
                    .endpoint(&endpoint)
                    .bucket(&bucket)
                    .access_key_id(&access_key_id)
                    .secret_access_key(&secret_access_key),
            )
            .context("build s3 operator from env")?
            .finish()
        } else {
            let location = std::env::var("PDS_BLOBSTORE_DISK_LOCATION")
                .unwrap_or_else(|_| "./blobs".to_owned());
            Operator::new(services::Fs::default().root(&location))
                .context("build fs operator from env")?
                .finish()
        };
        Ok(op)
    }

    /// Generate a 40-char hex temp key. Random, opaque, unique across actors.
    fn tmp_key() -> String {
        let mut bytes = [0u8; 20];
        rand::thread_rng().fill_bytes(&mut bytes);
        hex::encode(bytes)
    }

    fn tmp_path(&self, key: &str) -> String {
        format!("tmp/{}/{key}", self.did)
    }

    fn stored_path(&self, cid: Cid) -> String {
        format!("blocks/{}/{cid}", self.did)
    }

    fn quarantine_path(&self, cid: Cid) -> String {
        format!("quarantine/{}/{cid}", self.did)
    }
}

/// Map an OpenDAL [`Error`](opendal::Error) to [`BlobNotFoundError`] when the
/// backend reports the object is missing; otherwise preserve the original
/// error context.
fn map_not_found(err: opendal::Error) -> anyhow::Error {
    if err.kind() == ErrorKind::NotFound {
        BlobNotFoundError.into()
    } else {
        anyhow::Error::from(err)
    }
}

/// Increment the per-operation counter with a stable label.
fn counter_op(op: &str) {
    metrics::counter!(BLOB_OPS_TOTAL, "op" => op.to_owned()).increment(1);
}

fn histogram_put_bytes(n: usize) {
    metrics::histogram!(BLOB_PUT_BYTES).record(n as f64);
}

fn histogram_get_bytes(n: usize) {
    metrics::histogram!(BLOB_GET_BYTES).record(n as f64);
}

impl BlobStore for OpenDALBlobStore {
    type Stream = BoxedBlobStream;

    fn put_temp(&self, bytes: Vec<u8>) -> BoxFuture<'_, Result<String>> {
        let bytes_len = bytes.len();
        Box::pin(async move {
            timed("blob_put", async {
                counter_op("put_temp");
                let key = Self::tmp_key();
                let path = self.tmp_path(&key);
                self.op.write(&path, bytes).await.map_err(map_not_found)?;
                histogram_put_bytes(bytes_len);
                Ok(key)
            })
            .await
        })
    }

    fn make_permanent(&self, key: String, cid: Cid) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            counter_op("make_permanent");
            let tmp_path = self.tmp_path(&key);
            let stored_path = self.stored_path(cid);
            if !self.op.exists(&stored_path).await? {
                // rename: tmp/{did}/{key} -> blocks/{did}/{cid}
                self.op
                    .rename(&tmp_path, &stored_path)
                    .await
                    .map_err(map_not_found)?;
            }
            // Best-effort cleanup of the temp file. A failure here would only
            // leave a stray tmp/ entry behind, so log and move on rather than
            // failing the upload.
            if let Err(err) = self.op.delete(&tmp_path).await {
                tracing::error!(?err, %tmp_path, "could not delete file from temp storage");
            }
            Ok(())
        })
    }

    fn put_permanent(&self, cid: Cid, bytes: Vec<u8>) -> BoxFuture<'_, Result<()>> {
        let bytes_len = bytes.len();
        Box::pin(async move {
            timed("blob_put", async {
                counter_op("put_permanent");
                let path = self.stored_path(cid);
                self.op.write(&path, bytes).await.map_err(map_not_found)?;
                histogram_put_bytes(bytes_len);
                Ok(())
            })
            .await
        })
    }

    fn quarantine(&self, cid: Cid) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            counter_op("quarantine");
            let src = self.stored_path(cid);
            let dst = self.quarantine_path(cid);
            // rename: blocks/{did}/{cid} -> quarantine/{did}/{cid}
            self.op.rename(&src, &dst).await.map_err(map_not_found)
        })
    }

    fn unquarantine(&self, cid: Cid) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            counter_op("unquarantine");
            let src = self.quarantine_path(cid);
            let dst = self.stored_path(cid);
            // rename: quarantine/{did}/{cid} -> blocks/{did}/{cid}
            self.op.rename(&src, &dst).await.map_err(map_not_found)
        })
    }

    fn get_bytes(&self, cid: Cid) -> BoxFuture<'_, Result<Vec<u8>>> {
        Box::pin(async move {
            timed("blob_get", async {
                counter_op("get_bytes");
                let path = self.stored_path(cid);
                let buffer = self.op.read(&path).await.map_err(map_not_found)?;
                let bytes = buffer.to_vec();
                histogram_get_bytes(bytes.len());
                Ok(bytes)
            })
            .await
        })
    }

    fn get_stream(&self, cid: Cid) -> BoxFuture<'_, Result<Self::Stream>> {
        Box::pin(async move {
            timed("blob_get", async {
                counter_op("get_stream");
                let path = self.stored_path(cid);
                let reader = self.op.reader(&path).await.map_err(map_not_found)?;
                // Convert the OpenDAL reader to a futures::AsyncRead over the
                // entire byte range. The result implements futures::AsyncRead;
                // we wrap a poll_fn around poll_read to get a Stream<Bytes>.
                let mut async_reader = reader
                    .into_futures_async_read(..)
                    .await
                    .map_err(map_not_found)?;
                let stream = futures::stream::poll_fn(move |cx| {
                    let mut buf = [0u8; 8 * 1024];
                    let pinned = Pin::new(&mut async_reader);
                    match AsyncRead::poll_read(pinned, cx, &mut buf) {
                        Poll::Ready(Ok(0)) => Poll::Ready(None),
                        Poll::Ready(Ok(n)) => {
                            Poll::Ready(Some(Ok(Bytes::copy_from_slice(&buf[..n]))))
                        }
                        Poll::Ready(Err(err)) => Poll::Ready(Some(Err(anyhow::Error::from(err)))),
                        Poll::Pending => Poll::Pending,
                    }
                });
                // Count bytes as the caller drains the stream; we can't know the
                // size up-front without an extra stat call.
                let stream = stream.inspect_ok(|chunk| {
                    histogram_get_bytes(chunk.len());
                });
                Ok(stream.boxed())
            })
            .await
        })
    }

    fn has_temp(&self, key: String) -> BoxFuture<'_, Result<bool>> {
        Box::pin(async move {
            counter_op("has_temp");
            Ok(self.op.exists(&self.tmp_path(&key)).await?)
        })
    }

    fn has_stored(&self, cid: Cid) -> BoxFuture<'_, Result<bool>> {
        Box::pin(async move {
            counter_op("has_stored");
            Ok(self.op.exists(&self.stored_path(cid)).await?)
        })
    }

    fn delete(&self, cid: Cid) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            counter_op("delete");
            let path = self.stored_path(cid);
            self.op.delete(&path).await.map_err(map_not_found)
        })
    }

    fn delete_many(&self, cids: Vec<Cid>) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            counter_op("delete_many");
            let mut error_count = 0usize;
            for chunk in cids.chunks(DELETE_MANY_CHUNK_SIZE) {
                for cid in chunk {
                    if let Err(err) = BlobStore::delete(self, *cid).await {
                        tracing::error!(?err, cid = %cid, "error deleting blob");
                        error_count += 1;
                    }
                }
            }
            if error_count > 0 {
                anyhow::bail!("failed to delete {error_count} blobs");
            }
            Ok(())
        })
    }

    fn delete_all(&self) -> Option<BoxFuture<'_, Result<()>>> {
        let op = self.op.clone();
        let did = self.did.clone();
        Some(Box::pin(async move {
            counter_op("delete_all");
            // Wipe each prefix in turn. remove_all swallows the NotFound case
            // on services that report it as an error; an unexpected failure
            // on one prefix doesn't block wiping the other two.
            for prefix in [
                format!("tmp/{did}"),
                format!("blocks/{did}"),
                format!("quarantine/{did}"),
            ] {
                if let Err(err) = op.remove_all(&prefix).await
                    && err.kind() != ErrorKind::NotFound
                {
                    return Err(anyhow::Error::from(err));
                }
            }
            Ok(())
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsky_common::ipld::sha256_to_cid;
    use sha2::{Digest, Sha256};

    const DID_A: &str = "did:example:alice";
    const DID_B: &str = "did:example:bob";

    fn cid_for(bytes: &[u8]) -> Cid {
        sha256_to_cid(Sha256::digest(bytes).to_vec())
    }

    /// Build a per-test FS-backed operator rooted in a tempdir. FS supports
    /// `rename` (the Memory backend does not), which `make_permanent` and
    /// `quarantine` rely on.
    fn fs_op() -> (camino_tempfile::Utf8TempDir, Operator) {
        let dir = camino_tempfile::tempdir().unwrap();
        let op = OpenDALBlobStore::new_disk(dir.path(), DID_A).unwrap();
        let op = op.op.clone();
        (dir, op)
    }

    /// Two per-DID handles sharing one FS-backed operator.
    fn shared_store() -> (
        camino_tempfile::Utf8TempDir,
        Operator,
        OpenDALBlobStore,
        OpenDALBlobStore,
    ) {
        let dir = camino_tempfile::tempdir().unwrap();
        let op = OpenDALBlobStore::new_disk(dir.path(), DID_A)
            .unwrap()
            .op
            .clone();
        let a = OpenDALBlobStore::new(op.clone(), DID_A.to_owned());
        let b = OpenDALBlobStore::new(op.clone(), DID_B.to_owned());
        (dir, op, a, b)
    }

    /// `put_temp` returns a key that, after `make_permanent(cid)`, resolves to
    /// the original bytes via `get_bytes`.
    #[tokio::test]
    async fn temp_to_permanent_lifecycle() {
        let (_dir, op) = fs_op();
        let store = OpenDALBlobStore::new(op, DID_A.to_owned());
        let bytes = b"hello opendal blob".to_vec();
        let cid = cid_for(&bytes);

        let key = store.put_temp(bytes.clone()).await.unwrap();
        assert!(store.has_temp(key.clone()).await.unwrap());
        assert!(!store.has_stored(cid).await.unwrap());

        store.make_permanent(key.clone(), cid).await.unwrap();
        assert!(!store.has_temp(key.clone()).await.unwrap());
        assert!(store.has_stored(cid).await.unwrap());
        assert_eq!(store.get_bytes(cid).await.unwrap(), bytes);
    }

    /// `quarantine` moves a stored blob out of the active namespace;
    /// `unquarantine` puts it back. While quarantined, `has_stored` is false
    /// and reads surface [`BlobNotFoundError`].
    #[tokio::test]
    async fn quarantine_round_trip() {
        let (_dir, op) = fs_op();
        let store = OpenDALBlobStore::new(op, DID_A.to_owned());
        let bytes = b"quarantine me".to_vec();
        let cid = cid_for(&bytes);
        store.put_permanent(cid, bytes.clone()).await.unwrap();

        store.quarantine(cid).await.unwrap();
        assert!(!store.has_stored(cid).await.unwrap());
        let err = store.get_bytes(cid).await.unwrap_err();
        assert!(err.downcast_ref::<BlobNotFoundError>().is_some());

        store.unquarantine(cid).await.unwrap();
        assert!(store.has_stored(cid).await.unwrap());
        assert_eq!(store.get_bytes(cid).await.unwrap(), bytes);
    }

    /// Single and bulk delete remove the targeted CIDs and leave the rest
    /// untouched.
    #[tokio::test]
    async fn deletes_single_and_many() {
        let (_dir, op) = fs_op();
        let store = OpenDALBlobStore::new(op, DID_A.to_owned());
        let one = b"one".to_vec();
        let two = b"two".to_vec();
        let (cid_one, cid_two) = (cid_for(&one), cid_for(&two));
        store.put_permanent(cid_one, one).await.unwrap();
        store.put_permanent(cid_two, two).await.unwrap();
        assert!(store.has_stored(cid_one).await.unwrap());
        assert!(store.has_stored(cid_two).await.unwrap());

        store.delete(cid_one).await.unwrap();
        assert!(!store.has_stored(cid_one).await.unwrap());
        assert!(store.has_stored(cid_two).await.unwrap());

        store.delete_many(vec![cid_one, cid_two]).await.unwrap();
        assert!(!store.has_stored(cid_two).await.unwrap());
    }

    /// Two per-DID handles over one Operator cannot read each other's temp
    /// keys. The shared backend stays tenant-isolated because every key
    /// includes the DID.
    #[tokio::test]
    async fn per_did_isolates_temp_keys() {
        let (_dir, _op, alice, bob) = shared_store();
        let alice_key = alice.put_temp(b"alice tmp".to_vec()).await.unwrap();

        assert!(alice.has_temp(alice_key.clone()).await.unwrap());
        assert!(!bob.has_temp(alice_key).await.unwrap());
    }

    /// `delete_all` on a per-DID handle removes that actor's blocks/quarantine
    /// subtree but leaves the other actor's blobs in place.
    #[tokio::test]
    async fn delete_all_wipes_only_one_actor() {
        let (_dir, _op, alice, bob) = shared_store();
        let alice_cid = cid_for(b"alice blob");
        let bob_cid = cid_for(b"bob blob");
        alice.put_permanent(alice_cid, b"a".to_vec()).await.unwrap();
        bob.put_permanent(bob_cid, b"b".to_vec()).await.unwrap();
        alice.put_temp(b"alice tmp".to_vec()).await.unwrap();
        let bob_temp_key = bob.put_temp(b"bob tmp".to_vec()).await.unwrap();

        alice.delete_all().unwrap().await.unwrap();

        assert!(!alice.has_stored(alice_cid).await.unwrap());
        // bob's blobs survive alice's wipe: isolation is per-DID.
        assert!(bob.has_stored(bob_cid).await.unwrap());
        assert!(bob.has_temp(bob_temp_key).await.unwrap());
    }

    /// Per-DID key prefixes land where the layout says they should.
    #[tokio::test]
    async fn key_layout_matches_spec() {
        let (_dir, op) = fs_op();
        let store = OpenDALBlobStore::new(op.clone(), DID_A.to_owned());
        let key = store.put_temp(b"x".to_vec()).await.unwrap();
        let cid = cid_for(b"x");
        store.put_permanent(cid, b"x".to_vec()).await.unwrap();
        store.quarantine(cid).await.unwrap();

        // exists() walks the operator's metadata; reading by the exact path
        // shape is the strongest check that our prefixes are right.
        assert!(op.exists(&format!("tmp/{DID_A}/{key}")).await.unwrap());
        assert!(
            op.exists(&format!("quarantine/{DID_A}/{cid}"))
                .await
                .unwrap()
        );
        // After quarantine, the blocks entry is gone.
        assert!(!op.exists(&format!("blocks/{DID_A}/{cid}")).await.unwrap());
    }

    /// 32 concurrent `put_temp` calls on the same store all return distinct
    /// keys. The temp-key generator must not collide under contention.
    #[tokio::test]
    async fn concurrent_put_temp_keys_are_unique() {
        let (_dir, op) = fs_op();
        let store = std::sync::Arc::new(OpenDALBlobStore::new(op, DID_A.to_owned()));
        let handles: Vec<_> = (0..32)
            .map(|i| {
                let store = store.clone();
                tokio::spawn(async move { store.put_temp(vec![i as u8]).await.unwrap() })
            })
            .collect();
        let mut keys = std::collections::HashSet::new();
        for handle in handles {
            let key = handle.await.unwrap();
            assert!(store.has_temp(key.clone()).await.unwrap());
            assert!(keys.insert(key));
        }
        assert_eq!(keys.len(), 32);
    }

    /// `get_bytes` against an unknown CID yields an error downcastable to
    /// [`BlobNotFoundError`].
    #[tokio::test]
    async fn missing_blob_returns_blob_not_found() {
        let (_dir, op) = fs_op();
        let store = OpenDALBlobStore::new(op, DID_A.to_owned());
        let cid = cid_for(b"never uploaded");
        let err = store.get_bytes(cid).await.unwrap_err();
        assert!(
            err.downcast_ref::<BlobNotFoundError>().is_some(),
            "expected BlobNotFoundError, got {err:?}"
        );
    }
}
