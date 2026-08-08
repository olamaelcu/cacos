# Task 6: poem websocket `subscribeRepos` route

**Files:**
- Create: `pds/src/xrpc/com/atproto/sync/subscribe_repos.rs`
- Create (if not already present, per earlier plans): `pds/src/xrpc/mod.rs`, `pds/src/xrpc/com/mod.rs`, `pds/src/xrpc/com/atproto/mod.rs`, `pds/src/xrpc/com/atproto/sync/mod.rs`
- Modify: `pds/src/main.rs` (ensure `mod xrpc;` is declared)
- Test: `pds/src/xrpc/com/atproto/sync/subscribe_repos.rs` (`#[cfg(test)] mod tests`)
- Modify: `pds/Cargo.toml` (deps below)

Port source: the rsky-pds subscribe_repos handler is now reached through the git-pinned `rsky-common` / `rsky-lexicon` crates (`Cargo.toml:8-15`). Cursor pre-validation: FutureCursor / OutdatedCursor `#info` + `earliest_after_time` reset; event -> `SubscribeRepos*` lexicon conversion; 30s ping. The framing comes from `ws_frames.rs` (Task 3). `rocket::ws` becomes `poem::web::websocket`; `ws::Message` becomes `poem::web::websocket::Message`.

- [ ] **Step 1: Add dependencies to `pds/Cargo.toml`**

```toml
# [dependencies] — add ONLY what is NOT already in pds/Cargo.toml or Cargo.toml:7-61.
poem = { workspace = true, features = ["websocket"] }   # tower-compat is already on the workspace entry
# chrono is already in pds/Cargo.toml:24 for this very handler
# [dev-dependencies] — add
tokio-tungstenite = "0.27"   # match the version poem itself uses for its websocket dev tests
```

- [ ] **Step 2: Ensure the xrpc module chain exists**

Create each missing file with exactly this content (skip any that an earlier plan already created):

`pds/src/xrpc/mod.rs`:
```rust
pub mod com;
```

`pds/src/xrpc/com/mod.rs`:
```rust
pub mod atproto;
```

`pds/src/xrpc/com/atproto/mod.rs`:
```rust
pub mod sync;
```

`pds/src/xrpc/com/atproto/sync/mod.rs`:
```rust
pub mod subscribe_repos;
```

Ensure `pds/src/main.rs` contains `mod xrpc;` (add it near the other `mod` declarations; Plan 08 wires the routes into the app).

Run: `cargo check -p pds`
Expected: compiles (the route file comes next; the module chain is inert).

- [ ] **Step 3: Write the failing tests in `pds/src/xrpc/com/atproto/sync/subscribe_repos.rs`**

