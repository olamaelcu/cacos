//! SQLite-backed DID document cache.
//!
//! Mirrors `rsky_identity::types::DidCache` so the cacos PDS can swap the
//! in-process `MemoryCache` for a persisted implementation backed by the
//! `did_cache` SQLite database. Cache writes triggered by stale reads are
//! dispatched on the [`cacos_pds_core::background::BackgroundQueue`] passed to
//! [`DidSqliteCache::new`]; cache failures are logged and swallowed so
//! resolution never depends on the cache being healthy.
//!
//! **Time unit:** `CacheResult::updated_at` is reported in **microseconds**
//! since the Unix epoch, matching `rsky_identity::cache::MemoryCache` (which
//! uses `SystemTime::now().duration_since(UNIX_EPOCH).as_micros()`). The
//! underlying `did_doc` table stores `OffsetDateTime` via the typed entity
//! column; the cache converts to/from micros at the boundary.

use anyhow::Result;
use cacos_pds_core::background::BackgroundQueue;
use migration::entities::did_doc;
use migration::types::did::Did;
use rsky_identity::types::{CacheResult, DidCache, DidDocument, GetDocFn};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set, TransactionTrait};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct DidSqliteCache {
    db: sea_orm::DatabaseConnection,
    background_queue: BackgroundQueue,
    stale_ttl: Duration,
    max_ttl: Duration,
}

impl DidSqliteCache {
    pub fn new(
        db: sea_orm::DatabaseConnection,
        background_queue: BackgroundQueue,
        stale_ttl: Duration,
        max_ttl: Duration,
    ) -> Self {
        Self {
            db,
            background_queue,
            stale_ttl,
            max_ttl,
        }
    }

    /// Test seam: waits for any queued background refetches to settle.
    pub async fn process_all(&self) {
        self.background_queue.process_all().await;
    }
}

fn now_micros() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("timestamp in micros since UNIX epoch")
        .as_micros()
}

#[async_trait::async_trait]
impl DidCache for DidSqliteCache {
    async fn cache_did(&self, did: String, doc: DidDocument) -> Result<()> {
        let doc = serde_json::to_string(&doc)?;
        let now = now_micros();
        let did_typed: Did = Did::from(did.clone());
        let now_secs = (now / 1_000_000) as i64;
        let nanos = ((now % 1_000_000) as i32) * 1000;
        let now = time::OffsetDateTime::from_unix_timestamp_nanos(
            now_secs as i128 * 1_000_000_000 + nanos as i128,
        )
        .unwrap_or_else(|_| time::OffsetDateTime::now_utc());
        let model = did_doc::ActiveModel {
            did: Set(did_typed.clone()),
            doc: Set(doc.clone()),
            updated_at: Set(now),
        };
        let insert = did_doc::Entity::insert(model).exec(&self.db).await;
        if insert.is_err() {
            let active = did_doc::ActiveModel {
                did: Set(did_typed),
                doc: Set(doc),
                updated_at: Set(now),
            };
            let _ = active.update(&self.db).await;
        }
        Ok(())
    }

    async fn refresh_cache(&self, did: String, get_doc: GetDocFn) -> Result<()> {
        let cache = self.clone();
        self.background_queue.add(async move {
            match get_doc().await {
                Ok(Some(doc)) => cache.cache_did(did, doc).await,
                Ok(None) => cache.clear_entry(did).await,
                Err(err) => {
                    tracing::error!(%did, ?err, "refreshing did cache failed");
                    Ok(())
                }
            }
        });
        Ok(())
    }

    async fn check_cache(&self, did: String) -> Result<Option<CacheResult>> {
        let row = did_doc::Entity::find()
            .filter(did_doc::Column::Did.eq(Did::from(did.clone())))
            .one(&self.db)
            .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let doc: DidDocument = serde_json::from_str(&row.doc)?;
        let now = now_micros();
        // The `OffsetDateTime` round-trips to microseconds; subtract the
        // unix epoch to get µs.
        let updated_at = (row.updated_at.unix_timestamp_nanos() / 1000) as u128;
        let expired = now > updated_at + self.max_ttl.as_micros();
        let stale = now > updated_at + self.stale_ttl.as_micros();
        Ok(Some(CacheResult {
            did,
            doc,
            updated_at,
            stale,
            expired,
        }))
    }

    async fn clear_entry(&self, did: String) -> Result<()> {
        let did_typed: Did = Did::from(did);
        did_doc::Entity::delete_many()
            .filter(did_doc::Column::Did.eq(did_typed))
            .exec(&self.db)
            .await?;
        Ok(())
    }

