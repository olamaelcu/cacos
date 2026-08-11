//! `com.atproto.sync.*` handlers. The only handler wired today is
//! [`subscribe_repos`]; downstream plans add `getRepo`, `listBlobs`,
//! `requestCrawl`, etc.

pub mod subscribe_repos;

use poem::Route;

/// Adds every `com.atproto.sync.*` NSID handler onto `route` and returns
/// the modified Route. The full NSID path is used directly so the XRPC
/// convention (dotted, no slashes) is preserved.
pub fn routes(route: Route) -> Route {
    route.at(
        "/com.atproto.sync.subscribeRepos",
        poem::get(subscribe_repos::subscribe_repos),
    )
}
