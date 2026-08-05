//! Project-specific migration schema helpers.
//!
//! Column-def builders for `DbId` (ULID, stored as BINARY(16)).

use sea_orm_migration::prelude::*;

/// Create a `DbId` primary key column (non-nullable, no auto-increment).
pub fn pk_db_id<T: IntoIden>(col: T) -> ColumnDef {
    ColumnDef::new(col)
        .binary_len(16)
        .not_null()
        .primary_key()
        .take()
}

/// Create a non-nullable `BINARY(16)` column for `DbId` (ULID).
pub fn db_id<T: IntoIden>(col: T) -> ColumnDef {
    ColumnDef::new(col).binary_len(16).not_null().take()
}

/// Create a nullable `BINARY(16)` column for `DbId`.
pub fn db_id_null<T: IntoIden>(col: T) -> ColumnDef {
    ColumnDef::new(col).binary_len(16).null().take()
}