    async fn clear(&self) -> Result<()> {
        let tx = self.db.begin().await?;
        did_doc::Entity::delete_many().exec(&tx).await?;
        tx.commit().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsky_identity::types::DidDocument;
    use std::time::Duration;

    fn doc(did: &str) -> DidDocument {
        DidDocument {
            context: None,
            id: did.to_owned(),
            also_known_as: Some(vec![format!("at://{did}.example.com")]),
            verification_method: None,
            service: None,
        }
    }

    /// Opens a migrated `did_cache` database.
    ///
    /// `cacos-pds` reaches this through `crate::db::DatabaseKind::DidCache`,
    /// which also applies the shared connection options (WAL, busy timeout).
    /// This crate has no connection-options layer, so tests connect directly
    /// and run the same migrator.
    async fn open_did_cache_db(path: impl std::fmt::Display) -> sea_orm::DatabaseConnection {
        use migration::{MigratorTrait, migrator::DidCacheMigrator};

        let db = sea_orm::Database::connect(format!("sqlite://{path}?mode=rwc"))
            .await
            .unwrap();
        DidCacheMigrator::up(&db, None).await.unwrap();
        db
    }

    async fn cache_with_ttls(
        stale_ttl: Duration,
        max_ttl: Duration,
    ) -> (camino_tempfile::Utf8TempDir, DidSqliteCache) {
        let dir = camino_tempfile::Utf8TempDir::new().unwrap();
        let db = open_did_cache_db(dir.path().join("did_cache.sqlite")).await;
        let cache = DidSqliteCache::new(db, BackgroundQueue::default(), stale_ttl, max_ttl);
        (dir, cache)
    }

    #[tokio::test]
    #[allow(non_snake_case)]
    async fn caches_and_returns_fresh_docs() {
        let (_dir, cache) =
            cache_with_ttls(Duration::from_secs(3600), Duration::from_secs(86400)).await;
        assert!(
            cache
                .check_cache("did:example:alice".to_owned())
                .await
                .unwrap()
                .is_none()
        );
        cache
            .cache_did("did:example:alice".to_owned(), doc("did:example:alice"))
            .await
            .unwrap();
        let result = cache
            .check_cache("did:example:alice".to_owned())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.doc.id, "did:example:alice");
        assert!(!result.stale);
        assert!(!result.expired);
    }

    #[tokio::test]
    async fn reports_stale_and_expired_entries() {
        let (_dir, stale) =
            cache_with_ttls(Duration::from_millis(0), Duration::from_secs(86400)).await;
        stale
            .cache_did("did:example:bob".to_owned(), doc("did:example:bob"))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
        let result = stale
            .check_cache("did:example:bob".to_owned())
            .await
            .unwrap()
            .unwrap();
        assert!(result.stale);
        assert!(!result.expired);

        let (_dir2, expired) =
            cache_with_ttls(Duration::from_millis(0), Duration::from_millis(0)).await;
        expired
            .cache_did("did:example:bob".to_owned(), doc("did:example:bob"))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
        let result = expired
            .check_cache("did:example:bob".to_owned())
            .await
            .unwrap()
            .unwrap();
        assert!(result.stale);
        assert!(result.expired);
    }

    #[tokio::test]
    async fn refresh_cache_updates_and_clears_in_background() {
        let (_dir, cache) =
            cache_with_ttls(Duration::from_secs(3600), Duration::from_secs(86400)).await;
        cache
            .refresh_cache(
                "did:example:carol".to_owned(),
                Box::new(|| Box::pin(async { Ok(Some(doc("did:example:carol"))) })),
            )
            .await
            .unwrap();
        cache.process_all().await;
        assert!(
            cache
                .check_cache("did:example:carol".to_owned())
                .await
                .unwrap()
                .is_some()
        );

        cache
            .refresh_cache(
                "did:example:carol".to_owned(),
                Box::new(|| Box::pin(async { Ok(None) })),
            )
            .await
            .unwrap();
        cache.process_all().await;
        assert!(
            cache
                .check_cache("did:example:carol".to_owned())
                .await
                .unwrap()
                .is_none()
        );

        cache
            .refresh_cache(
                "did:example:carol".to_owned(),
                Box::new(|| Box::pin(async { anyhow::bail!("resolution failed") })),
            )
            .await
            .unwrap();
        cache.process_all().await;
        assert!(
            cache
                .check_cache("did:example:carol".to_owned())
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn clears_entries_and_all() {
        let (_dir, cache) =
            cache_with_ttls(Duration::from_secs(3600), Duration::from_secs(86400)).await;
        cache
            .cache_did("did:example:a".to_owned(), doc("did:example:a"))
            .await
            .unwrap();
        cache
            .cache_did("did:example:b".to_owned(), doc("did:example:b"))
            .await
            .unwrap();
        cache.clear_entry("did:example:a".to_owned()).await.unwrap();
        assert!(
            cache
                .check_cache("did:example:a".to_owned())
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            cache
                .check_cache("did:example:b".to_owned())
                .await
                .unwrap()
                .is_some()
        );
        cache.clear().await.unwrap();
        assert!(
            cache
                .check_cache("did:example:b".to_owned())
                .await
                .unwrap()
                .is_none()
        );
    }
}
