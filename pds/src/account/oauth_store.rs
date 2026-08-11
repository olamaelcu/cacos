//! OAuth store over sea-orm + sqlite.
//!
//! Implements [`rsky_oauth::store::OAuthStore`] for the cacos PDS. The
//! migration uses [`migration::types::db_id::DbId`] ULIDs as surrogate
//! primary keys on the `authorization_request`, `device`, `token`,
//! `account_device`, and `used_refresh_token` tables (per ADR 0002), and
//! stores the externally-meaningful opaque string identifiers rsky-oauth
//! generates (`requestId` on `authorization_request`, `deviceId` on
//! `device`) in companion TEXT UNIQUE columns — mirroring the existing
//! `token.tokenId` pattern. Internal FK columns reference the surrogate
//! `DbId`; the store resolves the external TEXT keys to DbIds at the
//! boundary.

use crate::db::entities::{
    account_device, authorization_request, device, token, used_refresh_token,
};
use migration::types::db_id::DbId;

use crate::account::helpers::account::{
    ActorAccount, AvailabilityFlags, get_account, get_account_by_email,
};
use crate::account::helpers::password::verify_account_password;

use rsky_oauth::OAuthError;
use rsky_oauth::request::RequestData;
use rsky_oauth::store::{AccountInfo, DeviceData, OAuthStore};
use rsky_oauth::token::{TokenData, TokenInfo};
use rsky_oauth::types::{AuthorizationRequestParameters, ClientAuth};
use sea_orm::sea_query::OnConflict;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, PaginatorTrait,
    QueryFilter, QuerySelect, Set, TransactionTrait,
};
use std::str::FromStr;
use time::OffsetDateTime;

fn server_error(err: impl std::fmt::Display) -> OAuthError {
    OAuthError::ServerError(err.to_string())
}

fn secs_to_offset(secs: u64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(secs as i64).expect("unix seconds within time range")
}

fn offset_to_secs(o: OffsetDateTime) -> u64 {
    o.unix_timestamp().max(0) as u64
}

fn actor_to_account_info(actor: ActorAccount) -> AccountInfo {
    AccountInfo {
        did: actor.did,
        handle: actor.handle,
        email: actor.email,
        deactivated: actor.deactivated_at.is_some(),
    }
}

fn to_json<T: serde::Serialize>(value: &T) -> Result<String, OAuthError> {
    serde_json::to_string(value).map_err(server_error)
}

fn from_json<T: serde::de::DeserializeOwned>(json: &str) -> Result<T, OAuthError> {
    serde_json::from_str(json).map_err(server_error)
}

async fn device_id_from_external<C>(db: &C, device_id: &str) -> Result<Option<DbId>, OAuthError>
where
    C: sea_orm::ConnectionTrait,
{
    let row = device::Entity::find()
        .filter(device::Column::DeviceId.eq(device_id.to_owned()))
        .select_only()
        .column(device::Column::Id)
        .into_tuple::<DbId>()
        .one(db)
        .await
        .map_err(server_error)?;
    Ok(row)
}

fn row_to_request_data(row: authorization_request::Model) -> Result<RequestData, OAuthError> {
    Ok(RequestData {
        client_id: row.client_id,
        client_auth: from_json::<ClientAuth>(&row.client_auth)?,
        parameters: from_json::<AuthorizationRequestParameters>(&row.parameters)?,
        expires_at: offset_to_secs(row.expires_at),
        device_id: row.external_device_id,
        did: row.did.map(|d| d.into_inner()),
        code: row.code,
    })
}

fn row_to_token_data(row: token::Model) -> Result<TokenData, OAuthError> {
    Ok(TokenData {
        created_at: offset_to_secs(row.created_at),
        updated_at: offset_to_secs(row.updated_at),
        expires_at: offset_to_secs(row.expires_at),
        client_id: row.client_id,
        client_auth: from_json::<ClientAuth>(&row.client_auth)?,
        device_id: row.external_device_id,
        did: row.did.into_inner(),
        parameters: from_json::<AuthorizationRequestParameters>(&row.parameters)?,
        code: row.code,
    })
}

fn row_to_device_data(row: device::Model) -> DeviceData {
    DeviceData {
        session_id: row.session_id,
        user_agent: row.user_agent,
        ip_address: row.ip_address,
        last_seen_at: offset_to_secs(row.last_seen_at),
    }
}

#[derive(Clone, Debug)]
pub struct PdsOAuthStore {
    db: sea_orm::DatabaseConnection,
}

impl PdsOAuthStore {
    pub fn new(db: sea_orm::DatabaseConnection) -> Self {
        Self { db }
    }

    pub fn db(&self) -> &sea_orm::DatabaseConnection {
        &self.db
    }

    /// Returns `true` if `did` exists and is currently locked out. An expired
    /// lockout (locked_until <= now) is treated as unlocked.
    pub async fn is_account_locked(&self, did: &str) -> Result<bool, OAuthError> {
        let (_, locked_until) =
            crate::account::helpers::account::get_account_lockout_state(did, &self.db)
                .await
                .map_err(server_error)?;
        match locked_until {
            None => Ok(false),
            Some(until) => Ok(until > unix_now_secs()),
        }
    }

