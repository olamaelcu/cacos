use std::time::Duration;

#[tokio::main]
async fn main() {
    cacos_pds::observability::metrics::init_metrics();
    let downcaster = cacos_pds::observability::tracing::init_tracing("info");
    let _timing = cacos_pds::observability::timing::TimingReporter::start(
        Duration::from_secs(10),
        downcaster,
    );

    let listener = poem::listener::TcpListener::bind("127.0.0.1:8080");
    tracing::info!("pds listening on http://127.0.0.1:8080");
    poem::Server::new(listener)
        .run(cacos_pds::xrpc::build_app())
        .await
        .expect("poem server failed");
}
