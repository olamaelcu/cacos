use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "repo_block")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub cid: String,
    #[sea_orm(column_name = "repoRev")]
    pub repo_rev: String,
    pub size: i64,
    pub content: Vec<u8>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