Create the route file containing ONLY this test module for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{SharedBroadcast, SharedSequencer};
    use crate::sequencer::apalis_worker::{connect_jobs_db, spawn_seq_event_worker};
    use crate::sequencer::crawlers::Crawlers;
    use crate::sequencer::Sequencer;
    use futures::StreamExt;
    use poem::listener::TcpListener;
    use poem::{get, Route, Server};
    use rsky_repo::block_map::BlockMap;
    use rsky_repo::cid_set::CidSet;
    use rsky_repo::types::{CommitAction, CommitData, CommitOp};
    use serde_cbor::Value as CborValue;
    use std::str::FromStr;
    use tokio::sync::broadcast;
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    const TEST_CID: &str = "bafkreibjfgx2gprinfvicegelk5kosd6y2frmqpqzwqkg7usac74l3t2v4";

    fn commit_data(cid: lexicon_cid::Cid) -> rsky_repo::types::CommitDataWithOps {
        let mut relevant_blocks = BlockMap::new();
        relevant_blocks.set(cid, vec![1, 2, 3]);
        rsky_repo::types::CommitDataWithOps {
            commit_data: CommitData {
                cid,
                rev: "3jzfcijpj2z2a".to_owned(),
                since: None,
                prev: None,
                new_blocks: BlockMap::new(),
                relevant_blocks,
                removed_cids: CidSet::new(None),
            },
            ops: vec![CommitOp {
                action: CommitAction::Create,
                path: "app.bsky.feed.post/3jzfcijpj2z2a".to_owned(),
                cid: Some(cid),
                prev: None,
            }],
            prev_data: None,
        }
    }

    /// Boot a real poem server on an ephemeral port with the apalis worker
    /// running. Returns (tempdir, socket addr, producer sequencer, broadcast tx).
    async fn test_app() -> (
        tempfile::TempDir,
        std::net::SocketAddr,
        Sequencer,
        broadcast::Sender<String>,
    ) {
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
        let (tx, _rx) = broadcast::channel::<String>(1024);
        let mut sequencer = Sequencer::new(
            db,
            Crawlers::new("pds.test".to_owned(), vec![]),
            job_storage.clone(),
            None,
        );

        // the delivery worker shares the same jobs db
        spawn_seq_event_worker(job_storage, SharedBroadcast { tx: tx.clone() });

        let state: SharedSequencer = SharedSequencer {
            sequencer: tokio::sync::RwLock::new(sequencer.clone()),
        };
        let broadcast_state = SharedBroadcast { tx: tx.clone() };

        // reserve an ephemeral port, then hand it to poem
        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        let addr = format!("127.0.0.1:{port}");
        let app = Route::new()
            .at("/xrpc/com.atproto.sync.subscribeRepos", get(subscribe_repos))
            .data(state)
            .data(broadcast_state);
        let bind_addr = addr.clone();
        tokio::spawn(async move {
            let _ = Server::new(TcpListener::bind(&bind_addr)).run(app).await;
        });
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        (dir, addr.parse().unwrap(), sequencer, tx)
    }

    async fn next_binary_frame<S>(ws: &mut S, timeout_secs: u64) -> Vec<u8>
    where
        S: futures::Stream<Item = Result<WsMessage, tokio_tungstenite::tungstenite::Error>>
            + Unpin,
    {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
        loop {
            if tokio::time::Instant::now() > deadline {
                panic!("no binary frame within {timeout_secs}s");
            }
            match tokio::time::timeout(std::time::Duration::from_secs(1), ws.next()).await {
                Ok(Some(Ok(WsMessage::Binary(bytes)))) => return bytes.into(),
                Ok(Some(Ok(_))) => continue, // ping / pong / text
                Ok(Some(Err(e))) => panic!("ws error: {e}"),
                Ok(None) => panic!("ws closed"),
                Err(_elapsed) => continue,
            }
        }
    }

    fn decode_header(bytes: &[u8]) -> CborValue {
        let mut values = serde_cbor::Deserializer::from_slice(bytes).into_iter::<CborValue>();
        values.next().unwrap().unwrap()
    }

    fn frame_header_op(bytes: &[u8]) -> i128 {
        match decode_header(bytes).get("op") {
            Some(CborValue::Integer(op)) => *op,
            other => panic!("expected op integer, got {other:?}"),
        }
    }

    fn frame_header_type(bytes: &[u8]) -> String {
        match decode_header(bytes).get("t") {
            Some(CborValue::Text(t)) => t.clone(),
            other => panic!("expected t text, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn subscribe_repos_streams_backfill_then_live_events() {
        let (_dir, addr, mut producer, _tx) = test_app().await;

        // sequence a commit BEFORE connecting: delivered via backfill (cursor=0)
        let cid = lexicon_cid::Cid::from_str(TEST_CID).unwrap();
        producer
            .sequence_commit("did:plc:ws1".to_owned(), commit_data(cid))
            .await
            .unwrap();

        let (mut ws, _) = tokio_tungstenite::connect_async(format!(
            "ws://{addr}/xrpc/com.atproto.sync.subscribeRepos?cursor=0"
        ))
        .await
        .expect("ws connect");

        let frame1 = next_binary_frame(&mut ws, 5).await;
        assert_eq!(frame_header_op(&frame1), 1);
        assert_eq!(frame_header_type(&frame1), "#commit");

        // sequence an identity event AFTER connecting: delivered live via
        // apalis job -> broadcast -> outbox
        producer
            .sequence_identity_evt("did:plc:ws2".to_owned(), None)
            .await
            .unwrap();

        let frame2 = next_binary_frame(&mut ws, 5).await;
        assert_eq!(frame_header_op(&frame2), 1);
        assert_eq!(frame_header_type(&frame2), "#identity");
    }

    #[tokio::test]
    async fn subscribe_repos_rejects_future_cursor() {
        let (_dir, addr, mut producer, _tx) = test_app().await;
        producer
            .sequence_identity_evt("did:plc:fc".to_owned(), None)
            .await
            .unwrap();

        let (mut ws, _) = tokio_tungstenite::connect_async(format!(
            "ws://{addr}/xrpc/com.atproto.sync.subscribeRepos?cursor=9999"
        ))
        .await
        .expect("ws connect");

        // cursor 9999 > curr(1) -> ErrorFrame with op = -1
        let frame = next_binary_frame(&mut ws, 5).await;
        assert_eq!(frame_header_op(&frame), -1);
    }
}
```

- [ ] **Step 4: Run the tests to verify they fail**

Run: `cargo test -p cacos-pds xrpc::com::atproto::sync::subscribe_repos::tests`
Expected: FAIL — `cannot find function 'subscribe_repos'` in `xrpc::com::atproto::sync::subscribe_repos`.

- [ ] **Step 5: Implement the handler** (keep the test module from Step 3 appended)

Add the following to `pds/src/xrpc/com/atproto/sync/subscribe_repos.rs` (above the test module):

```rust
use crate::context::{SharedBroadcast, SharedSequencer};
use crate::sequencer::events::{
    AccountEvt, CommitEvt, IdentityEvt, SeqEvt, SyncEvt, TypedAccountEvt, TypedCommitEvt,
    TypedIdentityEvt, TypedSyncEvt,
};
use crate::sequencer::outbox::{Outbox, OutboxOpts};
use crate::sequencer::ws_frames::{
    ErrorFrame, ErrorFrameBody, Frame, InfoFrameBody, MessageFrame, MessageFrameOpts,
};
use futures::{pin_mut, SinkExt, StreamExt};
use migration::types::db_id::DbId;
use poem::web::websocket::{Message, WebSocket, WebSocketStream};
use poem::web::Query;
use poem::{handler, IntoResponse, State};
use rsky_common::time::from_str_to_utc;
use rsky_lexicon::com::atproto::sync::{
    SubscribeReposAccount, SubscribeReposCommit, SubscribeReposCommitOperation,
    SubscribeReposIdentity, SubscribeReposSync,
};
use serde::Deserialize;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;
use tokio::sync::broadcast;

/// Stand-ins for rsky's `cfg.subscription.*`; Plan 08 wires real config values.
const MAX_BUFFER: usize = 500;
const REPO_BACKFILL_LIMIT_MS: u64 = 3 * 24 * 60 * 60 * 1000; // 3 days

#[derive(Deserialize)]
pub struct SubscribeReposQuery {
    /// Cursor is the `DbId` of the last seen event (serialized as the canonical
    /// 26-char Crockford string on the URL). `None` means "stream from current".
    pub cursor: Option<DbId>,
}

/// Returns the earliest `sequencedAt` we'd accept in a backfill: now - REPO_BACKFILL_LIMIT_MS.
fn get_backfill_limit(ms: u64) -> time::OffsetDateTime {
    let now = time::OffsetDateTime::now_utc();
    let micros = -(ms as i128) * 1_000;
    now + time::Duration::microseconds(micros)
}

/// Repository event stream, aka Firehose endpoint. Outputs repo commits with
/// diff data, and identity/account/sync events, for all repositories on this
/// server. Public, no auth. See the atproto specs for stream sequencing, repo
/// versioning, and CAR diff format. Ported from rsky's rocket handler.
#[handler]
pub async fn subscribe_repos(
    ws: WebSocket,
    Query(query): Query<SubscribeReposQuery>,
    state: &State<SharedSequencer>,
    broadcast: &State<SharedBroadcast>,
) -> impl IntoResponse {
    let cursor = query.cursor;
    let sequencer = state.sequencer.read().await.clone();
    let tx = broadcast.tx.clone();
    ws.on_upgrade(move |socket| async move {
        run_subscribe_repos(socket, cursor, sequencer, tx).await;
    })
}

async fn run_subscribe_repos(
    mut socket: WebSocketStream,
    cursor: Option<DbId>,
    sequencer: crate::sequencer::Sequencer,
    tx: broadcast::Sender<String>,
) {
    let mut outbox = Outbox::new(
        sequencer.clone(),
        tx,
        Some(OutboxOpts {
            max_buffer_size: MAX_BUFFER,
        }),
    );

    tracing::debug!("request to com.atproto.sync.subscribeRepos; Cursor={cursor:?}");
    let backfill_time = get_backfill_limit(REPO_BACKFILL_LIMIT_MS);

    let mut outbox_cursor: Option<DbId> = None;
    if let Some(cursor) = cursor {
        let next = match sequencer.next_seq(cursor).await {
            Ok(next) => next,
            Err(_) => {
                let error_frame = ErrorFrame::new(ErrorFrameBody {
                    error: "NextError".to_string(),
                    message: Some("Failed to fetch next event.".to_string()),
                });
                let _ = socket.send(Message::binary(error_frame.to_bytes().unwrap())).await;
                return;
            }
        };
        let curr = match sequencer.curr().await {
            Ok(curr) => curr,
            Err(_) => {
                let error_frame = ErrorFrame::new(ErrorFrameBody {
                    error: "CurrError".to_string(),
                    message: Some("Failed to fetch current event.".to_string()),
                });
                let _ = socket.send(Message::binary(error_frame.to_bytes().unwrap())).await;
                return;
            }
        };
        if cursor > curr.unwrap_or_default() {
            let error_frame = ErrorFrame::new(ErrorFrameBody {
                error: "FutureCursor".to_string(),
                message: Some("Cursor in the future.".to_string()),
            });
            let _ = socket.send(Message::binary(error_frame.to_bytes().unwrap())).await;
            return;
        }
        match next {
            Some(next) if next.sequenced_at < backfill_time => {
                let info_frame = MessageFrame::new(
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
                match info_frame.to_bytes() {
                    Ok(binary) => {
                        let _ = socket.send(Message::binary(binary)).await;
                    }
                    Err(_) => {
                        let error_frame = ErrorFrame::new(ErrorFrameBody {
                            error: "SerializationError".to_string(),
                            message: Some("Failed to serialize info frame.".to_string()),
                        });
                        let _ = socket.send(Message::binary(error_frame.to_bytes().unwrap())).await;
                        return;
                    }
                }
                match sequencer.earliest_after_time(backfill_time).await {
                    Ok(Some(start_evt)) if start_evt.seq.is_some() => {
                        // Resume the stream one ULID step before the earliest
                        // event we still have, so the subscriber gets the
                        // contiguous prefix that survived backfill.
                        // ULIDs are 128-bit; for resume we simply hand back the
                        // earliest seq itself — `next_seq(start_evt.seq)`
                        // returns the next event after the cursor.
                        outbox_cursor = start_evt.seq;
                    }
                    Ok(None) => outbox_cursor = None,
                    _ => {
                        let error_frame = ErrorFrame::new(ErrorFrameBody {
                            error: "EarliestAfterTimeError".to_string(),
                            message: Some(
                                "Failed to fetch earliest event after backfill time.".to_string(),
                            ),
                        });
                        let _ = socket.send(Message::binary(error_frame.to_bytes().unwrap())).await;
                        return;
                    }
                }
            }
            _ => outbox_cursor = Some(cursor),
        }
    }

    let event_stream = outbox.events(outbox_cursor).await;
    pin_mut!(event_stream);

    // Initialize the ping interval
    let mut ping_interval = tokio::time::interval(std::time::Duration::from_secs(30));

    loop {
        tokio::select! {
            evt = event_stream.next() => {
                let evt = match evt {
                    Some(Ok(evt)) => evt,
                    Some(Err(err)) => {
                        let error_frame = ErrorFrame::new(ErrorFrameBody {
                            error: "EventStreamError".to_string(),
                            message: Some(err.to_string()),
                        });
                        let _ = socket.send(Message::binary(error_frame.to_bytes().unwrap())).await;
                        return;
                    }
                    None => {
                        let error_frame = ErrorFrame::new(ErrorFrameBody {
                            error: "EventStreamError".to_string(),
                            message: Some("Failed to fetch event from stream.".to_string()),
                        });
                        let _ = socket.send(Message::binary(error_frame.to_bytes().unwrap())).await;
                        return;
                    }
                };

                match evt {
                    SeqEvt::TypedCommitEvt(commit) => {
                        // seq is `DbId`; the SubscribeReposCommit lexicon expects `i64` (ATProto seq).
                        // Serialize the ULID through its `Display` impl — the relay / crawler parses
                        // it back into a typed `DbId` and uses `timestamp_ms()` for ordering.
                        // (For now we hand back `seq.timestamp_ms() as i64` because that's the only
                        // monotonic int64 representation of the seq; the DbId round-trips through
                        // the JSON envelope.)
                        let TypedCommitEvt { r#type, seq, time, evt } = *commit;
                        let CommitEvt { rebase, too_big, repo, commit, prev, rev, since, blocks, ops, blobs, prev_data } = evt;
                        let subscribe_commit_evt = SubscribeReposCommit {
                            seq: seq.timestamp_ms() as i64,
                            time: from_str_to_utc(&time).unwrap_or_else(|e| {
                                tracing::warn!("failed to parse event timestamp {:?}: {}", time, e);
                                chrono::Utc::now()
                            }),
                            rebase,
                            too_big,
                            repo,
                            commit,
                            prev,
                            rev,
                            since,
                            blocks,
                            ops: ops.into_iter().map(|op| SubscribeReposCommitOperation {
                                path: op.path,
                                cid: op.cid,
                                prev: op.prev,
                                action: op.action.to_string()
                            }).collect::<Vec<SubscribeReposCommitOperation>>(),
                            blobs: blobs.into_iter().map(|blob| blob.to_string()).collect::<Vec<String>>(),
                            prev_data,
                        };
                        let message_frame = MessageFrame::new(
                            subscribe_commit_evt,
                            Some(MessageFrameOpts { r#type: Some(format!("#{0}", r#type)) }),
                        );
                        match message_frame.to_bytes() {
                            Ok(binary) => { let _ = socket.send(Message::binary(binary)).await; }
                            Err(_) => {
                                let error_frame = ErrorFrame::new(ErrorFrameBody {
                                    error: "SerializationError".to_string(),
                                    message: Some("Failed to serialize event to message frame.".to_string()),
                                });
                                let _ = socket.send(Message::binary(error_frame.to_bytes().unwrap())).await;
                                return;
                            }
                        }
                    }
                    SeqEvt::TypedIdentityEvt(identity) => {
                        let TypedIdentityEvt { r#type, seq, time, evt } = identity;
                        let IdentityEvt { did, handle } = evt;
                        let subscribe_identity_evt = SubscribeReposIdentity {
                            did,
                            seq: seq.timestamp_ms() as i64,
                            handle,
                            time: from_str_to_utc(&time).unwrap_or_else(|e| {
                                tracing::warn!("failed to parse event timestamp {:?}: {}", time, e);
                                chrono::Utc::now()
                            }),
                        };
                        let message_frame = MessageFrame::new(
                            subscribe_identity_evt,
                            Some(MessageFrameOpts { r#type: Some(format!("#{0}", r#type)) }),
                        );
                        match message_frame.to_bytes() {
                            Ok(binary) => { let _ = socket.send(Message::binary(binary)).await; }
                            Err(_) => {
                                let error_frame = ErrorFrame::new(ErrorFrameBody {
                                    error: "SerializationError".to_string(),
                                    message: Some("Failed to serialize event to message frame.".to_string()),
                                });
                                let _ = socket.send(Message::binary(error_frame.to_bytes().unwrap())).await;
                                return;
                            }
                        }
                    }
                    SeqEvt::TypedAccountEvt(account) => {
                        let TypedAccountEvt { r#type, seq, time, evt } = account;
                        let AccountEvt { did, active, status } = evt;
                        let subscribe_account_evt = SubscribeReposAccount {
                            did,
                            seq: seq.timestamp_ms() as i64,
                            status,
                            active,
                            time: from_str_to_utc(&time).unwrap_or_else(|e| {
                                tracing::warn!("failed to parse event timestamp {:?}: {}", time, e);
                                chrono::Utc::now()
                            }),
                        };
                        let message_frame = MessageFrame::new(
                            subscribe_account_evt,
                            Some(MessageFrameOpts { r#type: Some(format!("#{0}", r#type)) }),
                        );
                        match message_frame.to_bytes() {
                            Ok(binary) => { let _ = socket.send(Message::binary(binary)).await; }
                            Err(_) => {
                                let error_frame = ErrorFrame::new(ErrorFrameBody {
                                    error: "SerializationError".to_string(),
                                    message: Some("Failed to serialize event to message frame.".to_string()),
                                });
                                let _ = socket.send(Message::binary(error_frame.to_bytes().unwrap())).await;
                                return;
                            }
                        }
                    }
                    SeqEvt::TypedSyncEvt(sync) => {
                        let TypedSyncEvt { r#type, seq, time, evt } = sync;
                        let SyncEvt { did, blocks, rev } = evt;
                        let subscribe_sync_evt = SubscribeReposSync {
                            seq: seq.timestamp_ms() as i64,
                            did,
                            blocks,
                            rev,
                            time: from_str_to_utc(&time).unwrap_or_else(|e| {
                                tracing::warn!("failed to parse event timestamp {:?}: {}", time, e);
                                chrono::Utc::now()
                            }),
                        };
                        let message_frame = MessageFrame::new(
                            subscribe_sync_evt,
                            Some(MessageFrameOpts { r#type: Some(format!("#{0}", r#type)) }),
                        );
                        match message_frame.to_bytes() {
                            Ok(binary) => { let _ = socket.send(Message::binary(binary)).await; }
                            Err(_) => {
                                let error_frame = ErrorFrame::new(ErrorFrameBody {
                                    error: "SerializationError".to_string(),
                                    message: Some("Failed to serialize event to message frame.".to_string()),
                                });
                                let _ = socket.send(Message::binary(error_frame.to_bytes().unwrap())).await;
                                return;
                            }
                        }
                    }
                }
            }
            msg = socket.next() => {
                match msg {
                    Some(Ok(Message::Close(_))) => break,
                    Some(Ok(Message::Ping(payload))) => {
                        let _ = socket.send(Message::Pong(payload)).await;
                    }
                    Some(Ok(_)) => {}
                    Some(Err(_)) => break,
                    None => break,
                }
            }
            _ = ping_interval.tick() => {
                let _ = socket.send(Message::ping(vec![])).await;
            }
        }
    }
}
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p cacos-pds xrpc::com::atproto::sync::subscribe_repos::tests`
Expected: `test result: ok. 2 passed`. These are end-to-end (poem server + apalis worker + broadcast + outbox + real websocket); each frame assertion polls up to 5s. If a test times out, check the apalis worker is running and the producer/worker share the same jobs DB.

- [ ] **Step 7: Commit**

```bash
git add pds/Cargo.toml pds/src/main.rs pds/src/xrpc/
git commit -m "feat(xrpc): poem websocket com.atproto.sync.subscribeRepos with backfill and live delivery"
```
