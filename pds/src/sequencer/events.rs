//! Sequencer event types: envelope shapes (`Typed*Evt`) used both for the
//! raw rows persisted in `repo_seq` and for the JSON envelopes broadcast
//! to subscribe-repos clients.
//!
//! This is the typed port of `rsky-pds/src/sequencer/events.rs`. Block map
//! handling (`format_seq_commit`, `sync_evt_data_from_commit`) intentionally
//! delegates to the git-pinned `rsky-repo` crate so the on-wire and on-disk
//! formats stay byte-compatible with the reference implementation.

use crate::db::entities::repo_seq;
use crate::db::types;
use anyhow::Result;
use lexicon_cid::Cid;
use rsky_repo::block_map::BlockMap;
use rsky_repo::car::blocks_to_car_file;
use rsky_repo::types::{CommitAction, CommitDataWithOps};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

pub use rsky_common::struct_to_cbor;

/// Format an `OffsetDateTime` as an RFC-3339 timestamp with millisecond
/// precision and a trailing `Z`, matching the on-wire format used by the
/// firehose (and the on-disk `repo_seq.sequenced_at` column).
pub fn format_offset_datetime(dt: OffsetDateTime) -> String {
    dt.format(time::macros::format_description!(
        "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z"
    ))
    .unwrap_or_else(|_| dt.to_string())
}

