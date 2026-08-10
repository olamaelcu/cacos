//! `com.atproto.repo.*` route table.
//!
//! The handlers (`create_record`, `put_record`, `apply_writes`,
//! `upload_blob`, `delete_record`, `describe_repo`) wrap the three
//! repo-write boundaries (`repo_load`, `repo_write`, `seq_write`) and
//! the blob-write boundary (`blob_put`) with `timed()` so the
//! observability stage histogram and the inter-event timing layer
//! report them. See `docs/superpowers/plans/2026-08-04-09-observability-sweep/INDEX.md`.

pub mod apply_writes;
pub mod create_record;
pub mod delete_record;
pub mod describe_repo;
pub mod prepare;
pub mod put_record;
pub mod upload_blob;

use poem::{Route, get, post};

pub fn routes(route: Route) -> Route {
    route
        .at("/createRecord", post(create_record::create_record))
        .at("/putRecord", post(put_record::put_record))
        .at("/deleteRecord", post(delete_record::delete_record))
        .at("/applyWrites", post(apply_writes::apply_writes))
        .at("/uploadBlob", post(upload_blob::upload_blob))
        .at(
            "/describeRepo",
            get(describe_repo::describe_repo),
        )
}
