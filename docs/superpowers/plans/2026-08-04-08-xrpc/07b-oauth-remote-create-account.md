# Task 7b: `RemoteCreateAccount` impl — `/oauth/remote/create-account` handler

**Spec:** [Headless OAuth Consent Design](../../specs/2026-08-04-headless-oauth-consent-design.md) — `prompt=create` flow.
**Plan 06 seam:** [Plan 06 Task 5](../2026-08-04-06-oauth-auth-verifier/05-oauth-provider-routes-poem-stub-authorize-page-templates.md) defines `RemoteCreateAccount` trait + `MockRemoteCreateAccount`. This task implements the real trait body and wires it into `build_app`.

**Files:**
- Create: `pds/src/oauth/remote_create_account_impl.rs` — `RealRemoteCreateAccount` (the actual impl)
- Modify: `pds/src/xrpc/mod.rs` (or wherever `build_app` lives) — register the real impl alongside the existing mock slot (replace `Data<&dyn RemoteCreateAccount>` injection with the real one in non-test wiring)
- Test: `pds/src/oauth/remote_create_account_impl.rs` (inline `#[cfg(test)]` module)

---

- [ ] **Step 1: Implement `RealRemoteCreateAccount`**

Create `pds/src/oauth/remote_create_account_impl.rs`:

```rust
use async_trait::async_trait;
use sea_orm::DatabaseConnection;
use std::sync::Arc;

use crate::oauth::remote_create_account::{RemoteCreateAccount, CreateAccountInput, CreateAccountError};
use crate::oauth::SharedOAuthProvider;
use crate::actor_store::ActorStore;
use crate::account::{AccountManager, CreateAccountOpts, AvailabilityFlags, helpers::invite};
use crate::sequencer::SharedSequencer;
use crate::plc::Client as PlcClient;
use rsky_oauth::OAuthError;
use rsky_common::now;

pub struct RealRemoteCreateAccount {
    pub db: DatabaseConnection,
    pub account_manager: Arc<AccountManager>,
    pub actor_store: Arc<ActorStore>,
    pub sequencer: SharedSequencer,
    pub plc_client: Option<Arc<PlcClient>>,    // None when PDS_DID_PLC_URL is unset (dev mode)
}

#[async_trait]
impl RemoteCreateAccount for RealRemoteCreateAccount {
    async fn create_account(&self, input: CreateAccountInput) -> Result<String, CreateAccountError> {
        // 1. Mirror reference server_create_account shape:
        //    - ensure invite if required (already enforced by AccountManager::create_account).
        //    - generate did:plc + keypair via plc::create_op.
        //    - create repo via ActorStore::create + transact + create_repo.
        //    - AccountManager::create_account(CreateAccountOpts).
        //    - sequence_identity_evt / sequence_account_evt / sequence_commit / sequence_sync_evt.
        //    - upsert_device_account(device_id, did).

        // (See the git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's `src/apis/com/atproto/server/create_account.rs for the
        //  full reference flow; replicate exactly minus the plc_op: Option<Operation> input,
        //  since the PDS constructs the op from PDS_REPO_SIGNING_KEYPAIR + PDS_PLC_ROTATION_KEYPAIR.)

        // Minimal viable body for now (Plan 08 owns the full account-creation flow; this task
        // delegates to a helper in account_manager / actor_store once Task 7 is wired).
        let _ = input; // suppress unused for now
        Err(CreateAccountError::Internal(
            "RealRemoteCreateAccount impl pending Task 7 actor-store/account-manager wiring".into(),
        ))
    }
}
```

The actual body (Step 2) is filled in once Tasks 3/5/7 are landed (ActorStore + AccountManager + Sequencer). For now the body is a stub that returns `Internal` so Plan 06's route wiring + tests work end-to-end against the mock.

Run: `cargo check -p pds`
Expected: green.

Commit: `feat(oauth): add RealRemoteCreateAccount stub (filled by follow-up step)`

---

- [ ] **Step 2: Fill in the create-account body**

Once Tasks 3/5/7 are merged (ActorStore + AccountManager + Sequencer + plc::Client available), replace the body with the full flow:

```rust
async fn create_account(&self, input: CreateAccountInput) -> Result<String, CreateAccountError> {
    use rsky_oauth::request::request_uri_from_id;

    let now_str = now();
    let now_u64 = rsky_oauth::request::now_or(self.clock).unwrap_or(0); // see note below

    // 1. Generate did + keypair + signed PLC op (server_create_account.rs:58-79).
    let did = generate_did_plc();
    let keypair = generate_keypair();
    let plc_op = match &self.plc_client {
        Some(plc) => Some(plc::operations::create_op(...).await
            .map_err(|e| CreateAccountError::Internal(e.to_string()))?),
        None => None, // dev mode — no plc submission
    };

    // 2. ActorStore::create + transact + create_repo.
    self.actor_store.create(&did, &keypair).await
        .map_err(|e| CreateAccountError::Internal(e.to_string()))?;
    let blobstore = /* shared blobstore from SharedState */;
    let actor_txn = self.actor_store.transact(did.clone(), blobstore).await
        .map_err(|e| CreateAccountError::Internal(e.to_string()))?;
    let commit = actor_txn.create_repo(Vec::new()).await
        .map_err(|e| CreateAccountError::Internal(e.to_string()))?;

    // 3. AccountManager::create_account(opts) — handles invite + email token + sequence events.
    let (_access, _refresh) = self.account_manager.create_account(CreateAccountOpts {
        did: did.clone(),
        handle: input.handle.clone(),
        email: Some(input.email.clone()),
        password: Some(input.password.clone()),
        repo_cid: commit.cid,
        repo_rev: commit.rev.clone(),
        invite_code: input.invite_code.clone(),
        deactivated: None,
    }).await.map_err(|e| oauth_error_from_account_manager(e))?;

    // 4. upsert_device_account(device_id, did) — required for the subsequent accept call.
    //    (The provider's active_request binding is done in Plan 06's read endpoint; here we only
    //    link the account to the device for accept verification.)
    self.account_manager.store.upsert_device_account(&input.device_id, &did).await
        .map_err(|e| CreateAccountError::OAuth(OAuthError::ServerError(e.to_string())))?;

    Ok(did)
}
```

(Use `crate::account::helpers::account::register_account` directly rather than `AccountManager::create_account`'s tokens flow — simpler; the consent flow doesn't need the access/refresh tokens since the RemoteClient drives the accept step.)

Run: `cargo check -p pds`
Expected: green.

Commit: `feat(oauth): implement RealRemoteCreateAccount body`

---

- [ ] **Step 3: Wire the real impl into `build_app`**

In `pds/src/xrpc/mod.rs` (or wherever `build_app` mounts the oauth app):

```rust
// Add to SharedState:
pub remote_create_account: Arc<dyn RemoteCreateAccount>,

