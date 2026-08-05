use crate::types::did::Did;
use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "repo_root")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub did: Did,
    pub cid: String,
    pub rev: String,
    #[sea_orm(column_name = "indexedAt")]
    pub indexed_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
