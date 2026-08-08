<script lang="ts">
  import type { ClientInfo, SessionInfo } from '$lib/server/pds/types';
  let { rqid, state, deviceId, client, sessions }: {
    rqid: string; state: string; deviceId: string;
    client: ClientInfo; sessions: SessionInfo[];
  } = $props();
</script>

<h1>Choose an account for {client.name ?? 'this app'}</h1>
<ul>
  {#each sessions as s}
    <li>
      <form method="POST" action="?/select">
        <input type="hidden" name="rqid" value={rqid} />
        <input type="hidden" name="state" value={state} />
        <input type="hidden" name="device_id" value={deviceId} />
        <input type="hidden" name="did" value={s.did} />
        <button type="submit">{s.handle ?? s.did}</button>
      </form>
    </li>
  {/each}
</ul>
