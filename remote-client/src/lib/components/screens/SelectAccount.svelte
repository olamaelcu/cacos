<script lang="ts">
  import type { ClientInfo, SessionInfo } from '$lib/server/pds/types';
  import Screen from '$lib/components/Screen.svelte';
  let { rqid, state, deviceId, client, sessions }: {
    rqid: string; state: string; deviceId: string;
    client: ClientInfo; sessions: SessionInfo[];
  } = $props();
</script>

<Screen heading={`Choose an account for ${client.name ?? 'this app'}`}>
  <form method="POST" action="?/select" class="wa-stack">
    <input type="hidden" name="rqid" value={rqid} />
    <input type="hidden" name="state" value={state} />
    <input type="hidden" name="device_id" value={deviceId} />

    <wa-radio-group
      label="Account"
      name="did"
      value={sessions[0].did}
      required
    >
      {#each sessions as s}
        <wa-radio value={s.did}>{s.handle ?? s.did}</wa-radio>
      {/each}
    </wa-radio-group>

    <wa-button type="submit" variant="brand" style="width: 100%;">Continue</wa-button>
  </form>
</Screen>
