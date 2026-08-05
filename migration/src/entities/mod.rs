// migration/src/entities/mod.rs
//! sea-orm entities, one module per table.
//!
//! The account database (15 tables) is a port of
//! `vendor/rsky/rsky-pds/src/account_manager/db.rs`. `repo_root` is shared
//! with the actor database.

pub mod account;
pub mod account_device;
pub mod account_pref;
pub mod actor;
pub mod app_password;
pub mod authorization_request;
pub mod authorized_client;
pub mod backlink;
pub mod blob;
pub mod device;
pub mod did_doc;
pub mod email_token;
pub mod invite_code;
pub mod invite_code_use;
pub mod lexicon;
pub mod record;
pub mod record_blob;
pub mod refresh_token;
pub mod repo_block;
pub mod repo_root;
pub mod repo_seq;
pub mod space_blob_ref;
pub mod space_def;
pub mod space_host_reg;
pub mod space_member;
pub mod space_oplog;
pub mod space_record;
pub mod space_repo;
pub mod space_repo_notify;
pub mod space_used_jti;
pub mod space_writer;
pub mod token;
pub mod used_refresh_token;
