use std::future::Future;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use librqbit::dht::DhtPersistenceConfig;
use librqbit::limits::LimitsConfig;
use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, DhtSessionConfig, ListenerOptions, Session, SessionOptions,
};
use reqwest::Client;
use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};
use tokio::runtime::Handle;

use crate::error::{EngineError, Result};
use crate::lang::text;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TorrentState {
    pub source: String,
    #[serde(default)]
    pub info_hash: Option<String>,
    #[serde(default)]
    pub in_folder: bool,
    #[serde(default)]
    pub uploaded: u64,
}

pub struct Metadata {
    pub name: Option<String>,
    pub multi_file: bool,
}

pub fn is_magnet(source: &str) -> bool {
    source.trim().get(..7).is_some_and(|s| s.eq_ignore_ascii_case("magnet:"))
}

fn is_remote(source: &str) -> bool {
    let s = source.trim().to_ascii_lowercase();
    s.starts_with("http://") || s.starts_with("https://")
}

pub fn is_source(source: &str) -> bool {
    let s = source.trim();
    is_magnet(s) || s.split(['?', '#']).next().unwrap_or(s).to_ascii_lowercase().ends_with(".torrent")
}

pub fn local_path(source: &str) -> Option<PathBuf> {
    let s = source.trim();
    if is_magnet(s) || is_remote(s) {
        return None;
    }
    if s.get(..7).is_some_and(|p| p.eq_ignore_ascii_case("file://")) {
        return reqwest::Url::parse(s).ok()?.to_file_path().ok();
    }
    Some(PathBuf::from(s))
}

