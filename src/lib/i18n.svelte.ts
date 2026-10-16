import { tr } from "./locales/tr";

export type Language = "en" | "tr";
export type LanguagePref = "system" | Language;

export const LANGUAGES: { id: LanguagePref; label: string }[] = [
  { id: "system", label: "System" },
  { id: "en", label: "English" },
  { id: "tr", label: "Türkçe" },
];

function systemLanguage(): Language {
  const nav = typeof navigator === "undefined" ? "en" : navigator.language;
  return nav.toLowerCase().startsWith("tr") ? "tr" : "en";
}

let current = $state<Language>(systemLanguage());

export function setLanguage(pref: LanguagePref | undefined) {
  current = pref === "en" || pref === "tr" ? pref : systemLanguage();
  if (typeof document !== "undefined") document.documentElement.lang = current;
}

export const language = () => current;

export const locale = () => (current === "tr" ? "tr-TR" : "en-US");

export function queueLabel(q: { id: string; name: string }): string {
  return q.id === "main" && ["Main queue", "Ana kuyruk"].includes(q.name) ? t("Main queue") : q.name;
}

export function t(text: string, vars?: Record<string, string | number>): string {
  const out = current === "tr" ? (tr[text] ?? text) : text;
  return vars ? out.replace(/\{(\w+)\}/g, (m, k: string) => (k in vars ? String(vars[k]) : m)) : out;
}
