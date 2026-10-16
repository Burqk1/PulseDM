import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface Prefs {
  theme: "system" | "dark" | "light";
  ui_scale: number;
  relative_dates: boolean;
  tool_labels: boolean;
  progress_window: boolean;
  completion_window: boolean;
  notifications: boolean;
}

export const getPrefs = () => invoke<Prefs>("get_prefs");
export const setPrefs = (prefs: Prefs) => invoke<Prefs>("set_prefs", { prefs });

const darkQuery = typeof window !== "undefined" ? window.matchMedia("(prefers-color-scheme: dark)") : null;
let current: Prefs | null = null;

export function applyPrefs(p: Prefs) {
  current = p;
  const root = document.documentElement;
  const theme = p.theme === "system" ? (darkQuery?.matches ? "dark" : "light") : p.theme;
  root.dataset.theme = theme;
  root.style.colorScheme = theme;
  root.style.zoom = String(p.ui_scale || 1);
}

export async function followPrefs(onChange?: (p: Prefs) => void) {
  const p = await getPrefs();
  applyPrefs(p);
  onChange?.(p);
  darkQuery?.addEventListener("change", () => current && applyPrefs(current));
  return listen<Prefs>("prefs", (e) => {
    applyPrefs(e.payload);
    onChange?.(e.payload);
  });
}
