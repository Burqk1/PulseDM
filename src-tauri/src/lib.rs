mod checksum;
mod native_host;

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use base64::Engine as _;
use pulse_engine::{
    AddRequest, Config, DownloadOptions, DownloadView, Engine, EngineEvent, MediaInfo, Status, UrlInfo,
};
use serde::{Deserialize, Serialize};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, RunEvent, State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_updater::UpdaterExt;
use tokio::sync::broadcast::error::RecvError;
use uuid::Uuid;

const MAIN_WINDOW: &str = "main";
const ADD_ARG: &str = "--add";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AfterAll {
    #[default]
    Nothing,
    Shutdown,
    Sleep,
}

fn migrate_download_folder(engine: &Engine, data_dir: &std::path::Path) {
    let marker = data_dir.join(".folders-v1");
    if marker.exists() {
        return;
    }
    let mut cfg = engine.config();
    let old_default = std::env::var_os("USERPROFILE").map(|h| std::path::PathBuf::from(h).join("Downloads"));
    if old_default.as_deref() == Some(cfg.download_dir.as_path()) {
        cfg.download_dir = cfg.download_dir.join("Pulse DM");
        cfg.category_folders = true;
        let _ = engine.set_config(cfg);
    }
    let _ = std::fs::write(marker, b"");
}

const BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --disable-gpu";

const HIDDEN_ARG: &str = "--hidden";

const MEDIA_ASK: &str = "ask";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct AddDraft {
    url: String,
    headers: Vec<(String, String)>,
    file_name: Option<String>,
    media: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct Prefs {
    theme: String,
    ui_scale: f64,
    relative_dates: bool,
    tool_labels: bool,
    progress_window: bool,
    completion_window: bool,
    notifications: bool,
    auto_update: bool,
    language: String,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            ui_scale: 1.0,
            relative_dates: true,
            tool_labels: true,
            progress_window: true,
            completion_window: false,
            notifications: true,
            auto_update: true,
            language: "system".into(),
        }
    }
}

struct AppState {
    engine: Engine,
    after_all: Mutex<AfterAll>,
    drafts: Mutex<HashMap<String, AddDraft>>,
    next_window: AtomicU64,
    prefs: Mutex<Prefs>,
    prefs_path: std::path::PathBuf,
    update: Mutex<Option<tauri_plugin_updater::Update>>,
}

type CmdResult<T> = Result<T, String>;

fn l<'a>(en: &'a str, tr: &'a str) -> &'a str {
    if pulse_engine::lang::english() { en } else { tr }
}

fn apply_language(app: &AppHandle, prefs: &Prefs) {
    let english = match prefs.language.as_str() {
        "en" => true,
        "tr" => false,
        _ => !pulse_engine::lang::system_turkish(),
    };
    pulse_engine::lang::set_english(english);
    if let (Some(tray), Ok(menu)) = (app.tray_by_id("tray"), tray_menu(app)) {
        let _ = tray.set_menu(Some(menu));
    }
}

fn parse_id(id: &str) -> CmdResult<Uuid> {
    Uuid::parse_str(id).map_err(|e| e.to_string())
}

fn parse_ids(ids: &[String]) -> CmdResult<Vec<Uuid>> {
    ids.iter().map(|s| parse_id(s)).collect()
}

#[tauri::command]
fn list_downloads(state: State<AppState>) -> Vec<DownloadView> {
    state.engine.list()
}

#[tauri::command]
fn add_download(state: State<AppState>, request: AddRequest) -> CmdResult<DownloadView> {
    state.engine.add(request).map_err(|e| e.to_string())
}

#[tauri::command]
fn pause_download(state: State<AppState>, id: String) -> CmdResult<()> {
    state.engine.pause(parse_id(&id)?).map_err(|e| e.to_string())
}

#[tauri::command]
fn resume_download(state: State<AppState>, id: String) -> CmdResult<()> {
    state.engine.resume(parse_id(&id)?).map_err(|e| e.to_string())
}

