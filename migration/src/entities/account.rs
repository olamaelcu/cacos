use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "account")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub did: String,
    pub email: String,
    #[sea_orm(column_name = "recoveryKey")]
    pub recovery_key: Option<String>,
    pub password: String,
    #[sea_orm(column_name = "createdAt")]
    pub created_at: String,
    #[sea_orm(column_name = "invitesDisabled")]
    pub invites_disabled: i16,
    #[sea_orm(column_name = "emailConfirmedAt")]
    pub email_confirmed_at: Option<String>,
    #[sea_orm(column_name = "inviteNote")]
    pub invite_note: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
