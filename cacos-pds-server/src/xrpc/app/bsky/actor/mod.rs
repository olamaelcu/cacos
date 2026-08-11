//! Placeholder for `app.bsky.actor.*` handlers. Filled in by downstream
//! plans.

use poem::Route;

/// Adds the `app.bsky.actor.*` NSID handlers onto `route` and returns
/// the modified Route. Downstream plans attach real handlers here.
pub fn routes(route: Route) -> Route {
    route
}
