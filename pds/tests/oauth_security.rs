//! OAuth/PLC security regression tests.
//!
//! Pins the SSRF guard-rails on [`cacos_pds::plc::HttpPlcClient`] so the
//! loopback / RFC1918 / link-local deny list cannot regress.
//!
//! Each test points the client at a denied IP literal and asserts the
//! SSRF check rejects before the request goes out — the test does not
//! depend on a real listener at the target address because the check
//! runs synchronously on the host string.

use cacos_pds::plc::{HttpPlcClient, PlcClient};

fn build(endpoint: &str) -> HttpPlcClient {
    HttpPlcClient::new(endpoint.to_string()).expect("test setup: build HttpPlcClient")
}

#[tokio::test]
async fn http_plc_client_blocks_loopback_ip() {
    // 127.0.0.1 resolves to a deny-listed IPv4 loopback. The client
    // refuses before any socket I/O happens.
    let client = build("http://127.0.0.1:2583");
    let err = client
        .get_document_data("did:plc:abcd")
        .await
        .expect_err("loopback IP must be rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("denied IP"),
        "error must reference the SSRF denial path, got: {msg}"
    );
}

#[tokio::test]
async fn http_plc_client_blocks_loopback_subnet_ip() {
    // 127.0.0.0/8 is the entire loopback range, not just 127.0.0.1.
    let client = build("http://127.255.0.1:2583");
    let err = client
        .get_document_data("did:plc:abcd")
        .await
        .expect_err("loopback range must be rejected");
    assert!(err.to_string().contains("denied IP"));
}

#[tokio::test]
async fn http_plc_client_blocks_rfc1918_ip() {
    // 192.168.0.0/16 RFC1918 must be refused.
    let client = build("http://192.168.1.1:2583");
    let err = client
        .get_document_data("did:plc:abcd")
        .await
        .expect_err("RFC1918 IP must be rejected");
    assert!(err.to_string().contains("denied IP"));
}

#[tokio::test]
async fn http_plc_client_blocks_rfc1918_10_ip() {
    // 10.0.0.0/8 RFC1918 must be refused.
    let client = build("http://10.0.0.1:2583");
    let err = client
        .get_document_data("did:plc:abcd")
        .await
        .expect_err("RFC1918 10.0.0.0/8 must be rejected");
    assert!(err.to_string().contains("denied IP"));
}

#[tokio::test]
async fn http_plc_client_blocks_rfc1918_172_ip() {
    // 172.16.0.0/12 RFC1918 must be refused.
    let client = build("http://172.16.5.5:2583");
    let err = client
        .get_document_data("did:plc:abcd")
        .await
        .expect_err("RFC1918 172.16.0.0/12 must be rejected");
    assert!(err.to_string().contains("denied IP"));
}

#[tokio::test]
async fn http_plc_client_blocks_link_local_ip() {
    // 169.254.0.0/16 link-local must be refused; this includes the AWS
    // metadata service at 169.254.169.254.
    let client = build("http://169.254.169.254:80");
    let err = client
        .get_document_data("did:plc:abcd")
        .await
        .expect_err("link-local IP must be rejected");
    assert!(err.to_string().contains("denied IP"));
}

#[tokio::test]
async fn http_plc_client_blocks_ipv6_loopback() {
    // ::1 IPv6 loopback must be refused.
    let client = build("http://[::1]:2583");
    let err = client
        .get_document_data("did:plc:abcd")
        .await
        .expect_err("IPv6 loopback must be rejected");
    assert!(err.to_string().contains("denied IP"));
}

#[tokio::test]
async fn http_plc_client_blocks_ipv6_ula() {
    // fc00::/7 IPv6 unique-local must be refused.
    let client = build("http://[fd00::1]:2583");
    let err = client
        .get_document_data("did:plc:abcd")
        .await
        .expect_err("IPv6 ULA must be rejected");
    assert!(err.to_string().contains("denied IP"));
}

#[tokio::test]
async fn http_plc_client_blocks_ipv6_link_local() {
    // fe80::/10 IPv6 link-local must be refused.
    let client = build("http://[fe80::1]:2583");
    let err = client
        .get_document_data("did:plc:abcd")
        .await
        .expect_err("IPv6 link-local must be rejected");
    assert!(err.to_string().contains("denied IP"));
}

#[tokio::test]
async fn http_plc_client_rejects_unparseable_endpoint() {
    // A non-URL endpoint must surface a clear error rather than silently
    // falling back to the SSRF-bypassing default.
    let client = build("not a url");
    let err = client
        .get_document_data("did:plc:abcd")
        .await
        .expect_err("non-URL endpoint must be rejected");
    assert!(err.to_string().contains("invalid PLC endpoint URL"));
}

#[tokio::test]
async fn http_plc_client_blocks_unknown_hostname_with_public_ip() {
    // Construct a URL with a hostname that resolves to a public IP; the
    // SSRF guard is host-based via DNS resolution, so an unresolvable
    // hostname surfaces a clear DNS failure rather than bypassing the
    // guard.
    let client = build("http://plc.test.invalid");
    let err = client
        .get_document_data("did:plc:abcd")
        .await
        .expect_err("unresolvable hostname must surface a DNS error");
    let msg = err.to_string();
    assert!(
        msg.contains("DNS resolution failed") || msg.contains("denied IP"),
        "expected DNS or denial message, got: {msg}"
    );
}
