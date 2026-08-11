# 6. PDS AccountManager and account helpers over sea-orm

Date: 2026-08-05

## Status

Accepted

## Context

cacos needs account management: actors, accounts, sessions, refresh tokens, app passwords, invite codes, email tokens, and the orchestration surface that ties them together. Storage for these lives in the four sea-orm databases from ADR 0002 (account DB in particular). This ADR captures the structure and conventions of the `pds/src/account` module that holds it.

Several constraints shape the design:

- The public method signatures, option-struct names, and metric-increment points are contract surfaces consumed by the OAuth/auth_verifier work and the xrpc server handlers. None of those names may drift without coordinated updates in those modules.
- `INSERT ... ON CONFLICT DO NOTHING RETURNING` semantics matter for several helpers (notably `register_actor` / `register_account`, which use the returned row to detect duplicate-did rejection). Sea-orm's builder drops `RETURNING` on SQLite, so every query in this module is raw `Statement::from_sql_and_values`.
- `PDS_JWT_KEYPAIR` and `AuthScope` need a single canonical home; the OAuth/auth_verifier module must import them rather than re-define them.
- `PDS_REPO_SIGNING_KEYPAIR` needs a single canonical home under `crate::context` so the xrpc server handlers can compile against it.

## Decision

1. **One `AccountManager` plus six helper modules.** The helper set is structured as `account`, `password`, `email_token`, `invite`, `auth`, and `repo`. `AccountManager` itself is a thin wrapper (`pub struct AccountManager { pub db: DatabaseConnection }`) that forwards to the helpers. `account/mod.rs` re-exports the public types so consumers reach them via `crate::account::*`.

2. **Every query is raw `Statement::from_sql_and_values`** through a shared `helpers::sql()` builder. Raw statements were chosen over the sea-orm entity API specifically so the `RETURNING` semantics survive and the SQL stays transparent. The helpers take `&sea_orm::DatabaseConnection` as their last parameter (or `&DatabaseTransaction` for transactions started inside a helper).

3. **`ON CONFLICT ... DO UPDATE` upserts** (email_token, repo_root) use the same raw form, with explicit `INSERT INTO ... VALUES (...) ON CONFLICT (...) DO UPDATE SET ...`. Email-token upserts key on `(purpose, did)` and refresh both the token and the `requestedAt`. Repo-root upserts key on `did`.

4. **`DatabaseKind::Account.open(...)` is the only opening API.** The `test_util::test_db()` calls `DatabaseKind::Account.open(dir.path().join("account.sqlite")).await.unwrap()` — no `open_account_db` alias was added.

5. **`PDS_JWT_KEYPAIR` lives at `pds/src/account/helpers/auth.rs`** as `pub static LazyLock<ES256kKeyPair>`, derived from `PDS_JWT_KEY_K256_PRIVATE_KEY_HEX`. Any future `auth_verifier.rs` must import this and delete any duplicate. The same module owns `AuthScope`; any duplicate copy elsewhere must go.

6. **`PDS_REPO_SIGNING_KEYPAIR` lives at `cacos-pds-account/src/auth/keypairs.rs`** as `pub static LazyLock<secp256k1::Keypair>`, derived from `PDS_REPO_SIGNING_KEY_K256_PRIVATE_KEY_HEX`. (Originally at `pds/src/context.rs`; moved to the account crate as part of the crate split.)

7. **`DisableInviteCodesOpts` is defined in `helpers/invite.rs`** (next to its consumer) and re-exported from `crate::account` via `pub use`. This puts the public path at `crate::account::DisableInviteCodesOpts` without leaking the helper-internal layout.

8. **Workspace deps.** `rsky-common` and `rsky-lexicon` are pinned at the workspace root via `rsky-* = { git = "https://github.com/olamaelcu/rsky", rev = "..." }`; consumers use `rsky-common = { workspace = true }` and `rsky-lexicon = { workspace = true }`. `lexicon_cid` is similarly resolved via the workspace alias.

