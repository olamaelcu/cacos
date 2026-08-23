<script lang="ts">
  // Splash shown when a browser hits the RemoteClient root without a
  // `?rqid=&state=` pair. The PDS only redirects here with those
  // params; without them there's no flow to drive. This page explains
  // what the reference is and how to trigger the consent flow.
  import Screen from "$lib/components/Screen.svelte";
</script>

<Screen heading="cacos RemoteClient" wide>
  <img
    src="/cacos.jpg"
    height="320"
    alt="The national bird of Haiti nicknamed after the militant group of farmers who fought against US occupation."
  />
  <p>
    A reference SvelteKit server implementing the headless-consent and passkeys
    RemoteClient for the
    <wa-button
      href="https://github.com/olamaelcu/cacos"
      appearance="plain"
      size="small"
    >
      cacos
    </wa-button>
    ATProto PDS.
  </p>

  <h2 class="splash-h2">How the flow starts</h2>
  <p>
    This page is only visited after a cacos PDS redirects a browser here with <code
      >?rqid=…&amp;state=…</code
    >. To trigger the consent flow, hit any OAuth <code>/oauth/authorize</code>
    URL on the configured PDS; the PDS will <code>302</code> here with the rotating
    nonce, and the RemoteClient drives the screens from there.
  </p>

  <h2 class="splash-h2">Screens</h2>
  <ul class="splash-list">
    <li><strong>sign-in</strong> — identifier + password (or passkey)</li>
    <li>
      <strong>select</strong> — choose an account when the device has multiple sessions
    </li>
    <li><strong>consent</strong> — review scopes and Allow/Deny</li>
    <li>
      <strong>create</strong> — <code>prompt=create</code> account creation
    </li>
    <li>
      <strong>error</strong> — surfaces PDS
      <code>{"{error, error_description}"}</code>
    </li>
  </ul>

  <h2 class="splash-h2">Local dev</h2>
  <pre class="splash-pre">cd remote-client
cp .env.example .env  # set PDS_URL + PDS_OAUTH_REMOTE_CLIENT_TOKEN
pnpm dev              # serves https://localhost:5194 (mkcert TLS)</pre>

  <p>
    See <wa-button
      href="https://github.com/olamaelcu/cacos/tree/main/remote-client"
      appearance="plain"
      size="small"
    >
      remote-client/README.md
    </wa-button> for the full setup.
  </p>
</Screen>

<style>
  .splash-h2 {
    font-size: var(--wa-font-size-m);
    margin: var(--wa-space-m) 0 var(--wa-space-2xs);
  }
  .splash-list {
    margin: 0 0 var(--wa-space-s);
    padding-inline-start: var(--wa-space-m);
  }
  .splash-pre {
    background: var(--wa-color-surface-default);
    border: 1px solid var(--wa-color-surface-border);
    border-radius: var(--wa-border-radius-m);
    padding: var(--wa-space-s);
    overflow-x: auto;
    font-size: var(--wa-font-size-s);
    margin: 0 0 var(--wa-space-s);
  }
</style>
