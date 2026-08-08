<script lang="ts">
  import type { ClientInfo } from '$lib/server/pds/types';
  let { rqid, state, deviceId, client, error, error_description }: {
    rqid: string; state: string; deviceId: string;
    client: ClientInfo; error: string; error_description: string;
  } = $props();
</script>

<h1>Something went wrong</h1>
<p>{error_description || error}</p>
<p><a href="/">Start over</a></p>
