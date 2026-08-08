<script lang="ts">
  import type { Snippet } from 'svelte';
  let {
    children,
    wide = false,
    heading,
  }: {
    children: Snippet;
    wide?: boolean;
    heading?: string;
  } = $props();
</script>

<main class="screen">
  <wa-card class={wide ? 'screen-card screen-card-wide' : 'screen-card'}>
    {#if heading}
      <h1 slot="header" class="screen-heading">{heading}</h1>
    {/if}
    {@render children()}
  </wa-card>
</main>

<style>
  .screen {
    min-height: 100dvh;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: var(--wa-space-l);
    box-sizing: border-box;
  }

  .screen-card {
    width: 100%;
    max-inline-size: 26rem;
  }

  .screen-card-wide {
    max-inline-size: 44rem;
  }

  .screen-heading {
    margin: 0;
    font-size: var(--wa-font-size-l);
    font-weight: var(--wa-font-weight-semibold);
  }

  :global(.screen-card .wa-stack > *) {
    margin-block-end: var(--wa-space-s);
  }
  :global(.screen-card .wa-stack > :last-child) {
    margin-block-end: 0;
  }
</style>
