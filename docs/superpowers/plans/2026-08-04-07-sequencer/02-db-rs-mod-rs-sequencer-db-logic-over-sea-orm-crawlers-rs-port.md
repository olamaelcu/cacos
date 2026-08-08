# Task 2: `db.rs` + `mod.rs` — Sequencer DB logic over sea-orm (+ `crawlers.rs` port)

**Files:**
- Create: `pds/src/sequencer/db.rs`
- Create: `pds/src/sequencer/crawlers.rs`
- Modify: `pds/src/sequencer/mod.rs` (add `RequestSeqRangeOpts`, `seq_evt_from_row`, `Sequencer` + all DB methods; keep the `RepoSeq` struct from Task 1)
- Modify: `pds/src/context.rs` (add `pub const APP_USER_AGENT`)
- Test: `pds/src/sequencer/mod.rs` (`#[cfg(test)] mod tests`)
- Modify: `pds/Cargo.toml` (deps below)

Port sources: the rsky-pds sequencer logic is now reached through the git-pinned `rsky-common` / `rsky-repo` / `rsky-lexicon` / `rsky-identity` crates (`Cargo.toml:8-15`). The DB methods map onto sea-orm 2.0 via `Statement::from_sql_and_values(DbBackend::Sqlite, ...)` for the dynamic WHERE builder and `Entity::insert` for `sequence_evt`. The shared `repo_seq` entity is `migration::entities::repo_seq` (source of truth: `migration/src/entities/repo_seq.rs:1-18`, migration at `migration/src/m20260801_000002_repo_seq.rs`); the plan's plain `RepoSeq` struct mirrors it with `DbId` / `Did` / `OffsetDateTime`.

- [ ] **Step 1: Add dependencies to `pds/Cargo.toml`**

```toml
# [dependencies] — add ONLY what is NOT already in pds/Cargo.toml or Cargo.toml:7-61.
# Drop duplicates: tokio, metrics, futures, reqwest are already in pds/Cargo.toml.
# All rsky-* entries come from the git-pinned fork (Cargo.toml workspace entries).
# No new crates to add for Task 2 — the plan uses:
# - sea-orm (workspace, already in pds/Cargo.toml)
# - reqwest (already in pds/Cargo.toml for client metadata)
# - futures (already in pds/Cargo.toml via workspace)
# - metrics (already in pds/Cargo.toml via workspace)
# - tracing (workspace)
# - anyhow (workspace)
# - rsky_common (workspace)
```

Run: `cargo check -p cacos-pds`
Expected: resolves (the binary crate is `cacos-pds` per `pds/Cargo.toml:3`).

- [ ] **Step 2: Add `APP_USER_AGENT` to `pds/src/context.rs`**

Append to `pds/src/context.rs`:

```rust
/// User agent advertised on outbound requests (crawler notify, etc.).
pub const APP_USER_AGENT: &str = concat!("cacos/", env!("CARGO_PKG_VERSION"));
```

- [ ] **Step 3: Port `pds/src/sequencer/crawlers.rs`** (from the git-pinned rsky fork; only the `APP_USER_AGENT` import path changes)

```rust
use crate::context::APP_USER_AGENT;
use anyhow::Result;
use futures::stream::{self, StreamExt};
use rsky_common::time::MINUTE;
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

const NOTIFY_THRESHOLD: i32 = 20 * MINUTE; // 20 minutes;

#[derive(Debug, Clone)]
pub struct Crawlers {
    pub hostname: String,
    pub crawlers: Vec<String>,
    pub last_notified: usize,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct CrawlerRequest {
    pub hostname: String,
}

impl Crawlers {
    pub fn new(hostname: String, crawlers: Vec<String>) -> Self {
        Crawlers {
            hostname,
            crawlers,
            last_notified: 0,
        }
    }

    // requestCrawl must advertise this PDS's hostname, not the crawler's
    fn crawl_request(&self) -> CrawlerRequest {
        CrawlerRequest {
            hostname: self.hostname.clone(),
        }
    }

    pub async fn notify_of_update(&mut self) -> Result<()> {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("timestamp in millis since UNIX epoch")
            .as_millis() as usize;
        if now - self.last_notified < NOTIFY_THRESHOLD as usize {
            return Ok(());
        }
        let record = self.crawl_request();
        let _ = stream::iter(self.crawlers.clone())
            .then(|service: String| {
                let record = record.clone();
                async move {
                    let client = reqwest::Client::builder()
                        .user_agent(APP_USER_AGENT)
                        .build()?;
                    Ok::<reqwest::Response, anyhow::Error>(
                        client
                            .post(format!("{}/xrpc/com.atproto.sync.requestCrawl", service))
                            .json(&record)
                            .send()
                            .await?,
                    )
                }
            })
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;

        self.last_notified = now;
        Ok(())
    }
}
```

