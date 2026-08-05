// migration/src/m20260801_000002_repo_seq.rs
//! Sequencer database schema.
//!
//! Port of the rsky sequencer migration "001"
//! (`vendor/rsky/rsky-pds/src/sequencer/db.rs` lines 13-23).

use sea_orm_migration::prelude::*;

use crate::entities::repo_seq;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(repo_seq::Entity)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(repo_seq::Column::Seq)
                            .integer()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(repo_seq::Column::Did).string().not_null())
                    .col(ColumnDef::new(repo_seq::Column::EventType).string().not_null())
                    .col(ColumnDef::new(repo_seq::Column::Event).blob().not_null())
                    .col(
                        ColumnDef::new(repo_seq::Column::Invalidated)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(ColumnDef::new(repo_seq::Column::SequencedAt).timestamp().not_null())
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("repo_seq_did_idx")
                    .table(repo_seq::Entity)
                    .col(repo_seq::Column::Did)
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("repo_seq_event_type_idx")
                    .table(repo_seq::Entity)
                    .col(repo_seq::Column::EventType)
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("repo_seq_sequenced_at_index")
                    .table(repo_seq::Entity)
                    .col(repo_seq::Column::SequencedAt)
                    .to_owned(),
            )
            .await?;
        Ok(())
    }
}
