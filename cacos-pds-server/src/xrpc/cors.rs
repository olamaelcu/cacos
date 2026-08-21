//! CORS origin allowlist policy.
//!
//! [`CorsPolicy::from_config`] reads the allowlist from a
//! `ServerConfig`-style [`CorsConfig`]; if empty it falls back to echoing
//! the configured `public_url` origin, and if the URL has no parseable
//! origin it falls through to [`CorsPolicy::DenyAll`].
//!
//! [`CorsPolicy::allows`] is the single source of truth for whether a
//! given request `Origin` header value should be reflected in
//! `Access-Control-Allow-Origin`.

use std::collections::HashSet;

#[derive(Debug, Clone)]
pub enum CorsPolicy {
    /// Explicit operator allowlist.
    Allowlist(HashSet<String>),
    /// No allowlist configured: echo the public-URL origin so the PDS's
    /// own web surfaces can call it. Any other origin is denied.
    EchoPublicUrl(String),
    /// No allowlist and no parseable public URL: deny every origin.
    DenyAll,
}

impl CorsPolicy {
    /// Build the policy from a `ServerConfig`-style `CorsConfig`. Empty
    /// `allowed_origins` falls back to echoing the configured `public_url`
    /// origin (or denying all when the URL has no parseable origin).
    pub fn from_config(config: &cacos_pds_core::config::CorsConfig, public_url: &str) -> Self {
        if !config.allowed_origins.is_empty() {
            return Self::Allowlist(config.allowed_origins.clone());
        }
        match parse_origin(public_url) {
            Some(origin) => Self::EchoPublicUrl(origin),
            None => Self::DenyAll,
        }
    }

    pub fn allows(&self, origin: Option<&str>) -> bool {
        let Some(o) = origin else {
            return false;
        };
        match self {
            Self::Allowlist(set) => set.iter().any(|a| a == o),
            Self::EchoPublicUrl(p) => p == o,
            Self::DenyAll => false,
        }
    }
}

fn parse_origin(url: &str) -> Option<String> {
    let scheme_end = url.find("://")?;
    let (scheme, rest) = url.split_at(scheme_end);
    let rest = &rest[3..];
    let host_end = rest.find('/').unwrap_or(rest.len());
    let authority = &rest[..host_end];
    if authority.is_empty() {
        return None;
    }
    Some(format!("{scheme}://{authority}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_matches_exact_origins() {
        let mut set = HashSet::new();
        set.insert("https://app.example".to_string());
        let policy = CorsPolicy::Allowlist(set);
        assert!(policy.allows(Some("https://app.example")));
        assert!(!policy.allows(Some("https://other.example")));
        assert!(!policy.allows(None));
    }

    #[test]
    fn echo_public_url_matches_only_self() {
        let policy = CorsPolicy::EchoPublicUrl("https://pds.example".to_string());
        assert!(policy.allows(Some("https://pds.example")));
        assert!(!policy.allows(Some("https://attacker.example")));
        assert!(!policy.allows(None));
    }

    #[test]
    fn deny_all_rejects_everything() {
        let policy = CorsPolicy::DenyAll;
        assert!(!policy.allows(Some("https://anything.example")));
        assert!(!policy.allows(None));
    }

    #[test]
    fn parse_origin_strips_path() {
        assert_eq!(
            parse_origin("https://pds.example/foo/bar"),
            Some("https://pds.example".to_string())
        );
        assert_eq!(
            parse_origin("http://localhost:8080"),
            Some("http://localhost:8080".to_string())
        );
        assert_eq!(parse_origin("not a url"), None);
        assert_eq!(parse_origin(""), None);
    }
}
