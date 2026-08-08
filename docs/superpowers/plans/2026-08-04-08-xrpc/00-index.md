# Plan 08: Poem App + PDS-Ownable XRPC Handlers + well_known + Health Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Assemble the cacos PDS HTTP layer: a poem `build_app()` route tree mounting the PDS-ownable `com.atproto.*` XRPC handlers (server/identity/sync/repo/admin/temp), app.bsky get/put preferences, `.well-known/atproto-did`, and health endpoints, with the `ApiError` mapping, auth extractors, and per-NSID request metrics.

**Architecture:** Port the rsky-pds Rocket handlers (`the git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's `src/apis/`) to poem endpoints one handler at a time. Rocket guards become poem extractors (`State<SharedState>`, `Json`, `Query`, `Path`, `Bytes`, custom `FromRequest` auth guards); the `ApiError` enum becomes a poem `IntoResponse` producing the same `{error, message}` JSON with the same statuses. Shared state (AccountManager, ActorStore, Sequencer, IdResolver, BlobStore, PlcClient, ServerConfig, Mailer) lives in one `SharedState` struct registered with `.data(state)`. Appview-coupled logic (pipethrough, moderation.createReport, repo.getRecord's remote fallback, all app.bsky feed/notification/actor `get_*` except preferences) stays deferred.

**Tech Stack:** poem 3.x (with `tower-compat`), `poem::test::TestClient`, `rsky_lexicon` (path dep), `rsky_repo`, `rsky_identity`, `rsky_crypto`, `rsky_syntax`, `rsky_common`, `jwt-simple`, `secp256k1`, `reqwest` (plc client), `metrics` 0.24 facade, `tokio`, `tempfile`.

---

## Dependencies on Plans 01–07 (assumed API surface)

This plan integrates everything from Plans 01–07. `main.rs` calls `build_app()`. The exact names below must already exist (per the shared glossary); **adjust the import paths in this plan to the exact module layout Plans 01–07 chose if they differ** — the *names* are fixed.

| Component | Assumed path / type | Methods this plan calls |
|---|---|---|
| Config | `crate::config::{ServerConfig, env_to_cfg}` (Plan 01) | `service.public_url`, `service.hostname`, `service.did`, `identity.service_handle_domains`, `identity.plc_url`, `invites.required`, `invites.interval`, `invites.epoch`, `subscription.*` |
| Keypair statics | `crate::context::{PDS_REPO_SIGNING_KEYPAIR}`; `crate::xrpc::com::atproto::server::PDS_PLC_ROTATION_KEYPAIR` (defined in this plan, Task 5) | `.public_key()`, `.secret_key()` |
| DB pool | `crate::db` (Plan 01) | `get_migrated_db(location)` |
| AccountManager | `crate::account::AccountManager` + `crate::account::EmailTokenPurpose` (Plan 05) | `get_account`, `get_account_by_email`, `get_did_for_actor`, `is_account_activated`, `create_account(CreateAccountOpts)`, `get_account_admin_status`, `get_account_status`, `update_repo_root`, `delete_account`, `takedown_account`, `update_handle`, `deactivate_account`, `activate_account`, `create_session`, `rotate_refresh_token`, `revoke_refresh_token`, `create_invite_codes`, `create_account_invite_codes`, `get_account_invite_codes`, `get_invited_by_for_accounts`, `set_account_invites_disabled`, `disable_invite_codes(DisableInviteCodesOpts)`, `create_app_password`, `list_app_passwords`, `verify_account_password`, `verify_app_password`, `reset_password(ResetPasswordOpts)`, `update_account_password(UpdateAccountPasswordOpts)`, `revoke_app_password`, `confirm_email(ConfirmEmailOpts)`, `update_email(UpdateEmailOpts)`, `assert_valid_email_token`, `assert_valid_email_token_and_cleanup`, `create_email_token` |
| Account helpers | `crate::account::helpers::account::{ActorAccount, AvailabilityFlags, AccountStatus, format_account_status, FormattedAccountStatus}` (Plan 05); `crate::account::helpers::auth::{create_service_jwt, ServiceJwtParams, decode_refresh_token}` (Plan 05); `crate::account::helpers::invite::{CodeDetail, get_invite_codes_uses_v2}` (Plan 05) | as the reference handlers do |
| ActorStore | `crate::actor_store::ActorStore` (Plan 03), holding `storage: RepoStorage`, `record: RecordReader`, `blob: BlobReader`, and (Task 27) `pref: PreferenceReader` | `create(&did, &keypair)`, `destroy(&did, blobstore)`, `transact(did, blobstore)`, `read(did, blobstore)`, `reserve_keypair(Option<&str>)`, `get_sync_event_data()`, `get_repo_root()`; transaction has `create_repo`, `process_writes`, `process_import_repo`; `record.*`: `get_backlink_conflicts`, `get_record`, `list_records_for_collection`, `list_collections`, `record_count`, `get_record_takedown_status`, `get_current_record_cid`, `update_record_takedown_status`; `blob.*`: `upload_blob_and_get_metadata`, `track_untethered_blob`, `get_records_for_blob`, `verify_blob_and_make_permanent`, `list_missing_blobs(ListMissingBlobsOpts)`, `get_blob`, `get_blob_takedown_status`, `update_blob_takedown_status`, `list_blobs(ListBlobsOpts)`, `blob_count`, `record_blob_count` |
| BlobStore | `crate::blobstore::{BlobStore, OpenDALBlobStore}` (Plan 04) | shared `Arc<dyn BlobStore>`; cacos keys blobs by (did, cid) internally so one instance is shared (adaptation of the per-did `BlobstoreFactory`) |
| Auth verifier | `crate::auth::auth_verifier` (Plan 06): `AuthScope`, `AccessOutput`, `Credentials`, `AuthError`, `ValidateAccessTokenOpts`, `validate_access_token(auth_header: Option<&str>, scopes, opts)`, `validate_refresh_token(auth_header)`, `parse_basic_auth`, `admin_password_from_env`, `is_user_or_admin(&AccessOutput, &str)`, `PDS_JWT_KEYPAIR` | Task 2 wraps these in poem extractors |
| IdResolver | `crate::identity::IdResolver` = `rsky_identity::IdResolver` (Plan 06) | `lock.did.resolve(did, force_refresh)`, `lock.did.ensure_resolve(did, force_refresh)`, `lock.handle.resolve(handle)` |
| Sequencer | `crate::context::{SharedSequencer, SharedBroadcast}` where `SharedSequencer { pub sequencer: RwLock<Sequencer> }` + `crate::sequencer::Sequencer` (Plan 07) | `sequence_commit`, `sequence_identity_evt`, `sequence_account_evt`, `sequence_sync_evt`, `sequence_handle_update`, `delete_all_for_user`, `next_seq`, `curr`; events module re-exports `sync_evt_data_from_commit` |
| subscribeRepos | `crate::xrpc::com::atproto::sync::subscribe_repos::subscribe_repos` poem handler (Plan 07, same module tree) | takes `&State<SharedSequencer>` + `&State<SharedBroadcast>` — `build_app` must `.data(state.sequencer.clone())` and `.data(state.shared_broadcast.clone())`; mounted at `/xrpc/com.atproto.sync.subscribeRepos` (Task 23) |
| Metrics | `crate::observability::metrics` recorder + `/metrics` route (Plan 02) | Task 1 adds `cacos_http_*` request metrics via `metrics::increment_counter!` / `metrics::histogram!` |
| rsky crates | `rsky_lexicon`, `rsky_repo`, `rsky_identity`, `rsky_crypto`, `rsky_syntax`, `rsky_common` (git-pinned path deps) | as the reference handlers do |

**Poem API facts (verified against poem 3.1.12):** `Response::builder().status(StatusCode).content_type("...").body(impl Into<Body>)`; `FromRequest<'a>` is `async fn from_request(req: &'a Request, body: &mut RequestBody) -> Result<Self>` (poem `Result` = `Result<T, poem::Error>`); `poem::error::Error::from_response(resp)`; `TestClient::new(app).get(url).header(k, v).send().await` returns a `TestResponse` that wraps `pub Response` — read status/content-type via `resp.0.status()` / `resp.0.content_type()`, the body via `resp.0.into_body().into_string()/into_bytes()/into_json::<T>()` (all async), or use the assertion helpers (`resp.assert_status(...)`, `resp.assert_text(...)`, `resp.assert_bytes(...)`, `resp.json().await`); `Route::new().at(path, get(handler))`, `.data(state)` via `EndpointExt`; `State<T>` extractor; `.with(middleware)` for `Middleware`. `TestResponse` has no `body_string()`/`bytes()`/`status()` getters — the plan's tests use the `resp.0` idioms consistently.

**Locked-in decisions:**
- **PlcClient:** port the reqwest-based client (`the git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's `src/plc/`) as `PlcClientImpl` behind a `PlcClient` trait so identity handlers are testable with a mock. Env: `PDS_DID_PLC_URL` (already in `ServerConfig.identity.plc_url`). No identity handlers are deferred.
- **Preferences:** Plan 03 did not port the pref store, so this plan includes a minimal `actor_store/preference/mod.rs` port (`PreferenceReader` over the `account_pref` table) plus `app.bsky.actor.getPreferences` / `putPreferences` (Task 27).
- **repo.getRecord:** implement the **local-hosted branch only** (repo lives on this PDS → serve from the actor_store record reader). The appview pipethrough fallback stays deferred. (Fully local round-trips still work.)
- **Mailer:** out of scope to send real mail (mailgun). This plan ports the mailer as a **logging no-op** (`crate::mailer` free functions with the same signatures) so email handlers keep identical logic.
- **Blob constraints:** `repo/prepare.rs`'s app.bsky-specific `CONSTRAINTS` map depends on the generated `lexicons.toml`; port `find_blob_refs`/`blobs_for_write` with `BlobConstraint { max_size: None, accept: None }` (blob detection still works; size/mime constraints are app.bsky UI detail).
- **Handle validation:** port the small `crate::handle` module (mod.rs + errors.rs) from `the git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's `src/handle/`; slur/reserved lists come from `rsky_common::explicit_slurs::contains_explicit_slurs` and a `RESERVED_SUBDOMAINS` const (Task 4).
- **Deferred (appview-coupled):** `com.atproto.moderation.createReport`, `bsky_api_get/post_forwarder`, `repo.getRecord` remote fallback, `app.bsky.feed.*`, `app.bsky.notification.*`, `app.bsky.actor.getProfile(s)`.
- **Headless-consent `RemoteCreateAccount` impl (Task 7b):** Plan 06's `RemoteCreateAccount` trait seam (defined in [Plan 06 Task 5](../2026-08-04-06-oauth-auth-verifier/05-oauth-provider-routes-poem-stub-authorize-page-templates.md), spec: [Headless OAuth Consent Design](../../specs/2026-08-04-headless-oauth-consent-design.md)) is satisfied here by `RealRemoteCreateAccount`. Depends on `ActorStore` (Plan 03), `AccountManager` (Plan 05), `Sequencer` (Plan 07), and `PlcClient` (Task 18). Production wiring uses the real impl; tests use `MockRemoteCreateAccount`. `build_app` swaps based on `cfg!(test)`.
- **Metrics (Plan 02 prefix `cacos_`):** `cacos_http_requests_total{nsid}`, `cacos_http_request_errors_total{nsid}`, `cacos_http_request_duration_seconds{nsid}` (histogram) recorded by the `RequestMetrics` middleware (Task 1). Sub-stage timing (`auth`, `repo_load`, `write`, `sequence`) is emitted as tracing spans (`tracing::span!`) on write handlers — the timing-histogram fold is the Plan 09 sweep.

---

## Task Index

| Task # | Title | File |
|---|---|---|
| 1 | XRPC Foundation — `ApiError`, `build_app()`, metrics middleware, test utils | [01-xrpc-foundation.md](./01-xrpc-foundation.md) |
| 2 | Auth Extractors — `xrpc/auth_extractors.rs` | [02-auth-extractors.md](./02-auth-extractors.md) |
| 3 | well_known + health wiring + mailer | [03-well-known-health-mailer.md](./03-well-known-health-mailer.md) |
| 4 | Handle validation module port (`crate::handle`) | [04-handle-validation.md](./04-handle-validation.md) |
| 5 | Server module helpers — `server/mod.rs`, describeServer, getServiceAuth, reserveSigningKey | [05-server-module-helpers.md](./05-server-module-helpers.md) |
| 6 | Sessions — createSession, refreshSession, deleteSession, getSession | [06-sessions.md](./06-sessions.md) |
| 7 | Account lifecycle — createAccount, activateAccount, deactivateAccount, deleteAccount, checkAccountStatus | [07-account-lifecycle.md](./07-account-lifecycle.md) |
| 7b | **`RemoteCreateAccount` impl — `/oauth/remote/create-account` handler (headless-consent `prompt=create`)** | [07b-oauth-remote-create-account.md](./07b-oauth-remote-create-account.md) |
| 8 | Email handlers — updateEmail, confirmEmail, requestEmailConfirmation, requestEmailUpdate, requestPasswordReset, resetPassword, requestAccountDelete | [08-email-handlers.md](./08-email-handlers.md) |
| 9 | App passwords — createAppPassword, listAppPasswords, revokeAppPassword | [09-app-passwords.md](./09-app-passwords.md) |
| 10 | Invites — createInviteCode, createInviteCodes, getAccountInviteCodes | [10-invites.md](./10-invites.md) |
| 11 | Repo prepare + mod — `repo/prepare.rs` and `repo/mod.rs` | [11-repo-prepare-mod.md](./11-repo-prepare-mod.md) |
| 12 | repo.createRecord | [12-repo-create-record.md](./12-repo-create-record.md) |
| 13 | repo.putRecord | [13-repo-put-record.md](./13-repo-put-record.md) |
| 14 | repo.deleteRecord + listRecords + describeRepo | [14-repo-delete-list-describe.md](./14-repo-delete-list-describe.md) |
| 15 | repo.applyWrites | [15-repo-apply-writes.md](./15-repo-apply-writes.md) |
| 16 | repo.uploadBlob + listMissingBlobs | [16-repo-upload-blob-list-missing.md](./16-repo-upload-blob-list-missing.md) |
| 17 | repo.getRecord (local branch) + importRepo | [17-repo-get-record-import.md](./17-repo-get-record-import.md) |
| 18 | PLC client port — `plc/types.rs`, `plc/operations.rs`, `plc/mod.rs` | [18-plc-client-port.md](./18-plc-client-port.md) |
| 19 | Identity resolve — resolveDid, resolveHandle, resolveIdentity, refreshIdentity | [19-identity-resolve.md](./19-identity-resolve.md) |
| 20 | getRecommendedDidCredentials | [20-get-recommended-did-credentials.md](./20-get-recommended-did-credentials.md) |
| 21 | updateHandle | [21-update-handle.md](./21-update-handle.md) |
| 22 | submitPlcOperation, signPlcOperation, requestPlcOperationSignature | [22-plc-operations.md](./22-plc-operations.md) |
| 23 | Sync read endpoints — getRepo, getCheckout, getBlocks, getRecord(sync), getHead, getLatestCommit, getBlob | [23-sync-read-endpoints.md](./23-sync-read-endpoints.md) |
| 24 | Sync list endpoints — listBlobs, listRepos, getRepoStatus | [24-sync-list-endpoints.md](./24-sync-list-endpoints.md) |
| 25 | Admin group 1 — deleteAccount, disableAccountInvites, enableAccountInvites, disableInviteCodes, updateAccountEmail, updateAccountHandle, updateAccountPassword | [25-admin-group-1.md](./25-admin-group-1.md) |
| 26 | Admin group 2 + temp — getAccountInfo(s), getInviteCodes, getSubjectStatus, updateSubjectStatus, sendEmail, checkSignupQueue | [26-admin-group-2-temp.md](./26-admin-group-2-temp.md) |
| 27 | Preferences — `actor_store/preference/` port + app.bsky.actor.getPreferences/putPreferences | [27-preferences.md](./27-preferences.md) |
| 28 | Integration verification — full route tree, round-trip, metrics | [28-integration-verification.md](./28-integration-verification.md) |

---

## Self-Review Notes (run before execution)

**Spec coverage:** every PDS-ownable handler from `the git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's `src/apis/` is assigned a task: server (5–10), repo (12–17), identity (19–22, with plc in 18), sync (23–24), admin (25–26), temp (26), app.bsky prefs (27), well_known/health/mailer (3), handle (4), error/app/metrics (1), auth extractors (2), integration (28).

