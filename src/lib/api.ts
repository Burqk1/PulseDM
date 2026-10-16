import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { language } from "./i18n.svelte";

export type Status = "queued" | "connecting" | "downloading" | "paused" | "completed" | "failed" | "processing";
export type Category = "video" | "audio" | "archive" | "program" | "document" | "image" | "other";
export type AfterAll = "nothing" | "shutdown" | "sleep";

export interface Segment {
  start: number;
  pos: number;
  end: number;
}

export interface Download {
  id: string;
  url: string;
  file_name: string;
  dir: string;
  size: number | null;
  downloaded: number;
  status: Status;
  error: string | null;
  resumable: boolean;
  speed: number;
  eta_secs: number | null;
  connections: number;
  connections_setting: number | null;
  speed_limit: number;
  queue: string;
  segments: Segment[];
  parts: [number, number] | null;
  category: Category;
  added_at: number;
  completed_at: number | null;
}

export interface Schedule {
  enabled: boolean;
  days: number[];
  start: string | null;
  stop: string | null;
}

export interface QueueConfig {
  id: string;
  name: string;
  max_active: number;
  running: boolean;
  schedule: Schedule;
}

export interface HostRule {
  host: string;
  connections: number | null;
  speed_limit: number;
  user_agent: string | null;
}

export interface Config {
  download_dir: string;
  connections: number;
  speed_limit: number;
  max_retries: number;
  min_split: number;
  user_agent: string;
  proxy: { mode: "system" | "none" | "manual"; url: string };
  host_rules: HostRule[];
  queues: QueueConfig[];
  category_folders: boolean;
  keep_server_time: boolean;
  ignore_tls_errors: boolean;
}

export interface AddRequest {
  url: string;
  dir?: string | null;
  file_name?: string | null;
  headers?: [string, string][];
  paused?: boolean;
  queue?: string | null;
  connections?: number | null;
  speed_limit?: number | null;
  username?: string | null;
  password?: string | null;
  media_option?: string | null;
  title?: string | null;
}

export interface MediaOption {
  id: string;
  label: string;
  height: number | null;
  audio_only: boolean;
  size: number | null;
}

export interface MediaInfo {
  url: string;
  title: string;
  thumbnail: string | null;
  duration: number | null;
  uploader: string | null;
  site: string | null;
  options: MediaOption[];
}

export interface AddDraft {
  url: string;
  headers: [string, string][];
  file_name: string | null;
  media: boolean;
}

export interface UrlInfo {
  file_name: string;
  size: number | null;
  resumable: boolean;
  html: boolean;
}

export interface ToolProgress {
  name: string;
  downloaded: number;
  total: number | null;
  done: boolean;
}

export interface ChecksumResult {
  algorithm: string;
  actual: string;
  matches: boolean | null;
}

export type EngineEvent =
  | { type: "progress"; data: Download[] }
  | { type: "changed"; data: Download }
  | { type: "removed"; data: string }
  | { type: "config"; data: Config }
  | { type: "tool"; data: ToolProgress };

export interface UpdateInfo {
  version: string;
  notes: string | null;
}

