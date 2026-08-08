# Task 1: `events.rs` — event structs and formatters (port verbatim)

**Files:**
- Create: `pds/src/sequencer/mod.rs` (scaffold: module decls + ported `RepoSeq` struct)
- Create: `pds/src/sequencer/events.rs`
- Test: `pds/src/sequencer/events.rs` (`#[cfg(test)] mod tests`)
- Modify: `pds/Cargo.toml` (deps below)

Port source: the rsky-pds events.rs logic is now reached through the git-pinned `rsky-common` / `rsky-repo` / `rsky-lexicon` crates (`Cargo.toml:8-15`). The formatters return `crate::sequencer::RepoSeq` typed with `DbId` / `Did` / `OffsetDateTime` (see index.md "Cross-plan contracts" — the plan's plain struct mirrors the entity's typed columns).

- [ ] **Step 1: Add dependencies to `pds/Cargo.toml`**

Under `[dependencies]` add ONLY what is NOT already in `pds/Cargo.toml` (do not duplicate workspace entries):

```toml
serde_bytes = "0.11"   # was already on the workspace (Cargo.toml:38); add only if not already pinned
```

All other entries — `serde`, `serde_json`, `anyhow`, `tracing`, `rsky-common`, `rsky-repo`, `rsky-lexicon`, `chrono`, `time`, `lexicon_cid` (workspace alias `cid`) — are already declared in `pds/Cargo.toml` or `Cargo.toml:7-61`. The rsky crates come from the git-pinned fork (no `path = ".../vendor/rsky/..."` overrides).

Under `[dev-dependencies]` add ONLY what is missing:

```toml
ipld-core = "0.4"
serde_ipld_dagcbor = { workspace = true }   # already a dev-dep in pds/Cargo.toml:74; promote to [dependencies] in Step 1 — needed for runtime `struct_to_cbor` calls in formatters
tempfile = "3"
```

Run: `cargo check -p cacos-pds`
Expected: compiles, or fails only because `sequencer/events.rs` does not exist yet (fixed in Step 6). Note the binary crate's package name is `cacos-pds` (`pds/Cargo.toml:3`).

- [ ] **Step 2: Write the `RepoSeq` scaffold in `pds/src/sequencer/mod.rs`**

```rust
pub mod db;        // Task 2
pub mod events;
pub mod outbox;    // Task 5
pub mod ws_frames; // Task 3

use crate::migration_or_workspace_reexport::DbId; // re-exported through crate::db::entities::repo_seq path; otherwise:
                                                  // use migration::types::db_id::DbId;
                                                  // use migration::types::did::Did;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

/// Ported from rsky-pds `models::RepoSeq`. The sea-orm entity model for the
/// `repo_seq` table lives in `migration/src/entities/repo_seq.rs` (re-exported
/// through `pds::db::entities::repo_seq`); this plain struct mirrors that
/// entity with the typed wrappers (`DbId`, `Did`, `OffsetDateTime`) so the
/// formatters and outbox never touch raw `i64`/`String` for the PK, DID, or
/// timestamp. PK is application-generated (ULID), not SQL autoincrement.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RepoSeq {
    /// ULID PK. `None` only before the row is inserted; sea-orm reads back
    /// the assigned value, so callers see `Some(seq)` after `sequence_evt`.
    pub seq: Option<DbId>,
    pub did: Did,
    #[serde(rename = "eventType")]
    pub event_type: String,
    pub event: Vec<u8>,
    pub invalidated: Option<i16>,
    #[serde(rename = "sequencedAt")]
    pub sequenced_at: OffsetDateTime,
}

impl RepoSeq {
    pub fn new(
        did: Did,
        event_type: String,
        event: Vec<u8>,
        sequenced_at: OffsetDateTime,
    ) -> Self {
        RepoSeq {
            did,
            event_type,
            event,
            sequenced_at,
            invalidated: None, // default value used on insert (table default 0)
            seq: None,         // assigned by the application (DbId::new()) on insert
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use migration::types::{db_id::DbId, did::Did};
    use time::OffsetDateTime;

    #[test]
    fn repo_seq_new_uses_insert_defaults() {
        let did = Did::from("did:plc:x".to_owned());
        let now = OffsetDateTime::now_utc();
        let seq = RepoSeq::new(did.clone(), "append".to_owned(), vec![1, 2, 3], now);
        assert_eq!(seq.seq, None);
        assert_eq!(seq.invalidated, None);
        assert_eq!(seq.event_type, "append");
        assert_eq!(seq.did, did);
        assert_eq!(seq.sequenced_at, now);
    }

    #[test]
    fn repo_seq_serializes_event_type_and_sequenced_at_with_canonical_names() {
        let did = Did::from("did:plc:x".to_owned());
        let seq = RepoSeq::new(did, "append".to_owned(), vec![], OffsetDateTime::now_utc());
        let json = serde_json::to_value(&seq).unwrap();
        assert!(json.get("eventType").is_some());
        assert!(json.get("sequencedAt").is_some());
        // seq is None pre-insert, the JSON envelope skips nulls by serde default? — keep explicit:
        assert_eq!(json["eventType"], "append");
    }

    // unused-import suppression for tests that don't reference DbId directly
    #[allow(dead_code)]
    fn _typecheck_db_id() -> DbId { DbId::new() }
}
```

