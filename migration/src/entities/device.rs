use crate::types::db_id::DbId;
use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "device")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: DbId,
    #[sea_orm(column_name = "sessionId")]
    pub session_id: String,
    #[sea_orm(column_name = "userAgent")]
    pub user_agent: Option<String>,
    #[sea_orm(column_name = "ipAddress")]
    pub ip_address: String,
    #[sea_orm(column_name = "lastSeenAt")]
    pub last_seen_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
