# 12. cacos-pds security posture (R1-R13)

Date: 2026-08-10

## Status

Accepted

## Context

The cacos personal data server (PDS) exposes three authentication surfaces
that share one TCP listener but resolve to different trust boundaries:

1. **Legacy bearer JWT** — `Authorization: Bearer <jwt>` against the
   `PDS_JWT_KEY_K256_PRIVATE_KEY_HEX`-signed access / refresh token, validated
   by `pds/src/auth/auth_verifier.rs::validate_bearer_access_token`. Used by
   pre-OAuth clients and by the bearer-only `AccessStandard` / `AccessFull` /
   `AccessPrivileged` extractors.
2. **OAuth DPoP** — `Authorization: DPoP <access-token>` bound to a DPoP
   proof (RFC 9449) validated by
   `rsky_oauth::OAuthProvider::verify_access_token`. Used by every modern
   client that has migrated off the legacy bearer path.
3. **Admin basic-auth** — `Authorization: Basic <admin:secret>` against
   `PDS_ADMIN_TOKEN_<NAME>_{SECRET,SCOPES,NAME}` (or the legacy
   `PDS_ADMIN_PASSWORD` fallback). Used by privileged endpoints: invite-code
   issuance, account takedown / deletion, repo takedown. Validated by
   `auth_verifier::verify_admin_token` against the
   `AdminTokenRegistry`.

The security audit closed in this PR spans three waves of preceding
commits plus the R1-R13 work in this PR:

### Cumulative state from Wave 1-3 commits

- **5189e18** *feat(security): SSRF-hardened PLC client and mock gating.*
  `HttpPlcClient` rejects loopback / RFC1918 / link-local / IPv6 ULA /
  link-local so a misconfigured `PDS_PLC_URL=http://169.254.169.254`
  cannot reach the AWS metadata service. Mock clients are gated behind
  `--features test-utils` so a production build never bundles them.
- **ef02a57** *feat(security): secret hardening helpers and constant-time
  admin auth.* `read_secret` (`*_FILE` indirection), `validate_password_strength`
  (length + class diversity + deny-list), `AdminTokenRegistry` with
  `subtle::ConstantTimeEq` for both username and secret compare. Trusted
  OAuth clients use a constant-time membership check.
- **a0c7639** *feat(security): per-IP rate limiting, per-account login
  lockout, password-reset enumeration defense.* `RouteRateLimit` middleware
  with governor / DashMapStateStore keyed by peer IP for createSession /
  serverCreateAccount / requestPasswordReset / resetPassword /
  requestEmailConfirmation / requestEmailUpdate; per-account
  `failedLoginCount` / `lockedUntil`; response-shape parity between
  existing-account and missing-account flows so a reset call cannot
  enumerate which identifiers are taken.
- **2c1bf4b** *feat(security): wrap key material in secrecy::SecretBox for
  at-rest hygiene.* `PDS_JWT_KEYPAIR`, `PDS_REPO_SIGNING_KEYPAIR`,
  `PDS_PLC_ROTATION_KEYPAIR` (the global fallback; per-DID keys ship in
  2406425), and `PDS_DPOP_SECRET` now live behind `secrecy::SecretBox`
  wrappers so an accidental `format!("{key:?}")` cannot leak the bytes.
- **2406425** *feat(security): per-DID PLC rotation keys with migration
  tooling.* Each new account persists its own rotation keypair; legacy
  accounts are backfilled by `cacos-pds migrate rotation-keys` /
  `migrate plc-rotation-keys [--dry-run]`. The PLC sign path prefers the
  per-DID key with a global fallback for the migration window.
- **44ce845** *feat(security): wire OAuth provider, CORS allowlist, cookie
  hardening.* Carries R1 (OAuth provider registration), R2 (CORS
  allowlist), and R6 (cookie hardening) below.

### This PR

- **R1-R7** closes the OAuth / CORS / admin / cookie gaps that Wave 1-3
  did not touch.
- **R11-R13** adds a pluggable secret backend, keyed CSRF primitives for
  the headless-consent flow, and the upstream rsky-oaudit (ADR 0011).

The R-numbering below is the project-internal security-audit identifier
(not the same as the Wave 1-3 commit-message identifiers).

## Decision

### Mitigations shipped in this PR

#### R1 — OAuth provider registration (44ce845)

