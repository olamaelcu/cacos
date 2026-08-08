<script lang="ts">
  import AccountInfo from '$lib/components/screens/AccountInfo.svelte';
  let { data } = $props();
  let session = $derived({ did: data.did, handle: data.handle ?? null, email: data.email ?? null });
</script>
<AccountInfo {session} />