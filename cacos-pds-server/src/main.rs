use std::time::Duration;
use tracing_unwrap::ResultExt;

#[tokio::main]
async fn main() {
    if let Err(e) = cacos_pds_account::account::helpers::init_required_keys::init_required_keys() {
        eprintln!("fatal: {e}");
        std::process::exit(1);
    }

    cacos_pds_core::observability::metrics::init_metrics();
    let downcaster = cacos_pds_core::observability::tracing::init_tracing("info");
    let _timing = cacos_pds_core::observability::timing::TimingReporter::start(
        Duration::from_secs(10),
        downcaster,
    );

    // Build the shared OpenDAL operator (S3 if S3_ENDPOINT is set, else
    // disk) once for the process. Per-DID handles come from this via
    // `blobstore_for_did(did)` at the call site.
    cacos_pds_blobstore::init_operator().expect_or_log("blobstore init failed");

    let app = cacos_pds_server::xrpc::build_app().await;
    let listener = poem::listener::TcpListener::bind("127.0.0.1:8080");
    tracing::info!("cacos-pds-server listening on http://127.0.0.1:8080");
    poem::Server::new(listener)
        .run(app)
        .await
        .expect_or_log("poem server failed");
}