9. **Sea-orm 2.0 API conventions.** All raw-statement call sites use `db.execute_raw(...)`, `db.query_one_raw(...)`, `db.query_all_raw(...)`, and `tx.execute_raw(...)` / `tx.query_one_raw(...)`. Sea-orm 2.0's `ConnectionTrait` only exposes the `*_raw` variants for raw `Statement` values; the bare `db.execute(Statement)` form used by sea-orm 1.x does not resolve. The convention matches `pds/src/actor_store/blob/mod.rs`, which has 14+ call sites following the same pattern. `db.begin()` requires the `sea_orm::TransactionTrait` import in scope.

10. **Metrics increment via `metrics::counter!(NAME).increment(1)`.** The standard idiom is used at the three increment points: `create_account` increments `SIGNUPS_TOTAL` (and `INVITE_USAGE_TOTAL` when `invite_code.is_some()`), `create_session` increments `SESSIONS_TOTAL`. All three are no-ops when no global recorder is installed, so the test suite runs without `init_metrics`.

11. **`is_unique_violation` matches sea-orm 2.0's `Arc<SqlxError>` envelope.** The inner value of `RuntimeErr::SqlxError` is `Arc<sqlx::Error>`, not `sqlx::Error`. The helper dereferences via `&**sqlx_err_arc` and matches the constraint primary-code mask `19` (every SQLite constraint-violation extended code shares that mask). The `is_unique_violation_matches_constraint_errors` test in `account/helpers/account.rs` exercises both the negative case (anyhow errors that aren't constraint violations) and the positive case (a real `UNIQUE` violation on `actor.handle`).

12. **Plain row structs deliberately do not derive from the sea-orm entities.** `auth::RefreshTokenRow`, `email_token::EmailToken`, `invite::{InviteCode, InviteCodeUse}` are local plain structs. They are distinct from the sea-orm entities so the entity definitions can change without touching the helpers, and vice versa.

13. **Typed errors via `AuthHelperError`.** `decode_refresh_token` returns `anyhow::Result` but the error sites surface typed `AuthHelperError` variants (thiserror + `miette::Diagnostic`) wrapped via `anyhow::Error::new(...)`, matching the established `ConcurrentRefresh` pattern. The variants are `NotARefreshToken` (wrong scope) and `MissingRefreshClaim(&'static str)` (one of `subject` / `expiration` / `jti`). No `assert_eq!` panics, no `.unwrap()` on JWT claims — every failure path returns an `Err`.

