//! Smoke test that the account lockout migration ran successfully.
//! Verifies the expected `failedLoginCount` and `lockedUntil` columns
//! exist on the `account` table.

#[tokio::test]
async fn check_lockout_columns_present() {
    let (state, _dirs) = cacos_pds_server::xrpc::test_utils::test_state().await;
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
    let db = state.account_manager.db.clone();
    let stmt = Statement::from_string(
        DatabaseBackend::Sqlite,
        "PRAGMA table_info(account)".to_owned(),
    );
    let rows = db.query_all_raw(stmt).await.unwrap();
    let mut names: Vec<String> = rows
        .iter()
        .map(|row| row.try_get_by_index::<String>(1).unwrap())
        .collect();
    names.sort();
    assert!(
        names.iter().any(|n| n == "failedLoginCount"),
        "missing failedLoginCount column; got {names:?}"
    );
    assert!(
        names.iter().any(|n| n == "lockedUntil"),
        "missing lockedUntil column; got {names:?}"
    );
}
