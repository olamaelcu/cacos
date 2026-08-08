//! SQL-backed repository storage on top of sea-orm.
//!
//! Every rusqlite `db.run(|conn| ...)` becomes a sea-orm entity query (or raw
//! statement where the entity API is awkward). The in-memory `BlockMap` cache
//! (read/write-through) is preserved.
//!
//! Sync wrappers: the `ReadableBlockstore` trait requires future objects to be
//! `Send + Sync`. Sea-orm's async API returns `Send` futures but not `Sync`
//! (the underlying sqlx pool internals hold non-Sync state). To bridge that we
//! dispatch each DB call through `tokio::spawn`, which detaches the non-Sync
//! inner future and yields a `JoinHandle` that IS `Send + Sync`. The async
//! block on the trait's side then only awaits the `JoinHandle`, which is
//! `Send + Sync`.

use crate::actor_store::db::{repo_block, repo_root};
use crate::error::{PdsError, Result};
use lexicon_cid::Cid;
use rsky_repo::block_map::{BlockMap, BlocksAndMissing};
use rsky_repo::cid_set::CidSet;
use rsky_repo::storage::CidAndRev;
use rsky_repo::storage::readable_blockstore::ReadableBlockstore;
use rsky_repo::storage::types::RepoStorage;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::RwLock;

/// `?,?,?,…` placeholders for a SQL `IN (...)` clause.
pub(crate) fn placeholders(len: usize) -> String {
    vec!["?"; len].join(",")
}

use crate::observability::metrics::{
    ACTOR_CACHE_HITS_TOTAL, ACTOR_CACHE_MISSES_TOTAL, COMMITS_TOTAL,
};

#[derive(Clone, Debug)]
pub struct SqlRepoReader {
    pub cache: Arc<RwLock<BlockMap>>,
    pub db: sea_orm::DatabaseConnection,
    pub now: String,
    pub did: String,
}

impl SqlRepoReader {
    pub fn new(did: String, now: Option<String>, db: sea_orm::DatabaseConnection) -> Self {
        let now = now.unwrap_or_else(rsky_common::now);
        SqlRepoReader {
            cache: Arc::new(RwLock::new(BlockMap::new())),
            db,
            now,
            did,
        }
    }

    /// Look up content for a CID via the cache, falling back to the DB.
    /// Returns `Ok(Some(bytes))` on hit, `Ok(None)` on miss.
    async fn get_bytes_impl(&self, cid: &Cid) -> Result<Option<Vec<u8>>> {
        let cached = {
            let cache_guard = self.cache.read().await;
            cache_guard.get(*cid).cloned()
        };
        if let Some(bytes) = cached {
            metrics::counter!(ACTOR_CACHE_HITS_TOTAL, "did" => self.did.clone()).increment(1);
            return Ok(Some(bytes));
        }
        metrics::counter!(ACTOR_CACHE_MISSES_TOTAL, "did" => self.did.clone()).increment(1);

        let row = repo_block::Entity::find_by_id(cid.to_string())
            .one(&self.db)
            .await
            .map_err(|e| {
                PdsError::internal(
                    "SqlRepoReader::get_bytes_impl: find_by_id failed",
                    anyhow::Error::from(e),
                )
            })?;
        match row {
            None => Ok(None),
            Some(model) => {
                let bytes = model.content;
                let mut cache_guard = self.cache.write().await;
                cache_guard.set(*cid, bytes.clone());
                Ok(Some(bytes))
            }
        }
    }

