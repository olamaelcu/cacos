//! Per-actor preferences (app.bsky.actor.{get,put}Preferences).
//!
//! `account_pref` rows are unique on `name` within the per-actor DB
//! (one DID per actor DB), so `put` upserts by name.

use crate::db::account_pref;
use cacos_pds_core::error::{PdsError, Result};
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, Set,
    sea_query::OnConflict,
};

#[derive(Clone)]
pub struct PreferenceReader {
    pub did: String,
    pub db: DatabaseConnection,
}

impl PreferenceReader {
    pub fn new(did: String, db: DatabaseConnection) -> Self {
        Self { did, db }
    }

    pub async fn get(&self, name: &str) -> Result<Option<serde_json::Value>> {
        let row = account_pref::Entity::find()
            .filter(account_pref::Column::Name.eq(name))
            .one(&self.db)
            .await
            .map_err(|e| PdsError::internal("PreferenceReader::get", anyhow::Error::from(e)))?;
        Ok(row.and_then(|m| serde_json::from_str(&m.value_json).ok()))
    }

    pub async fn put(&self, name: &str, value: serde_json::Value) -> Result<()> {
        let value_json = serde_json::to_string(&value).map_err(|e| {
            PdsError::internal("PreferenceReader::put: serialize", anyhow::Error::from(e))
        })?;
        let id = migration::types::db_id::DbId::new();
        let am = account_pref::ActiveModel {
            id: Set(id),
            name: Set(name.to_string()),
            value_json: Set(value_json),
        };
        account_pref::Entity::insert(am)
            .on_conflict(
                OnConflict::column(account_pref::Column::Name)
                    .update_column(account_pref::Column::ValueJson)
                    .to_owned(),
            )
            .exec(&self.db)
            .await
            .map_err(|e| {
                PdsError::internal("PreferenceReader::put: insert", anyhow::Error::from(e))
            })?;
        Ok(())
    }

    pub async fn list(&self) -> Result<Vec<(String, serde_json::Value)>> {
        let rows = account_pref::Entity::find()
            .order_by_asc(account_pref::Column::Name)
            .all(&self.db)
            .await
            .map_err(|e| PdsError::internal("PreferenceReader::list", anyhow::Error::from(e)))?;
        Ok(rows
            .into_iter()
            .filter_map(|m| {
                serde_json::from_str(&m.value_json)
                    .ok()
                    .map(|v| (m.name, v))
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    async fn setup() -> (camino_tempfile::Utf8TempDir, PreferenceReader) {
        let dir = camino_tempfile::Utf8TempDir::new().unwrap();
        let db = cacos_pds_core::db::DatabaseKind::Actor
            .open(dir.path().join("store.sqlite"))
            .await
            .unwrap();
        let reader = PreferenceReader::new("did:example:alice".to_string(), db);
        (dir, reader)
    }

    #[tokio::test]
    async fn get_returns_none_when_missing() {
        let (_dir, reader) = setup().await;
        assert!(reader.get("missing").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn put_then_get_roundtrip() {
        let (_dir, reader) = setup().await;
        reader
            .put("app.bsky.actor.declinedCategories", json!(["spam"]))
            .await
            .unwrap();
        let got = reader
            .get("app.bsky.actor.declinedCategories")
            .await
            .unwrap();
        assert_eq!(got, Some(json!(["spam"])));
    }

    #[tokio::test]
    async fn put_upserts_on_conflict() {
        let (_dir, reader) = setup().await;
        reader.put("ns", json!("v1")).await.unwrap();
        reader.put("ns", json!("v2")).await.unwrap();
        let got = reader.get("ns").await.unwrap();
        assert_eq!(got, Some(json!("v2")));
        let all = reader.list().await.unwrap();
        assert_eq!(all, vec![("ns".to_string(), json!("v2"))]);
    }
}
