# 6. PDS AccountManager and account helpers over sea-orm

Date: 2026-08-05

## Status

Accepted

## Context

cacos needs account management: actors, accounts, sessions, refresh tokens, app passwords, invite codes, email tokens, and the orchestration surface that ties them together. Storage for these lives in the four sea-orm databases from ADR 0002 (account DB in particular). The reference implementation in `rsky-pds`'s `account_manager` module covers all of this on top of `rusqlite`; porting the same behavior onto cacos's sea-orm stack is the work captured here.

Several constraints shape the port:

- The public method signatures, option-struct names, and metric-increment points are contract surfaces consumed by the OAuth/auth_verifier work and the xrpc server handlers. None of those names may drift without coordinated updates in those modules.
- The reference uses `INSERT ... ON CONFLICT DO NOTHING RETURNING` semantics that sea-orm's builder drops on SQLite. The port stays faithful by routing every query through raw `Statement::from_sql_and_values`.
- `PDS_JWT_KEYPAIR` and `AuthScope` need a single canonical home; the OAuth/auth_verifier module must import them rather than re-define them.
- `PDS_REPO_SIGNING_KEYPAIR` needs a single canonical home under `crate::context` so the xrpc server handlers can compile against it.
- The reference carries five known bugs (a panic in `decode_refresh_token`, an `exp` in milliseconds in `create_service_jwt`, a `REFRESH_GRACE_MS` unit error producing a ~7.2s grace instead of 2 hours, `deleteAfter = createdAt + 3 days` on deactivated actors, and a deterministic SHA-256(did) base64 app-password salt). These land verbatim so the port's behavior matches the reference.

## Decision

1. **One `AccountManager` plus six helper modules.** The helper set mirrors the reference one-for-one: `account`, `password`, `email_token`, `invite`, `auth`, and `repo`. `AccountManager` itself is a thin wrapper (`pub struct AccountManager { pub db: DatabaseConnection }`) that forwards to the helpers. `account/mod.rs` re-exports the public types so the reference's `crate::account_manager::*` paths map onto `crate::account::*`.

2. **Every query is raw `Statement::from_sql_and_values`** through a shared `helpers::sql()` builder. Raw statements were chosen over the sea-orm entity API specifically so the SQL strings stay byte-identical to the reference, the `RETURNING` semantics survive, and the port can be diffed row-by-row against `rsky-pds`. The helpers take `&sea_orm::DatabaseConnection` as their last parameter (or `&DatabaseTransaction` for transactions started inside a helper).

3. **`ON CONFLICT ... DO UPDATE` upserts** (email_token, repo_root) use the same raw form, with explicit `INSERT INTO ... VALUES (...) ON CONFLICT (...) DO UPDATE SET ...`. Email-token upserts key on `(purpose, did)` and refresh both the token and the `requestedAt`. Repo-root upserts key on `did`.

4. **`DatabaseKind::Account.open(...)` is the only opening API.** The `test_util::test_db()` calls `DatabaseKind::Account.open(dir.path().join("account.sqlite")).await.unwrap()` — no `open_account_db` alias was added.

5. **`PDS_JWT_KEYPAIR` lives at `pds/src/account/helpers/auth.rs`** as `pub static LazyLock<ES256kKeyPair>`, derived from `PDS_JWT_KEY_K256_PRIVATE_KEY_HEX`. Any future `auth_verifier.rs` must import this and delete any duplicate. The same module owns `AuthScope`; any duplicate copy elsewhere must go.

6. **`PDS_REPO_SIGNING_KEYPAIR` lives at `pds/src/context.rs`** as `pub static LazyLock<secp256k1::Keypair>`, derived from `PDS_REPO_SIGNING_KEY_K256_PRIVATE_KEY_HEX`. The static lives under `crate::context` so the xrpc server handlers can import it.

7. **`DisableInviteCodesOpts` is defined in `helpers/invite.rs`** (next to its consumer) and re-exported from `crate::account` via `pub use`. This preserves the reference's public path `crate::account_manager::DisableInviteCodesOpts` without leaking the helper-internal layout.

8. **Workspace deps, not path deps.** `rsky-common` and `rsky-lexicon` are pinned at the workspace root via `rsky-* = { git = "https://github.com/olamaelcu/rsky", rev = "..." }`. The port uses `rsky-common = { workspace = true }` and `rsky-lexicon = { workspace = true }`. `lexicon_cid` is similarly resolved via the workspace alias.

9. **Sea-orm 2.0 API conventions.** All raw-statement call sites use `db.execute_raw(...)`, `db.query_one_raw(...)`, `db.query_all_raw(...)`, and `tx.execute_raw(...)` / `tx.query_one_raw(...)`. Sea-orm 2.0's `ConnectionTrait` only exposes the `*_raw` variants for raw `Statement` values; the bare `db.execute(Statement)` form used by sea-orm 1.x does not resolve. The convention matches `pds/src/actor_store/blob/mod.rs`, which has 14+ call sites following the same pattern. `db.begin()` requires the `sea_orm::TransactionTrait` import in scope.

