//! Logging no-op mailer.
//!
//! Free-function surface mirrors the git-pinned `olamaelcu/rsky` fork at rev
//! `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's
//! `src/mailer/mod.rs`) so handler ports stay 1:1; cacos emits a `tracing`
//! event instead of dispatching through mailgun.

use anyhow::Result;
use std::collections::HashMap;

pub struct MailOpts {
    pub to: String,
    pub subject: String,
    pub template: String,
    pub template_vars: HashMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct IdentifierAndTokenParams {
    pub identifier: String,
    pub token: String,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TokenParam {
    pub token: String,
}

pub async fn send_template(opts: MailOpts) -> Result<()> {
    // No SMTP provider in cacos yet; surface through tracing so flows are
    // observable and tests can assert tokens were minted via account_manager.
    tracing::info!(
        to = %opts.to,
        subject = %opts.subject,
        template = %opts.template,
        "mailer: template mail (not sent)"
    );
    Ok(())
}

pub async fn send_reset_password(to: String, params: IdentifierAndTokenParams) -> Result<()> {
    let mut template_vars = HashMap::new();
    template_vars.insert("identifier".to_string(), params.identifier);
    template_vars.insert("token".to_string(), params.token);
    send_template(MailOpts {
        to,
        subject: "Password Reset Requested".to_string(),
        template: "reset password".to_string(),
        template_vars,
    })
    .await
}

pub async fn send_account_delete(to: String, params: TokenParam) -> Result<()> {
    let mut template_vars = HashMap::new();
    template_vars.insert("token".to_string(), params.token);
    send_template(MailOpts {
        to,
        subject: "Account Deletion Requested".to_string(),
        template: "delete account".to_string(),
        template_vars,
    })
    .await
}

pub async fn send_confirm_email(to: String, params: TokenParam) -> Result<()> {
    let mut template_vars = HashMap::new();
    template_vars.insert("token".to_string(), params.token);
    send_template(MailOpts {
        to,
        subject: "Email Confirmation".to_string(),
        template: "confirm email".to_string(),
        template_vars,
    })
    .await
}

pub async fn send_update_email(to: String, params: TokenParam) -> Result<()> {
    let mut template_vars = HashMap::new();
    template_vars.insert("token".to_string(), params.token);
    send_template(MailOpts {
        to,
        subject: "Email Update Requested".to_string(),
        template: "email update".to_string(),
        template_vars,
    })
    .await
}

pub async fn send_plc_operation(to: String, params: TokenParam) -> Result<()> {
    let mut template_vars = HashMap::new();
    template_vars.insert("token".to_string(), params.token);
    send_template(MailOpts {
        to,
        subject: "PLC Update Operation Requested".to_string(),
        template: "plc operation".to_string(),
        template_vars,
    })
    .await
}

pub mod moderation {
    #[allow(unused_imports)] // mirrors rsky reference; kept verbatim for port parity
    use super::MailOpts;

    pub struct HtmlMailOpts {
        pub to: String,
        pub subject: String,
        pub html: String,
    }

    pub struct ModerationMailer;

    impl ModerationMailer {
        pub async fn send_html(opts: HtmlMailOpts) -> anyhow::Result<()> {
            tracing::info!(
                to = %opts.to,
                subject = %opts.subject,
                "mailer: moderation html mail (not sent)"
            );
            Ok(())
        }
    }
}