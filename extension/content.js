(() => {
  if (window.__pulseDm) return;
  window.__pulseDm = true;

  const MIN_W = 240;
  const MIN_H = 135;
  const IDLE = "Pulse DM ile indir";

  let current = null;
  let hideTimer = 0;
  let frame = 0;
  const dismissed = new WeakSet();

  const host = document.createElement("div");
  host.style.cssText = "position:fixed;z-index:2147483647;display:none;top:0;left:0;";
  const root = host.attachShadow({ mode: "closed" });
  root.innerHTML = `
    <style>
      .bar { display:flex; align-items:stretch; font:600 12px/1 "Segoe UI",system-ui,sans-serif;
             background:#2563eb; color:#fff; border-radius:7px; overflow:hidden;
             box-shadow:0 2px 12px rgba(0,0,0,.4); user-select:none; }
      button { all:unset; cursor:pointer; display:flex; align-items:center; gap:6px; padding:7px 10px; }
      button:hover { background:rgba(255,255,255,.16); }
      .x { padding:7px 8px; border-left:1px solid rgba(255,255,255,.25); opacity:.85; }
    </style>
    <div class="bar">
      <button class="dl" title="Bu videoyu Pulse DM ile indir">
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4"
             stroke-linecap="round" stroke-linejoin="round"><path d="M12 3v12M7 10l5 5 5-5M5 21h14"/></svg>
        <span></span>
      </button>
      <button class="x" title="Bu video için gizle">✕</button>
    </div>`;
  const label = root.querySelector("span");
  label.textContent = IDLE;

  const big = (r) => r.width >= MIN_W && r.height >= MIN_H;
  const inside = (r, x, y) => x >= r.left && x <= r.right && y >= r.top && y <= r.bottom;

  function place() {
    if (!current || !current.isConnected) return hide();
    const r = current.getBoundingClientRect();
    if (!big(r) || r.bottom < 0 || r.top > innerHeight) return hide();
    if (!host.isConnected) document.documentElement.appendChild(host);
    host.style.display = "block";
    host.style.left = `${Math.max(4, Math.min(r.right, innerWidth) - host.offsetWidth - 10)}px`;
    host.style.top = `${Math.max(4, r.top + 10)}px`;
  }

  function show(video) {
    if (dismissed.has(video)) return;
    current = video;
    label.textContent = IDLE;
    place();
  }

  function hide() {
    host.style.display = "none";
    current = null;
  }

  function videoAt(x, y) {
    for (const v of document.querySelectorAll("video")) {
      const r = v.getBoundingClientRect();
      if (big(r) && inside(r, x, y)) return v;
    }
    return null;
  }

  document.addEventListener(
    "mousemove",
    (e) => {
      if (frame) return;
      frame = requestAnimationFrame(() => {
        frame = 0;
        if (e.composedPath().includes(host)) return clearTimeout(hideTimer);
        const v = videoAt(e.clientX, e.clientY);
        if (v) {
          clearTimeout(hideTimer);
          if (v !== current) show(v);
        } else if (current) {
          clearTimeout(hideTimer);
          hideTimer = setTimeout(hide, 1200);
        }
      });
    },
    { passive: true, capture: true },
  );
  addEventListener("scroll", () => current && place(), { passive: true, capture: true });
  addEventListener("resize", () => current && place(), { passive: true });
  document.addEventListener("fullscreenchange", () => current && setTimeout(place, 200));

  function permalink(video) {
    const card = video.closest("article, shreddit-post, [data-testid='tweet']");
    const link = card?.querySelector("a[href*='/status/'], a[href*='/p/'], a[href*='/reel/'], a[href*='/comments/']");
    return link?.href ?? null;
  }

  root.querySelector(".x").addEventListener("click", (e) => {
    e.stopPropagation();
    if (current) dismissed.add(current);
    hide();
  });

  root.querySelector(".dl").addEventListener("click", (e) => {
    e.stopPropagation();
    const video = current;
    if (!video) return;
    label.textContent = "Gönderiliyor…";
    chrome.runtime.sendMessage(
      {
        type: "video",
        src: video.currentSrc || video.src || "",
        page: location.href,
        permalink: permalink(video),
        title: document.title,
      },
      (r) => {
        label.textContent = chrome.runtime.lastError
          ? "Eklenti yeniden yüklendi, sayfayı yenile"
          : r?.ok
            ? "Pulse DM'e gönderildi ✓"
            : r?.error || "Pulse DM'e ulaşılamadı";
        setTimeout(() => (label.textContent = IDLE), 2500);
      },
    );
  });
})();