**Deferred (documented):** moderation.createReport, pipethrough forwarders, repo.getRecord remote fallback, app.bsky feed/notification/actor get_* (except prefs), mailgun mailer, app.bsky blob constraints (CONSTRAINTS), external-handle resolution branch.

**Type consistency:** `SharedState` is defined once (Task 1) and every handler imports it the same way (`poem::State<SharedState>`); `PlcClient`/`PlcClientImpl`/`MockPlcClient` are defined in Task 18 and used in Tasks 7, 20–22, 25; `ApiError` variants used across tasks match the Task 1 enum exactly; extractor names (`AccessStandard`, `AccessFull`, `AccessStandardIncludeChecks`, `AccessStandardCheckTakedown`, `AccessFullImport`, `AccessStandardSignupQueued`, `Refresh`, `RevokeRefreshToken`, `AdminToken`, `Moderator`, `OptionalAccessOrAdminToken`, `UserDidAuth`, `UserDidAuthOptional`) are defined in Task 2 and referenced identically later. Metric names are constants in `xrpc/metrics.rs` (Task 1) and asserted in Task 28.

**Cross-plan consistency (for Plans 01–07):** the exact extractor names above, the `ApiError` variant list, deferred handler list (`getRecord` remote branch, `moderation.createReport`, all app.bsky `get_*` except prefs), the `PlcClient` trait decision (no identity handlers deferred), the pref-store port decision (included here), and the metric names `cacos_http_requests_total` / `cacos_http_request_errors_total` / `cacos_http_request_duration_seconds` (label `nsid`) are the canonical values this plan pins.

**Execution handoff:** plan saved to `docs/superpowers/plans/2026-08-04-08-xrpc.md`. Two execution options: (1) Subagent-Driven (recommended) — dispatch a fresh subagent per task with review between tasks; (2) Inline — execute task-by-task with checkpoints via executing-plans. Both use superpowers:subagent-driven-development / superpowers:executing-plans respectively.
