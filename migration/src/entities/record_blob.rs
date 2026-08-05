use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "record_blob")]
pub struct Model {
    #[sea_orm(primary_key, column_name = "blobCid")]
    pub blob_cid: String,
    #[sea_orm(primary_key, column_name = "recordUri")]
    pub record_uri: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
