<script lang="ts">
  import { onMount } from "svelte";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { getVersion } from "@tauri-apps/api/app";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { slide } from "svelte/transition";
  import { disable as disableAutostart, enable as enableAutostart, isEnabled as autostartEnabled } from "@tauri-apps/plugin-autostart";
  import Icon from "$lib/Icon.svelte";
  import { LANGUAGES, locale, queueLabel, t } from "$lib/i18n.svelte";
  import { applyPrefs, followPrefs, setPrefs, type Prefs } from "$lib/prefs";
  import {
    api,
    extractUrls,
    formatBytes,
    formatEta,
    formatSpeed,
    MB,
    type AfterAll,
    type Category,
    type Config,
    type Download,
    type QueueConfig,
    type Status,
    type UpdateInfo,
  } from "$lib/api";

  type SortKey = "name" | "size" | "status" | "speed" | "eta" | "added";
  type SettingsTab = "appearance" | "download" | "folders" | "network" | "windows" | "sites" | "browser";
  const SETTINGS_TABS: { id: SettingsTab; label: string; icon: string }[] = [
    { id: "appearance", label: "Appearance", icon: "image" },
    { id: "download", label: "Downloads", icon: "speed" },
    { id: "folders", label: "Folders", icon: "folder" },
    { id: "network", label: "Connection", icon: "link" },
    { id: "windows", label: "Windows", icon: "list" },
    { id: "sites", label: "Site rules", icon: "shield" },
    { id: "browser", label: "Browser", icon: "open" },
  ];

  let downloads = $state<Record<string, Download>>({});
  let config = $state<Config | null>(null);
  type Group = "all" | "finished" | "unfinished";
  let group = $state<Group>("all");
  let cat = $state<Category | null>(null);
  let queueSel = $state<string | null>(null);
  let openGroup = $state<Group | null>("all");
  let queuesOpen = $state(true);
  let search = $state("");
  let checked = $state<Set<string>>(new Set());
  let focused = $state<string | null>(null);
  let sortKey = $state<SortKey>("added");
  let sortAsc = $state(false);
  let toast = $state<{ msg: string; kind: "error" | "info" } | null>(null);
  let now = $state(Date.now() / 1000);
  let afterAll = $state<AfterAll>("nothing");
  let afterBanner = $state<AfterAll | null>(null);

  let settingsOpen = $state(false);
  let settingsTab = $state<SettingsTab>("appearance");
  let prefs = $state<Prefs | null>(null);
  let prefsDraft = $state<Prefs | null>(null);
  let draft = $state<Config | null>(null);
  let limitMb = $state(0);
  let minSplitKb = $state(1024);
  let extensionPath = $state("");
  let autostart = $state(true);

  let queueDraft = $state<QueueConfig | null>(null);
  let queueIsNew = $state(false);

  let removeIds = $state<string[] | null>(null);
  let removeFiles = $state(false);

  const appWindow = getCurrentWindow();
  let maximized = $state(false);
  let openMenu = $state<string | null>(null);
  let aboutOpen = $state(false);
  let appVersion = $state("");
  let update = $state<UpdateInfo | null>(null);
  let updateDismissed = $state(false);
  let installing = $state(false);
  let checking = $state(false);

  async function checkUpdates() {
    checking = true;
    try {
      update = await api.checkForUpdates();
      updateDismissed = false;
      if (!update) flash(t("Pulse DM is up to date ({version})", { version: appVersion }), "info");
    } catch (e) {
      flash(t("Could not check for updates: {error}", { error: String(e) }));
    } finally {
      checking = false;
    }
  }

  async function installUpdate() {
    installing = true;
    try {
      await api.installUpdate();
    } catch (e) {
      installing = false;
      flash(t("Could not install the update: {error}", { error: String(e) }));
    }
  }

  type MenuItem =
    | { label: string; kbd?: string; action: () => void; disabled?: () => boolean; checked?: () => boolean }
    | "-";

  function setAfterAll(mode: AfterAll) {
    afterAll = mode;
    run(api.setAfterAll(mode));
  }
  const MENUS: { id: string; label: string; items: MenuItem[] }[] = [
    {
      id: "file",
      label: "File",
      items: [
        { label: "Add link", kbd: "Ctrl+N", action: () => openAdd() },
        "-",
        { label: "Close window", action: () => appWindow.close() },
        { label: "Exit", action: () => run(api.quit()) },
      ],
    },
    {
      id: "tasks",
      label: "Tasks",
      items: [
        { label: "Resume selected", action: () => forSelection(api.resume), disabled: () => !selection.length },
        { label: "Pause selected", action: () => forSelection(api.pause), disabled: () => !selection.length },
        { label: "Delete selected", kbd: "Del", action: () => askRemove(selection), disabled: () => !selection.length },
        "-",
        { label: "Start all", action: () => run(api.resumeAll()) },
        { label: "Stop all", action: () => run(api.pauseAll()) },
        { label: "Clear finished", action: () => run(api.clearCompleted()) },
        "-",
        { label: "When done, do nothing", action: () => setAfterAll("nothing"), checked: () => afterAll === "nothing" },
        { label: "When done, shut down", action: () => setAfterAll("shutdown"), checked: () => afterAll === "shutdown" },
        { label: "When done, sleep", action: () => setAfterAll("sleep"), checked: () => afterAll === "sleep" },
      ],
    },
    {
      id: "tools",
      label: "Tools",
      items: [
        { label: "Settings", action: () => openSettings() },
        { label: "New queue", action: () => openQueue() },
        { label: "Site rules", action: () => openSettings("sites") },
        "-",
        { label: "Browser extension", action: () => openSettings("browser") },
      ],
    },
    {
      id: "help",
      label: "Help",
      items: [
        { label: "Check for updates", action: checkUpdates, disabled: () => checking || installing },
        "-",
        { label: "About Pulse DM", action: () => (aboutOpen = true) },
      ],
    },
  ];

  const ACTIVE: Status[] = ["queued", "connecting", "downloading", "processing"];
  const DAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

  const CATEGORIES: { id: Category; label: string }[] = [
    { id: "archive", label: "Archives" },
    { id: "program", label: "Programs" },
    { id: "video", label: "Videos" },
    { id: "audio", label: "Music" },
    { id: "image", label: "Images" },
    { id: "document", label: "Documents" },
    { id: "other", label: "Other" },
  ];
  const GROUPS: { id: Group; label: string; icon: string }[] = [
    { id: "all", label: "All", icon: "all" },
    { id: "finished", label: "Finished", icon: "finished" },
    { id: "unfinished", label: "Unfinished", icon: "unfinished" },
  ];
  const CATEGORY_LABEL = Object.fromEntries(CATEGORIES.map((c) => [c.id, c.label])) as Record<Category, string>;

  const STATUS_LABEL: Record<Status, string> = {
    queued: "Queued",
    connecting: "Connecting",
    downloading: "Downloading",
    paused: "Paused",
    completed: "Completed",
    failed: "Error",
    processing: "Processing",
  };

  const queues = $derived(config?.queues ?? []);
  const queueName = (id: string) => {
    const q = queues.find((q) => q.id === id);
    return q ? queueLabel(q) : t("Main queue");
  };

  function inView(d: Download): boolean {
    if (queueSel) return d.queue === queueSel && d.status !== "completed";
    if (group === "finished" && d.status !== "completed") return false;
    if (group === "unfinished" && d.status === "completed") return false;
    return !cat || d.category === cat;
  }

  function selectGroup(g: Group) {
    group = g;
    cat = null;
    queueSel = null;
    openGroup = g;
  }

  function selectCategory(g: Group, c: Category) {
    group = g;
    cat = c;
    queueSel = null;
  }

  function selectQueue(id: string) {
    queueSel = id;
    cat = null;
  }

  function sortValue(d: Download, key: SortKey): number | string {
    switch (key) {
      case "name": return d.file_name.toLowerCase();
      case "size": return d.size ?? -1;
      case "status": return progress(d);
      case "speed": return d.speed;
      case "eta": return d.eta_secs ?? Number.MAX_SAFE_INTEGER;
      case "added": return d.added_at;
    }
  }

  const all = $derived(Object.values(downloads));
  const queueCount = (id: string) => all.filter((d) => d.queue === id && d.status !== "completed").length;
  const anyFinished = $derived(all.some((d) => d.status === "completed"));

  const visible = $derived.by(() => {
    const q = search.trim().toLowerCase();
    const rows = all.filter((d) => inView(d) && (!q || d.file_name.toLowerCase().includes(q)));
    const dir = sortAsc ? 1 : -1;
    return rows.sort((a, b) => {
      const x = sortValue(a, sortKey);
      const y = sortValue(b, sortKey);
      return (x < y ? -1 : x > y ? 1 : 0) * dir || b.added_at - a.added_at;
    });
  });

  const totalSpeed = $derived(all.reduce((s, d) => s + (d.status === "downloading" ? d.speed : 0), 0));
  const selection = $derived(checked.size ? [...checked].filter((id) => downloads[id]) : focused ? [focused] : []);
  const allChecked = $derived(visible.length > 0 && visible.every((d) => checked.has(d.id)));

  function progress(d: Download): number {
    if (d.status === "completed") return 100;
    if (d.parts) return d.parts[1] ? (d.parts[0] / d.parts[1]) * 100 : 0;
    if (!d.size) return 0;
    return Math.min(100, (d.downloaded / d.size) * 100);
  }

  function ago(ts: number): string {
    if (prefs && !prefs.relative_dates) {
      return new Date(ts * 1000).toLocaleString(locale(), { dateStyle: "short", timeStyle: "short" });
    }
    const s = Math.max(0, Math.floor(now - ts));
    if (s < 60) return t("just now");
    if (s < 3600) return t("{n} min ago", { n: Math.floor(s / 60) });
    if (s < 86400) return t("{n} h ago", { n: Math.floor(s / 3600) });
    return t("{n} d ago", { n: Math.floor(s / 86400) });
  }

  function setSort(key: SortKey) {
    if (sortKey === key) sortAsc = !sortAsc;
    else {
      sortKey = key;
      sortAsc = key === "name";
    }
  }

  function openDetails(id: string) {
    run(api.openProgressWindow(id));
  }

  function toggleCheck(id: string) {
    const next = new Set(checked);
    next.has(id) ? next.delete(id) : next.add(id);
    checked = next;
  }

  function toggleAll() {
    checked = allChecked ? new Set() : new Set(visible.map((d) => d.id));
  }

  function flash(msg: string, kind: "error" | "info" = "error") {
    toast = { msg, kind };
    setTimeout(() => (toast = null), 4500);
  }

  async function run(p: Promise<unknown>, ok?: string) {
    try {
      await p;
      if (ok) flash(ok, "info");
    } catch (e) {
      flash(String(e));
    }
  }

  async function forSelection(fn: (id: string) => Promise<void>) {
    await Promise.all(selection.map((id) => fn(id).catch((e) => flash(String(e)))));
  }

  function openAdd(prefill = "") {
    run(api.openAddWindow({ url: prefill }));
  }

  async function pickDir(current: string): Promise<string> {
    const dir = await openDialog({ directory: true, defaultPath: current || undefined });
    return typeof dir === "string" ? dir : current;
  }

  function openSettings(tab: SettingsTab = "appearance") {
    if (!config || !prefs) return;
    draft = JSON.parse(JSON.stringify(config));
    prefsDraft = { ...prefs };
    limitMb = config.speed_limit / MB;
    minSplitKb = Math.round(config.min_split / 1024);
    settingsTab = tab;
    autostartEnabled().then((on) => (autostart = on), () => {});
    settingsOpen = true;
  }

  function closeSettings() {
    settingsOpen = false;
    if (prefs) applyPrefs(prefs);
  }

  function previewPrefs() {
    if (prefsDraft) applyPrefs(prefsDraft);
  }

  async function saveSettings() {
    if (!draft) return;
    draft.speed_limit = Math.max(0, Math.round(limitMb * MB));
    draft.min_split = Math.max(64, minSplitKb) * 1024;
    draft.host_rules = draft.host_rules.filter((r) => r.host.trim());
    try {
      config = await api.setConfig(draft);
      if (prefsDraft) prefs = await setPrefs(prefsDraft);
      settingsOpen = false;
    } catch (e) {
      flash(String(e));
    }
  }

  function openQueue(q?: QueueConfig) {
    queueIsNew = !q;
    queueDraft = q
      ? JSON.parse(JSON.stringify(q))
      : { id: `q${Date.now().toString(36)}`, name: "", max_active: 2, running: false, schedule: { enabled: false, days: [], start: "02:00", stop: "07:00" } };
  }

  async function saveQueue() {
    if (!config || !queueDraft) return;
    if (!queueDraft.name.trim()) return flash(t("Give the queue a name"));
    const next: Config = JSON.parse(JSON.stringify(config));
    const i = next.queues.findIndex((q) => q.id === queueDraft!.id);
    if (i >= 0) next.queues[i] = { ...queueDraft, running: next.queues[i].running };
    else next.queues.push(queueDraft);
    try {
      config = await api.setConfig(next);
      queueDraft = null;
    } catch (e) {
      flash(String(e));
    }
  }

  async function deleteQueue(id: string) {
    if (!config || id === "main") return;
    const next: Config = JSON.parse(JSON.stringify(config));
    next.queues = next.queues.filter((q) => q.id !== id);
    try {
      config = await api.setConfig(next);
      queueDraft = null;
      if (queueSel === id) queueSel = null;
    } catch (e) {
      flash(String(e));
    }
  }

  function toggleDay(day: number) {
    if (!queueDraft) return;
    const days = queueDraft.schedule.days;
    queueDraft.schedule.days = days.includes(day) ? days.filter((d) => d !== day) : [...days, day].sort();
  }

  function askRemove(ids: string[]) {
    if (!ids.length) return;
    removeFiles = false;
    removeIds = ids;
  }

  async function confirmRemove() {
    const ids = removeIds ?? [];
    removeIds = null;
    await Promise.all(ids.map((id) => api.remove(id, removeFiles).catch((e) => flash(String(e)))));
    checked = new Set([...checked].filter((id) => !ids.includes(id)));
    if (focused && ids.includes(focused)) focused = null;
  }

  function onKey(e: KeyboardEvent) {
    const typing = (e.target as HTMLElement)?.closest("input, textarea, select");
    if (e.key === "Escape") {
      if (openMenu) openMenu = null;
      else if (settingsOpen || removeIds || queueDraft || aboutOpen) {
        if (settingsOpen) closeSettings();
        aboutOpen = false;
        removeIds = null;
        queueDraft = null;
      } else focused = null;
    } else if (!typing && e.key === "Enter" && focused && downloads[focused]) {
      openDetails(focused);
    } else if (!typing && e.key === "Delete") {
      askRemove(selection);
    } else if (!typing && (e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "n") {
      e.preventDefault();
      openAdd();
    } else if (!typing && (e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "a") {
      e.preventDefault();
      checked = new Set(visible.map((d) => d.id));
    }
  }

  function onPaste(e: ClipboardEvent) {
    if ((e.target as HTMLElement)?.closest("input, textarea")) return;
    const text = e.clipboardData?.getData("text") ?? "";
    if (extractUrls(text).length) {
      e.preventDefault();
      openAdd(text);
    }
  }

  onMount(() => {
    const unlisten: (() => void)[] = [];
    const clock = setInterval(() => (now = Date.now() / 1000), 30_000);
    (async () => {
      unlisten.push(
        await api.onEvent((ev) => {
          if (ev.type === "progress") for (const d of ev.data) downloads[d.id] = d;
          else if (ev.type === "changed") downloads[ev.data.id] = ev.data;
          else if (ev.type === "removed") delete downloads[ev.data];
          else if (ev.type === "config") config = ev.data;
        }),
        await api.on<Download>("browser-added", (d) => {
          downloads[d.id] = d;
          flash(t("Added from the browser: {name}", { name: d.file_name }), "info");
        }),
        await api.on<UpdateInfo>("update-available", (info) => {
          if (update?.version !== info.version) updateDismissed = false;
          update = info;
        }),
        await api.on<AfterAll>("after-all", (mode) => {
          afterAll = "nothing";
          afterBanner = mode;
        }),
      );
      const list = await api.list();
      downloads = Object.fromEntries(list.map((d) => [d.id, d]));
      config = await api.getConfig();
      afterAll = await api.getAfterAll();
      appVersion = await getVersion();
      update = await api.pendingUpdate();
      maximized = await appWindow.isMaximized();
      unlisten.push(await followPrefs((p) => (prefs = p)));
      unlisten.push(await appWindow.onResized(async () => (maximized = await appWindow.isMaximized())));
    })();
    return () => {
      unlisten.forEach((u) => u());
      clearInterval(clock);
    };
  });
</script>

<svelte:window
  onkeydown={onKey}
  onpaste={onPaste}
  onclick={(e) => openMenu && !(e.target as HTMLElement)?.closest(".menu") && (openMenu = null)}
/>

{#snippet icon(name: string, size = 16)}<Icon {name} {size} />{/snippet}

{#snippet rich(text: string)}{#each text.split(/(`[^`]+`|\*\*[^*]+\*\*|\[[^\]]+\])/) as part, i (i)}{#if part.startsWith("`")}<code>{part.slice(1, -1)}</code>{:else if part.startsWith("**")}<b>{part.slice(2, -2)}</b>{:else if part.startsWith("[")}<kbd>{part.slice(1, -1)}</kbd>{:else}{part}{/if}{/each}{/snippet}

{#snippet th(key: SortKey, label: string, cls = "")}
  <button class="th {cls}" class:sorted={sortKey === key} onclick={() => setSort(key)}>
    {t(label)}
    {#if sortKey === key}<span class="arrow" class:asc={sortAsc}>{@render icon("chevron", 12)}</span>{/if}
  </button>
{/snippet}

<div class="app">
  <header class="titlebar" data-tauri-drag-region>
    <div class="brand" data-tauri-drag-region>
      <svg width="18" height="18" viewBox="0 0 512 512" aria-hidden="true">
        <rect x="16" y="16" width="480" height="480" rx="112" fill="var(--accent)" />
        <path d="M80 236h76l40-92 56 196 40-104h140" fill="none" stroke="#fff" stroke-width="44" stroke-linecap="round" stroke-linejoin="round" />
      </svg>
      <span data-tauri-drag-region>Pulse DM</span>
    </div>
    <nav class="menubar">
      {#each MENUS as m (m.id)}
        <div class="menu">
          <button class="menu-btn" class:open={openMenu === m.id}
            onclick={() => (openMenu = openMenu === m.id ? null : m.id)}
            onmouseenter={() => openMenu && (openMenu = m.id)}>{t(m.label)}</button>
          {#if openMenu === m.id}
            <div class="dropdown" role="menu">
              {#each m.items as item, i (i)}
                {#if item === "-"}<div class="sep"></div>
                {:else}
                  <button role="menuitem" disabled={item.disabled?.()} onclick={() => { openMenu = null; item.action(); }}>
                    <span><span class="check-mark">{item.checked?.() ? "✓" : ""}</span>{t(item.label)}</span>{#if item.kbd}<span class="kbd-hint">{item.kbd}</span>{/if}
                  </button>
                {/if}
              {/each}
            </div>
          {/if}
        </div>
      {/each}
    </nav>
    <div class="drag-fill" data-tauri-drag-region></div>
    <label class="search">
      {@render icon("search", 14)}
      <input placeholder={t("Search the list")} bind:value={search} />
    </label>
    <div class="win-controls">
      <button aria-label={t("Minimize")} onclick={() => appWindow.minimize()}>
        <svg width="10" height="10" viewBox="0 0 10 10"><path d="M0 5h10" stroke="currentColor" /></svg>
      </button>
      <button aria-label={maximized ? t("Restore") : t("Maximize")} onclick={() => appWindow.toggleMaximize()}>
        {#if maximized}
          <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor"><rect x="0.5" y="2.5" width="7" height="7" /><path d="M2.5 2.5V0.5h7v7h-2" /></svg>
        {:else}
          <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor"><rect x="0.5" y="0.5" width="9" height="9" /></svg>
        {/if}
      </button>
      <button class="close" aria-label={t("Close (to tray)")} title={t("Close: downloads keep running in the tray")} onclick={() => appWindow.close()}>
        <svg width="10" height="10" viewBox="0 0 10 10"><path d="M0 0l10 10M10 0L0 10" stroke="currentColor" /></svg>
      </button>
    </div>
  </header>

  <aside class="side">
    <nav class="card">
      {#each GROUPS as g (g.id)}
        <div class="group">
          <button class="head" class:active={!queueSel && group === g.id && !cat} onclick={() => selectGroup(g.id)}>
            {@render icon(g.icon, 15)}
            <span class="label">{t(g.label)}</span>
            <span class="chev" class:open={openGroup === g.id} role="presentation"
              onclick={(e) => { e.stopPropagation(); openGroup = openGroup === g.id ? null : g.id; }}>{@render icon("chevron", 12)}</span>
          </button>
          {#if openGroup === g.id}
            <div class="items" transition:slide={{ duration: 160 }}>
              {#each CATEGORIES as c (c.id)}
                <button class="item" class:active={!queueSel && group === g.id && cat === c.id} onclick={() => selectCategory(g.id, c.id)}>
                  {@render icon(c.id, 14)} <span class="label">{t(c.label)}</span>
                </button>
              {/each}
            </div>
          {/if}
        </div>
      {/each}
    </nav>

    <nav class="card">
      <button class="head" onclick={() => (queuesOpen = !queuesOpen)}>
        {@render icon("queue", 15)}
        <span class="label">{t("Queues")}</span>
        <span class="chev" class:open={queuesOpen}>{@render icon("chevron", 12)}</span>
      </button>
      {#if queuesOpen}
        <div class="items" transition:slide={{ duration: 160 }}>
          {#each queues as q (q.id)}
            <div class="item queue" class:active={queueSel === q.id}>
              <button class="q-main" onclick={() => selectQueue(q.id)} title={q.running ? t("Running") : t("Stopped")}>
                <span class="dot" class:on={q.running}></span>
                <span class="label">{queueLabel(q)}</span>
                {#if q.schedule.enabled}<span class="sched" title={t("Scheduled")}>{@render icon("clock", 11)}</span>{/if}
              </button>
              <span class="q-count">{queueCount(q.id)}</span>
              <span class="q-actions">
                <button class="mini" title={q.running ? t("Stop queue") : t("Start queue")} aria-label={t("Start/stop")}
                  onclick={() => run(q.running ? api.stopQueue(q.id) : api.startQueue(q.id))}>{@render icon(q.running ? "stop" : "play", 11)}</button>
                <button class="mini" title={t("Edit")} aria-label={t("Edit")} onclick={() => openQueue(q)}>{@render icon("edit", 11)}</button>
              </span>
            </div>
          {/each}
          <button class="item add-queue" onclick={() => openQueue()}>{@render icon("plus", 13)} <span class="label">{t("New queue")}</span></button>
        </div>
      {/if}
    </nav>
  </aside>

  <section class="content">
    <div class="toolbar" class:no-labels={prefs && !prefs.tool_labels}>
      <button class="add-btn" onclick={() => openAdd()} title={t("New download (Ctrl+N)")}>
        {@render icon("link")} {t("Add link")}
      </button>
      <span class="divider"></span>
      <button class="tool" disabled={!selection.length} onclick={() => forSelection(api.resume)}>{@render icon("play", 16)}<span>{t("Resume")}</span></button>
      <button class="tool" disabled={!selection.length} onclick={() => forSelection(api.pause)}>{@render icon("pause", 16)}<span>{t("Pause")}</span></button>
      <button class="tool" disabled={!selection.length} onclick={() => askRemove(selection)}>{@render icon("trash", 16)}<span>{t("Delete")}</span></button>
      <span class="divider"></span>
      <button class="tool" onclick={() => run(api.resumeAll())}>{@render icon("play", 16)}<span>{t("Start all")}</span></button>
      <button class="tool" onclick={() => run(api.pauseAll())}>{@render icon("stop", 16)}<span>{t("Stop all")}</span></button>
      <button class="tool" disabled={!anyFinished} onclick={() => run(api.clearCompleted())}>{@render icon("broom", 16)}<span>{t("Clear")}</span></button>
      <button class="tool" onclick={() => openSettings()}>{@render icon("settings", 16)}<span>{t("Settings")}</span></button>
    </div>

    {#if update && !updateDismissed}
      <div class="banner update">
        <span>{t("New version available:")} <b>Pulse DM {update.version}</b>{#if appVersion} <span class="muted">{t("(you have {version})", { version: appVersion })}</span>{/if}</span>
        <span class="banner-actions">
          <button class="btn" disabled={installing} onclick={() => (updateDismissed = true)}>{t("Later")}</button>
          <button class="btn primary" disabled={installing} onclick={installUpdate}>{installing ? t("Installing…") : t("Update and restart")}</button>
        </span>
      </div>
    {/if}
    {#if afterBanner === "shutdown"}
      <div class="banner">
        {t("All downloads finished. The computer will shut down in 60 seconds.")}
        <button class="btn" onclick={() => { run(api.cancelShutdown(), t("Shutdown cancelled")); afterBanner = null; }}>{t("Cancel")}</button>
      </div>
    {/if}

    <div class="table">
      <div class="thead">
        <label class="cell check"><input type="checkbox" checked={allChecked} onchange={toggleAll} aria-label={t("Select all")} /></label>
        {@render th("name", "Name")}
        {@render th("size", "Size", "num")}
        {@render th("status", "Status")}
        {@render th("speed", "Speed", "num")}
        {@render th("eta", "Time left", "num")}
        {@render th("added", "Added")}
      </div>

      <div class="tbody">
        {#if visible.length === 0}
          <div class="empty">
            {@render icon("speed", 34)}
            <h2>{all.length ? t("No downloads in this view") : t("No downloads yet")}</h2>
            <p>{@render rich(t("Copy a link and paste it with [Ctrl]+[V], or press **Add link**."))}</p>
            <button class="btn" onclick={() => openSettings("browser")}>{t("Install the browser extension")}</button>
          </div>
        {:else}
          {#each visible as d (d.id)}
            <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
            <div
              class="tr"
              class:focused={focused === d.id}
              class:checked={checked.has(d.id)}
              role="row"
              tabindex="-1"
              onclick={() => (focused = d.id)}
              ondblclick={() => openDetails(d.id)}
            >
              <label class="cell check" onclick={(e) => e.stopPropagation()}>
                <input type="checkbox" checked={checked.has(d.id)} onchange={() => toggleCheck(d.id)} aria-label={t("Select")} />
              </label>
              <div class="cell name">
                <span class="ficon">{@render icon(d.category, 15)}</span>
                <div class="name-text">
                  <span class="fname" title={d.file_name}>{d.file_name}</span>
                  <span class="fcat">{t(CATEGORY_LABEL[d.category])}{#if d.queue !== "main"} · {queueName(d.queue)}{/if}</span>
                </div>
              </div>
              <div class="cell num">{d.parts && d.status !== "completed" ? `${formatBytes(d.downloaded)}+` : formatBytes(d.size)}</div>
              <div class="cell status">
                {#if d.status === "completed"}
                  <span class="st-completed">{t("Completed")}</span>
                {:else if d.status === "failed"}
                  <span class="st-failed" title={d.error ?? ""}>{t("Error")}</span>
                {:else}
                  <span class="st-{d.status}">
                    {#if (d.size || d.parts) && d.downloaded > 0}{t("{n}%", { n: Math.floor(progress(d)) })} {/if}{t(STATUS_LABEL[d.status])}
                  </span>
                  <div class="mini-bar" class:indeterminate={!d.size && !d.parts && d.status === "downloading"}>
                    <div class="mini-fill st-bg-{d.status}" style="width:{progress(d)}%"></div>
                  </div>
                {/if}
              </div>
              <div class="cell num">{d.status === "downloading" ? formatSpeed(d.speed) : ""}</div>
              <div class="cell num">{d.status === "downloading" ? formatEta(d.eta_secs) : ""}</div>
              <div class="cell muted">{ago(d.added_at)}</div>
            </div>
          {/each}
        {/if}
      </div>
    </div>

    <footer class="statusbar">
      {#if afterAll !== "nothing"}
        <button class="after-chip" title={t("Change it from the Tasks menu")} onclick={() => setAfterAll("nothing")}>
          {@render icon("power", 13)} {afterAll === "shutdown" ? t("Shuts down when done") : t("Sleeps when done")} ✕
        </button>
      {/if}
      <span>{@render icon("list", 14)} {all.length}</span>
      <span>{@render icon("speed", 14)} {formatSpeed(totalSpeed)}</span>
    </footer>
  </section>

</div>

{#if settingsOpen && draft && prefsDraft}
  <div class="overlay" role="presentation" onclick={(e) => e.target === e.currentTarget && closeSettings()}>
    <form class="modal settings" onsubmit={(e) => { e.preventDefault(); saveSettings(); }}>
      <div class="modal-title">{@render icon("settings")} {t("Settings")}</div>
      <div class="settings-body">
        <nav class="settings-nav">
          {#each SETTINGS_TABS as tab (tab.id)}
            <button type="button" class:active={settingsTab === tab.id} onclick={() => (settingsTab = tab.id)}>
              {@render icon(tab.icon, 15)} {t(tab.label)}
            </button>
          {/each}
        </nav>

        <div class="settings-pane">
          {#if settingsTab === "appearance"}
            <div class="srow">
              <div><b>{t("Language")}</b><span>{t("System follows the Windows display language.")}</span></div>
              <select bind:value={prefsDraft.language} onchange={previewPrefs}>
                {#each LANGUAGES as lang (lang.id)}<option value={lang.id}>{lang.id === "system" ? t(lang.label) : lang.label}</option>{/each}
              </select>
            </div>
            <div class="srow">
              <div><b>{t("Theme")}</b><span>{t("System follows the Windows light/dark setting.")}</span></div>
              <div class="seg-control">
                {#each [["system", "System"], ["dark", "Dark"], ["light", "Light"]] as [v, l] (v)}
                  <button type="button" class:on={prefsDraft.theme === v} onclick={() => { if (prefsDraft) { prefsDraft.theme = v as Prefs["theme"]; previewPrefs(); } }}>{t(l)}</button>
                {/each}
              </div>
            </div>
            <div class="srow">
              <div><b>{t("Interface scale")}</b><span>{t("Size of text and buttons.")}</span></div>
              <select bind:value={prefsDraft.ui_scale} onchange={previewPrefs}>
                {#each [0.9, 1, 1.1, 1.25] as v (v)}<option value={v}>{t("{n}%", { n: Math.round(v * 100) })}</option>{/each}
              </select>
            </div>
            <label class="srow">
              <div><b>{t("Relative dates")}</b><span>{t("Like \"5 min ago\"; when off, the full date and time.")}</span></div>
              <input type="checkbox" bind:checked={prefsDraft.relative_dates} />
            </label>
            <label class="srow">
              <div><b>{t("Toolbar labels")}</b><span>{t("Show names under the icons.")}</span></div>
              <input type="checkbox" bind:checked={prefsDraft.tool_labels} />
            </label>
          {:else if settingsTab === "download"}
            <div class="srow">
              <div><b>{t("Connections")}</b><span>{t("Per download. Lowered automatically if the server sets a limit.")}</span></div>
              <div class="range"><input type="range" min="1" max="32" bind:value={draft.connections} /><b>{draft.connections}</b></div>
            </div>
            <div class="srow">
              <div><b>{t("Simultaneous downloads")}</b><span>{t("How many downloads run together in the main queue.")}</span></div>
              <div class="range"><input type="range" min="1" max="10" bind:value={draft.queues[0].max_active} /><b>{draft.queues[0].max_active}</b></div>
            </div>
            <div class="srow">
              <div><b>{t("Global speed limit")}</b><span>{t("MB/s, 0 = unlimited.")}</span></div>
              <input class="num" type="number" min="0" step="0.5" bind:value={limitMb} />
            </div>
            <div class="srow">
              <div><b>{t("Smallest part")}</b><span>{t("KB. Ranges smaller than this are not split.")}</span></div>
              <input class="num" type="number" min="64" step="64" bind:value={minSplitKb} />
            </div>
            <div class="srow">
              <div><b>{t("Retries")}</b><span>{t("A failing connection is retried at most this many times.")}</span></div>
              <input class="num" type="number" min="0" max="50" bind:value={draft.max_retries} />
            </div>
            <label class="srow">
              <div><b>{t("Keep the server's file date")}</b><span>{t("Give the finished file the server's modification date instead of today.")}</span></div>
              <input type="checkbox" bind:checked={draft.keep_server_time} />
            </label>
            <label class="srow">
              <div><b>{t("Ignore SSL certificate errors")}</b><span class="warn-text">{t("Unsafe. Only turn on for sites you trust that have a broken certificate.")}</span></div>
              <input type="checkbox" bind:checked={draft.ignore_tls_errors} />
            </label>
          {:else if settingsTab === "folders"}
            <div class="srow column">
              <div><b>{t("Default download folder")}</b></div>
              <div class="field-row">
                <input bind:value={draft.download_dir} />
                <button type="button" class="icon-btn boxed" aria-label={t("Choose folder")} onclick={async () => draft && (draft.download_dir = await pickDir(draft.download_dir))}>{@render icon("folder")}</button>
              </div>
            </div>
            <label class="srow">
              <div><b>{t("Category folders")}</b><span>{@render rich(t("Videos, archives, programs… are saved into their own subfolder (`Videos`, `Compressed`, `Programs`…)."))}</span></div>
              <input type="checkbox" bind:checked={draft.category_folders} />
            </label>
          {:else if settingsTab === "network"}
            <div class="srow">
              <div><b>{t("Proxy")}</b><span>{t("System uses the Windows proxy setting.")}</span></div>
              <select bind:value={draft.proxy.mode}>
                <option value="system">{t("System settings")}</option>
                <option value="none">{t("No proxy")}</option>
                <option value="manual">{t("Manual")}</option>
              </select>
            </div>
            {#if draft.proxy.mode === "manual"}
              <div class="srow column">
                <div><b>{t("Proxy address")}</b></div>
                <input bind:value={draft.proxy.url} placeholder={t("http://127.0.0.1:8080  or  socks5://user:password@host:1080")} />
              </div>
            {/if}
            <div class="srow column">
              <div><b>User-Agent</b><span>{t("Some sites block non-browser programs; you can enter your browser's User-Agent.")}</span></div>
              <input bind:value={draft.user_agent} />
            </div>
          {:else if settingsTab === "windows"}
            <label class="srow">
              <div><b>{t("Progress window")}</b><span>{t("Open a separate progress window when a download starts.")}</span></div>
              <input type="checkbox" bind:checked={prefsDraft.progress_window} />
            </label>
            <label class="srow">
              <div><b>{t("Completion window")}</b><span>{t("Bring its window forward when a download finishes (Open / Open folder).")}</span></div>
              <input type="checkbox" bind:checked={prefsDraft.completion_window} />
            </label>
            <label class="srow">
              <div><b>{t("Notifications")}</b><span>{t("Show a Windows notification when a download finishes or fails.")}</span></div>
              <input type="checkbox" bind:checked={prefsDraft.notifications} />
            </label>
            <label class="srow">
              <div><b>{t("Check for updates automatically")}</b><span>{t("Tell me when a new version is out; it installs in one click.")}</span></div>
              <input type="checkbox" bind:checked={prefsDraft.auto_update} />
            </label>
            <label class="srow">
              <div><b>{t("Start with Windows")}</b><span>{t("Waits in the tray; downloads from the browser start without opening the program.")}</span></div>
              <input type="checkbox" bind:checked={autostart} onchange={() => run(autostart ? enableAutostart() : disableAutostart())} />
            </label>
          {:else if settingsTab === "sites"}
            <p class="muted small">{t("Connection count and speed limit for a specific site. Subdomains are included.")}</p>
            <div class="rules">
              {#each draft.host_rules as rule, i (i)}
                <div class="rule">
                  <input bind:value={rule.host} placeholder={t("example.com")} />
                  <select value={rule.connections == null ? "" : String(rule.connections)} onchange={(e) => { const v = (e.target as HTMLSelectElement).value; rule.connections = v ? Number(v) : null; }}>
                    <option value="">{t("Connections: default")}</option>
                    {#each [1, 2, 4, 8, 16, 32] as n (n)}<option value={String(n)}>{t(n === 1 ? "{n} connection" : "{n} connections", { n })}</option>{/each}
                  </select>
                  <input type="number" min="0" step="0.5" value={rule.speed_limit / MB} onchange={(e) => (rule.speed_limit = Math.round(Number((e.target as HTMLInputElement).value) * MB))} title={t("Speed limit (MB/s)")} placeholder="MB/s" />
                  <button type="button" class="icon-btn" aria-label={t("Delete rule")} onclick={() => draft && draft.host_rules.splice(i, 1)}>{@render icon("trash")}</button>
                </div>
              {/each}
            </div>
            <div><button type="button" class="btn" onclick={() => draft?.host_rules.push({ host: "", connections: null, speed_limit: 0, user_agent: null })}>{@render icon("plus")} {t("Add rule")}</button></div>
          {:else}
            <p>{t("Install the extension so browser downloads are passed to Pulse DM:")}</p>
            <ol class="steps">
              <li>{@render rich(t("Open `chrome://extensions` in Chrome or `edge://extensions` in Edge."))}</li>
              <li>{@render rich(t("Turn on **Developer mode** in the top right."))}</li>
              <li>{@render rich(t("Press **Load unpacked** and choose the folder below."))}</li>
            </ol>
            <div>
              <button type="button" class="btn primary" onclick={async () => { try { extensionPath = await api.openExtensionFolder(); } catch (e) { flash(String(e)); } }}>
                {@render icon("folder")} {t("Open the extension folder")}
              </button>
            </div>
            {#if extensionPath}<code class="wrap">{extensionPath}</code>{/if}
            <p class="muted small">{t("The extension passes downloads on with the browser's cookies, so sites that need a sign-in work too.")}</p>
          {/if}
        </div>
      </div>

      <div class="modal-actions">
        <span class="spacer"></span>
        <button type="button" class="btn" onclick={closeSettings}>{t("Cancel")}</button>
        <button type="submit" class="btn primary">{t("Save")}</button>
      </div>
    </form>
  </div>
{/if}

{#if queueDraft}
  <div class="overlay" role="presentation" onclick={(e) => e.target === e.currentTarget && (queueDraft = null)}>
    <form class="modal" onsubmit={(e) => { e.preventDefault(); saveQueue(); }}>
      <div class="modal-title">{@render icon("queue")} {queueIsNew ? t("New queue") : t("Edit queue")}</div>
      <div class="grid2">
        <label>{t("Name")}<input bind:value={queueDraft.name} placeholder={t("Night downloads")} disabled={queueDraft.id === "main"} /></label>
        <label>{t("Simultaneous downloads")}<input type="number" min="1" max="20" bind:value={queueDraft.max_active} /></label>
      </div>
      <label class="check-row"><input type="checkbox" bind:checked={queueDraft.schedule.enabled} /> {t("Use the scheduler")}</label>
      {#if queueDraft.schedule.enabled}
        <div class="days">
          {#each DAYS as day, i (i)}
            <button type="button" class="day" class:on={queueDraft.schedule.days.includes(i)} onclick={() => toggleDay(i)}>{t(day)}</button>
          {/each}
        </div>
        <span class="muted small">{t("With no day selected it runs every day.")}</span>
        <div class="grid2">
          <label>{t("Start time")}<input type="time" bind:value={queueDraft.schedule.start} /></label>
          <label>{t("Stop time")}<input type="time" bind:value={queueDraft.schedule.stop} /></label>
        </div>
      {/if}
      <div class="modal-actions">
        {#if !queueIsNew && queueDraft.id !== "main"}
          <button type="button" class="btn danger" onclick={() => queueDraft && deleteQueue(queueDraft.id)}>{@render icon("trash")} {t("Delete")}</button>
        {/if}
        <span class="spacer"></span>
        <button type="button" class="btn" onclick={() => (queueDraft = null)}>{t("Cancel")}</button>
        <button type="submit" class="btn primary">{t("Save")}</button>
      </div>
    </form>
  </div>
{/if}

{#if removeIds}
  <div class="overlay" role="presentation" onclick={(e) => e.target === e.currentTarget && (removeIds = null)}>
    <div class="modal small">
      <div class="modal-title">{@render icon("trash")} {removeIds.length > 1 ? t("Delete {n} downloads", { n: removeIds.length }) : t("Delete download")}</div>
      {#if removeIds.length === 1 && downloads[removeIds[0]]}<p class="wrap">{downloads[removeIds[0]].file_name}</p>{/if}
      <label class="check-row"><input type="checkbox" bind:checked={removeFiles} /> {t("Also delete the files from disk")}</label>
      <div class="modal-actions">
        <span class="spacer"></span>
        <button class="btn" onclick={() => (removeIds = null)}>{t("Cancel")}</button>
        <button class="btn danger-fill" onclick={confirmRemove}>{t("Delete")}</button>
      </div>
    </div>
  </div>
{/if}

{#if aboutOpen}
  <div class="overlay" role="presentation" onclick={(e) => e.target === e.currentTarget && (aboutOpen = false)}>
    <div class="modal small about">
      <svg width="56" height="56" viewBox="0 0 512 512" aria-hidden="true">
        <rect x="16" y="16" width="480" height="480" rx="112" fill="var(--accent)" />
        <path d="M80 236h76l40-92 56 196 40-104h140" fill="none" stroke="#fff" stroke-width="40" stroke-linecap="round" stroke-linejoin="round" />
      </svg>
      <h3>Pulse DM</h3>
      <p class="muted">{t("Version {version}", { version: appVersion })} · Burqk1</p>
      <p>{t("A fast, free and open-source download manager.")}</p>
      <div class="modal-actions"><span class="spacer"></span><button class="btn" onclick={() => (aboutOpen = false)}>{t("Close")}</button></div>
    </div>
  </div>
{/if}

{#if toast}<div class="toast {toast.kind}" role="alert">{toast.msg}</div>{/if}

<style>
  .app {
    display: grid;
    grid-template-columns: 196px 1fr;
    grid-template-rows: auto 1fr;
    height: 100vh;
    gap: 0 10px;
    padding: 0 10px 10px;
  }
  .titlebar {
    grid-column: 1 / -1;
    display: flex;
    align-items: center;
    height: 38px;
    margin: 0 -10px 4px;
    padding-left: 14px;
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12.5px;
    margin-right: 14px;
  }
  .menubar {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .menu {
    position: relative;
  }
  .menu-btn {
    border: 0;
    background: transparent;
    padding: 5px 9px;
    border-radius: 6px;
    cursor: pointer;
    font-size: 12.5px;
  }
  .menu-btn:hover,
  .menu-btn.open {
    background: var(--hover);
  }
  .dropdown {
    position: absolute;
    top: calc(100% + 4px);
    left: 0;
    min-width: 220px;
    padding: 4px;
    background: var(--panel-2);
    border: 1px solid var(--border);
    border-radius: 8px;
    box-shadow: 0 10px 30px rgb(0 0 0 / 0.35);
    z-index: 30;
    display: flex;
    flex-direction: column;
  }
  .dropdown button {
    display: flex;
    justify-content: space-between;
    gap: 20px;
    padding: 7px 10px;
    border: 0;
    border-radius: 5px;
    background: transparent;
    text-align: left;
    cursor: pointer;
  }
  .dropdown button:hover:not(:disabled) {
    background: var(--accent-soft);
  }
  .dropdown button:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .dropdown .sep {
    height: 1px;
    margin: 4px 6px;
    background: var(--border);
  }
  .kbd-hint {
    color: var(--muted);
    font-size: 11.5px;
  }
  .drag-fill {
    flex: 1;
    align-self: stretch;
  }
  .search {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 220px;
    padding: 0 10px;
    margin-right: 10px;
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 8px;
    color: var(--muted);
  }
  .search input {
    border: 0;
    background: transparent;
    padding: 4px 0;
    width: 100%;
  }
  .about {
    align-items: center;
    text-align: center;
  }
  .about h3 {
    margin: 4px 0 0;
  }
  .side {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-height: 0;
    overflow-y: auto;
    scrollbar-width: none;
  }
  .card,
  .content {
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 10px;
    min-height: 0;
  }
  .card {
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: 4px;
  }
  .head,
  .item,
  .q-main {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    border: 0;
    background: transparent;
    border-radius: 6px;
    cursor: pointer;
    text-align: left;
    position: relative;
  }
  .head {
    padding: 6px 8px;
    font-size: 13px;
    font-weight: 600;
  }
  .item {
    padding: 5px 8px 5px 20px;
    font-size: 12.5px;
    color: var(--muted);
  }
  .label {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .head:hover,
  .item:hover {
    background: var(--hover);
    color: var(--text);
  }
  .head.active,
  .item.active {
    color: var(--text);
    font-weight: 600;
    background: var(--accent-soft);
  }
  .head.active::before,
  .item.active::before {
    content: "";
    position: absolute;
    left: -4px;
    top: 6px;
    bottom: 6px;
    width: 3px;
    border-radius: 3px;
    background: var(--accent);
  }
  .chev {
    margin-left: auto;
    display: inline-flex;
    padding: 2px;
    border-radius: 4px;
    color: var(--muted);
    transform: rotate(-90deg);
    transition: transform 0.16s ease;
  }
  .chev.open {
    transform: rotate(0deg);
  }
  .chev:hover {
    background: var(--panel-2);
  }
  .items {
    display: flex;
    flex-direction: column;
    gap: 1px;
    overflow: hidden;
  }
  .queue {
    padding: 0 4px 0 0;
  }
  .q-main {
    flex: 1;
    min-width: 0;
    padding: 5px 4px 5px 20px;
    color: inherit;
    font: inherit;
  }
  .q-count {
    font-size: 11.5px;
    color: var(--muted);
    padding: 0 4px;
  }
  .q-actions {
    display: none;
  }
  .queue:hover .q-actions {
    display: flex;
  }
  .queue:hover .q-count {
    display: none;
  }
  .add-queue {
    opacity: 0.8;
  }
  .sched {
    display: inline-flex;
    color: var(--accent);
  }
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--muted);
    flex: none;
  }
  .dot.on {
    background: var(--ok);
    box-shadow: 0 0 0 3px rgb(34 197 94 / 0.18);
  }
  .mini {
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    border: 0;
    border-radius: 5px;
    background: transparent;
    color: var(--muted);
    cursor: pointer;
    flex: none;
  }
  .mini:hover {
    background: var(--panel-2);
    color: var(--text);
  }
  .content {
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 6px 8px;
    border-bottom: 1px solid var(--border);
    overflow: hidden;
  }
  .add-btn {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 6px 12px;
    border: 0;
    border-radius: 8px;
    background: var(--accent);
    color: #fff;
    font-weight: 600;
    cursor: pointer;
    white-space: nowrap;
  }
  .add-btn:hover {
    filter: brightness(1.1);
  }
  .divider {
    width: 1px;
    align-self: stretch;
    background: var(--border);
    margin: 2px 8px;
  }
  .tool {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 3px;
    padding: 3px 8px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    cursor: pointer;
    font-size: 11px;
    white-space: nowrap;
  }
  .tool:hover:not(:disabled) {
    background: var(--hover);
  }
  .check-mark {
    display: inline-block;
    width: 16px;
    color: var(--accent);
  }
  .after-chip {
    margin-right: auto;
    display: flex;
    align-items: center;
    gap: 5px;
    border: 0;
    border-radius: 5px;
    padding: 1px 8px;
    background: color-mix(in srgb, var(--warn) 18%, transparent);
    color: var(--warn);
    font-size: 12px;
    cursor: pointer;
  }
  .no-labels .tool span {
    display: none;
  }
  .no-labels .tool {
    padding: 6px 8px;
  }
  .tool:disabled {
    opacity: 0.35;
    cursor: default;
  }
  .banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    margin: 8px 10px 0;
    padding: 8px 12px;
    background: color-mix(in srgb, var(--warn) 16%, transparent);
    border: 1px solid color-mix(in srgb, var(--warn) 50%, transparent);
    border-radius: 8px;
  }
  .banner.update {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    border-color: color-mix(in srgb, var(--accent) 45%, transparent);
  }
  .banner-actions {
    display: flex;
    gap: 6px;
  }
  .table {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .thead,
  .tr {
    display: grid;
    grid-template-columns: 34px minmax(160px, 1fr) 90px 150px 95px 95px 120px;
    align-items: center;
  }
  .thead {
    margin: 6px 6px 2px;
    background: var(--panel-2);
    border-radius: 6px;
    font-size: 12.5px;
  }
  .th {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 5px 8px;
    border: 0;
    border-left: 1px solid var(--border);
    background: transparent;
    color: var(--muted);
    cursor: pointer;
    text-align: left;
  }
  .th.sorted {
    color: var(--text);
  }
  .th.num {
    justify-content: flex-end;
  }
  .arrow {
    display: inline-flex;
  }
  .arrow.asc {
    transform: rotate(180deg);
  }
  .tbody {
    flex: 1;
    overflow-y: auto;
    padding: 0 8px 8px;
  }
  .tr {
    min-height: 36px;
    border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
    border-radius: 6px;
  }
  .tr:hover {
    background: var(--hover);
  }
  .tr.checked {
    background: var(--accent-soft);
  }
  .tr.focused {
    background: var(--accent-soft);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent) 60%, transparent);
  }
  .cell {
    padding: 0 8px;
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    font-variant-numeric: tabular-nums;
  }
  .cell.num {
    text-align: right;
  }
  .cell.check {
    display: grid;
    place-items: center;
    padding: 0;
  }
  .cell.check input {
    accent-color: var(--accent);
    width: 14px;
    height: 14px;
    margin: 0;
  }
  .cell.name {
    display: flex;
    align-items: center;
    gap: 9px;
  }
  .ficon {
    display: grid;
    place-items: center;
    color: var(--muted);
    flex: none;
  }
  .name-text {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .fname {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .fcat {
    font-size: 10.5px;
    color: var(--muted);
  }
  .cell.status {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .mini-bar {
    position: relative;
    height: 4px;
    width: 100%;
    max-width: 120px;
    background: var(--panel-2);
    border-radius: 99px;
    overflow: hidden;
  }
  .mini-fill {
    height: 100%;
    border-radius: 99px;
    transition: width 0.45s linear;
  }
  .indeterminate::after {
    content: "";
    position: absolute;
    inset: 0;
    width: 35%;
    background: var(--accent);
    animation: slide 1.1s infinite;
  }
  @keyframes slide {
    from { transform: translateX(-100%); }
    to { transform: translateX(300%); }
  }
  .st-downloading, .st-connecting { color: var(--text); }
  .st-queued { color: var(--muted); }
  .st-paused { color: var(--warn); }
  .st-completed { color: var(--ok); }
  .st-failed { color: var(--danger); }
  .st-bg-downloading, .st-bg-connecting { background: var(--accent); }
  .st-bg-queued { background: var(--muted); }
  .st-bg-paused { background: var(--warn); }
  .statusbar {
    display: flex;
    justify-content: flex-end;
    gap: 18px;
    padding: 6px 14px;
    border-top: 1px solid var(--border);
    color: var(--muted);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }
  .statusbar span {
    display: flex;
    align-items: center;
    gap: 5px;
  }
  .empty {
    height: 100%;
    display: grid;
    place-content: center;
    justify-items: center;
    gap: 4px;
    text-align: center;
    color: var(--muted);
    padding: 40px;
  }
  .empty :global(svg) {
    color: var(--accent);
  }
  .empty h2 {
    color: var(--text);
    font-size: 16px;
    margin: 10px 0 4px;
  }
  kbd {
    background: var(--panel-2);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 0 5px;
    font: inherit;
    font-size: 12px;
  }
  .modal label {
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .overlay {
    position: fixed;
    inset: 0;
    background: rgb(0 0 0 / 0.45);
    display: grid;
    place-items: center;
    padding: 16px;
    z-index: 10;
  }
  .modal {
    width: min(540px, 100%);
    max-height: calc(100vh - 32px);
    overflow-y: auto;
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 14px 16px 16px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    box-shadow: 0 16px 48px rgb(0 0 0 / 0.45);
  }
  .modal.wide {
    width: min(620px, 100%);
  }
  .modal.small {
    width: min(400px, 100%);
  }
  .modal-title {
    display: flex;
    align-items: center;
    gap: 8px;
    font-weight: 600;
    padding-bottom: 4px;
  }
  .modal .check-row {
    flex-direction: row;
    align-items: center;
    gap: 8px;
  }
  .modal p {
    margin: 0;
  }
  .field-row {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .field-row > input,
  .grow {
    flex: 1;
    min-width: 0;
  }
  .grid2 {
    grid-template-columns: 1fr 1fr 110px;
  }
  .tabs {
    display: flex;
    gap: 4px;
    border-bottom: 1px solid var(--border);
    margin-bottom: 4px;
  }
  .tabs button {
    border: 0;
    background: transparent;
    padding: 7px 12px;
    color: var(--muted);
    cursor: pointer;
    border-bottom: 2px solid transparent;
  }
  .tabs button.active {
    color: var(--text);
    border-bottom-color: var(--accent);
  }
  .modal.settings {
    width: min(820px, 100%);
  }
  .settings-body {
    display: grid;
    grid-template-columns: 170px 1fr;
    gap: 12px;
    height: min(440px, calc(100vh - 160px));
  }
  .settings-nav {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding-right: 10px;
    border-right: 1px solid var(--border);
  }
  .settings-nav button {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 10px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: var(--muted);
    cursor: pointer;
    text-align: left;
  }
  .settings-nav button:hover {
    background: var(--hover);
    color: var(--text);
  }
  .settings-nav button.active {
    background: var(--accent-soft);
    color: var(--text);
    font-weight: 600;
  }
  .settings-pane {
    display: flex;
    flex-direction: column;
    gap: 6px;
    overflow-y: auto;
    padding-right: 4px;
  }
  .srow {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 9px 12px;
    border-radius: 8px;
    background: var(--panel-2);
  }
  .modal .srow {
    flex-direction: row;
  }
  .modal .srow.column {
    flex-direction: column;
    align-items: stretch;
  }
  .srow > div:first-child {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .srow > div:first-child span {
    color: var(--muted);
    font-size: 12px;
  }
  .srow .warn-text {
    color: var(--warn) !important;
  }
  .srow input[type="checkbox"] {
    width: 16px;
    height: 16px;
    flex: none;
  }
  .srow select,
  .srow .num {
    width: 150px;
    flex: none;
  }
  .range {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: none;
  }
  .range input {
    width: 150px;
  }
  .range b {
    width: 22px;
    text-align: right;
  }
  .seg-control {
    display: flex;
    padding: 2px;
    border-radius: 7px;
    background: var(--panel);
    border: 1px solid var(--border);
    flex: none;
  }
  .seg-control button {
    border: 0;
    background: transparent;
    padding: 5px 12px;
    border-radius: 5px;
    cursor: pointer;
    color: var(--muted);
  }
  .seg-control button.on {
    background: var(--accent);
    color: #fff;
  }
  .rules {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .rule {
    display: grid;
    grid-template-columns: 1fr 150px 80px 32px;
    gap: 6px;
    align-items: center;
  }
  .steps {
    margin: 0;
    padding-left: 20px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .days {
    display: flex;
    gap: 4px;
  }
  .day {
    flex: 1;
    padding: 6px 0;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--panel-2);
    cursor: pointer;
  }
  .day.on {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }
  .modal-actions {
    display: flex;
    gap: 8px;
    margin-top: 6px;
  }
  .toast {
    position: fixed;
    bottom: 18px;
    left: 50%;
    transform: translateX(-50%);
    background: var(--panel-2);
    border: 1px solid var(--danger);
    border-radius: 8px;
    padding: 9px 14px;
    z-index: 20;
    max-width: 80vw;
  }
  .toast.info {
    border-color: var(--accent);
  }
</style>
