use crate::types::db_id::DbId;
use crate::types::did::Did;
use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "token")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: DbId,
    pub did: Did,
    #[sea_orm(column_name = "tokenId")]
    pub token_id: String,
    #[sea_orm(column_name = "createdAt")]
    pub created_at: OffsetDateTime,
    #[sea_orm(column_name = "updatedAt")]
    pub updated_at: OffsetDateTime,
    #[sea_orm(column_name = "expiresAt")]
    pub expires_at: OffsetDateTime,
    #[sea_orm(column_name = "clientId")]
    pub client_id: String,
    #[sea_orm(column_name = "clientAuth")]
    pub client_auth: String,
    #[sea_orm(column_name = "deviceId")]
    pub device_id: Option<DbId>,
    #[sea_orm(column_name = "externalDeviceId")]
    pub external_device_id: Option<String>,
    pub parameters: String,
    pub details: Option<String>,
    pub code: Option<String>,
    #[sea_orm(column_name = "currentRefreshToken")]
    pub current_refresh_token: Option<String>,
    pub scope: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