    /// Stamp the lockout deadline (unix seconds) for `did`.
    pub async fn lock_account(&self, did: &str, until_unix: i64) -> Result<(), OAuthError> {
        crate::account::helpers::account::set_account_locked_until(did, until_unix, &self.db)
            .await
            .map_err(server_error)
    }

    /// Increment the failed-login counter for `did`.
    pub async fn record_failed_login(&self, did: &str) -> Result<(), OAuthError> {
        crate::account::helpers::account::increment_failed_login_count(did, &self.db)
            .await
            .map_err(server_error)?;
        Ok(())
    }

    /// Clear the failed-login counter and any lockout deadline for `did`.
    pub async fn clear_failed_logins(&self, did: &str) -> Result<(), OAuthError> {
        crate::account::helpers::account::clear_account_lockout(did, &self.db)
            .await
            .map_err(server_error)
    }

    /// Records that `did`'s PLC document has been rewritten to drop the
    /// shared server rotation key. Used by the `migrate plc-rotation-keys`
    /// command to checkpoint progress so an interrupted pass resumes
    /// where it stopped.
    pub async fn mark_plc_rotation_keys_migrated(&self, did: &str) -> Result<(), OAuthError> {
        crate::account::helpers::account::mark_plc_rotation_keys_migrated(did, &self.db)
            .await
            .map_err(server_error)
    }

    /// Whether `did`'s PLC document has already been rewritten to drop
    /// the shared server rotation key. Unknown DIDs read as not migrated.
    pub async fn is_plc_rotation_keys_migrated(&self, did: &str) -> Result<bool, OAuthError> {
        crate::account::helpers::account::is_plc_rotation_keys_migrated(did, &self.db)
            .await
            .map_err(server_error)
    }
}

fn unix_now_secs() -> i64 {
    let system_time = std::time::SystemTime::now();
    let dt: OffsetDateTime = system_time.into();
    dt.unix_timestamp()
}

#[async_trait::async_trait]
impl OAuthStore for PdsOAuthStore {
    async fn create_request(&self, id: &str, data: &RequestData) -> Result<(), OAuthError> {
        let device_db_id = match &data.device_id {
            Some(external) => device_id_from_external(&self.db, external).await?,
            None => None,
        };
        let client_auth = to_json(&data.client_auth)?;
        let parameters = to_json(&data.parameters)?;
        let model = authorization_request::ActiveModel {
            id: Set(DbId::new()),
            request_id: Set(id.to_owned()),
            did: Set(data.did.clone().map(Into::into)),
            device_id: Set(device_db_id),
            external_device_id: Set(data.device_id.clone()),
            client_id: Set(data.client_id.clone()),
            client_auth: Set(client_auth),
            parameters: Set(parameters),
            expires_at: Set(secs_to_offset(data.expires_at)),
            code: Set(data.code.clone()),
        };
        model.insert(&self.db).await.map_err(server_error)?;
        Ok(())
    }

    async fn read_request(&self, id: &str) -> Result<Option<RequestData>, OAuthError> {
        let row = authorization_request::Entity::find()
            .filter(authorization_request::Column::RequestId.eq(id.to_owned()))
            .one(&self.db)
            .await
            .map_err(server_error)?;
        let row = match row {
            Some(r) => Some(r),
            None => match DbId::from_str(id) {
                Ok(dbid) => authorization_request::Entity::find_by_id(dbid)
                    .one(&self.db)
                    .await
                    .map_err(server_error)?,
                Err(_) => None,
            },
        };
        row.map(row_to_request_data).transpose()
    }

    async fn update_request(&self, id: &str, data: &RequestData) -> Result<(), OAuthError> {
        let row = authorization_request::Entity::find()
            .filter(authorization_request::Column::RequestId.eq(id.to_owned()))
            .one(&self.db)
            .await
            .map_err(server_error)?
            .ok_or_else(|| OAuthError::ServerError(format!("unknown request id: {id}")))?;
        let device_db_id = match &data.device_id {
            Some(external) => device_id_from_external(&self.db, external).await?,
            None => None,
        };
        let mut active: authorization_request::ActiveModel = row.into_active_model();
        active.did = Set(data.did.clone().map(Into::into));
        active.device_id = Set(device_db_id);
        active.external_device_id = Set(data.device_id.clone());
        active.client_auth = Set(to_json(&data.client_auth)?);
        active.parameters = Set(to_json(&data.parameters)?);
        active.expires_at = Set(secs_to_offset(data.expires_at));
        active.code = Set(data.code.clone());
        active.update(&self.db).await.map_err(server_error)?;
        Ok(())
    }

    async fn delete_request(&self, id: &str) -> Result<(), OAuthError> {
        let res = authorization_request::Entity::delete_many()
            .filter(authorization_request::Column::RequestId.eq(id.to_owned()))
            .exec(&self.db)
            .await
            .map_err(server_error)?;
        if res.rows_affected == 0 {
            return Err(OAuthError::ServerError(format!("unknown request id: {id}")));
        }
        Ok(())
    }

