# 9. PDS API surface (XRPC) over poem 3.x

Date: 2026-08-09

## Status

Accepted

## Context

The PDS had the typed `AccountManager`, `ActorStore`, `Sequencer`, and PLC plumbing in place but no real HTTP layer in front of them; the only route the server served was `GET /metrics`. The rsky reference ships a Rocket port, but the framework choice for cacos is poem 3.x (lighter, async-first, no codegen). The on-disk schema and typed-entity discipline (ADR-0006), the per-DID actor-store sharding (ADR-0004), and the typed sequencer (ADR-0008) define the collaborators the HTTP layer threads through; this ADR is about the surface that connects them to the XRPC wire protocol.

## Decision

1. **poem 3.x as the HTTP framework.** `pds/src/xrpc/mod.rs` exports `build_app_with_state(state: SharedState) -> impl Future<Output = poem::Route>` and the synchronous `build_app(...)` for tests. CORS via `Cors::allow_origins_fn` (poem 3 API). State carried by `poem::web::Data<&SharedState>` everywhere; per-module `routes(route: poem::Route) -> poem::Route` chains `.at(...)` onto the passed route (avoids radix-tree collisions on the `*--poem-rest` glob).

2. **`ApiError` / `ApiResult` boundary.** `pds/src/xrpc/error.rs` maps poem errors to AT Protocol's error shapes (`InvalidLogin`, `InvalidRequest`, `ExpiredToken`, `AccountTakendown`, `AccountNotFound`, `AuthRequired`, `InternalServerError`, `RuntimeError`, plus the `XRPCError { status, error, message }` envelope). Every handler returns `ApiResult<T>`.

3. **Metrics + health + well-known surface.** `pds/src/xrpc/{metrics.rs, health.rs, well_known.rs}` add `GET /metrics`, `GET /_health` (liveness, no auth), `GET /xrpc/_health` (readiness, no auth), and `GET /.well-known/atproto-did` (resolves Host → DID via `IdentityConfig.service_handle_domains`).

4. **14 auth extractors over the canonical `auth_verifier`.** `pds/src/xrpc/auth_extractors.rs` re-uses `PDS_JWT_KEYPAIR` and `AuthScope` from `pds/src/account/helpers/auth.rs` (no redefinition). Extractors: `AccessStandard`, `AccessFull`, `AccessPrivileged`, `AccessStandardIncludeChecks`, `AccessStandardCheckTakedown`, `AccessFullImport`, `AccessStandardSignupQueued`, `Refresh`, `RevokeRefreshToken`, `AdminToken`, `Moderator`, `OptionalAccessOrAdminToken`, `UserDidAuthOptional`, `UserDidAuth`.

5. **Handle validation in `pds::handle`.** `pds/src/handle/{mod.rs, errors.rs}` defines `normalize_and_validate_handle` plus a 40-entry `RESERVED_SUBDOMAINS` list. `IdentityConfig.service_handle_domains` is populated from `PDS_SERVICE_HANDLE_DOMAINS` (comma-separated).

6. **PLC client trait + reqwest impl + mock.** `pds/src/plc/{mod.rs, types.rs, operations.rs}`. `PlcClient` carries `ensure_http_prefix` and `encode_uri_component`; `PlcClientImpl` uses `reqwest`; `MockPlcClient` returns hardcoded `DocumentData` for tests. `ensure_http_prefix` preserves the scheme (`http://`/`https://`); un-prefixed input gets `https://` prepended.

7. **Mailer is a no-op log-and-return.** `pds/src/mailer.rs` exposes the same `send_*` family the Rocket port calls (`send_reset_password`, `send_confirm_email`, `send_update_email`, `send_account_delete`, `send_plc_operation`, plus `IdentifierAndTokenParams` and `TokenParam`). Each logs to `tracing` and returns `Ok(())`; handler signatures stay byte-compatible.

