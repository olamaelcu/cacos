use crate::types::db_id::DbId;
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "account_pref")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: DbId,
    pub name: String,
    #[sea_orm(column_name = "valueJson")]
    pub value_json: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
