<script lang="ts">
  import type { SessionInfo } from '$lib/server/pds/types';
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

<h2>Account info</h2>
<p><strong>DID:</strong> {session.did}</p>
<p><strong>Handle:</strong> {session.handle ?? '—'}</p>
{#if error}
  <p role="alert">Repo status unavailable: {error}</p>
{:else if status}
  <p><strong>Active:</strong> {status.active ? 'yes' : 'no'}</p>
  {#if status.rev}<p><strong>Rev:</strong> {status.rev}</p>{/if}
{:else}
  <p>Loading…</p>
{/if}