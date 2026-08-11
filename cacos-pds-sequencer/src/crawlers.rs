//! Notify subscribed relays/BGS that this PDS has new repo events.
//!
//! Rate limited to once every 20 minutes (`NOTIFY_THRESHOLD`); subsequent
//! calls within the window are no-ops. Port of `rsky-pds/src/crawlers.rs`.

use anyhow::Result;
use cacos_pds_core::error::PdsError;
use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

pub const APP_USER_AGENT: &str = concat!(
    env!("CARGO_PKG_HOMEPAGE"),
    "@",
    env!("CARGO_PKG_NAME"),
    "/",
    env!("CARGO_PKG_VERSION"),
);

const NOTIFY_THRESHOLD: i32 = 20 * 60 * 1000; // 20 minutes in milliseconds

#[derive(Debug, Clone)]
pub struct Crawlers {
    pub hostname: String,
    pub crawlers: Vec<String>,
    pub last_notified: usize,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct CrawlerRequest {
    pub hostname: String,
}

impl Crawlers {
    pub fn new(hostname: String, crawlers: Vec<String>) -> Self {
        Crawlers {
            hostname,
            crawlers,
            last_notified: 0,
        }
    }

    /// The body sent to each crawl service: advertise this PDS's hostname,
    /// not the crawler's.
    pub fn crawl_request(&self) -> CrawlerRequest {
        CrawlerRequest {
            hostname: self.hostname.clone(),
        }
    }

    pub async fn notify_of_update(&mut self) -> Result<()> {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_err(|e| PdsError::internal("crawlers: SystemTime", anyhow::Error::from(e)))?
            .as_millis() as usize;
        if now.saturating_sub(self.last_notified) < NOTIFY_THRESHOLD as usize {
            return Ok(());
        }
        let record = self.crawl_request();
        let _ = stream::iter(self.crawlers.clone())
            .then(|service: String| {
                let record = record.clone();
                async move {
                    let client = reqwest::Client::builder()
                        .user_agent(APP_USER_AGENT)
                        .build()?;
                    Ok::<reqwest::Response, anyhow::Error>(
                        client
                            .post(format!("{}/xrpc/com.atproto.sync.requestCrawl", service))
                            .json(&record)
                            .send()
                            .await?,
                    )
                }
            })
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;

        self.last_notified = now;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crawl_request_advertises_pds_hostname() {
        let crawlers = Crawlers::new(
            "pds.example.com".to_string(),
            vec![
                "https://relay1.example".to_string(),
                "https://relay2.example".to_string(),
            ],
        );
        assert_eq!(crawlers.crawl_request().hostname, "pds.example.com");
    }

    #[tokio::test]
    async fn notify_of_update_rate_limits_to_once_per_20_minutes() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let uri = server.uri();
        let mut crawlers = Crawlers::new("pds.example.com".to_string(), vec![uri.clone()]);
        crawlers.last_notified = 0;

        // First call: no rate-limit guard -> the relay is hit.
        Mock::given(method("POST"))
            .and(path("/xrpc/com.atproto.sync.requestCrawl"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        crawlers.notify_of_update().await.unwrap();
        let last_after_first = crawlers.last_notified;

        // Second call within 20 minutes: still rate-limited.
        Mock::given(method("POST"))
            .and(path("/xrpc/com.atproto.sync.requestCrawl"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        crawlers.notify_of_update().await.unwrap();
        assert_eq!(crawlers.last_notified, last_after_first);
    }
}
