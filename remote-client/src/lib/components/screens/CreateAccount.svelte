<script lang="ts">
  import type { ClientInfo } from '$lib/server/pds/types';
  let { rqid, state, deviceId, client, error, error_description }: {
    rqid: string; state: string; deviceId: string;
    client: ClientInfo; error: string | null; error_description: string | null;
  } = $props();
</script>

<h1>Create your account</h1>
<p>Signing up for {client.name ?? 'this app'}.</p>
{#if error}<p role="alert">{error_description ?? error}</p>{/if}
<form method="POST" action="?/createAccount">
  <input type="hidden" name="rqid" value={rqid} />
  <input type="hidden" name="state" value={state} />
  <input type="hidden" name="device_id" value={deviceId} />
  <label>Handle <input name="handle" required pattern="[a-z0-9-.]+" /></label>
  <label>Email <input name="email" type="email" required /></label>
  <label>Password <input name="password" type="password" required minlength="8" /></label>
  <label>Invite code (optional) <input name="invite_code" /></label>
  <button type="submit">Create account</button>
</form>
