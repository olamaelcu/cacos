// Sequencer + outbox for the firehose (subscribeRepos) endpoint.
//
// The sequencer persists a `repo_seq` row for every commit / identity /
// account / sync event, then enqueues an `SeqEventJob` to the apalis-sql
// worker. The worker publishes the serialized envelope to a
// `tokio::sync::broadcast` channel that the websocket subscribers drain.
//
// The `Outbox` provides a unified "backfill then live" stream that the
// subscribe-repos handler awaits.
//
// This file is `include!`-ed by `src/lib.rs`, so the submodules are
// declared in `lib.rs` (one canonical place). Do not re-declare them
// here. (Inner doc comments (`//!`) would require an item to attach to.)

use cacos_pds_core::error::PdsError;
use cacos_pds_core::observability::timing::timed;
use crate::apalis_worker::{SeqEventJob, enqueue_seq_event_job};
use crate::crawlers::Crawlers;
use crate::events::{
    RepoSeqNew, SeqEvt, TypedAccountEvt, TypedCommitEvt, TypedIdentityEvt, TypedSyncEvt,
    format_offset_datetime,
};
use migration::entities::repo_seq;
use migration::types::db_id::DbId;
use anyhow::Result;
use rsky_lexicon::com::atproto::sync::AccountStatus as RskyLexiconAccountStatus;
use rsky_repo::block_map::BlockMap;
use rsky_repo::types::CommitDataWithOps;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseBackend, DatabaseConnection, EntityTrait, QueryFilter,
    Statement,
};
use sqlx_0_8::sqlite::SqlitePool;
use std::sync::Arc;
use time::OffsetDateTime;

#[derive(Debug, Clone, Default)]
pub struct RequestSeqRangeOpts {
    pub earliest_seq: Option<DbId>,
    pub latest_seq: Option<DbId>,
    pub earliest_time: Option<OffsetDateTime>,
    pub limit: Option<i64>,
}

#[derive(Clone)]
pub struct Sequencer {
    pub db: DatabaseConnection,
    pub crawlers: Crawlers,
    pub job_pool: Option<Arc<SqlitePool>>,
    pub last_seen: Option<DbId>,
}

impl std::fmt::Debug for Sequencer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sequencer")
            .field("db", &"<DatabaseConnection>")
            .field("crawlers", &self.crawlers)
            .field("job_pool", &self.job_pool.as_ref().map(|_| "<SqlitePool>"))
            .field("last_seen", &self.last_seen)
            .finish()
    }
}

impl Sequencer {
    pub fn new(
        db: DatabaseConnection,
        crawlers: Crawlers,
        job_pool: Option<Arc<SqlitePool>>,
        last_seen: Option<DbId>,
    ) -> Self {
        Self {
            db,
            crawlers,
            job_pool,
            last_seen,
        }
    }

    pub async fn curr(&self) -> Result<Option<DbId>> {
        let stmt = Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT seq FROM repo_seq ORDER BY seq DESC LIMIT 1",
            vec![],
        );
        let row = self.db.query_one_raw(stmt).await?;
        if let Some(row) = row {
            let bytes: Vec<u8> = row.try_get_by_index(0)?;
            if bytes.len() == 16 {
                let mut arr = [0u8; 16];
                arr.copy_from_slice(&bytes);
                Ok(Some(DbId::from_bytes(arr)))
            } else {
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }

    pub async fn next_seq(&self, cursor: DbId) -> Result<Option<repo_seq::Model>> {
        let bytes = cursor.to_bytes().to_vec();
        let stmt = Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT seq, did, \"eventType\", event, invalidated, \"sequencedAt\" \
             FROM repo_seq WHERE seq > ?1 ORDER BY seq ASC LIMIT 1",
            vec![bytes.into()],
        );
        let row = self.db.query_one_raw(stmt).await?;
        match row {
            Some(row) => Ok(Some(repo_seq_from_row(&row)?)),
            None => Ok(None),
        }
    }

