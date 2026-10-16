<script lang="ts">
  let { name, size = 16 }: { name: string; size?: number } = $props();
</script>

<svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    {#if name === "link"}<path d="M10 13a5 5 0 0 0 7.5.5l3-3a5 5 0 0 0-7-7l-1.7 1.7" /><path d="M14 11a5 5 0 0 0-7.5-.5l-3 3a5 5 0 0 0 7 7l1.7-1.7" />
    {:else if name === "magnet"}<path d="M6 3v8a6 6 0 0 0 12 0V3" /><path d="M10 3v8a2 2 0 0 0 4 0V3" /><path d="M6 7h4M14 7h4" />
    {:else if name === "pause"}<rect x="6" y="5" width="4" height="14" rx="1" /><rect x="14" y="5" width="4" height="14" rx="1" />
    {:else if name === "play"}<path d="M7 4l13 8-13 8z" />
    {:else if name === "stop"}<rect x="5" y="5" width="14" height="14" rx="2" />
    {:else if name === "retry"}<path d="M3 12a9 9 0 1 0 3-6.7L3 8" /><path d="M3 3v5h5" />
    {:else if name === "folder"}<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />
    {:else if name === "open"}<path d="M14 3h7v7" /><path d="M10 14L21 3" /><path d="M21 14v5a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5" />
    {:else if name === "trash"}<path d="M3 6h18" /><path d="M8 6V4h8v2" /><path d="M19 6l-1 14H6L5 6" />
    {:else if name === "clipboard"}<rect x="8" y="2" width="8" height="4" rx="1" /><path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2" />
    {:else if name === "broom"}<path d="M3 21l7-7" /><path d="M14 4l6 6-6 6-6-6z" />
    {:else if name === "search"}<circle cx="11" cy="11" r="7" /><path d="M21 21l-4.3-4.3" />
    {:else if name === "close"}<path d="M6 6l12 12M18 6L6 18" />
    {:else if name === "chevron"}<path d="M6 9l6 6 6-6" />
    {:else if name === "speed"}<path d="M12 3v12" /><path d="M7 10l5 5 5-5" /><path d="M5 21h14" />
    {:else if name === "list"}<path d="M8 6h13M8 12h13M8 18h13M3 6h.01M3 12h.01M3 18h.01" />
    {:else if name === "queue"}<path d="M4 6h16M4 12h10M4 18h6" /><path d="M17 15l3 3-3 3" />
    {:else if name === "clock"}<circle cx="12" cy="12" r="9" /><path d="M12 7v5l3 2" />
    {:else if name === "plus"}<path d="M12 5v14M5 12h14" />
    {:else if name === "edit"}<path d="M4 20h4L19 9l-4-4L4 16z" />
    {:else if name === "power"}<path d="M12 3v9" /><path d="M6.3 6.3a8 8 0 1 0 11.4 0" />
    {:else if name === "shield"}<path d="M12 3l8 3v6c0 5-3.5 8-8 9-4.5-1-8-4-8-9V6z" /><path d="M9 12l2 2 4-4" />
    {:else if name === "settings"}<circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z" />
    {:else if name === "video"}<rect x="2" y="5" width="15" height="14" rx="2" /><path d="M17 10l5-3v10l-5-3z" />
    {:else if name === "audio"}<path d="M9 18V5l12-2v13" /><circle cx="6" cy="18" r="3" /><circle cx="18" cy="16" r="3" />
    {:else if name === "archive"}<rect x="3" y="3" width="18" height="5" rx="1" /><path d="M5 8v11a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8" /><path d="M10 12h4" />
    {:else if name === "program"}<rect x="3" y="3" width="7" height="7" rx="1" /><rect x="14" y="3" width="7" height="7" rx="1" /><rect x="3" y="14" width="7" height="7" rx="1" /><rect x="14" y="14" width="7" height="7" rx="1" />
    {:else if name === "document"}<path d="M14 3H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9z" /><path d="M14 3v6h6" /><path d="M8 13h8M8 17h5" />
    {:else if name === "image"}<rect x="3" y="3" width="18" height="18" rx="2" /><circle cx="9" cy="9" r="2" /><path d="M21 15l-5-5L5 21" />
    {:else if name === "all"}<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />
    {:else if name === "finished"}<circle cx="12" cy="12" r="9" /><path d="M8 12l3 3 5-6" />
    {:else if name === "unfinished"}<circle cx="12" cy="12" r="9" /><path d="M12 7v5l3 2" />
    {:else}<path d="M14 3H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9z" /><path d="M14 3v6h6" />
    {/if}
  </svg>
