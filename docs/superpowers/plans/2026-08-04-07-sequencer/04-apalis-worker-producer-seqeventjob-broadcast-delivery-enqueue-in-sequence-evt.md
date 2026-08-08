# Task 4: apalis worker + producer — `SeqEventJob`, broadcast delivery, enqueue in `sequence_evt`

**Files:**
- Create: `pds/src/sequencer/apalis_worker.rs`
- Modify: `pds/src/context.rs` (add `SharedBroadcast`, `SharedSequencer`)
- Modify: `pds/src/sequencer/mod.rs` (add `job_storage` field, `SeqEventJob`/`SqliteStorage` imports, enqueue in `sequence_evt`, `Storage` trait import)
- Test: `pds/src/sequencer/apalis_worker.rs` (`#[cfg(test)] mod tests`); update the `test_sequencer()` helper in `pds/src/sequencer/mod.rs`
- Modify: `pds/Cargo.toml` (deps below)

Port source: none — this is new glue replacing `EVENT_EMITTER`. Verified against `apalis` 0.7.4 + `apalis-sql` 0.7.4 (sqlx 0.8): the sqlite storage lives in the separate `apalis-sql` crate (`apalis_sql::sqlite::SqliteStorage`), `SqliteStorage::<()>::setup(&pool)` creates the job tables (the `migrate` feature, on by default), and jobs are plain `Serialize + Deserialize` structs (no derive macro in 0.7).

- [ ] **Step 1: Add dependencies to `pds/Cargo.toml`**

```toml
# [dependencies] — add (workspace already pins these in Cargo.toml:37-39)
apalis = { workspace = true }                       # 0.6
apalis-sql = { workspace = true }                   # 0.6, sqlite/tokio-comp/migrate
sqlx_0_8 = { workspace = true }                     # alias for sqlx 0.8 (apalis-sql 0.6's pin)
# NOTE: workspace sqlx is 0.9 (sea-orm's pin). apalis-sql 0.6 requires sqlx 0.8.
# Both versions resolve side-by-side in Cargo.lock; the 0.8 dep is aliased
# `sqlx_0_8` so Plan 09 can `use crate::db::sqlx_0_8` unambiguously.
```

Run: `cargo check -p cacos-pds`
Expected: resolves (large first build; apalis-core, sqlx 0.8, sqlx-sqlite are compiled alongside the existing sqlx 0.9 from sea-orm).

- [ ] **Step 2: Add the shared state to `pds/src/context.rs`**

Append:

```rust
use tokio::sync::broadcast;

/// Live-delivery glue replacing rsky's EVENT_EMITTER: a broadcast channel of
/// JSON event envelopes. The apalis worker publishes; each `Outbox` subscribes.
/// Clone this into the apalis worker state and the poem route state.
#[derive(Clone)]
pub struct SharedBroadcast {
    pub tx: broadcast::Sender<String>,
}

/// Shared handle to the sequencer for xrpc handlers. Shape matches Plan 08's
/// SharedState contract: `SharedState.sequencer: SharedSequencer` with a pub
/// `sequencer: RwLock<Sequencer>` field.
#[derive(Clone)]
pub struct SharedSequencer {
    pub sequencer: tokio::sync::RwLock<crate::sequencer::Sequencer>,
}
```

- [ ] **Step 3: Write the failing tests in `pds/src/sequencer/apalis_worker.rs`**

