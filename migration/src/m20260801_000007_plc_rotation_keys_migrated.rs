// migration/src/m20260801_000007_plc_rotation_keys_migrated.rs
//! Adds `plcRotationKeysMigrated` to `actor`, tracking whether a DID's PLC
//! document has been rewritten to drop the shared, server-wide rotation key
//! in favour of the account's own per-DID rotation key.
//!
//! The flag is the resume point for `cacos-pds migrate plc-rotation-keys`:
//! the pass skips rows already marked, so an interrupted run picks up where
//! it stopped instead of re-submitting PLC operations.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // camelCase to match the surrounding `actor` columns (`createdAt`,
        // `takedownRef`, `deactivatedAt`), so the raw SQL in
        // `pds::account::helpers::account` keeps one quoting convention.
        manager
            .alter_table(
                Table::alter()
                    .table(Actor::Table)
                    .add_column(
                        ColumnDef::new(Alias::new("plcRotationKeysMigrated"))
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Actor::Table)
                    .drop_column(Alias::new("plcRotationKeysMigrated"))
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
pub enum Actor {
    Table,
}
