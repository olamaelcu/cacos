use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "space_def")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub space_uri: String,
    pub space_type: String,
    pub skey: String,
    pub policy: String,
    pub app_access: String,
    pub allowed_clients: Option<String>,
    pub managing_app: Option<String>,
    pub deleted: i16,
    pub created_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
