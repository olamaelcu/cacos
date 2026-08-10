# 11. rsky-oauth security audit

Date: 2026-08-10

## Status

Accepted

## Context

The PDS relies on the git-pinned `rsky-oauth` crate (rev `aee5aec5ad9473d80232beab58ddba25a936298a`, hosted at github.com/olamaelcu/rsky) for every Authorization Server endpoint: pushed authorization request (RFC 9126 PAR), DPoP-bound token issuance (RFC 9449), JWKS, and the metadata document. The rsky-oauth crate is framework-agnostic — it owns only the JOSE primitives, DPoP proof validation, and the state machine that drives PAR -> authorize -> token. cacos wraps it with a poem route set (`pds/src/oauth/routes.rs`) and a SeaORM-backed `OAuthStore` (`pds/src/account/oauth_store.rs`).

This ADR documents the security posture cacos inherits from rsky-oauth unchanged, the open upstream issues that the cacos deployment inherits as known gaps, and the rationale for not forking or patching locally.

## Decision

### What cacos gets from rsky-oauth unchanged

1. **PKCE required for every client.** `rsky-oauth/src/client.rs:131-154` requires both `code_challenge` and `code_challenge_method=S256`; clients that omit either fail PAR validation. The `plain` PKCE method is deliberately rejected — legacy public clients cannot opt into the weaker variant.

2. **S256 only.** The PAR handler matches `Some(challenge), Some(CODE_CHALLENGE_METHOD_S256)`; any other `code_challenge_method` (including `plain` and missing) returns `OAuthError::InvalidRequest`. Token exchange calls `verify_code_challenge` (`rsky-oauth/src/token.rs:51-65`) which recomputes `base64url(SHA-256(verifier))` and refuses mismatches.

3. **Exact-match `redirect_uri` with RFC 8252 §7.3 loopback-port wildcard.** `compare_redirect_uri` (`rsky-oauth/src/client.rs:259-273`) returns `true` when the registered URI is byte-equal to the requested URI; it also accepts the loopback-IP wildcard (127.0.0.1 or `[::1]`, registered without a port, any port matches, scheme/host/path/query equal). Token-endpoint re-validation is enforced via the same helper chain on PAR.

4. **PAR (RFC 9126) required.** The metadata document at `rsky-oauth/src/provider.rs:772-773` advertises `require_pushed_authorization_requests: true`; the authorize endpoint rejects any flow that did not come in through PAR.

5. **DPoP (RFC 9449) required.** Every token request must carry a DPoP proof bound to the client's signing key; the `dpop_jkt` recorded at PAR time is matched against the proof's `jkt` (`rsky-oauth/src/provider.rs:454-462`, `rsky-oauth/src/provider.rs:561`). The replay store (default `InMemoryReplayStore`) consumes `jti` so a captured proof cannot be reused.

6. **`OAuthProviderConfig::trusted_clients` list.** Operators set `PDS_OAUTH_TRUSTED_CLIENTS` to grant non-confidential clients the silent-sign-on flow; everything else gets the consent page.

### Known upstream gaps cacos inherits

The following gaps are open issues against `github.com/olamaelcu/rsky` (the cacos fork). cacos tracks them rather than patching locally because the fixes are upstream-facing and the patch surface (constant-time comparison, parity across `client.rs` and `provider.rs`) is wider than the cacos deployment would justify.

- **`compare_redirect_uri` uses `==` not constant-time.** `rsky-oauth/src/client.rs:260-262` returns `true` for `registered == requested` without going through `subtle::ConstantTimeEq`. A network-local attacker who can submit many candidate redirect URIs against a known registered prefix can probe byte-by-byte. This is the same bug shape as a constant-time-compare regression in a CSRF or session-id check; the threat model for `redirect_uri` is narrower (the attacker needs to influence the client and observe response timing) but not zero.

- **Token-endpoint `redirect_uri` re-check uses `!=` not the helper.** `rsky-oauth/src/provider.rs:447-453` compares `redirect_uri != data.parameters.redirect_uri` directly. Two issues:
  1. It does not go through `compare_redirect_uri`, so a client that registered `http://127.0.0.1:3000/cb` (no port in the registered form via the loopback wildcard) and presents `http://127.0.0.1:9000/cb` at the token endpoint will be accepted if the byte-equality short-circuit was bypassed. Today the cacos deployment inherits the upstream behavior.
  2. Like `compare_redirect_uri`, the `!=` is not constant-time.

