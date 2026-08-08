//! Firehose endpoint: `com.atproto.sync.subscribeRepos`.
//!
//! Streams repo events (commit / identity / account / sync) to websocket
//! subscribers. The handler validates the cursor, backfills from the
//! sequencer DB if needed, then live-streams from the broadcast channel.

use crate::context::SharedSequencer;
use crate::db::types::db_id::DbId;
use crate::sequencer::apalis_worker::SharedBroadcast;
use crate::sequencer::events::{
    SeqEvt, TypedAccountEvt, TypedCommitEvt, TypedIdentityEvt, TypedSyncEvt,
};
use crate::sequencer::outbox::{Outbox, OutboxOpts};
use crate::sequencer::ws_frames::{
    ErrorFrame, ErrorFrameBody, Frame, InfoFrameBody, MessageFrame, MessageFrameOpts,
};
use chrono::offset::Utc as UtcOffset;
use chrono::{DateTime, Duration};
use futures::{SinkExt, StreamExt};
use poem::IntoResponse;
use poem::web::websocket::{Message, WebSocket};
use rsky_common::RFC3339_VARIANT;
use rsky_lexicon::com::atproto::sync::{
    SubscribeReposAccount, SubscribeReposCommit, SubscribeReposCommitOperation,
    SubscribeReposIdentity, SubscribeReposSync,
};
use std::time::SystemTime;
use tokio::time::{Duration as TokioDuration, interval};

fn get_backfill_limit(ms: u64) -> String {
    let system_time = SystemTime::now();
    let mut dt: DateTime<UtcOffset> = system_time.into();
    dt -= Duration::milliseconds(ms as i64);
    format!("{}", dt.format(RFC3339_VARIANT))
}

