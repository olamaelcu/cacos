# Task 5: `outbox.rs` — ported Outbox with broadcast subscription

**Files:**
- Create: `pds/src/sequencer/outbox.rs`
- Test: `pds/src/sequencer/outbox.rs` (`#[cfg(test)] mod tests`)
- Modify: `pds/Cargo.toml` (add `async-stream`)

Port source: the rsky-pds outbox is now reached through the git-pinned `rsky-common` / `rsky-repo` crates (`Cargo.toml:8-15`). The shape (`OutboxOpts`, `Outbox`, `PAGE_SIZE = 500`, `events()`, `get_backfill`) is preserved verbatim. **Adaptations (documented):** (1) `EVENT_EMITTER` subscription is replaced by a `tokio::sync::broadcast::Sender<String>` receiver; (2) the reference's live loop drains `out_buffer` with a 2s `timeout` — the broadcast receiver is drained with `try_recv()` before each buffer drain (async-stream's `try_stream!` cannot `yield` inside `tokio::select!` — see tokio-rs/async-stream#27/#63 — so the loop keeps the reference's proven `while let ... timeout(...)` shape); (3) `get_backfill`'s half-page stop reads `self.sequencer.curr()` instead of the poll-loop cursor (there is no poll loop; same semantics); (4) `cacos_outbox_buffer_lag` gauge is refreshed each loop iteration.

- [ ] **Step 1: Add the dependency to `pds/Cargo.toml`**

```toml
# [dependencies] — add
async-stream = "0.3"
```

- [ ] **Step 2: Write the failing tests in `pds/src/sequencer/outbox.rs`**

