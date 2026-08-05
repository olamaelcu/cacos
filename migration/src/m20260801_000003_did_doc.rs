// migration/src/m20260801_000003_did_doc.rs
//! DID cache database schema.
//!
//! Port of the rsky did-cache migration "001"
//! (`vendor/rsky/rsky-pds/src/did_cache.rs` lines 15-19).

use sea_orm_migration::prelude::*;

use crate::entities::did_doc;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(did_doc::Entity)
                    .if_not_exists()
                    .col(ColumnDef::new(did_doc::Column::Did).string().not_null().primary_key())
                    .col(ColumnDef::new(did_doc::Column::Doc).string().not_null())
                    .col(ColumnDef::new(did_doc::Column::UpdatedAt).timestamp().not_null())
                    .to_owned(),
            )
            .await?;
        Ok(())
    }
}