#[derive(serde::Deserialize, Default)]
pub struct SubscribeReposQuery {
    pub cursor: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct RepoBackfillConfig {
    pub repo_backfill_limit_ms: u64,
}

impl Default for RepoBackfillConfig {
    fn default() -> Self {
        Self {
            repo_backfill_limit_ms: 24 * 60 * 60 * 1000,
        }
    }
}

#[poem::handler]
pub async fn subscribe_repos(
    ws: WebSocket,
    shared: poem::web::Data<&SharedSequencer>,
    broadcast: poem::web::Data<&SharedBroadcast>,
    query: poem::web::Query<SubscribeReposQuery>,
) -> impl IntoResponse {
    let cursor = query.cursor;
    let config = RepoBackfillConfig::default();
    let backfill_time = get_backfill_limit(config.repo_backfill_limit_ms);

    let sequencer_lock = shared
        .sequencer
        .read()
        .expect("sequencer lock poisoned")
        .clone();
    let seq_broadcast = (*broadcast).clone();

    ws.on_upgrade(move |mut socket| async move {
        let mut outbox_cursor: Option<i64> = None;

        // Validate cursor.
        if let Some(cursor) = cursor {
            let curr = match sequencer_lock.curr().await {
                Ok(c) => c,
                Err(err) => {
                    let body = serde_json::json!({
                        "$type": "#error",
                        "name": "CurrError",
                        "message": err.to_string(),
                    });
                    let _ = socket.send(Message::Text(body.to_string())).await;
                    return;
                }
            };
            let curr_ts = curr.map(|s| s.0.timestamp_ms() as i64);
            if cursor > curr_ts.unwrap_or(0) {
                let err = ErrorFrame::new(ErrorFrameBody {
                    error: "FutureCursor".to_string(),
                    message: Some("Cursor in the future.".to_string()),
                });
                let bytes = err.to_bytes().unwrap_or_default();
                let _ = socket.send(Message::Binary(bytes)).await;
                return;
            }
            let next = sequencer_lock.next_seq(ulid_for_ts(cursor)).await;
            let next_ts = match next {
                Ok(Some(n)) => n.sequenced_at.unix_timestamp_nanos() as i64 / 1_000_000,
                _ => cursor,
            };
            let backfill_ms = parse_backfill_time(&backfill_time);
            if next_ts < backfill_ms {
                let info = MessageFrame::new(
                    InfoFrameBody {
                        name: "OutdatedCursor".to_string(),
                        message: Some(
                            "Requested cursor exceeded limit. Possibly missing events".to_string(),
                        ),
                    },
                    Some(MessageFrameOpts {
                        r#type: Some("#info".to_string()),
                    }),
                );
                let bytes = info.to_bytes().unwrap_or_default();
                let _ = socket.send(Message::Binary(bytes)).await;
                if let Ok(Some(earliest)) = sequencer_lock
                    .earliest_after_time(backfill_time_date(&backfill_time))
                    .await
                {
                    let ts = earliest.seq.0.timestamp_ms() as i64;
                    outbox_cursor = Some(ts.saturating_sub(1));
                }
            } else {
                outbox_cursor = Some(cursor);
            }
        }

        let mut outbox = Outbox::new(
            sequencer_lock.clone(),
            seq_broadcast.clone(),
            Some(OutboxOpts::default()),
        );
        let mut stream = outbox.events(outbox_cursor).await;
        let mut ping = interval(TokioDuration::from_secs(30));

        loop {
            tokio::select! {
                evt = stream.next() => {
                    match evt {
                        Some(Ok(evt)) => {
                            let bytes = match typed_event_to_frame_bytes(&evt) {
                                Ok(b) => b,
                                Err(err) => {
                                    let err = ErrorFrame::new(ErrorFrameBody {
                                        error: "SerializationError".to_string(),
                                        message: Some(err.to_string()),
                                    });
                                    let _ = socket.send(Message::Binary(err.to_bytes().unwrap_or_default())).await;
                                    return;
                                }
                            };
                            if socket.send(Message::Binary(bytes)).await.is_err() {
                                return;
                            }
                        }
                        Some(Err(err)) => {
                            let err = ErrorFrame::new(ErrorFrameBody {
                                error: "EventStreamError".to_string(),
                                message: Some(err.to_string()),
                            });
                            let _ = socket.send(Message::Binary(err.to_bytes().unwrap_or_default())).await;
                            return;
                        }
                        None => return,
                    }
                }
                incoming = socket.next() => {
                    match incoming {
                        Some(Ok(Message::Close(_))) => return,
                        Some(Ok(_)) => {}
                        Some(Err(_)) => return,
                        None => return,
                    }
                }
                _ = ping.tick() => {
                    if socket.send(Message::Ping(Vec::new())).await.is_err() {
                        return;
                    }
                }
            }
        }
    })
}

fn typed_event_to_frame_bytes(evt: &SeqEvt) -> anyhow::Result<Vec<u8>> {
    match evt {
        SeqEvt::TypedCommitEvt(commit) => {
            let TypedCommitEvt {
                r#type,
                seq,
                time,
                evt,
            } = commit.as_ref();
            let commit_evt = SubscribeReposCommit {
                seq: *seq,
                time: parse_time(time),
                rebase: evt.rebase,
                too_big: evt.too_big,
                repo: evt.repo.clone(),
                commit: evt.commit,
                prev: evt.prev,
                rev: evt.rev.clone(),
                since: evt.since.clone(),
                blocks: evt.blocks.clone(),
                ops: evt
                    .ops
                    .iter()
                    .map(|op| SubscribeReposCommitOperation {
                        path: op.path.clone(),
                        cid: op.cid,
                        prev: op.prev,
                        action: op.action.to_string(),
                    })
                    .collect(),
                blobs: evt.blobs.iter().map(|c| c.to_string()).collect(),
                prev_data: evt.prev_data,
            };
            let frame = MessageFrame::new(
                commit_evt,
                Some(MessageFrameOpts {
                    r#type: Some(format!("#{}", r#type)),
                }),
            );
            frame.to_bytes()
        }
        SeqEvt::TypedIdentityEvt(identity) => {
            let TypedIdentityEvt {
                r#type,
                seq,
                time,
                evt,
            } = identity;
            let id_evt = SubscribeReposIdentity {
                did: evt.did.clone(),
                seq: *seq,
                handle: evt.handle.clone(),
                time: parse_time(time),
            };
            let frame = MessageFrame::new(
                id_evt,
                Some(MessageFrameOpts {
                    r#type: Some(format!("#{}", r#type)),
                }),
            );
            frame.to_bytes()
        }
        SeqEvt::TypedAccountEvt(account) => {
            let TypedAccountEvt {
                r#type,
                seq,
                time,
                evt,
            } = account;
            let acc_evt = SubscribeReposAccount {
                seq: *seq,
                did: evt.did.clone(),
                status: evt.status.clone(),
                active: evt.active,
                time: parse_time(time),
            };
            let frame = MessageFrame::new(
                acc_evt,
                Some(MessageFrameOpts {
                    r#type: Some(format!("#{}", r#type)),
                }),
            );
            frame.to_bytes()
        }
        SeqEvt::TypedSyncEvt(sync) => {
            let TypedSyncEvt {
                r#type,
                seq,
                time,
                evt,
            } = sync;
            let sync_evt = SubscribeReposSync {
                seq: *seq,
                did: evt.did.clone(),
                blocks: evt.blocks.clone(),
                rev: evt.rev.clone(),
                time: parse_time(time),
            };
            let frame = MessageFrame::new(
                sync_evt,
                Some(MessageFrameOpts {
                    r#type: Some(format!("#{}", r#type)),
                }),
            );
            frame.to_bytes()
        }
    }
}

fn parse_time(time: &str) -> DateTime<UtcOffset> {
    DateTime::parse_from_rfc3339(time)
        .map(|dt| dt.with_timezone(&UtcOffset))
        .unwrap_or_else(|_| UtcOffset::now())
}

fn ulid_for_ts(ts_ms: i64) -> DbId {
    let timestamp_ms = ts_ms.max(0) as u64;
    DbId::from_bytes(ulid::Ulid::from_parts(timestamp_ms, 0u128).to_bytes())
}

fn parse_backfill_time(backfill_time: &str) -> i64 {
    DateTime::parse_from_rfc3339(backfill_time)
        .map(|dt| dt.timestamp_millis())
        .unwrap_or(0)
}

fn backfill_time_date(backfill_time: &str) -> time::OffsetDateTime {
    DateTime::parse_from_rfc3339(backfill_time)
        .map(|dt| {
            let dt_utc = dt.with_timezone(&UtcOffset);
            let nanos = dt_utc.timestamp_nanos_opt().unwrap_or(0);
            time::OffsetDateTime::from_unix_timestamp_nanos(nanos as i128)
                .unwrap_or(time::OffsetDateTime::UNIX_EPOCH)
        })
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscribe_repos_query_default_cursor_is_none() {
        let query: SubscribeReposQuery = serde_json::from_str("{}").unwrap();
        assert!(query.cursor.is_none());
    }

    #[test]
    fn future_cursor_is_rejected_with_error_frame() {
        let err = ErrorFrame::new(ErrorFrameBody {
            error: "FutureCursor".to_string(),
            message: Some("Cursor in the future.".to_string()),
        });
        let bytes = err.to_bytes().unwrap();
        let mut values =
            serde_cbor::Deserializer::from_slice(&bytes).into_iter::<serde_cbor::Value>();
        let header = values.next().unwrap().unwrap();
        let body = values.next().unwrap().unwrap();
        assert!(values.next().is_none());
        if let serde_cbor::Value::Map(map) = header {
            let op = map.get(&serde_cbor::Value::Text("op".to_string())).unwrap();
            assert_eq!(op, &serde_cbor::Value::Integer(-1));
        } else {
            panic!("expected map header");
        }
        if let serde_cbor::Value::Map(map) = body {
            assert_eq!(
                map.get(&serde_cbor::Value::Text("error".to_string())),
                Some(&serde_cbor::Value::Text("FutureCursor".to_string()))
            );
        } else {
            panic!("expected map body");
        }
    }

    #[test]
    fn info_frame_for_outdated_cursor_carries_correct_discriminator() {
        let frame = crate::sequencer::ws_frames::MessageFrame::new(
            crate::sequencer::ws_frames::InfoFrameBody {
                name: "OutdatedCursor".to_string(),
                message: Some("Requested cursor exceeded limit".to_string()),
            },
            Some(crate::sequencer::ws_frames::MessageFrameOpts {
                r#type: Some("#info".to_string()),
            }),
        );
        let bytes = frame.to_bytes().unwrap();
        let mut values =
            serde_cbor::Deserializer::from_slice(&bytes).into_iter::<serde_cbor::Value>();
        let header = values.next().unwrap().unwrap();
        let body = values.next().unwrap().unwrap();
        assert!(values.next().is_none());
        if let serde_cbor::Value::Map(map) = header {
            let op = map.get(&serde_cbor::Value::Text("op".to_string())).unwrap();
            assert_eq!(op, &serde_cbor::Value::Integer(1));
            let t = map.get(&serde_cbor::Value::Text("t".to_string())).unwrap();
            assert_eq!(t, &serde_cbor::Value::Text("#info".to_string()));
        } else {
            panic!("expected map header");
        }
        if let serde_cbor::Value::Map(map) = body {
            assert_eq!(
                map.get(&serde_cbor::Value::Text("name".to_string())),
                Some(&serde_cbor::Value::Text("OutdatedCursor".to_string()))
            );
        } else {
            panic!("expected map body");
        }
    }
}

// -----------------------------------------------------------------------------
// End-to-end TCP roundtrip tests
// -----------------------------------------------------------------------------
//
// These tests boot a real `poem::Server` bound to an ephemeral 127.0.0.1
// port and drive the `subscribe_repos` handler with a real
// `tokio_tungstenite::connect_async` client. They cover three flows:
//
// 1. **Backfill + live broadcast**: a `commit` row is sequenced before the
//    websocket connects; the WS client should receive it via the backfill
//    path. Then an `identity` event is sequenced after the connection and
//    should arrive via the live `tokio::sync::broadcast` path.
// 2. **Future cursor rejection**: a cursor strictly greater than every
//    `repo_seq.sequencedAt` is rejected with the on-wire `ErrorFrame`
//    (`op == -1`, `error == "FutureCursor"`).
//
// Each test relies on the apalis-style worker that the production
// sequencer already uses (`spawn_seq_event_worker`) to publish envelopes
// to the shared broadcast — no short-circuit bypass.
#[cfg(test)]
mod tcp_roundtrip_tests {
    use super::subscribe_repos;
    use crate::context::SharedSequencer;
    use crate::db::DatabaseKind;
    use crate::sequencer::Sequencer;
    use crate::sequencer::apalis_worker::{
        SharedBroadcast, connect_jobs_db, spawn_seq_event_worker,
    };
    use crate::sequencer::crawlers::Crawlers;
    use futures::StreamExt;
    use lexicon_cid::Cid;
    use rsky_repo::block_map::BlockMap;
    use rsky_repo::cid_set::CidSet;
    use rsky_repo::types::{CommitAction, CommitData, CommitDataWithOps, CommitOp};
    use std::str::FromStr;
    use std::time::Duration;

