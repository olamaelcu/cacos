<script lang="ts">
  import type { ClientInfo } from '$lib/server/pds/types';
  import { goto } from '$app/navigation';
  import Screen from '$lib/components/Screen.svelte';
  let { rqid, state: flowState, deviceId, client, loginHint, error }: {
    rqid: string; state: string; deviceId: string;
    client: ClientInfo; loginHint: string | null; error: string | null;
  } = $props();

  let passkeyError = $state<string | null>(null);
  let errorDialogEl: HTMLElement | undefined = $state();

  $effect(() => {
    const el = errorDialogEl;
    if (!el) return;
    const onHide = () => (passkeyError = null);
    el.addEventListener('wa-after-hide', onHide);
    return () => el.removeEventListener('wa-after-hide', onHide);
  });
</script>

<Screen heading={`Sign in to ${client.name ?? 'continue'}`}>
  {#if error}
    <wa-callout variant="danger" style="margin-block-end: var(--wa-space-s);">
      <wa-icon slot="icon" name="circle-exclamation"></wa-icon>
      {error}
    </wa-callout>
  {/if}

  <form method="POST" action="?/signIn" class="wa-stack">
    <input type="hidden" name="rqid" value={rqid} />
    <input type="hidden" name="state" value={flowState} />
    <input type="hidden" name="device_id" value={deviceId} />

    <wa-input
      name="identifier"
      label="Identifier"
      autocomplete="username"
      value={loginHint ?? ''}
      required
    ></wa-input>
    <wa-input
      name="password"
      label="Password"
      type="password"
      autocomplete="current-password"
      password-toggle
      required
    ></wa-input>

    <wa-button type="submit" variant="brand" style="width: 100%;">Sign in</wa-button>
  </form>

  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <wa-button
    appearance="outlined"
    style="width: 100%; margin-block-start: var(--wa-space-s);"
    onclick={async () => {
      const start = await fetch('/api/passkey/start', { method: 'POST', headers: {'content-type':'application/json'}, body: JSON.stringify({ rqid, state: flowState, mode: 'entryway' }) }).then(r => r.json());
      const { beginAuth, authToFinishBody } = await import('$lib/passkey/client');
      const r = await beginAuth(start);
      const body = authToFinishBody(rqid, flowState, deviceId, r);
      const res = await fetch('/api/passkey/finish', { method: 'POST', headers: {'content-type':'application/json'}, body: JSON.stringify(body) });
      if (!res.ok) {
        passkeyError = 'Passkey sign-in failed. Please try again.';
        return;
      }
      const payload = await res.json();
      goto(`/?rqid=${encodeURIComponent(rqid)}&state=${encodeURIComponent(payload.state ?? '')}`);
    }}
  >
    Sign in with passkey
  </wa-button>
</Screen>

<wa-dialog
  bind:this={errorDialogEl}
  label="Passkey sign-in failed"
  open={passkeyError !== null}
>
  {passkeyError ?? ''}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <wa-button slot="footer" variant="brand" onclick={() => (passkeyError = null)}>OK</wa-button>
</wa-dialog>
