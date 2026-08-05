use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "did_doc")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub did: String,
    pub doc: String,
    #[sea_orm(column_name = "updatedAt")]
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
