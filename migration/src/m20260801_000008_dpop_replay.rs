// migration/src/m20260801_000008_dpop_replay.rs
//! Persistent DPoP JTI replay store: rows hold a `(jti, expires_at)`
//! pair so the same JWT `jti` cannot be replayed while its row is still
//! live. A background task in `pds::account::oauth_store` prunes rows
//! whose `expires_at` is in the past.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(DpopReplay::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(DpopReplay::Jti)
                            .text()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(DpopReplay::ExpiresAt).integer().not_null())
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("dpop_replay_expires_at_idx")
                    .table(DpopReplay::Table)
                    .col(DpopReplay::ExpiresAt)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(DpopReplay::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
pub enum DpopReplay {
    Table,
    Jti,
    ExpiresAt,
}