    async fn consume_request_code(
        &self,
        code: &str,
    ) -> Result<Option<(String, RequestData)>, OAuthError> {
        let deleted = authorization_request::Entity::delete_many()
            .filter(authorization_request::Column::Code.eq(code.to_owned()))
            .filter(authorization_request::Column::Code.is_not_null())
            .exec_with_returning(&self.db)
            .await
            .map_err(server_error)?;
        let Some(row) = deleted.into_iter().next() else {
            return Ok(None);
        };
        let request_id = row.request_id.clone();
        let data = row_to_request_data(row)?;
        Ok(Some((request_id, data)))
    }

    async fn create_token(
        &self,
        token_id: &str,
        data: &TokenData,
        refresh_token: Option<&str>,
    ) -> Result<(), OAuthError> {
        let tx = self.db.begin().await.map_err(server_error)?;
        if let Some(refresh_token) = refresh_token {
            let used = used_refresh_token::Entity::find()
                .filter(used_refresh_token::Column::RefreshToken.eq(refresh_token.to_owned()))
                .count(&tx)
                .await
                .map_err(server_error)?;
            if used > 0 {
                return Err(OAuthError::ServerError(
                    "refresh token already in use".to_string(),
                ));
            }
        }
        let device_db_id = match &data.device_id {
            Some(external) => device_id_from_external(&tx, external).await?,
            None => None,
        };
        let model = token::ActiveModel {
            id: Set(DbId::new()),
            did: Set(data.did.clone().into()),
            token_id: Set(token_id.to_owned()),
            created_at: Set(secs_to_offset(data.created_at)),
            updated_at: Set(secs_to_offset(data.updated_at)),
            expires_at: Set(secs_to_offset(data.expires_at)),
            client_id: Set(data.client_id.clone()),
            client_auth: Set(to_json(&data.client_auth)?),
            device_id: Set(device_db_id),
            external_device_id: Set(data.device_id.clone()),
            parameters: Set(to_json(&data.parameters)?),
            details: Set(None),
            code: Set(data.code.clone()),
            current_refresh_token: Set(refresh_token.map(String::from)),
            scope: Set(Some(data.parameters.scope.clone())),
        };
        model.insert(&tx).await.map_err(server_error)?;
        tx.commit().await.map_err(server_error)
    }

    async fn read_token(&self, token_id: &str) -> Result<Option<TokenInfo>, OAuthError> {
        let row = token::Entity::find()
            .filter(token::Column::TokenId.eq(token_id.to_owned()))
            .one(&self.db)
            .await
            .map_err(server_error)?;
        match row {
            Some(row) => {
                let current_refresh_token = row.current_refresh_token.clone();
                let data = row_to_token_data(row)?;
                Ok(Some(TokenInfo {
                    token_id: token_id.to_owned(),
                    data,
                    current_refresh_token,
                }))
            }
            None => Ok(None),
        }
    }

    async fn find_token_by_refresh_token(
        &self,
        refresh_token: &str,
    ) -> Result<Option<TokenInfo>, OAuthError> {
        let used_pk = used_refresh_token::Entity::find()
            .filter(used_refresh_token::Column::RefreshToken.eq(refresh_token.to_owned()))
            .select_only()
            .column(used_refresh_token::Column::TokenId)
            .into_tuple::<DbId>()
            .one(&self.db)
            .await
            .map_err(server_error)?;
        let row = if let Some(pk) = used_pk {
            token::Entity::find_by_id(pk)
                .one(&self.db)
                .await
                .map_err(server_error)?
        } else {
            token::Entity::find()
                .filter(token::Column::CurrentRefreshToken.eq(refresh_token.to_owned()))
                .one(&self.db)
                .await
                .map_err(server_error)?
        };
        match row {
            None => Ok(None),
            Some(row) => {
                let token_id = row.token_id.clone();
                let current_refresh_token = row.current_refresh_token.clone();
                let data = row_to_token_data(row)?;
                Ok(Some(TokenInfo {
                    token_id,
                    data,
                    current_refresh_token,
                }))
            }
        }
    }

    async fn find_token_by_code(&self, code: &str) -> Result<Option<TokenInfo>, OAuthError> {
        let row = token::Entity::find()
            .filter(token::Column::Code.eq(code.to_owned()))
            .filter(token::Column::Code.is_not_null())
            .one(&self.db)
            .await
            .map_err(server_error)?;
        match row {
            Some(row) => {
                let token_id = row.token_id.clone();
                let current_refresh_token = row.current_refresh_token.clone();
                let data = row_to_token_data(row)?;
                Ok(Some(TokenInfo {
                    token_id,
                    data,
                    current_refresh_token,
                }))
            }
            None => Ok(None),
        }
    }

