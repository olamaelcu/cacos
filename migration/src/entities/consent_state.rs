use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "consent_state")]
pub struct Model {
    #[sea_orm(primary_key, column_name = "requestId")]
    pub request_id: String,
    #[sea_orm(column_name = "state")]
    pub state: String,
    #[sea_orm(column_name = "expiresAt")]
    pub expires_at: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
