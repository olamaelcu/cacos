use sea_orm::entity::prelude::*;
use crate::types::did::Did;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "space_member")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub space_uri: String,
    #[sea_orm(primary_key)]
    pub did: Did,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