    const TEST_CID: &str = "bafkreibjfgx2gprinfvicegelk5kosd6y2frmqpqzwqkg7usac74l3t2v4";
    const TEST_REPO_DID: &str = "did:plc:rounrdtrip";
    const TEST_REPO_REV: &str = "3jzfcijpj2z2a";
    const TEST_IDENTITY_DID: &str = "did:plc:identitylive";
    const TEST_IDENTITY_HANDLE: &str = "alice.roundtrip";

    fn make_commit_data() -> CommitDataWithOps {
        let cid = Cid::from_str(TEST_CID).expect("valid test CID");
        let mut blocks = BlockMap::new();
        let _ = blocks.add(cid);
        CommitDataWithOps {
            commit_data: CommitData {
                cid,
                rev: TEST_REPO_REV.to_string(),
                since: None,
                prev: None,
                new_blocks: BlockMap::new(),
                relevant_blocks: blocks,
                removed_cids: CidSet::new(None),
            },
            ops: vec![CommitOp {
                action: CommitAction::Create,
                path: format!("app.bsky.feed.post/{TEST_REPO_REV}"),
                cid: Some(cid),
                prev: None,
            }],
            prev_data: None,
        }
    }

    /// Build a fresh sequencer DB + jobs DB + worker + broadcast + shared
    /// sequencer for one test. Returns the test-owned `SharedSequencer`
    /// (used to insert events) plus the broadcast (used to subscribe a
    /// sentinel) plus the worker's join handle (caller must abort on
    /// teardown).
    async fn setup_env(
        dir: &camino_tempfile::Utf8TempDir,
    ) -> (
        SharedSequencer,
        SharedBroadcast,
        tokio::task::JoinHandle<()>,
    ) {
        let seq_db_path = dir.path().join("sequencer.sqlite");
        let jobs_db_path = dir.path().join("jobs.sqlite");
        let seq_db = DatabaseKind::Sequencer
            .open(&seq_db_path)
            .await
            .expect("open sequencer DB");
        let pool = connect_jobs_db(jobs_db_path.as_str())
            .await
            .expect("connect jobs DB");
        let broadcast = SharedBroadcast::new(64);
        let sequencer = Sequencer::new(
            seq_db.clone(),
            Crawlers::new("pds.test".to_string(), vec![]),
            Some(pool.clone()),
            None,
        );
        let shared_seq = SharedSequencer::new(sequencer);
        let worker = spawn_seq_event_worker(pool, broadcast.clone());
        (shared_seq, broadcast, worker)
    }

