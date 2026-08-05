use crate::types::did::Did;
use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "authorized_client")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub did: Did,
    #[sea_orm(primary_key, column_name = "clientId")]
    pub client_id: String,
    #[sea_orm(column_name = "createdAt")]
    pub created_at: OffsetDateTime,
    #[sea_orm(column_name = "updatedAt")]
    pub updated_at: OffsetDateTime,
    pub data: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
