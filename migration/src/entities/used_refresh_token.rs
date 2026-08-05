use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "used_refresh_token")]
pub struct Model {
    #[sea_orm(primary_key, column_name = "refreshToken")]
    pub refresh_token: String,
    #[sea_orm(column_name = "tokenId")]
    pub token_id: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