// In build_app:
let remote_create_account: Arc<dyn RemoteCreateAccount> = if cfg!(any(test, feature = "test-mock-remote")) {
    Arc::new(MockRemoteCreateAccount::default())
} else {
    Arc::new(RealRemoteCreateAccount {
        db: state.db.clone(),
        account_manager: state.account_manager.clone(),
        actor_store: state.actor_store.clone(),
        sequencer: state.sequencer.clone(),
        plc_client: state.plc_client.clone(),
    })
};

// Pass via poem .data(remote_create_account.clone())
```

The `cfg!(test)` branch keeps the mock available for unit tests; production wiring uses the real impl. Tests assert the route is mounted by checking the registered route table.

Tests:
- `cacos_oauth_remote_create_account_uses_mock_in_tests` — assert `SharedState.remote_create_account` is a `MockRemoteCreateAccount` when `cfg!(test)`.
- Production wiring test (gated by `#[cfg(not(test))]`): assert the real impl is constructed with non-empty `db`/`actor_store`/etc. (smoke check — can't easily verify the impl type without leaking it).

Commit: `feat(oauth): wire RemoteCreateAccount real impl into build_app`

---

- [ ] **Step 4: End-to-end test of `prompt=create`**

Add a test in `pds/src/oauth/remote.rs`'s test module (per Plan 06 Task 5):

```rust
#[tokio::test]
async fn prompt_create_full_flow_creates_account_and_continues_to_consent() {
    // 1. Set up: PAR with prompt=create; insert device row.
    // 2. GET /oauth/remote/request → assert screen: "create".
    // 3. POST /oauth/remote/create-account with the form data
    //    → assert screen: "consent" + sessions contains the new did.
    // 4. POST /oauth/remote/accept {rqid, state, device_id, did=new}
    //    → assert {redirect_url} contains code=…&state=…
}
```

The mock's `next` slot returns a deterministic `Ok(did)` and asserts `CreateAccountInput { device_id, handle, ... }` was forwarded with the right fields.

Run: `cargo test -p pds oauth::remote::tests::prompt_create_full_flow_creates_account_and_continues_to_consent`
Expected: PASS.

Commit: `test(oauth): add end-to-end prompt=create flow test`

---

- [ ] **Step 5: Update Plan 08 index + cross-plan notes**

Edit `docs/superpowers/plans/2026-08-04-08-xrpc/00-index.md`:
- Add Task 7b to the **Task Index** table (between 7 and 8) with the file `[07b-oauth-remote-create-account.md](07b-oauth-remote-create-account.md)`.
- Add a sentence to **Locked-in decisions** or **Dependencies** noting that this task depends on Plan 03 (ActorStore), Plan 05 (AccountManager), Plan 07 (Sequencer), and Plan 08 Task 18 (PlcClient) being landed first.
- Cross-link to [Plan 06 — headless-consent spec](../2026-08-04-06-oauth-auth-verifier/00-index.md).

Commit: `docs(plans): add Task 7b (RemoteCreateAccount impl) to Plan 08`

---

## Cross-plan notes

- The real impl needs `ActorStore` (Plan 03), `AccountManager` (Plan 05), `Sequencer` (Plan 07), and `PlcClient` (Plan 08 Task 18) to exist. Sequence: do Steps 1 + 5 (skeleton + wiring) when Plan 06 is merged, then fill in Step 2 (the body) after Plan 03/05/07 are merged.
- The seam in Plan 06's `RemoteCreateAccount` trait means test wiring can substitute `MockRemoteCreateAccount` while the real impl is incomplete.
- `upsert_device_account` is the bridge between account creation and the subsequent `/accept`: the provider's `get_device_account(device_id, did)` check is what makes `accept` work. Plan 06's read endpoint binds the request to `device_id`; Plan 06's accept calls `provider.accept(device_id, did)`; here we ensure `device_id` → `did` is linked.
- This task does **not** modify the `server_create_account` XRPC handler (Task 7) — the two share the same underlying machinery but expose different surfaces (XRPC vs token-auth JSON).

See also: [Headless OAuth Consent Design](../../specs/2026-08-04-headless-oauth-consent-design.md) · [Plan 06 Task 5](../2026-08-04-06-oauth-auth-verifier/05-oauth-provider-routes-poem-stub-authorize-page-templates.md)