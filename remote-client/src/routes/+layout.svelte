<script lang="ts">
  import { browser } from "$app/environment";

  let { children } = $props();

  if (browser) {
    import("@awesome.me/webawesome/dist/styles/webawesome.css");
    import("@awesome.me/webawesome/dist/webawesome.js");
  }
</script>

{@render children?.()}