    pub async fn earliest_after_time(
        &self,
        time: OffsetDateTime,
    ) -> Result<Option<repo_seq::Model>> {
        let formatted = format_offset_datetime(time);
        let stmt = Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT seq, did, \"eventType\", event, invalidated, \"sequencedAt\" \
             FROM repo_seq WHERE \"sequencedAt\" >= ?1 \
             ORDER BY \"sequencedAt\" ASC LIMIT 1",
            vec![formatted.into()],
        );
        let row = self.db.query_one_raw(stmt).await?;
        match row {
            Some(row) => Ok(Some(repo_seq_from_row(&row)?)),
            None => Ok(None),
        }
    }

    pub async fn request_seq_range(
        &self,
        opts: RequestSeqRangeOpts,
    ) -> Result<Vec<repo_seq::Model>> {
        timed("seq_poll", async {
            self.request_seq_range_inner(opts).await
        })
        .await
    }

    async fn request_seq_range_inner(
        &self,
        opts: RequestSeqRangeOpts,
    ) -> Result<Vec<repo_seq::Model>> {
        let RequestSeqRangeOpts {
            earliest_seq,
            latest_seq,
            earliest_time,
            limit,
        } = opts;

        let mut sql = String::from(
            "SELECT seq, did, \"eventType\", event, invalidated, \"sequencedAt\" \
             FROM repo_seq WHERE invalidated = 0",
        );
        let mut values: Vec<sea_orm::Value> = Vec::new();
        if let Some(earliest_seq) = earliest_seq {
            sql.push_str(&format!(" AND seq > ?{}", values.len() + 1));
            values.push(sea_orm::Value::Bytes(Some(
                earliest_seq.to_bytes().to_vec(),
            )));
        }
        if let Some(latest_seq) = latest_seq {
            sql.push_str(&format!(" AND seq <= ?{}", values.len() + 1));
            values.push(sea_orm::Value::Bytes(Some(latest_seq.to_bytes().to_vec())));
        }
        if let Some(earliest_time) = earliest_time {
            sql.push_str(&format!(" AND \"sequencedAt\" >= ?{}", values.len() + 1));
            values.push(sea_orm::Value::String(Some(format_offset_datetime(
                earliest_time,
            ))));
        }
        sql.push_str(" ORDER BY seq ASC");
        if let Some(limit) = limit {
            sql.push_str(&format!(" LIMIT ?{}", values.len() + 1));
            values.push(sea_orm::Value::BigInt(Some(limit)));
        }
        let stmt = Statement::from_sql_and_values(DatabaseBackend::Sqlite, sql, values);
        let rows = self.db.query_all_raw(stmt).await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows.iter() {
            out.push(repo_seq_from_row(row)?);
        }
        Ok(out)
    }

    pub async fn sequence_evt(&mut self, evt: RepoSeqNew) -> Result<DbId> {
        let mut active: repo_seq::ActiveModel = evt.into();
        // The seq is the primary key (typed DbId=ULID, not auto-increment);
        // generate it locally so the INSERT succeeds.
        let next_seq = DbId::new();
        active.seq = sea_orm::ActiveValue::Set(next_seq);
        let insert = repo_seq::Entity::insert(active)
            .exec_with_returning(&self.db)
            .await?;
        let seq = insert.seq;
        // Best-effort crawler notify; never block sequencing on it.
        if let Err(err) = self.crawlers.notify_of_update().await {
            tracing::warn!("crawler notify failed: {err}");
        }
        if let Some(pool) = &self.job_pool {
            let typed = typed_seq_evt(&insert)?;
            let seq_ms = insert.seq.0.timestamp_ms() as i64;
            let envelope = serde_json::to_string(&typed)?;
            let job = SeqEventJob {
                seq: seq_ms,
                envelope,
                pool: Some(pool.clone()),
            };
            if let Err(err) = enqueue_seq_event_job(pool.as_ref(), &job).await {
                tracing::warn!("failed to push seq event job: {err}");
            }
        }
        metrics::gauge!(cacos_pds_core::observability::metrics::LAST_SEQ,).set(seq.0.timestamp_ms() as f64);
        Ok(seq)
    }

    pub async fn sequence_commit(
        &mut self,
        did: String,
        commit_data: CommitDataWithOps,
    ) -> Result<DbId> {
        let evt = events::format_seq_commit(did, commit_data).await?;
        self.sequence_evt(evt).await
    }

    pub async fn sequence_handle_update(&mut self, did: String, handle: String) -> Result<DbId> {
        let evt = events::format_seq_handle_update(did, handle)?;
        self.sequence_evt(evt).await
    }

    pub async fn sequence_identity_evt(
        &mut self,
        did: String,
        handle: Option<String>,
    ) -> Result<DbId> {
        let evt = events::format_seq_identity_evt(did, handle)?;
        self.sequence_evt(evt).await
    }

    pub async fn sequence_account_evt(
        &mut self,
        did: String,
        active: bool,
        status: Option<RskyLexiconAccountStatus>,
    ) -> Result<DbId> {
        let evt = events::format_seq_account_evt(did, active, status)?;
        self.sequence_evt(evt).await
    }

    pub async fn sequence_sync_evt(
        &mut self,
        did: String,
        rev: String,
        blocks: BlockMap,
    ) -> Result<DbId> {
        let evt = events::format_seq_sync_evt(did, rev, blocks).await?;
        self.sequence_evt(evt).await
    }

    pub async fn delete_all_for_user(&self, did: &str) -> Result<()> {
        let did = did.to_string();
        repo_seq::Entity::delete_many()
            .filter(repo_seq::Column::Did.eq(did))
            .exec(&self.db)
            .await?;
        Ok(())
    }
}