Create `pds/src/sequencer/apalis_worker.rs` containing ONLY this test module for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::SharedBroadcast;
    use crate::sequencer::events::{IdentityEvt, TypedIdentityEvt};
    use migration::types::db_id::DbId;
    use std::time::Duration;
    use tokio::sync::broadcast;

    fn sample_envelope() -> String {
        serde_json::to_string(&TypedIdentityEvt {
            r#type: "identity".to_string(),
            seq: DbId::new(),
            time: time::OffsetDateTime::now_utc()
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap(),
            evt: IdentityEvt {
                did: "did:plc:job".to_owned(),
                handle: None,
            },
        })
        .unwrap()
    }

    #[tokio::test]
    async fn run_seq_event_job_publishes_envelope_to_broadcast() {
        let (tx, mut rx) = broadcast::channel::<String>(16);
        let state = SharedBroadcast { tx };
        let envelope = sample_envelope();
        let seq = DbId::new();
        let job = SeqEventJob {
            seq,
            envelope: envelope.clone(),
        };
        run_seq_event_job(job, Data(state)).await.unwrap();
        assert_eq!(rx.try_recv().unwrap(), envelope);
    }

    #[tokio::test]
    async fn enqueued_job_is_delivered_by_the_worker() {
        let dir = tempfile::tempdir().unwrap();
        let mut storage = connect_jobs_db(dir.path().join("jobs.sqlite"))
            .await
            .unwrap();
        let (tx, mut rx) = broadcast::channel::<String>(16);
        let envelope = sample_envelope();

        storage
            .push(SeqEventJob {
                seq: DbId::new(),
                envelope: envelope.clone(),
            })
            .await
            .unwrap();

        let worker_storage = storage.clone();
        let state = SharedBroadcast { tx: tx.clone() };
        let handle = tokio::spawn(async move {
            let worker = WorkerBuilder::new("test-worker")
                .data(state)
                .backend(worker_storage)
                .build(run_seq_event_job);
            let _ = worker.run().await;
        });

        // poll for delivery (the apalis-sql poller runs on a short interval)
        let mut seen = false;
        for _ in 0..100 {
            if let Ok(recv) = rx.try_recv() {
                assert_eq!(recv, envelope);
                seen = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(seen, "worker never published the envelope");
        handle.abort();
    }
}
```

- [ ] **Step 4: Run the tests to verify they fail**

Run: `cargo test -p cacos-pds sequencer::apalis_worker::tests`
Expected: FAIL — `cannot find struct, variant or union type 'SeqEventJob'` (and `connect_jobs_db`, `run_seq_event_job`, `Data`, `WorkerBuilder` unresolved).

- [ ] **Step 5: Implement `pds/src/sequencer/apalis_worker.rs`** (keep the test module from Step 3 appended)

```rust
use crate::context::SharedBroadcast;
use apalis::prelude::*;
use apalis_sql::sqlite::SqliteStorage;
use migration::types::db_id::DbId;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::str::FromStr;

/// One apalis job per sequenced event. `envelope` is the JSON-encoded `SeqEvt`
/// (see `events::seq_evt_to_envelope`); the worker re-reads nothing — the
/// envelope IS the event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeqEventJob {
    /// ULID `DbId` of the event. Serialized through `DbId`'s canonical
    /// `Display` impl (26-char Crockford) on the JSON envelope boundary;
    /// the apalis storage row keeps the BLOB(16) form for cheap ordering.
    pub seq: DbId,
    pub envelope: String,
}

/// Open (creating if missing) the sqlite jobs database and return an apalis
/// storage handle on it. `SqliteStorage::<()>::setup` creates the apalis job
/// tables via sqlx migrations (the `migrate` feature, enabled by default).
/// Env `PDS_JOBS_DB_LOCATION` (default `{data}/jobs.sqlite`) is resolved by the
/// caller (main.rs, Plan 08); this function takes the resolved path.
pub async fn connect_jobs_db(
    location: impl AsRef<Path>,
) -> anyhow::Result<SqliteStorage<SeqEventJob>> {
    let url = format!("sqlite://{}", location.as_ref().display());
    let options = SqliteConnectOptions::from_str(&url)?
        .create_if_missing(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await?;
    SqliteStorage::<()>::setup(&pool).await?;
    Ok(SqliteStorage::new(pool))
}

/// Deliver one sequenced event: publish the JSON envelope to the live
/// broadcast channel and record metrics. A send failure (no subscribers) is a
/// no-op — delivery is at-least-once via the jobs DB, and subscribers backfill
/// from `repo_seq` on connect.
pub async fn run_seq_event_job(
    job: SeqEventJob,
    state: Data<SharedBroadcast>,
) -> Result<(), apalis::prelude::Error> {
    let start = std::time::Instant::now();
    tracing::debug!(seq = job.seq, "delivering sequenced event");
    let _ = state.tx.send(job.envelope);
    metrics::counter!("cacos_seq_events_total").increment(1);
    metrics::histogram!(
        "cacos_sequencer_poll_interval_seconds",
        start.elapsed().as_secs_f64()
    );
    Ok(())
}

/// Spawn the single sequencer delivery worker on the current runtime.
pub fn spawn_seq_event_worker(
    storage: SqliteStorage<SeqEventJob>,
    state: SharedBroadcast,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let worker = WorkerBuilder::new("cacos-seq-events")
            .data(state)
            .backend(storage)
            .build(run_seq_event_job);
        let _ = worker.run().await;
    })
}

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
```

(Note: the `use sqlx::...` line may be placed with the other imports at the top instead; it is shown separately only to keep the diff small.)

- [ ] **Step 6: Wire the producer into `pds/src/sequencer/mod.rs`**

(a) Add imports at the top of the module:

```rust
use crate::sequencer::apalis_worker::SeqEventJob;
use crate::sequencer::events::seq_evt_to_envelope;
use apalis::prelude::Storage;
use apalis_sql::sqlite::SqliteStorage;
```

(b) Add the `job_storage` field to the struct and update the constructor:

```rust
#[derive(Debug, Clone)]
pub struct Sequencer {
    pub db: DatabaseConnection,
    pub crawlers: Crawlers,
    pub last_seen: Option<DbId>,
    pub job_storage: SqliteStorage<SeqEventJob>,
}