    async fn rotate_token(
        &self,
        token_id: &str,
        new_token_id: &str,
        new_refresh_token: &str,
        updated_at: u64,
        expires_at: u64,
    ) -> Result<(), OAuthError> {
        let tx = self.db.begin().await.map_err(server_error)?;
        let row = token::Entity::find()
            .filter(token::Column::TokenId.eq(token_id.to_owned()))
            .one(&tx)
            .await
            .map_err(server_error)?
            .ok_or_else(|| OAuthError::ServerError(format!("unknown token: {token_id}")))?;
        let pk = row.id;
        if let Some(current_refresh_token) = row.current_refresh_token.clone() {
            let model = used_refresh_token::ActiveModel {
                refresh_token: Set(current_refresh_token),
                token_id: Set(pk),
            };
            model.insert(&tx).await.map_err(server_error)?;
        }
        let reused = used_refresh_token::Entity::find()
            .filter(used_refresh_token::Column::RefreshToken.eq(new_refresh_token.to_owned()))
            .count(&tx)
            .await
            .map_err(server_error)?;
        if reused > 0 {
            return Err(OAuthError::ServerError(
                "new refresh token already in use".to_string(),
            ));
        }
        let mut active: token::ActiveModel = row.into_active_model();
        active.token_id = Set(new_token_id.to_owned());
        active.current_refresh_token = Set(Some(new_refresh_token.to_owned()));
        active.updated_at = Set(secs_to_offset(updated_at));
        active.expires_at = Set(secs_to_offset(expires_at));
        active.update(&tx).await.map_err(server_error)?;
        tx.commit().await.map_err(server_error)
    }

    async fn delete_token(&self, token_id: &str) -> Result<(), OAuthError> {
        let res = token::Entity::delete_many()
            .filter(token::Column::TokenId.eq(token_id.to_owned()))
            .exec(&self.db)
            .await
            .map_err(server_error)?;
        if res.rows_affected == 0 {
            return Err(OAuthError::ServerError(format!(
                "unknown token: {token_id}"
            )));
        }
        Ok(())
    }

    async fn authenticate_account(
        &self,
        identifier: &str,
        password_str: &str,
    ) -> Result<Option<AccountInfo>, OAuthError> {
        let flags = Some(AvailabilityFlags {
            include_taken_down: None,
            include_deactivated: Some(true),
        });
        let identifier_lc = identifier.to_ascii_lowercase();
        let found = if identifier_lc.contains('@') {
            get_account_by_email(&identifier_lc, flags, &self.db)
                .await
                .map_err(server_error)?
        } else {
            get_account(&identifier_lc, flags, &self.db)
                .await
                .map_err(server_error)?
        };
        let Some(actor) = found else {
            return Ok(None);
        };
        if self.is_account_locked(&actor.did).await? {
            return Ok(None);
        }
        let valid = verify_account_password(&actor.did, &password_str.to_string(), &self.db)
            .await
            .map_err(server_error)?;
        if valid {
            self.clear_failed_logins(&actor.did).await?;
        } else {
            self.record_failed_login(&actor.did).await?;
            let (count, _) =
                crate::account::helpers::account::get_account_lockout_state(&actor.did, &self.db)
                    .await
                    .map_err(server_error)?;
            if count >= 5 {
                let until = unix_now_secs() + 900;
                self.lock_account(&actor.did, until).await?;
            }
        }
        Ok(valid.then(|| actor_to_account_info(actor)))
    }

    async fn get_account(&self, did: &str) -> Result<Option<AccountInfo>, OAuthError> {
        let found = get_account(
            did,
            Some(AvailabilityFlags {
                include_taken_down: None,
                include_deactivated: Some(true),
            }),
            &self.db,
        )
        .await
        .map_err(server_error)?;
        Ok(found.map(actor_to_account_info))
    }

    async fn create_device(&self, device_id: &str, data: &DeviceData) -> Result<(), OAuthError> {
        let model = device::ActiveModel {
            id: Set(DbId::new()),
            device_id: Set(device_id.to_owned()),
            session_id: Set(data.session_id.clone()),
            user_agent: Set(data.user_agent.clone()),
            ip_address: Set(data.ip_address.clone()),
            last_seen_at: Set(secs_to_offset(data.last_seen_at)),
        };
        model.insert(&self.db).await.map_err(server_error)?;
        Ok(())
    }

    async fn read_device(&self, device_id: &str) -> Result<Option<DeviceData>, OAuthError> {
        let row = device::Entity::find()
            .filter(device::Column::DeviceId.eq(device_id.to_owned()))
            .one(&self.db)
            .await
            .map_err(server_error)?;
        Ok(row.map(row_to_device_data))
    }

    async fn update_device(&self, device_id: &str, data: &DeviceData) -> Result<(), OAuthError> {
        let row = device::Entity::find()
            .filter(device::Column::DeviceId.eq(device_id.to_owned()))
            .one(&self.db)
            .await
            .map_err(server_error)?
            .ok_or_else(|| OAuthError::ServerError(format!("unknown device: {device_id}")))?;
        let mut active: device::ActiveModel = row.into_active_model();
        active.session_id = Set(data.session_id.clone());
        active.user_agent = Set(data.user_agent.clone());
        active.ip_address = Set(data.ip_address.clone());
        active.last_seen_at = Set(secs_to_offset(data.last_seen_at));
        active.update(&self.db).await.map_err(server_error)?;
        Ok(())
    }