    /// Multi-cid variant. Splits into cached vs missing, queries missing via
    /// `is_in` in batches of 500, populates the cache.
    async fn get_blocks_impl(&self, cids: Vec<Cid>) -> Result<BlocksAndMissing> {
        let cached = {
            let mut cache_guard = self.cache.write().await;
            cache_guard.get_many(cids).map_err(|e| {
                PdsError::internal(
                    "SqlRepoReader::get_blocks_impl: BlockMap::get_many failed",
                    e,
                )
            })?
        };
        metrics::counter!(ACTOR_CACHE_HITS_TOTAL, "did" => self.did.clone())
            .increment(cached.blocks.size() as u64);
        metrics::counter!(ACTOR_CACHE_MISSES_TOTAL, "did" => self.did.clone())
            .increment(cached.missing.len() as u64);
        if cached.missing.is_empty() {
            return Ok(cached);
        }
        let missing_strings: Vec<String> = cached.missing.iter().map(|c| c.to_string()).collect();
        let mut missing = CidSet::new(Some(cached.missing));

        let mut blocks = BlockMap::new();
        for batch in missing_strings.chunks(500) {
            let rows = repo_block::Entity::find()
                .filter(repo_block::Column::Cid.is_in(batch.to_vec()))
                .all(&self.db)
                .await
                .map_err(|e| {
                    PdsError::internal(
                        "SqlRepoReader::get_blocks_impl: find().is_in failed",
                        anyhow::Error::from(e),
                    )
                })?;
            for model in rows {
                let cid = Cid::try_from(model.cid.as_str()).map_err(|e| {
                    PdsError::internal(
                        "SqlRepoReader::get_blocks_impl: Cid::try_from failed",
                        anyhow::Error::from(e),
                    )
                })?;
                blocks.set(cid, model.content);
                missing.delete(cid);
            }
        }

        {
            let mut cache_guard = self.cache.write().await;
            cache_guard.add_map(blocks.clone()).map_err(|e| {
                PdsError::internal(
                    "SqlRepoReader::get_blocks_impl: BlockMap::add_map failed",
                    e,
                )
            })?;
        }
        blocks.add_map(cached.blocks).map_err(|e| {
            PdsError::internal(
                "SqlRepoReader::get_blocks_impl: BlockMap::add_map(cached) failed",
                e,
            )
        })?;

        Ok(BlocksAndMissing {
            blocks,
            missing: missing.to_list(),
        })
    }
}

impl ReadableBlockstore for SqlRepoReader {
    fn get_bytes<'a>(
        &'a self,
        cid: &'a Cid,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Option<Vec<u8>>>> + Send + Sync + 'a>> {
        let self_clone = self.clone();
        let cid_owned = *cid;
        Box::pin(async move {
            let result = tokio::spawn(async move { self_clone.get_bytes_impl(&cid_owned).await })
                .await
                .map_err(|e| anyhow::anyhow!("SqlRepoReader::get_bytes: spawn failed: {e}"))??;
            Ok(result)
        })
    }

    fn has<'a>(
        &'a self,
        cid: Cid,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<bool>> + Send + Sync + 'a>> {
        let self_clone = self.clone();
        Box::pin(async move {
            let result = tokio::spawn(async move { self_clone.get_bytes_impl(&cid).await })
                .await
                .map_err(|e| anyhow::anyhow!("SqlRepoReader::has: spawn failed: {e}"))??;
            Ok(result.is_some())
        })
    }

    fn get_blocks<'a>(
        &'a self,
        cids: Vec<Cid>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<BlocksAndMissing>> + Send + Sync + 'a>> {
        let self_clone = self.clone();
        Box::pin(async move {
            let result = tokio::spawn(async move { self_clone.get_blocks_impl(cids).await })
                .await
                .map_err(|e| anyhow::anyhow!("SqlRepoReader::get_blocks: spawn failed: {e}"))??;
            Ok(result)
        })
    }
}

impl SqlRepoReader {
    /// Returns the (cid, rev) of the repo root, or `NotFound` when none.
    pub async fn get_root_detailed(&self) -> Result<CidAndRev> {
        let row = repo_root::Entity::find().one(&self.db).await.map_err(|e| {
            PdsError::internal(
                "SqlRepoReader::get_root_detailed: find().one failed",
                anyhow::Error::from(e),
            )
        })?;
        match row {
            None => Err(PdsError::NotFound("repo root".to_string())),
            Some(model) => {
                let cid = lexicon_cid::Cid::try_from(model.cid.as_str()).map_err(|e| {
                    PdsError::internal(
                        "SqlRepoReader::get_root_detailed: Cid::try_from failed",
                        anyhow::Error::from(e),
                    )
                })?;
                Ok(CidAndRev {
                    cid,
                    rev: model.rev,
                })
            }
        }
    }

    pub async fn list_existing_blocks(&self, cids: Vec<Cid>) -> Result<Vec<Cid>> {
        if cids.is_empty() {
            return Ok(vec![]);
        }

        let cid_strings: Vec<String> = cids.iter().map(ToString::to_string).collect();
        let mut existing = Vec::new();
        for batch in cid_strings.chunks(500) {
            let rows = repo_block::Entity::find()
                .filter(repo_block::Column::Cid.is_in(batch.to_vec()))
                .all(&self.db)
                .await
                .map_err(|e| {
                    PdsError::internal(
                        "SqlRepoReader::list_existing_blocks: find failed",
                        anyhow::Error::from(e),
                    )
                })?;
            for model in rows {
                let cid = Cid::try_from(model.cid.as_str()).map_err(|e| {
                    PdsError::internal(
                        "SqlRepoReader::list_existing_blocks: Cid::try_from failed",
                        anyhow::Error::from(e),
                    )
                })?;
                existing.push(cid);
            }
        }
        Ok(existing)
    }

