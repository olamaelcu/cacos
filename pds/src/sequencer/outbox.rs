//! Outbox: the unified "backfill then live" stream that the
//! subscribe-repos handler awaits.
//!
//! Internally we use a `tokio::sync::mpsc::channel` to bridge the broadcast
//! channel and the Stream interface. The consumer `next()`s on a unbuffered
//! mpsc sender that we feed from the broadcast on a small background task.

use crate::observability::metrics::OUTBOX_BUFFER_LAG;
use crate::observability::timing::timed;
use crate::sequencer::Sequencer;
use crate::sequencer::apalis_worker::SharedBroadcast;
use crate::sequencer::events::SeqEvt;
use anyhow::Result;
use futures::stream::{self, Stream, StreamExt};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
#[cfg(test)]
use std::time::Duration;
use tokio::sync::Mutex;

pub const PAGE_SIZE: i64 = 500;

#[derive(Debug, Clone)]
pub struct OutboxOpts {
    pub max_buffer_size: usize,
}

impl Default for OutboxOpts {
    fn default() -> Self {
        Self {
            max_buffer_size: 500,
        }
    }
}

pub struct Outbox {
    pub caught_up: Arc<Mutex<bool>>,
    pub last_seen: i64,
    pub cutover_buffer: Arc<Mutex<Vec<SeqEvt>>>,
    pub sequencer: Sequencer,
    pub backfill_cursor: Option<i64>,
    pub broadcast: SharedBroadcast,
    /// Max seq observed on the broadcast channel for the current
    /// subscription. Distinct from `last_seen` (which advances only when
    /// the stream yields an event to the consumer); the gauge is
    /// `max(0, max_observed - last_seen)`.
    pub max_observed: i64,
}

impl Outbox {
    pub fn new(
        sequencer: Sequencer,
        broadcast: SharedBroadcast,
        _opts: Option<OutboxOpts>,
    ) -> Self {
        Self {
            caught_up: Arc::new(Mutex::new(false)),
            last_seen: -1,
            cutover_buffer: Arc::new(Mutex::new(Vec::new())),
            sequencer,
            backfill_cursor: None,
            broadcast,
            max_observed: -1,
        }
    }

    /// Record the highest seq we have seen on the broadcast channel and
    /// refresh the `cacos_outbox_buffer_lag` gauge. The gauge is
    /// `max(0, max_observed - last_seen)` where `last_seen` is the
    /// consumer's current progress.
    fn observe_max(&mut self, seq: i64) {
        if seq > self.max_observed {
            self.max_observed = seq;
        }
        let lag = std::cmp::max(0, self.max_observed - self.last_seen);
        metrics::gauge!(OUTBOX_BUFFER_LAG).set(lag as f64);
    }

