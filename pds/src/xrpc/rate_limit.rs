//! Per-IP token-bucket rate limit middleware for sensitive endpoints.
//!
//! Uses the `governor` crate. Default 60 req/min/IP, configurable per
//! route via env. Disabled when the per-route limit is set to 0.

use governor::{RateLimiter, clock::DefaultClock, state::keyed::DashMapStateStore};
use poem::http::StatusCode;
use poem::{Endpoint, Error, IntoResponse, Middleware, Request, Response};
use std::net::IpAddr;
use std::sync::Arc;

pub type IpRateLimiter = RateLimiter<IpAddr, DashMapStateStore<IpAddr>, DefaultClock>;

pub fn ip_limiter(per_minute: u32) -> Arc<IpRateLimiter> {
    use std::num::NonZeroU32;
    let rpm = NonZeroU32::new(per_minute.max(1)).unwrap();
    Arc::new(RateLimiter::dashmap(governor::Quota::per_minute(rpm)))
}

pub struct RouteRateLimit {
    pub limiter: Arc<IpRateLimiter>,
}

impl<E: Endpoint> Middleware<E> for RouteRateLimit {
    type Output = RouteRateLimitEndpoint<E>;
    fn transform(&self, ep: E) -> Self::Output {
        RouteRateLimitEndpoint {
            ep,
            limiter: self.limiter.clone(),
        }
    }
}

pub struct RouteRateLimitEndpoint<E> {
    ep: E,
    limiter: Arc<IpRateLimiter>,
}

impl<E: Endpoint> Endpoint for RouteRateLimitEndpoint<E> {
    type Output = Response;
    async fn call(&self, req: Request) -> poem::Result<Self::Output> {
        let ip = req
            .remote_addr()
            .as_socket_addr()
            .map(|a| a.ip())
            .unwrap_or(IpAddr::from([0u8; 4]));
        if self.limiter.check_key(&ip).is_err() {
            return Err(Error::from_response(
                Response::builder()
                    .status(StatusCode::TOO_MANY_REQUESTS)
                    .content_type("application/json")
                    .body(r#"{"error":"RateLimitExceeded","message":"too many requests"}"#)
                    .into_response(),
            ));
        }
        let res = self.ep.call(req).await?;
        Ok(res.into_response())
    }
}
