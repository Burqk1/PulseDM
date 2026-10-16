import { DEFAULTS } from "./shared.js";

const $ = (id) => document.getElementById(id);
const msg = (key) => chrome.i18n.getMessage(key);

document.documentElement.lang = chrome.i18n.getUILanguage();
for (const el of document.querySelectorAll("[data-i18n]")) el.textContent = msg(el.dataset.i18n);
for (const el of document.querySelectorAll("[data-i18n-html]")) el.innerHTML = msg(el.dataset.i18nHtml);

async function init() {
  const s = { ...DEFAULTS, ...(await chrome.storage.local.get(Object.keys(DEFAULTS))) };
  $("enabled").checked = s.enabled;
  $("extensions").value = s.extensions;
  $("minSizeMb").value = s.minSizeMb;

  $("enabled").onchange = () => chrome.storage.local.set({ enabled: $("enabled").checked });
  $("save").onclick = async () => {
    await chrome.storage.local.set({ extensions: $("extensions").value.trim(), minSizeMb: Number($("minSizeMb").value) || 0 });
    $("save").textContent = msg("saved");
  };
  $("bypass").onclick = async () => {
    await chrome.storage.session.set({ bypassUntil: Date.now() + 60_000 });
    $("bypass").textContent = msg("bypassed");
  };

  chrome.runtime.sendMessage({ type: "ping" }, (ok) => {
    $("status").textContent = msg(ok ? "connected" : "appNotFound");
    $("status").className = `status ${ok ? "ok" : "err"}`;
  });
}

init();
