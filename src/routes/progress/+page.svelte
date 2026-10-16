<script lang="ts">
  import { onMount } from "svelte";
  import { slide } from "svelte/transition";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import Icon from "$lib/Icon.svelte";
  import TitleBar from "$lib/TitleBar.svelte";
  import { fitWindow } from "$lib/fitWindow";
  import {
    api,
    extractUrls,
    formatBytes,
    formatEta,
    formatSpeed,
    MB,
    type ChecksumResult,
    type Download,
    type QueueConfig,
    type Status,
  } from "$lib/api";

  const win = getCurrentWindow();
  const id = win.label.replace(/^progress-/, "");

  let d = $state<Download | null>(null);
  let tab = $state<"info" | "speed" | "done">("info");
  let showParts = $state(false);
  let defaultConnections = $state(8);
  let optConn = $state("");
  let optLimitMb = $state(0);
  let saved = $state(false);
  let queues = $state<QueueConfig[]>([]);
  let newUrl = $state("");
  let message = $state<{ text: string; error: boolean } | null>(null);
  let sumOpen = $state(false);
  let sumExpected = $state("");
  let sumBusy = $state(false);
  let sumResult = $state<ChecksumResult | null>(null);

  let whenDone = $state({ openFile: false, openFolder: false, closeWindow: false });
  const AUTO_CLOSE_SECS = 20;
  let autoClose = $state<number | null>(null);
  let autoTimer = 0;

  const LABEL: Record<Status, string> = {
    queued: "Sırada",
    connecting: "Bağlanıyor",
    downloading: "İndiriliyor",
    paused: "Duraklatıldı",
    completed: "Tamamlandı",
    failed: "Hata",
    processing: "İşleniyor",
  };
  const ACTIVE: Status[] = ["queued", "connecting", "downloading", "processing"];

  const done = $derived(d?.status === "completed");
  const pct = $derived.by(() => {
    if (!d) return 0;
    if (d.status === "completed") return 100;
    if (d.parts) return d.parts[1] ? (d.parts[0] / d.parts[1]) * 100 : 0;
    return d.size ? Math.min(100, (d.downloaded / d.size) * 100) : 0;
  });
  const title = $derived(d ? (done ? d.file_name : `%${Math.floor(pct)} ${d.file_name}`) : "Pulse DM");

  $effect(() => {
    win.setTitle(title);
  });

  function say(text: string, error = false) {
    message = { text, error };
    setTimeout(() => (message = null), 4000);
  }

  async function run(p: Promise<unknown>) {
    try {
      await p;
    } catch (e) {
      say(String(e), true);
    }
  }

  async function replaceUrl() {
    const url = extractUrls(newUrl)[0];
    if (!url) return say("Geçerli bir bağlantı gir", true);
    try {
      await api.setUrl(id, url);
      await api.resume(id);
      newUrl = "";
    } catch (e) {
      say(String(e), true);
    }
  }

  async function verify() {
    sumBusy = true;
    sumResult = null;
    try {
      sumResult = await api.verifyChecksum(id, sumExpected ? null : "sha256", sumExpected || null);
    } catch (e) {
      say(String(e), true);
    } finally {
      sumBusy = false;
    }
  }

  async function applySettings() {
    if (!d) return;
    await run(api.setOptions(d.id, { connections: optConn ? Number(optConn) : null, speed_limit: Math.round(optLimitMb * MB) }));
    saved = true;
    setTimeout(() => (saved = false), 1500);
  }

  async function cancel() {
    await run(api.pause(id));
    win.close();
  }

  function segState(s: { pos: number; end: number }): "done" | "active" | "idle" {
    if (s.pos >= s.end) return "done";
    return d?.status === "downloading" ? "active" : "idle";
  }

  function startAutoClose() {
    autoClose = AUTO_CLOSE_SECS;
    clearInterval(autoTimer);
    autoTimer = window.setInterval(() => {
      if (autoClose === null) return clearInterval(autoTimer);
      autoClose -= 1;
      if (autoClose <= 0) win.close();
    }, 1000);
  }

  function keepOpen() {
    autoClose = null;
    clearInterval(autoTimer);
  }

  async function onFinished() {
    if (whenDone.openFile) await run(api.openFile(id));
    if (whenDone.openFolder) await run(api.showInFolder(id));
    if (whenDone.closeWindow) return win.close();
    startAutoClose();
  }

  onMount(() => {
    let unlisten: (() => void) | undefined;
    (async () => {
      unlisten = await api.onEvent((ev) => {
        let next: Download | undefined;
        if (ev.type === "progress") next = ev.data.find((x) => x.id === id);
        else if (ev.type === "changed" && ev.data.id === id) next = ev.data;
        else if (ev.type === "removed" && ev.data === id) win.close();
        if (next) {
          const wasDone = d?.status === "completed";
          d = next;
          if (!wasDone && next.status === "completed") onFinished();
        }
      });
      const [list, config] = await Promise.all([api.list(), api.getConfig()]);
      d = list.find((x) => x.id === id) ?? null;
      defaultConnections = config.connections;
      queues = config.queues;
      if (d) {
        optConn = d.connections_setting ? String(d.connections_setting) : "";
        optLimitMb = d.speed_limit / MB;
      }
    })();
    return () => {
      unlisten?.();
      clearInterval(autoTimer);
    };
  });
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && win.close()} />

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="window" onmouseenter={keepOpen} onfocusin={keepOpen}>
  <TitleBar {title} minimize={!done} />

  {#if !d}
    <main use:fitWindow={{ width: 440, max: 600 }}><div class="body"><p class="muted">İndirme bulunamadı.</p></div></main>
  {:else if done}
    <main use:fitWindow={{ width: 440, max: 600 }}>
      <div class="body">
        <div class="complete">
          <div class="file-badge">
            <Icon name={d.category} size={26} />
            <span>{formatBytes(d.size)}</span>
          </div>
          <div class="complete-text">
            <b class="ok-title"><Icon name="finished" size={17} /> İndirme tamamlandı</b>
            <span class="fname" title={d.file_name}>{d.file_name}</span>
            {#if autoClose !== null}<span class="muted small">{autoClose} sn sonra kapanacak</span>{/if}
          </div>
        </div>
        {#if sumOpen}
          <div class="sum" transition:slide={{ duration: 140 }}>
            <div class="field-row">
              <input bind:value={sumExpected} placeholder="Beklenen hash (boş = SHA-256 hesapla)" />
              <button class="btn" disabled={sumBusy} onclick={verify}>{sumBusy ? "Hesaplanıyor…" : "Doğrula"}</button>
            </div>
            {#if sumResult}
              <code class="wrap">{sumResult.algorithm.toUpperCase()}: {sumResult.actual}</code>
              {#if sumResult.matches === true}<b class="ok">✓ Eşleşiyor, dosya sağlam</b>
              {:else if sumResult.matches === false}<b class="bad">✗ Eşleşmiyor, dosya bozuk ya da değiştirilmiş</b>{/if}
            {/if}
          </div>
        {/if}
        {#if message}<div class="message" class:error={message.error}>{message.text}</div>{/if}
      </div>
    </main>
    <footer>
      <button class="btn" onclick={() => run(api.openFile(id))}>Aç</button>
      <button class="btn" onclick={() => run(api.showInFolder(id))}>Klasörü aç</button>
      <button class="btn icon" class:active={sumOpen} title="Dosyayı doğrula (MD5 / SHA)" aria-label="Dosyayı doğrula" onclick={() => (sumOpen = !sumOpen)}><Icon name="shield" /></button>
      <span class="spacer"></span>
      <button class="btn" onclick={() => win.close()}>Kapat</button>
    </footer>
  {:else}
    <div class="tabs">
      <button class:active={tab === "info"} onclick={() => (tab = "info")}><Icon name="list" size={13} /> Bilgi</button>
      <button class:active={tab === "speed"} onclick={() => (tab = "speed")}><Icon name="speed" size={13} /> Hız</button>
      <button class:active={tab === "done"} onclick={() => (tab = "done")}><Icon name="finished" size={13} /> Tamamlanınca</button>
    </div>

    <main use:fitWindow={{ width: 440, max: 640 }}>
      <div class="body">
        {#if tab === "info"}
          <dl class="props">
            <dt>Ad:</dt><dd class="name" title={d.file_name}>{d.file_name}</dd>
            <dt>Durum:</dt><dd class="st-{d.status}">{LABEL[d.status]}</dd>
            <dt>Boyut:</dt><dd>{formatBytes(d.size)}</dd>
            <dt>İndirilen:</dt><dd>{formatBytes(d.downloaded)} (%{Math.floor(pct)})</dd>
            <dt>Hız:</dt><dd>{d.status === "downloading" ? formatSpeed(d.speed) : "—"}</dd>
            <dt>Kalan süre:</dt><dd>{d.status === "downloading" ? formatEta(d.eta_secs) || "—" : "—"}</dd>
            <dt>Devam desteği:</dt><dd class={d.resumable ? "ok" : "bad"}>{d.resumable ? "Var" : "Yok"}</dd>
            {#if d.error}<dt>Hata:</dt><dd class="bad wrap">{d.error}</dd>{/if}
          </dl>
          {#if d.status === "failed" || d.status === "paused"}
            <div class="field-row">
              <input bind:value={newUrl} placeholder="Link süresi dolduysa yeni bağlantıyı yapıştır" />
              <button class="btn" onclick={replaceUrl}>Yenile</button>
            </div>
          {/if}
        {:else if tab === "speed"}
          <label class="row">
            <span>Bağlantı sayısı</span>
            <select bind:value={optConn}>
              <option value="">Otomatik ({defaultConnections})</option>
              {#each [1, 2, 4, 6, 8, 12, 16, 24, 32] as n (n)}<option value={String(n)}>{n}</option>{/each}
            </select>
          </label>
          <label class="row">
            <span>Hız sınırı (MB/s)</span>
            <input type="number" min="0" step="0.5" bind:value={optLimitMb} placeholder="0 = sınırsız" />
          </label>
          <label class="row">
            <span>Kuyruk</span>
            <select value={d.queue} onchange={(e) => run(api.moveToQueue([id], (e.target as HTMLSelectElement).value))}>
              {#each queues as q (q.id)}<option value={q.id}>{q.name}</option>{/each}
            </select>
          </label>
          <div><button class="btn" onclick={applySettings}>{saved ? "Uygulandı ✓" : "Uygula"}</button></div>
        {:else}
          <label class="check"><input type="checkbox" bind:checked={whenDone.openFile} /> Dosyayı aç</label>
          <label class="check"><input type="checkbox" bind:checked={whenDone.openFolder} /> Klasörü aç</label>
          <label class="check"><input type="checkbox" bind:checked={whenDone.closeWindow} /> Bu pencereyi kapat</label>
          <span class="muted small">Bu indirme bitince yapılır (pencere açıkken).</span>
        {/if}

        <div class="bar"><div class="fill" class:paused={!ACTIVE.includes(d.status)} style="width:{pct}%"></div></div>

        {#if showParts && d.segments.length && d.size}
          <div class="parts-box" transition:slide={{ duration: 160 }}>
            <div class="map">
              {#each d.segments as s (s.start)}
                <div class="map-seg {segState(s)}" style="left:{(s.start / d.size) * 100}%;width:{((s.end - s.start) / d.size) * 100}%">
                  <div class="map-fill" style="width:{((s.pos - s.start) / Math.max(1, s.end - s.start)) * 100}%"></div>
                </div>
              {/each}
            </div>
            <div class="parts">
              <div class="prow head"><span>#</span><span>Durum</span><span>İndirilen</span><span>Toplam</span></div>
              {#each d.segments as s, i (s.start)}
                <div class="prow">
                  <span>{i + 1}</span>
                  <span class="seg-{segState(s)}">{segState(s) === "done" ? "Bitti" : segState(s) === "active" ? "Veri alınıyor" : "Bekliyor"}</span>
                  <span>{formatBytes(s.pos - s.start)}</span>
                  <span>{s.end > 1e18 ? "?" : formatBytes(s.end - s.start)}</span>
                </div>
              {/each}
            </div>
          </div>
        {/if}
        {#if message}<div class="message" class:error={message.error}>{message.text}</div>{/if}
      </div>
    </main>

    <footer>
      <button class="btn icon" title="Parça bilgisi" aria-label="Parça bilgisi" disabled={!d.segments.length} onclick={() => (showParts = !showParts)}>
        <span class="chev" class:open={showParts}><Icon name="chevron" size={14} /></span>
      </button>
      {#if showParts && d.segments.length}<span class="muted small">{d.connections} bağlantı aktif</span>{/if}
      <span class="spacer"></span>
      {#if ACTIVE.includes(d.status)}
        <button class="btn" onclick={() => run(api.pause(id))}><Icon name="pause" size={14} /> Duraklat</button>
      {:else}
        <button class="btn primary" onclick={() => run(api.resume(id))}><Icon name={d.status === "failed" ? "retry" : "play"} size={14} /> {d.status === "failed" ? "Tekrar dene" : "Sürdür"}</button>
      {/if}
      <button class="btn" onclick={cancel}>İptal</button>
    </footer>
  {/if}
</div>

<style>
  .window {
    height: 100vh;
    display: flex;
    flex-direction: column;
    background: var(--panel);
    border: 1px solid var(--border);
    font-size: 12.5px;
  }
  .tabs {
    display: flex;
    gap: 2px;
    margin: 0 10px;
    padding: 3px;
    background: var(--panel-2);
    border-radius: 7px;
    flex: none;
  }
  .tabs button {
    display: flex;
    align-items: center;
    gap: 5px;
    border: 0;
    background: transparent;
    padding: 5px 10px;
    border-radius: 5px;
    cursor: pointer;
    color: var(--muted);
    font-size: 12px;
  }
  .tabs button.active {
    background: var(--panel);
    color: var(--text);
  }
  main {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    scrollbar-width: none;
    padding: 8px 12px;
  }
  main::-webkit-scrollbar {
    display: none;
  }
  .body {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .props {
    display: grid;
    grid-template-columns: 96px 1fr;
    gap: 2px 8px;
    margin: 0;
    font-variant-numeric: tabular-nums;
  }
  .props dt {
    color: var(--muted);
  }
  .props dd {
    margin: 0;
    min-width: 0;
  }
  .name,
  .fname {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ok,
  .st-completed {
    color: var(--ok);
  }
  .bad,
  .st-failed {
    color: var(--danger);
  }
  .st-paused {
    color: var(--warn);
  }
  .st-queued {
    color: var(--muted);
  }
  .bar {
    height: 12px;
    border-radius: 4px;
    background: var(--panel-2);
    overflow: hidden;
    flex: none;
  }
  .fill {
    height: 100%;
    border-radius: 4px;
    background: linear-gradient(90deg, #4f8ff7, #a855f7);
    transition: width 0.45s linear;
  }
  .fill.paused {
    background: linear-gradient(90deg, #b45309, var(--warn));
  }
  .parts-box {
    display: flex;
    flex-direction: column;
    gap: 6px;
    overflow: hidden;
  }
  .map {
    position: relative;
    height: 10px;
    background: var(--panel-2);
    border-radius: 2px;
    overflow: hidden;
  }
  .map-seg {
    position: absolute;
    top: 0;
    bottom: 0;
    border-right: 2px solid var(--panel);
  }
  .map-fill {
    height: 100%;
    transition: width 0.45s linear;
  }
  .map-seg.done .map-fill {
    background: #15803d;
  }
  .map-seg.active .map-fill {
    background: var(--ok);
  }
  .map-seg.idle .map-fill {
    background: var(--warn);
  }
  .parts {
    background: var(--panel-2);
    border-radius: 6px;
    padding: 3px 0;
    font-size: 11.5px;
    font-variant-numeric: tabular-nums;
  }
  .prow {
    display: grid;
    grid-template-columns: 24px 1.2fr 1fr 1fr;
    padding: 2px 8px;
  }
  .prow.head {
    color: var(--muted);
    border-bottom: 1px solid var(--border);
  }
  .seg-done,
  .seg-active {
    color: var(--ok);
  }
  .seg-idle {
    color: var(--muted);
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
  }
  .row select,
  .row input {
    width: 170px;
    padding: 5px 8px;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .check input {
    accent-color: var(--accent);
  }
  .field-row {
    display: flex;
    gap: 6px;
  }
  .field-row input {
    flex: 1;
    min-width: 0;
    padding: 5px 8px;
  }
  .complete {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 4px 2px;
  }
  .file-badge {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    width: 64px;
    color: var(--muted);
    font-size: 11.5px;
    flex: none;
  }
  .complete-text {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  .ok-title {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--ok);
    font-size: 14px;
  }
  .sum {
    display: flex;
    flex-direction: column;
    gap: 6px;
    overflow: hidden;
  }
  .message {
    padding: 6px 10px;
    border-radius: 6px;
    border: 1px solid var(--accent);
  }
  .message.error {
    border-color: var(--danger);
  }
  footer {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 12px 10px;
    border-top: 1px solid var(--border);
    flex: none;
  }
  footer .btn {
    padding: 5px 12px;
  }
  .btn.icon {
    padding: 5px 7px;
  }
  .btn.icon.active {
    color: var(--accent);
    border-color: var(--accent);
  }
  .chev {
    display: inline-flex;
    transform: rotate(-90deg);
    transition: transform 0.15s;
  }
  .chev.open {
    transform: none;
  }
</style>
