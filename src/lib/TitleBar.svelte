<script lang="ts">
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { t } from "$lib/i18n.svelte";

  let { title, minimize = false }: { title: string; minimize?: boolean } = $props();
  const win = getCurrentWindow();
</script>

<header class="tb" data-tauri-drag-region>
  <svg width="16" height="16" viewBox="0 0 512 512" aria-hidden="true">
    <rect x="16" y="16" width="480" height="480" rx="112" fill="var(--accent)" />
    <path d="M80 236h76l40-92 56 196 40-104h140" fill="none" stroke="#fff" stroke-width="44" stroke-linecap="round" stroke-linejoin="round" />
  </svg>
  <span class="title" data-tauri-drag-region>{title}</span>
  <div class="win-controls">
    {#if minimize}
      <button aria-label={t("Minimize")} onclick={() => win.minimize()}>
        <svg width="10" height="10" viewBox="0 0 10 10"><path d="M0 5h10" stroke="currentColor" /></svg>
      </button>
    {/if}
    <button class="close" aria-label={t("Close")} onclick={() => win.close()}>
      <svg width="10" height="10" viewBox="0 0 10 10"><path d="M0 0l10 10M10 0L0 10" stroke="currentColor" /></svg>
    </button>
  </div>
</header>

<style>
  .tb {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 36px;
    padding-left: 12px;
    flex: none;
  }
  .title {
    flex: 1;
    min-width: 0;
    font-size: 12.5px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .win-controls button {
    width: 42px;
  }
</style>
