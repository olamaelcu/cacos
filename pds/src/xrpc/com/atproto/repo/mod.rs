//! `com.atproto.repo.*` route table.
//!
//! The handler modules (`create_record`, `put_record`, `apply_writes`,
//! `upload_blob`, etc.) land in Tasks 12–17 of Plan 08. Until then
//! `routes()` is a no-op pass-through; `prepare` exposes the write
//! preparation helpers consumed by those handlers.

pub mod prepare;

use poem::Route;

pub fn routes(route: Route) -> Route {
    route
}
