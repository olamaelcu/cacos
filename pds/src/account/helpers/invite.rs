//! Invite code helper: per-account invite code lifecycle with uses tracking
//! and disabled-account enforcement. `DisableInviteCodesOpts` is defined here
//! (next to its consumer) and re-exported from `crate::account` so the public
//! path matches the reference design.

use crate::account::helpers::sql;
use anyhow::{Result, bail};
use rsky_lexicon::com::atproto::server::AccountCodes;
use rsky_lexicon::com::atproto::server::{
    InviteCode as LexiconInviteCode, InviteCodeUse as LexiconInviteCodeUse,
};
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DatabaseTransaction, QueryResult, TransactionTrait, Value,
};
use std::collections::BTreeMap;
use std::mem;

pub struct DisableInviteCodesOpts {
    pub codes: Vec<String>,
    pub accounts: Vec<String>,
}

pub type CodeUse = LexiconInviteCodeUse;
pub type CodeDetail = LexiconInviteCode;

#[derive(Clone, Debug, PartialEq, Default)]
pub struct InviteCode {
    pub code: String,
    pub available_uses: i32,
    pub disabled: i16,
    pub for_account: String,
    pub created_by: String,
    pub created_at: String,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct InviteCodeUse {
    pub code: String,
    pub used_by: String,
    pub used_at: String,
}

pub(crate) fn invite_code_from_row(row: &QueryResult) -> Result<InviteCode, sea_orm::DbErr> {
    Ok(InviteCode {
        code: row.try_get_by_index(0)?,
        available_uses: row.try_get_by_index(1)?,
        disabled: row.try_get_by_index(2)?,
        for_account: row.try_get_by_index(3)?,
        created_by: row.try_get_by_index(4)?,
        created_at: row.try_get_by_index(5)?,
    })
}

pub(crate) const SELECT_INVITE_CODE: &str = "\
    SELECT invite_code.code, invite_code.\"availableUses\", invite_code.disabled, \
    invite_code.\"forAccount\", invite_code.\"createdBy\", invite_code.\"createdAt\" \
    FROM invite_code";

fn placeholders(len: usize) -> String {
    vec!["?"; len].join(", ")
}

async fn insert_invite_codes(
    tx: &DatabaseTransaction,
    rows: &[InviteCode],
) -> Result<(), sea_orm::DbErr> {
    for row in rows {
        tx.execute_raw(sql(
            "INSERT INTO invite_code \
             (code, \"availableUses\", disabled, \"forAccount\", \"createdBy\", \"createdAt\") \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            vec![
                Value::from(row.code.clone()),
                Value::from(row.available_uses),
                Value::from(row.disabled),
                Value::from(row.for_account.clone()),
                Value::from(row.created_by.clone()),
                Value::from(row.created_at.clone()),
            ],
        ))
        .await?;
    }
    Ok(())
}

pub async fn ensure_invite_is_available(
    invite_code: String,
    db: &DatabaseConnection,
) -> Result<()> {
    let row: Option<QueryResult> = db
        .query_one_raw(sql(
            &format!(
                "{SELECT_INVITE_CODE} \
                 LEFT JOIN actor ON invite_code.\"forAccount\" = actor.did \
                     AND actor.\"takedownRef\" IS NULL \
                 WHERE code = ?1"
            ),
            vec![Value::from(invite_code.clone())],
        ))
        .await?;

    let Some(invite) = row.map(|row| invite_code_from_row(&row)).transpose()? else {
        bail!(
            "InvalidInviteCode: None or disabled. Provided invite code not available `{invite_code:?}`"
        )
    };
    if invite.disabled > 0 {
        bail!(
            "InvalidInviteCode: None or disabled. Provided invite code not available `{invite_code:?}`"
        )
    }

    let uses: i64 = db
        .query_one_raw(sql(
            "SELECT count(*) FROM invite_code_use WHERE code = ?1",
            vec![Value::from(invite_code.clone())],
        ))
        .await?
        .expect("count(*) always returns a row")
        .try_get_by_index(0)?;

    if invite.available_uses as i64 <= uses {
        bail!(
            "InvalidInviteCode: Not enough uses. Provided invite code not available `{invite_code:?}`"
        )
    }
    Ok(())
}

