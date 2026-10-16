use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use librqbit::api::TorrentIdOrHash;
use librqbit::limits::LimitsConfig;
use librqbit::{AddTorrent, AddTorrentOptions, AddTorrentResponse, Session, TorrentStatsState};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, USER_AGENT};
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use tokio::runtime::Handle;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::category::category_of;
use crate::download::{self, Job, JobControl};
use crate::error::{EngineError, Result};
use crate::fsutil::{
    create_part_file, free_space, human_bytes, open_part_file, part_path, sanitize_file_name, unique_name,
};
use crate::hls::{self, HlsState};
use crate::lang::text;
use crate::limiter::RateLimiter;
use crate::media::{self, MediaInfo, MediaState, Protocol, StreamState};
use crate::probe::{expects_html, name_from_url, probe};
use crate::schedule::{QueueConfig, ScheduleClock};
use crate::segment::{SegmentState, SegmentTable};
use crate::tools::{ToolProgress, Tools};
use crate::torrent::{self, TorrentState};

pub const PART_EXT: &str = "pdm";
pub const MAIN_QUEUE: &str = "main";

const TICK: Duration = Duration::from_millis(500);
const DISK_MARGIN: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyMode {
    #[default]
    System,
    None,
    Manual,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProxyConfig {
    pub mode: ProxyMode,
    pub url: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HostRule {
    pub host: String,
    pub connections: Option<usize>,
    pub speed_limit: u64,
    pub user_agent: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub download_dir: PathBuf,
    pub connections: usize,
    pub speed_limit: u64,
    pub max_retries: u32,
    pub min_split: u64,
    pub max_auto_connections: usize,
    pub user_agent: String,
    pub proxy: ProxyConfig,
    pub host_rules: Vec<HostRule>,
    pub category_folders: bool,
    pub keep_server_time: bool,
    pub ignore_tls_errors: bool,
    pub queues: Vec<QueueConfig>,
}

impl Default for Config {
    fn default() -> Self {
        let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from);
        Self {
            download_dir: home.map(|h| h.join("Downloads").join("Pulse DM")).unwrap_or_else(std::env::temp_dir),
            connections: 8,
            speed_limit: 0,
            max_retries: 8,
            min_split: 1024 * 1024,
            max_auto_connections: 16,
            user_agent: concat!("PulseDM/", env!("CARGO_PKG_VERSION")).into(),
            proxy: ProxyConfig::default(),
            host_rules: Vec::new(),
            category_folders: true,
            keep_server_time: false,
            ignore_tls_errors: false,
            queues: vec![QueueConfig::main()],
        }
    }
}

impl Config {
    fn normalized(mut self) -> Self {
        self.connections = self.connections.clamp(1, 32);
        self.min_split = self.min_split.max(64 * 1024);
        if !self.queues.iter().any(|q| q.id == MAIN_QUEUE) {
            self.queues.insert(0, QueueConfig::main());
        }
        for q in &mut self.queues {
            q.max_active = q.max_active.clamp(1, 20);
        }
        self
    }

    fn rule_for(&self, url: &str) -> Option<&HostRule> {
        let host = Url::parse(url).ok()?.host_str()?.to_ascii_lowercase();
        self.host_rules.iter().find(|r| {
            let h = r.host.trim().to_ascii_lowercase();
            !h.is_empty() && (host == h || host.ends_with(&format!(".{h}")))
        })
    }

    fn queue_exists(&self, id: &str) -> bool {
        self.queues.iter().any(|q| q.id == id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Queued,
    Connecting,
    Downloading,
    Paused,
    Completed,
    Failed,
    Processing,
}

impl Status {
    fn is_live(self) -> bool {
        matches!(self, Status::Queued | Status::Connecting | Status::Downloading | Status::Processing)
    }
}

fn main_queue() -> String {
    MAIN_QUEUE.into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadRecord {
    pub id: Uuid,
    pub url: String,
    #[serde(default)]
    pub final_url: Option<String>,
    #[serde(default)]
    pub url_changed: bool,
    pub file_name: String,
    #[serde(default)]
    pub custom_name: bool,
    pub dir: PathBuf,
    pub size: Option<u64>,
    pub downloaded: u64,
    pub status: Status,
    pub error: Option<String>,
    pub resumable: bool,
    pub validator: Option<String>,
    #[serde(default)]
    pub last_modified: Option<String>,
    pub segments: Vec<SegmentState>,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    #[serde(default = "main_queue")]
    pub queue: String,
    #[serde(default)]
    pub connections: Option<usize>,
    #[serde(default)]
    pub speed_limit: u64,
    #[serde(default)]
    pub hls: Option<HlsState>,
    #[serde(default)]
    pub media: Option<MediaState>,
    #[serde(default)]
    pub torrent: Option<TorrentState>,
    pub added_at: u64,
    pub completed_at: Option<u64>,
}

impl DownloadRecord {
    fn owns_part(&self) -> bool {
        self.final_url.is_some()
            || self.size.is_some()
            || !self.segments.is_empty()
            || self.hls.is_some()
            || self.media.as_ref().is_some_and(|m| !m.streams.is_empty())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DownloadView {
    pub id: Uuid,
    pub url: String,
    pub file_name: String,
    pub dir: PathBuf,
    pub size: Option<u64>,
    pub downloaded: u64,
    pub status: Status,
    pub error: Option<String>,
    pub resumable: bool,
    pub speed: u64,
    pub eta_secs: Option<u64>,
    pub connections: usize,
    pub connections_setting: Option<usize>,
    pub speed_limit: u64,
    pub queue: String,
    pub segments: Vec<SegmentState>,
    pub parts: Option<(usize, usize)>,
    pub category: &'static str,
    pub torrent: bool,
    pub added_at: u64,
    pub completed_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum EngineEvent {
    Progress(Vec<DownloadView>),
    Changed(DownloadView),
    Removed(Uuid),
    Config(Config),
    Tool(ToolProgress),
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct AddRequest {
    pub url: String,
    pub dir: Option<PathBuf>,
    pub file_name: Option<String>,
    pub headers: Vec<(String, String)>,
    pub paused: bool,
    pub queue: Option<String>,
    pub connections: Option<usize>,
    pub speed_limit: Option<u64>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub media_option: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UrlInfo {
    pub file_name: String,
    pub size: Option<u64>,
    pub resumable: bool,
    pub html: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct DownloadOptions {
    pub connections: Option<usize>,
    pub speed_limit: u64,
}

struct Entry {
    rec: DownloadRecord,
    run: Option<Running>,
    speed: f64,
    last_bytes: u64,
    remove_after: Option<bool>,
}

struct Running {
    cancel: CancellationToken,
    control: Arc<JobControl>,
    table: Option<Arc<SegmentTable>>,
    hls: Option<Arc<Mutex<HlsState>>>,
    media_base: Arc<AtomicU64>,
    media_stream: Option<usize>,
    peers: Option<Arc<AtomicUsize>>,
    requeue: bool,
}

impl Entry {
    fn new(rec: DownloadRecord) -> Self {
        Self { rec, run: None, speed: 0.0, last_bytes: 0, remove_after: None }
    }

    fn live_table(&self) -> Option<&Arc<SegmentTable>> {
        self.run.as_ref()?.table.as_ref()
    }

    fn live_downloaded(&self) -> Option<u64> {
        let run = self.run.as_ref()?;
        let table = run.table.as_ref()?.downloaded();
        Some(if self.rec.media.is_some() { run.media_base.load(Ordering::Relaxed) + table } else { table })
    }

    fn hls_state(&self) -> Option<HlsState> {
        match self.run.as_ref().and_then(|r| r.hls.as_ref()) {
            Some(live) => Some(live.lock().unwrap().clone()),
            None => self.rec.hls.clone(),
        }
    }

    fn view(&self) -> DownloadView {
        let r = &self.rec;
        let table = self.live_table();
        let downloaded = self.live_downloaded().unwrap_or(r.downloaded);
        let speed = self.speed as u64;
        let eta_secs = match (r.size, speed) {
            (Some(size), s) if s > 0 => Some(size.saturating_sub(downloaded) / s),
            _ => None,
        };
        DownloadView {
            id: r.id,
            url: r.url.clone(),
            file_name: r.file_name.clone(),
            dir: r.dir.clone(),
            size: r.size,
            downloaded,
            status: r.status,
            error: r.error.clone(),
            resumable: r.resumable,
            speed,
            eta_secs,
            connections: match self.run.as_ref().and_then(|r| r.peers.as_ref()) {
                Some(peers) => peers.load(Ordering::Relaxed),
                None => table.map(|t| t.active_connections()).unwrap_or(0),
            },
            connections_setting: r.connections,
            speed_limit: r.speed_limit,
            queue: r.queue.clone(),
            segments: if r.hls.is_some() || r.media.is_some() || r.torrent.is_some() {
                Vec::new()
            } else {
                table.map(|t| t.snapshot()).unwrap_or_else(|| r.segments.clone())
            },
            parts: if r.media.is_some() { None } else { self.hls_state().map(|h| (h.written, h.total)) },
            category: if r.torrent.as_ref().is_some_and(|t| t.in_folder) { "other" } else { category_of(&r.file_name) },
            torrent: r.torrent.is_some(),
            added_at: r.added_at,
            completed_at: r.completed_at,
        }
    }

    fn persisted(&self) -> DownloadRecord {
        let mut r = self.rec.clone();
        if let (Some(m), Some(run)) = (&mut r.media, &self.run) {
            if let Some(st) = run.media_stream.and_then(|i| m.streams.get_mut(i)) {
                if let Some(h) = &run.hls {
                    st.hls = Some(h.lock().unwrap().clone());
                } else if let Some(t) = &run.table {
                    st.segments = t.snapshot();
                }
            }
            r.downloaded = self.live_downloaded().unwrap_or(r.downloaded);
        } else if r.torrent.is_some() {
            r.downloaded = self.live_downloaded().unwrap_or(r.downloaded);
        } else if r.hls.is_some() {
            r.hls = self.hls_state();
            r.downloaded = r.hls.as_ref().map(|h| h.out_size).unwrap_or(0);
        } else if let Some(t) = self.live_table() {
            r.segments = t.snapshot();
            r.downloaded = t.downloaded();
        }
        r
    }
}

struct Inner {
    rt: Handle,
    client: RwLock<Client>,
    limiter: Arc<RateLimiter>,
    host_limiters: Mutex<HashMap<String, Arc<RateLimiter>>>,
    config: RwLock<Config>,
    entries: Mutex<Vec<Entry>>,
    events: broadcast::Sender<EngineEvent>,
    state_dir: Option<PathBuf>,
    dirty: AtomicBool,
    config_dirty: AtomicBool,
    shutting_down: AtomicBool,
    tools: Tools,
    torrents: tokio::sync::Mutex<Option<Arc<Session>>>,
}

#[derive(Clone)]
pub struct Engine {
    inner: Arc<Inner>,
}

fn build_client(cfg: &Config) -> Result<Client> {
    let mut b = Client::builder()
        .http1_only()
        .user_agent(cfg.user_agent.clone())
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(64)
        .tcp_nodelay(true)
        .redirect(reqwest::redirect::Policy::limited(10))
        .danger_accept_invalid_certs(cfg.ignore_tls_errors);
    match cfg.proxy.mode {
        ProxyMode::System => {}
        ProxyMode::None => b = b.no_proxy(),
        ProxyMode::Manual => {
            let proxy = reqwest::Proxy::all(cfg.proxy.url.trim())
                .map_err(|e| EngineError::InvalidUrl(format!("proxy: {e}")))?;
            b = b.proxy(proxy);
        }
    }
    Ok(b.build()?)
}

impl Engine {
    pub fn start(defaults: Config, state_dir: Option<PathBuf>) -> Result<Self> {
        let config = state_dir
            .as_deref()
            .and_then(|d| std::fs::read(d.join("settings.json")).ok())
            .and_then(|b| serde_json::from_slice::<Config>(&b).ok())
            .unwrap_or(defaults)
            .normalized();

        let entries = state_dir
            .as_deref()
            .map(|d| load_records(&d.join("downloads.json")))
            .unwrap_or_default()
            .into_iter()
            .map(|mut rec| {
                if matches!(rec.status, Status::Connecting | Status::Downloading | Status::Processing) {
                    rec.status = Status::Queued;
                }
                Entry::new(rec)
            })
            .collect();

        let client = build_client(&config)?;
        let tools_dir =
            state_dir.as_ref().map(|d| d.join("tools")).unwrap_or_else(|| std::env::temp_dir().join("pulse-dm-tools"));
        let inner = Arc::new(Inner {
            rt: Handle::current(),
            tools: Tools::new(tools_dir, client.clone()),
            client: RwLock::new(client),
            limiter: Arc::new(RateLimiter::new(config.speed_limit)),
            host_limiters: Mutex::new(HashMap::new()),
            config: RwLock::new(config),
            entries: Mutex::new(entries),
            events: broadcast::channel(256).0,
            state_dir,
            dirty: AtomicBool::new(false),
            config_dirty: AtomicBool::new(false),
            shutting_down: AtomicBool::new(false),
            torrents: tokio::sync::Mutex::new(None),
        });
        ensure_category_dirs(&inner.config.read().unwrap());
        inner.clone().spawn_ticker();
        inner.schedule();
        Ok(Self { inner })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<EngineEvent> {
        self.inner.events.subscribe()
    }

    pub fn list(&self) -> Vec<DownloadView> {
        self.inner.entries.lock().unwrap().iter().map(Entry::view).collect()
    }

    pub fn get(&self, id: Uuid) -> Option<DownloadView> {
        self.inner.entries.lock().unwrap().iter().find(|e| e.rec.id == id).map(Entry::view)
    }

    pub fn is_idle(&self) -> bool {
        self.inner.entries.lock().unwrap().iter().all(|e| e.run.is_none() && e.rec.status != Status::Queued)
    }

    pub fn config(&self) -> Config {
        self.inner.config.read().unwrap().clone()
    }

    pub fn set_config(&self, config: Config) -> Result<Config> {
        let config = config.normalized();
        let old = self.config();
        if config.proxy != old.proxy
            || config.user_agent != old.user_agent
            || config.ignore_tls_errors != old.ignore_tls_errors
        {
            *self.inner.client.write().unwrap() = build_client(&config)?;
        }
        self.inner.limiter.set_rate(config.speed_limit);
        for (host, limiter) in self.inner.host_limiters.lock().unwrap().iter() {
            let rate = config.host_rules.iter().find(|r| r.host.eq_ignore_ascii_case(host)).map(|r| r.speed_limit);
            limiter.set_rate(rate.unwrap_or(0));
        }
        ensure_category_dirs(&config);
        *self.inner.config.write().unwrap() = config.clone();
        for e in self.inner.entries.lock().unwrap().iter() {
            if let (Some(run), None) = (&e.run, e.rec.connections) {
                let n = config.rule_for(&e.rec.url).and_then(|r| r.connections).unwrap_or(config.connections);
                run.control.set_connections(n);
            }
        }
        self.inner.config_dirty.store(true, Ordering::Relaxed);
        self.inner.emit(EngineEvent::Config(config.clone()));
        self.inner.schedule();
        Ok(config)
    }

    pub fn add(&self, req: AddRequest) -> Result<DownloadView> {
        if torrent::is_source(&req.url) {
            return self.add_torrent(req);
        }
        let url = Url::parse(req.url.trim()).map_err(|e| EngineError::InvalidUrl(e.to_string()))?;
        if !matches!(url.scheme(), "http" | "https") {
            {
                let scheme = url.scheme();
                return Err(EngineError::InvalidUrl(text!(
                    "unsupported protocol: {scheme}",
                    "desteklenmeyen protokol: {scheme}"
                )));
            }
        }
        let cfg = self.config();
        let custom = req.file_name.as_deref().map(str::trim).filter(|n| !n.is_empty());
        let guess = match &req.media_option {
            Some(_) => Some(format!("{}.mp4", req.title.as_deref().unwrap_or("video"))),
            None => name_from_url(&url),
        };
        let file_name = sanitize_file_name(&custom.map(str::to_owned).or(guess).unwrap_or_default());
        let mut headers = req.headers;
        if let Some(user) = req.username.filter(|u| !u.is_empty()) {
            let token = base64::engine::general_purpose::STANDARD
                .encode(format!("{user}:{}", req.password.unwrap_or_default()));
            headers.retain(|(k, _)| !k.eq_ignore_ascii_case("authorization"));
            headers.push(("Authorization".into(), format!("Basic {token}")));
        }
        let queue = req.queue.filter(|q| cfg.queue_exists(q)).unwrap_or_else(main_queue);
        let dir = req.dir.filter(|d| !d.as_os_str().is_empty()).unwrap_or_else(|| {
            let folder = if cfg.category_folders { category_folder(category_of(&file_name)) } else { None };
            folder.map(|f| cfg.download_dir.join(f)).unwrap_or_else(|| cfg.download_dir.clone())
        });
        let rec = DownloadRecord {
            id: Uuid::new_v4(),
            url: url.to_string(),
            final_url: None,
            url_changed: false,
            file_name,
            custom_name: custom.is_some(),
            dir,
            size: None,
            downloaded: 0,
            status: if req.paused { Status::Paused } else { Status::Queued },
            error: None,
            resumable: false,
            validator: None,
            last_modified: None,
            segments: Vec::new(),
            headers,
            queue,
            connections: req.connections.map(|c| c.clamp(1, 32)),
            speed_limit: req.speed_limit.unwrap_or(0),
            hls: None,
            media: req.media_option.map(|option| MediaState { page_url: url.to_string(), option, streams: Vec::new() }),
            torrent: None,
            added_at: now(),
            completed_at: None,
        };
        Ok(self.insert(rec))
    }

    fn insert(&self, rec: DownloadRecord) -> DownloadView {
        let entry = Entry::new(rec);
        let view = entry.view();
        self.inner.entries.lock().unwrap().push(entry);
        self.inner.emit(EngineEvent::Changed(view.clone()));
        self.inner.mark_dirty();
        self.inner.schedule();
        view
    }

    fn add_torrent(&self, req: AddRequest) -> Result<DownloadView> {
        let source = req.url.trim().to_string();
        let id = Uuid::new_v4();
        if let Some(path) = torrent::local_path(&source) {
            let bytes = std::fs::read(&path)?;
            torrent::metadata(&bytes)?;
            let cache = self.inner.torrent_cache(id);
            if let Some(dir) = cache.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&cache, &bytes)?;
        }
        let cfg = self.config();
        let custom = req.file_name.as_deref().map(str::trim).filter(|n| !n.is_empty());
        let file_name =
            sanitize_file_name(&custom.map(str::to_owned).unwrap_or_else(|| torrent::display_name(&source)));
        let queue = req.queue.filter(|q| cfg.queue_exists(q)).unwrap_or_else(main_queue);
        let rec = DownloadRecord {
            id,
            url: source.clone(),
            final_url: None,
            url_changed: false,
            file_name,
            custom_name: custom.is_some(),
            dir: req.dir.filter(|d| !d.as_os_str().is_empty()).unwrap_or_else(|| cfg.download_dir.clone()),
            size: None,
            downloaded: 0,
            status: if req.paused { Status::Paused } else { Status::Queued },
            error: None,
            resumable: true,
            validator: None,
            last_modified: None,
            segments: Vec::new(),
            headers: req.headers,
            queue,
            connections: None,
            speed_limit: req.speed_limit.unwrap_or(0),
            hls: None,
            media: None,
            torrent: Some(TorrentState { source, ..TorrentState::default() }),
            added_at: now(),
            completed_at: None,
        };
        Ok(self.insert(rec))
    }

    pub fn pause(&self, id: Uuid) -> Result<()> {
        self.inner.update(id, |e| {
            if e.rec.status.is_live() {
                e.rec.status = Status::Paused;
                if let Some(run) = &mut e.run {
                    run.requeue = false;
                    run.cancel.cancel();
                }
            }
        })
    }

    pub fn resume(&self, id: Uuid) -> Result<()> {
        self.inner.update(id, |e| {
            if matches!(e.rec.status, Status::Paused | Status::Failed) {
                e.rec.status = Status::Queued;
                e.rec.error = None;
            }
        })?;
        self.inner.schedule();
        Ok(())
    }

    pub fn pause_all(&self) {
        for id in self.ids() {
            let _ = self.pause(id);
        }
    }

    pub fn resume_all(&self) {
        for id in self.ids() {
            let _ = self.resume(id);
        }
    }

    pub fn set_url(&self, id: Uuid, url: &str) -> Result<()> {
        let parsed = Url::parse(url.trim()).map_err(|e| EngineError::InvalidUrl(e.to_string()))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            {
                let scheme = parsed.scheme();
                return Err(EngineError::InvalidUrl(text!(
                    "unsupported protocol: {scheme}",
                    "desteklenmeyen protokol: {scheme}"
                )));
            }
        }
        let mut result = Ok(());
        self.inner.update(id, |e| {
            if e.run.is_some() {
                result = Err(EngineError::Busy);
                return;
            }
            e.rec.url = parsed.to_string();
            e.rec.final_url = None;
            e.rec.url_changed = true;
            e.rec.error = None;
            if e.rec.status == Status::Failed {
                e.rec.status = Status::Paused;
            }
        })?;
        result
    }

    pub async fn probe_url(&self, url: &str, headers: Vec<(String, String)>) -> Result<UrlInfo> {
        let info = probe(&self.inner.client(), url, &header_map(&headers)).await?;
        Ok(UrlInfo { file_name: info.file_name, size: info.size, resumable: info.resumable, html: info.html })
    }

    pub async fn probe_media(&self, url: &str, headers: Vec<(String, String)>) -> Result<MediaInfo> {
        let inner = &self.inner;
        let report = |p: ToolProgress| inner.emit(EngineEvent::Tool(p));
        let ytdlp = inner.tools.ytdlp(&report).await?;
        let js = inner.tools.js_runtime(&report).await?;
        let v = media::extract(&ytdlp, &js, url, &headers).await?;
        Ok(media::info(url, &v))
    }

    pub fn set_options(&self, id: Uuid, opts: DownloadOptions) -> Result<()> {
        let cfg = self.config();
        self.inner.update(id, |e| {
            e.rec.connections = opts.connections.map(|c| c.clamp(1, 32));
            e.rec.speed_limit = opts.speed_limit;
            if let Some(run) = &e.run {
                let n = e.rec.connections.or_else(|| cfg.rule_for(&e.rec.url).and_then(|r| r.connections));
                run.control.set_connections(n.unwrap_or(cfg.connections));
                run.control.limiter.set_rate(opts.speed_limit);
            }
        })
    }

    pub fn move_to_queue(&self, ids: &[Uuid], queue: &str) -> Result<()> {
        if !self.config().queue_exists(queue) {
            return Err(EngineError::NotFound);
        }
        for &id in ids {
            self.inner.update(id, |e| e.rec.queue = queue.to_string())?;
        }
        self.inner.schedule();
        Ok(())
    }

    pub fn start_queue(&self, queue: &str) -> Result<()> {
        self.inner.set_queue_running(queue, true)
    }

    pub fn stop_queue(&self, queue: &str) -> Result<()> {
        self.inner.set_queue_running(queue, false)
    }

    pub fn remove(&self, id: Uuid, delete_files: bool) -> Result<()> {
        let removed = {
            let mut entries = self.inner.entries.lock().unwrap();
            let idx = entries.iter().position(|e| e.rec.id == id).ok_or(EngineError::NotFound)?;
            if let Some(run) = &entries[idx].run {
                run.cancel.cancel();
                entries[idx].remove_after = Some(delete_files);
                None
            } else {
                Some(entries.remove(idx).rec)
            }
        };
        if let Some(rec) = removed {
            if delete_files {
                delete_download_files(&rec);
            }
            self.inner.forget(&rec);
            self.inner.emit(EngineEvent::Removed(id));
            self.inner.mark_dirty();
        }
        Ok(())
    }

    pub fn clear_completed(&self) {
        let removed: Vec<Uuid> = {
            let mut entries = self.inner.entries.lock().unwrap();
            let ids = entries.iter().filter(|e| e.rec.status == Status::Completed).map(|e| e.rec.id).collect();
            for e in entries.iter().filter(|e| e.rec.status == Status::Completed) {
                self.inner.forget(&e.rec);
            }
            entries.retain(|e| e.rec.status != Status::Completed);
            ids
        };
        for id in removed {
            self.inner.emit(EngineEvent::Removed(id));
        }
        self.inner.mark_dirty();
    }

    pub async fn shutdown(&self) {
        self.inner.shutting_down.store(true, Ordering::SeqCst);
        for e in self.inner.entries.lock().unwrap().iter() {
            if let Some(run) = &e.run {
                run.cancel.cancel();
            }
        }
        for _ in 0..40 {
            if self.inner.entries.lock().unwrap().iter().all(|e| e.run.is_none()) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        self.inner.save();
        self.inner.save_config();
        if let Some(session) = self.inner.torrents.lock().await.take() {
            session.stop().await;
        }
    }

    fn ids(&self) -> Vec<Uuid> {
        self.inner.entries.lock().unwrap().iter().map(|e| e.rec.id).collect()
    }
}

impl Inner {
    fn emit(&self, ev: EngineEvent) {
        let _ = self.events.send(ev);
    }

    fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::Relaxed);
    }

    fn client(&self) -> Client {
        self.client.read().unwrap().clone()
    }

    fn update(&self, id: Uuid, f: impl FnOnce(&mut Entry)) -> Result<()> {
        let view = {
            let mut entries = self.entries.lock().unwrap();
            let e = entries.iter_mut().find(|e| e.rec.id == id).ok_or(EngineError::NotFound)?;
            f(e);
            e.view()
        };
        self.emit(EngineEvent::Changed(view));
        self.mark_dirty();
        Ok(())
    }

    fn set_queue_running(self: &Arc<Self>, queue: &str, running: bool) -> Result<()> {
        let config = {
            let mut cfg = self.config.write().unwrap();
            let q = cfg.queues.iter_mut().find(|q| q.id == queue).ok_or(EngineError::NotFound)?;
            q.running = running;
            cfg.clone()
        };
        if !running {
            for e in self.entries.lock().unwrap().iter_mut() {
                if e.rec.queue == queue
                    && let Some(run) = &mut e.run
                {
                    run.requeue = true;
                    run.cancel.cancel();
                }
            }
        }
        self.config_dirty.store(true, Ordering::Relaxed);
        self.emit(EngineEvent::Config(config));
        self.schedule();
        Ok(())
    }

    fn host_limiter(&self, rule: Option<&HostRule>) -> Option<Arc<RateLimiter>> {
        let rule = rule.filter(|r| r.speed_limit > 0)?;
        let mut map = self.host_limiters.lock().unwrap();
        let limiter =
            map.entry(rule.host.to_ascii_lowercase()).or_insert_with(|| Arc::new(RateLimiter::new(rule.speed_limit)));
        limiter.set_rate(rule.speed_limit);
        Some(limiter.clone())
    }

    fn schedule(self: &Arc<Self>) {
        if self.shutting_down.load(Ordering::SeqCst) {
            return;
        }
        let cfg = self.config.read().unwrap().clone();
        let mut start = Vec::new();
        {
            let mut entries = self.entries.lock().unwrap();
            for q in cfg.queues.iter().filter(|q| q.running) {
                let in_queue =
                    |e: &Entry| e.rec.queue == q.id || (q.id == MAIN_QUEUE && !cfg.queue_exists(&e.rec.queue));
                let mut active = entries.iter().filter(|e| e.run.is_some() && in_queue(e)).count();
                for e in entries.iter_mut() {
                    if active >= q.max_active {
                        break;
                    }
                    if e.rec.status == Status::Queued && e.run.is_none() && in_queue(e) {
                        let fixed = e.rec.connections.or_else(|| cfg.rule_for(&e.rec.url).and_then(|r| r.connections));
                        let control = Arc::new(match fixed {
                            Some(n) => JobControl::new(n, e.rec.speed_limit),
                            None => JobControl::automatic(cfg.connections, cfg.max_auto_connections, e.rec.speed_limit),
                        });
                        let cancel = CancellationToken::new();
                        e.run = Some(Running {
                            cancel: cancel.clone(),
                            control: control.clone(),
                            table: None,
                            hls: None,
                            media_base: Arc::new(AtomicU64::new(0)),
                            media_stream: None,
                            peers: None,
                            requeue: false,
                        });
                        e.last_bytes = e.rec.downloaded;
                        e.speed = 0.0;
                        start.push((e.rec.id, cancel, control));
                        active += 1;
                    }
                }
            }
        }
        for (id, cancel, control) in start {
            let inner = self.clone();
            self.rt.spawn(async move { inner.run_entry(id, cancel, control).await });
        }
    }

    async fn run_entry(self: Arc<Self>, id: Uuid, cancel: CancellationToken, control: Arc<JobControl>) {
        let result = self.drive(id, &cancel, &control).await;

        let mut entries = self.entries.lock().unwrap();
        let Some(idx) = entries.iter().position(|e| e.rec.id == id) else {
            return;
        };
        let e = &mut entries[idx];
        let requeue = e.run.as_ref().is_some_and(|r| r.requeue);
        if let Some(run) = e.run.take() {
            if let Some(m) = &mut e.rec.media {
                if let Some(st) = run.media_stream.and_then(|i| m.streams.get_mut(i)) {
                    if let Some(h) = &run.hls {
                        st.hls = Some(h.lock().unwrap().clone());
                    } else if let Some(t) = &run.table {
                        st.segments = t.snapshot();
                    }
                }
                if let Some(t) = &run.table {
                    e.rec.downloaded = run.media_base.load(Ordering::Relaxed) + t.downloaded();
                }
            } else if e.rec.torrent.is_some() {
                if let Some(t) = &run.table {
                    e.rec.downloaded = t.downloaded();
                }
            } else if let Some(h) = run.hls {
                let h = h.lock().unwrap().clone();
                e.rec.downloaded = h.out_size;
                e.rec.hls = Some(h);
            } else if let Some(t) = run.table {
                e.rec.segments = t.snapshot();
                e.rec.downloaded = t.downloaded();
            }
        }
        e.speed = 0.0;

        if let Some(delete_files) = e.remove_after {
            let rec = entries.remove(idx).rec;
            drop(entries);
            if delete_files {
                delete_download_files(&rec);
            }
            self.forget(&rec);
            self.emit(EngineEvent::Removed(id));
        } else {
            match result {
                Ok(()) => {
                    e.rec.status = Status::Completed;
                    e.rec.error = None;
                    e.rec.completed_at = Some(now());
                    if e.rec.media.is_some() {
                        e.rec.downloaded = e.rec.size.unwrap_or(e.rec.downloaded);
                    } else {
                        e.rec.size = Some(e.rec.downloaded);
                    }
                    e.rec.segments.clear();
                }
                Err(EngineError::Cancelled) => {
                    if self.shutting_down.load(Ordering::SeqCst) || requeue {
                        if e.rec.status != Status::Paused {
                            e.rec.status = Status::Queued;
                        }
                    } else if matches!(e.rec.status, Status::Connecting | Status::Downloading | Status::Processing) {
                        e.rec.status = Status::Paused;
                    }
                }
                Err(err) => {
                    e.rec.status = Status::Failed;
                    e.rec.error = Some(err.to_string());
                }
            }
            let view = e.view();
            drop(entries);
            self.emit(EngineEvent::Changed(view));
        }
        self.mark_dirty();
        self.schedule();
    }

    async fn drive(&self, id: Uuid, cancel: &CancellationToken, control: &Arc<JobControl>) -> Result<()> {
        match self.drive_once(id, cancel, control).await {
            Err(EngineError::Changed) => {
                let _ = self.with_entry(id, |e| {
                    let _ = std::fs::remove_file(part_path(&e.rec.dir, &e.rec.file_name, PART_EXT));
                    e.rec.segments.clear();
                    e.rec.downloaded = 0;
                    e.rec.size = None;
                });
                self.drive_once(id, cancel, control).await
            }
            other => other,
        }
    }

    async fn drive_once(&self, id: Uuid, cancel: &CancellationToken, control: &Arc<JobControl>) -> Result<()> {
        let rec = self.with_entry(id, |e| {
            e.rec.status = Status::Connecting;
            e.rec.error = None;
            e.rec.clone()
        })?;
        self.emit_view(id);
        let cfg = self.config.read().unwrap().clone();
        let rule = cfg.rule_for(&rec.url).cloned();
        let mut headers = header_map(&rec.headers);
        if let Some(ua) =
            rule.as_ref().and_then(|r| r.user_agent.as_deref()).and_then(|ua| HeaderValue::try_from(ua).ok())
        {
            headers.insert(USER_AGENT, ua);
        }
        let client = self.client();
        let mut limiters = vec![self.limiter.clone()];
        limiters.extend(self.host_limiter(rule.as_ref()));

        if rec.torrent.is_some() {
            return self.drive_torrent(id, cancel, rec, headers, client).await;
        }

        if let Some(h) = &rec.hls {
            let playlist_url = if rec.url_changed { rec.url.clone() } else { h.playlist_url.clone() };
            return self.drive_hls(id, cancel, control, rec, playlist_url, headers, &cfg, client, limiters).await;
        }

        if rec.media.is_some() {
            return self.drive_media(id, cancel, control, rec, headers, &cfg, client, limiters).await;
        }

        let existing_part = part_path(&rec.dir, &rec.file_name, PART_EXT);
        let resumable_state = rec.resumable && !rec.segments.is_empty() && existing_part.exists();

        let (file, table, rec) = if resumable_state {
            let rec = if rec.url_changed {
                let info = tokio::select! {
                    r = probe(&client, &rec.url, &headers) => r?,
                    _ = cancel.cancelled() => return Err(EngineError::Cancelled),
                };
                if info.html && !expects_html(&rec.file_name) {
                    return Err(EngineError::WebPage);
                }
                if !info.resumable || info.size != rec.size {
                    let want = rec.size.map(human_bytes).unwrap_or_else(|| "?".into());
                    let got = info.size.map(human_bytes).unwrap_or_else(|| "?".into());
                    return Err(EngineError::UrlMismatch(text!(
                        "expected size {want}, the new link has {got}",
                        "beklenen boyut {want}, yeni bağlantıdaki {got}"
                    )));
                }
                self.with_entry(id, |e| {
                    e.rec.final_url = Some(info.final_url.clone());
                    e.rec.validator = info.validator.clone();
                    e.rec.url_changed = false;
                    e.rec.clone()
                })?
            } else {
                rec
            };
            (open_part_file(&existing_part)?, SegmentTable::from_states(&rec.segments, cfg.min_split), rec)
        } else {
            let info = tokio::select! {
                r = probe(&client, &rec.url, &headers) => r?,
                _ = cancel.cancelled() => return Err(EngineError::Cancelled),
            };
            let wanted = if rec.custom_name { rec.file_name.clone() } else { info.file_name.clone() };
            if hls::looks_like_playlist(&info.final_url, info.content_type.as_deref()) {
                let rec = DownloadRecord { file_name: wanted, ..rec };
                return self.drive_hls(id, cancel, control, rec, info.final_url, headers, &cfg, client, limiters).await;
            }
            if info.html && !expects_html(&wanted) {
                return Err(EngineError::WebPage);
            }
            if rec.owns_part() {
                let _ = std::fs::remove_file(&existing_part);
            }
            std::fs::create_dir_all(&rec.dir)?;
            if let (Some(size), Some(free)) = (info.size, free_space(&rec.dir))
                && free < size.saturating_add(DISK_MARGIN)
            {
                return Err(EngineError::NoSpace { need: human_bytes(size), free: human_bytes(free) });
            }
            let (name, file) = loop {
                let name = unique_name(&rec.dir, &wanted, PART_EXT);
                match create_part_file(&part_path(&rec.dir, &name, PART_EXT), info.size) {
                    Ok(file) => break (name, file),
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(e) => return Err(e.into()),
                }
            };
            let table = match (info.resumable, info.size) {
                (true, Some(size)) => SegmentTable::new(size, control.connections(), cfg.min_split),
                (false, Some(size)) => {
                    SegmentTable::from_states(&[SegmentState { start: 0, pos: 0, end: size }], u64::MAX)
                }
                (_, None) => SegmentTable::unbounded(),
            };
            let segments = table.snapshot();
            let rec = self.with_entry(id, |e| {
                e.rec.file_name = name;
                e.rec.final_url = Some(info.final_url.clone());
                e.rec.url_changed = false;
                e.rec.size = info.size;
                e.rec.resumable = info.resumable;
                e.rec.validator = info.validator.clone();
                e.rec.last_modified = info.last_modified.clone();
                e.rec.segments = segments;
                e.rec.downloaded = 0;
                e.rec.clone()
            })?;
            (file, table, rec)
        };

        let table = Arc::new(table);
        self.with_entry(id, |e| {
            if let Some(run) = &mut e.run {
                run.table = Some(table.clone());
            }
            e.last_bytes = table.downloaded();
            if e.rec.status == Status::Connecting {
                e.rec.status = Status::Downloading;
            }
        })?;
        self.emit_view(id);
        self.mark_dirty();

        let job = Arc::new(Job {
            client,
            limiters,
            control: control.clone(),
            cancel: cancel.clone(),
            table: table.clone(),
            file: Arc::new(file),
            url: rec.final_url.clone().unwrap_or_else(|| rec.url.clone()),
            headers,
            ranged: rec.resumable,
            bounded: rec.size.is_some(),
            validator: rec.validator.clone(),
            max_retries: cfg.max_retries,
            accept_html: expects_html(&rec.file_name),
            max_request: None,
            live: AtomicUsize::new(0),
            refusals: AtomicUsize::new(0),
        });
        let result = download::run(job.clone()).await;
        drop(job);
        result?;

        let final_name = if rec.dir.join(&rec.file_name).exists() {
            unique_name(&rec.dir, &rec.file_name, "")
        } else {
            rec.file_name.clone()
        };
        let final_path = rec.dir.join(&final_name);
        std::fs::rename(part_path(&rec.dir, &rec.file_name, PART_EXT), &final_path)?;
        if cfg.keep_server_time
            && let Some(time) = rec.last_modified.as_deref().and_then(|t| httpdate::parse_http_date(t).ok())
        {
            let _ = std::fs::File::options().write(true).open(&final_path).and_then(|f| f.set_modified(time));
        }
        self.with_entry(id, |e| e.rec.file_name = final_name)?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn drive_hls(
        &self,
        id: Uuid,
        cancel: &CancellationToken,
        control: &Arc<JobControl>,
        rec: DownloadRecord,
        playlist_url: String,
        headers: HeaderMap,
        cfg: &Config,
        client: Client,
        limiters: Vec<Arc<RateLimiter>>,
    ) -> Result<()> {
        let playlist = tokio::select! {
            r = hls::load(&client, &playlist_url, &headers) => r?,
            _ = cancel.cancelled() => return Err(EngineError::Cancelled),
        };
        let total = playlist.segments.len();
        let part = part_path(&rec.dir, &rec.file_name, PART_EXT);
        let resume = rec.hls.clone().filter(|h| h.total == total && h.written > 0 && part.exists());

        let (name, state) = match resume {
            Some(mut st) => {
                open_part_file(&part)?.set_len(st.out_size)?;
                st.playlist_url = playlist_url.clone();
                (rec.file_name.clone(), st)
            }
            None => {
                if rec.hls.is_some() {
                    let _ = std::fs::remove_file(&part);
                }
                std::fs::create_dir_all(&rec.dir)?;
                let wanted = video_name(&rec.file_name, if playlist.init.is_some() { "mp4" } else { "ts" });
                let name = loop {
                    let name = unique_name(&rec.dir, &wanted, PART_EXT);
                    match create_part_file(&part_path(&rec.dir, &name, PART_EXT), None) {
                        Ok(_) => break name,
                        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                        Err(e) => return Err(e.into()),
                    }
                };
                (name, HlsState { playlist_url: playlist_url.clone(), total, written: 0, out_size: 0 })
            }
        };

        let counter = Arc::new(SegmentTable::counter());
        counter.set_counter(state.out_size);
        let state = Arc::new(Mutex::new(state));
        self.with_entry(id, |e| {
            e.rec.file_name = name.clone();
            e.rec.size = None;
            e.rec.resumable = true;
            e.rec.url_changed = false;
            e.rec.segments.clear();
            e.rec.hls = Some(state.lock().unwrap().clone());
            if let Some(run) = &mut e.run {
                run.table = Some(counter.clone());
                run.hls = Some(state.clone());
            }
            e.last_bytes = counter.downloaded();
            if e.rec.status == Status::Connecting {
                e.rec.status = Status::Downloading;
            }
        })?;
        self.emit_view(id);
        self.mark_dirty();

        let output = part_path(&rec.dir, &name, PART_EXT);
        let job = Arc::new(hls::HlsJob {
            client,
            headers,
            limiters,
            control: control.clone(),
            cancel: cancel.clone(),
            counter: counter.clone(),
            max_retries: cfg.max_retries,
            output: output.clone(),
            state: state.clone(),
        });
        hls::run(job, playlist).await?;

        let out_size = state.lock().unwrap().out_size;
        counter.set_counter(out_size);
        let final_name = if rec.dir.join(&name).exists() { unique_name(&rec.dir, &name, "") } else { name };
        std::fs::rename(&output, rec.dir.join(&final_name))?;
        self.with_entry(id, |e| e.rec.file_name = final_name)?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn drive_media(
        &self,
        id: Uuid,
        cancel: &CancellationToken,
        control: &Arc<JobControl>,
        rec: DownloadRecord,
        headers: HeaderMap,
        cfg: &Config,
        client: Client,
        limiters: Vec<Arc<RateLimiter>>,
    ) -> Result<()> {
        let mut media = rec.media.clone().expect("media download");
        let report = |p: ToolProgress| self.emit(EngineEvent::Tool(p));
        macro_rules! cancellable {
            ($fut:expr) => {
                tokio::select! {
                    r = $fut => r?,
                    _ = cancel.cancelled() => return Err(EngineError::Cancelled),
                }
            };
        }

        let ytdlp = cancellable!(self.tools.ytdlp(&report));
        let js = cancellable!(self.tools.js_runtime(&report));
        let info = cancellable!(media::extract(&ytdlp, &js, &media.page_url, &rec.headers));
        let picks = media::select(&info, &media.option)?;
        let audio_only = media.option == media::AUDIO_OPTION;
        let ffmpeg = if media::needs_ffmpeg(&picks, audio_only) {
            Some(cancellable!(self.tools.ffmpeg_dir(&report)))
        } else {
            None
        };
        let ext = media::output_ext(&picks, audio_only);

        let same = !media.streams.is_empty()
            && media.streams.len() == picks.len()
            && media.streams.iter().zip(&picks).all(|(s, p)| s.format_id == p.format_id);
        let name = if same {
            rec.file_name.clone()
        } else {
            for st in &media.streams {
                let _ = std::fs::remove_file(rec.dir.join(&st.part));
            }
            if rec.owns_part() {
                let _ = std::fs::remove_file(part_path(&rec.dir, &rec.file_name, PART_EXT));
            }
            let title = info.get("title").and_then(serde_json::Value::as_str).unwrap_or("video");
            let stem = match (rec.custom_name, rec.file_name.rsplit_once('.')) {
                (true, Some((stem, _))) => stem.to_string(),
                (true, None) => rec.file_name.clone(),
                (false, _) => title.to_string(),
            };
            let wanted = sanitize_file_name(&format!("{stem}.{ext}"));
            std::fs::create_dir_all(&rec.dir)?;
            let name = loop {
                let name = unique_name(&rec.dir, &wanted, PART_EXT);
                match create_part_file(&part_path(&rec.dir, &name, PART_EXT), None) {
                    Ok(_) => break name,
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(e) => return Err(e.into()),
                }
            };
            media.streams = picks
                .iter()
                .enumerate()
                .map(|(i, p)| StreamState {
                    format_id: p.format_id.clone(),
                    part: format!("{name}.{i}.{PART_EXT}"),
                    size: p.size,
                    ..Default::default()
                })
                .collect();
            name
        };

        let mut stream_headers = Vec::new();
        let mut probes = Vec::new();
        for p in &picks {
            let mut h = header_map(&p.headers);
            h.extend(headers.clone());
            probes.push(match p.protocol {
                Protocol::Http => Some(cancellable!(probe(&client, &p.url, &h))),
                Protocol::Hls => None,
            });
            stream_headers.push(h);
        }
        for (st, pr) in media.streams.iter_mut().zip(&probes) {
            if let Some(pr) = pr {
                st.size = pr.size.or(st.size);
            }
        }
        let total: Option<u64> = media.streams.iter().map(|s| s.size).sum();
        if let (Some(size), Some(free)) = (total, free_space(&rec.dir)) {
            let have: u64 = media.streams.iter().filter(|s| s.done).filter_map(|s| s.size).sum();
            if free < (size - have.min(size)).saturating_add(DISK_MARGIN) {
                return Err(EngineError::NoSpace { need: human_bytes(size), free: human_bytes(free) });
            }
        }
        let base = self
            .with_entry(id, |e| {
                e.rec.file_name = name.clone();
                e.rec.size = total;
                e.rec.resumable = true;
                e.rec.media = Some(media.clone());
                if e.rec.status == Status::Connecting {
                    e.rec.status = Status::Downloading;
                }
                e.run.as_ref().map(|r| r.media_base.clone())
            })?
            .unwrap_or_default();
        self.emit_view(id);
        self.mark_dirty();

        let mut done_bytes = 0u64;
        for (i, p) in picks.iter().enumerate() {
            let st = media.streams[i].clone();
            if st.done {
                done_bytes += st.size.or(st.hls.as_ref().map(|h| h.out_size)).unwrap_or(0);
                continue;
            }
            base.store(done_bytes, Ordering::Relaxed);
            let part = rec.dir.join(&st.part);
            let finished_bytes = match (p.protocol, &probes[i]) {
                (Protocol::Http, Some(pr)) => {
                    let resume = pr.resumable && !st.segments.is_empty() && part.exists() && st.size == pr.size;
                    let (file, table) = if resume {
                        (open_part_file(&part)?, SegmentTable::from_states(&st.segments, cfg.min_split))
                    } else {
                        let _ = std::fs::remove_file(&part);
                        let table = match (pr.resumable, pr.size) {
                            (true, Some(size)) => SegmentTable::new(size, control.connections(), cfg.min_split),
                            (false, Some(size)) => {
                                SegmentTable::from_states(&[SegmentState { start: 0, pos: 0, end: size }], u64::MAX)
                            }
                            (_, None) => SegmentTable::unbounded(),
                        };
                        (create_part_file(&part, pr.size)?, table)
                    };
                    let table = Arc::new(table);
                    self.with_entry(id, |e| {
                        if let Some(run) = &mut e.run {
                            run.table = Some(table.clone());
                            run.hls = None;
                            run.media_stream = Some(i);
                        }
                        e.last_bytes = done_bytes + table.downloaded();
                    })?;
                    let job = Arc::new(Job {
                        client: client.clone(),
                        limiters: limiters.clone(),
                        control: control.clone(),
                        cancel: cancel.clone(),
                        table: table.clone(),
                        file: Arc::new(file),
                        url: pr.final_url.clone(),
                        headers: stream_headers[i].clone(),
                        ranged: pr.resumable,
                        bounded: pr.size.is_some(),
                        validator: None,
                        max_retries: cfg.max_retries,
                        accept_html: false,
                        max_request: p.chunk_size,
                        live: AtomicUsize::new(0),
                        refusals: AtomicUsize::new(0),
                    });
                    let res = download::run(job.clone()).await;
                    drop(job);
                    let snap = table.snapshot();
                    self.with_entry(id, |e| {
                        if let Some(m) = &mut e.rec.media {
                            m.streams[i].segments = snap;
                            m.streams[i].done = res.is_ok();
                        }
                    })?;
                    res?;
                    table.downloaded()
                }
                _ => {
                    let playlist = cancellable!(hls::load(&client, &p.url, &stream_headers[i]));
                    let segs = playlist.segments.len();
                    let state = match st.hls.clone().filter(|h| h.total == segs && h.written > 0 && part.exists()) {
                        Some(h) => {
                            open_part_file(&part)?.set_len(h.out_size)?;
                            h
                        }
                        None => {
                            let _ = std::fs::remove_file(&part);
                            create_part_file(&part, None)?;
                            HlsState { playlist_url: p.url.clone(), total: segs, written: 0, out_size: 0 }
                        }
                    };
                    let counter = Arc::new(SegmentTable::counter());
                    counter.set_counter(state.out_size);
                    let state = Arc::new(Mutex::new(state));
                    self.with_entry(id, |e| {
                        if let Some(run) = &mut e.run {
                            run.table = Some(counter.clone());
                            run.hls = Some(state.clone());
                            run.media_stream = Some(i);
                        }
                        e.last_bytes = done_bytes + counter.downloaded();
                    })?;
                    let job = Arc::new(hls::HlsJob {
                        client: client.clone(),
                        headers: stream_headers[i].clone(),
                        limiters: limiters.clone(),
                        control: control.clone(),
                        cancel: cancel.clone(),
                        counter,
                        max_retries: cfg.max_retries,
                        output: part.clone(),
                        state: state.clone(),
                    });
                    let res = hls::run(job, playlist).await;
                    let now_state = state.lock().unwrap().clone();
                    self.with_entry(id, |e| {
                        if let Some(m) = &mut e.rec.media {
                            m.streams[i].hls = Some(now_state.clone());
                            m.streams[i].done = res.is_ok();
                        }
                        if let Some(run) = &mut e.run {
                            run.hls = None;
                        }
                    })?;
                    res?;
                    now_state.out_size
                }
            };
            done_bytes += finished_bytes;
        }
        base.store(done_bytes, Ordering::Relaxed);

        self.with_entry(id, |e| {
            e.rec.status = Status::Processing;
            if let Some(run) = &mut e.run {
                run.media_stream = None;
            }
        })?;
        self.emit_view(id);
        let target = part_path(&rec.dir, &name, PART_EXT);
        let inputs: Vec<PathBuf> = media.streams.iter().map(|s| rec.dir.join(&s.part)).collect();
        match &ffmpeg {
            Some(dir) => cancellable!(media::mux(dir, &inputs, &target, &ext)),
            None => std::fs::rename(&inputs[0], &target)?,
        }
        for input in &inputs {
            let _ = std::fs::remove_file(input);
        }
        let final_name = if rec.dir.join(&name).exists() { unique_name(&rec.dir, &name, "") } else { name.clone() };
        std::fs::rename(&target, rec.dir.join(&final_name))?;
        let size = std::fs::metadata(rec.dir.join(&final_name)).map(|m| m.len()).ok();
        self.with_entry(id, |e| {
            e.rec.file_name = final_name;
            e.rec.size = size.or(e.rec.size);
            if let Some(m) = &mut e.rec.media {
                m.streams.clear();
            }
        })?;
        Ok(())
    }

    fn torrent_dir(&self) -> PathBuf {
        self.state_dir.clone().unwrap_or_else(|| std::env::temp_dir().join("pulse-dm")).join("torrents")
    }

    fn torrent_cache(&self, id: Uuid) -> PathBuf {
        self.torrent_dir().join(format!("{id}.torrent"))
    }

    fn forget(&self, rec: &DownloadRecord) {
        if rec.torrent.is_some() {
            let _ = std::fs::remove_file(self.torrent_cache(rec.id));
        }
    }

    async fn torrent_session(&self) -> Result<Arc<Session>> {
        let mut slot = self.torrents.lock().await;
        if let Some(session) = slot.as_ref() {
            return Ok(session.clone());
        }
        let (dir, limit) = {
            let cfg = self.config.read().unwrap();
            (cfg.download_dir.clone(), cfg.speed_limit)
        };
        let session = torrent::session(&dir, &self.torrent_dir(), limit).await?;
        *slot = Some(session.clone());
        Ok(session)
    }

    async fn drive_torrent(
        &self,
        id: Uuid,
        cancel: &CancellationToken,
        rec: DownloadRecord,
        headers: HeaderMap,
        client: Client,
    ) -> Result<()> {
        let state = rec.torrent.clone().unwrap_or_default();
        let session = tokio::select! {
            s = self.torrent_session() => s?,
            _ = cancel.cancelled() => return Err(EngineError::Cancelled),
        };
        let cache = self.torrent_cache(id);
        let bytes = match std::fs::read(&cache) {
            Ok(bytes) => bytes,
            Err(_) => {
                let bytes = tokio::select! {
                    r = torrent::fetch(&session, &state.source, &client, &headers) => r?,
                    _ = cancel.cancelled() => return Err(EngineError::Cancelled),
                };
                torrent::metadata(&bytes)?;
                if let Some(dir) = cache.parent() {
                    std::fs::create_dir_all(dir)?;
                }
                std::fs::write(&cache, &bytes)?;
                bytes
            }
        };
        let meta = torrent::metadata(&bytes)?;

        let (file_name, in_folder) = if state.info_hash.is_some() {
            (rec.file_name.clone(), state.in_folder)
        } else {
            let name = sanitize_file_name(meta.name.as_deref().unwrap_or(&rec.file_name));
            let taken = rec.dir.join(&name).exists();
            if meta.multi_file || taken {
                let folder = if rec.custom_name { rec.file_name.clone() } else { name };
                (unique_name(&rec.dir, &folder, ""), true)
            } else {
                (name, false)
            }
        };
        let output = if in_folder { rec.dir.join(&file_name) } else { rec.dir.clone() };
        std::fs::create_dir_all(&output)?;

        let opts = AddTorrentOptions {
            output_folder: Some(output.to_string_lossy().into_owned()),
            overwrite: true,
            ratelimits: LimitsConfig { download_bps: torrent::bps(rec.speed_limit), upload_bps: None },
            ..Default::default()
        };
        let added = tokio::select! {
            r = session.add_torrent(AddTorrent::from_bytes(bytes), Some(opts)) => r.map_err(torrent::failed)?,
            _ = cancel.cancelled() => return Err(EngineError::Cancelled),
        };
        let handle = match added {
            AddTorrentResponse::Added(_, h) | AddTorrentResponse::AlreadyManaged(_, h) => h,
            AddTorrentResponse::ListOnly(_) => return Err(torrent::failed("unexpected list-only response")),
        };

        let counter = Arc::new(SegmentTable::counter());
        counter.set_counter(rec.downloaded);
        let peers = Arc::new(AtomicUsize::new(0));
        let info_hash = handle.info_hash().as_string();
        self.with_entry(id, |e| {
            e.rec.file_name = file_name.clone();
            e.rec.url_changed = false;
            if let Some(t) = &mut e.rec.torrent {
                t.info_hash = Some(info_hash);
                t.in_folder = in_folder;
            }
            if let Some(run) = &mut e.run {
                run.table = Some(counter.clone());
                run.peers = Some(peers.clone());
            }
            e.last_bytes = counter.downloaded();
        })?;
        self.emit_view(id);
        self.mark_dirty();

        let result = loop {
            let stats = handle.stats();
            if !matches!(stats.state, TorrentStatsState::Initializing { .. }) {
                counter.set_counter(stats.progress_bytes);
            }
            peers.store(stats.live.as_ref().map_or(0, |l| l.snapshot.peer_stats.live as usize), Ordering::Relaxed);
            let live = matches!(stats.state, TorrentStatsState::Live);
            let started = self
                .with_entry(id, |e| {
                    e.rec.size = Some(stats.total_bytes);
                    if let Some(t) = &mut e.rec.torrent {
                        t.uploaded = stats.uploaded_bytes;
                    }
                    let started = live && e.rec.status == Status::Connecting;
                    if started {
                        e.rec.status = Status::Downloading;
                    }
                    started
                })
                .unwrap_or(false);
            if started {
                self.emit_view(id);
            }
            if let Some(err) = stats.error {
                break Err(torrent::failed(err));
            }
            if stats.finished {
                break Ok(());
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(500)) => {}
                _ = cancel.cancelled() => break Err(EngineError::Cancelled),
            }
        };
        let _ = session.delete(TorrentIdOrHash::Id(handle.id()), false).await;
        result
    }

    fn with_entry<T>(&self, id: Uuid, f: impl FnOnce(&mut Entry) -> T) -> Result<T> {
        let mut entries = self.entries.lock().unwrap();
        let e = entries.iter_mut().find(|e| e.rec.id == id).ok_or(EngineError::NotFound)?;
        Ok(f(e))
    }

    fn emit_view(&self, id: Uuid) {
        if let Ok(view) = self.with_entry(id, |e| e.view()) {
            self.emit(EngineEvent::Changed(view));
        }
    }

    fn spawn_ticker(self: Arc<Self>) {
        let weak = Arc::downgrade(&self);
        self.rt.spawn(async move {
            let mut interval = tokio::time::interval(TICK);
            let mut ticks = 0u64;
            let mut clock = ScheduleClock::default();
            loop {
                interval.tick().await;
                let Some(inner) = weak.upgrade() else { return };
                ticks += 1;
                let views: Vec<DownloadView> = {
                    let mut entries = inner.entries.lock().unwrap();
                    entries
                        .iter_mut()
                        .filter_map(|e| {
                            let done = e.live_table()?.downloaded();
                            let instant = done.saturating_sub(e.last_bytes) as f64 / TICK.as_secs_f64();
                            e.last_bytes = done;
                            e.speed = if e.speed == 0.0 { instant } else { 0.3 * instant + 0.7 * e.speed };
                            e.rec.downloaded = done;
                            Some(e.view())
                        })
                        .collect()
                };
                if !views.is_empty() {
                    inner.emit(EngineEvent::Progress(views));
                    if ticks.is_multiple_of(10) {
                        inner.mark_dirty();
                    }
                }

                let queues = inner.config.read().unwrap().queues.clone();
                for (queue, run) in clock.due(&queues) {
                    let _ = inner.set_queue_running(&queue, run);
                }

                if inner.dirty.swap(false, Ordering::Relaxed) {
                    inner.save();
                }
                if inner.config_dirty.swap(false, Ordering::Relaxed) {
                    inner.save_config();
                }
            }
        });
    }

    fn save(&self) {
        let Some(dir) = &self.state_dir else { return };
        let records: Vec<DownloadRecord> = self.entries.lock().unwrap().iter().map(Entry::persisted).collect();
        if let Ok(json) = serde_json::to_vec_pretty(&records) {
            write_atomic(&dir.join("downloads.json"), &json);
        }
    }

    fn save_config(&self) {
        let Some(dir) = &self.state_dir else { return };
        if let Ok(json) = serde_json::to_vec_pretty(&*self.config.read().unwrap()) {
            write_atomic(&dir.join("settings.json"), &json);
        }
    }
}

fn write_atomic(path: &Path, data: &[u8]) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, data).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

const FOLDER_CATEGORIES: [&str; 6] = ["archive", "document", "image", "audio", "program", "video"];

fn ensure_category_dirs(cfg: &Config) {
    if cfg.category_folders {
        for dir in FOLDER_CATEGORIES.iter().filter_map(|c| category_folder(c)) {
            let _ = std::fs::create_dir_all(cfg.download_dir.join(dir));
        }
    }
}

fn category_folder(category: &str) -> Option<&'static str> {
    let tr = crate::lang::system_turkish();
    Some(match category {
        "video" => {
            if tr {
                "Video"
            } else {
                "Videos"
            }
        }
        "audio" => {
            if tr {
                "Müzik"
            } else {
                "Music"
            }
        }
        "archive" => {
            if tr {
                "Arşiv"
            } else {
                "Compressed"
            }
        }
        "program" => {
            if tr {
                "Program"
            } else {
                "Programs"
            }
        }
        "document" => {
            if tr {
                "Belge"
            } else {
                "Documents"
            }
        }
        "image" => {
            if tr {
                "Görsel"
            } else {
                "Pictures"
            }
        }
        _ => return None,
    })
}

fn video_name(name: &str, ext: &str) -> String {
    let stem = match name.rsplit_once('.') {
        Some((stem, e)) if matches!(e.to_ascii_lowercase().as_str(), "m3u8" | "m3u" | "ts" | "mp4") => stem,
        _ => name,
    };
    let stem = if stem.is_empty() { "video" } else { stem };
    format!("{stem}.{ext}")
}

fn load_records(path: &Path) -> Vec<DownloadRecord> {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn delete_download_files(rec: &DownloadRecord) {
    if let Some(t) = &rec.torrent {
        let name = rec.file_name.trim();
        let path = rec.dir.join(name);
        if !name.is_empty() && name != "." && name != ".." && path.parent() == Some(rec.dir.as_path()) {
            if t.in_folder {
                if t.info_hash.is_some() {
                    let _ = std::fs::remove_dir_all(&path);
                }
            } else {
                let _ = std::fs::remove_file(&path);
            }
        }
        return;
    }
    if let Some(m) = &rec.media {
        for st in &m.streams {
            let _ = std::fs::remove_file(rec.dir.join(&st.part));
        }
    }
    if rec.owns_part() {
        let _ = std::fs::remove_file(part_path(&rec.dir, &rec.file_name, PART_EXT));
    }
    if rec.status == Status::Completed {
        let _ = std::fs::remove_file(rec.dir.join(&rec.file_name));
    }
}

fn header_map(pairs: &[(String, String)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (k, v) in pairs {
        let (Ok(name), Ok(value)) = (HeaderName::try_from(k.as_str()), HeaderValue::try_from(v.as_str())) else {
            continue;
        };
        if matches!(name.as_str(), "range" | "if-range" | "host" | "content-length" | "accept-encoding") {
            continue;
        }
        map.append(name, value);
    }
    map
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}
