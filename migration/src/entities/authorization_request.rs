use sea_orm::entity::prelude::*;
use crate::types::db_id::DbId;
use crate::types::did::Did;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "authorization_request")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: DbId,
    pub did: Option<Did>,
    #[sea_orm(column_name = "deviceId")]
    pub device_id: Option<DbId>,
    #[sea_orm(column_name = "clientId")]
    pub client_id: String,
    #[sea_orm(column_name = "clientAuth")]
    pub client_auth: String,
    pub parameters: String,
    #[sea_orm(column_name = "expiresAt")]
    pub expires_at: OffsetDateTime,
    pub code: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
