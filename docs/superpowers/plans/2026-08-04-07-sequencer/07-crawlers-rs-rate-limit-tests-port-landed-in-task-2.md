# Task 7: `crawlers.rs` — rate-limit tests (port landed in Task 2)

**Files:**
- Modify: `pds/src/sequencer/crawlers.rs` (append `#[cfg(test)] mod tests`)
- Modify: `pds/Cargo.toml` (dev-dep `wiremock`)

The `Crawlers` port itself landed in Task 2 (it is a hard dependency of `sequence_evt`). This task adds the tests the spec calls for: the reference's `crawl_request_advertises_pds_hostname` unit test, plus a rate-limit integration test that hits a `wiremock` HTTP server so the reqwest call stays out of the network. No production code changes — these characterize the Task 2 port; if the rate limiter regressed, they fail.

- [ ] **Step 1: Add the dev-dependency to `pds/Cargo.toml`**

```toml
# [dev-dependencies] — add
wiremock = "0.6"
```

- [ ] **Step 2: Append the test module to `pds/src/sequencer/crawlers.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

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
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/xrpc/com.atproto.sync.requestCrawl"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        let mut crawlers = Crawlers::new("pds.example.com".to_owned(), vec![mock_server.uri()]);

        // first call: the window is open, one request goes out
        crawlers.notify_of_update().await.unwrap();
        // second call within the 20-minute window: suppressed
        crawlers.notify_of_update().await.unwrap();

        // simulate 20 minutes passing (last_notified is a public field)
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("timestamp in millis since UNIX epoch")
            .as_millis() as usize;
        crawlers.last_notified = now - NOTIFY_THRESHOLD as usize - 1;
        crawlers.notify_of_update().await.unwrap();

        let count = mock_server
            .received_requests()
            .await
            .expect("mock server stopped")
            .iter()
            .filter(|req| req.url.path().ends_with("com.atproto.sync.requestCrawl"))
            .count();
        assert_eq!(count, 2);
    }
}
```

- [ ] **Step 3: Run the tests to verify they pass**

Run: `cargo test -p cacos-pds sequencer::crawlers::tests`
Expected: `test result: ok. 2 passed` (both characterize the Task 2 port; the rate-limit test performs 2 real HTTP round-trips against the wiremock server, not the network).

- [ ] **Step 4: Run the full sequencer suite once more**

Run: `cargo test -p cacos-pds sequencer:: xrpc::com::atproto::sync::`
Expected: all sequencer, ws_frames, outbox, apalis_worker, crawlers, and subscribe_repos tests pass.

- [ ] **Step 5: Commit**

```bash
git add pds/Cargo.toml pds/src/sequencer/crawlers.rs
git commit -m "test(sequencer): crawler rate-limit and hostname tests via wiremock"
```
