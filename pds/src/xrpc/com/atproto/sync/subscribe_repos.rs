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
        let mut values = serde_cbor::Deserializer::from_slice(&bytes).into_iter::<serde_cbor::Value>();
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
        let mut values = serde_cbor::Deserializer::from_slice(&bytes).into_iter::<serde_cbor::Value>();
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
