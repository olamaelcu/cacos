use sea_orm::entity::prelude::*;
use crate::types::did::Did;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "repo_seq")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub seq: i64,
    pub did: Did,
    #[sea_orm(column_name = "eventType")]
    pub event_type: String,
    pub event: Vec<u8>,
    pub invalidated: Option<i16>,
    #[sea_orm(column_name = "sequencedAt")]
    pub sequenced_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