    async fn upsert_device_account(&self, device_id: &str, did: &str) -> Result<(), OAuthError> {
        let now = OffsetDateTime::now_utc();
        let Some(device_db_id) = device_id_from_external(&self.db, device_id).await? else {
            return Err(OAuthError::ServerError(format!(
                "unknown device: {device_id}"
            )));
        };
        let did_typed: migration::types::did::Did = did.to_owned().into();
        let model = account_device::ActiveModel {
            did: Set(did_typed.clone()),
            device_id: Set(device_db_id),
            created_at: Set(now),
            updated_at: Set(now),
        };
        let insert = account_device::Entity::insert(model).exec(&self.db).await;
        // Idempotent upsert: when the (did, deviceId) row already exists we
        // refresh updated_at via a plain UPDATE. `find_also_related` is not
        // available because the entity doesn't declare a relation.
        if insert.is_err() {
            account_device::Entity::update_many()
                .col_expr(account_device::Column::UpdatedAt, now.into())
                .filter(account_device::Column::Did.eq(did_typed))
                .filter(account_device::Column::DeviceId.eq(device_db_id))
                .exec(&self.db)
                .await
                .map_err(server_error)?;
        }
        Ok(())
    }

    async fn get_device_account(
        &self,
        device_id: &str,
        did: &str,
    ) -> Result<Option<AccountInfo>, OAuthError> {
        let Some(device_db_id) = device_id_from_external(&self.db, device_id).await? else {
            return Ok(None);
        };
        let did_typed: migration::types::did::Did = did.to_owned().into();
        let row = account_device::Entity::find()
            .filter(account_device::Column::DeviceId.eq(device_db_id))
            .filter(account_device::Column::Did.eq(did_typed))
            .one(&self.db)
            .await
            .map_err(server_error)?;
        match row {
            Some(_) => self.get_account(did).await,
            None => Ok(None),
        }
    }

    async fn list_device_accounts(&self, device_id: &str) -> Result<Vec<AccountInfo>, OAuthError> {
        let Some(device_db_id) = device_id_from_external(&self.db, device_id).await? else {
            return Ok(Vec::new());
        };
        let rows = account_device::Entity::find()
            .filter(account_device::Column::DeviceId.eq(device_db_id))
            .all(&self.db)
            .await
            .map_err(server_error)?;
        let mut accounts = Vec::with_capacity(rows.len());
        for row in rows {
            if let Some(account) = self.get_account(&row.did.into_inner()).await? {
                accounts.push(account);
            }
        }
        Ok(accounts)
    }

    async fn remove_device_account(&self, device_id: &str, did: &str) -> Result<(), OAuthError> {
        let Some(device_db_id) = device_id_from_external(&self.db, device_id).await? else {
            return Ok(());
        };
        let did_typed: migration::types::did::Did = did.to_owned().into();
        account_device::Entity::delete_many()
            .filter(account_device::Column::DeviceId.eq(device_db_id))
            .filter(account_device::Column::Did.eq(did_typed))
            .exec(&self.db)
            .await
            .map_err(server_error)?;
        Ok(())
    }

    async fn set_authorized_client(
        &self,
        did: &str,
        client_id: &str,
        scope: &str,
    ) -> Result<(), OAuthError> {
        let now = OffsetDateTime::now_utc();
        let authorized_scopes: Vec<&str> = scope.split_ascii_whitespace().collect();
        let data = serde_json::json!({ "authorizedScopes": authorized_scopes }).to_string();
        let did_typed: migration::types::did::Did = did.to_owned().into();
        let model = crate::db::entities::authorized_client::ActiveModel {
            did: Set(did_typed.clone()),
            client_id: Set(client_id.to_owned()),
            created_at: Set(now),
            updated_at: Set(now),
            data: Set(data),
        };
        let insert_res = crate::db::entities::authorized_client::Entity::insert(model)
            .on_conflict(
                OnConflict::columns([
                    crate::db::entities::authorized_client::Column::Did,
                    crate::db::entities::authorized_client::Column::ClientId,
                ])
                .update_columns([
                    crate::db::entities::authorized_client::Column::UpdatedAt,
                    crate::db::entities::authorized_client::Column::Data,
                ])
                .to_owned(),
            )
            .exec(&self.db)
            .await;
        if insert_res.is_err() {
            let new_data = serde_json::json!({ "authorizedScopes": authorized_scopes }).to_string();
            crate::db::entities::authorized_client::Entity::update_many()
                .col_expr(
                    crate::db::entities::authorized_client::Column::UpdatedAt,
                    now.into(),
                )
                .col_expr(
                    crate::db::entities::authorized_client::Column::Data,
                    new_data.into(),
                )
                .filter(
                    crate::db::entities::authorized_client::Column::Did
                        .eq(migration::types::did::Did::from(did.to_owned())),
                )
                .filter(
                    crate::db::entities::authorized_client::Column::ClientId
                        .eq(client_id.to_owned()),
                )
                .exec(&self.db)
                .await
                .map_err(server_error)?;
        }
        Ok(())
    }

    async fn get_authorized_client_scope(
        &self,
        did: &str,
        client_id: &str,
    ) -> Result<Option<String>, OAuthError> {
        let did_typed: migration::types::did::Did = did.to_owned().into();
        let row = crate::db::entities::authorized_client::Entity::find()
            .filter(crate::db::entities::authorized_client::Column::Did.eq(did_typed))
            .filter(
                crate::db::entities::authorized_client::Column::ClientId.eq(client_id.to_owned()),
            )
            .one(&self.db)
            .await
            .map_err(server_error)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let parsed: serde_json::Value = from_json(&row.data)?;
        let scopes = parsed["authorizedScopes"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|s| s.as_str())
                    .collect::<Vec<&str>>()
                    .join(" ")
            })
            .unwrap_or_default();
        Ok(Some(scopes))
    }
}

