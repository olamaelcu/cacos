<script lang="ts">
  import type { ClientInfo } from '$lib/server/pds/types';
  import Screen from '$lib/components/Screen.svelte';
  let { rqid, state, deviceId, client, error, error_description }: {
    rqid: string; state: string; deviceId: string;
    client: ClientInfo; error: string; error_description: string;
  } = $props();
</script>

<Screen heading="Something went wrong">
  <wa-callout variant="danger">
    <wa-icon slot="icon" name="circle-exclamation"></wa-icon>
    {error_description || error}
  </wa-callout>

  <wa-button href="/" variant="brand" style="width: 100%; margin-block-start: var(--wa-space-s);">
    Start over
  </wa-button>
</Screen>
