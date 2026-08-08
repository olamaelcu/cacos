<script lang="ts">
  import type { ClientInfo } from '$lib/server/pds/types';
  import Screen from '$lib/components/Screen.svelte';
  let { rqid, state, deviceId, client, error, error_description }: {
    rqid: string; state: string; deviceId: string;
    client: ClientInfo; error: string | null; error_description: string | null;
  } = $props();
</script>

<Screen heading="Create your account">
  <p class="screen-lede">
    Signing up for {client.name ?? 'this app'}.
  </p>

  {#if error}
    <wa-callout variant="danger" style="margin-block-end: var(--wa-space-s);">
      <wa-icon slot="icon" name="circle-exclamation"></wa-icon>
      {error_description ?? error}
    </wa-callout>
  {/if}

  <form method="POST" action="?/createAccount" class="wa-stack">
    <input type="hidden" name="rqid" value={rqid} />
    <input type="hidden" name="state" value={state} />
    <input type="hidden" name="device_id" value={deviceId} />

    <wa-input name="handle" label="Handle" pattern="[a-z0-9-.]+" required></wa-input>
    <wa-input name="email" label="Email" type="email" required></wa-input>
    <wa-input name="password" label="Password" type="password" minlength={8} required></wa-input>
    <wa-input name="invite_code" label="Invite code (optional)"></wa-input>

    <wa-button type="submit" variant="brand" style="width: 100%;">Create account</wa-button>
  </form>
</Screen>

<style>
  .screen-lede {
    margin: 0 0 var(--wa-space-s);
    color: var(--wa-color-text-quiet);
  }
</style>
