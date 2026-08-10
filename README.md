# Cacos

> A white-label Rust [ATProto PDS][1] by [Olamaelcu][]

[1]: https://atproto.com/guides/glossary#pds-personal-data-server
[Olamaelcu]: https://github.com/olamaelcu

Cacos is a white-label ATProto Personal Data Server written in Rust. It
organises its data across four SQLite domains (Account, Sequencer, DID cache,
and a per-actor Actor store sharded by `sha256(did)`), persists each
actor's repository under their own SQLite file, and serves blob traffic
through an [OpenDAL](https://docs.rs/opendal)-backed blobstore with per-DID partitioning so a single
backend (S3/MinIO or local disk) can host every tenant without leakage.
Observability is first-class: a Prometheus recorder exposes a `cacos_`-prefixed
metric set at `/metrics` alongside tracing/timing layers. Account lifecycle, sessions, app passwords, invites, and the headless
OAuth provider (`/oauth/*`, `/oauth/remote/*`) are wired in front of the
[sea-orm](https://docs.rs/sea-orm) account store; the OAuth surface is conditionally mounted — it
only boots when `PDS_JWT_KEY_K256_PRIVATE_KEY_HEX` is set, and
deployments that don't need federated OAuth still get the same PDS
surface with no extra surface area or dependencies. The typed
`Sequencer` populates `repo_seq` and feeds `com.atproto.sync.subscribeRepos`
over a WebSocket backed by an apalis-shaped SQLite jobs table.

[ATProto]: https://atproto.com

[Olamaelcu]: https://github.com/olamaelcu

## Status

Storage, observability, account management, the per-actor repo primitives,
the OAuth provider/auth-verifier, the typed sequencer + firehose, and the
`com.atproto.*` XRPC surface ([poem](https://docs.rs/poem) 3.x) are in place. The server currently
serves the XRPC routes, `GET /metrics`, `GET /_health`, `GET /xrpc/_health`,
`GET /.well-known/atproto-did`, and (when configured) `/oauth/*`.

## What's there

### Workspace

A two-crate Cargo workspace:

- `pds/` — the [cacos-pds](https://docs.rs/cacos-pds) binary crate. Runs a [poem](https://docs.rs/poem) HTTP server on
  `127.0.0.1:8080`.
- `migration/` — the [cacos-migration](https://docs.rs/cacos-migration) crate. Holds the [sea-orm](https://docs.rs/sea-orm) entities and
  migrators for every PDS database.

The workspace pins the [rsky][rsky] ATProto protocol crates from a fork at a
specific git rev, so updates pull a known revision rather than whatever
upstream `main` happens to be.

[rsky]: https://github.com/olamaelcu/rsky

### Storage

Four SQLite databases, one file per concern, all opened with WAL and `foreign_keys=ON`:

| Database  | File pattern          | Migrator           | Purpose                                  |
|-----------|-----------------------|--------------------|------------------------------------------|
| Account   | `account.sqlite`      | `AccountMigrator`  | Accounts, actors, sessions, invites, OAuth |
| Sequencer | `sequencer.sqlite`    | `SequencerMigrator`| `repo_seq` event log for firehose        |
| DID cache | `did_cache.sqlite`    | `DidCacheMigrator` | Resolved `did_doc` rows                  |
| Actor     | `store.sqlite` per DID | `ActorMigrator`    | Per-user records, blocks, blobs, spaces |

Open and migrate via `pds::db::DatabaseKind::open`. Migrations are idempotent
and use a custom bookkeeping table name per database (`account_migrations`,
`sequencer_migrations`, `did_cache_migrations`, `actor_migrations`) so they
don't collide with other sea-orm migrators sharing a connection.

### Actor store

`pds::actor_store::ActorStore` manages per-actor SQLite files and [secp256k1](https://docs.rs/secp256k1)
keypairs under a root directory:

- One SQLite file per DID, sharded by the first two hex chars of `sha256(did)`
  to keep directory fan-out reasonable.
- Reserved keypairs in `reserved_keys/` for in-flight signups, addressable
  either by `did:key` or by the future DID.
- Per-DID write locks so transactors for the same actor serialize.
- LRU cache of open `DatabaseConnection`s, evictable by DID.
- Path traversal hardening (`assert_safe_path_part`) on every DID-derived path
  component.

`ActorStoreReader` exposes read APIs (`get_repo_root`, `get_sync_event_data`).
`ActorStoreTransactor` holds the per-DID write guard plus the loaded keypair
and provides `create_repo`, `format_commit`, `process_writes`, and
`process_import_repo`.

### Repo storage

`actor_store::repo::sql_repo::SqlRepoReader` implements the upstream
`RepoStorage` and `ReadableBlockstore` traits on top of sea-orm. The `BlockMap`
in-memory cache from upstream is preserved (read/write-through). Cache hits and
misses feed `cacos_actor_cache_hits_total` and
`cacos_actor_cache_misses_total`.

Async trait methods that the upstream interface wants as `Send + Sync` futures
are dispatched through `tokio::spawn`, which detaches [sea-orm][]'s non-`Sync`
inner state and yields a `JoinHandle` that IS `Send + Sync`.

### Records

`actor_store::record::RecordReader` covers per-actor record lookups
(`get_record`, `has_record`, `list_records`, `list_collections`,
`record_count`). Backlinks live in a separate table; indexing runs through
`ActorStoreTransactor::index_writes`.

### Blobstore

`pds::blobstore::OpenDALBlobStore` wraps a shared [OpenDAL](https://docs.rs/opendal) `Operator` (S3,
filesystem, or memory) with per-DID partitioning. Every key carries the owning
DID, so a single backend serves every tenant without leakage between them:

- `blocks/{did}/{cid}` — committed blobs.
- `tmp/{did}/{key}` — untethered uploads. `key` is a 40-char hex string minted
  by `put_temp`.
- `quarantine/{did}/{cid}` — takedown staging area.

Per-DID partitioning is baked into the wrapper at construction time
(`OpenDALBlobStore::new(op, did)`); the shared `Operator` is built once
at startup and held in a `OnceLock`, so every per-DID handle is a cheap
clone of the same operator. S3 is selected when `S3_ENDPOINT` is set
(reads `S3_BUCKET` / `S3_ACCESS_KEY_ID` / `S3_SECRET_ACCESS_KEY`);
otherwise the filesystem at `PDS_BLOBSTORE_DISK_LOCATION` (default
`./blobs`). `delete_all` walks the three prefixes for the actor's DID
and returns `Some(...)` rather than the upstream `None` default.

Reads stream through a `futures::AsyncRead` wrapper around [OpenDAL](https://docs.rs/opendal)'s `Reader`,
so no [tokio-util](https://docs.rs/tokio-util) adapter gets pulled in. The `BlobStore` trait and
`MemoryBlobStore` test double come from [rsky_blobstore](https://docs.rs/rsky_blobstore). `delete_all` on a
per-DID handle wipes only that actor's three prefixes.

Backend selection is environment-driven: `S3_*` vars switch to S3/MinIO,
otherwise the filesystem at `PDS_BLOBSTORE_DISK_LOCATION` (default `./blobs`).

### Observability

One tracing subscriber stack, with a Prometheus recorder on the side:

- `EnvFilter` driven by `RUST_LOG`, default `info`.
- `MetricsLayer` to label metrics with span context.
- `fmt` for human-readable logs.
- [tracing-timing](https://docs.rs/tracing-timing) histograms, with a `TimingReporter` background task that
  snapshots p50/p90/p99 every 10 seconds into `cacos_timing_p{50,90,99}_seconds`
  gauges.

Metrics use a Prometheus recorder with a fixed `cacos_` prefix and HELP lines
registered up front. The full set lives in `pds::observability::metrics`. `GET
/metrics` serves the current snapshot in text exposition format. `timed(...)`
wrappers under `blob_*`, `repo_*`, and `sequencer` paths feed the
`cacos_timing_seconds` histogram family; the sequencer's apalis-shaped worker
increments `cacos_seq_events_total` and records
`cacos_sequencer_poll_interval_seconds` (label `kind` ∈ {`publish`, `poll`});
`cacos_outbox_buffer_lag` is updated by the `Outbox` live loop; the actor
cache counters (`cacos_actor_cache_hits_total` / `cacos_actor_cache_misses_total`)
and `cacos_blob_ops_total` (label `op`) are populated by their respective
call sites.

### Background queue

`pds::background::BackgroundQueue` is a small in-process task drainer. Accepts
async work, schedules it on Tokio with a bounded semaphore (default concurrency
5), and lets callers wait for the queue to drain via `Notify`. Task failures
are logged and swallowed; they never surface to the caller that enqueued them.
Intended for blob deref cleanup and similar fire-and-forget work.

### Errors

A single `PdsError` enum ([thiserror](https://docs.rs/thiserror) + [miette](https://docs.rs/miette) diagnostic codes) covers the app
surface: `Database`, `Internal { reason, source }`, `NotFound`, `InvalidInput`.
The `Internal` variant wraps the `anyhow::Error` sources that still come back
from upstream rsky traits; that migration is tracked for a later cleanup but
lives at exactly one call site per variant today.

## Dev setup

`mise.toml` pins Rust 1.97.0, [cargo-nextest](https://docs.rs/cargo-nextest), and `nose`, and defines tasks:

| Task                 | What it does                                  |
|----------------------|-----------------------------------------------|
| `mise run check`     | `cargo check --workspace --all-targets`       |
| `mise run fmt`       | `cargo fmt --all -- --check`                  |
| `mise run format`    | `cargo fmt --all`                             |
| `mise run lint`      | `cargo clippy --workspace --all-targets -- -D warnings` |
| `mise run test`      | `cargo nextest run --workspace`               |
| `mise run build`     | `cargo build --workspace`                     |
| `mise run dup`       | `nose` duplication report (excludes migration files) |
| `mise run infra-up`  | Start MinIO in the background                 |
| `mise run infra-down`| Stop MinIO                                    |
| `mise run dev`       | `cargo run -p cacos-pds` (depends on `infra-up`) |

`docker-compose.yaml` runs MinIO plus a `mc-init` sidecar that creates the `cacos` bucket on first boot.
