use sea_orm::entity::prelude::*;
use crate::types::db_id::DbId;
use crate::types::did::Did;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "account_device")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub did: Did,
    #[sea_orm(primary_key, column_name = "deviceId")]
    pub device_id: DbId,
    #[sea_orm(column_name = "createdAt")]
    pub created_at: OffsetDateTime,
    #[sea_orm(column_name = "updatedAt")]
    pub updated_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