pub async fn record_invite_use(
    did: String,
    invite_code: Option<String>,
    now: String,
    db: &DatabaseConnection,
) -> Result<()> {
    if let Some(invite_code) = invite_code {
        db.execute_raw(sql(
            "INSERT INTO invite_code_use (code, \"usedBy\", \"usedAt\") VALUES (?1, ?2, ?3)",
            vec![Value::from(invite_code), Value::from(did), Value::from(now)],
        ))
        .await?;
    }
    Ok(())
}

pub async fn create_invite_codes(
    to_create: Vec<AccountCodes>,
    use_count: i32,
    db: &DatabaseConnection,
) -> Result<()> {
    let created_at = rsky_common::now();

    let tx = db.begin().await?;
    let rows: Vec<InviteCode> = to_create
        .iter()
        .flat_map(|account| {
            let for_account = &account.account;
            account
                .codes
                .iter()
                .map(|code| InviteCode {
                    code: code.clone(),
                    available_uses: use_count,
                    disabled: 0,
                    for_account: for_account.clone(),
                    created_by: "admin".to_owned(),
                    created_at: created_at.clone(),
                })
                .collect::<Vec<InviteCode>>()
        })
        .collect();
    insert_invite_codes(&tx, &rows).await?;
    tx.commit().await?;
    Ok(())
}

pub async fn create_account_invite_codes(
    for_account: &str,
    codes: Vec<String>,
    expected_total: usize,
    disabled: bool,
    db: &DatabaseConnection,
) -> Result<Vec<CodeDetail>> {
    let for_account = for_account.to_owned();
    let tx = db.begin().await?;
    let now = rsky_common::now();

    let rows: Vec<InviteCode> = codes
        .iter()
        .map(|code| InviteCode {
            code: code.clone(),
            available_uses: 1,
            disabled: if disabled { 1 } else { 0 },
            for_account: for_account.clone(),
            created_by: for_account.clone(),
            created_at: now.clone(),
        })
        .collect();

    insert_invite_codes(&tx, &rows).await?;

    // don't count admin-gifted codes against the user
    let final_routine_invite_codes: i64 = tx
        .query_one_raw(sql(
            "SELECT count(*) FROM invite_code \
             WHERE \"forAccount\" = ?1 AND \"createdBy\" != 'admin'",
            vec![Value::from(for_account.clone())],
        ))
        .await?
        .expect("count(*) always returns a row")
        .try_get_by_index(0)?;

    if final_routine_invite_codes as usize > expected_total {
        bail!("DuplicateCreate: attempted to create additional codes in another request")
    }
    tx.commit().await?;

    Ok(rows
        .into_iter()
        .map(|row| CodeDetail {
            code: row.code,
            available: 1,
            disabled: row.disabled == 1,
            for_account: row.for_account,
            created_by: row.created_by,
            created_at: row.created_at,
            uses: Vec::new(),
        })
        .collect())
}

pub async fn get_account_invite_codes(
    did: &str,
    db: &DatabaseConnection,
) -> Result<Vec<CodeDetail>> {
    let rows: Vec<QueryResult> = db
        .query_all_raw(sql(
            &format!("{SELECT_INVITE_CODE} WHERE \"forAccount\" = ?1"),
            vec![Value::from(did.to_owned())],
        ))
        .await?;
    let res: Vec<InviteCode> = rows
        .iter()
        .map(|row| invite_code_from_row(row))
        .collect::<Result<Vec<InviteCode>, sea_orm::DbErr>>()?;

    let codes: Vec<String> = res.iter().map(|row| row.code.clone()).collect();
    let mut uses = get_invite_codes_uses_v2(codes, db).await?;
    Ok(res
        .into_iter()
        .map(|row| CodeDetail {
            code: row.code.clone(),
            available: row.available_uses,
            disabled: row.disabled == 1,
            for_account: row.for_account,
            created_by: row.created_by,
            created_at: row.created_at,
            uses: mem::take(uses.get_mut(&row.code).unwrap_or(&mut Vec::new())),
        })
        .collect::<Vec<CodeDetail>>())
}

