use sea_orm::entity::prelude::*;
use crate::types::did::Did;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "email_token")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub purpose: String,
    #[sea_orm(primary_key)]
    pub did: Did,
    pub token: String,
    #[sea_orm(column_name = "requestedAt")]
    pub requested_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
