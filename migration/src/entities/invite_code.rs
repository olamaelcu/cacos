use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "invite_code")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub code: String,
    #[sea_orm(column_name = "availableUses")]
    pub available_uses: i32,
    pub disabled: i16,
    #[sea_orm(column_name = "forAccount")]
    pub for_account: String,
    #[sea_orm(column_name = "createdBy")]
    pub created_by: String,
    #[sea_orm(column_name = "createdAt")]
    pub created_at: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