    /// Delete a batch of blocks by cid (batched 500-at-a-time like the read path).
    /// Also evicts the entries from the local BlockMap cache so a subsequent
    /// `has()`/`get_bytes()` reflects the deletion immediately.
    pub async fn delete_many(&self, cids: Vec<lexicon_cid::Cid>) -> Result<()> {
        if cids.is_empty() {
            return Ok(());
        }
        let cid_strings: Vec<String> = cids.iter().map(|c| c.to_string()).collect();
        for batch in cid_strings.chunks(500) {
            repo_block::Entity::delete_many()
                .filter(repo_block::Column::Cid.is_in(batch.to_vec()))
                .exec(&self.db)
                .await
                .map_err(|e| {
                    PdsError::internal("SqlRepoReader::delete_many: failed", anyhow::Error::from(e))
                })?;
        }
        // Evict from the in-memory cache so subsequent reads reflect the
        // deletion. Our cache is the single source of truth for
        // `has()`/`get_bytes()`.
        let mut cache_guard = self.cache.write().await;
        for cid in &cids {
            cache_guard.map.remove(&cid.to_string());
        }
        Ok(())
    }

    async fn put_block_impl(
        &self,
        cid: lexicon_cid::Cid,
        bytes: Vec<u8>,
        rev: String,
    ) -> Result<()> {
        let size = bytes.len() as i64;
        let am = repo_block::ActiveModel {
            cid: sea_orm::Set(cid.to_string()),
            repo_rev: sea_orm::Set(rev),
            size: sea_orm::Set(size),
            content: sea_orm::Set(bytes.clone()),
        };
        repo_block::Entity::insert(am)
            .on_conflict_do_nothing()
            .exec_without_returning(&self.db)
            .await
            .map_err(|e| {
                PdsError::internal(
                    "SqlRepoReader::put_block_impl: insert failed",
                    anyhow::Error::from(e),
                )
            })?;
        let mut cache_guard = self.cache.write().await;
        cache_guard.set(cid, bytes);
        Ok(())
    }

    async fn put_many_impl(&self, to_put: BlockMap, rev: String) -> Result<()> {
        let entries: Vec<(String, Vec<u8>)> = to_put
            .map
            .iter()
            .map(|(cid_str, bytes)| (cid_str.clone(), bytes.0.clone()))
            .collect();
        for (cid_str, content) in entries {
            let size = content.len() as i64;
            let am = repo_block::ActiveModel {
                cid: sea_orm::Set(cid_str),
                repo_rev: sea_orm::Set(rev.clone()),
                size: sea_orm::Set(size),
                content: sea_orm::Set(content),
            };
            repo_block::Entity::insert(am)
                .on_conflict_do_nothing()
                .exec_without_returning(&self.db)
                .await
                .map_err(|e| {
                    PdsError::internal(
                        "SqlRepoReader::put_many_impl: insert failed",
                        anyhow::Error::from(e),
                    )
                })?;
        }
        // Note: put_many does NOT populate the cache. Match that.
        Ok(())
    }

    async fn update_root_impl(
        &self,
        cid: lexicon_cid::Cid,
        rev: String,
        is_create: Option<bool>,
    ) -> Result<()> {
        let now_dt =
            time::OffsetDateTime::parse(&self.now, &time::format_description::well_known::Rfc3339)
                .unwrap_or_else(|_| time::OffsetDateTime::now_utc());
        let did = migration::types::did::Did::new(self.did.clone());
        let cid_string = cid.to_string();
        let is_create = is_create.unwrap_or(false);
        if is_create {
            let am = repo_root::ActiveModel {
                did: sea_orm::Set(did),
                cid: sea_orm::Set(cid_string),
                rev: sea_orm::Set(rev),
                indexed_at: sea_orm::Set(now_dt),
            };
            repo_root::Entity::insert(am)
                .exec(&self.db)
                .await
                .map_err(|e| {
                    PdsError::internal(
                        "SqlRepoReader::update_root_impl: insert (create) failed",
                        anyhow::Error::from(e),
                    )
                })?;
        } else {
            let am = repo_root::ActiveModel {
                did: sea_orm::Set(did),
                cid: sea_orm::Set(cid_string),
                rev: sea_orm::Set(rev),
                indexed_at: sea_orm::Set(now_dt),
            };
            repo_root::Entity::update(am)
                .exec(&self.db)
                .await
                .map_err(|e| {
                    PdsError::internal(
                        "SqlRepoReader::update_root_impl: update failed",
                        anyhow::Error::from(e),
                    )
                })?;
        }
        Ok(())
    }

