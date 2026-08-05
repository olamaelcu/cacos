use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "account_device")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub did: String,
    #[sea_orm(primary_key, column_name = "deviceId")]
    pub device_id: String,
    #[sea_orm(column_name = "createdAt")]
    pub created_at: String,
    #[sea_orm(column_name = "updatedAt")]
    pub updated_at: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