pub fn typed_seq_evt(row: &repo_seq::Model) -> Result<SeqEvt> {
    let seq = row.seq.0.timestamp_ms() as i64;
    let time = events::format_offset_datetime(row.sequenced_at);
    let typed = match row.event_type.as_str() {
        "append" | "rebase" => {
            let evt: events::CommitEvt = serde_ipld_dagcbor::from_slice(&row.event)
                .map_err(|e| PdsError::internal("dagcbor decode commit", e))?;
            SeqEvt::TypedCommitEvt(Box::new(TypedCommitEvt {
                r#type: "commit".to_string(),
                seq,
                time,
                evt,
            }))
        }
        "sync" => {
            let evt: events::SyncEvt = serde_ipld_dagcbor::from_slice(&row.event)
                .map_err(|e| PdsError::internal("dagcbor decode sync", e))?;
            SeqEvt::TypedSyncEvt(TypedSyncEvt {
                r#type: "sync".to_string(),
                seq,
                time,
                evt,
            })
        }
        "identity" => {
            let evt: events::IdentityEvt = serde_ipld_dagcbor::from_slice(&row.event)
                .map_err(|e| PdsError::internal("dagcbor decode identity", e))?;
            SeqEvt::TypedIdentityEvt(TypedIdentityEvt {
                r#type: "identity".to_string(),
                seq,
                time,
                evt,
            })
        }
        "account" => {
            let evt: events::AccountEvt = serde_ipld_dagcbor::from_slice(&row.event)
                .map_err(|e| PdsError::internal("dagcbor decode account", e))?;
            SeqEvt::TypedAccountEvt(TypedAccountEvt {
                r#type: "account".to_string(),
                seq,
                time,
                evt,
            })
        }
        other => anyhow::bail!("invalid event type: {other}"),
    };
    Ok(typed)
}

pub fn repo_seq_from_row(row: &sea_orm::QueryResult) -> Result<repo_seq::Model> {
    let bytes: Vec<u8> = row.try_get_by_index(0)?;
    let seq = if bytes.len() == 16 {
        let mut arr = [0u8; 16];
        arr.copy_from_slice(&bytes);
        DbId::from_bytes(arr)
    } else {
        return Err(PdsError::internal(
            "invalid seq bytes",
            anyhow::anyhow!("expected 16 bytes, got {}", bytes.len()),
        )
        .into());
    };
    let did_str: String = row.try_get_by_index(1)?;
    let event_type: String = row.try_get_by_index(2)?;
    let event: Vec<u8> = row.try_get_by_index(3)?;
    let invalidated: Option<i16> = row.try_get_by_index(4)?;
    let sequenced_at_str: String = row.try_get_by_index(5)?;
    let format = time::macros::format_description!(
        "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z"
    );
    let sequenced_at = OffsetDateTime::parse(&sequenced_at_str, format)
        .or_else(|_| {
            OffsetDateTime::parse(
                &sequenced_at_str,
                &time::format_description::well_known::Rfc3339,
            )
        })
        .map_err(|e| PdsError::internal("sequencer parse OffsetDateTime", e))?;
    Ok(repo_seq::Model {
        seq,
        did: did_str.into(),
        event_type,
        event,
        invalidated,
        sequenced_at,
    })
}

// Have to make sure this import is used since sqlx is a workspace dep
#[allow(dead_code)]
fn _sqlx_pool_marker(_: &SqlitePool) {}

#[cfg(test)]
pub(crate) mod test_util {
    use super::*;
    use cacos_pds_core::db::DatabaseKind;
    use cacos_pds_core::db::tests::TestDatabaseKind;
    use crate::crawlers::Crawlers;