#[tauri::command]
fn remove_download(state: State<AppState>, id: String, delete_files: bool) -> CmdResult<()> {
    state.engine.remove(parse_id(&id)?, delete_files).map_err(|e| e.to_string())
}

#[tauri::command]
fn pause_all(state: State<AppState>) {
    state.engine.pause_all();
}

#[tauri::command]
fn resume_all(state: State<AppState>) {
    state.engine.resume_all();
}

#[tauri::command]
fn clear_completed(state: State<AppState>) {
    state.engine.clear_completed();
}

#[tauri::command]
fn get_config(state: State<AppState>) -> Config {
    state.engine.config()
}

#[tauri::command]
fn set_config(state: State<AppState>, config: Config) -> CmdResult<Config> {
    state.engine.set_config(config).map_err(|e| e.to_string())
}

#[tauri::command]
fn set_url(state: State<AppState>, id: String, url: String) -> CmdResult<()> {
    state.engine.set_url(parse_id(&id)?, &url).map_err(|e| e.to_string())
}

#[tauri::command]
fn set_options(state: State<AppState>, id: String, options: DownloadOptions) -> CmdResult<()> {
    state.engine.set_options(parse_id(&id)?, options).map_err(|e| e.to_string())
}

#[tauri::command]
fn move_to_queue(state: State<AppState>, ids: Vec<String>, queue: String) -> CmdResult<()> {
    state.engine.move_to_queue(&parse_ids(&ids)?, &queue).map_err(|e| e.to_string())
}

#[tauri::command]
fn start_queue(state: State<AppState>, queue: String) -> CmdResult<()> {
    state.engine.start_queue(&queue).map_err(|e| e.to_string())
}

#[tauri::command]
fn stop_queue(state: State<AppState>, queue: String) -> CmdResult<()> {
    state.engine.stop_queue(&queue).map_err(|e| e.to_string())
}

#[tauri::command]
async fn probe_media(state: State<'_, AppState>, url: String, headers: Vec<(String, String)>) -> CmdResult<MediaInfo> {
    state.engine.probe_media(&url, headers).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn probe_url(state: State<'_, AppState>, url: String, headers: Vec<(String, String)>) -> CmdResult<UrlInfo> {
    state.engine.probe_url(&url, headers).await.map_err(|e| e.to_string())
}

#[tauri::command]
fn take_add_draft(window: tauri::WebviewWindow, state: State<AppState>) -> Option<AddDraft> {
    state.drafts.lock().unwrap().remove(window.label())
}

#[tauri::command]
async fn open_add_window(app: AppHandle, draft: AddDraft) {
    show_add_window(&app, draft);
}

#[tauri::command]
async fn open_progress_window(app: AppHandle, id: String) {
    show_progress_window(&app, &id);
}

#[derive(Serialize)]
struct ChecksumResult {
    algorithm: String,
    actual: String,
    matches: Option<bool>,
}

#[tauri::command]
async fn verify_checksum(
    state: State<'_, AppState>,
    id: String,
    algorithm: Option<String>,
    expected: Option<String>,
) -> CmdResult<ChecksumResult> {
    let d = state.engine.get(parse_id(&id)?).ok_or(l("Download not found", "İndirme bulunamadı"))?;
    if d.status != Status::Completed {
        return Err(l("The file is not finished yet", "Dosya henüz tamamlanmadı").into());
    }
    let expected = expected.map(|e| e.trim().to_ascii_lowercase()).filter(|e| !e.is_empty());
    let algorithm = match (&expected, algorithm) {
        (Some(e), _) => checksum::algorithm_for(e)
            .ok_or(l(
                "Unrecognised hash length (expected MD5, SHA-1, SHA-256 or SHA-512)",
                "Hash uzunluğu tanınmadı (MD5, SHA-1, SHA-256 veya SHA-512 olmalı)",
            ))?
            .to_string(),
        (None, Some(a)) => a,
        (None, None) => "sha256".into(),
    };
    let path = d.dir.join(&d.file_name);
    let algo = algorithm.clone();
    let actual = tauri::async_runtime::spawn_blocking(move || checksum::file_digest(&path, &algo))
        .await
        .map_err(|e| e.to_string())??;
    let matches = expected.map(|e| e == actual);
    Ok(ChecksumResult { algorithm, actual, matches })
}

#[tauri::command]
fn get_after_all(state: State<AppState>) -> AfterAll {
    *state.after_all.lock().unwrap()
}

#[tauri::command]
fn set_after_all(state: State<AppState>, mode: AfterAll) {
    *state.after_all.lock().unwrap() = mode;
}

#[tauri::command]
fn cancel_shutdown() -> CmdResult<()> {
    run_hidden("shutdown", &["/a"])
}

const UPDATE_EVERY: std::time::Duration = std::time::Duration::from_secs(12 * 3600);

#[derive(Debug, Clone, Serialize)]
struct UpdateInfo {
    version: String,
    notes: Option<String>,
}

async fn check_update(app: &AppHandle) -> Result<Option<UpdateInfo>, String> {
    let update = app.updater().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())?;
    let Some(update) = update else { return Ok(None) };
    let info = UpdateInfo { version: update.version.clone(), notes: update.body.clone() };
    *app.state::<AppState>().update.lock().unwrap() = Some(update);
    let _ = app.emit("update-available", info.clone());
    Ok(Some(info))
}