/// Wall-clock now, typed as `OffsetDateTime`.
pub fn now_offset() -> OffsetDateTime {
    OffsetDateTime::now_utc()
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CommitEvtOpAction {
    Create,
    Update,
    Delete,
}

impl std::fmt::Display for CommitEvtOpAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CommitEvtOpAction::Create => write!(f, "create"),
            CommitEvtOpAction::Update => write!(f, "update"),
            CommitEvtOpAction::Delete => write!(f, "delete"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct CommitEvtOp {
    pub action: CommitEvtOpAction,
    pub path: String,
    pub cid: Option<Cid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev: Option<Cid>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct CommitEvt {
    pub rebase: bool,
    #[serde(rename = "tooBig")]
    pub too_big: bool,
    pub repo: String,
    pub commit: Cid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev: Option<Cid>,
    pub rev: String,
    pub since: Option<String>,
    #[serde(with = "serde_bytes")]
    pub blocks: Vec<u8>,
    pub ops: Vec<CommitEvtOp>,
    pub blobs: Vec<Cid>,
    #[serde(rename = "prevData", default, skip_serializing_if = "Option::is_none")]
    pub prev_data: Option<Cid>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct HandleEvt {
    pub did: String,
    pub handle: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct IdentityEvt {
    pub did: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct AccountEvt {
    pub did: String,
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<RskyLexiconAccountStatus>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct SyncEvt {
    pub did: String,
    #[serde(with = "serde_bytes")]
    pub blocks: Vec<u8>,
    pub rev: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct TypedSyncEvt {
    pub r#type: String,
    pub seq: i64,
    pub time: String,
    pub evt: SyncEvt,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct TypedCommitEvt {
    pub r#type: String,
    pub seq: i64,
    pub time: String,
    pub evt: CommitEvt,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct TypedHandleEvt {
    pub r#type: String,
    pub seq: i64,
    pub time: String,
    pub evt: HandleEvt,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct TypedIdentityEvt {
    pub r#type: String,
    pub seq: i64,
    pub time: String,
    pub evt: IdentityEvt,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct TypedAccountEvt {
    pub r#type: String,
    pub seq: i64,
    pub time: String,
    pub evt: AccountEvt,
}

use rsky_lexicon::com::atproto::sync::AccountStatus as RskyLexiconAccountStatus;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum SeqEvt {
    TypedCommitEvt(Box<TypedCommitEvt>),
    TypedIdentityEvt(TypedIdentityEvt),
    TypedAccountEvt(TypedAccountEvt),
    TypedSyncEvt(TypedSyncEvt),
}

impl<'de> Deserialize<'de> for SeqEvt {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        let typ = value
            .get("type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| serde::de::Error::missing_field("type"))?;
        match typ {
            "commit" => serde_json::from_value::<TypedCommitEvt>(value)
                .map(Box::new)
                .map(SeqEvt::TypedCommitEvt)
                .map_err(serde::de::Error::custom),
            "identity" => serde_json::from_value::<TypedIdentityEvt>(value)
                .map(SeqEvt::TypedIdentityEvt)
                .map_err(serde::de::Error::custom),
            "account" => serde_json::from_value::<TypedAccountEvt>(value)
                .map(SeqEvt::TypedAccountEvt)
                .map_err(serde::de::Error::custom),
            "sync" => serde_json::from_value::<TypedSyncEvt>(value)
                .map(SeqEvt::TypedSyncEvt)
                .map_err(serde::de::Error::custom),
            _ => Err(serde::de::Error::custom("Unknown event type")),
        }
    }
}

impl SeqEvt {
    pub fn seq(&self) -> i64 {
        match self {
            SeqEvt::TypedCommitEvt(e) => e.seq,
            SeqEvt::TypedIdentityEvt(e) => e.seq,
            SeqEvt::TypedAccountEvt(e) => e.seq,
            SeqEvt::TypedSyncEvt(e) => e.seq,
        }
    }
}

/// Encapsulate a typed event into the `(seq, typed)` byte buffer that the
/// sequencer persists in `repo_seq.event`. The returned DB row is ready for
/// INSERT — the `seq` is filled by `sequence_evt`.
pub fn seq_evt_to_envelope(evt: SeqEvt) -> Result<repo_seq::ActiveModel> {
    let (did, event_type, body) = match evt {
        SeqEvt::TypedCommitEvt(e) => {
            let did = e.evt.repo.clone();
            let body = serde_ipld_dagcbor::to_vec(&e.evt)?;
            (did, "append".to_string(), body)
        }
        SeqEvt::TypedIdentityEvt(e) => {
            let did = e.evt.did.clone();
            let body = serde_ipld_dagcbor::to_vec(&e.evt)?;
            (did, "identity".to_string(), body)
        }
        SeqEvt::TypedAccountEvt(e) => {
            let did = e.evt.did.clone();
            let body = serde_ipld_dagcbor::to_vec(&e.evt)?;
            (did, "account".to_string(), body)
        }
        SeqEvt::TypedSyncEvt(e) => {
            let did = e.evt.did.clone();
            let body = serde_ipld_dagcbor::to_vec(&e.evt)?;
            (did, "sync".to_string(), body)
        }
    };
    let now = now_offset();
    Ok(repo_seq::ActiveModel {
        seq: sea_orm::NotSet,
        did: sea_orm::ActiveValue::Set(types::did::Did::from(did)),
        event_type: sea_orm::ActiveValue::Set(event_type),
        event: sea_orm::ActiveValue::Set(body),
        invalidated: sea_orm::ActiveValue::Set(Some(0)),
        sequenced_at: sea_orm::ActiveValue::Set(now),
    })
}

/// Encode a `CommitEvt` (the DAG-CBOR body) into a `repo_seq` insert row.
pub async fn format_seq_commit(did: String, commit_data: CommitDataWithOps) -> Result<RepoSeqNew> {
    let mut blocks_to_send = BlockMap::new();
    blocks_to_send.add_map(commit_data.commit_data.new_blocks.clone())?;
    blocks_to_send.add_map(commit_data.commit_data.relevant_blocks.clone())?;
    let ops = commit_data
        .ops
        .iter()
        .map(|op| {
            let action = match op.action {
                CommitAction::Create => CommitEvtOpAction::Create,
                CommitAction::Update => CommitEvtOpAction::Update,
                CommitAction::Delete => CommitEvtOpAction::Delete,
            };
            CommitEvtOp {
                action,
                path: op.path.clone(),
                cid: op.cid,
                prev: op.prev,
            }
        })
        .collect::<Vec<_>>();
    let car_slice = blocks_to_car_file(Some(&commit_data.commit_data.cid), blocks_to_send).await?;

    let evt = CommitEvt {
        rebase: false,
        too_big: false,
        repo: did.clone(),
        commit: commit_data.commit_data.cid,
        prev: None,
        rev: commit_data.commit_data.rev,
        since: commit_data.commit_data.since,
        blocks: car_slice,
        ops,
        blobs: vec![],
        prev_data: commit_data.prev_data,
    };

    Ok(RepoSeqNew::new(
        did,
        "append".to_string(),
        serde_ipld_dagcbor::to_vec(&evt)?,
        now_offset(),
    ))
}

pub fn format_seq_handle_update(did: String, handle: String) -> Result<RepoSeqNew> {
    let evt = HandleEvt {
        did: did.clone(),
        handle,
    };
    Ok(RepoSeqNew::new(
        did,
        "handle".to_string(),
        serde_ipld_dagcbor::to_vec(&evt)?,
        now_offset(),
    ))
}

pub fn format_seq_identity_evt(did: String, handle: Option<String>) -> Result<RepoSeqNew> {
    let evt = IdentityEvt {
        did: did.clone(),
        handle,
    };
    Ok(RepoSeqNew::new(
        did,
        "identity".to_string(),
        serde_ipld_dagcbor::to_vec(&evt)?,
        now_offset(),
    ))
}

pub fn format_seq_account_evt(
    did: String,
    active: bool,
    status: Option<RskyLexiconAccountStatus>,
) -> Result<RepoSeqNew> {
    let evt = AccountEvt {
        did: did.clone(),
        active,
        status,
    };
    Ok(RepoSeqNew::new(
        did,
        "account".to_string(),
        serde_ipld_dagcbor::to_vec(&evt)?,
        now_offset(),
    ))
}

pub async fn format_seq_sync_evt(did: String, rev: String, blocks: BlockMap) -> Result<RepoSeqNew> {
    let car_slice = blocks_to_car_file(None, blocks).await?;
    let evt = SyncEvt {
        did: did.clone(),
        rev,
        blocks: car_slice,
    };
    Ok(RepoSeqNew::new(
        did,
        "sync".to_string(),
        serde_ipld_dagcbor::to_vec(&evt)?,
        now_offset(),
    ))
}

/// Build a `SyncEvtData` from a `CommitDataWithOps`. Returns an error if the
/// commit block is missing from the relevant blocks map.
pub fn sync_evt_data_from_commit(
    commit_data: CommitDataWithOps,
) -> Result<crate::actor_store::repo::types::SyncEvtData> {
    let mut blocks = commit_data.commit_data.relevant_blocks.clone();
    let cid: Vec<Cid> = vec![commit_data.commit_data.cid];
    let blocks_and_missing = blocks.get_many(cid)?;
    if !blocks_and_missing.missing.is_empty() {
        anyhow::bail!("commit block was not found, could not build sync event");
    }
    Ok(crate::actor_store::repo::types::SyncEvtData {
        cid: commit_data.commit_data.cid,
        rev: commit_data.commit_data.rev,
        blocks: blocks_and_missing.blocks,
    })
}

/// Construct a typed `SeqEvt` from a freshly persisted `repo_seq` row.
/// The inverse of `seq_evt_to_envelope`.
pub fn envelope_from_repo_row(row: &repo_seq::Model) -> Result<SeqEvt> {
    let seq = row.seq.0.timestamp_ms() as i64;
    let time = row
        .sequenced_at
        .format(time::macros::format_description!(
            "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z"
        ))
        .unwrap_or_else(|_| row.sequenced_at.to_string());
    let typed = match row.event_type.as_str() {
        "append" | "rebase" => {
            let evt: CommitEvt = serde_ipld_dagcbor::from_slice(&row.event)?;
            SeqEvt::TypedCommitEvt(Box::new(TypedCommitEvt {
                r#type: "commit".to_string(),
                seq,
                time,
                evt,
            }))
        }
        "sync" => {
            let evt: SyncEvt = serde_ipld_dagcbor::from_slice(&row.event)?;
            SeqEvt::TypedSyncEvt(TypedSyncEvt {
                r#type: "sync".to_string(),
                seq,
                time,
                evt,
            })
        }
        "identity" => {
            let evt: IdentityEvt = serde_ipld_dagcbor::from_slice(&row.event)?;
            SeqEvt::TypedIdentityEvt(TypedIdentityEvt {
                r#type: "identity".to_string(),
                seq,
                time,
                evt,
            })
        }
        "account" => {
            let evt: AccountEvt = serde_ipld_dagcbor::from_slice(&row.event)?;
            SeqEvt::TypedAccountEvt(TypedAccountEvt {
                r#type: "account".to_string(),
                seq,
                time,
                evt,
            })
        }
        _ => anyhow::bail!("invalid event type: {}", row.event_type),
    };
    Ok(typed)
}

/// Newtype wrapper around the parameters needed to build a `repo_seq` row,
/// used so `format_seq_*` helpers can construct values without coupling
/// directly to the sea-orm ActiveModel.
#[derive(Debug, Clone, PartialEq)]
pub struct RepoSeqNew {
    pub did: types::did::Did,
    pub event_type: String,
    pub event: Vec<u8>,
    pub sequenced_at: OffsetDateTime,
}

impl RepoSeqNew {
    pub fn new(
        did: impl Into<types::did::Did>,
        event_type: impl Into<String>,
        event: Vec<u8>,
        sequenced_at: OffsetDateTime,
    ) -> Self {
        Self {
            did: did.into(),
            event_type: event_type.into(),
            event,
            sequenced_at,
        }
    }
}

impl From<RepoSeqNew> for repo_seq::ActiveModel {
    fn from(value: RepoSeqNew) -> Self {
        repo_seq::ActiveModel {
            seq: sea_orm::NotSet,
            did: sea_orm::ActiveValue::Set(value.did),
            event_type: sea_orm::ActiveValue::Set(value.event_type),
            event: sea_orm::ActiveValue::Set(value.event),
            invalidated: sea_orm::ActiveValue::Set(Some(0)),
            sequenced_at: sea_orm::ActiveValue::Set(value.sequenced_at),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ipld_core::ipld::Ipld;
    use rsky_repo::types::CommitData;
    use std::str::FromStr;

    const TEST_CID: &str = "bafkreibjfgx2gprinfvicegelk5kosd6y2frmqpqzwqkg7usac74l3t2v4";

    fn make_commit_data() -> CommitDataWithOps {
        let cid = Cid::from_str(TEST_CID).unwrap();
        let mut blocks = BlockMap::new();
        let _ = blocks.add(cid);
        CommitDataWithOps {
            commit_data: CommitData {
                cid,
                rev: "3jzfcijpj2z2a".to_string(),
                since: None,
                prev: None,
                new_blocks: BlockMap::new(),
                relevant_blocks: blocks,
                removed_cids: rsky_repo::cid_set::CidSet::new(None),
            },
            ops: vec![rsky_repo::types::CommitOp {
                action: CommitAction::Create,
                path: "app.bsky.feed.post/3jzfcijpj2z2a".to_string(),
                cid: Some(cid),
                prev: None,
            }],
            prev_data: None,
        }
    }

    #[test]
    fn commit_event_round_trips_through_dag_cbor() {
        let evt = CommitEvt {
            rebase: false,
            too_big: false,
            repo: "did:plc:test".to_string(),
            commit: Cid::from_str(TEST_CID).unwrap(),
            prev: None,
            rev: "3jzfcijpj2z2a".to_string(),
            since: None,
            blocks: vec![1, 2, 3],
            ops: vec![CommitEvtOp {
                action: CommitEvtOpAction::Create,
                path: "app.bsky.feed.post/3jzfcijpj2z2a".to_string(),
                cid: Some(Cid::from_str(TEST_CID).unwrap()),
                prev: None,
            }],
            blobs: vec![],
            prev_data: None,
        };
        let bytes = serde_ipld_dagcbor::to_vec(&evt).unwrap();
        let decoded: CommitEvt = serde_ipld_dagcbor::from_slice(&bytes).unwrap();
        assert_eq!(decoded.ops.len(), 1);
        assert_eq!(decoded.ops[0].action, CommitEvtOpAction::Create);
        // "tooBig" snake-case in Rust -> camelCase in CBOR
        let ipld: Ipld = serde_ipld_dagcbor::from_slice(&bytes).unwrap();
        let Ipld::Map(map) = ipld else {
            panic!("expected map")
        };
        assert!(map.contains_key("tooBig"));
    }

    #[test]
    fn sync_event_serializes_blocks_as_cbor_bytes() {
        let evt = SyncEvt {
            did: "did:plc:test".to_string(),
            blocks: vec![1, 2, 3],
            rev: "3jzfcijpj2z2a".to_string(),
        };
        let bytes = serde_ipld_dagcbor::to_vec(&evt).unwrap();
        let decoded: Ipld = serde_ipld_dagcbor::from_slice(&bytes).unwrap();
        let Ipld::Map(map) = decoded else {
            panic!("expected map")
        };
        match map.get("blocks") {
            Some(Ipld::Bytes(b)) => assert_eq!(b, &vec![1u8, 2, 3]),
            other => panic!("expected bytes, got {other:?}"),
        }
    }

    #[test]
    fn identity_event_omits_absent_handle() {
        let evt = IdentityEvt {
            did: "did:plc:test".to_string(),
            handle: None,
        };
        let bytes = serde_ipld_dagcbor::to_vec(&evt).unwrap();
        let ipld: Ipld = serde_ipld_dagcbor::from_slice(&bytes).unwrap();
        let Ipld::Map(map) = ipld else {
            panic!("expected map")
        };
        assert!(!map.contains_key("handle"));

        let evt = IdentityEvt {
            did: "did:plc:test".to_string(),
            handle: Some("alice.test".to_string()),
        };
        let bytes = serde_ipld_dagcbor::to_vec(&evt).unwrap();
        let ipld: Ipld = serde_ipld_dagcbor::from_slice(&bytes).unwrap();
        let Ipld::Map(map) = ipld else {
            panic!("expected map")
        };
        assert!(map.contains_key("handle"));
    }

    #[test]
    fn account_event_omits_absent_status() {
        let evt = AccountEvt {
            did: "did:plc:test".to_string(),
            active: true,
            status: None,
        };
        let bytes = serde_ipld_dagcbor::to_vec(&evt).unwrap();
        let ipld: Ipld = serde_ipld_dagcbor::from_slice(&bytes).unwrap();
        let Ipld::Map(map) = ipld else {
            panic!("expected map")
        };
        assert!(!map.contains_key("status"));

        let evt = AccountEvt {
            did: "did:plc:test".to_string(),
            active: false,
            status: Some(RskyLexiconAccountStatus::Takendown),
        };
        let bytes = serde_ipld_dagcbor::to_vec(&evt).unwrap();
        let ipld: Ipld = serde_ipld_dagcbor::from_slice(&bytes).unwrap();
        let Ipld::Map(map) = ipld else {
            panic!("expected map")
        };
        assert!(map.contains_key("status"));
    }

    #[test]
    fn sync_evt_data_from_commit_returns_error_when_block_missing() {
        let mut data = make_commit_data();
        // Replace relevant_blocks with an empty map so the cid is missing
        data.commit_data.relevant_blocks = BlockMap::new();
        let result = sync_evt_data_from_commit(data);
        assert!(result.is_err());
    }
}
