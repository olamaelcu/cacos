use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "backlink")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub uri: String,
    #[sea_orm(primary_key)]
    pub path: String,
    #[sea_orm(column_name = "linkTo")]
    pub link_to: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