`bootstrap_oauth_app` now returns
`OAuthBootstrap { endpoint, provider: Arc<OAuthProvider> }`; `build_app_with_state`
calls `register_auth_dependencies` with the provider before nesting the
endpoint. The `OAUTH_PROVIDER` RwLock overwrite on the second registration
is intentional and safe; the test path still works because
`SharedStateFromEnv::from_env` retains the previous
`register_auth_dependencies(None)` call.

#### R2 — CORS allowlist with public_url fallback (44ce845)

`pds/src/xrpc/cors.rs` defines `CorsPolicy::{Allowlist, EchoPublicUrl,
DenyAll}`. `PDS_CORS_ALLOWED_ORIGINS` takes precedence; otherwise the
request `Origin` must match the origin of `PDS_PUBLIC_URL` exactly;
otherwise no `Access-Control-Allow-Origin` header is emitted (credentialed
cross-origin rejected). Wildcard CORS is gone.

#### R3 — Per-IP rate limiting + login lockout + reset enumeration defense (a0c7639)

`RouteRateLimit` middleware using the `governor` crate's
`DashMapStateStore` keyed by peer IP. Defaults: 10/min for createSession
and serverCreateAccount, 5/min for password-reset and email-ops. Zero
disables the limiter. Per-account `failedLoginCount` / `lockedUntil`
protects against brute force; reset-flow responses are shape-parity
between existing and missing accounts so identifiers cannot be enumerated.

#### R4 — secrecy::SecretBox wrapping for key material (2c1bf4b)

`PDS_JWT_KEYPAIR`, `PDS_REPO_SIGNING_KEYPAIR`, `PDS_PLC_ROTATION_KEYPAIR`,
and `PDS_DPOP_SECRET` are wrapped in `secrecy::SecretBox`. Public static
types (`ES256kKeyPair`, `secp256k1::Keypair`) are unchanged so the call
sites are not affected.

#### R5 — Per-DID PLC rotation keys (2406425)

`ActorStore` gains `rotation_keypair(did)` / `create_rotation_keypair(did)`
/ `write_rotation_key_from_bytes(did, secret_bytes)` etc.; `create_account`
persists a fresh keypair per new DID; legacy accounts are backfilled by
the CLI migration. The PLC sign path prefers the per-DID key with a
global fallback during the migration window.

#### R6 — Cookie hardening (44ce845)

`pds/src/oauth/mod.rs` gains `device_cookie_name` /
`device_cookie_secure`. `ensure_device_session` now takes the public URL,
sets `Secure=true` when `PDS_PUBLIC_URL` starts with `https://`, and
renames the cookie to `__Host-device-id` (the RFC 6265bis prefix required
for the `Secure` flag). `Path=/` is pinned when the `__Host-` prefix is
in use. The same helper applies to any csrf-cookie construction.

#### R7 — Admin scope gating (this PR)

`Credentials.admin_scopes: Option<AdminScopeSet>` (in
`pds/src/auth/auth_verifier.rs:140`) is populated by
`verify_admin_token` from the configured `AdminTokenRegistry`. Three new
extractors in `pds/src/xrpc/auth_extractors.rs` —
`RequireInviteAdmin`, `RequireAccountAdmin`, `RequireTakedownAdmin` —
compose an `AdminToken` check with a scope gate on
`Credentials.admin_scopes`. The invite-code endpoints
(`com.atproto.server.createInviteCode[s]`) switch to `RequireInviteAdmin`;
`com.atproto.server.deleteAccount` switches to `RequireAccountAdmin`. A
token whose configured scope set does not include the requested action
fails closed with a 401 `AuthRequiredError` carrying the missing-scope
hint. Wildcard-scope entries grant every checked scope by definition.

#### R11 — Pluggable SecretProvider (this PR)

`pds/src/account/helpers/secret_provider.rs` defines a `SecretProvider`
trait with three impls: `EnvSecretProvider` (default; the existing
`*_FILE` indirection), `FileSecretProvider` (`PDS_SECRET_BACKEND=file:<dir>`),
and `KmsSecretProvider` (`PDS_SECRET_BACKEND=kms`). The KMS impl
deliberately refuses every read with `SecretError::KmsUnavailable` —
silently falling back to the environment would defeat the point of
selecting a managed backend. `SecretProvider::read_with_kms` is the
forward-compatibility hook for envelope-encrypted secrets; the default
ignores `kms_key_id` and defers to `read`.

`PDS_SECRET_BACKEND` is read once and cached behind a
`OnceLock<Mutex<Option<Arc<dyn SecretProvider>>>>`. Misconfigured
backend selections are *not* cached, so correcting the env and retrying
works without a restart.

