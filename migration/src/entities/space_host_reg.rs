use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "space_host_reg")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub space_uri: String,
    #[sea_orm(primary_key)]
    pub endpoint: String,
    pub expires_at: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
