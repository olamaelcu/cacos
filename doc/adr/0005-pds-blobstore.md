# 5. PDS blob store over OpenDAL with per-DID key partitioning

Date: 2026-08-05

## Status

Accepted

## Context

PDS blobs (uploaded media, avatars, embeds) live in object storage rather than SQLite — sizes are unbounded and the read pattern is bytes-out rather than rows-out. The atproto protocol crates ship a `BlobStore` trait (`Send + Sync + Debug`, `BoxFuture`-based, `anyhow::Result`) plus a boxed-stream associated type, a `BlobNotFoundError`, and an in-memory `MemoryBlobStore` test double. We need a production backend that works against both local disk (single-node dev) and S3 / MinIO / R2 (production / `mise dev`), with per-DID tenant isolation so one actor's destroy doesn't reach into another's directory and one actor's bulk list / delete doesn't scan another actor's prefix.

## Decision

1. **OpenDAL backend (`opendal = "0.53"`, features `services-fs`, `services-s3`).** Same version pinned by the rsky fork's `opendal_blobstore.rs`; S3 covers MinIO / R2 / DO Spaces / AWS. `services-memory` is always-on (no feature flag). OpenDAL's `Operator` is internally `Arc`-wrapped, so cloning it for every per-DID handle is cheap.

2. **Per-DID partitioning via key prefix.** The shared `Operator` is built **once** at PDS startup and held in a process-wide `OnceLock<Operator>`. A per-DID wrapper bakes the actor's DID into every key: `blocks/{did}/{cid}` for stored, `tmp/{did}/{key}` for untethered uploads, `quarantine/{did}/{cid}` for takedowns. Tenant isolation is restored by construction; there is no cross-actor prefix enumeration path.

3. **Per-DID struct shape `OpenDALBlobStore { op: Operator, did: String }`.** DID is baked in at construction (`OpenDALBlobStore::new(op, did)`); `for_did`-style factories are not used. The shared `Operator` is cloned (cheap) into every handle.

4. **Startup wiring via `OnceLock<Operator>`.** `pds/src/blobstore/mod.rs` exposes `init_operator()` (reads env, builds the shared operator, sets the `OnceLock`) and `blobstore_for_did(did) -> Arc<dyn BlobStore<Stream = BoxedBlobStream>>` (clones the operator, wraps in the per-DID handle). `main()` calls `init_operator` once before serving. `blobstore_for_did` panics if `init_operator` hasn't run; that's a programmer error.

5. **S3-first env reading.** If `S3_ENDPOINT` is set, build the S3 backend from `S3_BUCKET` / `S3_ACCESS_KEY_ID` / `S3_SECRET_ACCESS_KEY`. Otherwise read `PDS_BLOBSTORE_DISK_LOCATION` (default `./blobs`) and build the filesystem backend. The chosen backend is logged at startup so operators can confirm.

6. **`delete_all` override returns `Some(future)`** that walks the three prefixes via `op.remove_all(prefix)` for that actor's `blocks/{did}/`, `tmp/{did}/`, and `quarantine/{did}/`. This restores the rsky `DiskBlobStore::delete_all` wholesale-actor-wipe behavior — overrriding the upstream `BlobStore` default `None`. `ActorStore::destroy(did, blobstore)` calls `blobstore.delete_all()` before `remove_dir_all`.

7. **Streaming reads via `into_futures_async_read` + `poll_fn`.** `Operator::reader(path)` returns a `Reader`; converting to `futures::AsyncRead` via `into_futures_async_read(..)` and driving a `poll_fn` over `AsyncRead::poll_read` produces the trait's `BoxedBlobStream` items (`Result<bytes::Bytes>`). No `tokio-util` dependency.

8. **Tests use FS-backed operators rooted in `camino_tempfile::tempdir()`.** The `Memory` backend is in scope for the test suite (`cargo test --lib blobstore::tests` runs with it), but the rename operations `make_permanent` and `quarantine` rely on, so they switch to `Fs`. The 15 blobstore tests cover round-trip, quarantine, single + bulk delete, per-DID isolation, wholesale wipe, key layout, concurrent temp-key uniqueness, and the not-found error path.

9. **`BackgroundQueue` carries the per-DID blobstore handle.** `BlobReader::delete_dereferenced_blobs` enqueues a background `delete_many` task that captures an `Arc<dyn BlobStore<Stream = BoxedBlobStream>>` clone (the per-DID handle from `ActorStore::read(did, blobstore)`). The queue itself is unchanged — it's the upstream `rsky-pds::background::BackgroundQueue` ported verbatim, with one new module-level `BackgroundQueue` instance per `ActorStoreReader`.

10. **Metrics.** Per-op counter `cacos_blob_ops_total` (new const `BLOB_OPS_TOTAL`, `describe_counter!` added to `pds/src/observability/metrics.rs`, label `op` carrying the trait method name). The byte histograms `cacos_blob_put_bytes` / `cacos_blob_get_bytes` (Plan 02 consts `BLOB_PUT_BYTES` / `BLOB_GET_BYTES`) record sizes inside the private `put` / `get` helpers — Plan 09's observability sweep will wrap those helpers with `timed("blob_put" | "blob_get", ..)` without double-counting bytes.

11. **`ActorStore` API churn.** `read` / `transact` / `destroy` now take `blobstore: Arc<dyn BlobStore<Stream = BoxedBlobStream>>` as a second parameter. `ActorStoreReader` gains `pub blob: BlobReader`. The three deferred `process_write_blobs(writes).await?` calls inside `create_repo` / `process_import_repo` / `process_writes` (called out in ADR-0004's consequences) are restored.

12. **`BlobNotFoundError` is the canonical not-found type.** Every `opendal::Error` with `ErrorKind::NotFound` is translated via a private `map_not_found` helper to `anyhow::Error::new(BlobNotFoundError)` so the upstream trait's not-found type propagates uniformly across disk and S3 backends.

## Consequences

- **Per-call env re-read is gone.** `blobstore::from_env(did)` now reads env on every call, which is misleading once `init_operator` is mandatory. Plan 09 should rewrite `from_env` to delegate to `blobstore_for_did(did)` (or delete it).

- **Every `ActorStore` caller pays a one-arg bump.** `tests.rs` (20 sites) updated to `store.read(did, test_blobstore(did))`. Plan 08's HTTP handlers will follow the same pattern: `crate::blobstore::blobstore_for_did(did)` at the boundary, pass the handle into `ActorStore::read` / `transact` / `destroy`.

- **The `Memory` backend cannot `rename`** (it returns `Unsupported`). Tests that exercise `make_permanent` / `quarantine` / `unquarantine` therefore use the `Fs` backend over a tempdir; tests that only exercise temp + get + delete stay on `Memory`.

- **Cross-plan coupling.** Plan 08's `sync/getBlob` and `space/getBlob` handlers must `try_collect` the `BoxedBlobStream` to `Vec<Bytes>` then flatten to `Vec<u8>` (mirror of `rsky-pds/src/apis/com/atproto/sync/get_blob.rs:42-43`). Plan 09 wraps the `OpenDALPerDidBlobStore::put` / `get` helpers with `timed("blob_put", ..)` / `timed("blob_get", ..)` while keeping the byte-histogram recording inside those helpers.

- **The `tokio-util::io::ReaderStream` path the original plan called out is not used.** The fork's `into_futures_async_read` + `poll_fn` approach is the chosen path; no new dependency is added.
