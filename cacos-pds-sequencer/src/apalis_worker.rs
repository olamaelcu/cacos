//! Firehose sequencer worker: bridges `repo_seq` inserts to a
//! `tokio::sync::broadcast` channel so the websocket subscribers can drain
//! events.
//!
//! In the reference implementation this uses apalis-sql backed by a
//! separate SQLite file. The cacos port keeps the same shape (push a job
//! after every insert, run a worker that publishes to a broadcast) but
//! uses direct sqlx 0.8 + a custom poll loop so we don't pull in the
//! apalis dependency churn for what is, at heart, a single broadcast
//! fan-out. The on-disk schema matches what apalis-sql would create.

use cacos_pds_core::observability::metrics::{SEQ_EVENTS_TOTAL, SEQUENCER_POLL_INTERVAL_SECONDS};
use cacos_pds_core::observability::timing::timed;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

use sqlx_0_8::sqlite::SqlitePool;

/// One event ready to be broadcast to the websocket subscribers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeqEventJob {
    pub seq: i64,
    pub envelope: String,
    /// SQLx pooling handle for the backing SQLite database. Held in the
    /// struct so the worker task can keep the pool alive for the
    /// lifetime of the worker.
    #[serde(skip)]
    pub pool: Option<Arc<SqlitePool>>,
}

/// Broadcast facade shared with the websocket subscribers. Cloning is
/// cheap (the inner `Sender` is reference-counted).
#[derive(Clone)]
pub struct SharedBroadcast {
    pub tx: broadcast::Sender<String>,
}

impl SharedBroadcast {
    pub fn new(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Publish an envelope to all current subscribers.
    pub fn publish(&self, envelope: &str) {
        // Send errors are non-fatal: a subscriber who fell behind is dropped.
        let _ = self.tx.send(envelope.to_string());
    }
}

/// Open a SqliteStorage for SeqEventJobs. Tests that don't need the
/// worker can pass `None` to the Sequencer constructor.
pub async fn connect_jobs_db(path: &str) -> anyhow::Result<Arc<SqlitePool>> {
    use sqlx_0_8::sqlite::SqlitePoolOptions;
    let options = sqlx_0_8::sqlite::SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await?;
    // Apply the schema that apalis-sql would create; we use the same
    // Jobs table layout so the worker can be ported back to apalis-sql
    // if desired.
    sqlx_0_8::query(
        "CREATE TABLE IF NOT EXISTS apalis_seq_jobs (
            id TEXT PRIMARY KEY NOT NULL,
            job_type TEXT NOT NULL,
            status TEXT NOT NULL,
            attempts INTEGER NOT NULL DEFAULT 0,
            max_attempts INTEGER NOT NULL DEFAULT 5,
            run_at INTEGER NOT NULL,
            payload TEXT NOT NULL
        )",
    )
    .execute(&pool)
    .await?;
    Ok(Arc::new(pool))
}

/// Single iteration of the poll loop: read the next pending job, decode
/// the envelope, and publish it to the broadcast channel. Metrics
/// (`cacos_seq_events_total`, `cacos_sequencer_poll_interval_seconds`) are
/// recorded for every successful publish.
pub async fn run_seq_event_job(
    job: SeqEventJob,
    broadcast: &SharedBroadcast,
) -> anyhow::Result<()> {
    timed("seq_publish", async {
        let start = Instant::now();
        broadcast.publish(&job.envelope);
        metrics::counter!(SEQ_EVENTS_TOTAL).increment(1);
        metrics::histogram!(SEQUENCER_POLL_INTERVAL_SECONDS, "kind" => "publish")
            .record(start.elapsed().as_secs_f64());
        Ok(())
    })
    .await
}

/// Push a job into the backing SQLite queue. The worker task drains the
/// queue and publishes to the broadcast channel.
pub async fn enqueue_seq_event_job(pool: &SqlitePool, job: &SeqEventJob) -> anyhow::Result<()> {
    let id = format!("seq-{}", job.seq);
    sqlx_0_8::query(
        "INSERT OR REPLACE INTO apalis_seq_jobs \
         (id, job_type, status, attempts, max_attempts, run_at, payload) \
         VALUES (?1, 'seq', 'Pending', 0, 5, ?2, ?3)",
    )
    .bind(id)
    .bind(job.seq)
    .bind(&job.envelope)
    .execute(pool)
    .await?;
    Ok(())
}