Create `pds/src/sequencer/outbox.rs` containing ONLY this test module for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::SharedBroadcast;
    use crate::sequencer::apalis_worker::connect_jobs_db;
    use crate::sequencer::crawlers::Crawlers;
    use crate::sequencer::events::{seq_evt_to_envelope, IdentityEvt, SeqEvt, TypedIdentityEvt};
    use crate::sequencer::Sequencer;
    use futures::StreamExt;
    use migration::types::db_id::DbId;
    use std::time::Duration as StdDuration;
    use tokio::sync::broadcast;
    use tokio::time::{timeout, Duration as TokioDuration};

    async fn test_sequencer() -> (tempfile::TempDir, Sequencer) {
        let dir = tempfile::tempdir().unwrap();
        // Open via `DatabaseKind::Sequencer.open()` (impl at pds/src/db/mod.rs:39-66);
        // `crate::db::open_sequencer_db` is the thin shim added by this plan.
        let db = crate::db::DatabaseKind::Sequencer
            .open(camino::Utf8Path::new(&dir.path().join("sequencer.sqlite")))
            .await
            .unwrap();
        let job_storage = connect_jobs_db(dir.path().join("jobs.sqlite"))
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

    fn identity_envelope(seq: DbId, did: &str) -> String {
        seq_evt_to_envelope(&SeqEvt::TypedIdentityEvt(TypedIdentityEvt {
            r#type: "identity".to_string(),
            seq,
            time: time::OffsetDateTime::now_utc()
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap(),
            evt: IdentityEvt {
                did: did.to_owned(),
                handle: None,
            },
        }))
    }

    #[tokio::test]
    async fn events_backfills_from_cursor_then_continues_live() {
        let (_dir, mut sequencer) = test_sequencer().await;
        for i in 1..=3 {
            sequencer
                .sequence_identity_evt(migration::types::did::Did::from(format!("did:plc:ob{i}")), None)
                .await
                .unwrap();
        }
        let (tx, _rx) = broadcast::channel::<String>(64);
        let mut outbox = Outbox::new(
            sequencer.clone(),
            tx.clone(),
            Some(OutboxOpts { max_buffer_size: 500 }),
        );
        let mut stream = outbox.events(Some(DbId::new())).await;

        // backfill from cursor 1 -> events 2 and 3
        let evt2 = stream.next().await.unwrap().unwrap();
        assert_eq!(evt2.seq(), 2);
        let evt3 = stream.next().await.unwrap().unwrap();
        assert_eq!(evt3.seq(), 3);

        // live delivery through the broadcast channel (the stream is already
        // subscribed, so the envelope is not dropped)
        tx.send(identity_envelope(DbId::new(), "did:plc:ob4")).unwrap();
        let evt4 = timeout(TokioDuration::from_secs(5), stream.next())
            .await
            .expect("timed out waiting for live event")
            .unwrap()
            .unwrap();
        assert_eq!(evt4.seq(), 4);
    }

    #[tokio::test]
    async fn events_drops_stale_envelopes_at_or_below_last_seen() {
        let (_dir, mut sequencer) = test_sequencer().await;
        for i in 1..=3 {
            sequencer
                .sequence_identity_evt(migration::types::did::Did::from(format!("did:plc:ob{i}")), None)
                .await
                .unwrap();
        }
        let (tx, _rx) = broadcast::channel::<String>(64);
        let mut outbox = Outbox::new(
            sequencer.clone(),
            tx.clone(),
            Some(OutboxOpts { max_buffer_size: 500 }),
        );
        let mut stream = outbox.events(Some(DbId::new())).await;
        let evt2 = stream.next().await.unwrap().unwrap();
        assert_eq!(evt2.seq(), 2);
        let evt3 = stream.next().await.unwrap().unwrap();
        assert_eq!(evt3.seq(), 3);

        // a stale envelope (seq 3, already delivered) is dropped by the
        // `seq > last_seen` guard; the fresh seq 5 still arrives
        tx.send(identity_envelope(DbId::new(), "did:plc:stale")).unwrap();
        tx.send(identity_envelope(DbId::new(), "did:plc:ob5")).unwrap();
        let evt5 = timeout(TokioDuration::from_secs(5), stream.next())
            .await
            .expect("timed out waiting for live event")
            .unwrap()
            .unwrap();
        assert_eq!(evt5.seq(), 5);
    }

    #[tokio::test]
    async fn events_without_cursor_streams_live_only() {
        let (_dir, sequencer) = test_sequencer().await;
        let (tx, _rx) = broadcast::channel::<String>(64);
        let mut outbox = Outbox::new(sequencer.clone(), tx.clone(), None);

        // drive the stream in a task so it subscribes before we send
        let collector = tokio::spawn(async move {
            let mut stream = outbox.events(None).await;
            let first = stream.next().await.unwrap().unwrap();
            first.seq()
        });
        // wait until the outbox holds a broadcast receiver
        for _ in 0..100 {
            if tx.receiver_count() > 0 {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(10)).await;
        }
        tx.send(identity_envelope(DbId::new(), "did:plc:live")).unwrap();
        let seq = timeout(TokioDuration::from_secs(5), collector)
            .await
            .expect("collector never received an event")
            .unwrap();
        assert_eq!(seq, 1);
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p cacos-pds sequencer::outbox::tests`
Expected: FAIL — `cannot find struct, variant or union type 'Outbox'` (and `OutboxOpts`, `Outbox::new`, `Outbox::events` unresolved).

- [ ] **Step 4: Implement `pds/src/sequencer/outbox.rs`** (keep the test module from Step 2 appended)

```rust
use crate::sequencer::events::SeqEvt;
use crate::sequencer::{RequestSeqRangeOpts, Sequencer};
use anyhow::{anyhow, Result};
use async_stream::try_stream;
use futures::stream::Stream;
use futures::StreamExt;
use rsky_common::r#async::{AsyncBuffer, AsyncBufferFullError};
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex, RwLock};
use tokio::time::{timeout, Duration};

#[derive(Debug, Clone)]
pub struct OutboxOpts {
    pub max_buffer_size: usize,
}

pub struct Outbox {
    caught_up: Arc<Mutex<bool>>,
    pub last_seen: DbId,
    pub cutover_buffer: Arc<Mutex<Vec<SeqEvt>>>,
    pub out_buffer: Arc<RwLock<AsyncBuffer<SeqEvt>>>,
    pub sequencer: Sequencer,
    pub backfill_cursor: Option<DbId>,
    /// Live-delivery subscription replacing the reference EVENT_EMITTER.
    broadcast: broadcast::Sender<String>,
}

const PAGE_SIZE: i64 = 500;

impl Outbox {
    pub fn new(
        sequencer: Sequencer,
        broadcast: broadcast::Sender<String>,
        opts: Option<OutboxOpts>,
    ) -> Self {
        let OutboxOpts { max_buffer_size } = opts.unwrap_or(OutboxOpts {
            max_buffer_size: 500,
        });
        Self {
            sequencer,
            broadcast,
            caught_up: Arc::new(Mutex::new(false)),
            last_seen: DbId::default(),
            cutover_buffer: Arc::new(Mutex::new(vec![])),
            out_buffer: Arc::new(RwLock::new(AsyncBuffer::new(Some(max_buffer_size)))),
            backfill_cursor: None,
        }
    }

    pub async fn events(
        &mut self,
        backfill_cursor: Option<DbId>,
    ) -> impl Stream<Item = Result<SeqEvt>> + use<'_> {
        try_stream! {
            if let Some(cursor) = backfill_cursor {
                let backfill_stream = self.get_backfill(cursor).await;
                futures::pin_mut!(backfill_stream);
                while let Some(Ok(evt)) = backfill_stream.next().await {
                    yield evt;
                }
            } else {
                let mut bool_lock = self.caught_up.lock().await;
                *bool_lock = true;
            }

            let caught_up = Arc::clone(&self.caught_up);
            let out_buffer = Arc::clone(&self.out_buffer);
            let cutover_buffer = Arc::clone(&self.cutover_buffer);
            let mut rx = self.broadcast.subscribe();

            if let Some(cursor) = backfill_cursor {
                let earliest_seq = if self.last_seen > DbId::default() {
                    Some(self.last_seen)
                } else {
                    Some(cursor)
                };
                let cutover_evts = self.sequencer.request_seq_range(RequestSeqRangeOpts {
                    earliest_seq,
                    latest_seq: None,
                    earliest_time: None,
                    limit: Some(PAGE_SIZE),
                }).await?;
                {
                    let out_buffer_lock = self.out_buffer.read().await;
                    let mut cutover_lock = self.cutover_buffer.lock().await;
                    out_buffer_lock.push_many(cutover_evts);
                    out_buffer_lock.push_many(cutover_lock.drain(..).collect());
                }
            }
            {
                let mut bool_lock = self.caught_up.lock().await;
                *bool_lock = true;
            }

            loop {
                // 1) drain any pending live envelopes into the buffer
                loop {
                    match rx.try_recv() {
                        Ok(envelope) => {
                            let evt: SeqEvt = serde_json::from_str(&envelope).unwrap();
                            add_to_buffer(
                                Arc::clone(&caught_up),
                                Arc::clone(&out_buffer),
                                Arc::clone(&cutover_buffer),
                                evt,
                            )
                            .await;
                        }
                        Err(broadcast::error::TryRecvError::Empty) => break,
                        Err(broadcast::error::TryRecvError::Lagged(_)) => continue,
                        Err(broadcast::error::TryRecvError::Closed) => break,
                    }
                }

                // 2) drain the out buffer (reference pattern; yields directly)
                while let Ok(Some(res)) =
                    timeout(Duration::from_secs(2), self.out_buffer.write().await.next()).await
                {
                    let evt = res.map_err(|error| {
                        match error.downcast_ref() {
                            Some(AsyncBufferFullError(_)) => {
                                anyhow!("Stream consumer too slow.".to_string())
                            }
                            _ => anyhow!(error.to_string()),
                        }
                    })?;
                    if evt.seq() > self.last_seen {
                        self.last_seen = evt.seq();
                        let lag = self.outbox_lag().await;
                        metrics::gauge!("cacos_outbox_buffer_lag", lag as f64);
                        yield evt;
                    }
                }

                // 3) idle tick: refresh the lag gauge, avoid busy-spinning
                let lag = self.outbox_lag().await;
                metrics::gauge!("cacos_outbox_buffer_lag", lag as f64);
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }

    pub async fn get_backfill(
        &mut self,
        backfill_cursor: DbId,
    ) -> impl Stream<Item = Result<SeqEvt>> + use<'_> {
        try_stream! {
            loop {
                let earliest_seq = if self.last_seen > DbId::default() {
                    Some(self.last_seen)
                } else {
                    Some(backfill_cursor)
                };
                let evts = match self.sequencer.request_seq_range(RequestSeqRangeOpts {
                    earliest_seq,
                    latest_seq: None,
                    earliest_time: None,
                    limit: Some(PAGE_SIZE),
                }).await {
                    Ok(res) => res,
                    Err(_) => break
                };
                for evt in evts.iter() {
                    self.last_seen = evt.seq();
                    yield evt.clone();
                }
                // reference reads the poll-loop cursor here; cacos has no poll
                // loop, so the current max seq in the db (same half-page stop)
                let seq_cursor = self
                    .sequencer
                    .curr()
                    .await
                    .unwrap_or(Some(DbId::default()))
                    .unwrap_or_default();
                if seq_cursor.timestamp_ms() as i64 - self.last_seen.timestamp_ms() as i64
                    < (PAGE_SIZE / 2)
                {
                    break;
                }
                if evts.is_empty() {
                    break;
                }
            }
        }
    }

    /// Returns the lag in milliseconds between the last delivered event and
    /// the current DB max. ULID `timestamp_ms()` is the monotonic surrogate
    /// ordering key.
    async fn outbox_lag(&self) -> i64 {
        let curr = self
            .sequencer
            .curr()
            .await
            .unwrap_or(Some(DbId::default()))
            .unwrap_or_default();
        let lag_ms = curr.timestamp_ms() as i64 - self.last_seen.timestamp_ms() as i64;
        lag_ms.max(0)
    }
}

/// Route one live envelope: buffer it for the subscriber, or stage it in the
/// cutover buffer while the subscriber is still catching up (reference
/// `add_to_buffer`).
async fn add_to_buffer(
    caught_up: Arc<Mutex<bool>>,
    out_buffer: Arc<RwLock<AsyncBuffer<SeqEvt>>>,
    cutover_buffer: Arc<Mutex<Vec<SeqEvt>>>,
    evt: SeqEvt,
) {
    if *caught_up.lock().await {
        out_buffer.read().await.push_many(vec![evt]);
    } else {
        cutover_buffer.lock().await.push(evt);
    }
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p cacos-pds sequencer::outbox::tests`
Expected: `test result: ok. 3 passed` (each test polls up to 5s for live events; the backfill assertions are immediate).

- [ ] **Step 6: Commit**

```bash
git add pds/Cargo.toml pds/src/sequencer/outbox.rs
git commit -m "feat(sequencer): port outbox with broadcast subscription (backfill, cutover, live loop)"
```
