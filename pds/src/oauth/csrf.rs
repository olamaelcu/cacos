//! Keyed CSRF tokens for a cookie-authenticated consent UI.
//!
//! # Status: primitive only — deliberately not wired to `/oauth/remote/*`
//!
//! The `/oauth/remote/*` endpoints are **server-to-server**: the single
//! configured RemoteClient calls them from its backend with
//! `Authorization: Bearer <PDS_OAUTH_REMOTE_CLIENT_TOKEN>` and passes
//! `device_id` in the JSON body. No browser ever POSTs to the PDS, and the
//! PDS sets no cookie on that path, so those requests carry no ambient
//! credential for a cross-origin page to abuse — the precondition for CSRF
//! simply is not present. See the headless-consent design spec:
//!
//! > No PDS-set cookies, no browser-form CSRF at this layer. The browser
//! > never POSTs to the PDS. CSRF is the RemoteClient's concern within its
//! > own session.
//!
//! Requiring a CSRF cookie on those handlers would reject every legitimate
//! RemoteClient call (servers do not send browser cookies) and take the
//! whole consent flow down while defending against nothing. Replay of a
//! leaked consent URL is already handled by the rotating one-time `state`
//! nonce in [`crate::db::consent_state`].
//!
//! These helpers exist for a future consent surface that the PDS serves
//! *directly to a browser* with a session cookie. They are a strict
//! improvement over the unkeyed [`crate::oauth::csrf_token`] (which hashes
//! the cookie with bare SHA-256 and is likewise unused): the token is a
//! keyed HMAC, so it cannot be forged from a known device id, and
//! [`verify`] compares in constant time.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

/// HMAC-SHA256 over the device id, keyed by the server secret.
fn tag(device_id: &str, secret: &[u8]) -> Vec<u8> {
    let mut mac =
        <Hmac<Sha256> as Mac>::new_from_slice(secret).expect("HMAC accepts a key of any length");
    mac.update(device_id.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

/// Mints the CSRF token bound to `device_id`.
pub fn issue(device_id: &str, secret: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(tag(device_id, secret))
}

/// Constant-time check that `token` was issued for `device_id` under
/// `secret`. Returns `false` for malformed base64 rather than erroring, so
/// callers cannot accidentally distinguish "bad encoding" from "bad tag".
pub fn verify(device_id: &str, token: &str, secret: &[u8]) -> bool {
    let Ok(decoded) = URL_SAFE_NO_PAD.decode(token) else {
        return false;
    };
    tag(device_id, secret)
        .as_slice()
        .ct_eq(decoded.as_slice())
        .into()
}

/// `__Host-`-prefixed cookie name on HTTPS (the browser then pins it to the
/// host: no `Domain`, `Path=/`, `Secure`). Plain HTTP cannot use the prefix
/// because it mandates `Secure`. Mirrors
/// [`crate::oauth::device_cookie_name`].
pub fn cookie_name(public_url: &str) -> &'static str {
    if public_url.starts_with("https://") {
        "__Host-csrf-token"
    } else {
        "csrf-token"
    }
}

/// Whether the CSRF cookie should be marked `Secure`.
pub fn cookie_secure(public_url: &str) -> bool {
    public_url.starts_with("https://")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &[u8] = b"unit-test-csrf-secret";

    #[test]
    fn cookie_name_uses_host_prefix_only_on_https() {
        assert_eq!(cookie_name("https://pds.example"), "__Host-csrf-token");
        assert!(cookie_secure("https://pds.example"));
        assert_eq!(cookie_name("http://localhost:2583"), "csrf-token");
        assert!(!cookie_secure("http://localhost:2583"));
    }

    #[test]
    fn token_is_not_forgeable_without_the_secret() {
        let token = issue("dev-1", SECRET);
        assert!(!verify("dev-1", &token, b"different-secret"));
    }
}