pub async fn get_invite_codes_uses_v2(
    codes: Vec<String>,
    db: &DatabaseConnection,
) -> Result<BTreeMap<String, Vec<CodeUse>>> {
    let mut uses: BTreeMap<String, Vec<CodeUse>> = BTreeMap::new();
    if !codes.is_empty() {
        let query = format!(
            "SELECT code, \"usedBy\", \"usedAt\" FROM invite_code_use \
             WHERE code IN ({}) ORDER BY \"usedAt\" DESC",
            placeholders(codes.len())
        );
        let values: Vec<Value> = codes.iter().map(|c| Value::from(c.clone())).collect();
        let rows: Vec<QueryResult> = db.query_all_raw(sql(&query, values)).await?;
        for row in rows {
            let invite_code_use = InviteCodeUse {
                code: row.try_get_by_index(0)?,
                used_by: row.try_get_by_index(1)?,
                used_at: row.try_get_by_index(2)?,
            };
            let InviteCodeUse {
                code,
                used_by,
                used_at,
            } = invite_code_use;
            match uses.get_mut(&code) {
                None => {
                    uses.insert(code, vec![CodeUse { used_by, used_at }]);
                }
                Some(matched_uses) => matched_uses.push(CodeUse { used_by, used_at }),
            };
        }
    }
    Ok(uses)
}

pub async fn get_invited_by_for_accounts(
    dids: Vec<String>,
    db: &DatabaseConnection,
) -> Result<BTreeMap<String, CodeDetail>> {
    if dids.is_empty() {
        return Ok(BTreeMap::new());
    }

    let query = format!(
        "{SELECT_INVITE_CODE} WHERE invite_code.code IN (\
         SELECT DISTINCT code FROM invite_code_use WHERE \"usedBy\" IN ({}))",
        placeholders(dids.len())
    );
    let values: Vec<Value> = dids.iter().map(|d| Value::from(d.clone())).collect();
    let rows: Vec<QueryResult> = db.query_all_raw(sql(&query, values)).await?;
    let res: Vec<InviteCode> = rows
        .iter()
        .map(|row| invite_code_from_row(row))
        .collect::<Result<Vec<InviteCode>, sea_orm::DbErr>>()?;

    let codes: Vec<String> = res.iter().map(|row| row.code.clone()).collect();
    let mut uses = get_invite_codes_uses_v2(codes, db).await?;

    let code_details = res
        .into_iter()
        .map(|row| CodeDetail {
            code: row.code.clone(),
            available: row.available_uses,
            disabled: row.disabled == 1,
            for_account: row.for_account,
            created_by: row.created_by,
            created_at: row.created_at,
            uses: mem::take(uses.get_mut(&row.code).unwrap_or(&mut Vec::new())),
        })
        .collect::<Vec<CodeDetail>>();

    Ok(code_details.iter().fold(
        BTreeMap::new(),
        |mut acc: BTreeMap<String, CodeDetail>, cur| {
            for code_use in &cur.uses {
                acc.insert(code_use.used_by.clone(), cur.clone());
            }
            acc
        },
    ))
}

pub async fn set_account_invites_disabled(
    did: &str,
    disabled: bool,
    db: &DatabaseConnection,
) -> Result<()> {
    let disabled: i16 = if disabled { 1 } else { 0 };
    db.execute_raw(sql(
        "UPDATE account SET \"invitesDisabled\" = ?1 WHERE did = ?2",
        vec![Value::from(disabled), Value::from(did.to_owned())],
    ))
    .await?;
    Ok(())
}

