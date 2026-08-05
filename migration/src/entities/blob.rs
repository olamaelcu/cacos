use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "blob")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub cid: String,
    #[sea_orm(column_name = "mimeType")]
    pub mime_type: String,
    pub size: i64,
    #[sea_orm(column_name = "tempKey")]
    pub temp_key: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    #[sea_orm(column_name = "createdAt")]
    pub created_at: String,
    #[sea_orm(column_name = "takedownRef")]
    pub takedown_ref: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
