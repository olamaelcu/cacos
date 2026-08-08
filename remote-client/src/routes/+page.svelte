<script lang="ts">
  import SignIn from '$lib/components/screens/SignIn.svelte';
  import SelectAccount from '$lib/components/screens/SelectAccount.svelte';
  import Consent from '$lib/components/screens/Consent.svelte';
  import CreateAccount from '$lib/components/screens/CreateAccount.svelte';
  import ErrorScreen from '$lib/components/screens/Error.svelte';
  let { data, form } = $props();
  const { rqid, state, deviceId, payload } = $derived(data);
  const base = $derived({ rqid, state, deviceId, client: payload.client });
</script>

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
