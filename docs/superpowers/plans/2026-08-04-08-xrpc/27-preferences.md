# Task 27: Preferences — `actor_store/preference/` port + app.bsky.actor.getPreferences/putPreferences

Plan 03 did not port the pref store; this task adds the minimal `PreferenceReader` over the `account_pref` table and the two app.bsky handlers (decision in the header).

**Files:**
- Create: `pds/src/actor_store/preference/mod.rs`
- Create: `pds/src/actor_store/preference/util.rs`
- Modify: `pds/src/actor_store/mod.rs` (expose `pref` on the read/transact handles, per Plan 03's pattern)
- Create: `pds/src/xrpc/app/bsky/actor/get_preferences.rs`
- Create: `pds/src/xrpc/app/bsky/actor/put_preferences.rs`
- Modify: `pds/src/xrpc/app/bsky/actor/mod.rs` (route table; replace Task 1 placeholder)
- Test: `pds/tests/preferences_test.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// pds/tests/preferences_test.rs
use pds::xrpc::build_app;
use pds::xrpc::test_utils::{create_test_account, test_state};
use poem::test::TestClient;
use serde_json::json;

#[tokio::test]
async fn put_then_get_preferences() {
    let (state, _dirs) = test_state().await;
    let (access, _refresh) = create_test_account(&state, "did:plc:alice", "alice.test").await;
    let app = build_app(state);
    let cli = TestClient::new(app);

    let resp = cli
        .post("/xrpc/app.bsky.actor.putPreferences")
        .header("Authorization", format!("Bearer {access}"))
        .body_json(&json!({
            "preferences": [
                { "$type": "app.bsky.actor.defs#adultContentPref", "enabled": true }
            ]
        }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);

    let resp = cli
        .get("/xrpc/app.bsky.actor.getPreferences")
        .header("Authorization", format!("Bearer {access}"))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    let prefs = body["preferences"].as_array().unwrap();
    assert_eq!(prefs.len(), 1);
    assert_eq!(prefs[0]["$type"], "app.bsky.actor.defs#adultContentPref");
    assert_eq!(prefs[0]["enabled"], true);
}

#[tokio::test]
async fn put_preferences_outside_namespace_rejected() {
    let (state, _dirs) = test_state().await;
    let (access, _refresh) = create_test_account(&state, "did:plc:bob", "bob.test").await;
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/app.bsky.actor.putPreferences")
        .header("Authorization", format!("Bearer {access}"))
        .body_json(&json!({
            "preferences": [
                { "$type": "com.example.defs#somePref", "enabled": true }
            ]
        }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::BAD_REQUEST);
}
```

> **Test note:** `putPreferences`/`getPreferences` need the actor DB to exist (the `account_pref` table lives in the per-user DB). `create_test_account` creates the account row but not the actor store file; the preference handlers open the actor DB through Plan 03's per-user DB bootstrap (migrate-on-open). Tests create the actor store first via `seed_actor_repo` (Task 12) — add that call to both tests if Plan 03 requires an explicit open.

Run: `cargo test -p pds --test preferences_test`
Expected: FAIL — modules/handlers missing.

- [ ] **Step 2: Implement `preference/util.rs` and `preference/mod.rs`**

Port of `the git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's `src/actor_store/preference/{mod.rs,util.rs}`.

```rust
// pds/src/actor_store/preference/util.rs
use crate::auth::auth_verifier::AuthScope;

const FULL_ACCESS_ONLY_PREFS: [&str; 1] = ["app.bsky.actor.defs#personalDetailsPref"];

pub fn pref_in_scope(scope: AuthScope, pref_type: String) -> bool {
    if scope == AuthScope::Access {
        return true;
    }
    !FULL_ACCESS_ONLY_PREFS.contains(&&*pref_type)
}
```

```rust
// pds/src/actor_store/preference/mod.rs
use crate::actor_store::db::ActorDb;
use crate::actor_store::preference::util::pref_in_scope;
use crate::auth::auth_verifier::AuthScope;
use anyhow::{bail, Result};
use rsky_lexicon::app::bsky::actor::RefPreferences;

pub struct PreferenceReader {
    pub did: String,
    pub db: ActorDb,
}

impl PreferenceReader {
    pub fn new(did: String, db: ActorDb) -> Self {
        PreferenceReader { did, db }
    }

    pub async fn get_preferences(
        &self,
        namespace: Option<String>,
        scope: AuthScope,
    ) -> Result<Vec<RefPreferences>> {
        let rows: Vec<(String, String)> = self
            .db
            .run(|conn| {
                let mut stmt =
                    conn.prepare("SELECT name, \"valueJson\" FROM account_pref ORDER BY id ASC")?;
                let rows = stmt
                    .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .collect::<Result<Vec<(String, String)>, rusqlite::Error>>()?;
                Ok(rows)
            })
            .await?;
        rows.into_iter()
            .filter(|(name, _)| match &namespace {
                None => true,
                Some(namespace) => pref_match_namespace(namespace, name),
            })
            .filter(|(name, _)| pref_in_scope(scope.clone(), name.clone()))
            .map(
                |(name, value_json)| match serde_json::from_str::<RefPreferences>(&value_json) {
                    Ok(pref) => Ok(pref),
                    Err(error) => bail!("preferences json invalid for {name}: {error}"),
                },
            )
            .collect::<Result<Vec<RefPreferences>>>()
    }

    #[tracing::instrument(skip_all)]
    pub async fn put_preferences(
        &self,
        values: Vec<RefPreferences>,
        namespace: String,
        scope: AuthScope,
    ) -> Result<()> {
        if !values
            .iter()
            .all(|value| pref_match_namespace(&namespace, &value.get_type()))
        {
            bail!("Some preferences are not in the {namespace} namespace")
        }
        let not_in_scope = values
            .iter()
            .filter(|value| !pref_in_scope(scope.clone(), value.get_type()))
            .collect::<Vec<&RefPreferences>>();
        if !not_in_scope.is_empty() {
            bail!("Do not have authorization to set preferences.");
        }
        let put_prefs = values
            .into_iter()
            .map(|value| Ok((value.get_type(), serde_json::to_string(&value)?)))
            .collect::<Result<Vec<(String, String)>>>()?;
        self.db
            .tx(move |tx| {
                let mut stmt = tx.prepare("SELECT id, name FROM account_pref")?;
                let all_prefs = stmt
                    .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .collect::<Result<Vec<(i64, String)>, rusqlite::Error>>()?;
                drop(stmt);
                let all_pref_ids_in_namespace = all_prefs
                    .iter()
                    .filter(|(_, name)| pref_match_namespace(&namespace, name))
                    .filter(|(_, name)| pref_in_scope(scope.clone(), name.clone()))
                    .map(|(id, _)| *id)
                    .collect::<Vec<i64>>();
                if !all_pref_ids_in_namespace.is_empty() {
                    let sql = format!(
                        "DELETE FROM account_pref WHERE id IN ({})",
                        placeholders(all_pref_ids_in_namespace.len())
                    );
                    tx.execute(
                        &sql,
                        rusqlite::params_from_iter(all_pref_ids_in_namespace.iter()),
                    )?;
                }
                if !put_prefs.is_empty() {
                    let mut stmt = tx.prepare(
                        "INSERT INTO account_pref (name, \"valueJson\") VALUES (?1, ?2)",
                    )?;
                    for (name, value_json) in &put_prefs {
                        stmt.execute(rusqlite::params![name, value_json])?;
                    }
                }
                Ok(())
            })
            .await
    }
}

pub fn pref_match_namespace(namespace: &str, fullname: &str) -> bool {
    fullname == namespace || fullname.starts_with(&format!("{namespace}."))
}

/// Generates `(?,?,...)` for a count — same helper as Plan 03's sql_repo.
pub fn placeholders(count: usize) -> String {
    std::iter::repeat("?")
        .take(count)
        .collect::<Vec<_>>()
        .join(",")
}

pub mod util;
```

> **Plan 03 note:** `ActorDb` (with `run` and `tx`) and the `account_pref` table migration come from Plan 03/Plan 01's actor migrator (the PROMPT.md lists the lexicon tables in `account_migrator.rs`). If Plan 03 placed `ActorDb` elsewhere, adjust the import.

- [ ] **Step 3: Implement `get_preferences.rs` and `put_preferences.rs`**

```rust
// pds/src/xrpc/app/bsky/actor/get_preferences.rs
use crate::xrpc::auth_extractors::AccessStandard;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use anyhow::Result;
use poem::web::Json;
use poem::State;
use rsky_lexicon::app::bsky::actor::{GetPreferencesOutput, RefPreferences};

async fn inner_get_preferences(
    auth: AccessStandard,
    state: &SharedState,
) -> Result<GetPreferencesOutput> {
    let auth = auth.access.credentials.unwrap();
    let requester = auth.did.unwrap().clone();
    let actor_store = state
        .actor_store
        .read(requester.clone(), state.blobstore.clone())
        .await?;
    let preferences: Vec<RefPreferences> = actor_store
        .pref
        .get_preferences(Some("app.bsky".to_string()), auth.scope.unwrap())
        .await?;

    Ok(GetPreferencesOutput { preferences })
}

/// GET /xrpc/app.bsky.actor.getPreferences
#[poem::handler]
pub async fn get_preferences(
    auth: AccessStandard,
    state: State<SharedState>,
) -> ApiResult<Json<GetPreferencesOutput>> {
    match inner_get_preferences(auth, &state).await {
        Ok(res) => Ok(Json(res)),
        Err(error) => {
            tracing::error!("{error}");
            Err(ApiError::RuntimeError)
        }
    }
}
```

```rust
// pds/src/xrpc/app/bsky/actor/put_preferences.rs
use crate::xrpc::auth_extractors::AccessStandard;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use anyhow::Result;
use poem::web::Json;
use poem::State;
use rsky_lexicon::app::bsky::actor::PutPreferencesInput;

async fn inner_put_preferences(
    body: PutPreferencesInput,
    auth: AccessStandard,
    state: &SharedState,
) -> Result<(), ApiError> {
    let PutPreferencesInput { preferences } = body;
    let auth = auth.access.credentials.unwrap();
    let requester = auth.did.unwrap().clone();
    let actor_store = state
        .actor_store
        .transact(requester.clone(), state.blobstore.clone())
        .await
        .map_err(|_| ApiError::RuntimeError)?;
    actor_store
        .pref
        .put_preferences(preferences, "app.bsky".to_string(), auth.scope.unwrap())
        .await
        .map_err(|_| ApiError::RuntimeError)?;
    Ok(())
}

/// POST /xrpc/app.bsky.actor.putPreferences
#[poem::handler]
pub async fn put_preferences(
    body: Json<PutPreferencesInput>,
    auth: AccessStandard,
    state: State<SharedState>,
) -> ApiResult<()> {
    match inner_put_preferences(body.0, auth, &state).await {
        Ok(_) => Ok(()),
        Err(error) => Err(error),
    }
}
```

- [ ] **Step 4: Replace the app.bsky.actor placeholder with the route table**

```rust
// pds/src/xrpc/app/bsky/actor/mod.rs
pub mod get_preferences;
pub mod put_preferences;

pub fn routes() -> poem::Route {
    use poem::get;
    use poem::post;
    poem::Route::new()
        .at("/getPreferences", get(get_preferences::get_preferences))
        .at("/putPreferences", post(put_preferences::put_preferences))
}
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p pds --test preferences_test`
Expected: PASS (2 tests).

- [ ] **Step 6: Commit**

```bash
git add pds/src/actor_store/preference pds/src/xrpc/app pds/tests/preferences_test.rs
git commit -m "feat(prefs): PreferenceReader + app.bsky get/putPreferences"
```