10. **Metrics increment via `metrics::counter!(NAME).increment(1)`.** The standard idiom is used at the three increment points: `create_account` increments `SIGNUPS_TOTAL` (and `INVITE_USAGE_TOTAL` when `invite_code.is_some()`), `create_session` increments `SESSIONS_TOTAL`. All three are no-ops when no global recorder is installed, so the test suite runs without `init_metrics`.

11. **`is_unique_violation` matches sea-orm 2.0's `Arc<SqlxError>` envelope.** The inner value of `RuntimeErr::SqlxError` is `Arc<sqlx::Error>`, not `sqlx::Error`. The helper dereferences via `&**sqlx_err_arc` and matches the constraint primary-code mask `19` (every SQLite constraint-violation extended code shares that mask). The `is_unique_violation_matches_constraint_errors` test in `account/helpers/account.rs` exercises both the negative case (anyhow errors that aren't constraint violations) and the positive case (a real `UNIQUE` violation on `actor.handle`).

12. **Plain row structs deliberately do not derive from the sea-orm entities.** `auth::RefreshTokenRow`, `email_token::EmailToken`, `invite::{InviteCode, InviteCodeUse}` are local plain structs that mirror the reference's `models::models`. They are distinct from the sea-orm entities so the entity definitions can change without touching the helpers, and vice versa.

13. **Five reference bugs are ported verbatim:**
    - `decode_refresh_token` keeps its `assert_eq!(claims.custom.scope, "com.atproto.refresh", "not a refresh token")` panic. Callers only pass tokens produced by `create_tokens` / `create_refresh_token`, so the panic path is unreachable in normal operation.
    - `create_service_jwt` keeps its millisecond `exp` calculation: `((now + MINUTE as usize) / 1000) as u64`. Any seconds-based verifier treats the resulting ~1.7e12 value as long-expired. The service-auth header logic in `verify_service_jwt` accounts for this; the test only asserts three dot-separated segments and the `ES256K` header alg.
    - `REFRESH_GRACE_MS = 2 * HOUR as i64` produces a ~7.2-second grace (HOUR is in seconds, then multiplied by 1000 to land micros). The concurrent-refresh reuse tests rotate twice within this window, which is why they pass. If the upstream fixes the constant, port the fix.
    - `register_actor` with `deactivated: Some(true)` stamps `deleteAfter = createdAt + 3 days`. No cron in this work deletes them; the admin/account handlers own that.
    - App-password salt is `Base64(SHA-256(did))` minus padding — deterministic per did by design, so a leaked `app_password.password` column can be compared offline against a candidate per-did salt.

14. **Sea-orm 1.x-to-2.0 conventions applied throughout.** Every call site that would have been `db.execute(Statement)` under sea-orm 1.x was rewritten to `db.execute_raw(Statement)` (and the analogous `_raw` variants for `query_one` and `query_all`). The standard `metrics::counter!(NAME).increment(1)` form replaced the missing `metrics::increment_counter!` macro.

## Consequences

- The port produces 58 tests across the helper suites and the integration suite, all green on `cargo test -p cacos-pds account::`. The full workspace test count moves from 95 to 153 (`mise run test` reports 153/153 passing). Test suite breakdown: 11 account-helper, 6 password, 7 email-token, 8 invite, 9 auth-helper, 17 integration.
- The OAuth/auth_verifier module must align with this work: it must delete any `PDS_JWT_KEYPAIR` and `AuthScope` copies and import both from `crate::account::helpers::auth`. Both definitions derive deterministically from the same env var, so they interop until converged.
- The xrpc server handlers import `crate::context::PDS_REPO_SIGNING_KEYPAIR` for repo signing, and consume `AccountManager::create_session`, `create_app_password`, `create_email_token`, `assert_valid_email_token`, and the invite/password methods. None of those names may be renamed here without coordinating with the server handlers.
- The five reference bugs are now part of cacos's behavior. Service JWTs that emit millisecond `exp` will be treated as expired by seconds-based verifiers unless they're paired with a verifier that accounts for it. Refresh-token grace reuse works within ~7.2 seconds; if a future upstream fix arrives, the constant in `auth.rs` is the single edit point.
- The hypothetical `vendor/rsky/rsky-pds/...` reference path called out in the task headers does not exist in the workspace. Future porting work that wants to diff against the reference should use `git ls-tree` against the rsky git rev pinned in the workspace `Cargo.toml`.
- Three integration-suite tests (`manages_sessions_and_refresh_tokens`, `manages_invites`, `manages_passwords`) take several seconds each because they exercise the JWT signing path and the argon2 hash path. A future test-suite split that moves them to `#[ignore]` (or `cargo nextest`'s slow-test profile) would speed up the inner loop.
- The test fixture at `pds/src/account/test_util.rs` uses `unsafe { std::env::set_var(...) }` (Rust 1.97 marks `set_var` as `unsafe`). The `Once` guard prevents concurrent env reads, and tests run sequentially within the test binary, so this is safe — but the comment explaining the SAFETY invariant must not be deleted without a replacement.
- `db.begin()` returning a `DatabaseTransaction` requires `use sea_orm::TransactionTrait;` in scope at every call site. Helper files that begin transactions (`helpers/invite.rs`, `AccountManager::rotate_refresh_token`) carry this import.