fn spawn_update_checks(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(20)).await;
        let mut announced = None;
        loop {
            let enabled = app.state::<AppState>().prefs.lock().unwrap().auto_update;
            if enabled
                && let Ok(Some(info)) = check_update(&app).await
                && announced.as_deref() != Some(info.version.as_str())
            {
                let _ = app
                    .notification()
                    .builder()
                    .title("Pulse DM")
                    .body(format!("{} {}", l("New version available:", "Yeni sürüm hazır:"), info.version))
                    .show();
                announced = Some(info.version);
            }
            tokio::time::sleep(UPDATE_EVERY).await;
        }
    });
}

#[tauri::command]
async fn check_for_updates(app: AppHandle) -> CmdResult<Option<UpdateInfo>> {
    check_update(&app).await
}

#[tauri::command]
fn pending_update(state: State<AppState>) -> Option<UpdateInfo> {
    state.update.lock().unwrap().as_ref().map(|u| UpdateInfo { version: u.version.clone(), notes: u.body.clone() })
}

#[tauri::command]
async fn install_update(app: AppHandle) -> CmdResult<()> {
    let update =
        app.state::<AppState>().update.lock().unwrap().take().ok_or(l("No update found", "Güncelleme bulunamadı"))?;
    app.state::<AppState>().engine.shutdown().await;
    update.download_and_install(|_, _| {}, || {}).await.map_err(|e| e.to_string())?;
    app.restart();
}

#[tauri::command]
fn get_prefs(state: State<AppState>) -> Prefs {
    state.prefs.lock().unwrap().clone()
}

#[tauri::command]
fn set_prefs(app: AppHandle, state: State<AppState>, mut prefs: Prefs) -> CmdResult<Prefs> {
    prefs.ui_scale = prefs.ui_scale.clamp(0.8, 1.5);
    let json = serde_json::to_vec_pretty(&prefs).map_err(|e| e.to_string())?;
    std::fs::write(&state.prefs_path, json).map_err(|e| e.to_string())?;
    *state.prefs.lock().unwrap() = prefs.clone();
    apply_language(&app, &prefs);
    let _ = app.emit("prefs", prefs.clone());
    Ok(prefs)
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    quit(&app);
}

