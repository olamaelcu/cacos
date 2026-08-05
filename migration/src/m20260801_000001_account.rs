// migration/src/m20260801_000001_account.rs
//! Account database schema.
//!
//! Port of the rsky account_manager migration "001"
//! (`vendor/rsky/rsky-pds/src/account_manager/db.rs` lines 13-158).

use sea_orm_migration::prelude::*;

use crate::entities::{
    account, account_device, actor, app_password, authorization_request, authorized_client, device,
    email_token, invite_code, invite_code_use, lexicon, refresh_token, repo_root, token,
    used_refresh_token,
};
use crate::schema::{db_id, db_id_null, pk_db_id};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // ---- actor -------------------------------------------------------
        manager
            .create_table(
                Table::create()
                    .table(actor::Entity)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(actor::Column::Did)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(actor::Column::Handle).string())
                    .col(
                        ColumnDef::new(actor::Column::CreatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .col(ColumnDef::new(actor::Column::TakedownRef).string())
                    .col(ColumnDef::new(actor::Column::DeactivatedAt).timestamp())
                    .col(ColumnDef::new(actor::Column::DeleteAfter).timestamp())
                    .to_owned(),
            )
            .await?;
        // Functional indexes (lower(handle)) panic in sea-query's SQLite
        // IndexBuilder — raw SQL escape hatch (verified note 5).
        raw_sql(
            manager,
            "CREATE UNIQUE INDEX actor_handle_lower_idx ON actor (lower(handle))",
        )
        .await?;
        manager
            .create_index(
                Index::create()
                    .name("actor_cursor_idx")
                    .table(actor::Entity)
                    .col(actor::Column::CreatedAt)
                    .col(actor::Column::Did)
                    .to_owned(),
            )
            .await?;

        // ---- account -----------------------------------------------------
        manager
            .create_table(
                Table::create()
                    .table(account::Entity)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(account::Column::Did)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(account::Column::Email).string().not_null())
                    .col(ColumnDef::new(account::Column::RecoveryKey).string())
                    .col(
                        ColumnDef::new(account::Column::Password)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(account::Column::CreatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(account::Column::InvitesDisabled)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(ColumnDef::new(account::Column::EmailConfirmedAt).timestamp())
                    .col(ColumnDef::new(account::Column::InviteNote).string())
                    .to_owned(),
            )
            .await?;
        raw_sql(
            manager,
            "CREATE UNIQUE INDEX account_email_lower_idx ON account (lower(email))",
        )
        .await?;

        // ---- app_password ------------------------------------------------
        manager
            .create_table(
                Table::create()
                    .table(app_password::Entity)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(app_password::Column::Did)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(app_password::Column::Name)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(app_password::Column::Password)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(app_password::Column::CreatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(app_password::Column::Privileged)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .primary_key(
                        Index::create()
                            .name("pk_app_password")
                            .col(app_password::Column::Did)
                            .col(app_password::Column::Name),
                    )
                    .to_owned(),
            )
            .await?;

        // ---- refresh_token -----------------------------------------------
        manager
            .create_table(
                Table::create()
                    .table(refresh_token::Entity)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(refresh_token::Column::Id)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(refresh_token::Column::Did)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(refresh_token::Column::ExpiresAt)
                            .timestamp()
                            .not_null(),
                    )
                    .col(ColumnDef::new(refresh_token::Column::NextId).string())
                    .col(ColumnDef::new(refresh_token::Column::AppPasswordName).string())
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("refresh_token_did_idx")
                    .table(refresh_token::Entity)
                    .col(refresh_token::Column::Did)
                    .to_owned(),
            )
            .await?;

        // ---- repo_root ---------------------------------------------------
        manager
            .create_table(
                Table::create()
                    .table(repo_root::Entity)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(repo_root::Column::Did)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(repo_root::Column::Cid).string().not_null())
                    .col(ColumnDef::new(repo_root::Column::Rev).string().not_null())
                    .col(
                        ColumnDef::new(repo_root::Column::IndexedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // ---- invite_code -------------------------------------------------
        manager
            .create_table(
                Table::create()
                    .table(invite_code::Entity)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(invite_code::Column::Code)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(invite_code::Column::AvailableUses)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(invite_code::Column::Disabled)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(invite_code::Column::ForAccount)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(invite_code::Column::CreatedBy)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(invite_code::Column::CreatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("invite_code_for_account_idx")
                    .table(invite_code::Entity)
                    .col(invite_code::Column::ForAccount)
                    .to_owned(),
            )
            .await?;

        // ---- invite_code_use ---------------------------------------------
        manager
            .create_table(
                Table::create()
                    .table(invite_code_use::Entity)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(invite_code_use::Column::Code)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(invite_code_use::Column::UsedBy)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(invite_code_use::Column::UsedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .primary_key(
                        Index::create()
                            .name("pk_invite_code_use")
                            .col(invite_code_use::Column::Code)
                            .col(invite_code_use::Column::UsedBy),
                    )
                    .to_owned(),
            )
            .await?;

        // ---- email_token -------------------------------------------------
        manager
            .create_table(
                Table::create()
                    .table(email_token::Entity)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(email_token::Column::Purpose)
                            .string()
                            .not_null(),
                    )
                    .col(ColumnDef::new(email_token::Column::Did).string().not_null())
                    .col(
                        ColumnDef::new(email_token::Column::Token)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(email_token::Column::RequestedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .primary_key(
                        Index::create()
                            .name("pk_email_token")
                            .col(email_token::Column::Purpose)
                            .col(email_token::Column::Did),
                    )
                    .to_owned(),
            )
            .await?;
        // `UNIQUE (purpose, token)` from the reference DDL, as a named unique
        // index (functionally identical on SQLite).
        manager
            .create_index(
                Index::create()
                    .name("uq_email_token_purpose_token")
                    .unique()
                    .table(email_token::Entity)
                    .col(email_token::Column::Purpose)
                    .col(email_token::Column::Token)
                    .to_owned(),
            )
            .await?;

        // ---- authorization_request --------------------------------------
        manager
            .create_table(
                Table::create()
                    .table(authorization_request::Entity)
                    .if_not_exists()
                    .col(pk_db_id(authorization_request::Column::Id))
                    .col(ColumnDef::new(authorization_request::Column::Did).string())
                    .col(db_id_null(authorization_request::Column::DeviceId))
                    .col(
                        ColumnDef::new(authorization_request::Column::ClientId)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(authorization_request::Column::ClientAuth)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(authorization_request::Column::Parameters)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(authorization_request::Column::ExpiresAt)
                            .timestamp()
                            .not_null(),
                    )
                    .col(ColumnDef::new(authorization_request::Column::Code).string())
                    .to_owned(),
            )
            .await?;
        raw_sql(
            manager,
            "CREATE UNIQUE INDEX authorization_request_code_idx ON authorization_request (code DESC) WHERE code IS NOT NULL",
        )
        .await?;
        manager
            .create_index(
                Index::create()
                    .name("authorization_request_expires_at_idx")
                    .table(authorization_request::Entity)
                    .col(authorization_request::Column::ExpiresAt)
                    .to_owned(),
            )
            .await?;

        // ---- device ------------------------------------------------------
        manager
            .create_table(
                Table::create()
                    .table(device::Entity)
                    .if_not_exists()
                    .col(pk_db_id(device::Column::Id))
                    .col(
                        ColumnDef::new(device::Column::SessionId)
                            .string()
                            .not_null(),
                    )
                    .col(ColumnDef::new(device::Column::UserAgent).string())
                    .col(
                        ColumnDef::new(device::Column::IpAddress)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(device::Column::LastSeenAt)
                            .timestamp()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;
        // `UNIQUE ("sessionId")` from the reference DDL, as a named unique index.
        manager
            .create_index(
                Index::create()
                    .name("uq_device_session_id")
                    .unique()
                    .table(device::Entity)
                    .col(device::Column::SessionId)
                    .to_owned(),
            )
            .await?;

        // ---- account_device ---------------------------------------------
        manager
            .create_table(
                Table::create()
                    .table(account_device::Entity)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(account_device::Column::Did)
                            .string()
                            .not_null(),
                    )
                    .col(db_id(account_device::Column::DeviceId))
                    .col(
                        ColumnDef::new(account_device::Column::CreatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(account_device::Column::UpdatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .primary_key(
                        Index::create()
                            .name("pk_account_device")
                            .col(account_device::Column::DeviceId)
                            .col(account_device::Column::Did),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_account_device_account")
                            .from(account_device::Entity, account_device::Column::Did)
                            .to(account::Entity, account::Column::Did)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_account_device_device")
                            .from(account_device::Entity, account_device::Column::DeviceId)
                            .to(device::Entity, device::Column::Id)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("account_device_did_idx")
                    .table(account_device::Entity)
                    .col(account_device::Column::Did)
                    .to_owned(),
            )
            .await?;

        // ---- authorized_client ------------------------------------------
        manager
            .create_table(
                Table::create()
                    .table(authorized_client::Entity)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(authorized_client::Column::Did)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(authorized_client::Column::ClientId)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(authorized_client::Column::CreatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(authorized_client::Column::UpdatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(authorized_client::Column::Data)
                            .string()
                            .not_null(),
                    )
                    .primary_key(
                        Index::create()
                            .name("pk_authorized_client")
                            .col(authorized_client::Column::Did)
                            .col(authorized_client::Column::ClientId),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_authorized_client_account")
                            .from(authorized_client::Entity, authorized_client::Column::Did)
                            .to(account::Entity, account::Column::Did)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // ---- token -------------------------------------------------------
        manager
            .create_table(
                Table::create()
                    .table(token::Entity)
                    .if_not_exists()
                    .col(pk_db_id(token::Column::Id))
                    .col(ColumnDef::new(token::Column::Did).string().not_null())
                    .col(ColumnDef::new(token::Column::TokenId).string().not_null())
                    .col(
                        ColumnDef::new(token::Column::CreatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(token::Column::UpdatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(token::Column::ExpiresAt)
                            .timestamp()
                            .not_null(),
                    )
                    .col(ColumnDef::new(token::Column::ClientId).string().not_null())
                    .col(
                        ColumnDef::new(token::Column::ClientAuth)
                            .string()
                            .not_null(),
                    )
                    .col(db_id_null(token::Column::DeviceId))
                    .col(
                        ColumnDef::new(token::Column::Parameters)
                            .string()
                            .not_null(),
                    )
                    .col(ColumnDef::new(token::Column::Details).string())
                    .col(ColumnDef::new(token::Column::Code).string())
                    .col(ColumnDef::new(token::Column::CurrentRefreshToken).string())
                    .col(ColumnDef::new(token::Column::Scope).string())
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("token_did_idx")
                    .table(token::Entity)
                    .col(token::Column::Did)
                    .to_owned(),
            )
            .await?;
        raw_sql(
            manager,
            "CREATE UNIQUE INDEX token_code_idx ON token (code DESC) WHERE code IS NOT NULL",
        )
        .await?;
        // The two `UNIQUE (...)` constraints from the reference DDL.
        manager
            .create_index(
                Index::create()
                    .name("uq_token_current_refresh_token")
                    .unique()
                    .table(token::Entity)
                    .col(token::Column::CurrentRefreshToken)
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("uq_token_token_id")
                    .unique()
                    .table(token::Entity)
                    .col(token::Column::TokenId)
                    .to_owned(),
            )
            .await?;

        // ---- used_refresh_token -----------------------------------------
        manager
            .create_table(
                Table::create()
                    .table(used_refresh_token::Entity)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(used_refresh_token::Column::RefreshToken)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(db_id(used_refresh_token::Column::TokenId))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_used_refresh_token_token")
                            .from(
                                used_refresh_token::Entity,
                                used_refresh_token::Column::TokenId,
                            )
                            .to(token::Entity, token::Column::Id)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("used_refresh_token_id_idx")
                    .table(used_refresh_token::Entity)
                    .col(used_refresh_token::Column::TokenId)
                    .to_owned(),
            )
            .await?;

        // ---- lexicon -----------------------------------------------------
        manager
            .create_table(
                Table::create()
                    .table(lexicon::Entity)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(lexicon::Column::Nsid)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(lexicon::Column::CreatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(lexicon::Column::UpdatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .col(ColumnDef::new(lexicon::Column::LastSucceededAt).timestamp())
                    .col(ColumnDef::new(lexicon::Column::Uri).string())
                    .col(ColumnDef::new(lexicon::Column::Lexicon).string())
                    .to_owned(),
            )
            .await?;
        raw_sql(
            manager,
            "CREATE INDEX lexicon_failures_idx ON lexicon (\"updatedAt\" DESC) WHERE lexicon IS NULL",
        )
        .await?;

        Ok(())
    }
}

/// Execute raw DDL (used for functional indexes, which sea-query's SQLite
/// IndexBuilder cannot express — verified note 5).
async fn raw_sql(manager: &SchemaManager<'_>, sql: &str) -> Result<(), DbErr> {
    use sea_orm_migration::sea_orm::{ConnectionTrait, Statement};
    manager
        .get_connection()
        .execute_raw(Statement::from_string(
            manager.get_database_backend(),
            sql.to_owned(),
        ))
        .await?;
    Ok(())
}
