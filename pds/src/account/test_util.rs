//! Shared test fixtures for the account module: env + migrated temp DB.
//! Mirrors `vendor/rsky/rsky-pds/src/account_manager/tests.rs` `init_env`.

use sea_orm::DatabaseConnection;
use std::sync::Once;

pub const TEST_CID: &str = "bafkreibjfgx2gprinfvicegelk5kosd6y2frmqpqzwqkg7usac74l3t2v4";

static INIT_ENV: Once = Once::new();

pub(crate) fn init_env() {
    INIT_ENV.call_once(|| {
        let defaults = [
            ("PDS_SERVICE_DID", "did:web:localho.st"),
            (
                "PDS_JWT_KEY_K256_PRIVATE_KEY_HEX",
                "9d5907143471e8f0e8df0f8b9512a8c5377878ee767f18fcf961055ecfc071cd",
            ),
            (
                "PDS_REPO_SIGNING_KEY_K256_PRIVATE_KEY_HEX",
                "71cfcf4882a6cff494c3d0affadd3858eb3a5838e7b5e15170e696a590a4fa01",
            ),
        ];
        for (key, value) in defaults {
            if std::env::var(key).is_err() {
                // SAFETY: tests run sequentially within a process, no concurrent
                // env reads; matches the rsky-pds reference fixture.
                unsafe {
                    std::env::set_var(key, value);
                }
            }
        }
    });
}

/// Opens a migrated, temp-file account DB via Plan 01's bootstrap.
/// Returns the TempDir so it stays alive for the test's duration.
pub(crate) async fn test_db() -> (camino_tempfile::Utf8TempDir, DatabaseConnection) {
    init_env();
    let dir = camino_tempfile::Utf8TempDir::new().unwrap();
    let db = crate::db::DatabaseKind::Account
        .open(dir.path().join("account.sqlite"))
        .await
        .unwrap();
    (dir, db)
}
