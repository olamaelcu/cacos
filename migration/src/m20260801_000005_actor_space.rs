// migration/src/m20260801_000005_actor_space.rs
//! Actor database schema, migration 002 (space tables).
//!
//! Port of rsky actor-store migration "002"
//! (`vendor/rsky/rsky-pds/src/actor_store/db/mod.rs` lines 69-146).

use sea_orm_migration::prelude::*;

use crate::entities::{
    space_blob_ref, space_def, space_host_reg, space_member, space_oplog, space_record,
    space_repo, space_repo_notify, space_used_jti, space_writer,
};
use crate::schema::pk_db_id;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // ---- space_repo --------------------------------------------------
        manager
            .create_table(
                Table::create()
                    .table(space_repo::Entity)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(space_repo::Column::SpaceUri)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(space_repo::Column::Authority).string().not_null())
                    .col(ColumnDef::new(space_repo::Column::SpaceType).string().not_null())
                    .col(ColumnDef::new(space_repo::Column::Skey).string().not_null())
                    .col(ColumnDef::new(space_repo::Column::Rev).string().not_null())
                    .col(ColumnDef::new(space_repo::Column::LthashState).blob().not_null())
                    .col(ColumnDef::new(space_repo::Column::OplogFloorRev).string())
                    .col(
                        ColumnDef::new(space_repo::Column::Deleted)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(ColumnDef::new(space_repo::Column::CreatedAt).timestamp().not_null())
                    .to_owned(),
            )
            .await?;

        // ---- space_record ------------------------------------------------
        manager
            .create_table(
                Table::create()
                    .table(space_record::Entity)
                    .if_not_exists()
                    .col(ColumnDef::new(space_record::Column::SpaceUri).string().not_null())
                    .col(ColumnDef::new(space_record::Column::Collection).string().not_null())
                    .col(ColumnDef::new(space_record::Column::Rkey).string().not_null())
                    .col(ColumnDef::new(space_record::Column::Cid).string().not_null())
                    .col(ColumnDef::new(space_record::Column::Rev).string().not_null())
                    .col(ColumnDef::new(space_record::Column::Value).blob().not_null())
                    .primary_key(
                        Index::create()
                            .name("pk_space_record")
                            .col(space_record::Column::SpaceUri)
                            .col(space_record::Column::Collection)
                            .col(space_record::Column::Rkey),
                    )
                    .to_owned(),
            )
            .await?;

        // ---- space_oplog -------------------------------------------------
        manager
            .create_table(
                Table::create()
                    .table(space_oplog::Entity)
                    .if_not_exists()
                    .col(pk_db_id(space_oplog::Column::Id))
                    .col(ColumnDef::new(space_oplog::Column::SpaceUri).string().not_null())
                    .col(ColumnDef::new(space_oplog::Column::Rev).string().not_null())
                    .col(ColumnDef::new(space_oplog::Column::Collection).string().not_null())
                    .col(ColumnDef::new(space_oplog::Column::Rkey).string().not_null())
                    .col(ColumnDef::new(space_oplog::Column::Cid).string())
                    .col(ColumnDef::new(space_oplog::Column::Prev).string())
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("space_oplog_space_idx")
                    .table(space_oplog::Entity)
                    .col(space_oplog::Column::SpaceUri)
                    .col(space_oplog::Column::Id)
                    .to_owned(),
            )
            .await?;

        // ---- space_blob_ref ---------------------------------------------
        manager
            .create_table(
                Table::create()
                    .table(space_blob_ref::Entity)
                    .if_not_exists()
                    .col(ColumnDef::new(space_blob_ref::Column::SpaceUri).string().not_null())
                    .col(ColumnDef::new(space_blob_ref::Column::BlobCid).string().not_null())
                    .col(ColumnDef::new(space_blob_ref::Column::Collection).string().not_null())
                    .col(ColumnDef::new(space_blob_ref::Column::Rkey).string().not_null())
                    .primary_key(
                        Index::create()
                            .name("pk_space_blob_ref")
                            .col(space_blob_ref::Column::SpaceUri)
                            .col(space_blob_ref::Column::BlobCid)
                            .col(space_blob_ref::Column::Collection)
                            .col(space_blob_ref::Column::Rkey),
                    )
                    .to_owned(),
            )
            .await?;

        // ---- space_repo_notify ------------------------------------------
        manager
            .create_table(
                Table::create()
                    .table(space_repo_notify::Entity)
                    .if_not_exists()
                    .col(ColumnDef::new(space_repo_notify::Column::SpaceUri).string().not_null())
                    .col(ColumnDef::new(space_repo_notify::Column::Endpoint).string().not_null())
                    .col(ColumnDef::new(space_repo_notify::Column::ExpiresAt).timestamp().not_null())
                    .primary_key(
                        Index::create()
                            .name("pk_space_repo_notify")
                            .col(space_repo_notify::Column::SpaceUri)
                            .col(space_repo_notify::Column::Endpoint),
                    )
                    .to_owned(),
            )
            .await?;

        // ---- space_def ---------------------------------------------------
        manager
            .create_table(
                Table::create()
                    .table(space_def::Entity)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(space_def::Column::SpaceUri)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(space_def::Column::SpaceType).string().not_null())
                    .col(ColumnDef::new(space_def::Column::Skey).string().not_null())
                    .col(
                        ColumnDef::new(space_def::Column::Policy)
                            .string()
                            .not_null()
                            .default("member-list"),
                    )
                    .col(
                        ColumnDef::new(space_def::Column::AppAccess)
                            .string()
                            .not_null()
                            .default("open"),
                    )
                    .col(ColumnDef::new(space_def::Column::AllowedClients).string())
                    .col(ColumnDef::new(space_def::Column::ManagingApp).string())
                    .col(
                        ColumnDef::new(space_def::Column::Deleted)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(ColumnDef::new(space_def::Column::CreatedAt).timestamp().not_null())
                    .to_owned(),
            )
            .await?;

        // ---- space_member ------------------------------------------------
        manager
            .create_table(
                Table::create()
                    .table(space_member::Entity)
                    .if_not_exists()
                    .col(ColumnDef::new(space_member::Column::SpaceUri).string().not_null())
                    .col(ColumnDef::new(space_member::Column::Did).string().not_null())
                    .primary_key(
                        Index::create()
                            .name("pk_space_member")
                            .col(space_member::Column::SpaceUri)
                            .col(space_member::Column::Did),
                    )
                    .to_owned(),
            )
            .await?;

        // ---- space_writer ------------------------------------------------
        manager
            .create_table(
                Table::create()
                    .table(space_writer::Entity)
                    .if_not_exists()
                    .col(ColumnDef::new(space_writer::Column::SpaceUri).string().not_null())
                    .col(ColumnDef::new(space_writer::Column::Did).string().not_null())
                    .col(ColumnDef::new(space_writer::Column::Rev).string().not_null())
                    .col(ColumnDef::new(space_writer::Column::Hash).string())
                    .primary_key(
                        Index::create()
                            .name("pk_space_writer")
                            .col(space_writer::Column::SpaceUri)
                            .col(space_writer::Column::Did),
                    )
                    .to_owned(),
            )
            .await?;

        // ---- space_host_reg ----------------------------------------------
        manager
            .create_table(
                Table::create()
                    .table(space_host_reg::Entity)
                    .if_not_exists()
                    .col(ColumnDef::new(space_host_reg::Column::SpaceUri).string().not_null())
                    .col(ColumnDef::new(space_host_reg::Column::Endpoint).string().not_null())
                    .col(ColumnDef::new(space_host_reg::Column::ExpiresAt).timestamp().not_null())
                    .primary_key(
                        Index::create()
                            .name("pk_space_host_reg")
                            .col(space_host_reg::Column::SpaceUri)
                            .col(space_host_reg::Column::Endpoint),
                    )
                    .to_owned(),
            )
            .await?;

        // ---- space_used_jti ----------------------------------------------
        manager
            .create_table(
                Table::create()
                    .table(space_used_jti::Entity)
                    .if_not_exists()
                    .col(ColumnDef::new(space_used_jti::Column::Jti).string().not_null().primary_key())
                    .col(ColumnDef::new(space_used_jti::Column::Exp).big_integer().not_null())
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}