    /// Bind a real TCP listener on an ephemeral 127.0.0.1 port, hand the
    /// resulting acceptor to `poem::Server`, and spawn the server task.
    async fn spawn_app<E>(app: E) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>)
    where
        E: poem::Endpoint<Output = poem::Response> + 'static,
    {
        use poem::listener::{Acceptor, Listener};
        let acceptor = poem::listener::TcpListener::bind("127.0.0.1:0")
            .into_acceptor()
            .await
            .expect("bind ephemeral TCP listener");
        let addr = acceptor
            .local_addr()
            .into_iter()
            .next()
            .expect("listener local addr")
            .as_socket_addr()
            .cloned()
            .expect("socket addr");
        let handle = tokio::spawn(async move {
            let _ = poem::Server::new_with_acceptor(acceptor).run(app).await;
        });
        // Tiny grace period so the kernel hands the listening socket to
        // poem before the client tries to connect.
        tokio::time::sleep(Duration::from_millis(25)).await;
        (addr, handle)
    }

    /// Read from the WS client stream until we get a binary data frame,
    /// transparently skipping control frames (Ping/Pong). This matches how
    /// a real firehose client behaves — `tungstenite` auto-responds to
    /// Pings with Pongs but our handler doesn't auto-pong, so the client
    /// receives Ping frames as part of the stream.
    async fn next_binary_frame<S>(stream: &mut S) -> Vec<u8>
    where
        S: futures::Stream<
                Item = std::result::Result<
                    tokio_tungstenite::tungstenite::Message,
                    tokio_tungstenite::tungstenite::Error,
                >,
            > + Unpin,
    {
        let deadline = Duration::from_secs(10);
        loop {
            let msg = tokio::time::timeout(deadline, stream.next())
                .await
                .unwrap_or_else(|_| panic!("timed out waiting for next ws binary frame"))
                .expect("ws stream closed unexpectedly")
                .expect("ws error");
            match msg {
                tokio_tungstenite::tungstenite::Message::Binary(b) => {
                    return b.to_vec();
                }
                tokio_tungstenite::tungstenite::Message::Ping(_) => continue,
                tokio_tungstenite::tungstenite::Message::Pong(_) => continue,
                tokio_tungstenite::tungstenite::Message::Close(frame) => {
                    panic!("ws closed before binary frame arrived: {frame:?}")
                }
                other => panic!("expected binary frame, got {other:?}"),
            }
        }
    }

