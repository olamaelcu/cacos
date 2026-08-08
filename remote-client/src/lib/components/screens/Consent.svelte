<script lang="ts">
  import type { ClientInfo, SessionInfo } from '$lib/server/pds/types';
  import Screen from '$lib/components/Screen.svelte';
  let { rqid, state: flowState, deviceId, client, scopes, session }: {
    rqid: string; state: string; deviceId: string;
    client: ClientInfo; scopes: string[]; session: SessionInfo;
  } = $props();

  let passkeyBusy = $state(false);
  let passkeyMessage = $state<string | null>(null);
  let passkeyError = $state<string | null>(null);
  let messageDialogEl: HTMLElement | undefined = $state();
  let errorDialogEl: HTMLElement | undefined = $state();

  $effect(() => {
    const el = messageDialogEl;
    if (!el) return;
    const onHide = () => (passkeyMessage = null);
    el.addEventListener('wa-after-hide', onHide);
    return () => el.removeEventListener('wa-after-hide', onHide);
  });

  $effect(() => {
    const el = errorDialogEl;
    if (!el) return;
    const onHide = () => (passkeyError = null);
    el.addEventListener('wa-after-hide', onHide);
    return () => el.removeEventListener('wa-after-hide', onHide);
  });
</script>

<Screen heading={`${client.name ?? 'This app'} wants to access your account`}>
  <div class="wa-stack">
    <wa-callout>
      <wa-icon slot="icon" name="user"></wa-icon>
      <strong>Account:</strong>
      {session.handle ?? session.did}
      {#if session.email}<div class="account-email">{session.email}</div>{/if}
    </wa-callout>

    <wa-callout variant="neutral">
      <wa-icon slot="icon" name="key"></wa-icon>
      <strong>Scopes:</strong>
      {scopes.join(', ')}
    </wa-callout>

    <div class="consent-actions wa-cluster">
      <form method="POST" action="?/accept">
        <input type="hidden" name="rqid" value={rqid} />
        <input type="hidden" name="state" value={flowState} />
        <input type="hidden" name="device_id" value={deviceId} />
        <input type="hidden" name="did" value={session.did} />
        <wa-button type="submit" variant="success">Allow</wa-button>
      </form>
      <form method="POST" action="?/reject">
        <input type="hidden" name="rqid" value={rqid} />
        <input type="hidden" name="state" value={flowState} />
        <input type="hidden" name="device_id" value={deviceId} />
        <wa-button type="submit" variant="danger" appearance="outlined">Deny</wa-button>
      </form>
    </div>

    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <wa-button
      appearance="plain"
      style="width: 100%;"
      disabled={passkeyBusy}
      onclick={async () => {
        passkeyBusy = true;
        try {
          const startRes = await fetch('/api/passkey/register/start', { method: 'POST', headers: {'content-type':'application/json'}, body: JSON.stringify({ rqid, state: flowState, did: session.did }) });
          if (!startRes.ok) throw new Error(`start failed (${startRes.status})`);
          const start = await startRes.json();
          const { beginRegister, registerToFinishBody } = await import('$lib/passkey/client');
          const r = await beginRegister(start);
          const body = registerToFinishBody(rqid, flowState, deviceId, session.did, r);
          const finRes = await fetch('/api/passkey/register/finish', { method: 'POST', headers: {'content-type':'application/json'}, body: JSON.stringify(body) });
          if (!finRes.ok) throw new Error(`finish failed (${finRes.status})`);
          passkeyMessage = 'Passkey added.';
        } catch (e) {
          passkeyError = String(e);
        } finally {
          passkeyBusy = false;
        }
      }}
    >
      Add a passkey
    </wa-button>

    <wa-button href={`/account?did=${session.did}&handle=${session.handle ?? ''}&email=${session.email ?? ''}`} appearance="plain">
      View account info
    </wa-button>
  </div>
</Screen>

<wa-dialog
  bind:this={messageDialogEl}
  label="Passkey added"
  open={passkeyMessage !== null}
>
  Your passkey is now linked to this account.
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <wa-button slot="footer" variant="brand" onclick={() => (passkeyMessage = null)}>OK</wa-button>
</wa-dialog>

<wa-dialog
  bind:this={errorDialogEl}
  label="Could not add passkey"
  open={passkeyError !== null}
>
  {passkeyError ?? ''}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <wa-button slot="footer" variant="brand" onclick={() => (passkeyError = null)}>OK</wa-button>
</wa-dialog>

<style>
  .consent-actions {
    gap: var(--wa-space-s);
  }
  .consent-actions form {
    display: contents;
  }
  .account-email {
    margin-block-start: var(--wa-space-2xs);
    color: var(--wa-color-text-quiet);
    font-size: var(--wa-font-size-s);
  }
</style>
