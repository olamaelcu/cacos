use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "email_token")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub purpose: String,
    #[sea_orm(primary_key)]
    pub did: String,
    pub token: String,
    #[sea_orm(column_name = "requestedAt")]
    pub requested_at: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
