
import { DEFAULTS, isVideoSite } from "./shared.js";

const HOST = "com.novament.pulsedm";

async function settings() {
  return { ...DEFAULTS, ...(await chrome.storage.local.get(Object.keys(DEFAULTS))) };
}

const baseName = (p) => (p || "").split(/[\\/]/).pop();
const extOf = (name) => (/\.([a-z0-9]{1,8})$/i.exec(name || "")?.[1] ?? "").toLowerCase();
const safeName = (title) => (title || "video").replace(/[<>:"/\\|?*]+/g, " ").replace(/\s+/g, " ").trim().slice(0, 120);

async function cookieHeader(url) {
  try {
    const cookies = await chrome.cookies.getAll({ url });
    return cookies.map((c) => `${c.name}=${c.value}`).join("; ");
  } catch {
    return "";
  }
}

async function sendToPulse({ url, filename, referrer, media = false }) {
  const reply = await chrome.runtime.sendNativeMessage(HOST, {
    type: "add",
    url,
    filename: filename || null,
    referrer: referrer || null,
    cookies: await cookieHeader(url),
    userAgent: navigator.userAgent,
    media,
  });
  if (!reply?.ok) throw new Error(reply?.error || chrome.i18n.getMessage("noReply"));
}

async function ping() {
  try {
    return (await chrome.runtime.sendNativeMessage(HOST, { type: "ping" }))?.ok === true;
  } catch {
    return false;
  }
}

async function flashBadge(text, color) {
  await chrome.action.setBadgeBackgroundColor({ color });
  await chrome.action.setBadgeText({ text });
  setTimeout(() => chrome.action.setBadgeText({ text: "" }), 2500);
}

async function shouldCapture(item, name) {
  const s = await settings();
  if (!s.enabled || item.incognito) return false;
  if (!/^https?:/i.test(item.finalUrl || item.url)) return false;
  const { bypassUntil = 0 } = await chrome.storage.session.get("bypassUntil");
  if (bypassUntil > Date.now()) {
    await chrome.storage.session.remove("bypassUntil");
    return false;
  }
  const exts = s.extensions.toLowerCase().split(/[\s,;]+/).filter(Boolean);
  if (exts.length && !exts.includes(extOf(name))) return false;
  const size = item.totalBytes > 0 ? item.totalBytes : item.fileSize;
  if (s.minSizeMb > 0 && size > 0 && size < s.minSizeMb * 1024 * 1024) return false;
  return true;
}

chrome.downloads.onDeterminingFilename.addListener((item, suggest) => {
  (async () => {
    const name = baseName(item.filename);
    if (!(await shouldCapture(item, name))) return;
    try {
      await sendToPulse({ url: item.finalUrl || item.url, filename: name, referrer: item.referrer });
      await chrome.downloads.cancel(item.id);
      await chrome.downloads.erase({ id: item.id });
      await flashBadge("✓", "#2563eb");
    } catch (e) {
      console.warn("Could not hand off to Pulse DM, the browser downloads it:", e);
    }
  })().finally(() => {
    try {
      suggest();
    } catch {
    }
  });
  return true;
});

chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.create({ id: "pulse-link", title: chrome.i18n.getMessage("downloadWithPulse"), contexts: ["link"] });
  chrome.contextMenus.create({ id: "pulse-media", title: chrome.i18n.getMessage("downloadWithPulse"), contexts: ["video", "audio", "image"] });
});

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  const url = info.menuItemId === "pulse-link" ? info.linkUrl : info.srcUrl;
  if (!url || !/^https?:/i.test(url)) return;
  try {
    await sendToPulse({ url, referrer: tab?.url, media: isVideoSite(url) });
    await flashBadge("✓", "#2563eb");
  } catch (e) {
    console.warn(e);
    await flashBadge("!", "#dc2626");
  }
});

const header = (headers, name) => headers?.find((h) => h.name.toLowerCase() === name)?.value ?? "";
const MAX_MEDIA = 40;

async function remember(tabId, media) {
  const key = `media_${tabId}`;
  const list = (await chrome.storage.session.get(key))[key] ?? [];
  if (list.some((m) => m.url === media.url)) return;
  list.push(media);
  await chrome.storage.session.set({ [key]: list.slice(-MAX_MEDIA) });
}

async function isMasterPlaylist(url) {
  try {
    const text = await (await fetch(url, { credentials: "include" })).text();
    return text.includes("#EXT-X-STREAM-INF");
  } catch {
    return false;
  }
}

chrome.webRequest.onHeadersReceived.addListener(
  (d) => {
    if (d.tabId < 0) return;
    const type = header(d.responseHeaders, "content-type").toLowerCase();
    if (/\.m3u8(\?|$)/i.test(d.url) || type.includes("mpegurl")) {
      isMasterPlaylist(d.url).then((master) => remember(d.tabId, { url: d.url, kind: "hls", master, at: Date.now() }));
    } else if (/^(video|audio)\//.test(type) && !/\.(ts|m4s|aac)(\?|$)/i.test(d.url)) {
      remember(d.tabId, { url: d.url, kind: "file", at: Date.now() });
    }
  },
  { urls: ["<all_urls>"], types: ["xmlhttprequest", "media", "other"] },
  ["responseHeaders"],
);

chrome.tabs.onUpdated.addListener((tabId, change) => {
  if (change.status === "loading" && change.url) chrome.storage.session.remove(`media_${tabId}`);
});
chrome.tabs.onRemoved.addListener((tabId) => chrome.storage.session.remove(`media_${tabId}`));

async function downloadVideo({ src, page, permalink, title }, tab) {
  const pageUrl = permalink || page;
  const feedOnly = !permalink && /(^|\.)tiktok\.com$/.test(new URL(page).hostname) && !/\/video\/\d+/.test(page);
  if (feedOnly && /^https?:/i.test(src)) return sendToPulse({ url: src, referrer: page, filename: `${safeName(title)}.mp4` });
  if (isVideoSite(pageUrl)) return sendToPulse({ url: pageUrl, referrer: tab?.url, media: true });
  if (/^https?:/i.test(src) && !/\.m3u8(\?|$)/i.test(src)) return sendToPulse({ url: src, referrer: page });
  const key = `media_${tab?.id}`;
  const seen = ((await chrome.storage.session.get(key))[key] ?? []).slice().reverse();
  const stream = seen.find((m) => m.kind === "hls" && m.master) ?? seen.find((m) => m.kind === "hls") ?? seen[0];
  if (stream) {
    const filename = stream.kind === "hls" ? `${safeName(title)}.ts` : null;
    return sendToPulse({ url: stream.url, referrer: page, filename });
  }
  return sendToPulse({ url: pageUrl, referrer: page, media: true });
}

chrome.runtime.onMessage.addListener((msg, sender, reply) => {
  if (msg?.type === "ping") {
    ping().then(reply);
  } else if (msg?.type === "video") {
    downloadVideo(msg, sender.tab).then(
      () => reply({ ok: true }),
      (e) => reply({ ok: false, error: String(e.message || e) }),
    );
  } else {
    return false;
  }
  return true;
});
