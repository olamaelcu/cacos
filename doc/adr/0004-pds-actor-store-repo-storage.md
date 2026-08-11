# 4. PDS actor store & repo storage over per-DID SQLite + sea-orm

Date: 2026-08-05

## Status

Accepted

## Context

We needed per-actor block and root storage that plugs into the atproto `ReadableBlockstore` and `RepoStorage` traits (`rsky-repo`'s concrete shape) while replacing the rusqlite wrapper with sea-orm, keeping the trait surface so downstream code (`Repo::format_init_commit`, `Repo::load`, `Repo::format_commit`, MST handling, CAR export) works unchanged. The cacos workspace is on Rust edition 2024, which forbids naming transitive-only dependencies in code. Object storage (MinIO/S3) is the home of the *blob store* (Plan 04, OpenDAL), not the repo store — this ADR is scoped to the SQLite-backed repo/record side.

## Decision

1. **Per-DID SQLite for blocks.** Repo blocks live in a `repo_block` table inside a per-DID SQLite file. Actor directories are sharded by `hex(sha256(did))[0..2]`. MinIO/S3 is not used here; object storage lives in the blob store.

2. **sea-orm entities, not raw SQL + private row structs.** Queries are declarative against `repo_block::Entity`, `repo_root::Entity`, `record::Entity`, and `backlink::Entity`. The originally-planned private `RepoBlock`/`Record`/`Backlink` row structs and `Statement::from_sql_and_values` port were dropped in favor of reusing entity Models directly.

3. **`DatabaseKind::Actor.open` is the only opening API.** A free `open_actor_db` function was refactored away in favor of the typed enum method. `pds/src/actor_store/db/mod.rs` re-exports `DatabaseKind` so callers do `crate::actor_store::db::DatabaseKind::Actor.open(path)`. The method takes `impl AsRef<Utf8Path>` and returns `migration::error::Result<DatabaseConnection>`, which converts to `PdsError` via a `From` impl.

4. **miette + thiserror errors, with a single `anyhow` wrapper variant.** New `pds/src/error.rs` defines `PdsError` (miette `Diagnostic` + `thiserror::Error`). The single `Internal { reason, source: anyhow::Error }` variant is the one place rsky's anyhow-returning calls land. All app-level code returns `crate::error::Result<T>` (which is `miette::Result<T, PdsError>`). `anyhow` is a direct dependency solely because (a) the rsky-repo trait signatures name `anyhow::Result` in their method return types, and (b) `Internal.source` holds `anyhow::Error`.

5. **`anyhow = "1.0.79"` pinned in the workspace.** This matches the pins in `rsky-repo` / `rsky-common` / `rsky-crypto` / `rsky-lexicon`. No rsky crate re-exports `anyhow`, so a direct dependency is unavoidable today. A comment in `pds/src/error.rs` notes the intent to remove `anyhow` entirely once rsky is forked or vendored to return a proper error type — that removal should land in a **separate branch** alongside the rsky fork so it can be coordinated.

6. **`Send + Sync` futures via `tokio::spawn`.** sea-orm's async functions return `Send` but not `Sync` futures; the rsky `ReadableBlockstore` and `RepoStorage` traits require `Pin<Box<dyn Future<Output = anyhow::Result<…>> + Send + Sync>>`. Trait methods clone `self` and dispatch the work via `tokio::spawn`, then `await` the `JoinHandle` (which is `Send + Sync`). State stays consistent across the spawn boundary through the shared `Arc<RwLock<BlockMap>>` and the cloned `DatabaseConnection` (both `Clone`).

7. **`apply_commit` is transactional.** `update_root` + `put_many` + `delete_many` are wrapped in one sea-orm transaction so a mid-commit failure rolls back the whole commit. Sea-orm 2.0's AFIT `transaction_async` form is used because the older boxed-dyn closure shape misbehaves with our closure; `TransactionError<PdsError>` is mapped back to `PdsError` explicitly.

8. **camino + camino-tempfile everywhere, not `tempfile`.** `DatabaseKind::Actor.open` takes `impl AsRef<Utf8Path>`. `camino_tempfile::Utf8TempDir` yields `Utf8Path` directly, avoiding UTF-8 conversion errors at the boundary.

9. **Defer `process_write_blobs`, `BlobReader`, `BackgroundQueue`, and `pref: PreferenceReader`.** Plan 04 closes the blobstore seam (adds `blob: BlobReader` to `ActorStoreReader`, `background_queue` to `ActorStore`, and a `blobstore: Arc<dyn BlobStore>` parameter to `read`/`transact`/`destroy`). Plan 08 closes the preference-store seam and adds the deferred `RecordReader` methods (`list_records_for_collection`, `get_record_takedown_status`, `update_record_takedown_status`, `list_existing_blocks`, plus serde derives on `GetRecord`). Plan 09 must skip its Task 5 Step 1 (the cache counter consts and describes already exist) and only add `timed(...)` wrapping plus `cacos_timing_seconds`. Plan 09's expected `test_store` helper is preserved.

10. **Metrics via plain `metrics::counter!` macros.** `cacos_actor_cache_hits_total`, `cacos_actor_cache_misses_total`, and `cacos_commits_total` are registered with `describe_counter!` in `pds/src/observability/metrics.rs`. The two cache counters carry the `did` label.

## Consequences

- **Plan 04 (blobstore) must update its assumptions.** It currently assumes `crate::actor_store::db::open_actor_db` exists; the correct call is `crate::db::DatabaseKind::Actor.open(path)`. It must add the `blob: BlobReader` field to `ActorStoreReader`, the `background_queue` field to `ActorStore`, and the `blobstore: Arc<dyn BlobStore>` parameter to `read` / `transact` / `destroy`. The three `process_write_blobs(...)` calls inside `create_repo` / `process_import_repo` / `process_writes` must be restored.

- **Plan 08 (xrpc) must add** `pref: PreferenceReader` and the deferred `RecordReader` methods listed above, alongside the pref store port.

- **Plan 09 (observability sweep) must skip** its Task 5 Step 1 (counter registration already happened in this plan) and only add the `timed(...)` wrapping plus the `cacos_timing_seconds` family.

- **The `tokio::spawn` shim adds a hop per trait method call.** This is acceptable today; revisit if profiling shows it on a hot path.

- **Removing `anyhow` from `pds` is now blocked on the rsky fork.** Track that work as a separate-branch effort so it lands alongside the corresponding rsky changes; this ADR's decision 5 is the artifact of that intent.

- **Seams deliberately left open** (must be closed by later plans):
  - `actor_store::db::open_actor_db` does not exist — use `DatabaseKind::Actor.open`.
  - `ActorStoreReader` has no `blob` field.
  - `ActorStore` has no `background_queue` field.
  - `read` / `transact` / `destroy` take no `blobstore` parameter.
  - `process_write_blobs` calls are omitted inside `create_repo` / `process_import_repo` / `process_writes`.
  - `pref: PreferenceReader` is deferred.
  - `RecordReader` is missing `list_records_for_collection`, `get_record_takedown_status`, `update_record_takedown_status`, `list_existing_blocks`, and serde derives on `GetRecord`.
  - `get_backlink_conflicts` and `get_backlinks` are now real implementations in `cacos-pds-actor-store/src/record/mod.rs` (with tests), not stubs.