pub fn display_name(source: &str) -> String {
    let s = source.trim();
    if is_magnet(s) {
        let url = reqwest::Url::parse(s).ok();
        let param = |key: &str| {
            url.as_ref()?
                .query_pairs()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        if let Some(name) = param("dn") {
            return name;
        }
        let hash = param("xt").and_then(|xt| xt.rsplit(':').next().map(|h| h.chars().take(8).collect::<String>()));
        return format!("magnet-{}", hash.unwrap_or_default());
    }
    let path = s.split(['?', '#']).next().unwrap_or(s);
    let last = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let last = percent_encoding::percent_decode_str(last).decode_utf8_lossy().into_owned();
    match last.len().checked_sub(8).filter(|&i| last[i..].eq_ignore_ascii_case(".torrent")) {
        Some(i) if i > 0 => last[..i].to_string(),
        _ => "torrent".into(),
    }
}

pub fn metadata(bytes: &[u8]) -> Result<Metadata> {
    let meta = librqbit::torrent_from_bytes(bytes).map_err(|_| invalid())?;
    let info = &meta.info.data;
    let name =
        info.name.as_ref().map(|n| String::from_utf8_lossy(n.as_ref()).trim().to_string()).filter(|n| !n.is_empty());
    Ok(Metadata { name, multi_file: info.files.is_some() })
}

fn invalid() -> EngineError {
    EngineError::BadResponse(text!("this is not a valid .torrent file", "bu geçerli bir .torrent dosyası değil"))
}

pub fn failed(e: impl std::fmt::Display) -> EngineError {
    EngineError::BadResponse(format!("BitTorrent: {e:#}"))
}

pub fn bps(limit: u64) -> Option<NonZeroU32> {
    NonZeroU32::new(limit.min(u32::MAX as u64) as u32)
}

pub async fn session(download_dir: &Path, state_dir: &Path, download_limit: u64) -> Result<Arc<Session>> {
    std::fs::create_dir_all(state_dir)?;
    let opts = SessionOptions {
        dht: Some(DhtSessionConfig {
            persistence: Some(DhtPersistenceConfig {
                config_filename: Some(state_dir.join("dht.json")),
                ..Default::default()
            }),
            ..Default::default()
        }),
        listen: Some(ListenerOptions { enable_upnp_port_forwarding: true, ..Default::default() }),
        ratelimits: LimitsConfig { download_bps: bps(download_limit), upload_bps: None },
        ..Default::default()
    };
    Session::new_with_opts(download_dir.to_path_buf(), opts).await.map_err(failed)
}

pub const IDLE_SHUTDOWN: Duration = Duration::from_secs(60);

#[derive(Default)]
struct HubState {
    session: Option<Arc<Session>>,
    users: usize,
    generation: u64,
}

pub struct TorrentHub {
    state: tokio::sync::Mutex<HubState>,
    idle: Duration,
    rt: Handle,
}

pub struct SessionLease {
    session: Arc<Session>,
    hub: Arc<TorrentHub>,
}

impl std::ops::Deref for SessionLease {
    type Target = Arc<Session>;

    fn deref(&self) -> &Arc<Session> {
        &self.session
    }
}

impl Drop for SessionLease {
    fn drop(&mut self) {
        self.hub.release();
    }
}

impl TorrentHub {
    pub fn new(rt: Handle, idle: Duration) -> Arc<Self> {
        Arc::new(Self { state: tokio::sync::Mutex::new(HubState::default()), idle, rt })
    }

    pub async fn acquire(self: &Arc<Self>, start: impl Future<Output = Result<Arc<Session>>>) -> Result<SessionLease> {
        let mut st = self.state.lock().await;
        let session = match &st.session {
            Some(session) => session.clone(),
            None => {
                let session = start.await?;
                st.session = Some(session.clone());
                session
            }
        };
        st.users += 1;
        st.generation += 1;
        Ok(SessionLease { session, hub: self.clone() })
    }

    fn release(self: &Arc<Self>) {
        let hub = self.clone();
        self.rt.spawn(async move {
            let generation = {
                let mut st = hub.state.lock().await;
                st.users = st.users.saturating_sub(1);
                if st.users > 0 {
                    return;
                }
                st.generation
            };
            tokio::time::sleep(hub.idle).await;
            let idle = {
                let mut st = hub.state.lock().await;
                if st.users > 0 || st.generation != generation { None } else { st.session.take() }
            };
            if let Some(session) = idle {
                session.stop().await;
            }
        });
    }

    pub async fn is_running(&self) -> bool {
        self.state.lock().await.session.is_some()
    }

    pub async fn shutdown(&self) {
        let session = self.state.lock().await.session.take();
        if let Some(session) = session {
            session.stop().await;
        }
    }
}

pub async fn fetch(session: &Arc<Session>, source: &str, client: &Client, headers: &HeaderMap) -> Result<Vec<u8>> {
    if is_magnet(source) {
        let opts = AddTorrentOptions { list_only: true, ..Default::default() };
        return match session.add_torrent(AddTorrent::from_url(source.trim()), Some(opts)).await.map_err(failed)? {
            AddTorrentResponse::ListOnly(r) => Ok(r.torrent_bytes.to_vec()),
            _ => Err(invalid()),
        };
    }
    if is_remote(source) {
        let resp = client.get(source.trim()).headers(headers.clone()).send().await?.error_for_status()?;
        return Ok(resp.bytes().await?.to_vec());
    }
    let path = local_path(source).ok_or_else(invalid)?;
    Ok(std::fs::read(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_sources() {
        assert!(is_source("magnet:?xt=urn:btih:abc"));
        assert!(is_source("MAGNET:?xt=urn:btih:abc"));
        assert!(is_source("https://example.com/files/ubuntu.iso.torrent?x=1"));
        assert!(is_source(r"C:\Users\a\Downloads\file.torrent"));
        assert!(!is_source("https://example.com/file.zip"));
        assert!(!is_source("https://example.com/torrent"));
    }

    #[test]
    fn names_from_sources() {
        assert_eq!(display_name("magnet:?xt=urn:btih:0123456789abcdef&dn=Big%20Buck%20Bunny"), "Big Buck Bunny");
        assert_eq!(display_name("magnet:?xt=urn:btih:0123456789abcdef"), "magnet-01234567");
        assert_eq!(display_name("https://x.org/a/Sintel%20Movie.torrent?dl=1"), "Sintel Movie");
        assert_eq!(display_name(r"C:\dl\linux.iso.torrent"), "linux.iso");
    }

    #[test]
    fn local_paths() {
        assert_eq!(local_path(r"C:\dl\a.torrent"), Some(PathBuf::from(r"C:\dl\a.torrent")));
        assert_eq!(local_path("magnet:?xt=urn:btih:abc"), None);
        assert_eq!(local_path("https://x.org/a.torrent"), None);
    }

    #[test]
    fn rejects_garbage_metadata() {
        assert!(metadata(b"not a torrent").is_err());
    }

    async fn offline_session(dir: &Path) -> Result<Arc<Session>> {
        let opts = SessionOptions {
            dht: None,
            listen: None,
            disable_local_service_discovery: true,
            disable_trackers: true,
            ..Default::default()
        };
        Session::new_with_opts(dir.to_path_buf(), opts).await.map_err(failed)
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn hub_stops_the_session_only_when_idle() {
        let dir = std::env::temp_dir().join(format!("pulse-hub-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let idle = Duration::from_millis(200);
        let hub = TorrentHub::new(Handle::current(), idle);
        let wait = || tokio::time::sleep(idle * 3);

        let first = hub.acquire(offline_session(&dir)).await.unwrap();
        let second = hub.acquire(offline_session(&dir)).await.unwrap();
        assert!(Arc::ptr_eq(&first, &second), "both users share one session");

        drop(first);
        wait().await;
        assert!(hub.is_running().await, "still in use by the second download");

        drop(second);
        tokio::time::sleep(idle / 4).await;
        let again = hub.acquire(offline_session(&dir)).await.unwrap();
        wait().await;
        assert!(hub.is_running().await, "a new download during the grace period keeps it alive");

        drop(again);
        wait().await;
        assert!(!hub.is_running().await, "stopped after the grace period with no users");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
