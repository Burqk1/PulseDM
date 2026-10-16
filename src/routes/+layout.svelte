<script lang="ts">
  import "../app.css";
  import { onMount } from "svelte";
  import { followPrefs } from "$lib/prefs";

  let { children } = $props();

  onMount(() => {
    const stop = followPrefs();
    return () => {
      stop.then((un) => un());
    };
  });
</script>

{@render children()}
