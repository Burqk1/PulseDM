
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

const action = chrome.action ?? chrome.browserAction;

async function flashBadge(text, color) {
  await action.setBadgeBackgroundColor({ color });
  await action.setBadgeText({ text });
  setTimeout(() => action.setBadgeText({ text: "" }), 2500);
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

async function capture(item, name) {
  if (!(await shouldCapture(item, name))) return false;
  try {
    await sendToPulse({ url: item.finalUrl || item.url, filename: name, referrer: item.referrer });
    await chrome.downloads.cancel(item.id);
    await chrome.downloads.erase({ id: item.id });
    await flashBadge("✓", "#2563eb");
    return true;
  } catch (e) {
    console.warn("Could not hand off to Pulse DM, the browser downloads it:", e);
    return false;
  }
}

const urlName = (url) => {
  try {
    return decodeURIComponent(baseName(new URL(url).pathname));
  } catch {
    return "";
  }
};

function dispositionName(value) {
  const star = /filename\*\s*=\s*[^']*'[^']*'([^;]+)/i.exec(value);
  if (star) {
    try {
      return baseName(decodeURIComponent(star[1].trim().replace(/^"|"$/g, "")));
    } catch {
    }
  }
  const plain = /filename\s*=\s*(?:"([^"]*)"|([^;]+))/i.exec(value);
  return baseName((plain?.[1] ?? plain?.[2] ?? "").trim());
}

const INLINE_TYPES = /^(text\/|image\/|video\/|audio\/|multipart\/|application\/(pdf|json|xml|javascript|x-javascript|ecmascript|xhtml\+xml|rss\+xml|atom\+xml)\b)|\+(xml|json)\b/;
const HANDLED_FOR_MS = 15_000;
const handled = new Map();

function markHandled(url) {
  const now = Date.now();
  handled.set(url, now);
  for (const [u, at] of handled) if (now - at > HANDLED_FOR_MS) handled.delete(u);
}

const handledRecently = (url) => Date.now() - (handled.get(url) ?? 0) < HANDLED_FOR_MS;

function captureResponses() {
  chrome.webRequest.onHeadersReceived.addListener(
    async (d) => {
      if (d.method !== "GET" || d.statusCode < 200 || d.statusCode >= 300 || d.statusCode === 204 || d.statusCode === 205) {
        return {};
      }
      const disposition = header(d.responseHeaders, "content-disposition");
      const type = header(d.responseHeaders, "content-type").toLowerCase().trim();
      const attachment = /^\s*attachment\b/i.test(disposition);
      if (!attachment && (!type || INLINE_TYPES.test(type))) return {};
      const name = dispositionName(disposition) || urlName(d.url);
      const item = { url: d.url, incognito: d.incognito, totalBytes: Number(header(d.responseHeaders, "content-length")) || 0 };
      if (!(await shouldCapture(item, name))) return {};
      try {
        await sendToPulse({ url: d.url, filename: name, referrer: d.originUrl || d.documentUrl });
        markHandled(d.url);
        flashBadge("✓", "#2563eb");
        return { cancel: true };
      } catch (e) {
        console.warn("Could not hand off to Pulse DM, the browser downloads it:", e);
        return {};
      }
    },
    { urls: ["<all_urls>"], types: ["main_frame", "sub_frame"] },
    ["blocking", "responseHeaders"],
  );
}

if (chrome.downloads.onDeterminingFilename) {
  chrome.downloads.onDeterminingFilename.addListener((item, suggest) => {
    capture(item, baseName(item.filename)).finally(() => {
      try {
        suggest();
      } catch {
      }
    });
    return true;
  });
} else {
  captureResponses();
  chrome.downloads.onCreated.addListener(async (item) => {
    if (!/^https?:/i.test(item.url) || item.byExtensionId) return;
    if (handledRecently(item.url)) {
      await chrome.downloads.cancel(item.id).catch(() => {});
      await chrome.downloads.erase({ id: item.id }).catch(() => {});
      return;
    }
    if (Date.now() - new Date(item.startTime).getTime() > 10_000) return;
    if (await capture(item, baseName(item.filename) || urlName(item.url))) markHandled(item.url);
  });
}

chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.create({ id: "pulse-link", title: chrome.i18n.getMessage("downloadWithPulse"), contexts: ["link"] });
  chrome.contextMenus.create({ id: "pulse-media", title: chrome.i18n.getMessage("downloadWithPulse"), contexts: ["video", "audio", "image"] });
});

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  const url = info.menuItemId === "pulse-link" ? info.linkUrl : info.srcUrl;
  if (!url || !/^(https?|magnet):/i.test(url)) return;
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

async function redditStream(stream, tab) {
  if (stream) return stream;
  const key = `media_${tab?.id}`;
  const seen = ((await chrome.storage.session.get(key))[key] ?? []).slice().reverse();
  const reddit = seen.filter((m) => m.kind === "hls" && /^https:\/\/v\.redd\.it\//.test(m.url));
  return (reddit.find((m) => m.master) ?? reddit[0])?.url ?? null;
}

async function downloadVideo({ src, page, permalink, title, stream: playerStream, postTitle }, tab) {
  if (/(^|\.)reddit\.com$/.test(new URL(page).hostname)) {
    const hls = await redditStream(playerStream, tab);
    if (hls) return sendToPulse({ url: hls, referrer: page, media: true, filename: safeName(postTitle || title) });
  }
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
  } else if (msg?.type === "magnet" && /^magnet:/i.test(msg.url)) {
    sendToPulse({ url: msg.url, referrer: sender.tab?.url }).then(
      () => {
        flashBadge("✓", "#2563eb");
        reply({ ok: true });
      },
      (e) => reply({ ok: false, error: String(e.message || e) }),
    );
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