8. **`com.atproto.server` module with 25-NSID route table.** `pds/src/xrpc/com/atproto/server/mod.rs` declares `PDS_PLC_ROTATION_KEYPAIR` (lazy env), `AssertionContents`, helpers (`validate_handle`, `gen_invite_code`, `gen_invite_codes`, `get_random_token`, `safe_resolve_did_doc`, `is_valid_did_doc_for_service`, `assert_valid_did_documents_for_service`, `assert_valid_doc_contents`), plus the `PROTECTED_METHODS` / `PRIVILEGED_METHODS` const lists. The 25 NSIDs cover: sessions (createSession/refreshSession/deleteSession/getSession), account lifecycle (createAccount/activateAccount/deactivateAccount/deleteAccount/checkAccountStatus), email (confirmEmail/updateEmail/requestEmailConfirmation/requestEmailUpdate/requestPasswordReset/resetPassword/requestAccountDelete), app passwords (createAppPassword/listAppPasswords/revokeAppPassword), invites (createInviteCode/createInviteCodes/getAccountInviteCodes), describeServer, getServiceAuth, reserveSigningKey.

9. **`createAccount` ports the full PLC + repo flow.** `pds/src/xrpc/com/atproto/server/create_account.rs` calls `actor_store.create` → `plc_client.send_operation` → `account_manager.create_account` → sequences identity/account/commit/sync events. The `SharedSequencer` lock is `std::sync::RwLock`; the inner `Sequencer` is cloned out of the lock before any `.await`.

10. **`com.atproto.repo` route module + `prepare` helpers.** `pds/src/xrpc/com/atproto/repo/{mod.rs, prepare.rs}` declare the route table for repo.* and the typed `prepare_create` / `prepare_update` / `prepare_delete` / `find_blob_refs` / `blobs_for_write` helpers. Backlink paths use a structural `<$type>/<slot>` heuristic.

## Consequences

- 281 tests pass on `feature/plan-08-xrpc`: 243 lib + 38 integration across 9 binaries (`xrpc_smoke`, `well_known_test`, `app_smoke`, `metrics_endpoint`, `server_sessions_test`, `app_passwords_test`, `email_handlers_test`, `invites_test`, `account_lifecycle_test`). 0 failures. `cargo clippy -p cacos-pds --all-targets -- -D warnings` clean.
- `SharedState` at `pds/src/xrpc/types.rs:24` is the single source of truth; every handler takes `poem::web::Data<&SharedState>` and accesses `state.account_manager`, `state.actor_store`, `state.plc_client`, `state.sequencer`, `state.id_resolver`, `state.config`, `state.blobstore`.
- New direct deps in `pds/Cargo.toml`: `tempfile`, `poem`, `indexmap = "2"`, `data-encoding = "2"`, `email_address = "0.2"`.
- `PlcClient` trait methods take `&str` (not `&String`); two call sites pass `&did.to_string()` because `rsky_common::encode_uri_component(input: &String)` is fixed in the external API.
- `routes()` signature is `(Route) -> Route`, not `() -> Route`: matches `build_app_with_state`'s chaining pattern.
- `email_address = "0.2"` is in `pds/Cargo.toml` for `createAccount`'s input validation.
- `SharedSequencer` is `std::sync::RwLock`; the lock guard is never held across an `.await` (poem requires `Send` futures; `RwLockWriteGuard` is `!Send`).
- `account_manager` and `actor_store` are concrete types in `SharedState`; `plc_client` is `Arc<dyn PlcClient>`, `sequencer` is `SharedSequencer` (`Arc<RwLock<Sequencer>>`), `id_resolver` is `Arc<tokio::sync::RwLock<IdResolver>>`, `blobstore` is `Arc<dyn BlobStore>`.
- `routes()` tables register NSID leaf paths (e.g. `/xrpc/createSession`), not dotted NSIDs; the `*--poem-rest` glob matches on the leaf segment.
