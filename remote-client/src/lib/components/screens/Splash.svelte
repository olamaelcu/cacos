<script lang="ts">
  // Splash shown when a browser hits the RemoteClient root without a
  // `?rqid=&state=` pair. The PDS only redirects here with those
  // params; without them there's no flow to drive. This page explains
  // what the reference is and how to trigger the consent flow.
</script>

<main>
  <h1>cacos RemoteClient</h1>
  <p>
    A reference SvelteKit server implementing the headless-consent and
    passkeys RemoteClient for the <a href="https://github.com/olamaelcu/cacos">cacos</a>
    ATProto PDS.
  </p>

  <h2>How the flow starts</h2>
  <p>
    This page is only visited after a cacos PDS redirects a browser here
    with <code>?rqid=…&amp;state=…</code>. To trigger the consent flow, hit
    any OAuth <code>/oauth/authorize</code> URL on the configured PDS; the
    PDS will <code>302</code> here with the rotating nonce, and the
    RemoteClient drives the screens from there.
  </p>

  <h2>Screens</h2>
  <ul>
    <li><strong>sign-in</strong> — identifier + password (or passkey)</li>
    <li><strong>select</strong> — choose an account when the device has multiple sessions</li>
    <li><strong>consent</strong> — review scopes and Allow/Deny</li>
    <li><strong>create</strong> — <code>prompt=create</code> account creation</li>
    <li><strong>error</strong> — surfaces PDS <code>{'{error, error_description}'}</code></li>
  </ul>

  <h2>Local dev</h2>
  <pre>cd remote-client
cp .env.example .env  # set PDS_URL + PDS_OAUTH_REMOTE_CLIENT_TOKEN
pnpm dev              # serves https://localhost:5194 (mkcert TLS)</pre>

  <p>See <code>remote-client/README.md</code> for the full setup.</p>
</main>