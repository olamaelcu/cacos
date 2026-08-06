# 7. OAuth provider and auth verifier over sea-orm entities

Date: 2026-08-05

## Status

Accepted

## Context

The PDS exposes an OAuth 2.0 authorization server (PAR, token, revoke, JWKS, and
the two well-known metadata documents) plus per-request auth verification for
the XRPC surface. The authorization and consent flow is headless: a single
operator-configured RemoteClient owns HTML rendering, and the PDS exposes a
token-authenticated JSON API the RemoteClient drives step by step. Account
sessions, authorization requests, tokens, devices, and consent state must
persist across the four SQLite databases introduced in ADR-0002, using typed
columns (`DbId` ULIDs, `Did`, `OffsetDateTime`).

## Decision

1. **Entity-based data access, not raw SQL.** The OAuth store is implemented
   with the sea-orm entity API over the typed columns from ADR-0002. Sea-orm's
   `exec_with_returning` provides the atomic consume-once semantics that a
   `DELETE ... RETURNING` statement would otherwise require, and transactions
   keep refresh-token rotation replay-safe.

2. **Surrogate `DbId` primary keys with companion TEXT keys.** The store
   generates `DbId` ULIDs for surrogate primary keys, and each table carries a
   TEXT UNIQUE column holding the externally-meaningful identifier that flows
   through the API (`requestId` on authorization requests, `deviceId` on
   devices, mirroring `tokenId` on tokens). Internal foreign keys reference the
   surrogate `DbId`; the store resolves the external TEXT key to the `DbId` at
   the boundary. This keeps the typed-column discipline from ADR-0002 while
   preserving the round-trip of opaque identifiers.

3. **Headless-consent nonce store.** A `consent_state` table (in the account
   database) holds one rotating one-time nonce per authorization request.
   `/oauth/authorize` mints the nonce and 302s the browser to the RemoteClient;
   every remote API call atomically validates and rotates it (expiry extends by
   a 300s inactivity window), and accept/reject delete the row. State mismatch
   or expiry returns 401. The remote API is guarded by a constant-time bearer
   token from `PDS_OAUTH_REMOTE_CLIENT_URL`/`PDS_OAUTH_REMOTE_CLIENT_TOKEN`.

4. **Framework-agnostic auth verifier.** Access/refresh/service JWT
   validation, DPoP-bound access-token validation, admin basic-auth, and
   account-status checks live as pure functions with no HTTP types; poem
   extractors are layered on top by the server handlers.

5. **Intentional divergences from common upstream bugs, pinned by tests.**
   Service-JWT expiry compares `now` in seconds (RFC 7519 units) rather than
   microseconds; issuers listed in the trust set are treated as trusted
   (unlisted issuers are rejected); the JWS signature is base64url-decoded and
   verified against the SHA-256 digest of the signing input; and the DID cache
   reports timestamps in microseconds to match the in-memory reference. Each
   behavior is pinned by a round-trip or error-branch test.

6. **Remote account creation is a seam.** `create-account` delegates to a
   `RemoteCreateAccount` trait; the server registers the real implementation at
   startup, and tests use a mock. The remote API surface is independent of how
   accounts are created.

7. **SSRF-hardened client metadata fetcher.** The fetcher accepts https URLs
   only, times out after 10s, disables redirects, requires an
   `application/json` content type, and caps response bodies at 512 KiB.

## Consequences

- The account database schema gains `consent_state` plus the companion TEXT
  columns on the authorization-request, device, and token tables; the migration
  test's expected table list must track these.
- Entity-based queries trade a little SQL transparency for compile-time typing;
  the `DbId`/`Did`/`OffsetDateTime` types catch column misuse at compile time,
  and the raw-SQL `RETURNING`/`ON CONFLICT` workarounds called out in ADR-0006
  are not needed here.
- The headless consent flow means no PDS-rendered HTML and no browser-form CSRF
  at this layer; the PDS validates a nonce + token on every call. When the
  RemoteClient is not configured, `/oauth/authorize` returns 503 by design.
- The auth verifier's pure-function split keeps the XRPC handlers thin and
  unit-testable without a running server, at the cost of a small registration
  step (`register_auth_dependencies`) the bootstrap must perform once.
- The behavioral fixes in decision 5 diverge from prior art on purpose and are
  covered by tests, so a future reader can distinguish intentional behavior
  from regression.
- A single `RemoteCreateAccount` impl is required before `create-account` works
  end to end; the mock only serves the API tests.
