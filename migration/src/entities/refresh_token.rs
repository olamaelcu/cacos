use sea_orm::entity::prelude::*;
use crate::types::did::Did;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "refresh_token")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: String,
    pub did: Did,
    #[sea_orm(column_name = "expiresAt")]
    pub expires_at: OffsetDateTime,
    #[sea_orm(column_name = "nextId")]
    pub next_id: Option<String>,
    #[sea_orm(column_name = "appPasswordName")]
    pub app_password_name: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