    fn decode_two(bytes: &[u8]) -> (serde_cbor::Value, serde_cbor::Value) {
        let mut iter = serde_cbor::Deserializer::from_slice(bytes).into_iter::<serde_cbor::Value>();
        let header = iter.next().expect("missing header").expect("header CBOR");
        let body = iter.next().expect("missing body").expect("body CBOR");
        assert!(
            iter.next().is_none(),
            "frame must be exactly two CBOR values"
        );
        (header, body)
    }

    fn map_get<'a>(map: &'a serde_cbor::Value, key: &str) -> Option<&'a serde_cbor::Value> {
        let serde_cbor::Value::Map(entries) = map else {
            panic!("expected cbor map, got {map:?}");
        };
        entries
            .iter()
            .find(|(k, _)| matches!(k, serde_cbor::Value::Text(t) if t == key))
            .map(|(_, v)| v)
    }

    fn build_app(
        shared_seq: SharedSequencer,
        broadcast: SharedBroadcast,
    ) -> impl poem::Endpoint<Output = poem::Response> {
        use poem::EndpointExt;
        poem::Route::new()
            .at(
                "/xrpc/com.atproto.sync.subscribeRepos",
                poem::get(subscribe_repos),
            )
            .data(shared_seq)
            .data(broadcast)
    }

    #[tokio::test]
    async fn subscribe_repos_roundtrip_streams_commit_backfill_then_identity_live() {
        let dir = camino_tempfile::tempdir().expect("tempdir");
        let (shared_seq, broadcast, worker) = setup_env(&dir).await;

        let app = build_app(shared_seq.clone(), broadcast.clone());
        let (addr, server) = spawn_app(app).await;

        // Subscribe to the broadcast BEFORE inserting events so the test
        // acts as a sentinel: if the apalis worker is publishing, we will
        // see an envelope; if not, the WS would also miss the live
        // identity event.
        let mut bcast_rx = broadcast.tx.subscribe();

        // Insert a commit BEFORE the WS client connects so the handler
        // delivers it via the backfill path (DB read > cursor=0). We
        // take the lock briefly to clone the inner Sequencer (matching
        // the handler's read+clone pattern), then drop the guard before
        // awaiting so the std::sync::RwLock is not held across `.await`.
        let inserted_seq = {
            let mut sequencer_clone = shared_seq.sequencer.read().expect("sequencer lock").clone();
            sequencer_clone
                .sequence_commit(TEST_REPO_DID.to_string(), make_commit_data())
                .await
                .expect("sequence_commit")
        };
        assert!(
            inserted_seq.timestamp_ms() > 0,
            "DbId timestamp should be > 0"
        );

        // Connect via real WebSocket client.
        let url = format!("ws://{addr}/xrpc/com.atproto.sync.subscribeRepos?cursor=0");
        let (mut ws, _resp) = tokio_tungstenite::connect_async(&url)
            .await
            .expect("ws connect");

        // Frame #1: the backfilled #commit.
        let bytes = next_binary_frame(&mut ws).await;
        let (header, body) = decode_two(&bytes);
        assert_eq!(
            map_get(&header, "op"),
            Some(&serde_cbor::Value::Integer(1)),
            "header.op must be 1 (FrameType::Message)"
        );
        assert_eq!(
            map_get(&header, "t"),
            Some(&serde_cbor::Value::Text("#commit".to_string())),
            "header.t must be #commit"
        );
        assert_eq!(
            map_get(&body, "repo"),
            Some(&serde_cbor::Value::Text(TEST_REPO_DID.to_string())),
            "body.repo must equal the inserted DID"
        );
        assert!(
            map_get(&body, "commit").is_some(),
            "body must include a commit CID, got {body:?}"
        );
        assert_eq!(
            map_get(&body, "rev"),
            Some(&serde_cbor::Value::Text(TEST_REPO_REV.to_string())),
            "body.rev must equal the commit rev"
        );
        match map_get(&body, "ops") {
            Some(serde_cbor::Value::Array(ops)) => {
                assert!(
                    !ops.is_empty(),
                    "body.ops must include at least one operation"
                );
                let op = &ops[0];
                let serde_cbor::Value::Map(op_map) = op else {
                    panic!("expected ops[0] to be a map, got {op:?}");
                };
                let has_path = op_map
                    .iter()
                    .any(|(k, _)| matches!(k, serde_cbor::Value::Text(t) if t == "path"));
                let has_action = op_map
                    .iter()
                    .any(|(k, _)| matches!(k, serde_cbor::Value::Text(t) if t == "action"));
                assert!(has_path && has_action, "ops[0] must have path+action");
            }
            other => panic!("body.ops must be a CBOR array, got {other:?}"),
        }

        // The broadcast sentinel should have received the commit envelope
        // produced by the apalis worker; this verifies the producer/worker
        // are wired to the SAME jobs DB.
        let envelope = tokio::time::timeout(Duration::from_secs(5), bcast_rx.recv())
            .await
            .expect("broadcast envelope must arrive within 5s")
            .expect("broadcast channel closed");
        assert!(
            envelope.contains("\"commit\""),
            "expected commit envelope, got {envelope}"
        );

        // Now insert an identity event LIVE, AFTER the WS is connected and
        // (implicitly) subscribed to the broadcast — the handler should
        // pick it up via the live broadcast path. Same read+clone pattern
        // as the commit insert above to avoid holding the lock across
        // `.await`.
        {
            let mut sequencer_clone = shared_seq.sequencer.read().expect("sequencer lock").clone();
            sequencer_clone
                .sequence_identity_evt(
                    TEST_IDENTITY_DID.to_string(),
                    Some(TEST_IDENTITY_HANDLE.to_string()),
                )
                .await
                .expect("sequence_identity_evt");
        }

        // Frame #2: the live #identity.
        let bytes = next_binary_frame(&mut ws).await;
        let (header, body) = decode_two(&bytes);
        assert_eq!(
            map_get(&header, "op"),
            Some(&serde_cbor::Value::Integer(1)),
            "header.op must be 1 (FrameType::Message)"
        );
        assert_eq!(
            map_get(&header, "t"),
            Some(&serde_cbor::Value::Text("#identity".to_string())),
            "header.t must be #identity"
        );
        assert_eq!(
            map_get(&body, "did"),
            Some(&serde_cbor::Value::Text(TEST_IDENTITY_DID.to_string())),
            "body.did must equal the inserted DID"
        );
        assert_eq!(
            map_get(&body, "handle"),
            Some(&serde_cbor::Value::Text(TEST_IDENTITY_HANDLE.to_string())),
            "body.handle must equal the inserted handle"
        );

        let _ = ws.close(None).await;
        server.abort();
        worker.abort();
    }

    #[tokio::test]
    async fn subscribe_repos_roundtrip_rejects_future_cursor_with_error_frame() {
        let dir = camino_tempfile::tempdir().expect("tempdir");
        let (shared_seq, broadcast, worker) = setup_env(&dir).await;

        let app = build_app(shared_seq.clone(), broadcast.clone());
        let (addr, server) = spawn_app(app).await;

        // No events have been inserted; the empty sequencer DB reports
        // `curr() == None` so `curr_ts.unwrap_or(0) == 0`. cursor=99999
        // is strictly greater than 0, so the FutureCursor branch fires.
        let url = format!("ws://{addr}/xrpc/com.atproto.sync.subscribeRepos?cursor=99999");
        let (mut ws, _resp) = tokio_tungstenite::connect_async(&url)
            .await
            .expect("ws connect");

        let bytes = next_binary_frame(&mut ws).await;
        let (header, body) = decode_two(&bytes);
        assert_eq!(
            map_get(&header, "op"),
            Some(&serde_cbor::Value::Integer(-1)),
            "header.op must be -1 (FrameType::Error)"
        );
        assert!(
            map_get(&header, "t").is_none(),
            "header.t must be absent on ErrorFrame, got header={header:?}"
        );
        assert_eq!(
            map_get(&body, "error"),
            Some(&serde_cbor::Value::Text("FutureCursor".to_string())),
            "body.error must equal \"FutureCursor\""
        );

        let _ = ws.close(None).await;
        server.abort();
        worker.abort();
    }
}