    /// `root + put_many + delete_many` run inside one sea-orm transaction. A
    /// mid-commit failure rolls back the whole commit. The in-memory BlockMap
    /// cache is not populated here.
    async fn apply_commit_impl(
        &self,
        commit: rsky_repo::types::CommitData,
        is_create: Option<bool>,
    ) -> Result<()> {
        let now_str = self.now.clone();
        let did = self.did.clone();
        let is_create = is_create.unwrap_or(false);
        let cid_string = commit.cid.to_string();
        let rev = commit.rev.clone();
        let blocks: Vec<(String, Vec<u8>)> = commit
            .new_blocks
            .map
            .iter()
            .map(|(cid_str, bytes)| (cid_str.clone(), bytes.0.clone()))
            .collect();
        let removed: Vec<String> = commit
            .removed_cids
            .to_list()
            .iter()
            .map(|c| c.to_string())
            .collect();

        // Pre-compute now_dt OUTSIDE the transaction closure.
        let now_dt =
            time::OffsetDateTime::parse(&now_str, &time::format_description::well_known::Rfc3339)
                .unwrap_or_else(|_| time::OffsetDateTime::now_utc());

        // Use the AFIT `transaction_async` to avoid the boxed-dyn shenanigans
        // the trait form forces. The error type is `TransactionError<PdsError>`;
        // map it back to `PdsError` so the surrounding fn returns the project's
        // canonical error.
        self.db
            .transaction_async::<_, (), PdsError>(async move |txn| {
                let did = migration::types::did::Did::new(did);
                if is_create {
                    let am = repo_root::ActiveModel {
                        did: sea_orm::Set(did),
                        cid: sea_orm::Set(cid_string),
                        rev: sea_orm::Set(rev.clone()),
                        indexed_at: sea_orm::Set(now_dt),
                    };
                    repo_root::Entity::insert(am).exec(txn).await.map_err(|e| {
                        PdsError::internal(
                            "apply_commit: root insert (create) failed",
                            anyhow::Error::from(e),
                        )
                    })?;
                } else {
                    let am = repo_root::ActiveModel {
                        did: sea_orm::Set(did),
                        cid: sea_orm::Set(cid_string),
                        rev: sea_orm::Set(rev.clone()),
                        indexed_at: sea_orm::Set(now_dt),
                    };
                    repo_root::Entity::update(am).exec(txn).await.map_err(|e| {
                        PdsError::internal(
                            "apply_commit: root update failed",
                            anyhow::Error::from(e),
                        )
                    })?;
                }
                for (cid_str, content) in &blocks {
                    let size = content.len() as i64;
                    let am = repo_block::ActiveModel {
                        cid: sea_orm::Set(cid_str.clone()),
                        repo_rev: sea_orm::Set(rev.clone()),
                        size: sea_orm::Set(size),
                        content: sea_orm::Set(content.clone()),
                    };
                    repo_block::Entity::insert(am)
                        .on_conflict_do_nothing()
                        .exec_without_returning(txn)
                        .await
                        .map_err(|e| {
                            PdsError::internal(
                                "apply_commit: block insert failed",
                                anyhow::Error::from(e),
                            )
                        })?;
                }
                for batch in removed.chunks(500) {
                    repo_block::Entity::delete_many()
                        .filter(repo_block::Column::Cid.is_in(batch.to_vec()))
                        .exec(txn)
                        .await
                        .map_err(|e| {
                            PdsError::internal(
                                "apply_commit: block delete failed",
                                anyhow::Error::from(e),
                            )
                        })?;
                }
                metrics::counter!(COMMITS_TOTAL).increment(1);
                Ok(())
            })
            .await
            .map_err(|te| match te {
                sea_orm::TransactionError::Connection(d) => PdsError::Database(d),
                sea_orm::TransactionError::Transaction(p) => p,
            })?;

        // Cache eviction only happens AFTER the commit succeeds, so a rolled-back
        // commit leaves the cache intact.
        let cid_keys: Vec<String> = commit
            .removed_cids
            .to_list()
            .iter()
            .map(|c| c.to_string())
            .collect();
        {
            let mut cache_guard = self.cache.write().await;
            for key in cid_keys {
                cache_guard.map.remove(&key);
            }
        }
        Ok(())
    }