/// Persistent DPoP `jti` replay store backed by the `dpop_replay` table
/// (added in `migration::m20260801_000008_dpop_replay`). Each row is a
/// `(jti, expires_at)` pair; replay inserts that lose the unique-key
/// race return `false` (replay detected), fresh inserts return `true`.
///
/// A background task schedules [`DbBackedReplayStore::prune_expired`]
/// every five minutes via [`cacos_pds_identity::background::BackgroundQueue`]; the
/// `expires_at` index keeps the prune cheap.
pub struct DbBackedReplayStore {
    pub db: std::sync::Arc<sea_orm::DatabaseConnection>,
}

// Cloning is cheap (inner `Arc`) and lets the same store be passed both as
// `Arc<DbBackedReplayStore>` (for the periodic prune task) and as
// `Box<dyn ReplayStore>` (for the `DpopManager`).
impl Clone for DbBackedReplayStore {
    fn clone(&self) -> Self {
        Self {
            db: self.db.clone(),
        }
    }
}

impl DbBackedReplayStore {
    pub fn new(db: std::sync::Arc<sea_orm::DatabaseConnection>) -> Self {
        Self { db }
    }

    /// Delete every row whose `expires_at` is strictly less than `now`.
    /// Returns the number of rows removed. Used by the periodic prune
    /// task; not part of the [`rsky_oauth::ReplayStore`] trait.
    pub async fn prune_expired(&self, now: u64) -> Result<usize, OAuthError> {
        let res = self
            .db
            .execute_raw(crate::account::helpers::sql(
                "DELETE FROM dpop_replay WHERE expires_at < ?1",
                vec![sea_orm::Value::from(now as i64)],
            ))
            .await
            .map_err(server_error)?;
        Ok(res.rows_affected() as usize)
    }
}

impl rsky_oauth::ReplayStore for DbBackedReplayStore {
    fn consume(&self, jti: &str, exp: u64, _now: u64) -> bool {
        // The `ReplayStore` trait is sync (rsky-oauth calls it from inside
        // the async `check_proof`), but the underlying DB call is async.
        // Bridge sync → async by spinning up a fresh current-thread runtime
        // on a dedicated thread. The dedicated thread is not owned by any
        // outer runtime, so `block_on` cannot deadlock with
        // `#[tokio::main]`'s `current_thread` flavor.
        let db = self.db.clone();
        let jti_owned = jti.to_owned();
        let join: std::thread::JoinHandle<Result<bool, sea_orm::DbErr>> =
            std::thread::Builder::new()
                .name("dpop-replay-store".to_owned())
                .spawn(move || {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("build DPoP replay store runtime");
                    rt.block_on(async move {
                        let stmt = crate::account::helpers::sql(
                            "INSERT INTO dpop_replay (jti, expires_at) VALUES (?1, ?2) \
                             ON CONFLICT (jti) DO NOTHING",
                            vec![
                                sea_orm::Value::from(jti_owned),
                                sea_orm::Value::from(exp as i64),
                            ],
                        );
                        let res = db.execute_raw(stmt).await?;
                        Ok(res.rows_affected() > 0)
                    })
                })
                .expect("spawn DPoP replay store thread");
        match join.join() {
            Ok(Ok(inserted)) => inserted,
            Ok(Err(err)) => {
                tracing::warn!(?err, "dpop replay store insert failed");
                false
            }
            Err(err) => {
                if let Some(msg) = err.downcast_ref::<&str>() {
                    tracing::warn!(%msg, "dpop replay store thread panicked");
                } else if let Some(msg) = err.downcast_ref::<String>() {
                    tracing::warn!(%msg, "dpop replay store thread panicked");
                } else {
                    tracing::warn!("dpop replay store thread panicked");
                }
                false
            }
        }
    }
}

#[cfg(test)]
pub mod helpers {
    use crate::account::AccountManager;
    use crate::account::CreateAccountOpts;
    use crate::db::DatabaseKind;
    use camino::Utf8Path;
    use std::path::PathBuf;
    use std::sync::Once;

    static INIT_ENV: Once = Once::new();

    pub fn init_env() {
        INIT_ENV.call_once(|| {
            let defaults = [
                ("PDS_SERVICE_DID", "did:web:localho.st"),
                (
                    "PDS_JWT_KEY_K256_PRIVATE_KEY_HEX",
                    "9d5907143471e8f0e8df0f8b9512a8c5377878ee767f18fcf961055ecfc071cd",
                ),
                (
                    "PDS_REPO_SIGNING_KEY_K256_PRIVATE_KEY_HEX",
                    "71cfcf4882a6cff494c3d0affadd3858eb3a5838e7b5e15170e696a590a4fa01",
                ),
            ];
            for (key, value) in defaults {
                if std::env::var(key).is_err() {
                    // SAFETY: tests run sequentially within a process.
                    unsafe { std::env::set_var(key, value) };
                }
            }
        });
    }

