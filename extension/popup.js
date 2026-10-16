import { DEFAULTS } from "./shared.js";

const $ = (id) => document.getElementById(id);

async function init() {
  const s = { ...DEFAULTS, ...(await chrome.storage.local.get(Object.keys(DEFAULTS))) };
  $("enabled").checked = s.enabled;
  $("extensions").value = s.extensions;
  $("minSizeMb").value = s.minSizeMb;

  $("enabled").onchange = () => chrome.storage.local.set({ enabled: $("enabled").checked });
  $("save").onclick = async () => {
    await chrome.storage.local.set({ extensions: $("extensions").value.trim(), minSizeMb: Number($("minSizeMb").value) || 0 });
    $("save").textContent = "Kaydedildi ✓";
  };
  $("bypass").onclick = async () => {
    await chrome.storage.session.set({ bypassUntil: Date.now() + 60_000 });
    $("bypass").textContent = "Sonraki indirme tarayıcıda kalacak (1 dk)";
  };

  chrome.runtime.sendMessage({ type: "ping" }, (ok) => {
    $("status").textContent = ok ? "Bağlı" : "Uygulama bulunamadı";
    $("status").className = `status ${ok ? "ok" : "err"}`;
  });
}

init();