    pub async fn _test_sequencer() -> Sequencer {
        let db = DatabaseKind::Sequencer.open_test_db().await;
        Sequencer::new(
            db.clone(),
            Crawlers::new("pds.test".to_string(), vec![]),
            None,
            None,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cacos_pds_core::db::DatabaseKind;
    use cacos_pds_core::db::tests::TestDatabaseKind;
    use migration::types::did::Did;
    use crate::crawlers::Crawlers;
    use crate::events::{CommitEvt, now_offset};
    use sea_orm::EntityTrait;

    pub(crate) async fn test_sequencer() -> (Sequencer, cacos_pds_core::db::tests::TestDb) {
        let db = DatabaseKind::Sequencer.open_test_db().await;
        let seq = Sequencer::new(
            db.clone(),
            Crawlers::new("pds.test".to_string(), vec![]),
            None,
            None,
        );
        (seq, db)
    }

    fn make_commit_event_bytes() -> Vec<u8> {
        let evt = CommitEvt {
            rebase: false,
            too_big: false,
            repo: "did:plc:test".to_string(),
            commit: lexicon_cid::Cid::default(),
            prev: None,
            rev: "rev".to_string(),
            since: None,
            blocks: vec![1, 2, 3],
            ops: vec![],
            blobs: vec![],
            prev_data: None,
        };
        serde_ipld_dagcbor::to_vec(&evt).unwrap()
    }

    #[tokio::test]
    async fn repo_seq_new_uses_insert_defaults() {
        let (mut seq, _db) = test_sequencer().await;
        let evt = RepoSeqNew::new(
            Did::from("did:plc:test".to_string()),
            "identity".to_string(),
            make_commit_event_bytes(),
            now_offset(),
        );
        let inserted = seq.sequence_evt(evt).await.unwrap();
        let rows = repo_seq::Entity::find().all(&seq.db).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].seq, inserted);
        assert_eq!(rows[0].event_type, "identity");
        assert_eq!(rows[0].invalidated, Some(0));
    }

    #[tokio::test]
    async fn sequences_and_reads_events() {
        let (mut seq, _db) = test_sequencer().await;
        let did = Did::from("did:plc:alice".to_string());
        let evt1 = RepoSeqNew::new(
            did.clone(),
            "identity".to_string(),
            serde_ipld_dagcbor::to_vec(&events::IdentityEvt {
                did: "did:plc:alice".to_string(),
                handle: Some("alice.test".to_string()),
            })
            .unwrap(),
            now_offset(),
        );
        let evt2 = RepoSeqNew::new(
            did.clone(),
            "account".to_string(),
            serde_ipld_dagcbor::to_vec(&events::AccountEvt {
                did: "did:plc:alice".to_string(),
                active: true,
                status: None,
            })
            .unwrap(),
            now_offset(),
        );
        let s1 = seq.sequence_evt(evt1).await.unwrap();
        let s2 = seq.sequence_evt(evt2).await.unwrap();
        assert!(s1 < s2);

        let rows = seq
            .request_seq_range(RequestSeqRangeOpts {
                earliest_seq: None,
                latest_seq: None,
                earliest_time: None,
                limit: Some(10),
            })
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].event_type, "identity");
        assert_eq!(rows[1].event_type, "account");
    }

    #[tokio::test]
    async fn request_seq_range_records_seq_poll_timing() {
        cacos_pds_core::observability::metrics::init_metrics();
        let (mut seq, _db) = test_sequencer().await;
        let did = Did::from("did:plc:seqpoll".to_string());
        let evt = RepoSeqNew::new(
            did.clone(),
            "identity".to_string(),
            serde_ipld_dagcbor::to_vec(&events::IdentityEvt {
                did: "did:plc:seqpoll".to_string(),
                handle: Some("alice.test".to_string()),
            })
            .unwrap(),
            now_offset(),
        );
        seq.sequence_evt(evt).await.unwrap();
        let _ = seq
            .request_seq_range(RequestSeqRangeOpts {
                earliest_seq: None,
                latest_seq: None,
                earliest_time: None,
                limit: Some(10),
            })
            .await
            .unwrap();
        let snapshot = cacos_pds_core::observability::metrics::render();
        let needle = "cacos_timing_seconds_count{stage=\"seq_poll\"}";
        assert!(
            snapshot.contains(needle),
            "expected seq_poll stage sample: {snapshot}"
        );
    }

    #[tokio::test]
    async fn deletes_events_for_user() {
        let (mut seq, _db) = test_sequencer().await;
        let alice = Did::from("did:plc:alice".to_string());
        let bob = Did::from("did:plc:bob".to_string());
        for did in [&alice, &bob] {
            let evt = RepoSeqNew::new(
                did.clone(),
                "identity".to_string(),
                vec![1, 2, 3],
                now_offset(),
            );
            seq.sequence_evt(evt).await.unwrap();
        }
        seq.delete_all_for_user("did:plc:alice").await.unwrap();
        let rows = repo_seq::Entity::find().all(&seq.db).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].did, bob);
    }
}
