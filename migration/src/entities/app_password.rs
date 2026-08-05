use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "app_password")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub did: String,
    #[sea_orm(primary_key)]
    pub name: String,
    pub password: String,
    #[sea_orm(column_name = "createdAt")]
    pub created_at: String,
    pub privileged: i16,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
