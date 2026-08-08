# Sequencer Implementation Plan (Plan 07: events.rs + apalis + poem websocket subscribeRepos)

The architecture, contracts, and reconciliation decisions for this plan are recorded in [ADR-0008 — Sequencer and subscribeRepos firehose](../../../doc/adr/0008-sequencer-and-subscriberepos-firehose.md).

**Goal:** Port the rsky sequencer (event formatting, repo_seq DB logic, outbox backfill/cutover) into the cacos `pds` crate, replacing the reference `EVENT_EMITTER` poll loop with a delivery channel (one job per sequenced event) and a poem websocket `subscribeRepos` endpoint.

**Status:** Implemented on branch `2026-08-04-07-sequencer` (worktree `.worktrees/2026-08-04-07-sequencer`). Workspace test suite: 228 passed, 0 failed.

**See also:**
- [ADR-0008 — Sequencer and subscribeRepos firehose](../../../doc/adr/0008-sequencer-and-subscriberepos-firehose.md)
- [ADR-0002 — PDS migration crate](../../../doc/adr/0002-pds-migration-crate.md) (typed `repo_seq` columns)
- [ADR-0003 — PDS observability stack](../../../doc/adr/0003-pds-observability-stack.md) (`cacos_*` metrics reserved for this work)
- [ADR-0006 — AccountManager and account helpers over sea-orm](../../../doc/adr/0006-pds-accountmanager-and-account-helpers-over-sea-orm.md) (raw `Statement::from_sql_and_values` convention reused by the sequencer)
- [ADR-0007 — OAuth provider and auth verifier over sea-orm entities](../../../doc/adr/0007-oauth-provider-and-auth-verifier-over-sea-orm-entities.md) (cross-plan sequencing contract referenced by `RemoteCreateAccount`)

## Cross-plan contracts (as built)

- **Workspace:** root `/home/vrgl/Code/olamaelcu/cacos`, members `["migration", "pds"]`. All code lives in the `pds` crate.
- **Typed entity:** `migration/src/entities/repo_seq.rs` is the source of truth (re-exported via `pds/src/db/entities::repo_seq`). Columns: `seq: DbId` (BLOB(16) ULID PK, `auto_increment = false`), `did: Did` (TEXT), `event_type: String` (column `"eventType"`), `event: Vec<u8>` (BLOB), `invalidated: Option<i16>` (default 0), `sequenced_at: OffsetDateTime` (column `"sequencedAt"`).
- **Database open:** `DatabaseKind::Sequencer.open(path)` (impl `pds/src/db/mod.rs:39-66`).
- **rsky crates:** git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (Cargo.toml:7-15); no `vendor/` checkout.
- **Sequencer public API:** `Sequencer::new(db, crawlers, job_pool, last_seen)` returning `Self`; methods `curr`, `next_seq`, `earliest_after_time`, `request_seq_range`, `sequence_evt`, `sequence_commit`, `sequence_handle_update`, `sequence_identity_evt`, `sequence_account_evt`, `sequence_sync_evt`, `delete_all_for_user`. `RequestSeqRangeOpts { earliest_seq, latest_seq, earliest_time, limit }` uses typed `DbId` / `OffsetDateTime` cursors.
- **Delivery channel:** `SharedBroadcast { tx: broadcast::Sender<String> }` in `pds/src/sequencer/apalis_worker.rs`; `spawn_seq_event_worker(pool, broadcast)` runs the poller. The on-disk schema is `apalis_seq_jobs` (mirrors apalis-sql layout).
- **Metrics:** `cacos_last_seq` (gauge) from `sequence_evt`; `cacos_seq_events_total` (counter) and `cacos_sequencer_poll_interval_seconds` (histogram) from the worker; `cacos_outbox_buffer_lag` (gauge) from `Outbox::events`.
- **Config knobs:** `MAX_BUFFER: usize = 500`, `REPO_BACKFILL_LIMIT_MS: u64 = 3 * 24 * 60 * 60 * 1000` in the handler.
- **Documented deviations:** `EVENT_EMITTER` and the in-process poll loop are not ported; the typed `apalis` 0.7 worker was replaced with direct `sqlx_0_8` over an `apalis_seq_jobs` table that matches the apalis-sql schema (see ADR-0008 decision 4).

## File structure (as built)

```
pds/
└── src/
    ├── context.rs                    # SharedBroadcast / SharedSequencer seam (now in apalis_worker.rs)
    ├── sequencer/
    │   ├── mod.rs                    # Sequencer, RequestSeqRangeOpts, typed_seq_evt, repo_seq_from_row
    │   ├── events.rs                 # CommitEvt/HandleEvt/IdentityEvt/AccountEvt/SyncEvt/Typed*Evt/SeqEvt + formatters
    │   ├── db.rs                     # placeholder migration shim (entity is the migration crate)
    │   ├── crawlers.rs               # Crawlers + CrawlerRequest + APP_USER_AGENT
    │   ├── ws_frames.rs              # FrameType, MessageFrame, ErrorFrame, InfoFrameBody
    │   ├── apalis_worker.rs          # SeqEventJob, SharedBroadcast, connect_jobs_db, run_seq_event_job, spawn_seq_event_worker
    │   └── outbox.rs                 # Outbox + OutboxStream (broadcast subscription, backfill/cutover/live loop)
    └── xrpc/com/atproto/sync/
        └── subscribe_repos.rs        # poem WebSocket handler (cursor validation, OutdatedCursor, 30s ping)
```
