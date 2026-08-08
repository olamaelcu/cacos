<script lang="ts">
  import type { SessionInfo } from '$lib/server/pds/types';
  import Screen from '$lib/components/Screen.svelte';
  let { session }: { session: SessionInfo } = $props();
  let status = $state<{ did: string; active: boolean; rev?: string|null } | null>(null);
  let error = $state<string | null>(null);

  $effect(() => {
    fetch(`/api/repo-status?did=${encodeURIComponent(session.did)}`)
      .then(r => r.ok ? r.json() : Promise.reject(r.statusText))
      .then(j => status = j)
      .catch(e => error = String(e));
  });
</script>

<Screen heading="Account info">
  <dl class="info-list">
    <dt>DID</dt>
    <dd>{session.did}</dd>

    <dt>Handle</dt>
    <dd>{session.handle ?? '—'}</dd>

    {#if session.email}
      <dt>Email</dt>
      <dd>{session.email}</dd>
    {/if}

    {#if error}
      <dt>Status</dt>
      <dd>
        <wa-callout variant="danger" style="margin: 0;">
          <wa-icon slot="icon" name="circle-exclamation"></wa-icon>
          Repo status unavailable: {error}
        </wa-callout>
      </dd>
    {:else if status}
      <dt>Active</dt>
      <dd>{status.active ? 'yes' : 'no'}</dd>

      {#if status.rev}
        <dt>Rev</dt>
        <dd><code>{status.rev}</code></dd>
      {/if}
    {:else}
      <dt>Status</dt>
      <dd><wa-spinner></wa-spinner></dd>
    {/if}
  </dl>
</Screen>

<style>
  .info-list {
    display: grid;
    grid-template-columns: max-content 1fr;
    column-gap: var(--wa-space-m);
    row-gap: var(--wa-space-2xs);
    margin: 0;
  }
  .info-list dt {
    color: var(--wa-color-text-quiet);
    font-weight: var(--wa-font-weight-semibold);
  }
  .info-list dd {
    margin: 0;
    overflow-wrap: anywhere;
  }
</style>