    pub struct TestDb {
        pub db: sea_orm::DatabaseConnection,
        pub _dir: camino_tempfile::Utf8TempDir,
    }

    impl TestDb {
        pub async fn new() -> Self {
            init_env();
            let dir = camino_tempfile::Utf8TempDir::new().unwrap();
            let path: PathBuf = dir.path().join("account.sqlite").into();
            let db = DatabaseKind::Account
                .open(Utf8Path::from_path(&path).unwrap())
                .await
                .unwrap();
            Self { db, _dir: dir }
        }

        pub async fn create_account(&self) -> (String, String) {
            use crate::account::test_util::TEST_CID;
            use lexicon_cid::Cid;
            use std::str::FromStr;
            let did = format!(
                "did:plc:{}",
                hex::encode(rsky_crypto::utils::random_bytes(8))
            );
            let email = format!("{did}@example.com");
            let handle = format!(
                "user-{}.test",
                hex::encode(rsky_crypto::utils::random_bytes(3))
            );
            let manager = AccountManager::new(self.db.clone());
            manager
                .create_account(CreateAccountOpts {
                    did: did.clone(),
                    handle,
                    email: Some(email.clone()),
                    password: Some("password123".to_owned()),
                    invite_code: None,
                    deactivated: None,
                    repo_cid: Cid::from_str(TEST_CID).unwrap(),
                    repo_rev: "3jzfcijpj2z2a".to_owned(),
                })
                .await
                .unwrap();
            (did, email)
        }
    }
}

#[cfg(test)]
mod oauth_store_tests {
    use super::helpers::{TestDb, init_env};
    use super::*;
    use rsky_oauth::request::RequestData;
    use rsky_oauth::token::TokenData;
    use rsky_oauth::types::{AuthorizationRequestParameters, ClientAuth};

    const NOW: u64 = 1_700_000_000;

    fn parameters() -> AuthorizationRequestParameters {
        AuthorizationRequestParameters {
            client_id: "https://app.example.com/client".to_string(),
            response_type: "code".to_string(),
            redirect_uri: "https://app.example.com/cb".to_string(),
            scope: "atproto transition:generic".to_string(),
            state: Some("state-1".to_string()),
            code_challenge: "challenge".to_string(),
            code_challenge_method: "S256".to_string(),
            login_hint: None,
            prompt: Some("consent".to_string()),
            dpop_jkt: Some("jkt-1".to_string()),
        }
    }

    fn request_data(_id: &str) -> RequestData {
        RequestData {
            client_id: "https://app.example.com/client".to_string(),
            client_auth: ClientAuth::None,
            parameters: parameters(),
            expires_at: NOW + 300,
            device_id: None,
            did: None,
            code: None,
        }
    }

    fn token_data(did: &str) -> TokenData {
        TokenData {
            created_at: NOW,
            updated_at: NOW,
            expires_at: NOW + 3600,
            client_id: "https://app.example.com/client".to_string(),
            client_auth: ClientAuth::PrivateKeyJwt {
                alg: "ES256".to_string(),
                kid: "key-1".to_string(),
                jkt: "client-jkt".to_string(),
            },
            device_id: None,
            did: did.to_string(),
            parameters: parameters(),
            code: Some("cod-1".to_string()),
        }
    }

    fn device_data() -> rsky_oauth::store::DeviceData {
        rsky_oauth::store::DeviceData {
            session_id: "ses-1".to_string(),
            user_agent: Some("test-agent".to_string()),
            ip_address: "127.0.0.1".to_string(),
            last_seen_at: NOW,
        }
    }

    async fn test_store() -> (TestDb, PdsOAuthStore, String, String) {
        init_env();
        let test_db = TestDb::new().await;
        let (did, email) = test_db.create_account().await;
        let store = PdsOAuthStore::new(test_db.db.clone());
        (test_db, store, did, email)
    }