    /// Stream the repository as a CAR file. Returns the CAR-encoded bytes
    /// starting from the optional root CID; if None, returns the blocks but
    /// not a root.
    pub async fn get_car_stream(&self, root_cid: Option<lexicon_cid::Cid>) -> Result<Vec<u8>> {
        let blocks = self.get_block_range(&None, &None).await?;
        let block_map = {
            let mut bm = BlockMap::new();
            for model in blocks {
                let cid = lexicon_cid::Cid::try_from(model.cid.as_str()).map_err(|e| {
                    PdsError::internal(
                        "SqlRepoReader::get_car_stream: Cid::try_from failed",
                        anyhow::Error::from(e),
                    )
                })?;
                bm.set(cid, model.content);
            }
            bm
        };
        rsky_repo::car::blocks_to_car_file(root_cid.as_ref(), block_map)
            .await
            .map_err(|e| {
                PdsError::internal(
                    "SqlRepoReader::get_car_stream: blocks_to_car_file failed",
                    e,
                )
            })
    }

    /// Keyset paginated block retrieval. After-cursor takes priority: if
    /// `after_rev` is Some, walk backwards from (after_rev, after_cid?).
    pub async fn get_block_range(
        &self,
        after_rev: &Option<String>,
        after_cid: &Option<rsky_repo::storage::CidAndRev>,
    ) -> Result<Vec<crate::actor_store::db::repo_block::Model>> {
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};

        let mut query = crate::actor_store::db::repo_block::Entity::find();

        if let Some(rev) = after_rev {
            let cid_str = after_cid.as_ref().map(|cr| cr.cid.to_string());
            // Keyset pagination: ("repoRev", cid) < (?, ?) → rev < ? OR (rev = ? AND cid < ?)
            query = match cid_str {
                Some(c) => query.filter(
                    sea_orm::Condition::any()
                        .add(sea_orm::Condition::all().add(
                            crate::actor_store::db::repo_block::Column::RepoRev.lt(rev.clone()),
                        ))
                        .add(
                            sea_orm::Condition::all()
                                .add(
                                    crate::actor_store::db::repo_block::Column::RepoRev
                                        .eq(rev.clone()),
                                )
                                .add(crate::actor_store::db::repo_block::Column::Cid.lt(c)),
                        ),
                ),
                None => query
                    .filter(crate::actor_store::db::repo_block::Column::RepoRev.lt(rev.clone())),
            };
        }

        let rows = query
            .order_by_desc(crate::actor_store::db::repo_block::Column::RepoRev)
            .order_by_desc(crate::actor_store::db::repo_block::Column::Cid)
            .limit(500)
            .all(&self.db)
            .await
            .map_err(|e| {
                PdsError::internal(
                    "SqlRepoReader::get_block_range: find failed",
                    anyhow::Error::from(e),
                )
            })?;

        Ok(rows)
    }

    pub async fn count_blocks(&self) -> Result<usize> {
        use sea_orm::PaginatorTrait;
        let count = crate::actor_store::db::repo_block::Entity::find()
            .count(&self.db)
            .await
            .map_err(|e| {
                PdsError::internal(
                    "SqlRepoReader::count_blocks: find().count failed",
                    anyhow::Error::from(e),
                )
            })?;
        Ok(count as usize)
    }

    /// Load up to 15 blocks with the given `repo_rev` from the DB into the
    /// in-memory `BlockMap` cache.
    pub async fn cache_rev(&self, rev: String) -> Result<()> {
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};

        let rows = crate::actor_store::db::repo_block::Entity::find()
            .filter(crate::actor_store::db::repo_block::Column::RepoRev.eq(rev))
            .limit(15)
            .all(&self.db)
            .await
            .map_err(|e| {
                PdsError::internal(
                    "SqlRepoReader::cache_rev: find failed",
                    anyhow::Error::from(e),
                )
            })?;

        let mut cache_guard = self.cache.write().await;
        for model in rows {
            let cid = lexicon_cid::Cid::try_from(model.cid.as_str()).map_err(|e| {
                PdsError::internal(
                    "SqlRepoReader::cache_rev: Cid::try_from failed",
                    anyhow::Error::from(e),
                )
            })?;
            cache_guard.set(cid, model.content);
        }
        Ok(())
    }
}

