# cacos RemoteClient (reference)

A reference SvelteKit server implementing the RemoteClient role from the
cacos headless-consent and passkeys specs.

## Run against cacos

```bash
# 1. Start cacos PDS (in a separate terminal, from the repo root):
mise run dev
# PDS listens on http://127.0.0.1:8080 by default.

# 2. Tell the PDS about this RemoteClient:
export PDS_OAUTH_REMOTE_CLIENT_URL=https://localhost:5194
export PDS_OAUTH_REMOTE_CLIENT_TOKEN=replace-me   # any shared secret

# 3. Start the RemoteClient:
cd remote-client
cp .env.example .env
# edit .env: PDS_URL, PDS_OAUTH_REMOTE_CLIENT_TOKEN
pnpm install
pnpm dev
# serves https://localhost:5194 (mkcert installs a local CA on first run)
```

Open https://localhost:5194 and trigger any `oauth/authorize` URL on the PDS to walk the consent flow.

## Tests

```bash
pnpm test          # vitest unit tests
pnpm e2e           # playwright e2e, gated — needs a real PDS
RUN_E2E=1 pnpm e2e # actually run e2e against the running PDS
```

## What works today vs. what's contract-only

| Surface | Status |
|---|---|
| Consent screens (sign-in, select, consent, create, error) | works against the real PDS (Plan 06 done). |
| Passkeys (auth, enrollment) | built to spec contract; integration pending the Plan 06 passkey follow-up. |
| Account info via `com.atproto.sync.getRepoStatus` | built to spec contract; integration pending Plan 08 Task 24. |