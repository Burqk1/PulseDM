use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use futures_util::StreamExt;
use reqwest::Client;
use serde::Serialize;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex as AsyncMutex;

use crate::error::{EngineError, Result};

const YTDLP_URL: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe";
const FFMPEG_URL: &str =
    "https://github.com/yt-dlp/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl-shared.zip";
const DENO_URL: &str = "https://github.com/denoland/deno/releases/latest/download/deno-x86_64-pc-windows-msvc.zip";
const YTDLP_MAX_AGE: Duration = Duration::from_secs(3 * 24 * 3600);

#[derive(Debug, Clone, Serialize)]
pub struct ToolProgress {
    pub name: &'static str,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub done: bool,
}

pub(crate) struct Tools {
    dir: PathBuf,
    client: Client,
    lock: AsyncMutex<()>,
}

pub(crate) type Progress<'a> = &'a (dyn Fn(ToolProgress) + Send + Sync);

impl Tools {
    pub fn new(dir: PathBuf, client: Client) -> Self {
        Self { dir, client, lock: AsyncMutex::new(()) }
    }

    pub async fn ytdlp(&self, progress: Progress<'_>) -> Result<PathBuf> {
        let _guard = self.lock.lock().await;
        let exe = self.dir.join("yt-dlp.exe");
        if !exe.exists() {
            self.fetch(YTDLP_URL, &exe, "yt-dlp", progress).await?;
        } else if age(&exe).is_some_and(|a| a > YTDLP_MAX_AGE) {
            let _ = hidden(&exe).arg("-U").output().await;
            touch(&exe);
        }
        Ok(exe)
    }

    pub async fn ffmpeg_dir(&self, progress: Progress<'_>) -> Result<PathBuf> {
        let _guard = self.lock.lock().await;
        let bin = self.dir.join("ffmpeg");
        if !bin.join("ffmpeg.exe").exists() {
            let zip = self.dir.join("ffmpeg.zip");
            self.fetch(FFMPEG_URL, &zip, "ffmpeg", progress).await?;
            extract_matching(&zip, &bin, |p| p.contains("/bin/") && (p.ends_with(".exe") || p.ends_with(".dll")))
                .await?;
            let _ = std::fs::remove_file(&zip);
        }
        Ok(bin)
    }

    pub async fn js_runtime(&self, progress: Progress<'_>) -> Result<String> {
        if let Some(node) = find_in_path("node.exe") {
            return Ok(format!("node:{}", node.display()));
        }
        let _guard = self.lock.lock().await;
        let deno = self.dir.join("deno.exe");
        if !deno.exists() {
            let zip = self.dir.join("deno.zip");
            self.fetch(DENO_URL, &zip, "Deno", progress).await?;
            extract_matching(&zip, &self.dir, |p| p.ends_with("deno.exe")).await?;
            let _ = std::fs::remove_file(&zip);
        }
        Ok(format!("deno:{}", deno.display()))
    }

    async fn fetch(&self, url: &str, dest: &Path, name: &'static str, progress: Progress<'_>) -> Result<()> {
        tokio::fs::create_dir_all(&self.dir).await?;
        let resp = self.client.get(url).send().await?;
        if !resp.status().is_success() {
            return Err(EngineError::BadResponse(format!("{name} indirilemedi: HTTP {}", resp.status().as_u16())));
        }
        let total = resp.content_length();
        let tmp = dest.with_extension("download");
        let mut file = tokio::fs::File::create(&tmp).await?;
        let mut stream = resp.bytes_stream();
        let mut downloaded = 0u64;
        let mut last_report = 0u64;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            file.write_all(&chunk).await?;
            downloaded += chunk.len() as u64;
            if downloaded - last_report > 512 * 1024 {
                last_report = downloaded;
                progress(ToolProgress { name, downloaded, total, done: false });
            }
        }
        file.flush().await?;
        drop(file);
        tokio::fs::rename(&tmp, dest).await?;
        progress(ToolProgress { name, downloaded, total, done: true });
        Ok(())
    }
}

pub(crate) fn hidden(program: &Path) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(program);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    cmd.kill_on_drop(true);
    cmd
}

fn age(path: &Path) -> Option<Duration> {
    SystemTime::now().duration_since(std::fs::metadata(path).ok()?.modified().ok()?).ok()
}

fn touch(path: &Path) {
    if let Ok(f) = std::fs::OpenOptions::new().append(true).open(path) {
        let _ = f.set_modified(SystemTime::now());
    }
}

fn find_in_path(exe: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(exe)).find(|p| p.is_file())
}

async fn extract_matching(zip: &Path, dest: &Path, wanted: impl Fn(&str) -> bool + Send + 'static) -> Result<()> {
    let (zip, dest) = (zip.to_path_buf(), dest.to_path_buf());
    tokio::task::spawn_blocking(move || -> Result<()> {
        let bad = |e: zip::result::ZipError| EngineError::BadResponse(format!("arşiv açılamadı: {e}"));
        let mut archive = zip::ZipArchive::new(std::fs::File::open(&zip)?).map_err(bad)?;
        std::fs::create_dir_all(&dest)?;
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i).map_err(bad)?;
            let path = entry.name().replace('\\', "/");
            if entry.is_dir() || !wanted(&path) {
                continue;
            }
            let Some(name) = path.rsplit('/').next() else {
                continue;
            };
            let mut out = std::fs::File::create(dest.join(name))?;
            std::io::copy(&mut entry, &mut out)?;
        }
        Ok(())
    })
    .await
    .expect("extract task panicked")
}