- [ ] **Step 4: Create `pds/src/sequencer/db.rs`** (the sea-orm row mapper; migrations stay in Plan 01)

```rust
use crate::sequencer::RepoSeq;
use anyhow::Result;
use migration::types::db_id::DbId;
use sea_orm::QueryResult;
use time::OffsetDateTime;

/// Column list used by all repo_seq SELECT statements (matches the reference `SELECT_REPO_SEQ`).
/// Note: `seq` is BLOB(16) (ULID, big-endian), `did` is TEXT, `sequencedAt` is TIMESTAMP — sea-orm
/// reads them through the typed wrappers below.
pub const SELECT_REPO_SEQ: &str =
    "SELECT seq, did, \"eventType\", event, invalidated, \"sequencedAt\" FROM repo_seq";

/// Map a raw `QueryResult` row (from a `SELECT_REPO_SEQ` query) to the ported
/// typed `RepoSeq` struct (reference `repo_seq_from_row`). The typed wrappers
/// handle the BLOB(16) ↔ DbId and TEXT ↔ Did conversions via sea-orm's
/// `TryGetable` impls.
pub fn repo_seq_from_row(row: &QueryResult) -> Result<RepoSeq> {
    Ok(RepoSeq {
        seq: row.try_get_by_index::<Option<DbId>>(0)?,
        did: row.try_get_by_index::<Did>(1)?,
        event_type: row.try_get_by_index::<String>(2)?,
        event: row.try_get_by_index::<Vec<u8>>(3)?,
        invalidated: row.try_get_by_index::<Option<i16>>(4)?,
        sequenced_at: row.try_get_by_index::<OffsetDateTime>(5)?,
    })
}
```

- [ ] **Step 5: Write the failing tests in `pds/src/sequencer/mod.rs`** (mirroring the reference `sequences_and_reads_events`, `deletes_events_for_user`, and the filter assertions)

