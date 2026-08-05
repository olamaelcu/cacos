use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "record")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub uri: String,
    pub cid: String,
    pub collection: String,
    pub rkey: String,
    #[sea_orm(column_name = "repoRev")]
    pub repo_rev: String,
    #[sea_orm(column_name = "indexedAt")]
    pub indexed_at: String,
    #[sea_orm(column_name = "takedownRef")]
    pub takedown_ref: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