#### R12 — Keyed CSRF primitives for the headless-consent flow (this PR)

`pds/src/oauth/csrf.rs` exposes `issue` / `verify` over HMAC-SHA256 keyed
by the server secret, plus `cookie_name` / `cookie_secure` that mirror
the device-cookie helpers (the `__Host-csrf-token` prefix on HTTPS,
`Path=/`, `Secure`). `verify` uses `subtle::ConstantTimeEq` and returns
`false` for malformed base64 rather than erroring, so callers cannot
distinguish "bad encoding" from "bad tag".

> **Status: primitive only — deliberately not wired to
> `/oauth/remote/*`.** The headless-consent endpoints are
> **server-to-server**: the single configured RemoteClient calls them
> from its backend with `Authorization: Bearer <PDS_OAUTH_REMOTE_CLIENT_TOKEN>`
> and passes `device_id` in the JSON body. No browser ever POSTs to the
> PDS, and the PDS sets no cookie on that path, so those requests carry
> no ambient credential for a cross-origin page to abuse — the
> precondition for CSRF simply is not present. Replay of a leaked consent
> URL is already handled by the rotating one-time `state` nonce in
> `crate::db::consent_state`. The primitive is staged here so a future
> browser-served consent surface can adopt it without a follow-up audit.

`/oauth/remote/*` is also wrapped in a per-IP `RouteRateLimit` (default
`PDS_RATELIMIT_OAUTH_REMOTE_PER_MINUTE=10`, clamped to a minimum of
1/min) so a hostile RemoteClient call cannot flood unauthenticated
requests past the `TokenGuard`.

#### R13 — rsky-oauth security audit / ADR 0011 (this PR)

`docs/adr/0011-rsky-oauth-audit.md` documents the posture cacos inherits
from `rsky-oauth` (PKCE-S256 required, exact-match `redirect_uri` with
RFC 8252 §7.3 loopback wildcard, PAR + DPoP required), the three open
upstream gaps (constant-time compare, token-endpoint wildcard,
`plain` PKCE deliberately rejected), and the mitigations cacos applies on
top (`subtle::ConstantTimeEq` for `PDS_OAUTH_TRUSTED_CLIENTS`,
SSRF-hardened client-metadata fetcher, SSRF-hardened PLC client).

### Known limitations deferred

The following items are tracked as deferred rather than shipped because
they need infrastructure or operator coordination that is out of scope
for this audit:

- **R5 — no TLS enforcement at server bind.** cacos binds plain HTTP
  and expects TLS termination at a reverse proxy (nginx, Caddy, a cloud
  load-balancer). The cleartext password auth path
  (`com.atproto.server.createSession`) therefore depends on the proxy
  terminating TLS before the request reaches the listener. Operators
  fronting the PDS directly with a raw TCP listener expose every
  password to the network. A future change should add an opt-in
  `rustls` listener (and a `PDS_REQUIRE_TLS=true` panic at boot when
  the proxy path is not configured).
- **R8 — CORS preflight cross-browser testing.** `CorsPolicy` is unit
  tested for allowlist matching and exact-origin echo, but the
  preflight round-trip across Chromium / Firefox / Safari is not yet
  pinned by an integration test. A future change should add a
  Playwright test matrix.
- **R9 — distributed rate limit via Redis.** `RouteRateLimit` is
  per-process (`DashMapStateStore`). Horizontal-scale deployments see
  the union of per-replica limits, not the operator-configured
  per-route budget. A future change should swap in a Redis-backed store
  with the same governor semantics.
- **R10 — distributed session state.** Refresh-token and DPoP-nonce
  state live in `account.sqlite` and the in-memory replay store
  respectively. The session path works behind a single PDS but a
  restart loses DPoP nonces (acceptable: nonces are short-lived and
  rotate); horizontal scale loses refresh-token visibility across
  replicas. A future change should externalise both behind a shared
  store (Redis or the same `account.sqlite` mounted on a shared volume).

## Consequences

### Positive

- **OAuth is functional end-to-end.** `Authorization: DPoP <access-token>`
  reaches the registered provider and validates against the DPoP proof
  (RFC 9449) with the actual nonce rotation. Pre-R1 builds returned
  `InternalServerError("OAuth provider is not configured")` on every
  DPoP request.
- **The CORS hole is closed.** Wildcard `Access-Control-Allow-Origin`
  with `Allow-Credentials: true` is gone; the browser will reject
  credentialed cross-origin requests against an unconfigured origin
  before the request body is parsed.
