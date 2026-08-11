# 13. Crate Splitting

Date: 2026-08-11

## Status

Accepted

## Context

`cacos-pds` had grown into a single 27k-LOC binary crate holding every
concern of the PDS: account manager + auth verifier, actor store,
sequencer, OAuth provider, XRPC HTTP surface, observability, db
helpers, error type, config loader, and the binary itself. Concerns that
are layered in spirit (a transport layer should not import a SQL
migrator) shared a single crate root and a single `pub mod …` namespace,
which made three failure modes chronic:

1. **Compile time.** Changing `cds/src/observability/timing.rs` rebuilt
   every downstream target including the OAuth route set, the
   sequencer, and the actor store. A 1-line tweak could trigger a
   full-link rebuild.
2. **Reuse.** A future relay-style binary (`cacos-bgs`), an admin CLI,
   or a mock PDS for integration tests could in principle depend on
   only the sequencer + actor store + plc + blobstore, but the
   workspace shape forced it to depend on the entire `cacos-pds` crate
   (and its daemon) or fork the source.
3. **API boundaries.** All paths were `pub` inside one crate root, so
   internal refactors rippled through to every call site. The `auth_verifier.rs`
   file held 1,426 LOC / 75 symbols with no enforced internal structure;
   callers reached directly into `crate::xrpc::rate_limit` and
   `crate::db::DatabaseKind` from anywhere.

## Decision

We split `cacos-pds` into a layered Cargo workspace of 12 crates with
a strict one-way dependency arrow enforced by Cargo itself:

```
       foundation: cacos-migration, cacos-pds-core
                  |
            account / actor-store / sequencer
                  |
                oauth
                  |
              server  (lib + bin)
```

Plus five L1 leaves (`blobstore`, `plc`, `identity`, `handle`,
`mailer`) that depend only on `cacos-migration` and external
[rsky][rsky] protocol crates. Higher layers may depend on lower
ones; lower ones never reach upward. Reverse edges (e.g. server
importing from actor-store importing from server) are impossible at
the Cargo resolver level.

The split landed across nine sequential merge commits, each
independently green against the four project-rule verification gates
(`cargo build`, `--lib` test, `--tests` integration test,
`cargo clippy -D warnings`). The bin target that previously lived
in `pds/src/main.rs` was split into two
binaries:

- `cacos-pds-server` (HTTP daemon on `127.0.0.1:8080`), as the
  `[[bin]]` of `cacos-pds-server`. Replaces the boot block that used to
  live in `pds/src/main.rs`.
- `cacos-pds-migrate` (operator CLI: `cacos-pds-migrate
  rotation-keys | plc-rotation-keys [--dry-run]`). Replaces the
  `migrate` subcommand dispatch that used to live in
  `pds/src/main.rs`.

`pds/` is now a thin shell whose only purpose is to host
`pds/tests/*.rs` integration suites; its `lib.rs` is a doc-comment
stub plus `pub mod context;` (the last in-tree module, holding
`SharedSequencer` — slated for cleanup in a follow-up).

Each extracted crate uses `include!("mod.rs")` (where the lib has
both submodule declarations and a body) or a flat LAYOUT A
(`src/lib.rs` carries the body) per the actor-store subagent's
convention. The `auth/auth_verifier.rs` file was split into five
submodules inside `cacos-pds-account/src/auth/verifier/`:
`register.rs` (env-var wiring + `LazyLock` statics), `bearer.rs`
(HTTP header parsing + JWT verify orchestration — the bulk of the
file), `dpop.rs` (placeholder for future DPoP-only helpers),
`admin.rs` (admin-token / account-status checks), and
`service_jwt.rs` (cross-service JWT signing/verification).

Per the project's "hard renames, no re-export shims" policy (from the
split design), every call site was updated in place. The break is
visible at the operator CLI: `cacos-pds migrate X` →
`cacos-pds-migrate X`. Operator scripts and CI must be updated.

[rsky]: https://github.com/olamaelcu/rsky

## Consequences

Easier:

- **Per-crate rebuilds.** Cargo's incremental compilation skips
  crates whose deps don't change. A tweak to the blobstore now
  rebuilds only `cacos-pds-blobstore` and its direct consumers; the
  sequencer and OAuth remain cached.
- **Reuse targets.** A relay-style binary can depend on
  `cacos-pds-{actor-store,sequencer,plc,blobstore,core,account}`
  without pulling in the daemon. An admin CLI can depend on
  `cacos-pds-{account,core,handle,mailer}` and skip the OAuth + XRPC
  stack entirely. A mock PDS for downstream integration tests gets a
  documented seam.
- **API boundaries.** Each crate's `pub` surface is now the
  contract; `cargo doc` and `cargo public-api` can audit it. The
  auth-verifier split enforces a smaller blast radius for changes to
  the bearer-vs-admin-vs-service-JWT concerns.
- **Clippy coverage per crate.** `cargo clippy -p cacos-pds-X` runs
  against just one crate's public API, catching more issues than a
  whole-tree run.
- **CI parallelism.** `cargo nextest --workspace` runs each crate's
  tests in parallel; the integration suite in `pds/tests/` still
  covers the whole stack end-to-end.

Harder:

- **CLI rename.** Anything scripted as `cacos-pds migrate X` must
  become `cacos-pds-migrate X`. Mitigated by the migration landing in
  one PR with a clear commit message; documented in the README and
  this ADR.
- **Cross-crate `#[cfg(test)]` plumbing.** Tests inside one crate
  can't see `#[cfg(test)]` items in another. The `db::tests`
  module in `cacos-pds-core` is feature-gated (`test-utils`) and made
  `pub` so downstream integration suites can construct the
  `TestDb` / `TestDatabaseKind` types. Trade-off: a small set of
  test-only types are exposed in the lib surface; fine because
  consumers only see them when they opt into the feature.
- **Sequencer ULID race in tests.** `sequencer::outbox::tests::events_backfills_from_cursor_then_continues_live`
  was a pre-existing flake (exists on `main` before the split; the
  test suite uses a process-global ULID counter that's subject to
  cross-test interference). The split did not introduce it. Not
  fixed by this ADR; tracked separately.
- **Operator-CLI thread-safety race in oauth_security tests.**
  `cors_allows_configured_origin` and
  `oauth_remote_rate_limit_blocks_after_threshold` use
  `unsafe { std::env::set_var(...) }` while tests run in parallel.
  Pass under `--test-threads=1`. Pre-existing; not introduced by the
  split.
- **`Cargo.lock` becomes canonical.** Per the project's rule
  ("commit Cargo.lock, don't add it to per-crate .gitignore"), the
  workspace's root lockfile is the single source of truth and must
  be tracked. Several subagents initially added `/Cargo.lock` to
  per-crate `.gitignore`; the dedicated `Stop ignoring Cargo.lock`
  commit reversed those lines.
- **`SharedSequencer` is still in `pds/src/context.rs`.** It has its
  canonical home in `cacos-pds-sequencer::shared_sequencer` but
  `pds/src/context` remains as a thin `pub mod context;` shim for any
  callers that still reference `crate::context`. A follow-up commit
  will retire it once every caller is rewired.
