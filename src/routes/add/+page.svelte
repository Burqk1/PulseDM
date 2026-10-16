<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { readText } from "@tauri-apps/plugin-clipboard-manager";
  import Icon from "$lib/Icon.svelte";
  import TitleBar from "$lib/TitleBar.svelte";
  import { fitWindow } from "$lib/fitWindow";
  import { getPrefs } from "$lib/prefs";
  import { queueLabel, t } from "$lib/i18n.svelte";
  import {
    api,
    extractUrls,
    formatBytes,
    formatDuration,
    isVideoSite,
    type MediaInfo,
    type QueueConfig,
    type ToolProgress,
    type UrlInfo,
  } from "$lib/api";

  const win = getCurrentWindow();

  let urlText = $state("");
  let headers = $state<[string, string][]>([]);
  let dir = $state("");
  let name = $state("");
  let nameEdited = false;
  let dirEdited = false;
  let queues = $state<QueueConfig[]>([]);
  let queue = $state("main");
  let showMore = $state(false);
  let user = $state("");
  let pass = $state("");
  let conns = $state("");

  let info = $state<UrlInfo | null>(null);
  let media = $state<MediaInfo | null>(null);
  let option = $state("");
  let loading = $state(false);
  let error = $state<string | null>(null);
  let tool = $state<ToolProgress | null>(null);
  let busy = $state(false);
  let probeSeq = 0;

  const urls = $derived(extractUrls(urlText));
  const single = $derived(urls.length === 1 ? urls[0] : null);
  let mediaRequested = $state(false);
  let draftTitle = $state<string | null>(null);
  const isMedia = $derived(!!single && (mediaRequested || isVideoSite(single)));

  const toolNames: Record<string, string> = { "yt-dlp": "video reader", ffmpeg: "video merger", Deno: "JavaScript engine" };

  async function inspect() {
    const url = single;
    const seq = ++probeSeq;
    info = null;
    media = null;
    error = null;
    if (!url) return;
    loading = true;
    try {
      if (isMedia) {
        const m = await api.probeMedia(url, headers);
        if (seq !== probeSeq) return;
        media = m;
        option = (m.options.find((o) => o.id === "h1080") ?? m.options[0])?.id ?? "";
      } else {
        const i = await api.probeUrl(url, headers);
        if (seq !== probeSeq) return;
        info = i;
        if (!nameEdited) name = i.file_name;
      }
    } catch (e) {
      if (seq === probeSeq) error = String(e);
    } finally {
      if (seq === probeSeq) loading = false;
    }
  }

  let typingTimer = 0;
  function onUrlInput() {
    clearTimeout(typingTimer);
    typingTimer = window.setTimeout(inspect, 500);
  }

  async function paste() {
    try {
      const text = await readText();
      if (text && extractUrls(text).length) {
        urlText = text.trim();
        inspect();
      }
    } catch {
    }
  }

  async function pickDir() {
    const picked = await openDialog({ directory: true, defaultPath: dir || undefined });
    if (typeof picked === "string") {
      dir = picked;
      dirEdited = true;
    }
  }

  async function submit(start: boolean, asFile = false) {
    if (!urls.length) return;
    busy = true;
    try {
      let firstId: string | null = null;
      for (const url of urls) {
        const d = await api.add({
          url,
          headers,
          dir: dirEdited && dir ? dir : null,
          file_name: single && name.trim() && (!isMedia || asFile) ? name.trim() : null,
          paused: !start,
          queue,
          connections: conns ? Number(conns) : null,
          username: user || null,
          password: pass || null,
          media_option: isMedia && !asFile ? option : null,
          title: draftTitle ?? media?.title ?? null,
        });
        firstId ??= d.id;
      }
      if (start && firstId && urls.length === 1 && (await getPrefs()).progress_window) {
        await api.openProgressWindow(firstId);
      }
      await win.close();
    } catch (e) {
      error = String(e);
      busy = false;
    }
  }

  onMount(() => {
    let unlisten: (() => void) | undefined;
    (async () => {
      unlisten = await api.onEvent((ev) => {
        if (ev.type === "tool") tool = ev.data.done ? null : ev.data;
      });
      const [draft, config] = await Promise.all([api.takeAddDraft(), api.getConfig()]);
      dir = config.download_dir;
      queues = config.queues;
      if (draft?.url) {
        urlText = draft.url;
        headers = draft.headers;
        mediaRequested = draft.media;
        if (draft.media && draft.file_name) draftTitle = draft.file_name;
        if (draft.file_name) {
          name = draft.file_name;
          nameEdited = true;
        }
        inspect();
      } else {
        await paste();
      }
    })();
    return () => unlisten?.();
  });

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") win.close();
    else if (e.key === "Enter" && !e.shiftKey && (e.target as HTMLElement)?.tagName !== "TEXTAREA") submit(true);
  }