#[tauri::command]
fn open_file(app: AppHandle, state: State<AppState>, id: String) -> CmdResult<()> {
    let d = state.engine.get(parse_id(&id)?).ok_or(l("Download not found", "İndirme bulunamadı"))?;
    app.opener().open_path(d.dir.join(&d.file_name).to_string_lossy(), None::<&str>).map_err(|e| e.to_string())
}

#[tauri::command]
fn show_in_folder(app: AppHandle, state: State<AppState>, id: String) -> CmdResult<()> {
    let d = state.engine.get(parse_id(&id)?).ok_or(l("Download not found", "İndirme bulunamadı"))?;
    let finished = d.dir.join(&d.file_name);
    let target = if finished.exists() { finished } else { d.dir.clone() };
    app.opener().reveal_item_in_dir(target).map_err(|e| e.to_string())
}

#[tauri::command]
fn open_extension_folder(app: AppHandle) -> CmdResult<String> {
    let bundled = app.path().resource_dir().ok().map(|d| d.join("extension"));
    let dev = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("extension");
    let dir = bundled
        .filter(|d| d.join("manifest.json").exists())
        .or_else(|| dev.join("manifest.json").exists().then_some(dev))
        .ok_or(l("Extension folder not found", "Eklenti klasörü bulunamadı"))?;
    let path = dir.to_string_lossy().replace(r"\\?\", "");
    app.opener().open_path(&path, None::<&str>).map_err(|e| e.to_string())?;
    Ok(path)
}

#[tauri::command]
fn magnet_handler(app: AppHandle) -> bool {
    app.deep_link().is_registered("magnet").unwrap_or(false)
}

#[tauri::command]
fn set_magnet_handler(app: AppHandle, enabled: bool) -> CmdResult<bool> {
    let links = app.deep_link();
    if enabled { links.register("magnet") } else { links.unregister("magnet") }.map_err(|e| e.to_string())?;
    Ok(links.is_registered("magnet").unwrap_or(enabled))
}

#[tauri::command]
fn open_firefox_extension(app: AppHandle) -> CmdResult<()> {
    app.opener()
        .open_url("https://github.com/Burqk1/PulseDM/releases/latest/download/pulse-dm-firefox.xpi", None::<&str>)
        .map_err(|e| e.to_string())
}

fn run_hidden(program: &str, args: &[&str]) -> CmdResult<()> {
    let mut cmd = std::process::Command::new(program);
    cmd.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd.spawn().map(|_| ()).map_err(|e| e.to_string())
}

fn perform_after_all(app: &AppHandle, mode: AfterAll) {
    let result = match mode {
        AfterAll::Nothing => return,
        AfterAll::Shutdown => run_hidden(
            "shutdown",
            &["/s", "/t", "60", "/c", l("Pulse DM: all downloads finished.", "Pulse DM: tüm indirmeler tamamlandı.")],
        ),
        AfterAll::Sleep => run_hidden("rundll32.exe", &["powrprof.dll,SetSuspendState", "0,1,0"]),
    };
    match result {
        Ok(()) => {
            let _ = app.emit("after-all", mode);
        }
        Err(e) => eprintln!("after-all action failed: {e}"),
    }
}

fn spawn_event_pump(app: &AppHandle, engine: &Engine) {
    let mut events = engine.subscribe();
    let app = app.clone();
    let mut last: HashMap<Uuid, Status> = engine.list().into_iter().map(|d| (d.id, d.status)).collect();
    tauri::async_runtime::spawn(async move {
        loop {
            let ev = match events.recv().await {
                Ok(ev) => ev,
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            };
            if let EngineEvent::Changed(d) = &ev {
                let before = last.insert(d.id, d.status);
                if before != Some(d.status) {
                    on_status_change(&app, d);
                }
            }
            let _ = app.emit("engine", ev);
        }
    });
}

fn on_status_change(app: &AppHandle, d: &DownloadView) {
    let prefs = app.state::<AppState>().prefs.lock().unwrap().clone();
    let notify = |title: &str, body: &str| {
        if prefs.notifications {
            let _ = app.notification().builder().title(title).body(body).show();
        }
    };
    if d.status == Status::Completed && prefs.completion_window {
        show_progress_window(app, &d.id.to_string());
    }
    match d.status {
        Status::Completed => notify(l("Download complete", "İndirme tamamlandı"), &d.file_name),
        Status::Failed => notify(
            l("Download failed", "İndirme başarısız"),
            &format!("{}: {}", d.file_name, d.error.clone().unwrap_or_default()),
        ),
        _ => return,
    }
    let state = app.state::<AppState>();
    if state.engine.is_idle() {
        let mode = std::mem::take(&mut *state.after_all.lock().unwrap());
        perform_after_all(app, mode);
    }
}

fn torrent_arg(args: &[String]) -> Option<String> {
    args.iter().skip(1).find(|a| pulse_engine::torrent::is_source(a)).cloned()
}

fn open_torrent(app: &AppHandle, source: String) {
    show_add_window(app, AddDraft { url: source, headers: Vec::new(), file_name: None, media: false });
}

fn handle_args(app: &AppHandle, args: &[String]) {
    if !args.iter().any(|a| a == ADD_ARG) {
        if let Some(source) = torrent_arg(args) {
            open_torrent(app, source);
        }
        return;
    }
    let Some(pos) = args.iter().position(|a| a == ADD_ARG) else { return };
    let Some(payload) = args.get(pos + 1) else { return };
    let request = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<AddRequest>(&bytes).ok());
    let Some(request) = request else {
        eprintln!("ignoring malformed {ADD_ARG} payload");
        return;
    };
    show_add_window(
        app,
        AddDraft {
            media: request.media_option.as_deref() == Some(MEDIA_ASK),
            url: request.url,
            headers: request.headers,
            file_name: request.file_name,
        },
    );
}