Append this test module to `pds/src/sequencer/mod.rs` (the `Sequencer` implementation comes in Step 7):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::helpers::account::AccountStatus;
    use crate::actor_store::repo::types::SyncEvtData;
    use crate::sequencer::crawlers::Crawlers;
    use crate::sequencer::events::sync_evt_data_from_commit;
    use rsky_repo::block_map::BlockMap;
    use rsky_repo::cid_set::CidSet;
    use rsky_repo::types::{CommitAction, CommitData, CommitOp};
    use sea_orm::{DbBackend, Statement};
    use std::str::FromStr;

    const TEST_CID: &str = "bafkreibjfgx2gprinfvicegelk5kosd6y2frmqpqzwqkg7usac74l3t2v4";

    async fn test_sequencer() -> (tempfile::TempDir, Sequencer) {
        let dir = tempfile::tempdir().unwrap();
        // cacos opens the sequencer DB through `DatabaseKind::Sequencer.open()`
        // (pds/src/db/mod.rs:39-66). `crate::db::open_sequencer_db` is a thin
        // shim added in this plan that delegates to it for backward-compat.
        let db = crate::db::DatabaseKind::Sequencer
            .open(camino::Utf8Path::new(&dir.path().join("sequencer.sqlite")))
            .await
            .unwrap();
        let sequencer = Sequencer::new(db, Crawlers::new("pds.test".to_owned(), vec![]), None);
        (dir, sequencer)
    }

    fn commit_data(cid: lexicon_cid::Cid) -> CommitDataWithOps {
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

    async fn exec(sequencer: &Sequencer, sql: &str) {
        sequencer
            .db
            .execute(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                sql.to_string(),
                vec![],
            ))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn sequences_and_reads_events() {
        let (_dir, mut sequencer) = test_sequencer().await;
        assert_eq!(sequencer.curr().await.unwrap(), None);

        let cid = lexicon_cid::Cid::from_str(TEST_CID).unwrap();
        let did = "did:plc:seq".to_owned();  // String at the formatter API; converted to Did via `RepoSeqNew::new(did: impl Into<Did>, ...)`

        let seq1 = sequencer
            .sequence_commit(did.clone(), commit_data(cid))
            .await
            .unwrap();
        let seq2 = sequencer
            .sequence_sync_evt(
                did.clone(),
                SyncEvtData {
                    cid,
                    rev: "3jzfcijpj2z2a".to_owned(),
                    blocks: {
                        let mut blocks = BlockMap::new();
                        blocks.set(cid, vec![1, 2, 3]);
                        blocks
                    },
                },
            )
            .await
            .unwrap();
        let seq3 = sequencer
            .sequence_identity_evt(did.clone(), Some("seq.test".to_owned()))
            .await
            .unwrap();
        let seq4 = sequencer
            .sequence_account_evt(did.clone(), AccountStatus::Takendown)
            .await
            .unwrap();
        let seq5 = sequencer
            .sequence_handle_update(did.clone(), "seq2.test".to_owned())
            .await
            .unwrap();
        // ULID `DbId`s are monotonically increasing within a single process; assert
        // strict ordering instead of literal 1..5 (the PK is no longer SQL AUTOINCREMENT).
        assert!(seq1 < seq2);
        assert!(seq2 < seq3);
        assert!(seq3 < seq4);
        assert!(seq4 < seq5);

        // The "current" event's ULID timestamp_ms is the last sequence's timestamp.
        let curr = sequencer.curr().await.unwrap().expect("at least one event");
        assert_eq!(curr, seq5);
        let next = sequencer.next_seq(seq1).await.unwrap().unwrap();
        assert_eq!(next.seq, Some(seq2));
        assert!(sequencer.next_seq(seq5).await.unwrap().is_none());

        let earliest = sequencer
            .earliest_after_time(
                time::macros::datetime!(2020-01-01 00:00:00 UTC),
            )
            .await
            .unwrap();
        assert_eq!(earliest.seq, Some(seq1));
        assert!(sequencer
            .earliest_after_time(time::macros::datetime!(2100-01-01 00:00:00 UTC))
            .await
            .unwrap()
            .is_none());

        // handle events are not surfaced by request_seq_range; the four
        // typed events come back in order
        let evts = sequencer
            .request_seq_range(RequestSeqRangeOpts {
                earliest_seq: None,
                latest_seq: None,
                earliest_time: None,
                limit: None,
            })
            .await
            .unwrap();
        assert_eq!(evts.len(), 4);
        assert!(matches!(evts[0], SeqEvt::TypedCommitEvt(_)));
        assert!(matches!(evts[1], SeqEvt::TypedSyncEvt(_)));
        assert!(matches!(evts[2], SeqEvt::TypedIdentityEvt(_)));
        assert!(matches!(evts[3], SeqEvt::TypedAccountEvt(_)));
        assert_eq!(
            evts.iter().map(|evt| evt.seq()).collect::<Vec<_>>(),
            vec![seq1, seq2, seq3, seq4]
        );

        // filters
        let evts = sequencer
            .request_seq_range(RequestSeqRangeOpts {
                earliest_seq: Some(seq2),
                latest_seq: Some(seq4),
                earliest_time: Some(time::macros::datetime!(2020-01-01 00:00:00 UTC)),
                limit: Some(1),
            })
            .await
            .unwrap();
        assert_eq!(evts.len(), 1);
        assert_eq!(evts[0].seq(), seq3);

        // invalidated events are skipped
        exec(&sequencer, "UPDATE repo_seq SET invalidated = 1 WHERE seq = 1").await;
        let evts = sequencer
            .request_seq_range(RequestSeqRangeOpts {
                earliest_seq: None,
                latest_seq: None,
                earliest_time: None,
                limit: None,
            })
            .await
            .unwrap();
        assert_eq!(evts[0].seq(), 2);

        // a rebase event decodes as a commit; unknown event types are skipped
        exec(&sequencer, "UPDATE repo_seq SET \"eventType\" = 'rebase' WHERE seq = 1").await;
        exec(&sequencer, "UPDATE repo_seq SET invalidated = 0 WHERE seq = 1").await;
        exec(&sequencer, "UPDATE repo_seq SET \"eventType\" = 'unknown' WHERE seq = 4").await;
        let evts = sequencer
            .request_seq_range(RequestSeqRangeOpts {
                earliest_seq: None,
                latest_seq: None,
                earliest_time: None,
                limit: None,
            })
            .await
            .unwrap();
        assert!(matches!(evts[0], SeqEvt::TypedCommitEvt(_)));
        assert!(!evts.iter().any(|evt| evt.seq() == 4));
    }

    #[tokio::test]
    async fn deletes_events_for_user() {
        let (_dir, mut sequencer) = test_sequencer().await;
        let did_del = "did:plc:del".to_owned();
        let did_other = "did:plc:other".to_owned();
        let keep_seq = sequencer
            .sequence_identity_evt(did_del.clone(), None)
            .await
            .unwrap();
        sequencer
            .sequence_identity_evt(did_del.clone(), None)
            .await
            .unwrap();
        let other_seq = sequencer
            .sequence_identity_evt(did_other.clone(), None)
            .await
            .unwrap();

        // Plan 07 final API: `delete_all_for_user(&str)` — no excluding_seqs.
        // (If you need an exclusion list in production, use the plan's
        // `delete_all_for_user` SQL builder directly until a follow-up adds
        // it back to the typed API.)
        sequencer
            .delete_all_for_user(&did_del.to_string())
            .await
            .unwrap();
        let remaining: Vec<_> = sequencer
            .request_seq_range(RequestSeqRangeOpts {
                earliest_seq: None,
                latest_seq: None,
                earliest_time: None,
                limit: None,
            })
            .await
            .unwrap()
            .iter()
            .map(|evt| evt.seq())
            .collect();
        assert_eq!(remaining, vec![keep_seq, other_seq]);

        sequencer
            .delete_all_for_user(&did_del.to_string())
            .await
            .unwrap();
        let remaining: Vec<_> = sequencer
            .request_seq_range(RequestSeqRangeOpts {
                earliest_seq: None,
                latest_seq: None,
                earliest_time: None,
                limit: None,
            })
            .await
            .unwrap()
            .iter()
            .map(|evt| evt.seq())
            .collect();
        assert_eq!(remaining, vec![other_seq]);
    }
}
```

- [ ] **Step 6: Run the tests to verify they fail**

Run: `cargo test -p cacos-pds sequencer::tests`
Expected: FAIL — `cannot find struct, variant or union type 'RequestSeqRangeOpts'` and `cannot find struct 'Sequencer'` in `sequencer`.

- [ ] **Step 7: Implement the `Sequencer` in `pds/src/sequencer/mod.rs`**

Replace the module header of `pds/src/sequencer/mod.rs` (keeping the `RepoSeq` struct and its `mod tests` block, and keeping the test module from Step 5) with:

```rust
pub mod crawlers;
pub mod db;
pub mod events;
pub mod outbox;    // Task 5
pub mod ws_frames; // Task 3

