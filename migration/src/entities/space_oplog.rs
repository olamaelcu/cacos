use sea_orm::entity::prelude::*;
use crate::types::db_id::DbId;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "space_oplog")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: DbId,
    pub space_uri: String,
    pub rev: String,
    pub collection: String,
    pub rkey: String,
    pub cid: Option<String>,
    pub prev: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
