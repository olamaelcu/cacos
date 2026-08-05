use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "space_repo")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub space_uri: String,
    pub authority: String,
    pub space_type: String,
    pub skey: String,
    pub rev: String,
    pub lthash_state: Vec<u8>,
    pub oplog_floor_rev: Option<String>,
    pub deleted: i16,
    pub created_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
