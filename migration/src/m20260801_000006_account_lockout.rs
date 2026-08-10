// migration/src/m20260801_000006_account_lockout.rs
//! Adds `failedLoginCount` and `lockedUntil` columns for per-account
//! login lockout (5 failed attempts in 15 minutes triggers a 15-minute
//! lockout).

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // SQLite's ALTER TABLE only accepts one column per statement, so
        // split the additions. Column names match the camelCase convention
        // used by existing `account.*` columns (`createdAt`,
        // `emailConfirmedAt`, etc.) so the SQL helpers in
        // `pds::account::helpers::account` can keep using the same quoted
        // identifiers.
        manager
            .alter_table(
                Table::alter()
                    .table(Account::Table)
                    .add_column(
                        ColumnDef::new(Alias::new("failedLoginCount"))
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Account::Table)
                    .add_column(ColumnDef::new(Alias::new("lockedUntil")).integer().null())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Account::Table)
                    .drop_column(Alias::new("lockedUntil"))
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Account::Table)
                    .drop_column(Alias::new("failedLoginCount"))
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
pub enum Account {
    Table,
}
