//! XRPC route assembly.

pub mod com;

/// Builds the app. Opens the account DB from `PDS_DB_PATH` (default
/// `./account.sqlite`) when the OAuth JWT key is configured, and nests the
/// OAuth route set (provider routes + headless-consent remote API)
/// alongside `/metrics`.
pub async fn build_app() -> impl poem::Endpoint<Output = poem::Response> {
    let base = poem::Route::new().at(
        "/metrics",
        poem::get(crate::observability::http::metrics_handler),
    );
    let oauth_app = if std::env::var("PDS_JWT_KEY_K256_PRIVATE_KEY_HEX").is_ok() {
        let db_path =
            std::env::var("PDS_DB_PATH").unwrap_or_else(|_| "./account.sqlite".to_string());
        match crate::db::DatabaseKind::Account
            .open(camino::Utf8Path::new(&db_path))
            .await
        {
            Ok(account_db) => crate::oauth::bootstrap_oauth_app(account_db),
            Err(err) => {
                tracing::error!(%err, "failed to open account database");
                None
            }
        }
    } else {
        None
    };
    match oauth_app {
        Some(app) => {
            // Merge the OAuth route set at the root without stripping the
            // path prefix (the oauth routes carry their full paths). The
            // inner `oauth_app` carries its own `Data`.
            base.nest_no_strip("/", app)
        }
        None => base,
    }
}