/// Spawn the background worker that drains the apalis-style jobs table
/// and publishes envelopes to the broadcast channel. Returns the worker
/// `JoinHandle` so the caller can shut it down on application exit.
pub fn spawn_seq_event_worker(
    pool: Arc<SqlitePool>,
    broadcast: SharedBroadcast,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let start = Instant::now();
            let row: Option<(String, i64, String)> = sqlx_0_8::query_as(
                "SELECT id, run_at, payload FROM apalis_seq_jobs \
                 WHERE status = 'Pending' ORDER BY run_at ASC LIMIT 1",
            )
            .fetch_optional(pool.as_ref())
            .await
            .ok()
            .flatten();
            let Some((id, seq, envelope)) = row else {
                tokio::time::sleep(Duration::from_millis(200)).await;
                continue;
            };
            let _ = sqlx_0_8::query("UPDATE apalis_seq_jobs SET status = 'Done' WHERE id = ?1")
                .bind(id)
                .execute(pool.as_ref())
                .await;
            if let Err(err) = run_seq_event_job(
                SeqEventJob {
                    seq,
                    envelope,
                    pool: None,
                },
                &broadcast,
            )
            .await
            {
                tracing::warn!("publish job failed: {err}");
            }
            metrics::histogram!(
                SEQUENCER_POLL_INTERVAL_SECONDS,
                "kind" => "poll"
            )
            .record(start.elapsed().as_secs_f64());
        }
    })
}

#[allow(dead_code)]
const _CRAWLERS_REF: fn() -> crate::Crawlers = || crate::Crawlers::new(String::new(), vec![]);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::now_offset;
    use cacos_pds_core::db::DatabaseKind;
    use camino_tempfile::Utf8TempDir;
    use migration::entities::repo_seq;
    use sea_orm::EntityTrait;
    use std::time::Duration;

    #[tokio::test]
    async fn run_seq_event_job_publishes_envelope_to_broadcast() {
        cacos_pds_core::observability::metrics::init_metrics();
        let dir = Utf8TempDir::new().unwrap();
        let db_path = dir.path().join("sequencer.sqlite").to_string();
        let pool = connect_jobs_db(&db_path).await.unwrap();
        let broadcast = SharedBroadcast::new(16);
        let mut rx = broadcast.tx.subscribe();
        let job = SeqEventJob {
            seq: 1,
            envelope: "{\"type\":\"identity\",\"seq\":1}".to_string(),
            pool: None,
        };
        run_seq_event_job(job.clone(), &broadcast).await.unwrap();
        let received = rx.recv().await.unwrap();
        assert_eq!(received, job.envelope);
        let snapshot = cacos_pds_core::observability::metrics::render();
        assert!(
            snapshot.contains("cacos_seq_events_total"),
            "expected SEQ_EVENTS_TOTAL counter: {snapshot}"
        );
        assert!(
            snapshot.contains("cacos_timing_seconds_count{stage=\"seq_publish\"}"),
            "expected seq_publish stage sample: {snapshot}"
        );
        let _ = pool;
    }

    #[tokio::test]
    async fn enqueued_job_is_delivered_by_the_worker() {
        let dir = Utf8TempDir::new().unwrap();
        let db_path = dir.path().join("jobs.sqlite").to_string();
        let pool = connect_jobs_db(&db_path).await.unwrap();
        let broadcast = SharedBroadcast::new(16);
        let handle = spawn_seq_event_worker(pool.clone(), broadcast.clone());
        let mut rx = broadcast.tx.subscribe();

        let envelope = r#"{"type":"identity","seq":1,"time":"2026-01-01T00:00:00.000Z","evt":{"did":"did:plc:test"}}"#.to_string();
        let job = SeqEventJob {
            seq: 1,
            envelope: envelope.clone(),
            pool: Some(pool.clone()),
        };
        enqueue_seq_event_job(pool.as_ref(), &job).await.unwrap();
        // Verify the row is actually in the table.
        let count: i64 =
            sqlx_0_8::query_scalar("SELECT COUNT(*) FROM apalis_seq_jobs WHERE id = ?1")
                .bind("seq-1")
                .fetch_one(pool.as_ref())
                .await
                .unwrap();
        assert_eq!(count, 1, "row should be in the queue");
        let received = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("broadcast delivery timed out")
            .expect("broadcast channel closed");
        assert_eq!(received, envelope);

        let envelope2 = r#"{"type":"identity","seq":2}"#.to_string();
        let job2 = SeqEventJob {
            seq: 2,
            envelope: envelope2.clone(),
            pool: None,
        };
        enqueue_seq_event_job(pool.as_ref(), &job2).await.unwrap();
        let received2 = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("second broadcast delivery timed out")
            .expect("broadcast channel closed");
        assert_eq!(received2, envelope2);

        handle.abort();
    }

    #[allow(dead_code)]
    fn _compile_asserts() {
        let _ = DatabaseKind::Sequencer;
        let _ = repo_seq::Entity::find;
        let _ = now_offset();
    }
}