export const api = {
  list: () => invoke<Download[]>("list_downloads"),
  add: (request: AddRequest) => invoke<Download>("add_download", { request }),
  pause: (id: string) => invoke<void>("pause_download", { id }),
  resume: (id: string) => invoke<void>("resume_download", { id }),
  remove: (id: string, deleteFiles: boolean) => invoke<void>("remove_download", { id, deleteFiles }),
  pauseAll: () => invoke<void>("pause_all"),
  resumeAll: () => invoke<void>("resume_all"),
  clearCompleted: () => invoke<void>("clear_completed"),
  getConfig: () => invoke<Config>("get_config"),
  setConfig: (config: Config) => invoke<Config>("set_config", { config }),
  setUrl: (id: string, url: string) => invoke<void>("set_url", { id, url }),
  setOptions: (id: string, options: { connections: number | null; speed_limit: number }) =>
    invoke<void>("set_options", { id, options }),
  moveToQueue: (ids: string[], queue: string) => invoke<void>("move_to_queue", { ids, queue }),
  startQueue: (queue: string) => invoke<void>("start_queue", { queue }),
  stopQueue: (queue: string) => invoke<void>("stop_queue", { queue }),
  verifyChecksum: (id: string, algorithm: string | null, expected: string | null) =>
    invoke<ChecksumResult>("verify_checksum", { id, algorithm, expected }),
  getAfterAll: () => invoke<AfterAll>("get_after_all"),
  setAfterAll: (mode: AfterAll) => invoke<void>("set_after_all", { mode }),
  cancelShutdown: () => invoke<void>("cancel_shutdown"),
  openFile: (id: string) => invoke<void>("open_file", { id }),
  showInFolder: (id: string) => invoke<void>("show_in_folder", { id }),
  openExtensionFolder: () => invoke<string>("open_extension_folder"),
  quit: () => invoke<void>("quit_app"),
  probeMedia: (url: string, headers: [string, string][] = []) => invoke<MediaInfo>("probe_media", { url, headers }),
  probeUrl: (url: string, headers: [string, string][] = []) => invoke<UrlInfo>("probe_url", { url, headers }),
  takeAddDraft: () => invoke<AddDraft | null>("take_add_draft"),
  openAddWindow: (draft: Partial<AddDraft> = {}) =>
    invoke<void>("open_add_window", { draft: { url: "", headers: [], file_name: null, media: false, ...draft } }),
  openProgressWindow: (id: string) => invoke<void>("open_progress_window", { id }),
  checkForUpdates: () => invoke<UpdateInfo | null>("check_for_updates"),
  pendingUpdate: () => invoke<UpdateInfo | null>("pending_update"),
  installUpdate: () => invoke<void>("install_update"),
  onEvent: (cb: (e: EngineEvent) => void): Promise<UnlistenFn> => listen<EngineEvent>("engine", (e) => cb(e.payload)),
  on: <T>(name: string, cb: (payload: T) => void): Promise<UnlistenFn> => listen<T>(name, (e) => cb(e.payload)),
};

const UNITS = ["B", "KB", "MB", "GB", "TB"];

export function formatBytes(n: number | null | undefined): string {
  if (n == null) return "?";
  let i = 0;
  let v = n;
  while (v >= 1024 && i < UNITS.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(i === 0 || v >= 100 ? 0 : 1)} ${UNITS[i]}`;
}

export function formatSpeed(bps: number): string {
  return `${formatBytes(bps)}/s`;
}

export function formatEta(secs: number | null): string {
  if (secs == null) return "";
  const [s, mi, hr, d] = language() === "tr" ? [" sn", " dk", " sa", " gün"] : ["s", "m", "h", "d"];
  if (secs < 60) return `${secs}${s}`;
  const m = Math.floor(secs / 60);
  if (m < 60) return `${m}${mi} ${secs % 60}${s}`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}${hr} ${m % 60}${mi}`;
  return `${Math.floor(h / 24)}${d} ${h % 24}${hr}`;
}

export function extractUrls(text: string): string[] {
  return [...new Set(text.match(/https?:\/\/.+?(?=https?:\/\/|[\s"'<>]|$)/g) ?? [])];
}

export const MB = 1024 * 1024;

const VIDEO_SITES = [
  "youtube.com", "youtu.be", "x.com", "twitter.com", "instagram.com", "tiktok.com", "vimeo.com",
  "dailymotion.com", "twitch.tv", "facebook.com", "fb.watch", "reddit.com", "soundcloud.com", "bilibili.com",
  "kick.com", "streamable.com", "ok.ru", "vk.com",
];

export function isVideoSite(url: string): boolean {
  try {
    const host = new URL(url).hostname.toLowerCase();
    return VIDEO_SITES.some((s) => host === s || host.endsWith(`.${s}`));
  } catch {
    return false;
  }
}

export function formatDuration(secs: number | null): string {
  if (!secs) return "";
  const s = Math.round(secs);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const pad = (n: number) => String(n).padStart(2, "0");
  return h ? `${h}:${pad(m)}:${pad(s % 60)}` : `${m}:${pad(s % 60)}`;
}
