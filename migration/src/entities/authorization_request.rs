use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "authorization_request")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: String,
    pub did: Option<String>,
    #[sea_orm(column_name = "deviceId")]
    pub device_id: Option<String>,
    #[sea_orm(column_name = "clientId")]
    pub client_id: String,
    #[sea_orm(column_name = "clientAuth")]
    pub client_auth: String,
    pub parameters: String,
    #[sea_orm(column_name = "expiresAt")]
    pub expires_at: String,
    pub code: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