Note: `pub mod db;`, `pub mod outbox;`, `pub mod ws_frames;` are declared now so the module tree is stable; the files are created in Tasks 2, 5, 3 respectively. `pub mod events;` resolves in Step 6.

- [ ] **Step 3: Run the scaffold test to verify it passes**

Run: `cargo test -p cacos-pds sequencer::tests::repo_seq_new_uses_insert_defaults`
Expected: `test result: ok. 1 passed`

- [ ] **Step 4: Write the failing tests for `events.rs`** (golden-shape tests ported from the git-pinned `rsky-common`/`rsky-repo`/`rsky-lexicon` crates at rev `aee5aec5ad9473d80232beab58ddba25a936298a` — same shape as the reference `sequencer/tests.rs`)

Create `pds/src/sequencer/events.rs` containing ONLY this test module for now (the implementation replaces it in Step 6):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::helpers::account::AccountStatus;
    use crate::actor_store::repo::types::SyncEvtData;
    use ipld_core::ipld::Ipld;
    use lexicon_cid::Cid;
    use rsky_repo::block_map::BlockMap;
    use rsky_repo::cid_set::CidSet;
    use rsky_repo::types::{CommitAction, CommitData, CommitOp};
    use std::str::FromStr;

    const TEST_CID: &str = "bafkreibjfgx2gprinfvicegelk5kosd6y2frmqpqzwqkg7usac74l3t2v4";

    fn commit_data(cid: Cid) -> CommitDataWithOps {
        let mut relevant_blocks = BlockMap::new();
        relevant_blocks.set(cid, vec![1, 2, 3]);
        CommitDataWithOps {
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
    // The formatters take `Did`, not `&str`. The test bodies convert the
    // string DID at the call site (see updated test bodies below).

    fn as_map(ipld: Ipld) -> std::collections::BTreeMap<String, Ipld> {
        match ipld {
            Ipld::Map(map) => map,
            other => panic!("expected cbor map, got {other:?}"),
        }
    }

    fn sorted_keys(map: &std::collections::BTreeMap<String, Ipld>) -> Vec<&str> {
        map.keys().map(|key| key.as_str()).collect()
    }

    #[tokio::test]
    async fn commit_event_matches_reference_shape() {
        let cid = Cid::from_str(TEST_CID).unwrap();
        let evt = format_seq_commit("did:plc:golden".to_owned(), commit_data(cid))
            .await
            .unwrap();
        assert_eq!(evt.event_type, "append");
        let map = as_map(serde_ipld_dagcbor::from_slice(&evt.event).unwrap());
        // deprecated `prev` is omitted; `prevData` only appears when present
        assert_eq!(
            sorted_keys(&map),
            vec!["blobs", "blocks", "commit", "ops", "rebase", "repo", "rev", "since", "tooBig"]
        );
        assert!(matches!(map.get("blocks"), Some(Ipld::Bytes(_))));
        assert_eq!(map.get("rebase"), Some(&Ipld::Bool(false)));
        assert_eq!(map.get("tooBig"), Some(&Ipld::Bool(false)));
        assert_eq!(map.get("since"), Some(&Ipld::Null));
        let Some(Ipld::List(ops)) = map.get("ops") else {
            panic!("expected ops list");
        };
        let Ipld::Map(op) = &ops[0] else {
            panic!("expected op map");
        };
        // creates have no `prev`, and `cid` is always present
        assert_eq!(
            op.keys().map(|key| key.as_str()).collect::<Vec<&str>>(),
            vec!["action", "cid", "path"]
        );
        assert_eq!(op.get("action"), Some(&Ipld::String("create".to_owned())));
    }

    #[tokio::test]
    async fn commit_event_includes_prev_data_and_op_prev() {
        let cid = Cid::from_str(TEST_CID).unwrap();
        let mut data = commit_data(cid);
        data.prev_data = Some(cid);
        data.ops = vec![CommitOp {
            action: CommitAction::Delete,
            path: "app.bsky.feed.post/3jzfcijpj2z2a".to_owned(),
            cid: None,
            prev: Some(cid),
        }];
        let evt = format_seq_commit("did:plc:golden".to_owned(), data)
            .await
            .unwrap();
        let map = as_map(serde_ipld_dagcbor::from_slice(&evt.event).unwrap());
        assert_eq!(map.get("prevData"), Some(&Ipld::Link(cid)));
        let Some(Ipld::List(ops)) = map.get("ops") else {
            panic!("expected ops list");
        };
        let Ipld::Map(op) = &ops[0] else {
            panic!("expected op map");
        };
        // deletes carry a null `cid` and the previous record cid in `prev`
        assert_eq!(op.get("cid"), Some(&Ipld::Null));
        assert_eq!(op.get("prev"), Some(&Ipld::Link(cid)));
    }

    #[tokio::test]
    async fn sync_event_matches_reference_shape() {
        let cid = Cid::from_str(TEST_CID).unwrap();
        let mut blocks = BlockMap::new();
        blocks.set(cid, vec![1, 2, 3]);
        let evt = format_seq_sync_evt(
            "did:plc:golden".to_owned(),
            SyncEvtData {
                cid,
                rev: "3jzfcijpj2z2a".to_owned(),
                blocks,
            },
        )
        .await
        .unwrap();
        assert_eq!(evt.event_type, "sync");
        let map = as_map(serde_ipld_dagcbor::from_slice(&evt.event).unwrap());
        assert_eq!(sorted_keys(&map), vec!["blocks", "did", "rev"]);
        // the CAR slice is a CBOR byte string, not an integer array
        assert!(matches!(map.get("blocks"), Some(Ipld::Bytes(_))));
    }

    #[tokio::test]
    async fn identity_event_omits_absent_handle() {
        let evt = format_seq_identity_evt("did:plc:golden".to_owned(), None)
            .await
            .unwrap();
        assert_eq!(evt.event_type, "identity");
        let map = as_map(serde_ipld_dagcbor::from_slice(&evt.event).unwrap());
        assert_eq!(sorted_keys(&map), vec!["did"]);

        let evt = format_seq_identity_evt("did:plc:golden".to_owned(), Some("alice.test".to_owned()))
            .await
            .unwrap();
        let map = as_map(serde_ipld_dagcbor::from_slice(&evt.event).unwrap());
        assert_eq!(sorted_keys(&map), vec!["did", "handle"]);
        assert_eq!(
            map.get("handle"),
            Some(&Ipld::String("alice.test".to_owned()))
        );
    }

    #[tokio::test]
    async fn account_event_matches_reference_shape() {
        let evt = format_seq_account_evt("did:plc:golden".to_owned(), AccountStatus::Active)
            .await
            .unwrap();
        assert_eq!(evt.event_type, "account");
        let map = as_map(serde_ipld_dagcbor::from_slice(&evt.event).unwrap());
        assert_eq!(sorted_keys(&map), vec!["active", "did"]);
        assert_eq!(map.get("active"), Some(&Ipld::Bool(true)));

        for (status, expected) in [
            (AccountStatus::Takendown, "takendown"),
            (AccountStatus::Suspended, "suspended"),
            (AccountStatus::Deleted, "deleted"),
            (AccountStatus::Deactivated, "deactivated"),
        ] {
            let evt = format_seq_account_evt("did:plc:golden".to_owned(), status)
                .await
                .unwrap();
            let map = as_map(serde_ipld_dagcbor::from_slice(&evt.event).unwrap());
            assert_eq!(sorted_keys(&map), vec!["active", "did", "status"]);
            assert_eq!(map.get("active"), Some(&Ipld::Bool(false)));
            assert_eq!(map.get("status"), Some(&Ipld::String(expected.to_owned())));
        }
    }

    #[tokio::test]
    async fn sync_evt_data_from_commit_requires_commit_block() {
        let cid = Cid::from_str(TEST_CID).unwrap();
        let data = commit_data(cid);
        let sync_data = sync_evt_data_from_commit(data).await.unwrap();
        assert_eq!(sync_data.cid, cid);
        assert_eq!(sync_data.rev, "3jzfcijpj2z2a");

        let mut missing = commit_data(cid);
        missing.commit_data.relevant_blocks = BlockMap::new();
        let err = sync_evt_data_from_commit(missing).await.unwrap_err();
        assert!(err.to_string().contains("commit block was not found"));
    }
}
```

- [ ] **Step 5: Run the tests to verify they fail**

Run: `cargo test -p cacos-pds sequencer::events::tests`
Expected: FAIL — `cannot find function 'format_seq_commit'` (and the other formatters) in `sequencer::events`.

- [ ] **Step 6: Implement `pds/src/sequencer/events.rs`** (full port; only `use` paths change)

Replace the test-only file with the full module (keep the test module from Step 4 appended below):

```rust
use crate::account::helpers::account::AccountStatus;
use crate::actor_store::repo::types::SyncEvtData;
use crate::sequencer::RepoSeq;
use anyhow::Result;
use lexicon_cid::Cid;
use migration::types::db_id::DbId;
use migration::types::did::Did;
use rsky_common;
use rsky_common::struct_to_cbor;
use rsky_lexicon::com::atproto::sync::AccountStatus as LexiconAccountStatus;
use rsky_repo::block_map::BlockMap;
use rsky_repo::car::blocks_to_car_file;
use rsky_repo::types::{CommitAction, CommitDataWithOps};
use serde::de::Error as DeserializerError;
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CommitEvtOpAction {
    Create,
    Update,
    Delete,
}

impl fmt::Display for CommitEvtOpAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Match each variant and write its lowercase representation.
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
    /// For updates and deletes, the previous record CID. Omitted for creates.
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
    /// DEPRECATED -- unused in sync v1.1. Retained for deserializing legacy events.
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
    pub status: Option<LexiconAccountStatus>,
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
    pub r#type: String, // 'sync'
    #[serde(serialize_with = "crate::sequencer::serialize_db_id", deserialize_with = "crate::sequencer::deserialize_db_id")]
    pub seq: DbId,
    pub time: String,
    pub evt: SyncEvt,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct TypedCommitEvt {
    pub r#type: String, // 'commit'
    #[serde(serialize_with = "crate::sequencer::serialize_db_id", deserialize_with = "crate::sequencer::deserialize_db_id")]
    pub seq: DbId,
    pub time: String,
    pub evt: CommitEvt,
}

impl Default for TypedCommitEvt {
    fn default() -> Self {
        Self {
            r#type: "commit".to_string(),
            seq: DbId::default(),
            time: OffsetDateTime::now_utc()
                .format(&Rfc3339)
                .unwrap_or_else(|_| String::from("1970-01-01T00:00:00Z")),
            evt: CommitEvt {
                rebase: false,
                too_big: false,
                repo: "".to_string(),
                commit: Default::default(),
                prev: None,
                rev: "".to_string(),
                since: None,
                blocks: vec![],
                ops: vec![],
                blobs: vec![],
                prev_data: None,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct TypedHandleEvt {
    pub r#type: String, // 'handle'
    #[serde(serialize_with = "crate::sequencer::serialize_db_id", deserialize_with = "crate::sequencer::deserialize_db_id")]
    pub seq: DbId,
    pub time: String,
    pub evt: HandleEvt,
}

impl Default for TypedHandleEvt {
    fn default() -> Self {
        Self {
            r#type: "handle".to_string(),
            seq: DbId::default(),
            time: OffsetDateTime::now_utc()
                .format(&Rfc3339)
                .unwrap_or_else(|_| String::from("1970-01-01T00:00:00Z")),
            evt: HandleEvt {
                did: "".to_string(),
                handle: "".to_string(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct TypedIdentityEvt {
    pub r#type: String, // 'identity'
    #[serde(serialize_with = "crate::sequencer::serialize_db_id", deserialize_with = "crate::sequencer::deserialize_db_id")]
    pub seq: DbId,
    pub time: String,
    pub evt: IdentityEvt,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct TypedAccountEvt {
    pub r#type: String, // 'account'
    #[serde(serialize_with = "crate::sequencer::serialize_db_id", deserialize_with = "crate::sequencer::deserialize_db_id")]
    pub seq: DbId,
    pub time: String,
    pub evt: AccountEvt,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum SeqEvt {
    TypedCommitEvt(Box<TypedCommitEvt>),
    // TypedHandleEvt(TypedHandleEvt),
    TypedIdentityEvt(TypedIdentityEvt),
    TypedAccountEvt(TypedAccountEvt),
    // TypedTombstoneEvt(TypedTombstoneEvt),
    TypedSyncEvt(TypedSyncEvt),
}

impl<'de> Deserialize<'de> for SeqEvt {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        // Determine the correct variant based on the "type" field and deserialize accordingly.
        if let Some(typ) = value.get("type") {
            match typ.as_str() {
                Some("commit") => Ok(SeqEvt::TypedCommitEvt(
                    serde_json::from_value(value).map_err(DeserializerError::custom)?,
                )),
                Some("sync") => Ok(SeqEvt::TypedSyncEvt(
                    serde_json::from_value(value).map_err(DeserializerError::custom)?,
                )),
                Some("identity") => Ok(SeqEvt::TypedIdentityEvt(
                    serde_json::from_value(value).map_err(DeserializerError::custom)?,
                )),
                Some("account") => Ok(SeqEvt::TypedAccountEvt(
                    serde_json::from_value(value).map_err(DeserializerError::custom)?,
                )),
                _ => Err(DeserializerError::custom("Unknown event type")),
            }
        } else {
            Err(DeserializerError::missing_field("type"))
        }
    }
}

impl SeqEvt {
    /// Returns the typed ULID `DbId` of the event. The `DbId`'s monotonic
    /// `timestamp_ms()` is the stable ordering key used by the outbox
    /// (`cacos_outbox_buffer_lag`, `seq > last_seen` guards).
    pub fn seq(&self) -> DbId {
        match self {
            SeqEvt::TypedCommitEvt(this) => this.seq,
            SeqEvt::TypedIdentityEvt(this) => this.seq,
            SeqEvt::TypedAccountEvt(this) => this.seq,
            SeqEvt::TypedSyncEvt(this) => this.seq,
        }
    }
}

/// JSON envelope for the delivery channel: `serde_json::to_string` of the typed
/// event. The outbox parses it back with `serde_json::from_str::<SeqEvt>`.
pub fn seq_evt_to_envelope(evt: &SeqEvt) -> String {
    serde_json::to_string(evt).expect("SeqEvt is always serializable")
}

pub async fn format_seq_commit(
    did: String,
    commit_data: CommitDataWithOps,
) -> Result<RepoSeq> {
    let mut blocks_to_send = BlockMap::new();
    blocks_to_send.add_map(commit_data.commit_data.new_blocks)?;
    blocks_to_send.add_map(commit_data.commit_data.relevant_blocks)?;
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
    // Create the CAR file with all blocks
    let car_slice = blocks_to_car_file(Some(&commit_data.commit_data.cid), blocks_to_send).await?;

    let evt = CommitEvt {
        rebase: false,
        too_big: false, // always false in Sync 1.1
        repo: did.clone(),
        commit: commit_data.commit_data.cid,
        prev: None, // deprecated in Sync 1.1; reference implementation omits it
        rev: commit_data.commit_data.rev,
        since: commit_data.commit_data.since,
        blocks: car_slice,
        ops,
        blobs: vec![],
        prev_data: commit_data.prev_data,
    };

    Ok(RepoSeq::new(
        Did::from(did.clone()),
        "append".to_string(),
        struct_to_cbor(&evt)?,
        OffsetDateTime::now_utc(),
    ))
}

pub async fn format_seq_handle_update(did: String, handle: String) -> Result<RepoSeq> {
    let evt = HandleEvt {
        did: did.to_string(),
        handle,
    };
    Ok(RepoSeq::new(
        Did::from(did),
        "handle".to_string(),
        struct_to_cbor(&evt)?,
        OffsetDateTime::now_utc(),
    ))
}

pub async fn format_seq_identity_evt(
    did: String,
    handle: Option<String>,
) -> Result<RepoSeq> {
    let mut evt = IdentityEvt {
        did: did.to_string(),
        handle: None,
    };
    if let Some(handle) = handle {
        evt.handle = Some(handle);
    }
    Ok(RepoSeq::new(
        Did::from(did),
        "identity".to_string(),
        struct_to_cbor(&evt)?,
        OffsetDateTime::now_utc(),
    ))
}

pub async fn format_seq_account_evt(did: String, status: AccountStatus) -> Result<RepoSeq> {
    let mut evt = AccountEvt {
        did: did.to_string(),
        active: matches!(status, AccountStatus::Active),
        status: None,
    };
    if !matches!(status, AccountStatus::Active) {
        evt.status = Some(match status {
            AccountStatus::Takendown => LexiconAccountStatus::Takendown,
            AccountStatus::Suspended => LexiconAccountStatus::Suspended,
            AccountStatus::Deleted => LexiconAccountStatus::Deleted,
            AccountStatus::Deactivated => LexiconAccountStatus::Deactivated,
            AccountStatus::Desynchronized => LexiconAccountStatus::Desynchronized,
            AccountStatus::Throttled => LexiconAccountStatus::Throttled,
            _ => panic!("Conditional failed and allowed an invalid account status."),
        });
    }

    Ok(RepoSeq::new(
        Did::from(did),
        "account".to_string(),
        struct_to_cbor(&evt)?,
        OffsetDateTime::now_utc(),
    ))
}

pub async fn format_seq_sync_evt(did: String, data: SyncEvtData) -> Result<RepoSeq> {
    let blocks = blocks_to_car_file(Some(&data.cid), data.blocks).await?;
    let evt = SyncEvt {
        did: did.to_string(),
        rev: data.rev,
        blocks,
    };
    Ok(RepoSeq::new(
        Did::from(did),
        "sync".to_string(),
        struct_to_cbor(&evt)?,
        OffsetDateTime::now_utc(),
    ))
}

pub async fn sync_evt_data_from_commit(mut commit_data: CommitDataWithOps) -> Result<SyncEvtData> {
    let cid = vec![commit_data.commit_data.cid];
    match commit_data.commit_data.relevant_blocks.get_many(cid) {
        Ok(blocks_and_missing) if !blocks_and_missing.missing.is_empty() => Err(anyhow::anyhow!(
            "commit block was not found, could not build sync event"
        )),
        Ok(blocks_and_missing) => Ok(SyncEvtData {
            rev: commit_data.commit_data.rev,
            cid: commit_data.commit_data.cid,
            blocks: blocks_and_missing.blocks,
        }),
        Err(e) => Err(e),
    }
}
```

Then append the `#[cfg(test)] mod tests { ... }` block from Step 4 verbatim at the end of the file.

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test -p cacos-pds sequencer::events::tests`
Expected: `test result: ok. 6 passed` (5 golden-shape tests + `sync_evt_data_from_commit_requires_commit_block`).

- [ ] **Step 8: Commit**

```bash
git add pds/Cargo.toml pds/src/sequencer/mod.rs pds/src/sequencer/events.rs
git commit -m "feat(sequencer): port rsky event formatting (events.rs) and typed RepoSeq struct"

(commit each bucket separately — this is part of the schema-drift bucket)
```
