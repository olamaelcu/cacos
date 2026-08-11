use std::time::Duration;
use tracing_unwrap::ResultExt;

#[tokio::main]
async fn main() {
    if let Err(e) = cacos_pds::account::helpers::init_required_keys::init_required_keys() {
        eprintln!("fatal: {e}");
        std::process::exit(1);
    }

    cacos_pds::observability::metrics::init_metrics();
    let downcaster = cacos_pds::observability::tracing::init_tracing("info");
    let _timing = cacos_pds::observability::timing::TimingReporter::start(
        Duration::from_secs(10),
        downcaster,
    );

    // Subcommand dispatch. Without args, fall through to the PDS server
    // start. `cacos-pds migrate rotation-keys` backfills per-DID
    // rotation keys for legacy actors; `cacos-pds migrate
    // plc-rotation-keys` rewrites each DID document to drop the shared
    // server rotation key.
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 2 && args[1] == "migrate" {
        run_migrate_subcommand(&args[2..]).await;
        return;
    }

    // Build the shared OpenDAL operator (S3 if S3_ENDPOINT is set, else
    // disk) once for the process. Per-DID handles come from this via
    // `blobstore_for_did(did)` at the call site.
    cacos_pds::blobstore::init_operator().expect_or_log("blobstore init failed");

    let app = cacos_pds::xrpc::build_app().await;
    let listener = poem::listener::TcpListener::bind("127.0.0.1:8080");
    tracing::info!("pds listening on http://127.0.0.1:8080");
    poem::Server::new(listener)
        .run(app)
        .await
        .expect_or_log("poem server failed");
}

async fn run_migrate_subcommand(args: &[String]) {
    let sub = args.first().map(String::as_str).unwrap_or("");
    match sub {
        "rotation-keys" => {
            if let Err(err) = migrate_rotation_keys().await {
                eprintln!("migrate rotation-keys failed: {err:?}");
                std::process::exit(1);
            }
        }
        "plc-rotation-keys" => {
            let dry_run = args.iter().any(|a| a == "--dry-run");
            if let Err(err) = migrate_plc_rotation_keys(dry_run).await {
                eprintln!("migrate plc-rotation-keys failed: {err:?}");
                std::process::exit(1);
            }
        }
        _ => {
            eprintln!("usage: cacos-pds migrate rotation-keys|plc-rotation-keys [--dry-run]");
            std::process::exit(2);
        }
    }
}

/// Walks the actor store directory. For every DID with a `store.sqlite`
/// but no `rotation_key` file, writes the global `PDS_PLC_ROTATION_KEYPAIR`
/// secret bytes as the per-DID rotation key. Idempotent: DIDs that
/// already have a rotation key are skipped.
///
/// The shared server-wide rotation key controls every DID document until
/// each actor has been migrated; this command is the seam between the
/// old and new worlds.
async fn migrate_rotation_keys() -> anyhow::Result<()> {
    use cacos_pds::xrpc::com::atproto::server::PDS_PLC_ROTATION_KEYPAIR;

    let cfg = cacos_pds::config::env_to_cfg();
    let state = cacos_pds::xrpc::types::SharedStateFromEnv::from_env(&cfg).await;

    // Touching the static forces the env var to be parsed up-front; if
    // `PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX` is missing, the
    // LazyLock panics here instead of mid-walk.
    let secret_bytes = PDS_PLC_ROTATION_KEYPAIR.secret_bytes().to_vec();

    let actor_dir = state.actor_store.directory.clone();
    let mut scanned = 0usize;
    let mut already = 0usize;
    let mut seeded = 0usize;
    let mut errors = 0usize;

    let prefix_dirs = match tokio::fs::read_dir(&actor_dir).await {
        Ok(rd) => rd,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            println!(
                "actor store directory {} does not exist; nothing to migrate",
                actor_dir.display()
            );
            return Ok(());
        }
        Err(err) => {
            return Err(anyhow::anyhow!(
                "failed to read actor store directory {}: {err}",
                actor_dir.display()
            ));
        }
    };
    let mut prefix_dirs = prefix_dirs;
    while let Some(prefix_entry) = prefix_dirs.next_entry().await? {
        let prefix_path = prefix_entry.path();
        if !prefix_entry.file_type().await?.is_dir() {
            continue;
        }
        let prefix_name = match prefix_entry.file_name().into_string() {
            Ok(s) => s,
            Err(_) => continue,
        };
        if prefix_name.len() != 2 || hex::decode(&prefix_name).is_err() {
            continue;
        }
        let mut did_entries = match tokio::fs::read_dir(&prefix_path).await {
            Ok(rd) => rd,
            Err(err) => {
                tracing::warn!(prefix = %prefix_path.display(), %err, "skipping prefix dir");
                continue;
            }
        };
        while let Some(did_entry) = did_entries.next_entry().await? {
            let did_path = did_entry.path();
            if !did_entry.file_type().await?.is_dir() {
                continue;
            }
            let did = match did_entry.file_name().into_string() {
                Ok(s) => s,
                Err(_) => continue,
            };
            scanned += 1;
            // Skip DIDs without a sqlite store: an empty directory is
            // not a real actor.
            if !did_path.join("store.sqlite").exists() {
                continue;
            }
            if state.actor_store.has_rotation_keypair(&did) {
                already += 1;
                continue;
            }
            match state
                .actor_store
                .write_rotation_key_from_bytes(&did, &secret_bytes)
                .await
            {
                Ok(()) => {
                    seeded += 1;
                    println!("seeded rotation key for {did}");
                }
                Err(err) => {
                    errors += 1;
                    tracing::error!(%did, %err, "failed to seed rotation key");
                }
            }
        }
    }

    println!(
        "migrate rotation-keys: scanned={scanned} already_had={already} seeded={seeded} errors={errors}"
    );
    Ok(())
}

