use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "actor")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub did: String,
    pub handle: Option<String>,
    #[sea_orm(column_name = "createdAt")]
    pub created_at: String,
    #[sea_orm(column_name = "takedownRef")]
    pub takedown_ref: Option<String>,
    #[sea_orm(column_name = "deactivatedAt")]
    pub deactivated_at: Option<String>,
    #[sea_orm(column_name = "deleteAfter")]
    pub delete_after: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