impl RepoStorage for SqlRepoReader {
    fn get_root<'a>(
        &'a self,
    ) -> Pin<
        Box<
            dyn std::future::Future<Output = std::option::Option<lexicon_cid::Cid>>
                + Send
                + Sync
                + 'a,
        >,
    > {
        Box::pin(async move {
            let reader = self.clone();
            tokio::spawn(async move { reader.get_root_detailed().await })
                .await
                .ok()
                .and_then(|r| r.ok())
                .map(|r| r.cid)
        })
    }

    fn put_block<'a>(
        &'a self,
        cid: lexicon_cid::Cid,
        bytes: Vec<u8>,
        rev: String,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + Sync + 'a>> {
        Box::pin(async move {
            let reader = self.clone();
            tokio::spawn(async move { reader.put_block_impl(cid, bytes, rev).await })
                .await
                .map_err(|e| {
                    anyhow::Error::from(PdsError::internal(
                        "put_block: join failed",
                        anyhow::Error::from(e),
                    ))
                })?
                .map_err(anyhow::Error::from)
        })
    }

    fn put_many<'a>(
        &'a self,
        to_put: BlockMap,
        rev: String,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + Sync + 'a>> {
        Box::pin(async move {
            let reader = self.clone();
            tokio::spawn(async move { reader.put_many_impl(to_put, rev).await })
                .await
                .map_err(|e| {
                    anyhow::Error::from(PdsError::internal(
                        "put_many: join failed",
                        anyhow::Error::from(e),
                    ))
                })?
                .map_err(anyhow::Error::from)
        })
    }

    fn update_root<'a>(
        &'a self,
        cid: lexicon_cid::Cid,
        rev: String,
        is_create: Option<bool>,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + Sync + 'a>> {
        Box::pin(async move {
            let reader = self.clone();
            tokio::spawn(async move { reader.update_root_impl(cid, rev, is_create).await })
                .await
                .map_err(|e| {
                    anyhow::Error::from(PdsError::internal(
                        "update_root: join failed",
                        anyhow::Error::from(e),
                    ))
                })?
                .map_err(anyhow::Error::from)
        })
    }

    fn apply_commit<'a>(
        &'a self,
        commit: rsky_repo::types::CommitData,
        is_create: Option<bool>,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + Sync + 'a>> {
        Box::pin(async move {
            let reader = self.clone();
            tokio::spawn(async move { reader.apply_commit_impl(commit, is_create).await })
                .await
                .map_err(|e| {
                    anyhow::Error::from(PdsError::internal(
                        "apply_commit: join failed",
                        anyhow::Error::from(e),
                    ))
                })?
                .map_err(anyhow::Error::from)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use metrics_exporter_prometheus::PrometheusBuilder;

    fn cid_for(value: &[u8]) -> Cid {
        use sha2::{Digest, Sha256};
        rsky_common::ipld::sha256_to_cid(Sha256::digest(value).to_vec())
    }

    async fn test_reader() -> (camino_tempfile::Utf8TempDir, SqlRepoReader) {
        let dir = camino_tempfile::Utf8TempDir::new().unwrap();
        let db = crate::db::DatabaseKind::Actor
            .open(dir.path().join("store.sqlite"))
            .await
            .unwrap();
        let reader = SqlRepoReader::new("did:example:alice".to_owned(), None, db);
        (dir, reader)
    }

    /// Seeds a block row directly (the RepoStorage `put_block` lands with
    /// Plan 03's repo-storage work). Idempotent via ON CONFLICT DO NOTHING.
    async fn seed_block(reader: &SqlRepoReader, bytes: &[u8], rev: &str) -> Cid {
        use sea_orm::Set;
        let cid = cid_for(bytes);
        let am = repo_block::ActiveModel {
            cid: Set(cid.to_string()),
            repo_rev: Set(rev.to_owned()),
            size: Set(bytes.len() as i64),
            content: Set(bytes.to_vec()),
        };
        // Idempotent: ON CONFLICT DO NOTHING (entity conflict on primary key).
        let _ = repo_block::Entity::insert(am)
            .on_conflict_do_nothing()
            .exec_without_returning(&reader.db)
            .await
            .unwrap();
        cid
    }

    #[tokio::test]
    async fn list_existing_blocks_returns_only_present_cids() {
        let (_dir, reader) = test_reader().await;
        let present = cid_for(b"present");
        let missing = cid_for(b"missing");
        reader
            .put_block(present, b"present".to_vec(), "rev-1".to_owned())
            .await
            .unwrap();

        let existing = reader
            .list_existing_blocks(vec![present, missing])
            .await
            .unwrap();

        assert_eq!(existing, vec![present]);
    }

    #[tokio::test]
    async fn get_bytes_cache_and_get_blocks() {
        let (_dir, reader) = test_reader().await;
        let bytes = b"block-one".to_vec();
        let cid = seed_block(&reader, &bytes, "rev-1").await;
        // seeding the same cid again is idempotent
        seed_block(&reader, &bytes, "rev-1").await;
        assert_eq!(reader.get_bytes(&cid).await.unwrap(), Some(bytes.clone()));
        // cached path
        assert_eq!(reader.get_bytes(&cid).await.unwrap(), Some(bytes.clone()));
        assert!(reader.has(cid).await.unwrap());
        let missing_cid = cid_for(b"missing");
        assert!(!reader.has(missing_cid).await.unwrap());

        let got = reader.get_blocks(vec![cid, missing_cid]).await.unwrap();
        assert_eq!(got.blocks.get(cid), Some(&bytes));
        assert_eq!(got.missing, vec![missing_cid]);
    }

    #[tokio::test]
    async fn fresh_reader_fetches_from_db_and_caches() {
        let (_dir, reader) = test_reader().await;
        let bytes = b"persisted".to_vec();
        let cid = seed_block(&reader, &bytes, "rev-1").await;

        // a second reader over the same db starts with a cold cache
        let fresh = SqlRepoReader::new(reader.did.clone(), None, reader.db.clone());
        assert_eq!(fresh.get_bytes(&cid).await.unwrap(), Some(bytes.clone()));

        // a third reader exercises the multi-block db path, then the all-cached path
        let fresh_two = SqlRepoReader::new(reader.did.clone(), None, reader.db.clone());
        let first = fresh_two.get_blocks(vec![cid]).await.unwrap();
        assert!(first.missing.is_empty());
        let second = fresh_two.get_blocks(vec![cid]).await.unwrap();
        assert!(second.missing.is_empty());
        assert_eq!(second.blocks.get(cid), Some(&bytes));
    }

    #[test]
    fn cache_hit_miss_counters() {
        // We use a sync `#[test]` with a hand-built current_thread runtime
        // because the impl methods are async and we need to wrap the whole
        // future in `metrics::with_local_recorder` (a sync closure). The trait
        // methods would `tokio::spawn` to make their futures `Sync`, which
        // moves the work off-thread and busts the thread-local recorder scope,
        // so we call the impl methods directly here.
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (_dir, reader) = rt.block_on(test_reader());
        let bytes = b"counted".to_vec();
        let cid = rt.block_on(seed_block(&reader, &bytes, "rev-1"));

        let missing = cid_for(b"nope");
        metrics::with_local_recorder(&recorder, || {
            rt.block_on(async {
                // first read: cache miss, then db hit -> misses += 1
                let _ = reader.get_bytes_impl(&cid).await.unwrap();
                // second read: served from the in-memory BlockMap cache -> hits += 1
                let _ = reader.get_bytes_impl(&cid).await.unwrap();
                // a never-stored cid: cache miss + db miss -> misses += 1
                let _ = reader.get_bytes_impl(&missing).await.unwrap();
            });
        });
        let out = handle.render();
        assert!(
            out.contains("cacos_actor_cache_hits_total"),
            "cache hits counter missing:\n{out}"
        );
        assert!(
            out.contains("cacos_actor_cache_misses_total"),
            "cache misses counter missing:\n{out}"
        );
    }

    #[tokio::test]
    async fn root_lifecycle() {
        let (_dir, reader) = test_reader().await;
        assert!(reader.get_root().await.is_none());
        assert!(reader.get_root_detailed().await.is_err());

        let root_one = cid_for(b"root-one");
        reader
            .update_root(root_one, "rev-1".to_owned(), Some(true))
            .await
            .unwrap();
        assert_eq!(reader.get_root().await, Some(root_one));

        let root_two = cid_for(b"root-two");
        reader
            .update_root(root_two, "rev-2".to_owned(), None)
            .await
            .unwrap();
        let detailed = reader.get_root_detailed().await.unwrap();
        assert_eq!(detailed.cid, root_two);
        assert_eq!(detailed.rev, "rev-2");
    }

    #[tokio::test]
    async fn apply_commit_writes_and_removes() {
        let (_dir, reader) = test_reader().await;
        let removed = b"removed".to_vec();
        let removed_cid = cid_for(&removed);
        reader
            .put_block(removed_cid, removed, "rev-1".to_owned())
            .await
            .unwrap();
        reader
            .update_root(removed_cid, "rev-1".to_owned(), Some(true))
            .await
            .unwrap();

        let added = b"added".to_vec();
        let added_cid = cid_for(&added);
        let mut new_blocks = BlockMap::new();
        new_blocks.set(added_cid, added.clone());
        let commit = rsky_repo::types::CommitData {
            cid: added_cid,
            rev: "rev-2".to_owned(),
            since: Some("rev-1".to_owned()),
            prev: Some(removed_cid),
            new_blocks,
            relevant_blocks: BlockMap::new(),
            removed_cids: CidSet::new(Some(vec![removed_cid])),
        };
        reader.apply_commit(commit, None).await.unwrap();

        let detailed = reader.get_root_detailed().await.unwrap();
        assert_eq!(detailed.rev, "rev-2");
        assert!(!reader.has(removed_cid).await.unwrap());
        assert!(reader.has(added_cid).await.unwrap());
    }

    #[tokio::test]
    async fn apply_commit_rolls_back_on_root_conflict() {
        let (_dir, reader) = test_reader().await;
        let root_one = cid_for(b"root-one");
        reader
            .update_root(root_one, "rev-1".to_owned(), Some(true))
            .await
            .unwrap();

        // A create-commit whose root INSERT conflicts with the existing
        // (did, PRIMARY KEY) row must fail AND leave no partial state behind.
        // The transactional apply_commit guarantees rollback, and this test
        // pins that guarantee.
        let new_root = cid_for(b"new-root");
        let mut new_blocks = BlockMap::new();
        new_blocks.set(new_root, b"new-block".to_vec());
        let commit = rsky_repo::types::CommitData {
            cid: new_root,
            rev: "rev-2".to_owned(),
            since: Some("rev-1".to_owned()),
            prev: Some(root_one),
            new_blocks,
            relevant_blocks: BlockMap::new(),
            removed_cids: CidSet::new(None),
        };
        assert!(reader.apply_commit(commit, Some(true)).await.is_err());

        // root unchanged
        let detailed = reader.get_root_detailed().await.unwrap();
        assert_eq!(detailed.rev, "rev-1");
        // the block from the failed commit was rolled back with the transaction
        assert!(!reader.has(new_root).await.unwrap());
    }

    #[tokio::test]
    async fn put_many_and_delete_many() {
        let (_dir, reader) = test_reader().await;
        let a = b"alpha".to_vec();
        let b = b"bravo".to_vec();
        let a_cid = cid_for(&a);
        let b_cid = cid_for(&b);
        let mut bm = BlockMap::new();
        bm.set(a_cid, a.clone());
        bm.set(b_cid, b.clone());
        reader.put_many(bm, "rev-1".to_owned()).await.unwrap();

        // After put_many the DB has both blocks but the cache is unchanged.
        assert!(reader.has(a_cid).await.unwrap());
        assert!(reader.has(b_cid).await.unwrap());

        reader.delete_many(vec![a_cid]).await.unwrap();

        // delete_many should evict from the cache too.
        let cache = reader.cache.read().await;
        assert!(cache.get(a_cid).is_none());
        drop(cache);

        // a is gone, b remains.
        assert!(!reader.has(a_cid).await.unwrap());
        assert!(reader.has(b_cid).await.unwrap());
    }

    #[tokio::test]
    async fn cache_rev_populates_cache() {
        let (_dir, reader) = test_reader().await;
        let rev = "rev-cache-test";
        let bytes = b"cached-block".to_vec();
        let cid = seed_block(&reader, &bytes, rev).await;

        // fresh reader with empty cache.
        let fresh = SqlRepoReader::new(reader.did.clone(), None, reader.db.clone());
        assert!(fresh.cache.read().await.get(cid).is_none());

        fresh.cache_rev(rev.to_owned()).await.unwrap();

        let cache = fresh.cache.read().await;
        assert_eq!(cache.get(cid), Some(&bytes));
    }

    #[tokio::test]
    async fn car_stream_and_block_range() {
        let (_dir, reader) = test_reader().await;
        // Seed 3 blocks under one rev.
        for content in [b"one" as &[u8], b"two", b"three"] {
            seed_block(&reader, content, "rev-A").await;
        }
        // Different rev for the 4th.
        seed_block(&reader, b"four", "rev-B").await;

        let count = reader.count_blocks().await.unwrap();
        assert_eq!(count, 4);

        let blocks = reader.get_block_range(&None, &None).await.unwrap();
        assert_eq!(blocks.len(), 4);

        let car = reader.get_car_stream(None).await.unwrap();
        assert!(!car.is_empty());
    }

    #[test]
    fn apply_commit_records_commits_counter() {
        // Sync-test + current_thread runtime, same pattern as
        // `cache_hit_miss_counters`: the trait body `tokio::spawn`s onto a
        // worker thread, which would lose the thread-local recorder scope.
        // Calling the impl method directly keeps the counter increment inside
        // the recorded thread.
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (_dir, reader) = rt.block_on(test_reader());

        let root_cid = cid_for(b"root");
        let mut new_blocks = BlockMap::new();
        new_blocks.set(root_cid, b"root-block".to_vec());
        let commit = rsky_repo::types::CommitData {
            cid: root_cid,
            rev: "rev-1".to_owned(),
            since: None,
            prev: None,
            new_blocks,
            relevant_blocks: BlockMap::new(),
            removed_cids: CidSet::new(None),
        };

        metrics::with_local_recorder(&recorder, || {
            rt.block_on(async {
                reader.apply_commit_impl(commit, Some(true)).await.unwrap();
            });
        });
        let out = handle.render();
        assert!(
            out.contains("cacos_commits_total 1"),
            "expected commits counter in: {out}"
        );
    }
}