    #[tokio::test]
    async fn request_crud_roundtrip() {
        let (_test_db, store, _did, _email) = test_store().await;
        assert!(store.read_request("req-1").await.unwrap().is_none());
        store
            .create_request("req-1", &request_data("req-1"))
            .await
            .unwrap();
        let read = store.read_request("req-1").await.unwrap();
        assert_eq!(read, Some(request_data("req-1")));

        let mut updated = request_data("req-1");
        updated.did = Some("did:plc:requested".to_string());
        updated.device_id = Some("dev-1".to_string());
        updated.code = Some("cod-abc".to_string());
        updated.expires_at = NOW + 600;
        store.update_request("req-1", &updated).await.unwrap();
        assert_eq!(
            store.read_request("req-1").await.unwrap(),
            Some(updated.clone())
        );

        let (id, data) = store
            .consume_request_code("cod-abc")
            .await
            .unwrap()
            .expect("code consumed");
        assert_eq!(id, "req-1");
        assert_eq!(data, updated);
        assert!(store.read_request("req-1").await.unwrap().is_none());
        assert!(
            store
                .consume_request_code("cod-abc")
                .await
                .unwrap()
                .is_none()
        );

        store
            .create_request("req-2", &request_data("req-2"))
            .await
            .unwrap();
        store.delete_request("req-2").await.unwrap();
        assert!(store.read_request("req-2").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn token_lifecycle_and_refresh_rotation() {
        let (_test_db, store, _did, _email) = test_store().await;
        let did = "did:plc:tokenlifecycle";
        assert!(store.read_token("tok-1").await.unwrap().is_none());
        store
            .create_token("tok-1", &token_data(did), Some("ref-1"))
            .await
            .unwrap();

        let info = store.read_token("tok-1").await.unwrap().unwrap();
        assert_eq!(info.token_id, "tok-1");
        assert_eq!(info.data.did, did);
        assert_eq!(info.current_refresh_token.as_deref(), Some("ref-1"));

        let by_code = store.find_token_by_code("cod-1").await.unwrap().unwrap();
        assert_eq!(by_code.token_id, "tok-1");
        let by_refresh = store
            .find_token_by_refresh_token("ref-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(by_refresh.token_id, "tok-1");

        store
            .rotate_token("tok-1", "tok-2", "ref-2", NOW + 100, NOW + 100 + 3600)
            .await
            .unwrap();
        assert!(store.read_token("tok-1").await.unwrap().is_none());
        let rotated = store.read_token("tok-2").await.unwrap().unwrap();
        assert_eq!(rotated.data.updated_at, NOW + 100);
        assert_eq!(rotated.data.created_at, NOW);
        assert_eq!(rotated.current_refresh_token.as_deref(), Some("ref-2"));

        let replay = store
            .find_token_by_refresh_token("ref-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(replay.token_id, "tok-2");
        assert_eq!(replay.current_refresh_token.as_deref(), Some("ref-2"));

        assert!(
            store
                .create_token("tok-3", &token_data("did:plc:reused"), Some("ref-1"))
                .await
                .is_err()
        );
        assert!(
            store
                .rotate_token("tok-2", "tok-4", "ref-1", NOW + 200, NOW + 200 + 3600)
                .await
                .is_err()
        );
        assert!(
            store
                .rotate_token("tok-missing", "tok-5", "ref-5", NOW, NOW)
                .await
                .is_err()
        );

        store.delete_token("tok-2").await.unwrap();
        assert!(store.read_token("tok-2").await.unwrap().is_none());
        assert!(
            store
                .find_token_by_refresh_token("ref-1")
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .find_token_by_refresh_token("ref-2")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn device_and_device_account_lifecycle() {
        let (_test_db, store, did, _email) = test_store().await;
        assert!(store.read_device("dev-1").await.unwrap().is_none());
        assert!(store.update_device("dev-1", &device_data()).await.is_err());
        store.create_device("dev-1", &device_data()).await.unwrap();
        assert_eq!(
            store.read_device("dev-1").await.unwrap(),
            Some(device_data())
        );

        let mut updated = device_data();
        updated.last_seen_at = NOW + 60;
        updated.user_agent = None;
        store.update_device("dev-1", &updated).await.unwrap();
        assert_eq!(store.read_device("dev-1").await.unwrap(), Some(updated));

        assert!(
            store
                .get_device_account("dev-1", &did)
                .await
                .unwrap()
                .is_none()
        );
        store.upsert_device_account("dev-1", &did).await.unwrap();
        store.upsert_device_account("dev-1", &did).await.unwrap();
        let account = store
            .get_device_account("dev-1", &did)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(account.did, did);
        assert!(account.handle.is_some());
        assert_eq!(store.list_device_accounts("dev-1").await.unwrap().len(), 1);
        assert!(
            store
                .list_device_accounts("dev-2")
                .await
                .unwrap()
                .is_empty()
        );
        store.remove_device_account("dev-1", &did).await.unwrap();
        assert!(
            store
                .list_device_accounts("dev-1")
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn authenticates_known_account() {
        let (_test_db, store, did, email) = test_store().await;

        assert_eq!(store.get_account(&did).await.unwrap().unwrap().did, did);
        let auth = store
            .authenticate_account(&email, "password123")
            .await
            .unwrap()
            .expect("credentials valid");
        assert_eq!(auth.did, did);
        assert!(
            store
                .authenticate_account(&email, "wrong")
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .authenticate_account("nobody.test", "password123")
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .authenticate_account("nobody@example.com", "password123")
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .get_account("did:plc:missing")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn authorized_clients() {
        let (_test_db, store, did, _email) = test_store().await;

        assert!(
            store
                .get_authorized_client_scope(&did, "client-1")
                .await
                .unwrap()
                .is_none()
        );
        store
            .set_authorized_client(&did, "client-1", "atproto")
            .await
            .unwrap();
        assert_eq!(
            store
                .get_authorized_client_scope(&did, "client-1")
                .await
                .unwrap()
                .as_deref(),
            Some("atproto")
        );
        store
            .set_authorized_client(&did, "client-1", "atproto transition:generic")
            .await
            .unwrap();
        assert_eq!(
            store
                .get_authorized_client_scope(&did, "client-1")
                .await
                .unwrap()
                .as_deref(),
            Some("atproto transition:generic")
        );
    }
}
