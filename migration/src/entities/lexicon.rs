use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "lexicon")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub nsid: String,
    #[sea_orm(column_name = "createdAt")]
    pub created_at: OffsetDateTime,
    #[sea_orm(column_name = "updatedAt")]
    pub updated_at: OffsetDateTime,
    #[sea_orm(column_name = "lastSucceededAt")]
    pub last_succeeded_at: Option<OffsetDateTime>,
    pub uri: Option<String>,
    pub lexicon: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
