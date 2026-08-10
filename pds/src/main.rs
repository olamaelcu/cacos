use std::time::Duration;
use tracing_unwrap::ResultExt;

#[tokio::main]
async fn main() {
    cacos_pds::observability::metrics::init_metrics();
    let downcaster = cacos_pds::observability::tracing::init_tracing("info");
    let _timing = cacos_pds::observability::timing::TimingReporter::start(
        Duration::from_secs(10),
        downcaster,
    );

    // Build the shared OpenDAL operator (S3 if S3_ENDPOINT is set, else
    // disk) once for the process. Per-DID handles come from this via
    // `blobstore_for_did(did)` at the call site.
    cacos_pds::blobstore::init_operator().expect_or_log("blobstore init failed");

    let app = cacos_pds::xrpc::build_app().await;
    let listener = poem::listener::TcpListener::bind("127.0.0.1:8080");
    tracing::info!("pds listening on http://127.0.0.1:8080");
    poem::Server::new(listener)
        .run(app)
        .await
        .expect_or_log("poem server failed");
}