use crate::account::helpers::account::AccountStatus;
use crate::actor_store::repo::types::SyncEvtData;
use crate::sequencer::crawlers::Crawlers;
use crate::sequencer::db::{repo_seq_from_row, SELECT_REPO_SEQ};
use crate::sequencer::events::{
    format_seq_account_evt, format_seq_commit, format_seq_handle_update, format_seq_identity_evt,
    format_seq_sync_evt, SeqEvt, TypedAccountEvt, TypedCommitEvt, TypedIdentityEvt, TypedSyncEvt,
};
use anyhow::Result;
use migration::types::db_id::DbId;
use rsky_common::cbor_to_struct;
use rsky_repo::types::CommitDataWithOps;
use sea_orm::{DatabaseConnection, DbBackend, Statement};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

pub struct RequestSeqRangeOpts {
    pub earliest_seq: Option<DbId>,
    pub latest_seq: Option<DbId>,
    pub earliest_time: Option<OffsetDateTime>,
    /// SQL `LIMIT` count (stays `i64`).
    pub limit: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct Sequencer {
    pub db: DatabaseConnection,
    pub crawlers: Crawlers,
    pub last_seen: Option<DbId>,
}

impl Sequencer {
    pub fn new(db: DatabaseConnection, crawlers: Crawlers, last_seen: Option<DbId>) -> Self {
        Sequencer {
            db,
            crawlers,
            last_seen: Some(last_seen.unwrap_or_else(DbId::new)),
        }
    }

