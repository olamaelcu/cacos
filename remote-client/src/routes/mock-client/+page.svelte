<script lang="ts">
  import Screen from "$lib/components/Screen.svelte";
  let { data } = $props();
  let kind = $derived<"success" | "error" | "pending">(
    data.error ? "error" : data.code ? "success" : "pending",
  );
</script>

<Screen heading="Mock OAuth Client" wide>
  <p class="lede">
    This page simulates a third-party OAuth client receiving the callback from
    the cacos PDS consent flow. Triggered by clicking Allow or Deny on
    <a href="/?mock=consent"><code>/?mock=consent</code></a>.
  </p>

  {#if kind === "success"}
    <wa-callout variant="success" style="margin-block-end: var(--wa-space-s);">
      <wa-icon slot="icon" name="check"></wa-icon>
      Authorization granted
    </wa-callout>
    <dl class="info">
      <dt>code</dt>
      <dd><code>{data.code}</code></dd>
      {#if data.state}
        <dt>state</dt>
        <dd><code>{data.state}</code></dd>
      {/if}
    </dl>
  {:else if kind === "error"}
    <wa-callout variant="danger" style="margin-block-end: var(--wa-space-s);">
      <wa-icon slot="icon" name="circle-exclamation"></wa-icon>
      Authorization denied
    </wa-callout>
    <dl class="info">
      <dt>error</dt>
      <dd><code>{data.error}</code></dd>
      <dt>error_description</dt>
      <dd><code>{data.error_description ?? ""}</code></dd>
      {#if data.state}
        <dt>state</dt>
        <dd><code>{data.state}</code></dd>
      {/if}
    </dl>
  {:else}
    <wa-callout variant="neutral" style="margin-block-end: var(--wa-space-s);">
      <wa-icon slot="icon" name="info-circle"></wa-icon>
      No callback params. Visit
      <a href="/?mock=consent"><code>/?mock=consent</code></a> and click Allow or
      Deny to populate this page.
    </wa-callout>
  {/if}

  <nav class="controls">
    <wa-button href="/?mock=consent" variant="brand">Run flow again</wa-button>
    <wa-button href="/" appearance="outlined">Back to splash</wa-button>
  </nav>
</Screen>

<style>
  .lede {
    color: var(--wa-color-text-quiet);
    margin: 0 0 var(--wa-space-s);
  }
  .info {
    display: grid;
    grid-template-columns: max-content 1fr;
    column-gap: var(--wa-space-m);
    row-gap: var(--wa-space-2xs);
    margin: 0 0 var(--wa-space-m);
  }
  .info dt {
    color: var(--wa-color-text-quiet);
    font-weight: var(--wa-font-weight-semibold);
  }
  .info dd {
    margin: 0;
    overflow-wrap: anywhere;
  }
  .controls {
    display: flex;
    gap: var(--wa-space-s);
    margin-block-start: var(--wa-space-m);
    flex-wrap: wrap;
  }
</style>
