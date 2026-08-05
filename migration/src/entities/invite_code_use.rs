use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "invite_code_use")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub code: String,
    #[sea_orm(primary_key, column_name = "usedBy")]
    pub used_by: String,
    #[sea_orm(column_name = "usedAt")]
    pub used_at: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