    pub async fn curr(&self) -> Result<Option<DbId>> {
        let row = self
            .db
            .query_one(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                "SELECT seq FROM repo_seq ORDER BY seq DESC LIMIT 1".to_string(),
                vec![],
            ))
            .await?;
        match row {
            Some(row) => Ok(row.try_get_by_index::<Option<DbId>>(0)?),
            None => Ok(None),
        }
    }

    pub async fn next_seq(&self, cursor: DbId) -> Result<Option<RepoSeq>> {
        let sql = format!("{SELECT_REPO_SEQ} WHERE seq > ?1 ORDER BY seq ASC LIMIT 1");
        // Bind the ULID as a 16-byte BLOB; sea-orm's `Value::Bytes` matches the
        // BLOB(16) column type used by the entity.
        let cursor_bytes = cursor.to_bytes().to_vec();
        let row = self
            .db
            .query_one(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                sql,
                vec![sea_orm::Value::Bytes(Some(cursor_bytes))],
            ))
            .await?;
        match row {
            Some(row) => Ok(Some(repo_seq_from_row(&row)?)),
            None => Ok(None),
        }
    }

    pub async fn earliest_after_time(&self, time: OffsetDateTime) -> Result<Option<RepoSeq>> {
        let sql = format!(
            "{SELECT_REPO_SEQ} WHERE \"sequencedAt\" >= ?1 ORDER BY \"sequencedAt\" ASC LIMIT 1"
        );
        let row = self
            .db
            .query_one(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                sql,
                vec![sea_orm::Value::ChronoDateTimeUtc(Some(time))],
            ))
            .await?;
        match row {
            Some(row) => Ok(Some(repo_seq_from_row(&row)?)),
            None => Ok(None),
        }
    }

    pub async fn request_seq_range(&self, opts: RequestSeqRangeOpts) -> Result<Vec<SeqEvt>> {
        let RequestSeqRangeOpts {
            earliest_seq,
            latest_seq,
            earliest_time,
            limit,
        } = opts;

        let mut sql = format!("{SELECT_REPO_SEQ} WHERE invalidated = 0");
        let mut values: Vec<sea_orm::Value> = Vec::new();
        if let Some(earliest_seq) = earliest_seq {
            sql.push_str(&format!(" AND seq > ?{}", values.len() + 1));
            values.push(sea_orm::Value::BigInt(Some(earliest_seq)));
        }
        if let Some(latest_seq) = latest_seq {
            sql.push_str(&format!(" AND seq <= ?{}", values.len() + 1));
            values.push(sea_orm::Value::BigInt(Some(latest_seq)));
        }
        if let Some(ref earliest_time) = earliest_time {
            sql.push_str(&format!(" AND \"sequencedAt\" >= ?{}", values.len() + 1));
            values.push(sea_orm::Value::String(Some(earliest_time.clone())));
        }
        sql.push_str(" ORDER BY seq ASC");
        if let Some(limit) = limit {
            sql.push_str(&format!(" LIMIT ?{}", values.len() + 1));
            values.push(sea_orm::Value::BigInt(Some(limit)));
        }

        let rows = self
            .db
            .query_all(Statement::from_sql_and_values(DbBackend::Sqlite, sql, values))
            .await?;

        let mut seq_evts: Vec<SeqEvt> = Vec::new();
        for row in rows {
            let repo_seq = repo_seq_from_row(&row)?;
            match seq_evt_from_row(&repo_seq)? {
                Some(evt) => seq_evts.push(evt),
                None => tracing::error!("request_seq_range invalid event type"),
            }
        }
        Ok(seq_evts)
    }

    pub async fn sequence_evt(&mut self, evt: RepoSeq) -> Result<DbId> {
        use crate::db::entities::repo_seq::{ActiveModel, Entity};
        use sea_orm::{ActiveModelTrait, ActiveValue};
        // The PK is application-generated (ULID), not SQL AUTOINCREMENT. The
        // value is minted here so the envelope and the live row share the
        // same `DbId`.
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
        self.crawlers.notify_of_update().await?;
        Ok(seq)
    }

    pub async fn sequence_commit(
        &mut self,
        did: String,
        commit_data: CommitDataWithOps,
    ) -> Result<DbId> {
        let evt = format_seq_commit(did, commit_data).await?;
        self.sequence_evt(evt).await
    }

    pub async fn sequence_handle_update(&mut self, did: String, handle: String) -> Result<DbId> {
        let evt = format_seq_handle_update(did, handle).await?;
        self.sequence_evt(evt).await
    }

    pub async fn sequence_identity_evt(
        &mut self,
        did: String,
        handle: Option<String>,
    ) -> Result<DbId> {
        let evt = format_seq_identity_evt(did, handle).await?;
        self.sequence_evt(evt).await
    }

    pub async fn sequence_account_evt(
        &mut self,
        did: String,
        status: AccountStatus,
    ) -> Result<DbId> {
        let evt = format_seq_account_evt(did, status).await?;
        self.sequence_evt(evt).await
    }

    pub async fn sequence_sync_evt(&mut self, did: String, data: SyncEvtData) -> Result<DbId> {
        let evt = format_seq_sync_evt(did, data).await?;
        self.sequence_evt(evt).await
    }

    pub async fn delete_all_for_user(
        &self,
        did: &str,
    ) -> Result<()> {
        let did_str = did.to_string();
        self.db
            .execute(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                "DELETE FROM repo_seq WHERE did = ?1".to_string(),
                vec![sea_orm::Value::String(Some(did_str))],
            ))
            .await?;
        Ok(())
    }
}