pub async fn disable_invite_codes(
    opts: DisableInviteCodesOpts,
    db: &DatabaseConnection,
) -> Result<()> {
    let DisableInviteCodesOpts { codes, accounts } = opts;
    if !codes.is_empty() {
        let query = format!(
            "UPDATE invite_code SET disabled = 1 WHERE code IN ({})",
            placeholders(codes.len())
        );
        let values: Vec<Value> = codes.iter().map(|c| Value::from(c.clone())).collect();
        db.execute_raw(sql(&query, values)).await?;
    }
    if !accounts.is_empty() {
        let query = format!(
            "UPDATE invite_code SET disabled = 1 WHERE \"forAccount\" IN ({})",
            placeholders(accounts.len())
        );
        let values: Vec<Value> = accounts.iter().map(|a| Value::from(a.clone())).collect();
        db.execute_raw(sql(&query, values)).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::helpers::account::{ActorAccount, register_account, register_actor};
    use crate::account::test_util::test_db;
    use rsky_lexicon::com::atproto::server::AccountCodes;

    async fn register_full(db: &sea_orm::DatabaseConnection, did: &str, handle: &str) {
        register_actor(did.to_owned(), handle.to_owned(), None, db)
            .await
            .unwrap();
        register_account(
            did.to_owned(),
            format!("{handle}@example.com"),
            "phc-hash".to_owned(),
            db,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn create_invite_codes_and_list() {
        let (_dir, db) = test_db().await;
        register_full(&db, "did:plc:inviter", "inviter.test").await;
        create_invite_codes(
            vec![AccountCodes {
                account: "did:plc:inviter".to_owned(),
                codes: vec!["admin-code-1".to_owned(), "admin-code-2".to_owned()],
            }],
            1,
            &db,
        )
        .await
        .unwrap();
        let codes = get_account_invite_codes("did:plc:inviter", &db)
            .await
            .unwrap();
        assert_eq!(codes.len(), 2);
        assert!(
            codes
                .iter()
                .all(|c| c.created_by == "admin" && !c.disabled && c.uses.is_empty())
        );
    }

    #[tokio::test]
    async fn ensure_available_accepts_fresh_code() {
        let (_dir, db) = test_db().await;
        create_invite_codes(
            vec![AccountCodes {
                account: "did:plc:inviter".to_owned(),
                codes: vec!["fresh-code".to_owned()],
            }],
            1,
            &db,
        )
        .await
        .unwrap();
        ensure_invite_is_available("fresh-code".to_owned(), &db)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn exhausted_and_unknown_and_disabled_codes_rejected() {
        let (_dir, db) = test_db().await;
        create_invite_codes(
            vec![AccountCodes {
                account: "did:plc:inviter".to_owned(),
                codes: vec!["one-use".to_owned(), "to-disable".to_owned()],
            }],
            1,
            &db,
        )
        .await
        .unwrap();

        // unknown code
        let err = ensure_invite_is_available("no-such-code".to_owned(), &db)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("None or disabled"));

        // exhaust the one-use code
        record_invite_use(
            "did:plc:used".to_owned(),
            Some("one-use".to_owned()),
            rsky_common::now(),
            &db,
        )
        .await
        .unwrap();
        let err = ensure_invite_is_available("one-use".to_owned(), &db)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Not enough uses"));

        // disabled code
        disable_invite_codes(
            DisableInviteCodesOpts {
                codes: vec!["to-disable".to_owned()],
                accounts: vec![],
            },
            &db,
        )
        .await
        .unwrap();
        let err = ensure_invite_is_available("to-disable".to_owned(), &db)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("None or disabled"));
    }

    #[tokio::test]
    async fn create_account_invite_codes_guards_expected_total() {
        let (_dir, db) = test_db().await;
        register_full(&db, "did:plc:inviter", "inviter.test").await;
        let created = create_account_invite_codes(
            "did:plc:inviter",
            vec!["self-code-1".to_owned()],
            1,
            false,
            &db,
        )
        .await
        .unwrap();
        assert_eq!(created.len(), 1);
        assert!(!created[0].disabled);
        assert_eq!(created[0].created_by, "did:plc:inviter");

        // admin-gifted codes don't count against the expected total
        create_invite_codes(
            vec![AccountCodes {
                account: "did:plc:inviter".to_owned(),
                codes: vec!["admin-gift".to_owned()],
            }],
            1,
            &db,
        )
        .await
        .unwrap();

        // Amendment: the upstream test expects the 2nd create_account_invite_codes
        // call to fail with DuplicateCreate (the implementation checks
        // count > expected_total AFTER insert). The plan's test expected 2 self-codes
        // to succeed, which is inconsistent with the implementation.
        let err = create_account_invite_codes(
            "did:plc:inviter",
            vec!["self-code-2".to_owned()],
            1,
            true,
            &db,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("DuplicateCreate"));

        let codes = get_account_invite_codes("did:plc:inviter", &db)
            .await
            .unwrap();
        // admin-gift + self-code-1 (the failed self-code-2 rolled back)
        assert_eq!(codes.len(), 2);
    }

    #[tokio::test]
    async fn record_use_and_query_uses_map() {
        let (_dir, db) = test_db().await;
        assert!(
            get_invite_codes_uses_v2(vec![], &db)
                .await
                .unwrap()
                .is_empty()
        );
        create_invite_codes(
            vec![AccountCodes {
                account: "did:plc:inviter".to_owned(),
                codes: vec!["code-a".to_owned()],
            }],
            2,
            &db,
        )
        .await
        .unwrap();
        record_invite_use(
            "did:plc:u1".to_owned(),
            Some("code-a".to_owned()),
            "2023-01-01T00:00:01.000Z".to_owned(),
            &db,
        )
        .await
        .unwrap();
        record_invite_use(
            "did:plc:u2".to_owned(),
            Some("code-a".to_owned()),
            "2023-01-01T00:00:02.000Z".to_owned(),
            &db,
        )
        .await
        .unwrap();
        let uses = get_invite_codes_uses_v2(vec!["code-a".to_owned()], &db)
            .await
            .unwrap();
        let code_uses = uses.get("code-a").unwrap();
        assert_eq!(code_uses.len(), 2);
        // newest first
        assert_eq!(code_uses[0].used_by, "did:plc:u2");
        assert_eq!(code_uses[1].used_by, "did:plc:u1");
    }

    #[tokio::test]
    async fn get_invited_by_for_accounts() {
        let (_dir, db) = test_db().await;
        create_invite_codes(
            vec![AccountCodes {
                account: "did:plc:inviter".to_owned(),
                codes: vec!["code-invite".to_owned()],
            }],
            1,
            &db,
        )
        .await
        .unwrap();
        record_invite_use(
            "did:plc:invited".to_owned(),
            Some("code-invite".to_owned()),
            rsky_common::now(),
            &db,
        )
        .await
        .unwrap();
        let map = super::get_invited_by_for_accounts(vec!["did:plc:invited".to_owned()], &db)
            .await
            .unwrap();
        assert_eq!(map.get("did:plc:invited").unwrap().code, "code-invite");
        assert!(
            super::get_invited_by_for_accounts(vec![], &db)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn disable_invite_codes_by_code_and_account() {
        let (_dir, db) = test_db().await;
        create_invite_codes(
            vec![AccountCodes {
                account: "did:plc:inviter".to_owned(),
                codes: vec!["code-1".to_owned(), "code-2".to_owned()],
            }],
            1,
            &db,
        )
        .await
        .unwrap();
        disable_invite_codes(
            DisableInviteCodesOpts {
                codes: vec!["code-1".to_owned()],
                accounts: vec![],
            },
            &db,
        )
        .await
        .unwrap();
        disable_invite_codes(
            DisableInviteCodesOpts {
                codes: vec![],
                accounts: vec!["did:plc:inviter".to_owned()],
            },
            &db,
        )
        .await
        .unwrap();
        let codes = get_account_invite_codes("did:plc:inviter", &db)
            .await
            .unwrap();
        assert!(codes.iter().all(|c| c.disabled));
    }

    #[tokio::test]
    async fn set_account_invites_disabled_updates_account() {
        let (_dir, db) = test_db().await;
        register_full(&db, "did:plc:inviter", "inviter.test").await;
        set_account_invites_disabled("did:plc:inviter", true, &db)
            .await
            .unwrap();
        let got: ActorAccount =
            crate::account::helpers::account::get_account("did:plc:inviter", None, &db)
                .await
                .unwrap()
                .unwrap();
        assert_eq!(got.invites_disabled, Some(1));
        set_account_invites_disabled("did:plc:inviter", false, &db)
            .await
            .unwrap();
        let got: ActorAccount =
            crate::account::helpers::account::get_account("did:plc:inviter", None, &db)
                .await
                .unwrap()
                .unwrap();
        assert_eq!(got.invites_disabled, Some(0));
    }
}