impl Sequencer {
    pub fn new(
        db: DatabaseConnection,
        crawlers: Crawlers,
        job_storage: SqliteStorage<SeqEventJob>,
        last_seen: Option<DbId>,
    ) -> Self {
        Sequencer {
            db,
            crawlers,
            last_seen: Some(last_seen.unwrap_or_else(DbId::new)),
            job_storage,
        }
    }
    // ... all other methods unchanged
```

(c) Replace `sequence_evt` with the version that enqueues a job after the insert:

```rust
    pub async fn sequence_evt(&mut self, evt: RepoSeq) -> Result<DbId> {
        use crate::db::entities::repo_seq::{ActiveModel, Entity};
        use sea_orm::{ActiveModelTrait, ActiveValue};
        // The PK is application-generated (ULID), not SQL AUTOINCREMENT. The
        // value is minted here so the envelope and the live row share the same
        // `DbId`.
        let seq = DbId::new();
        let res = Entity::insert(ActiveModel {
            seq: ActiveValue::Set(seq),
            did: ActiveValue::Set(evt.did.clone()),
            event: ActiveValue::Set(evt.event.clone()),
            event_type: ActiveValue::Set(evt.event_type.clone()),
            invalidated: ActiveValue::Set(Some(0)), // table default; set explicitly
            sequenced_at: ActiveValue::Set(evt.sequenced_at),
        })
        .exec(&self.db)
        .await?;
        metrics::gauge!("cacos_last_seq", seq.timestamp_ms() as f64);

        // At-least-once delivery: one apalis job per sequenced event, carrying
        // the JSON envelope. The worker publishes it to the broadcast channel.
        let mut inserted = evt;
        inserted.seq = Some(seq);
        if let Some(seq_evt) = seq_evt_from_row(&inserted)? {
            let envelope = seq_evt_to_envelope(&seq_evt);
            self.job_storage.push(SeqEventJob { seq, envelope }).await?;
        }

        self.crawlers.notify_of_update().await?;
        Ok(seq)
    }
```

- [ ] **Step 7: Update the `test_sequencer()` helper in `pds/src/sequencer/mod.rs`** (the constructor now takes `job_storage`)

Replace the helper body with:

```rust
    async fn test_sequencer() -> (tempfile::TempDir, Sequencer) {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::db::open_sequencer_db(dir.path().join("sequencer.sqlite"))
            .await
            .unwrap();
        let job_storage = crate::sequencer::apalis_worker::connect_jobs_db(
            dir.path().join("jobs.sqlite"),
        )
        .await
        .unwrap();
        let sequencer = Sequencer::new(
            db,
            Crawlers::new("pds.test".to_owned(), vec![]),
            job_storage,
            None,
        );
        (dir, sequencer)
    }
```

- [ ] **Step 8: Run all sequencer tests to verify they pass**

Run: `cargo test -p cacos-pds sequencer::`
Expected: `test result: ok. N passed` — `repo_seq_new_uses_insert_defaults`, `sequences_and_reads_events`, `deletes_events_for_user`, `run_seq_event_job_publishes_envelope_to_broadcast`, `enqueued_job_is_delivered_by_the_worker` (the last one polls up to 5s).

- [ ] **Step 9: Commit**

```bash
git add pds/Cargo.toml pds/src/context.rs pds/src/sequencer/mod.rs pds/src/sequencer/apalis_worker.rs
git commit -m "feat(sequencer): apalis-backed delivery — one SeqEventJob per sequenced event to a broadcast channel"
```