fn popup_window(app: &AppHandle, label: &str, route: &str, title: &str, size: (f64, f64)) -> tauri::Result<()> {
    WebviewWindowBuilder::new(app, label, WebviewUrl::App(route.into()))
        .title(title)
        .inner_size(size.0, size.1)
        .min_inner_size(380.0, 140.0)
        .decorations(false)
        .shadow(true)
        .additional_browser_args(BROWSER_ARGS)
        .always_on_top(true)
        .skip_taskbar(false)
        .visible(false)
        .center()
        .focused(true)
        .build()
        .map(|_| ())
}

fn show_add_window(app: &AppHandle, draft: AddDraft) {
    let state = app.state::<AppState>();
    let label = format!("add-{}", state.next_window.fetch_add(1, Ordering::Relaxed));
    state.drafts.lock().unwrap().insert(label.clone(), draft);
    if let Err(e) = popup_window(app, &label, "add", l("Add download", "İndirme ekle"), (600.0, 250.0)) {
        eprintln!("add window: {e}");
    }
}

fn show_progress_window(app: &AppHandle, id: &str) {
    let label = format!("progress-{id}");
    if let Some(w) = app.get_webview_window(&label) {
        let _ = w.unminimize();
        let _ = w.set_focus();
        return;
    }
    if let Err(e) = popup_window(app, &label, "progress", "Pulse DM", (440.0, 260.0)) {
        eprintln!("progress window: {e}");
    }
}

fn show_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(MAIN_WINDOW) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let _ = WebviewWindowBuilder::new(app, MAIN_WINDOW, WebviewUrl::default())
        .title("Pulse DM")
        .inner_size(1000.0, 560.0)
        .min_inner_size(760.0, 420.0)
        .decorations(false)
        .shadow(true)
        .additional_browser_args(BROWSER_ARGS)
        .center()
        .build();
}

fn quit(app: &AppHandle) {
    let engine = app.state::<AppState>().engine.clone();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        engine.shutdown().await;
        app.exit(0);
    });
}

