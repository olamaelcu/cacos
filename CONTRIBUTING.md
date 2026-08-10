# Contributing

Thanks for your interest in cacos. This guide covers how to file issues,
the development workflow, and the rules of engagement for each part of
the codebase. Read it before opening a pull request.

## Code of Conduct

All participants are expected to follow the spirit of the
[Contributor Covenant](https://www.contributor-covenant.org/version/2/1/code_of_conduct/).
Be patient, be kind, assume good faith, and leave the codebase in a
better shape than you found it.

## Filing issues

Use the issue tracker for this repository. cacos is a single-repo
project: there are no submodules and no per-crate split repositories.

A useful issue includes: the symptom, the steps to reproduce, the
expected and actual behavior, and a log excerpt or screenshot. For HTTP
or XRPC bugs, also note the NSID, the request method, the response
status / body, and the cacos commit (`git rev-parse HEAD`) you
reproduced against. For blobstore bugs, note whether you reproduced
against the local filesystem backend or MinIO/S3, and the relevant
`S3_*` / `PDS_BLOBSTORE_DISK_LOCATION` values you had set.

## Working in a worktree

The repository's `.worktree/` directory is reserved for task isolation.
Always work in a worktree, never directly on `main`:

```bash
git worktree add .worktree/short-topic-name -b feat/short-topic-name
cd .worktree/short-topic-name
```

Why:

- The `pds/` build produces large `target/` directories and Cargo's
  incremental cache. A worktree keeps each branch's build artifacts
  out of `main`'s working tree.
- The `nose` duplication report runs against a fixed set of source
  roots; keeping changes in a worktree makes "works on my machine"
  match what CI sees.
- The CI runner provisions a worktree per PR; matching the workflow
  locally makes surprises rare.

Before removing a worktree, commit or stash everything in it
(`git status` clean) — the `.worktree/` rule forbids deleting an
uncommitted worktree.

## Commit messages

The project follows a lightweight Conventional Commits style with DCO
signoff.

- **Subject line** — imperative mood, ≤ 72 characters, no trailing
  period. Prefix with the area: `pds: …`, `migration: …`,
  `remote-client: …`, `mise: …`, `docs: …`, `ci: …`, `infra: …`.
- **Body** — wrap at 72 columns; explain *why*, not *what*. Link
  relevant issues with `Refs #123` or `Closes #123`.
- **DCO signoff** — every commit must include a `Signed-off-by:` line.
  Use `git commit -s` to add it automatically. By signing off you
  certify the [Developer Certificate of Origin 1.1](https://developercertificate.org/).
- **AI attribution** — if an AI tool (Claude, Copilot, Cursor, …) made
  a non-trivial contribution to the change, add a `Co-Authored-By:`
  trailer naming the tool. The repo follows the model-attribution
  convention; see the `model-attribution` skill for the canonical
  trailer format per provider.

Example:

```
pds: route com.atproto.server.createSession through AccountManager

Switches the createSession handler from the hand-rolled password check
to AccountManager::create_session. Refresh-token rotation, signup
metrics, and account-status checks now live in one place.

Refs #42
Signed-off-by: Jane Doe <jane@example.com>
Co-Authored-By: Claude <noreply@anthropic.com>
```

### Updating the rsky pin

The `Cargo.toml` workspace pins the [rsky][rsky] ATProto protocol crates
from a fork at a specific git rev. Bumping that rev is high-blast-radius
because every `rsky-*` crate is rebuilt and a subtle upstream change can
break sea-orm queries or the auth verifier. Do it in its own commit,
run the full workspace test suite (`mise run test`), and call it out in
the PR description.

[rsky]: https://github.com/olamaelcu/rsky

## Coding standards

Each part of the repository has its own contributor notes:

- The [cacos README](./README.md) is the high-level orientation: what
  the workspace looks like, how storage is sharded, where metrics and
  observability live.
- [`doc/adr/`](./doc/adr/) holds the accepted architecture decision
  records. New ADRs go in chronological order; cross-reference the
  decision number in commit messages and PR descriptions.
- The `pds/src/xrpc/` module table is the inventory of wired XRPC
  handlers; `pds/src/sequencer/` documents the firehose shape; the
  `pds::db::DatabaseKind` enum is the single opening API for every
  SQLite database.

A few conventions that apply across the whole repo:

- **No hand-edits to generated files.** Sea-orm entity models
  (`migration/src/entities/`), poem-openapi generated bindings (none
  today, but if any land in `pds/src/xrpc/` they stay generated), and
  the Cargo lockfile are build artifacts. The lockfile is checked in
  and updated by `cargo` only.
- **No secrets in commits.** Anything that would otherwise live in a
  `.env` file or `docker-compose.yaml` overrides stays out of the
  diff. The MinIO credentials in `mise.toml` are dev-only fixtures,
  not production secrets.
- **No surprises in shared config.** Changes to `Cargo.toml`,
  `mise.toml`, `mise.lock`, `rust-toolchain.toml`, `docker-compose.yaml`,
  and `.gitignore` are high-blast-radius. Discuss them in an issue
  first and call them out in the PR description.
- **No emojis in source files or commits.** Keep the diff readable in
  monochrome.

## Testing

The single test entry point is `mise run test` (which calls
`cargo nextest run --workspace`). A few rules of the road:

- **All targets** — `mise run check` type-checks every target
  (lib + bins + tests + examples). Run it before pushing.
- **Formatting** — `mise run fmt` checks rustfmt formatting;
  `mise run format` applies it. CI runs `mise run fmt`.
- **Linting** — `mise run lint` runs clippy with warnings denied
  (`-D warnings`). CI treats a warning as a failure.
- **Duplication** — `mise run dup` runs `nose` against the `pds/`,
  `pds/tests/`, and `migration/src/` source roots. The migration
  crate's `m2026*` migrator files are excluded via `nose.toml`.
- **Test fixtures** — integration tests that need MinIO depend on
  `mise run infra-up` having run. The unit-test layer does not need
  any infra; if your test does, gate it on the dev profile and skip
  it in CI's no-infra run.
- **PRDs of the four SQLite databases** — when a test needs a clean
  account / sequencer / did-cache / actor database, use
  `pds::db::DatabaseKind::open` with a `camino_tempfile::tempdir()`
  path. The old free `open_*_db` helpers no longer exist.

## Submitting a pull request

1. **Open an issue first** for non-trivial changes so the design can
   be discussed. Small bug fixes and typo corrections do not need an
   issue.
2. **Branch from `main`** (or the long-lived branch the issue
   references) inside a worktree, not from a fork you cannot push to.
3. **Keep the diff focused.** One concern per PR. If a fix has
   follow-up work, file a follow-up issue and link it from the PR.
4. **Update the docs that go with the change.** If you add an
   end-user-facing feature, update `README.md`. If you change a
   workflow, update this file. Architectural decisions land as ADRs
   under `doc/adr/`.
5. **Fill in the PR template** with:
   - What the change does and why.
   - How you tested it (`mise run test`, `mise run lint`, etc.).
   - Anything reviewers should pay extra attention to.
   - The cacos commit SHA you tested against, and the rsky fork rev
     if you bumped it.
6. **Wait for CI.** Two approvals are required for non-trivial
   changes; one is enough for typo / docs / generated-file PRs.

### Pre-PR checklist

- [ ] `git status` is clean (no stray files staged or unstaged)
- [ ] `git diff --stat` matches the PR description
- [ ] Commit messages follow the convention above and include
      `Signed-off-by:`
- [ ] AI contributions carry a `Co-Authored-By:` trailer
- [ ] `mise run check` passes (or you only touched non-Rust files)
- [ ] `mise run test` passes
- [ ] `mise run lint` passes
- [ ] `mise run fmt` passes
- [ ] `mise run dup` shows no new duplication hotspots
- [ ] No hand-edits to generated files (sea-orm entities, etc.)
- [ ] No secrets, API keys, or `.env` content in the diff
- [ ] README.md / doc/adr/ updated to match the change
- [ ] If you bumped the rsky rev, the commit is its own and the PR
      description calls it out

## Working with the rsky fork

Most ATProto surface area in cacos comes from the [rsky][rsky] protocol
crates pinned in `Cargo.toml`. The typical flow when upstream rsky
changes:

1. Open an issue or PR on <https://github.com/olamaelcu/rsky> for the
   upstream change. Land it on the `main` branch of the fork.
2. In this repository, bump the rev in the `[workspace.dependencies]`
   block of `Cargo.toml` and re-pin every `rsky-*` entry to the same
   SHA. Use the same rev for all `rsky-*` crates — they are
   intra-versioned.
   ```bash
   # After updating the Cargo.toml rev:
   cargo update -p rsky-repo -p rsky-crypto -p rsky-syntax \
                -p rsky-common -p rsky-blobstore -p rsky-lexicon \
                -p rsky-oauth -p rsky-identity
   ```
3. Run `mise run test` and `mise run lint`. Expect a sea-orm query
   change or a JWT-claim change to ripple through `pds/src/account/`
   and `pds/src/xrpc/auth_extractors.rs`.
4. Land the bump as its own commit. If a hand-port in cacos is needed
   to keep the workspace green, land that as a separate commit so a
   partial merge cannot land one half of the change without the other.

If a change crosses both sides of the cacos / rsky boundary (e.g. you
are using a new lexicon type or a new repo event shape), it is usually
cleaner to land the rsky side first, then the cacos call sites in this
repository.

## Release process (for maintainers)

Tag-driven, manual for now:

1. Pick the version. Bump the `version` in the `cacos-pds` and
   `cacos-migration` `Cargo.toml` manifests. Keep them in lockstep.
2. Run `mise run lint` and `mise run test`. Verify the green build
   locally.
3. Tag the release: `git tag -s vX.Y.Z -m "vX.Y.Z"`. Push the tag:
   `git push origin vX.Y.Z`.
4. Build a release artifact: `mise run build` (or
   `cargo build --release -p cacos-pds`). The resulting binary lives
   under `target/release/cacos-pds`.

## Questions

If something is unclear, open a draft PR or an issue and ask. The
"rules" above are not enforced by CI as a hard gate — they exist to
make review easier, not to gatekeep contributions.
