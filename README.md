# Cacos

> A white-label Rust [ATProto PDS][1] by [Olamaelcu][]

[1]: https://atproto.com/guides/glossary#pds-personal-data-server
[Olamaelcu]: https://github.com/olamaelcu

Cacos is a white-label ATProto Personal Data Server written in Rust. It
organises its data across four SQLite domains (Account, Sequencer, DID cache,
and a per-actor Actor store sharded by `sha256(did)`), persists each
actor's repository under their own SQLite file, and serves blob traffic
through an OpenDAL-backed blobstore with per-DID partitioning so a single
backend (S3/MinIO or local disk) can host every tenant without leakage.
Observability is first-class: a Prometheus recorder exposes a `cacos_`-prefixed
metric set at `/metrics` alongside tracing/timing layers. The headless
OAuth provider (`/oauth/*`, `/oauth/remote/*`) is conditionally mounted —
it only boots when `PDS_JWT_KEY_K256_PRIVATE_KEY_HEX` is set; deployments
that don't need federated OAuth still get the same PDS surface with no
extra surface area or dependencies.

[ATProto]: https://atproto.com
[Olamaelcu]: https://github.com/olamaelcu

## Status

Early scaffolding. Storage, observability, and the per-actor repo primitives
are in place. No `com.atproto.*` XRPC handlers are wired to HTTP yet; the only
route the server currently serves is `GET /metrics`.

## What's there

### Workspace

A two-crate Cargo workspace:

- `pds/` — the `cacos-pds` binary crate. Runs a poem HTTP server on
  `127.0.0.1:8080`.
- `migration/` — the `cacos-migration` crate. Holds the sea-orm entities and
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

`pds::actor_store::ActorStore` manages per-actor SQLite files and `secp256k1`
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

`pds::blobstore::OpenDALBlobStore` wraps a shared OpenDAL `Operator` (S3,
filesystem, or memory) with per-DID partitioning. Every key carries the owning
DID, so a single backend serves every tenant without leakage between them:

- `blocks/{did}/{cid}` — committed blobs.
- `tmp/{did}/{key}` — untethered uploads. `key` is a 40-char hex string minted
  by `put_temp`.
- `quarantine/{did}/{cid}` — takedown staging area.

Reads stream through a `futures::AsyncRead` wrapper around OpenDAL's `Reader`,
so no `tokio-util` adapter gets pulled in. The `BlobStore` trait and
`MemoryBlobStore` test double come from `rsky_blobstore`. `delete_all` on a
per-DID handle wipes only that actor's three prefixes.

Backend selection is environment-driven: `S3_*` vars switch to S3/MinIO,
otherwise the filesystem at `PDS_BLOBSTORE_DISK_LOCATION` (default `./blobs`).

### Observability

One tracing subscriber stack, with a Prometheus recorder on the side:

- `EnvFilter` driven by `RUST_LOG`, default `info`.
- `MetricsLayer` to label metrics with span context.
- `fmt` for human-readable logs.
- `tracing-timing` histograms, with a `TimingReporter` background task that
  snapshots p50/p90/p99 every 10 seconds into `cacos_timing_p{50,90,99}_seconds`
  gauges.

Metrics use a Prometheus recorder with a fixed `cacos_` prefix and HELP lines
registered up front. The full set lives in `pds::observability::metrics`. `GET
/metrics` serves the current snapshot in text exposition format. Counters and
gauges for plans that haven't landed yet (`cacos_signups_total`,
`cacos_sessions_total`, `cacos_seq_events_total`, `cacos_outbox_buffer_lag`,
`cacos_commits_total`) are registered but unpopulated.

### Background queue

`pds::background::BackgroundQueue` is a small in-process task drainer. Accepts
async work, schedules it on Tokio with a bounded semaphore (default concurrency
5), and lets callers wait for the queue to drain via `Notify`. Task failures
are logged and swallowed; they never surface to the caller that enqueued them.
Intended for blob deref cleanup and similar fire-and-forget work.

### Errors

A single `PdsError` enum (`thiserror` + `miette` diagnostic codes) covers the app
surface: `Database`, `Internal { reason, source }`, `NotFound`, `InvalidInput`.
The `Internal` variant wraps the `anyhow::Error` sources that still come back
from upstream rsky traits; that migration is tracked for a later cleanup but
lives at exactly one call site per variant today.

## Dev setup

`mise.toml` pins Rust 1.97.0, `cargo-nextest`, and `nose`, and defines tasks:

| Task                 | What it does                                  |
|----------------------|-----------------------------------------------|
| `mise run check`     | `cargo check --workspace --all-targets`       |
| `mise run fmt`       | `cargo fmt --all -- --check`                  |
| `mise run format`    | `cargo fmt --all`                             |
| `mise run lint`      | `cargo clippy --workspace --all-targets -- -D warnings` |
| `mise run test`      | `cargo nextest run --workspace`               |
| `mise run dup`       | `nose` duplication report (excludes migration files) |
| `mise run infra-up`  | Start MinIO in the background                 |
| `mise run infra-down`| Stop MinIO                                    |
| `mise run dev`       | `cargo run -p cacos-pds` (depends on `infra-up`) |

`docker-compose.yaml` runs MinIO plus a `mc-init` sidecar that creates the `cacos` bucket on first boot.

[sea-orm]:https://docs.rs/sea-orm
