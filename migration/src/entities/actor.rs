use sea_orm::entity::prelude::*;
use crate::types::did::Did;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "actor")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub did: Did,
    pub handle: Option<String>,
    #[sea_orm(column_name = "createdAt")]
    pub created_at: OffsetDateTime,
    #[sea_orm(column_name = "takedownRef")]
    pub takedown_ref: Option<String>,
    #[sea_orm(column_name = "deactivatedAt")]
    pub deactivated_at: Option<OffsetDateTime>,
    #[sea_orm(column_name = "deleteAfter")]
    pub delete_after: Option<OffsetDateTime>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
