(() => {
  if (window.__pulseDm) return;
  window.__pulseDm = true;

  const MIN_W = 240;
  const MIN_H = 135;
  const msg = (key) => chrome.i18n.getMessage(key);
  const IDLE = msg("downloadWithPulse");

  let catchMagnets = false;
  const readMagnetSetting = () =>
    chrome.storage.local.get(["enabled", "magnets"], (s) => (catchMagnets = s.enabled !== false && s.magnets !== false));
  readMagnetSetting();
  chrome.storage.onChanged.addListener((_, area) => area === "local" && readMagnetSetting());

  document.addEventListener(
    "click",
    (e) => {
      if (!catchMagnets || e.defaultPrevented || e.button !== 0) return;
      const link = e.composedPath().find((n) => n instanceof HTMLAnchorElement && /^magnet:/i.test(n.href));
      if (!link) return;
      e.preventDefault();
      e.stopPropagation();
      chrome.runtime.sendMessage({ type: "magnet", url: link.href }, (r) => {
        if (chrome.runtime.lastError || !r?.ok) location.href = link.href;
      });
    },
    true,
  );

  let current = null;
  let hideTimer = 0;
  let frame = 0;
  const dismissed = new WeakSet();

  const host = document.createElement("div");
  host.style.cssText = "position:fixed;z-index:2147483647;display:none;top:0;left:0;";
  const root = host.attachShadow({ mode: "closed" });
  const el = (tag, props = {}, ...children) => {
    const node = Object.assign(document.createElement(tag), props);
    node.append(...children);
    return node;
  };
  const SVG = "http://www.w3.org/2000/svg";
  const icon = document.createElementNS(SVG, "svg");
  for (const [k, v] of Object.entries({
    width: 14, height: 14, viewBox: "0 0 24 24", fill: "none", stroke: "currentColor",
    "stroke-width": 2.4, "stroke-linecap": "round", "stroke-linejoin": "round",
  })) icon.setAttribute(k, v);
  const path = document.createElementNS(SVG, "path");
  path.setAttribute("d", "M12 3v12M7 10l5 5 5-5M5 21h14");
  icon.append(path);

  const label = el("span", { textContent: IDLE });
  root.append(
    el("style", {
      textContent: `
      .bar { display:flex; align-items:stretch; font:600 12px/1 "Segoe UI",system-ui,sans-serif;
             background:#2563eb; color:#fff; border-radius:7px; overflow:hidden;
             box-shadow:0 2px 12px rgba(0,0,0,.4); user-select:none; }
      button { all:unset; cursor:pointer; display:flex; align-items:center; gap:6px; padding:7px 10px; }
      button:hover { background:rgba(255,255,255,.16); }
      .x { padding:7px 8px; border-left:1px solid rgba(255,255,255,.25); opacity:.85; }`,
    }),
    el(
      "div",
      { className: "bar" },
      el("button", { className: "dl", title: msg("downloadThisVideo") }, icon, label),
      el("button", { className: "x", title: msg("hideForVideo"), textContent: "✕" }),
    ),
  );

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

  function videoIn(scope, x, y) {
    for (const v of scope.querySelectorAll("video")) {
      const r = v.getBoundingClientRect();
      if (big(r) && inside(r, x, y)) return v;
    }
    return null;
  }

  function deepVideoIn(scope, x, y, depth = 0) {
    const hit = videoIn(scope, x, y);
    if (hit || depth > 3) return hit;
    for (const el of scope.querySelectorAll("*")) {
      const found = el.shadowRoot && deepVideoIn(el.shadowRoot, x, y, depth + 1);
      if (found) return found;
    }
    return null;
  }

  function videoAt(x, y) {
    const direct = videoIn(document, x, y);
    if (direct) return direct;
    for (const el of document.elementsFromPoint(x, y)) {
      const found = el.shadowRoot && deepVideoIn(el.shadowRoot, x, y);
      if (found) return found;
      if (/^SHREDDIT-PLAYER/.test(el.tagName) && big(el.getBoundingClientRect())) return el;
    }
    return null;
  }

  function closestDeep(el, selector) {
    for (let node = el; node; node = node.getRootNode()?.host) {
      const hit = node.closest?.(selector);
      if (hit) return hit;
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

  function tiktokLink(video) {
    if (/\/video\/\d+/.test(location.pathname)) return location.href;
    const card = closestDeep(video, "article, [data-e2e*='item-container'], [class*='DivItemContainer']");
    const scope = card ?? video.parentElement;
    const link = scope?.querySelector("a[href*='/video/']");
    if (link) return link.href;
    let id = null;
    for (let node = video; node && node !== card?.parentElement && !id; node = node.parentElement) {
      id = /(\d{17,20})/.exec(node.id ?? "")?.[1] ?? null;
    }
    id ??= /(\d{17,20})/.exec(scope?.querySelector("[id*='xgwrapper']")?.id ?? "")?.[1] ?? null;
    if (!id) return null;
    const user = /\/@([\w.-]+)/.exec(scope?.querySelector("a[href*='/@']")?.getAttribute("href") ?? "")?.[1] ?? "";
    return `https://www.tiktok.com/@${user}/video/${id}`;
  }

  function permalink(video) {
    if (/(^|\.)tiktok\.com$/.test(location.hostname)) return tiktokLink(video);
    const post = closestDeep(video, "shreddit-post");
    if (post?.getAttribute("permalink")) return new URL(post.getAttribute("permalink"), location.origin).href;
    const card = closestDeep(video, "article, shreddit-post, [data-testid='tweet']");
    const link = card?.querySelector("a[href*='/status/'], a[href*='/p/'], a[href*='/reel/'], a[href*='/comments/']");
    return link?.href ?? null;
  }

  const REDDIT_STREAM = /https:\/\/v\.redd\.it\/[^"'\s]+?\.m3u8[^"'\s]*/;

  function redditInfo(video) {
    const player = closestDeep(video, "shreddit-player, shreddit-player-2") ?? video;
    let stream = null;
    for (const el of [player, video]) {
      for (const attr of el.attributes ?? []) stream ??= REDDIT_STREAM.exec(attr.value)?.[0] ?? null;
    }
    const post = closestDeep(video, "shreddit-post");
    return { stream: stream?.replaceAll("&amp;", "&") ?? null, postTitle: post?.getAttribute("post-title") ?? null };
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
    label.textContent = msg("sending");
    chrome.runtime.sendMessage(
      {
        type: "video",
        src: video.currentSrc || video.src || "",
        page: location.href,
        permalink: permalink(video),
        title: document.title,
        ...(/(^|\.)reddit\.com$/.test(location.hostname) ? redditInfo(video) : {}),
      },
      (r) => {
        label.textContent = chrome.runtime.lastError
          ? msg("extensionReloaded")
          : r?.ok
            ? msg("sent")
            : r?.error || msg("unreachable");
        setTimeout(() => (label.textContent = IDLE), 2500);
      },
    );
  });
})();