- **No `plain` PKCE for legacy clients.** `rsky-oauth/src/client.rs:131-154` deliberately rejects `code_challenge_method=plain`. This is documented as a security choice rather than a gap; we list it here so the cacos maintainers know that a future AT Protocol client that requires `plain` cannot be onboarded without an upstream change.

### Mitigations cacos applies on top

- **`subtle::ConstantTimeEq` for `PDS_OAUTH_TRUSTED_CLIENTS` membership.** `pds/src/oauth/mod.rs::is_trusted_oauth_client` walks the configured set with `subtle::ConstantTimeEq` so the operator-configured allow-list cannot leak via timing. This is independent of the upstream `redirect_uri` issue and protects the silent-sign-on path from local attackers.

- **SSRF hardening on client-metadata fetches.** `pds/src/oauth/fetcher.rs::HttpClientMetadataFetcher` enforces https-only URLs, 10 s timeout, no redirects, `application/json` content-type, and a 512 KiB response cap. Client metadata is fetched once per `client_id` and cached.

- **SSRF hardening on the PLC client.** `pds/src/plc/mod.rs::HttpPlcClient` resolves the configured `PDS_PLC_URL` to IPs and refuses every IP in 127.0.0.0/8, 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, 169.254.0.0/16, ::1, fc00::/7, fe80::/10. This blocks the operator-misconfiguration path that would otherwise let `PDS_PLC_URL=http://169.254.169.254` reach the AWS metadata service.

- **`HttpPlcClient` only — no SSRF in `OAuthProvider`.** The PAR -> authorize -> token flow only calls rsky-oauth against already-validated URLs; the fetcher is the only outbound HTTP surface cacos adds.

### Decision

cacos inherits `rsky-oauth` unchanged at the pinned commit `aee5aec5ad9473d80232beab58ddba25a936298a`. The three known gaps above are tracked upstream at github.com/olamaelcu/rsky. cacos does not fork or patch the crate locally because:

1. The redirect_uri timing channel is a narrow threat model (local attacker who can already influence the client request).
2. The token-endpoint `!=` issue is a code-correctness gap that requires the upstream helper to handle the loopback wildcard consistently; a local fork would diverge from rsky.
3. Patching `rsky-oauth` locally forces a fork-and-rebase path that breaks the workspace-level dependency contract (the workspace pins the commit in `Cargo.toml`'s `[workspace.dependencies]`).

A future ADR will revisit this decision if upstream ships a fix or if cacos sees exploitation evidence.

## Consequences

- cacos gets framework-agnostic OAuth provider semantics for free (PAR + DPoP + PKCE-S256 + JWKS) and does not need to maintain its own JOSE primitives.
- cacos inherits three known gaps from rsky-oauth (`==`/`!=` constant-time, loopback wildcard at token endpoint, no `plain` PKCE). These are documented for operators and tracked upstream.
- `pds/Cargo.toml` continues to pin `rsky-oauth` to the cacos fork at the existing rev; any local patch would require a fork-and-rebase of the whole `rsky-*` family of crates.
- `pds/src/oauth/mod.rs::is_trusted_oauth_client` (added in this audit) protects the operator-configured silent-sign-on allow-list from timing attacks. The implementation uses `subtle::ConstantTimeEq` and materialises the set via `OnceLock<HashSet<String>>` so the hot path is allocation-free.
- `pds/src/oauth/fetcher.rs::HttpClientMetadataFetcher` and `pds/src/plc/mod.rs::HttpPlcClient` together cover the two outbound HTTP surfaces cacos adds; both enforce scheme, timeout, content-type, and size limits.
- `pds/tests/oauth_security.rs` pins the SSRF guard-rails on the PLC client (loopback / RFC1918 / link-local / IPv6 ULA) so the deny list cannot regress.
- Open issues: the three rsky-oauth gaps above (constant-time compare, token-endpoint wildcard, `plain` PKCE) are tracked at github.com/olamaelcu/rsky.