14. **Service JWT `exp` is in seconds.** `create_service_jwt` computes `exp = now_seconds + MINUTE/1000` (matching atproto's `iat + MINUTE/1e3`). rsky-common's `MINUTE = 60_000` ms, so `MINUTE/1000 = 60` seconds. The earlier `(now + MINUTE)/1000` produced ~1.7e12 (milliseconds) and made service tokens effectively never expire under seconds-based verifiers; that bug is fixed. Test: `service_jwt_exp_is_seconds_since_epoch`.

15. **Refresh-token grace is 2 hours, hoisted constant.** `REFRESH_GRACE_MS: i64 = 2 * 60 * 60 * 1000` lives at module scope in `pds/src/account/mod.rs` with the millisecond unit spelled out (avoids the previous unit ambiguity around rsky-common's `HOUR`, which is `3_600_000` ms — not 3600 s). `grace_expires_at = now_micros + REFRESH_GRACE_MS * 1000` lands at `now + 7_200 s = now + 2 h`. The constant is regression-pinned by `refresh_grace_period_is_two_hours` so the misdiagnosis that previously claimed a ~7.2 s grace can't recur.

16. **`register_actor` does not stamp `deleteAfter` at registration.** A deactivated account created at signup gets `deactivatedAt = createdAt` and `deleteAfter = NULL`. An explicit `deleteAfter` is only stamped through the admin deactivate path (`AccountManager::deactivate_account(did, delete_after)`). The earlier automatic `createdAt + 3 days` stamp was dropped because no cleanup job honors it in cacos today. Tests `register_actor_with_deactivated_flag` and `creates_account_without_credentials_and_deactivated` assert `delete_after.is_none()`.

17. **App-password salt is deterministic per did, matching atproto's reference design.** `hash_app_password(did, password)` derives the salt from `Base64(SHA-256(did))` and hashes with argon2. The salt is also embedded in the stored argon2 PHC string, so an attacker with DB read access already has it; the deterministic aspect only means identical (did, password) pairs produce identical hashes. The design tradeoff matches atproto and is kept here. Switching to a random salt would change the stored format and is not done.

18. **Sea-orm 2.0 conventions applied throughout.** Every call site that would have been `db.execute(Statement)` under sea-orm 1.x was rewritten to `db.execute_raw(Statement)` (and the analogous `_raw` variants for `query_one` and `query_all`). The standard `metrics::counter!(NAME).increment(1)` form replaced the missing `metrics::increment_counter!` macro.

## Consequences

- The module ships 62 tests across the helper suites and the integration suite, all green on `cargo test -p cacos-pds account::`. The full workspace test count is 155 (`mise run test` reports 155/155 passing). Test suite breakdown: 11 account-helper, 6 password, 7 email-token, 8 invite, 12 auth-helper (was 9; added `decode_refresh_token_rejects_non_refresh_token`, `decode_refresh_token_rejects_garbage`, `service_jwt_exp_is_seconds_since_epoch`), 18 integration (was 17; added `refresh_grace_period_is_two_hours`).
- The OAuth/auth_verifier module must align with this work: it must delete any `PDS_JWT_KEYPAIR` and `AuthScope` copies and import both from `crate::account::helpers::auth`. Both definitions derive deterministically from the same env var, so they interop until converged.
- The xrpc server handlers import `crate::context::PDS_REPO_SIGNING_KEYPAIR` for repo signing, and consume `AccountManager::create_session`, `create_app_password`, `create_email_token`, `assert_valid_email_token`, and the invite/password methods. None of those names may be renamed here without coordinating with the server handlers.
- `decode_refresh_token` no longer panics on a non-refresh token or a missing JWT claim — it returns a typed `AuthHelperError::NotARefreshToken` or `AuthHelperError::MissingRefreshClaim(name)`. Consumers matching on the error chain can distinguish them via `err.downcast_ref::<AuthHelperError>()`.
- Service JWT `exp` is in seconds since the Unix epoch. A seconds-based verifier sees a sane future timestamp (now + 60 s by default). When a `verify_service_jwt` lands, it must interpret `exp` as seconds, matching the atproto reference.
- The refresh-token grace is exactly 2 hours; the `refresh_grace_period_is_two_hours` unit test pins the constant and the math so the unit-ambiguity that previously produced a ~7.2 s misdiagnosis cannot recur.
- Deactivated accounts created at signup are no longer scheduled for automatic deletion; the `deleteAfter` column stays null until an admin path stamps it via `AccountManager::deactivate_account`. Any future cleanup job that deletes accounts past their `deleteAfter` only operates on rows where `deleteAfter IS NOT NULL`.
- Three integration-suite tests (`manages_sessions_and_refresh_tokens`, `manages_invites`, `manages_passwords`) take several seconds each because they exercise the JWT signing path and the argon2 hash path. A future test-suite split that moves them to `#[ignore]` (or `cargo nextest`'s slow-test profile) would speed up the inner loop.
- The test fixture at `pds/src/account/test_util.rs` uses `unsafe { std::env::set_var(...) }` (Rust 1.97 marks `set_var` as `unsafe`). The `Once` guard prevents concurrent env reads, and tests run sequentially within the test binary, so this is safe — but the comment explaining the SAFETY invariant must not be deleted without a replacement.
- `db.begin()` returning a `DatabaseTransaction` requires `use sea_orm::TransactionTrait;` in scope at every call site. Helper files that begin transactions (`helpers/invite.rs`, `AccountManager::rotate_refresh_token`) carry this import.
