pub mod errors;

use errors::{Error, ErrorKind, Result};
use rsky_syntax::handle::{is_valid_tld, normalize_and_ensure_valid_handle};

/// Service-domain validation does not need the network. This plan's variant
/// takes `&[String]` service domains directly instead of the rocket `State`
/// wrappers of the reference.
pub async fn normalize_and_validate_handle(
    opts: HandleValidationOpts,
    service_handle_domains: &[String],
) -> Result<String> {
    let handle = base_normalize_and_validate(&opts.handle)?;
    if !is_valid_tld(&handle) {
        return Err(Error::new(
            ErrorKind::InvalidHandle,
            "Handle TLD is invalid or disallowed",
        ));
    }
    if rsky_common::explicit_slurs::contains_explicit_slurs(&handle) {
        return Err(Error::new(
            ErrorKind::InvalidHandle,
            "Inappropriate language in handle",
        ));
    }
    if is_service_domain(&handle, service_handle_domains) {
        ensure_handle_service_constraints(
            &handle,
            service_handle_domains,
            opts.allow_reserved.unwrap_or(false),
        )?;
    } else {
        if opts.did.is_none() {
            return Err(Error::new(
                ErrorKind::UnsupportedDomain,
                "Not a supported handle domain",
            ));
        }
        // External (non-service-domain) handles must resolve to the caller's
        // DID. Resolution requires the IdResolver and is wired in the handlers
        // that allow external handles (see Task 21). For the PDS port, only
        // service-domain handles are allowed, mirroring the common deployment.
        return Err(Error::new(
            ErrorKind::UnsupportedDomain,
            "Not a supported handle domain",
        ));
    }
    Ok(handle)
}

pub struct HandleValidationOpts {
    pub handle: String,
    pub did: Option<String>,
    pub allow_reserved: Option<bool>,
}

fn base_normalize_and_validate(handle: &str) -> Result<String> {
    match normalize_and_ensure_valid_handle(handle) {
        Ok(normalized) => Ok(normalized),
        Err(e) => Err(Error::new(ErrorKind::InvalidHandle, &e.to_string())),
    }
}

fn as_dotted_suffix(domain: &str) -> String {
    if domain.starts_with('.') {
        domain.to_string()
    } else {
        format!(".{domain}")
    }
}

fn is_service_domain(handle: &str, available_user_domains: &[String]) -> bool {
    available_user_domains
        .iter()
        .any(|domain| handle.ends_with(as_dotted_suffix(domain).as_str()))
}

fn ensure_handle_service_constraints(
    handle: &str,
    available_user_domains: &[String],
    allow_reserved: bool,
) -> Result<()> {
    let supported_domain = available_user_domains
        .iter()
        .map(|d| as_dotted_suffix(d))
        .find(|domain| handle.ends_with(domain.as_str()))
        .ok_or_else(|| Error::new(ErrorKind::InvalidHandle, "Invalid domain"))?;

    let front = handle[..handle.len() - supported_domain.len()].to_string();

    if front.contains('.') {
        return Err(Error::new(
            ErrorKind::InvalidHandle,
            "Invalid characters in handle",
        ));
    }
    if front.len() < 3 {
        return Err(Error::new(ErrorKind::InvalidHandle, "Handle too short"));
    }
    if front.len() > 18 {
        return Err(Error::new(ErrorKind::InvalidHandle, "Handle too long"));
    }
    if !allow_reserved && RESERVED_SUBDOMAINS.iter().any(|&r| r == front) {
        return Err(Error::new(ErrorKind::HandleNotAvailable, "Reserved handle"));
    }
    Ok(())
}

/// Subset of the git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's `src/handle/reserved.rs` (ATP + common terms).
pub static RESERVED_SUBDOMAINS: &[&str] = &[
    "at", "atp", "plc", "pds", "did", "repo", "tid", "nsid", "xrpc", "lex", "lexicon", "bsky",
    "bluesky", "handle", "www", "admin", "root", "mail", "smtp", "imap", "webmail", "api", "oauth",
    "status", "help", "support", "docs", "blog", "news", "press", "about", "contact", "login",
    "signup", "register", "account", "billing", "payments", "privacy", "terms", "security",
    "abuse", "spam", "legal", "dmca",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn validates_service_domain_handles() {
        let domains = vec![".test".to_string()];
        let handle = normalize_and_validate_handle(
            HandleValidationOpts {
                handle: "alice.test".to_string(),
                did: None,
                allow_reserved: None,
            },
            &domains,
        )
        .await
        .unwrap();
        assert_eq!(handle, "alice.test");
    }

    #[tokio::test]
    async fn rejects_multi_label_service_handles() {
        let domains = vec![".test".to_string()];
        assert!(
            normalize_and_validate_handle(
                HandleValidationOpts {
                    handle: "a.b.test".to_string(),
                    did: None,
                    allow_reserved: None,
                },
                &domains,
            )
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn rejects_reserved_subdomains() {
        let domains = vec![".test".to_string()];
        assert!(
            normalize_and_validate_handle(
                HandleValidationOpts {
                    handle: "admin.test".to_string(),
                    did: None,
                    allow_reserved: None,
                },
                &domains,
            )
            .await
            .is_err()
        );
    }
}
