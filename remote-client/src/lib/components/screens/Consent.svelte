<script lang="ts">
  import type { ClientInfo, SessionInfo } from '$lib/server/pds/types';
  let { rqid, state, deviceId, client, scopes, session }: {
    rqid: string; state: string; deviceId: string;
    client: ClientInfo; scopes: string[]; session: SessionInfo;
  } = $props();
</script>

<h1>{client.name ?? 'This app'} wants to access your account</h1>
<p>Account: <strong>{session.handle ?? session.did}</strong></p>
<p>Scopes: {scopes.join(', ')}</p>

<form method="POST" action="?/accept" style="display:inline">
  <input type="hidden" name="rqid" value={rqid} />
  <input type="hidden" name="state" value={state} />
  <input type="hidden" name="device_id" value={deviceId} />
  <input type="hidden" name="did" value={session.did} />
  <button type="submit">Allow</button>
</form>
<form method="POST" action="?/reject" style="display:inline">
  <input type="hidden" name="rqid" value={rqid} />
  <input type="hidden" name="state" value={state} />
  <input type="hidden" name="device_id" value={deviceId} />
  <button type="submit">Deny</button>
</form>