fn tray_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let show = MenuItem::with_id(app, "show", l("Open Pulse DM", "Pulse DM'i aç"), true, None::<&str>)?;
    let pause = MenuItem::with_id(app, "pause_all", l("Pause all", "Tümünü duraklat"), true, None::<&str>)?;
    let resume = MenuItem::with_id(app, "resume_all", l("Resume all", "Tümünü sürdür"), true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", l("Exit", "Çıkış"), true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    Menu::with_items(app, &[&show, &pause, &resume, &sep, &quit_item])
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let menu = tray_menu(app)?;
    TrayIconBuilder::with_id("tray")
        .icon(app.default_window_icon().cloned().expect("app icon"))
        .tooltip("Pulse DM")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main_window(app),
            "pause_all" => app.state::<AppState>().engine.pause_all(),
            "resume_all" => app.state::<AppState>().engine.resume_all(),
            "quit" => quit(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if args.iter().any(|a| a == ADD_ARG) || torrent_arg(&args).is_some() {
                handle_args(app, &args);
            } else {
                show_main_window(app);
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![HIDDEN_ARG]),
        ))
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let prefs: Prefs = std::fs::read(data_dir.join("prefs.json"))
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default();
            apply_language(app.handle(), &prefs);
            let engine =
                tauri::async_runtime::block_on(async { Engine::start(Config::default(), Some(data_dir.clone())) })?;
            migrate_download_folder(&engine, &data_dir);
            spawn_event_pump(app.handle(), &engine);
            app.manage(AppState {
                engine,
                after_all: Mutex::new(AfterAll::Nothing),
                drafts: Mutex::new(HashMap::new()),
                next_window: AtomicU64::new(1),
                prefs: Mutex::new(prefs),
                prefs_path: data_dir.join("prefs.json"),
                update: Mutex::new(None),
            });

            if let Err(e) = native_host::register(&data_dir) {
                eprintln!("browser integration not registered: {e}");
            }
            build_tray(app.handle())?;
            spawn_update_checks(app.handle());

            let first_run_marker = data_dir.join(".autostart-initialized");
            if !first_run_marker.exists() {
                use tauri_plugin_autostart::ManagerExt;
                let _ = app.autolaunch().enable();
                let _ = std::fs::write(&first_run_marker, b"");
            }

            let handle = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                for url in event.urls() {
                    if pulse_engine::torrent::is_source(url.as_str()) {
                        open_torrent(&handle, url.to_string());
                    }
                }
            });

            let args: Vec<String> = std::env::args().collect();
            if !args.iter().any(|a| a == HIDDEN_ARG || a == ADD_ARG) && torrent_arg(&args).is_none() {
                show_main_window(app.handle());
            }
            handle_args(app.handle(), &args);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_downloads,
            add_download,
            pause_download,
            resume_download,
            remove_download,
            pause_all,
            resume_all,
            clear_completed,
            get_config,
            set_config,
            set_url,
            set_options,
            move_to_queue,
            start_queue,
            stop_queue,
            verify_checksum,
            probe_media,
            probe_url,
            take_add_draft,
            open_add_window,
            open_progress_window,
            get_prefs,
            set_prefs,
            check_for_updates,
            pending_update,
            install_update,
            get_after_all,
            set_after_all,
            cancel_shutdown,
            open_extension_folder,
            open_firefox_extension,
            magnet_handler,
            set_magnet_handler,
            quit_app,
            open_file,
            show_in_folder,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Pulse DM");

    app.run(|_app, event| match event {
        RunEvent::ExitRequested { code: None, api, .. } => api.prevent_exit(),
        #[cfg(target_os = "macos")]
        RunEvent::Opened { urls } => {
            for url in urls {
                let source =
                    url.to_file_path().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| url.to_string());
                if pulse_engine::torrent::is_source(&source) {
                    open_torrent(_app, source);
                }
            }
        }
        _ => {}
    });
}