</script>

<svelte:window onkeydown={onKey} />

<div class="window">
  <TitleBar title={isMedia ? t("Download video") : t("Add download")} />

  <main use:fitWindow={{ width: 600, max: 720 }}>
    <div class="body">
    <div class="field-row">
      <textarea
        rows={urls.length > 1 ? 3 : 1}
        bind:value={urlText}
        oninput={onUrlInput}
        placeholder={t("https://…  (one link per line for several)")}
        spellcheck="false"
      ></textarea>
      <button class="icon-btn boxed" title={t("Paste from clipboard")} aria-label={t("Paste from clipboard")} onclick={paste}><Icon name="clipboard" /></button>
    </div>

    {#if media}
      <div class="media-head">
        {#if media.thumbnail}<img class="thumb" src={media.thumbnail} alt="" />{/if}
        <div class="media-meta">
          <b class="media-title" title={draftTitle ?? media.title}>{draftTitle ?? media.title}</b>
          <span class="muted">{[media.uploader, formatDuration(media.duration), media.site].filter(Boolean).join(" · ")}</span>
        </div>
      </div>
      <div class="qualities" role="radiogroup" aria-label={t("Quality")}>
        {#each media.options as o (o.id)}
          <label class="quality" class:on={option === o.id}>
            <input type="radio" name="quality" value={o.id} bind:group={option} />
            <span>{o.label}</span>
            <span class="muted">{o.size ? `~${formatBytes(o.size)}` : ""}</span>
          </label>
        {/each}
      </div>
    {/if}

    <div class="field-row">
      <input bind:value={dir} oninput={() => (dirEdited = true)} placeholder={t("Save to folder")} />
      <button class="icon-btn boxed" title={t("Choose folder")} aria-label={t("Choose folder")} onclick={pickDir}><Icon name="folder" /></button>
    </div>

    {#if !media}
      <div class="field-row">
        {#if urls.length > 1}
          <span class="muted grow">{t("{n} links will be added", { n: urls.length })}</span>
        {:else}
          <input class="grow" bind:value={name} oninput={() => (nameEdited = true)} placeholder={t("File name")} />
        {/if}
        <div class="size-box" title={info?.resumable ? t("Can resume where it left off") : ""}>
          {#if loading}
            <span class="spinner"></span>
          {:else if info}
            <b>{formatBytes(info.size)}</b>
            <span class={info.resumable ? "ok" : "warn"}>{info.resumable ? t("✓ resumable") : t("not resumable")}</span>
          {/if}
        </div>
        <button class="icon-btn boxed" title={t("Check again")} aria-label={t("Check again")} onclick={inspect}><Icon name="retry" /></button>
        <button class="icon-btn boxed" class:active={showMore} title={t("Advanced")} aria-label={t("Advanced")} onclick={() => (showMore = !showMore)}><Icon name="settings" /></button>
      </div>
    {/if}

    {#if showMore && !media}
      <div class="more">
        <input bind:value={user} placeholder={t("User name")} autocomplete="off" />
        <input type="password" bind:value={pass} placeholder={t("Password")} autocomplete="off" />
        <select bind:value={conns} title={t("Connections")}>
          <option value="">{t("Connections: default")}</option>
          {#each [1, 2, 4, 8, 16, 32] as n (n)}<option value={String(n)}>{t(n === 1 ? "{n} connection" : "{n} connections", { n })}</option>{/each}
        </select>
        <select bind:value={queue} title={t("Queue")}>
          {#each queues as q (q.id)}<option value={q.id}>{queueLabel(q)}</option>{/each}
        </select>
      </div>
    {/if}

    {#if isMedia && !media && loading}
      <div class="status-line">
        <span class="spinner"></span>
        {#if tool}
          {t("First use: downloading the {tool}…", { tool: t(toolNames[tool.name] ?? tool.name) })} {tool.total ? t("{n}%", { n: Math.floor((tool.downloaded / tool.total) * 100) }) : formatBytes(tool.downloaded)}
        {:else}
          {t("Getting video details…")}
        {/if}
      </div>
    {/if}
    {#if error}<p class="error wrap">{error}</p>{/if}
    {#if info?.html}<p class="warn small">{t("This link returns a web page; the page will be downloaded, not a file.")}</p>{/if}

    <div class="actions">
      <button class="btn" disabled={busy || !urls.length || (isMedia && !media && !error)} onclick={() => submit(false, !!error)} title={t("Add to the list without starting")}>{t("Add")}</button>
      <button class="btn primary" disabled={busy || !urls.length || (isMedia && !media && !error)} onclick={() => submit(true, !!error)}>
        {error && isMedia ? t("Download as a file") : t("Download")}
      </button>
      <span class="spacer"></span>
      <button class="btn" onclick={() => win.close()}>{t("Cancel")}</button>
    </div>
    </div>
  </main>
</div>

<style>
  .window {
    height: 100vh;
    display: flex;
    flex-direction: column;
    background: var(--panel);
    border: 1px solid var(--border);
  }
  .body {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  main {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 6px 16px 14px;
    min-height: 0;
    overflow-y: auto;
    scrollbar-width: none;
  }
  .field-row {
    display: flex;
    gap: 6px;
    align-items: stretch;
  }
  .field-row > input,
  .field-row > textarea,
  .grow {
    flex: 1;
    min-width: 0;
  }
  .grow.muted {
    align-self: center;
  }
  textarea {
    resize: none;
  }
  .size-box {
    display: flex;
    flex-direction: column;
    justify-content: center;
    align-items: flex-end;
    min-width: 96px;
    font-size: 11.5px;
    font-variant-numeric: tabular-nums;
  }
  .size-box b {
    font-size: 12.5px;
  }
  .ok {
    color: var(--ok);
  }
  .warn {
    color: var(--warn);
  }
  .icon-btn.active {
    color: var(--accent);
    border-color: var(--accent);
  }
  .more {
    display: grid;
    grid-template-columns: 1fr 1fr 1fr 1fr;
    gap: 6px;
  }
  .media-head {
    display: flex;
    gap: 12px;
    align-items: center;
  }
  .thumb {
    width: 150px;
    aspect-ratio: 16 / 9;
    object-fit: cover;
    border-radius: 8px;
    background: var(--panel-2);
    flex: none;
  }
  .media-meta {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }
  .media-title {
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
    user-select: text;
  }
  .qualities {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 6px;
    overflow-y: auto;
    min-height: 0;
  }
  .quality {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--panel-2);
    cursor: pointer;
  }
  .quality.on {
    border-color: var(--accent);
    background: var(--accent-soft);
  }
  .quality input {
    accent-color: var(--accent);
    margin: 0;
  }
  .quality .muted {
    margin-left: auto;
    font-size: 12px;
  }
  .status-line {
    display: flex;
    align-items: center;
    gap: 10px;
    color: var(--muted);
  }
  .spinner {
    width: 16px;
    height: 16px;
    border: 2px solid var(--border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
    flex: none;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .error {
    color: var(--danger);
    margin: 0;
  }
  .actions {
    display: flex;
    gap: 8px;
    margin-top: 4px;
  }
</style>
