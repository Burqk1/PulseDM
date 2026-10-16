export const DEFAULTS = {
  enabled: true,
  minSizeMb: 0,
  extensions:
    "exe msi msix zip rar 7z gz xz bz2 tar iso img dmg pkg deb rpm apk " +
    "mp4 mkv avi mov wmv webm flv m4v mp3 flac wav aac m4a ogg opus " +
    "pdf epub docx xlsx pptx bin",
};

const VIDEO_SITES = [
  "youtube.com", "youtu.be", "x.com", "twitter.com", "instagram.com", "tiktok.com", "vimeo.com",
  "dailymotion.com", "twitch.tv", "facebook.com", "fb.watch", "reddit.com", "soundcloud.com", "bilibili.com",
  "kick.com", "streamable.com", "ok.ru", "vk.com",
];

export function isVideoSite(url) {
  try {
    const host = new URL(url).hostname.toLowerCase();
    return VIDEO_SITES.some((s) => host === s || host.endsWith(`.${s}`));
  } catch {
    return false;
  }
}