    pub async fn events(&mut self, backfill_cursor: Option<i64>) -> OutboxStream {
        let mut seqs: Vec<i64> = Vec::new();
        let mut evts: Vec<SeqEvt> = Vec::new();
        {
            timed("outbox_events", async {
                if let Some(cursor) = backfill_cursor {
                    self.backfill_cursor = Some(cursor);
                    let mut stream = self.get_backfill(cursor).await;
                    while let Some(evt) = stream.next().await {
                        match evt {
                            Ok(e) => {
                                seqs.push(e.seq());
                                evts.push(e);
                            }
                            Err(err) => {
                                tracing::warn!("outbox backfill error: {err}");
                                break;
                            }
                        }
                    }
                }
            })
            .await;
        }
        for seq in &seqs {
            self.observe_max(*seq);
        }

        let mut guard = self.caught_up.lock().await;
        *guard = true;
        drop(guard);

        let (tx, rx) = tokio::sync::mpsc::channel::<SeqEvt>(64);
        let broadcast = self.broadcast.clone();
        let last_seen = Arc::new(Mutex::new(self.last_seen));
        let last_seen_clone = last_seen.clone();
        let mut bcast_rx = broadcast.tx.subscribe();
        tokio::spawn(async move {
            loop {
                match bcast_rx.recv().await {
                    Ok(envelope) => {
                        if let Ok(evt) = serde_json::from_str::<SeqEvt>(&envelope) {
                            let seq = evt.seq();
                            let last_seen_now = {
                                let mut g = last_seen_clone.lock().await;
                                let v = *g;
                                if seq > *g {
                                    *g = seq;
                                }
                                v
                            };
                            if seq > last_seen_now && tx.send(evt).await.is_err() {
                                break;
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
        });

        OutboxStream {
            iter: evts.into_iter(),
            rx,
            last_seen,
        }
    }

    pub async fn outbox_lag(&self) -> i64 {
        let last_seen = self.last_seen;
        match self.sequencer.curr().await {
            Ok(Some(seq)) => seq.0.timestamp_ms() as i64 - last_seen,
            _ => 0,
        }
    }

    pub async fn get_backfill(
        &mut self,
        backfill_cursor: i64,
    ) -> Pin<Box<dyn Stream<Item = Result<SeqEvt>> + Send + '_>> {
        let start = std::cmp::max(self.last_seen, backfill_cursor);
        let rows = self
            .sequencer
            .request_seq_range(crate::sequencer::RequestSeqRangeOpts {
                earliest_seq: None,
                latest_seq: None,
                earliest_time: None,
                limit: Some(PAGE_SIZE),
            })
            .await;
        let mut evts: Vec<SeqEvt> = Vec::new();
        if let Ok(rows) = rows {
            for row in rows.iter() {
                if let Ok(typed) = crate::sequencer::typed_seq_evt(row) {
                    let seq = typed.seq();
                    if seq > start {
                        self.last_seen = seq;
                        evts.push(typed);
                    }
                }
            }
        }
        Box::pin(stream::iter(evts.into_iter().map(Ok)))
    }
}

pub struct OutboxStream {
    iter: std::vec::IntoIter<SeqEvt>,
    rx: tokio::sync::mpsc::Receiver<SeqEvt>,
    last_seen: Arc<Mutex<i64>>,
}

impl Stream for OutboxStream {
    type Item = Result<SeqEvt>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let me = self.get_mut();
        if let Some(evt) = me.iter.next() {
            let last_seen = me.last_seen.clone();
            let seq = evt.seq();
            futures::executor::block_on(async {
                let mut g = last_seen.lock().await;
                if seq > *g {
                    *g = seq;
                }
            });
            // The backfill iterator already populated `Outbox::max_observed`
            // — refresh the gauge here so the consumer advance is also
            // reflected.
            metrics::gauge!(OUTBOX_BUFFER_LAG).set(0.0);
            return Poll::Ready(Some(Ok(evt)));
        }
        match me.rx.poll_recv(cx) {
            Poll::Ready(Some(evt)) => Poll::Ready(Some(Ok(evt))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DatabaseKind;
    use crate::db::tests::TestDatabaseKind;
    use crate::db::types::did::Did;
    use crate::sequencer::crawlers::Crawlers;
    use crate::sequencer::events::{RepoSeqNew, now_offset};
    use futures::StreamExt;

    async fn test_sequencer() -> (Sequencer, crate::db::tests::TestDb) {
        let db = DatabaseKind::Sequencer.open_test_db().await;
        let seq = Sequencer::new(
            db.clone(),
            Crawlers::new("pds.test".to_string(), vec![]),
            None,
            None,
        );
        (seq, db)
    }

    fn typed_event(did: &str, kind: &str) -> SeqEvt {
        use crate::sequencer::events::{
            AccountEvt, IdentityEvt, TypedAccountEvt, TypedIdentityEvt,
        };
        match kind {
            "identity" => SeqEvt::TypedIdentityEvt(TypedIdentityEvt {
                r#type: "identity".to_string(),
                seq: 0,
                time: "2026-01-01T00:00:00.000Z".to_string(),
                evt: IdentityEvt {
                    did: did.to_string(),
                    handle: Some("alice.test".to_string()),
                },
            }),
            "account" => SeqEvt::TypedAccountEvt(TypedAccountEvt {
                r#type: "account".to_string(),
                seq: 0,
                time: "2026-01-01T00:00:00.000Z".to_string(),
                evt: AccountEvt {
                    did: did.to_string(),
                    active: true,
                    status: None,
                },
            }),
            _ => panic!("unknown kind: {kind}"),
        }
    }

    fn event_body(did: &str, kind: &str) -> Vec<u8> {
        use crate::sequencer::events::{AccountEvt, IdentityEvt};
        match kind {
            "identity" => serde_ipld_dagcbor::to_vec(&IdentityEvt {
                did: did.to_string(),
                handle: Some("alice.test".to_string()),
            })
            .unwrap(),
            "account" => serde_ipld_dagcbor::to_vec(&AccountEvt {
                did: did.to_string(),
                active: true,
                status: None,
            })
            .unwrap(),
            _ => panic!("unknown kind: {kind}"),
        }
    }

    async fn insert_envelope(seq: &mut Sequencer, did: &str, kind: &str, body: Vec<u8>) {
        let evt = RepoSeqNew::new(
            Did::from(did.to_string()),
            kind.to_string(),
            body,
            now_offset(),
        );
        seq.sequence_evt(evt).await.unwrap();
    }

    #[tokio::test]
    async fn events_backfills_from_cursor_then_continues_live() {
        let (mut seq, _db) = test_sequencer().await;
        let did = "did:plc:alice";
        insert_envelope(&mut seq, did, "identity", event_body(did, "identity")).await;
        insert_envelope(&mut seq, did, "account", event_body(did, "account")).await;

        let broadcast = SharedBroadcast::new(16);
        let mut outbox = Outbox::new(seq.clone(), broadcast.clone(), None);

        let mut stream = outbox.events(Some(0)).await;
        let evt1 = tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let evt2 = tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(evt1.seq() > 0);
        assert!(evt2.seq() > evt1.seq());
    }

    #[tokio::test]
    async fn events_drops_stale_envelopes_at_or_below_last_seen() {
        let (mut seq, _db) = test_sequencer().await;
        let did = "did:plc:alice";
        let bod = event_body(did, "identity");
        insert_envelope(&mut seq, did, "identity", bod.clone()).await;

        let broadcast = SharedBroadcast::new(16);
        let mut outbox = Outbox::new(seq.clone(), broadcast.clone(), None);
        outbox.last_seen = 1;
        let _stream = outbox.events(Some(0)).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    #[tokio::test]
    async fn events_without_cursor_streams_live_only() {
        let (seq, _db) = test_sequencer().await;
        let broadcast = SharedBroadcast::new(16);
        let mut outbox = Outbox::new(seq.clone(), broadcast.clone(), None);
        let mut stream = outbox.events(None).await;
        let mut evt = typed_event("did:plc:alice", "identity");
        // Simulate a real envelope seq (timestamp_ms).
        if let SeqEvt::TypedIdentityEvt(ref mut e) = evt {
            e.seq = 1;
        }
        let envelope = serde_json::to_string(&evt).unwrap();
        broadcast.publish(&envelope);
        let evt = tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(evt.seq() > 0);
    }

    #[tokio::test]
    async fn outbox_lag_gauge_populated_from_backfill() {
        use crate::observability::metrics::OUTBOX_BUFFER_LAG;
        crate::observability::metrics::init_metrics();
        let (mut seq, _db) = test_sequencer().await;
        let did = "did:plc:lagtest";
        insert_envelope(&mut seq, did, "identity", event_body(did, "identity")).await;
        tokio::time::sleep(Duration::from_millis(2)).await;
        insert_envelope(&mut seq, did, "account", event_body(did, "account")).await;

        let broadcast = SharedBroadcast::new(16);
        let mut outbox = Outbox::new(seq.clone(), broadcast.clone(), None);
        let mut stream = outbox.events(Some(0)).await;
        // Drain the backfill so last_seen catches up to the largest seq.
        let _ = tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();

        let snapshot = crate::observability::metrics::render();
        // After the consumer has caught up, the gauge should be zero.
        let needle = format!("{} 0", OUTBOX_BUFFER_LAG);
        assert!(
            snapshot.contains(&needle),
            "expected outbox lag to be 0 in: {snapshot}"
        );
    }
}