- **Deployment errors are loud.** `SecretError::NotFound`,
  `SecretError::KmsUnavailable`, and an unknown `PDS_SECRET_BACKEND`
  panic at boot rather than silently falling back to the environment.
  `register_auth_dependencies(None)` followed by a build_app call
  fails closed at first DPoP request, not at startup.
- **DPoP replay survives restart + scales.** The default
  `InMemoryReplayStore` consumes `jti` so a captured proof cannot be
  reused; on restart the in-memory state clears, which is acceptable
  because the nonces are short-lived. (See R10 for the multi-replica
  path.)
- **Cookies are hardened.** `__Host-` prefix + `Secure` + `Path=/` on
  HTTPS deployments; legacy name + `Secure` off on HTTP for
  development. CSRF cookie helper applies the same shape.
- **Admin routes are scoped.** A token configured with only
  `TakedownAdmin` cannot issue invite codes; a token configured with
  only `InviteAdmin` cannot delete an account. The legacy
  `PDS_ADMIN_PASSWORD` default still grants `Wildcard` for backward
  compatibility.
- **Rate limiting covers the credential-mutating path.** Per-IP limits
  on createSession, serverCreateAccount, requestPasswordReset,
  resetPassword, requestEmailConfirmation, requestEmailUpdate, and the
  headless-consent POST endpoints (`/oauth/remote/*`).
- **CSRF primitives are ready for a future browser-served consent
  surface.** The keyed-HMAC design replaces the unkeyed SHA-256
  primitive in `crate::oauth::csrf_token` (also unused). Forward-
  compatible with envelope-encrypted secrets via `read_with_kms`.
- **Pluggable SecretProvider** lets a future KMS / Vault / HSM
  integration slot in without touching the call sites that read
  secrets — they keep going through `read_secret`. The KMS impl
  refuses reads today rather than falling back to env, so an operator
  who selects `kms` gets a loud failure instead of a quiet
  bypass.

### Negative

- **Single-PDS-instance rate limiting.** `governor` + `DashMapStateStore`
  is per-process; horizontal scale multiplies the budget by the replica
  count. (R9.)
- **No HSM integration today — only the trait.** `KmsSecretProvider`
  exists as a placeholder; selecting it returns `KmsUnavailable` for
  every read. The wiring for AWS KMS / GCP KMS / Vault lives behind a
  follow-up change.
- **`PDS_JWT_KEY_K256_PRIVATE_KEY_HEX` alias still tolerated.** The
  audit did not delete the legacy env name; a future change should
  consolidate on the canonical `PDS_*` form once the runbook is
  updated.
- **Cleartext password auth still relies on TLS-terminating proxy.**
  `createSession` accepts a plaintext password over HTTP; the PDS
  binds plain HTTP. An operator who fronts the PDS directly exposes
  every password. (R5-deferred.)

## Rollout

- **Operators running pre-R1 builds MUST upgrade for OAuth to function.**
  Pre-R1 builds return `InternalServerError("OAuth provider is not
  configured")` on every DPoP request; the only working auth path is
  the legacy bearer JWT. OAuth-bound clients (any modern client) will
  not authenticate against a pre-R1 build.
- **Wildcard-CORS deployments MUST set `PDS_CORS_ALLOWED_ORIGINS`
  explicitly.** R2 deletes the wildcard `Access-Control-Allow-Origin`
  fallback. Operators who relied on a permissive CORS policy must
  enumerate the allowed origins in `PDS_CORS_ALLOWED_ORIGINS`
  (comma-separated) or rely on the `PDS_PUBLIC_URL`-exact fallback.
  The pre-R1 permissive default is gone.
- **Horizontal-scale deployments MUST persist `account.sqlite` to a
  shared volume.** `account.sqlite` is the only backing store for
  refresh tokens, OAuth client registrations, PLC rotation keys (per
  DID), and the actor store. A horizontally scaled PDS without a
  shared volume sees inconsistent auth state across replicas and may
  double-issue refresh tokens. (R10 tracks the Redis / shared-store
  migration.)
- **Deployments wanting to avoid boot panics MUST set every required
  env var up-front.** `SecretError::NotFound`, `SecretError::KmsUnavailable`,
  an unknown `PDS_SECRET_BACKEND`, and `OAuth provider is not configured`
  are now loud failures. Operators upgrading from pre-R1 builds must
  audit their env file against the runbook before deploying; the
  pre-R1 "best-effort fallback" behavior is gone.