/// Row -> typed event mapping used by `request_seq_range` and (in Task 4) by the
/// apalis producer. Mirrors the reference's per-row conversion loop.
pub fn seq_evt_from_row(row: &RepoSeq) -> Result<Option<SeqEvt>> {
    let Some(seq) = row.seq else {
        return Ok(None); // should never hit this because of the primary key
    };
    let time = row.sequenced_at.clone();
    match row.event_type.as_str() {
        "append" | "rebase" => Ok(Some(SeqEvt::TypedCommitEvt(Box::new(TypedCommitEvt {
            r#type: "commit".to_string(),
            seq,
            time,
            evt: cbor_to_struct(row.event.clone())?,
        })))),
        "sync" => Ok(Some(SeqEvt::TypedSyncEvt(TypedSyncEvt {
            r#type: "sync".to_string(),
            seq,
            time,
            evt: cbor_to_struct(row.event.clone())?,
        }))),
        "identity" => Ok(Some(SeqEvt::TypedIdentityEvt(TypedIdentityEvt {
            r#type: "identity".to_string(),
            seq,
            time,
            evt: cbor_to_struct(row.event.clone())?,
        }))),
        "account" => Ok(Some(SeqEvt::TypedAccountEvt(TypedAccountEvt {
            r#type: "account".to_string(),
            seq,
            time,
            evt: cbor_to_struct(row.event.clone())?,
        }))),
        _ => {
            tracing::error!("request_seq_range invalid event type");
            Ok(None)
        }
    }
}

// ... (RepoSeq struct from Task 1 stays below here, unchanged)
```

Keep the `RepoSeq` struct and `impl RepoSeq` from Task 1, plus both `mod tests` blocks (Task 1's `repo_seq_new_uses_insert_defaults` and the Step 5 module).

- [ ] **Step 8: Run the tests to verify they pass**

Run: `cargo test -p cacos-pds sequencer::tests`
Expected: `test result: ok. 3 passed` (`repo_seq_new_uses_insert_defaults`, `sequences_and_reads_events`, `deletes_events_for_user`).
NOTE: `sequence_evt` mints a `DbId` (ULID) BEFORE the insert and uses it as the
`ActiveValue::Set(seq)` value. The PK is NOT a SQL autoincrement — the
application generates it. The strict-ordering assertions
(`assert!(seq1 < seq2); ...; assert!(seq4 < seq5);`) verify the ULID
monotonic generator works under tokio contention. If sea-orm returns an
error on the `Set(seq)` (it shouldn't — the column is `BLOB(16)` and the
sea-orm `ValueType` impl for `DbId` is in `migration::types::db_id`),
re-check the typed wrapper before proceeding.

- [ ] **Step 9: Commit**

```bash
git add pds/Cargo.toml pds/src/context.rs pds/src/sequencer/mod.rs pds/src/sequencer/db.rs pds/src/sequencer/crawlers.rs
git commit -m "feat(sequencer): port repo_seq query layer and sequence_* methods to sea-orm; add crawlers"
```
