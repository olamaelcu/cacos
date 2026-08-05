use crate::types::db_id::DbId;
use crate::types::did::Did;
use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "authorization_request")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: DbId,
    #[sea_orm(column_name = "requestId")]
    pub request_id: String,
    pub did: Option<Did>,
    /// Surrogate FK to the device row.
    #[sea_orm(column_name = "deviceId")]
    pub device_id: Option<DbId>,
    /// Opaque external device id supplied by rsky-oauth / the RemoteClient.
    /// Preserves the trait-level `RequestData::device_id: Option<String>`
    /// across read/write round-trips when the row only stores the DbId FK.
    #[sea_orm(column_name = "externalDeviceId")]
    pub external_device_id: Option<String>,
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
