use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "repo_seq")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub seq: i64,
    pub did: String,
    #[sea_orm(column_name = "eventType")]
    pub event_type: String,
    pub event: Vec<u8>,
    pub invalidated: Option<i16>,
    #[sea_orm(column_name = "sequencedAt")]
    pub sequenced_at: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
