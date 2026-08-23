<script lang="ts">
  import SignIn from '$lib/components/screens/SignIn.svelte';
  import SelectAccount from '$lib/components/screens/SelectAccount.svelte';
  import Consent from '$lib/components/screens/Consent.svelte';
  import CreateAccount from '$lib/components/screens/CreateAccount.svelte';
  import ErrorScreen from '$lib/components/screens/Error.svelte';
  import Splash from '$lib/components/screens/Splash.svelte';
  let { data, form } = $props();
</script>

{#if data.splash}
  <Splash />
  {#if import.meta.env.DEV}
    <nav class="mock-triggers" aria-label="Mock flow (dev only)">
      <h2>Mock flow (dev only)</h2>
      <div class="mock-buttons">
        <wa-button href="/?mock=sign-in" appearance="outlined">sign-in</wa-button>
        <wa-button href="/?mock=select" appearance="outlined">select</wa-button>
        <wa-button href="/?mock=consent" appearance="outlined">consent</wa-button>
        <wa-button href="/?mock=create" appearance="outlined">create</wa-button>
        <wa-button href="/?mock=error" appearance="outlined">error</wa-button>
        <wa-button href="/account?mock" appearance="outlined">account</wa-button>
        <wa-button href="/mock-client" appearance="outlined">mock-client</wa-button>
      </div>
    </nav>
  {/if}
{:else}
  {@const { rqid, state, deviceId, payload } = data}
  {@const base = { rqid, state, deviceId, client: payload.client }}
  {#if payload.screen === 'sign-in'}
    <SignIn {...base} loginHint={payload.login_hint} error={form?.error ?? null} />
  {:else if payload.screen === 'select'}
    <SelectAccount {...base} sessions={payload.sessions} />
  {:else if payload.screen === 'consent'}
    <Consent {...base} scopes={payload.scopes} session={payload.sessions[0]} />
  {:else if payload.screen === 'create'}
    <CreateAccount {...base} error={form?.error ?? null} error_description={form?.error_description ?? null} />
  {:else if payload.screen === 'error'}
    <ErrorScreen {...base} error={payload.error ?? 'error'} error_description={payload.error_description ?? ''} />
  {/if}
{/if}

<style>
  .mock-triggers {
    position: fixed;
    inset-block-end: var(--wa-space-m);
    inset-inline-end: var(--wa-space-m);
    background: var(--wa-color-surface-default);
    border: 1px solid var(--wa-color-surface-border);
    border-radius: var(--wa-border-radius-m);
    padding: var(--wa-space-s);
    max-inline-size: 18rem;
    font-size: var(--wa-font-size-s);
    box-shadow: 0 2px 8px rgb(0 0 0 / 8%);
  }
  .mock-triggers h2 {
    font-size: var(--wa-font-size-s);
    margin: 0 0 var(--wa-space-2xs);
    color: var(--wa-color-text-quiet);
  }
  .mock-buttons {
    display: flex;
    flex-wrap: wrap;
    gap: var(--wa-space-2xs);
  }
</style>
