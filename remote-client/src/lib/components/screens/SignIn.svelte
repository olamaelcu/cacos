<script lang="ts">
  import type { ClientInfo } from '$lib/server/pds/types';
  let { rqid, state, deviceId, client, loginHint, error }: {
    rqid: string; state: string; deviceId: string;
    client: ClientInfo; loginHint: string | null; error: string | null;
  } = $props();
</script>

<h1>Sign in to {client.name ?? 'continue'}</h1>
{#if error}<p role="alert">{error}</p>{/if}
<form method="POST" action="?/signIn">
  <input type="hidden" name="rqid" value={rqid} />
  <input type="hidden" name="state" value={state} />
  <input type="hidden" name="device_id" value={deviceId} />
  <label>Identifier <input name="identifier" autocomplete="username" value={loginHint ?? ''} required /></label>
  <label>Password <input name="password" type="password" autocomplete="current-password" required /></label>
  <button type="submit">Sign in</button>
</form>
