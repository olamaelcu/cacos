// migration/src/m20260801_000004_actor.rs
//! Actor database schema, migration 001 (core tables).
//!
//! Port of rsky actor-store migration "001"
//! (`vendor/rsky/rsky-pds/src/actor_store/db/mod.rs` lines 14-66).

use sea_orm_migration::prelude::*;

use crate::entities::{account_pref, backlink, blob, record, record_blob, repo_block, repo_root};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // ---- repo_root ---------------------------------------------------
        manager
            .create_table(
                Table::create()
                    .table(repo_root::Entity)
                    .if_not_exists()
                    .col(ColumnDef::new(repo_root::Column::Did).string().not_null().primary_key())
                    .col(ColumnDef::new(repo_root::Column::Cid).string().not_null())
                    .col(ColumnDef::new(repo_root::Column::Rev).string().not_null())
                    .col(ColumnDef::new(repo_root::Column::IndexedAt).string().not_null())
                    .to_owned(),
            )
            .await?;

        // ---- repo_block --------------------------------------------------
        manager
            .create_table(
                Table::create()
                    .table(repo_block::Entity)
                    .if_not_exists()
                    .col(ColumnDef::new(repo_block::Column::Cid).string().not_null().primary_key())
                    .col(ColumnDef::new(repo_block::Column::RepoRev).string().not_null())
                    .col(ColumnDef::new(repo_block::Column::Size).big_integer().not_null())
                    .col(ColumnDef::new(repo_block::Column::Content).blob().not_null())
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("repo_block_repo_rev_idx")
                    .table(repo_block::Entity)
                    .col(repo_block::Column::RepoRev)
                    .col(repo_block::Column::Cid)
                    .to_owned(),
            )
            .await?;

        // ---- record ------------------------------------------------------
        manager
            .create_table(
                Table::create()
                    .table(record::Entity)
                    .if_not_exists()
                    .col(ColumnDef::new(record::Column::Uri).string().not_null().primary_key())
                    .col(ColumnDef::new(record::Column::Cid).string().not_null())
                    .col(ColumnDef::new(record::Column::Collection).string().not_null())
                    .col(ColumnDef::new(record::Column::Rkey).string().not_null())
                    .col(ColumnDef::new(record::Column::RepoRev).string().not_null())
                    .col(ColumnDef::new(record::Column::IndexedAt).string().not_null())
                    .col(ColumnDef::new(record::Column::TakedownRef).string())
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("record_cid_idx")
                    .table(record::Entity)
                    .col(record::Column::Cid)
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("record_collection_idx")
                    .table(record::Entity)
                    .col(record::Column::Collection)
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("record_repo_rev_idx")
                    .table(record::Entity)
                    .col(record::Column::RepoRev)
                    .to_owned(),
            )
            .await?;

        // ---- blob --------------------------------------------------------
        manager
            .create_table(
                Table::create()
                    .table(blob::Entity)
                    .if_not_exists()
                    .col(ColumnDef::new(blob::Column::Cid).string().not_null().primary_key())
                    .col(ColumnDef::new(blob::Column::MimeType).string().not_null())
                    .col(ColumnDef::new(blob::Column::Size).big_integer().not_null())
                    .col(ColumnDef::new(blob::Column::TempKey).string())
                    .col(ColumnDef::new(blob::Column::Width).big_integer())
                    .col(ColumnDef::new(blob::Column::Height).big_integer())
                    .col(ColumnDef::new(blob::Column::CreatedAt).string().not_null())
                    .col(ColumnDef::new(blob::Column::TakedownRef).string())
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("blob_tempkey_idx")
                    .table(blob::Entity)
                    .col(blob::Column::TempKey)
                    .to_owned(),
            )
            .await?;

        // ---- record_blob -------------------------------------------------
        manager
            .create_table(
                Table::create()
                    .table(record_blob::Entity)
                    .if_not_exists()
                    .col(ColumnDef::new(record_blob::Column::BlobCid).string().not_null())
                    .col(ColumnDef::new(record_blob::Column::RecordUri).string().not_null())
                    .primary_key(
                        Index::create()
                            .name("pk_record_blob")
                            .col(record_blob::Column::BlobCid)
                            .col(record_blob::Column::RecordUri),
                    )
                    .to_owned(),
            )
            .await?;

        // ---- backlink ----------------------------------------------------
        manager
            .create_table(
                Table::create()
                    .table(backlink::Entity)
                    .if_not_exists()
                    .col(ColumnDef::new(backlink::Column::Uri).string().not_null())
                    .col(ColumnDef::new(backlink::Column::Path).string().not_null())
                    .col(ColumnDef::new(backlink::Column::LinkTo).string().not_null())
                    .primary_key(
                        Index::create()
                            .name("pk_backlink")
                            .col(backlink::Column::Uri)
                            .col(backlink::Column::Path),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("backlink_link_to_idx")
                    .table(backlink::Entity)
                    .col(backlink::Column::Path)
                    .col(backlink::Column::LinkTo)
                    .to_owned(),
            )
            .await?;

        // ---- account_pref ------------------------------------------------
        manager
            .create_table(
                Table::create()
                    .table(account_pref::Entity)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(account_pref::Column::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(account_pref::Column::Name).string().not_null())
                    .col(ColumnDef::new(account_pref::Column::ValueJson).string().not_null())
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}
