<script lang="ts">
  import type { ClientInfo } from '$lib/server/pds/types';
  import { goto } from '$app/navigation';
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
<button type="button" onclick={async () => {
  const start = await fetch('/api/passkey/start', { method: 'POST', headers: {'content-type':'application/json'}, body: JSON.stringify({ rqid, state, mode: 'entryway' }) }).then(r => r.json());
  const { beginAuth, authToFinishBody } = await import('$lib/passkey/client');
  const r = await beginAuth(start);
  const body = authToFinishBody(rqid, state, deviceId, r);
  const res = await fetch('/api/passkey/finish', { method: 'POST', headers: {'content-type':'application/json'}, body: JSON.stringify(body) });
  if (!res.ok) { alert('Passkey sign-in failed'); return; }
  const payload = await res.json();
  // Re-enter the flow with the rotated state so the next screen renders.
  goto(`/?rqid=${encodeURIComponent(rqid)}&state=${encodeURIComponent(payload.state ?? '')}`);
}}>Sign in with passkey</button>