/// Walks every DID whose PLC document still lists the shared server
/// rotation key, fetches the per-DID rotation key from the actor store,
/// builds a minimal PLC update op that replaces `rotation_keys` with
/// just the per-DID key, signs and submits it, and marks the actor row
/// as migrated on success.
///
/// Pass `--dry-run` to log what would change without contacting PLC.
/// Idempotent: actors already marked migrated are skipped.
async fn migrate_plc_rotation_keys(dry_run: bool) -> anyhow::Result<()> {
    use cacos_pds::plc::operations::CreateAtprotoUpdateOpOpts;
    use cacos_pds::plc::operations::create_atproto_update_op;
    use cacos_pds::plc::types::{CompatibleOp, CompatibleOpOrTombstone, OpOrTombstone};
    use cacos_pds::xrpc::com::atproto::server::PDS_PLC_ROTATION_KEYPAIR;

    let cfg = cacos_pds::config::env_to_cfg();
    let state = cacos_pds::xrpc::types::SharedStateFromEnv::from_env(&cfg).await;

    let dids = state
        .account_manager
        .list_unmigrated_plc_rotation_key_dids()
        .await?;
    if dids.is_empty() {
        println!("migrate plc-rotation-keys: nothing to do");
        return Ok(());
    }
    println!(
        "migrate plc-rotation-keys: {n} DID(s) pending{}",
        if dry_run { " (dry-run)" } else { "" },
        n = dids.len()
    );

    let global_rotation_did = cacos_pds::xrpc::com::atproto::server::global_plc_rotation_key_did();

    let mut updated = 0usize;
    let mut skipped_no_key = 0usize;
    let mut skipped_already_clean = 0usize;
    let mut errors = 0usize;

    for did in &dids {
        let Some(per_did_keypair) = state.actor_store.rotation_keypair(did).await.ok() else {
            tracing::warn!(%did, "no per-DID rotation key on disk; skipping");
            skipped_no_key += 1;
            continue;
        };
        let per_did_did_key = rsky_crypto::utils::encode_did_key(&per_did_keypair.public_key());

        let last_op = match state.plc_client.get_last_op(did).await {
            Ok(op) => op,
            Err(err) => {
                tracing::error!(%did, %err, "get_last_op failed");
                errors += 1;
                continue;
            }
        };
        let normalized: CompatibleOp = match last_op {
            CompatibleOpOrTombstone::CreateOpV1(op) => CompatibleOp::CreateOpV1(op),
            CompatibleOpOrTombstone::Operation(op) => CompatibleOp::Operation(op),
            CompatibleOpOrTombstone::Tombstone(_) => {
                tracing::warn!(%did, "DID is tombstoned; skipping");
                skipped_already_clean += 1;
                continue;
            }
        };
        let new_op = match create_atproto_update_op(
            normalized,
            &per_did_keypair.secret_key(),
            CreateAtprotoUpdateOpOpts {
                signing_key: None,
                handle: None,
                pds: None,
                rotation_keys: Some(vec![per_did_did_key.clone()]),
            },
        )
        .await
        {
            Ok(op) => op,
            Err(err) => {
                tracing::error!(%did, %err, "failed to build PLC update op");
                errors += 1;
                continue;
            }
        };
        // Skip DIDs whose document already only lists the per-DID key —
        // nothing to do, but mark them migrated so the next run skips.
        if !global_rotation_did
            .as_ref()
            .is_some_and(|g| new_op.rotation_keys.contains(g))
        {
            println!("{did}: document already lists per-DID key only; marking migrated");
            if !dry_run
                && let Err(err) = state
                    .account_manager
                    .mark_plc_rotation_keys_migrated(did)
                    .await
            {
                tracing::error!(%did, %err, "failed to mark migrated");
                errors += 1;
            }
            skipped_already_clean += 1;
            continue;
        }

        if dry_run {
            println!(
                "{did}: would submit PLC op replacing rotation_keys with [{}]",
                per_did_did_key
            );
            continue;
        }

        match state
            .plc_client
            .send_operation(did, &OpOrTombstone::Operation(new_op))
            .await
        {
            Ok(()) => {
                println!("{did}: submitted PLC rotation-key rewrite");
                if let Err(err) = state
                    .account_manager
                    .mark_plc_rotation_keys_migrated(did)
                    .await
                {
                    tracing::error!(%did, %err, "submitted but failed to mark migrated");
                    errors += 1;
                } else {
                    updated += 1;
                }
            }
            Err(err) => {
                tracing::error!(%did, %err, "PLC send_operation failed");
                errors += 1;
            }
        }
    }

    // Touch the global key to fail fast if the env var is missing; the
    // server start path already validates it.
    let _ = PDS_PLC_ROTATION_KEYPAIR.public_key();

    println!(
        "migrate plc-rotation-keys: updated={updated} skipped_no_key={skipped_no_key} \
         skipped_already_clean={skipped_already_clean} errors={errors}"
    );
    Ok(())
}
