use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use pulse_engine::{AddRequest, Config, DownloadOptions, DownloadView, Engine, PART_EXT, QueueConfig, Status};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use uuid::Uuid;

type Routes = HashMap<String, (String, Vec<u8>)>;

#[derive(Clone, Default)]
struct Opts {
    no_ranges: bool,
    delay: Duration,
    max_concurrent: usize,
    html: bool,
    auth: Option<String>,
    routes: Option<Arc<Routes>>,
}

async fn serve(data: Arc<Vec<u8>>, opts: Opts) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let active = Arc::new(AtomicUsize::new(0));
    tokio::spawn(async move {
        loop {
            let (sock, _) = listener.accept().await.unwrap();
            tokio::spawn(handle(sock, data.clone(), opts.clone(), active.clone()));
        }
    });
    format!("http://{addr}/files/test%20file.bin")
}

async fn handle(sock: TcpStream, data: Arc<Vec<u8>>, opts: Opts, active: Arc<AtomicUsize>) {
    let mut reader = BufReader::new(sock);
    let mut range = None;
    let mut auth = None;
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).await.unwrap_or(0) == 0 {
        return;
    }
    let path = request_line.split_whitespace().nth(1).unwrap_or("/").to_string();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
            return;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(v) = lower.strip_prefix("range: bytes=") {
            let (a, b) = v.split_once('-').unwrap();
            range = Some((a.parse::<usize>().unwrap(), b.parse::<usize>().ok()));
        } else if lower.starts_with("authorization:") {
            auth = Some(line["authorization:".len()..].trim().to_string());
        }
    }
    let mut sock = reader.into_inner();

    if opts.auth.is_some() && auth != opts.auth {
        let _ = sock.write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
        return;
    }
    let in_flight = active.fetch_add(1, Ordering::SeqCst) + 1;
    struct Guard(Arc<AtomicUsize>);
    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }
    let _guard = Guard(active);
    if opts.max_concurrent > 0 && in_flight > opts.max_concurrent {
        let _ = sock
            .write_all(
                b"HTTP/1.1 503 Service Unavailable\r\nRetry-After: 0\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .await;
        return;
    }
    if opts.html {
        let page = b"<html><body>Link expired</body></html>";
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            page.len()
        );
        let _ = sock.write_all(head.as_bytes()).await;
        let _ = sock.write_all(page).await;
        return;
    }

    if let Some(routes) = &opts.routes {
        let Some((ctype, body)) = routes.get(&path) else {
            let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
            return;
        };
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        if sock.write_all(head.as_bytes()).await.is_err() {
            return;
        }
        for chunk in body.chunks(64 * 1024) {
            if sock.write_all(chunk).await.is_err() {
                return;
            }
            if !opts.delay.is_zero() {
                tokio::time::sleep(opts.delay).await;
            }
        }
        return;
    }

    let len = data.len();
    let (head, body) = match range {
        Some((start, end)) if !opts.no_ranges => {
            let end = end.unwrap_or(len - 1).min(len - 1);
            (
                format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Type: application/octet-stream\r\nContent-Range: bytes {start}-{end}/{len}\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nETag: \"v1\"\r\nLast-Modified: Wed, 21 Oct 2015 07:28:00 GMT\r\nConnection: close\r\n\r\n",
                    end - start + 1
                ),
                &data[start..=end],
            )
        }
        _ => (format!("HTTP/1.1 200 OK\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n"), &data[..]),
    };
    if sock.write_all(head.as_bytes()).await.is_err() {
        return;
    }
    for chunk in body.chunks(64 * 1024) {
        if sock.write_all(chunk).await.is_err() {
            return;
        }
        if !opts.delay.is_zero() {
            tokio::time::sleep(opts.delay).await;
        }
    }
}

fn test_data(len: usize) -> Arc<Vec<u8>> {
    Arc::new((0..len).map(|i| ((i * 31 + i / 997) % 251) as u8).collect())
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pulse-test-{name}-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn config(dir: &Path, connections: usize) -> Config {
    Config {
        download_dir: dir.to_path_buf(),
        connections,
        min_split: 128 * 1024,
        max_retries: 3,
        category_folders: false,
        ..Config::default()
    }
}

async fn wait_for(engine: &Engine, id: Uuid, what: &str, pred: impl Fn(&DownloadView) -> bool) -> DownloadView {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let v = engine.get(id).unwrap();
        if pred(&v) {
            return v;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}: {v:?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn add(engine: &Engine, url: &str) -> Uuid {
    engine.add(AddRequest { url: url.into(), ..Default::default() }).unwrap().id
}

fn finished(v: &DownloadView) -> bool {
    matches!(v.status, Status::Completed | Status::Failed)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn segmented_download_is_byte_exact() {
    let data = test_data(5 * 1024 * 1024 + 123);
    let url = serve(data.clone(), Opts::default()).await;
    let dir = temp_dir("seg");
    let engine = Engine::start(config(&dir, 8), None).unwrap();

    let id = add(&engine, &url);
    let v = wait_for(&engine, id, "completion", |v| v.status == Status::Completed).await;

    assert_eq!(v.file_name, "test file.bin");
    assert!(v.resumable);
    assert_eq!(v.size, Some(data.len() as u64));
    assert_eq!(std::fs::read(dir.join("test file.bin")).unwrap(), *data);
    assert!(!dir.join(format!("test file.bin.{PART_EXT}")).exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn server_without_ranges_uses_single_stream() {
    let data = test_data(3 * 1024 * 1024);
    let url = serve(data.clone(), Opts { no_ranges: true, ..Default::default() }).await;
    let dir = temp_dir("norange");
    let engine = Engine::start(config(&dir, 8), None).unwrap();

    let id = add(&engine, &url);
    let v = wait_for(&engine, id, "completion", |v| v.status == Status::Completed).await;

    assert!(!v.resumable);
    assert_eq!(std::fs::read(dir.join("test file.bin")).unwrap(), *data);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pause_and_resume_keeps_progress() {
    let data = test_data(4 * 1024 * 1024);
    let url = serve(data.clone(), Opts { delay: Duration::from_millis(10), ..Default::default() }).await;
    let dir = temp_dir("pause");
    let engine = Engine::start(config(&dir, 4), None).unwrap();

    let id = add(&engine, &url);
    wait_for(&engine, id, "some progress", |v| v.downloaded > 512 * 1024).await;
    engine.pause(id).unwrap();
    let paused = wait_for(&engine, id, "pause", |v| v.status == Status::Paused && v.connections == 0).await;
    assert!(paused.downloaded > 0 && paused.downloaded < data.len() as u64);
    assert!(dir.join(format!("test file.bin.{PART_EXT}")).exists());

    engine.resume(id).unwrap();
    wait_for(&engine, id, "completion", |v| v.status == Status::Completed).await;
    assert_eq!(std::fs::read(dir.join("test file.bin")).unwrap(), *data);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn restart_continues_from_saved_state() {
    let data = test_data(4 * 1024 * 1024);
    let url = serve(data.clone(), Opts { delay: Duration::from_millis(10), ..Default::default() }).await;
    let dir = temp_dir("restart");
    let state = dir.join("state");

    let engine = Engine::start(config(&dir, 4), Some(state.clone())).unwrap();
    let id = add(&engine, &url);
    wait_for(&engine, id, "some progress", |v| v.downloaded > 512 * 1024).await;
    engine.shutdown().await;
    drop(engine);

    let engine = Engine::start(Config::default(), Some(state)).unwrap();
    assert_eq!(engine.config().download_dir, dir, "settings are restored from the state dir");
    let before = engine.get(id).unwrap().downloaded;
    assert!(before > 0, "progress should survive a restart");
    wait_for(&engine, id, "completion", |v| v.status == Status::Completed).await;
    assert_eq!(std::fs::read(dir.join("test file.bin")).unwrap(), *data);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn duplicate_names_get_a_counter() {
    let data = test_data(300 * 1024);
    let url = serve(data.clone(), Opts::default()).await;
    let dir = temp_dir("dup");
    let engine = Engine::start(config(&dir, 4), None).unwrap();

    let a = add(&engine, &url);
    wait_for(&engine, a, "first", |v| v.status == Status::Completed).await;
    let b = add(&engine, &url);
    let v = wait_for(&engine, b, "second", |v| v.status == Status::Completed).await;
    assert_eq!(v.file_name, "test file (1).bin");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn simultaneous_same_name_downloads_do_not_collide() {
    let data = test_data(2 * 1024 * 1024);
    let url = serve(data.clone(), Opts::default()).await;
    let dir = temp_dir("race");
    let engine = Engine::start(config(&dir, 4), None).unwrap();

    let ids: Vec<Uuid> = (0..3).map(|_| add(&engine, &url)).collect();
    let mut names = Vec::new();
    for id in ids {
        let v = wait_for(&engine, id, "completion", finished).await;
        assert_eq!(v.status, Status::Completed, "{v:?}");
        assert_eq!(std::fs::read(dir.join(&v.file_name)).unwrap(), *data);
        names.push(v.file_name);
    }
    names.sort();
    names.dedup();
    assert_eq!(
        names.len(),
        3,
        "each download needs its own file: {names:?} files={:?}",
        std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect::<Vec<_>>()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn adding_the_same_file_again_never_touches_the_running_one() {
    let data = test_data(3 * 1024 * 1024);
    let url = serve(data.clone(), Opts { delay: Duration::from_millis(10), ..Default::default() }).await;
    let dir = temp_dir("again");
    let engine = Engine::start(config(&dir, 4), None).unwrap();

    let a = add(&engine, &url);
    wait_for(&engine, a, "first running", |v| v.downloaded > 256 * 1024).await;
    let b = add(&engine, &url);
    let b_removed = add(&engine, &url);
    engine.remove(b_removed, true).unwrap();

    let va = wait_for(&engine, a, "first", finished).await;
    let vb = wait_for(&engine, b, "second", finished).await;
    assert_eq!(va.status, Status::Completed, "{va:?}");
    assert_eq!(vb.status, Status::Completed, "{vb:?}");
    assert_ne!(va.file_name, vb.file_name);
    assert_eq!(std::fs::read(dir.join(&va.file_name)).unwrap(), *data);
    assert_eq!(std::fs::read(dir.join(&vb.file_name)).unwrap(), *data);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn adapts_to_servers_that_limit_connections() {
    let data = test_data(4 * 1024 * 1024);
    let url =
        serve(data.clone(), Opts { max_concurrent: 2, delay: Duration::from_millis(2), ..Default::default() }).await;
    let dir = temp_dir("limit");
    let engine = Engine::start(config(&dir, 8), None).unwrap();

    let id = add(&engine, &url);
    let v = wait_for(&engine, id, "completion", finished).await;
    assert_eq!(v.status, Status::Completed, "{v:?}");
    assert_eq!(std::fs::read(dir.join("test file.bin")).unwrap(), *data);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn expired_link_is_detected_and_can_be_replaced() {
    let data = test_data(3 * 1024 * 1024);
    let good = serve(data.clone(), Opts { delay: Duration::from_millis(10), ..Default::default() }).await;
    let expired = serve(data.clone(), Opts { html: true, ..Default::default() }).await;
    let dir = temp_dir("expired");
    let engine = Engine::start(config(&dir, 4), None).unwrap();

    let id = add(&engine, &expired);
    let v = wait_for(&engine, id, "failure", finished).await;
    assert_eq!(v.status, Status::Failed);
    assert!(v.error.unwrap().contains("web sayfası"));

    let id = add(&engine, &good);
    wait_for(&engine, id, "some progress", |v| v.downloaded > 256 * 1024).await;
    engine.pause(id).unwrap();
    wait_for(&engine, id, "pause", |v| v.status == Status::Paused && v.connections == 0).await;
    engine.set_url(id, &expired).unwrap();
    engine.resume(id).unwrap();
    let v = wait_for(&engine, id, "failure", finished).await;
    assert_eq!(v.status, Status::Failed);

    let kept = v.downloaded;
    assert!(kept > 0);
    engine.set_url(id, &good).unwrap();
    engine.resume(id).unwrap();
    let v = wait_for(&engine, id, "completion", finished).await;
    assert_eq!(v.status, Status::Completed, "{v:?}");
    assert_eq!(std::fs::read(dir.join(&v.file_name)).unwrap(), *data);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn connection_count_changes_while_running() {
    let data = test_data(6 * 1024 * 1024);
    let url = serve(data.clone(), Opts { delay: Duration::from_millis(15), ..Default::default() }).await;
    let dir = temp_dir("live");
    let engine = Engine::start(config(&dir, 2), None).unwrap();

    let id = add(&engine, &url);
    wait_for(&engine, id, "2 connections", |v| v.connections == 2).await;
    engine.set_options(id, DownloadOptions { connections: Some(6), speed_limit: 0 }).unwrap();
    wait_for(&engine, id, "more connections", |v| v.connections > 2 || v.status == Status::Completed).await;
    engine.set_options(id, DownloadOptions { connections: Some(1), speed_limit: 0 }).unwrap();
    wait_for(&engine, id, "one connection", |v| v.connections <= 1).await;
    let v = wait_for(&engine, id, "completion", finished).await;
    assert_eq!(v.status, Status::Completed, "{v:?}");
    assert_eq!(std::fs::read(dir.join("test file.bin")).unwrap(), *data);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stopped_queue_waits_until_started() {
    let data = test_data(512 * 1024);
    let url = serve(data.clone(), Opts::default()).await;
    let dir = temp_dir("queue");
    let mut cfg = config(&dir, 4);
    cfg.queues.push(QueueConfig { id: "night".into(), name: "Gece".into(), ..Default::default() });
    let engine = Engine::start(cfg, None).unwrap();

    let id = engine.add(AddRequest { url: url.clone(), queue: Some("night".into()), ..Default::default() }).unwrap().id;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(engine.get(id).unwrap().status, Status::Queued, "queue is stopped");

    engine.start_queue("night").unwrap();
    wait_for(&engine, id, "completion", |v| v.status == Status::Completed).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn basic_auth_is_sent() {
    let data = test_data(256 * 1024);
    let url = serve(data.clone(), Opts { auth: Some("Basic dXNlcjpwYXNz".into()), ..Default::default() }).await;
    let dir = temp_dir("auth");
    let engine = Engine::start(config(&dir, 2), None).unwrap();

    let id = add(&engine, &url);
    let v = wait_for(&engine, id, "failure", finished).await;
    assert!(v.error.unwrap().contains("401"));

    let id = engine
        .add(AddRequest { url, username: Some("user".into()), password: Some("pass".into()), ..Default::default() })
        .unwrap()
        .id;
    let v = wait_for(&engine, id, "completion", finished).await;
    assert_eq!(v.status, Status::Completed, "{v:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn per_download_speed_limit_is_enforced() {
    let data = test_data(600 * 1024);
    let url = serve(data.clone(), Opts::default()).await;
    let dir = temp_dir("limit-dl");
    let engine = Engine::start(config(&dir, 4), None).unwrap();

    let started = Instant::now();
    let id = engine.add(AddRequest { url, speed_limit: Some(256 * 1024), ..Default::default() }).unwrap().id;
    wait_for(&engine, id, "completion", |v| v.status == Status::Completed).await;
    assert!(started.elapsed() >= Duration::from_millis(1200), "took {:?}", started.elapsed());
}

async fn hls_site(segments: usize, delay: Duration) -> (String, Vec<u8>) {
    use aes::cipher::{BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};
    let key = [7u8; 16];
    let mut routes = HashMap::new();
    let mut expected = Vec::new();
    let mut media = String::from(
        "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXT-X-MEDIA-SEQUENCE:3\n#EXT-X-KEY:METHOD=AES-128,URI=\"../key.bin\"\n",
    );
    for i in 0..segments {
        let plain: Vec<u8> = (0..100_000 + i * 777).map(|j| ((j * 7 + i * 13) % 251) as u8).collect();
        let mut iv = [0u8; 16];
        iv[8..].copy_from_slice(&(3 + i as u64).to_be_bytes());
        let enc = cbc::Encryptor::<aes::Aes128>::new(&key.into(), &iv.into()).encrypt_padded_vec_mut::<Pkcs7>(&plain);
        routes.insert(format!("/hi/s{i}.ts"), ("video/mp2t".to_string(), enc));
        media.push_str(&format!("#EXTINF:4.0,\ns{i}.ts\n"));
        expected.extend_from_slice(&plain);
    }
    media.push_str("#EXT-X-ENDLIST\n");
    let master = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=200000\nlo/index.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=6000000\nhi/index.m3u8\n";
    let mpegurl = "application/vnd.apple.mpegurl".to_string();
    routes.insert("/master.m3u8".into(), (mpegurl.clone(), master.as_bytes().to_vec()));
    routes.insert("/hi/index.m3u8".into(), (mpegurl.clone(), media.into_bytes()));
    routes.insert("/lo/index.m3u8".into(), (mpegurl, b"#EXTM3U\n#EXTINF:4,\nbad.ts\n#EXT-X-ENDLIST\n".to_vec()));
    routes.insert("/key.bin".into(), ("application/octet-stream".into(), key.to_vec()));

    let base = serve(Arc::new(Vec::new()), Opts { routes: Some(Arc::new(routes)), delay, ..Default::default() }).await;
    let root = base.split("/files/").next().unwrap().to_string();
    (format!("{root}/master.m3u8"), expected)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hls_video_is_downloaded_decrypted_and_joined() {
    let (url, expected) = hls_site(12, Duration::ZERO).await;
    let dir = temp_dir("hls");
    let engine = Engine::start(config(&dir, 4), None).unwrap();

    let id = add(&engine, &url);
    let v = wait_for(&engine, id, "completion", finished).await;
    assert_eq!(v.status, Status::Completed, "{v:?}");
    assert_eq!(v.file_name, "master.ts");
    assert_eq!(v.parts, Some((12, 12)));
    assert_eq!(std::fs::read(dir.join("master.ts")).unwrap(), expected, "best quality, decrypted, in order");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hls_video_pauses_and_resumes() {
    let (url, expected) = hls_site(40, Duration::from_millis(30)).await;
    let dir = temp_dir("hls-pause");
    let engine = Engine::start(config(&dir, 4), None).unwrap();

    let id = engine.add(AddRequest { url, file_name: Some("Film.ts".into()), ..Default::default() }).unwrap().id;
    wait_for(&engine, id, "some segments", |v| v.parts.is_some_and(|(done, _)| done >= 5)).await;
    engine.pause(id).unwrap();
    let paused = wait_for(&engine, id, "pause", |v| v.status == Status::Paused).await;
    let (done, total) = paused.parts.unwrap();
    assert!(done >= 5 && done < total, "{done}/{total}");

    engine.resume(id).unwrap();
    let v = wait_for(&engine, id, "completion", finished).await;
    assert_eq!(v.status, Status::Completed, "{v:?}");
    assert_eq!(std::fs::read(dir.join("Film.ts")).unwrap(), expected);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn adds_connections_when_each_one_is_capped() {
    let data = test_data(40 * 1024 * 1024);
    let url = serve(data.clone(), Opts { delay: Duration::from_millis(40), ..Default::default() }).await;
    let dir = temp_dir("ramp");
    let engine = Engine::start(Config { max_auto_connections: 8, ..config(&dir, 2) }, None).unwrap();

    let id = add(&engine, &url);
    let mut most = 0;
    let deadline = Instant::now() + Duration::from_secs(40);
    let v = loop {
        let v = engine.get(id).unwrap();
        most = most.max(v.connections);
        if finished(&v) {
            break v;
        }
        assert!(Instant::now() < deadline, "timed out: {v:?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(v.status, Status::Completed, "{v:?}");
    assert!(most > 2, "should have ramped up from 2 connections, saw at most {most}");
    assert_eq!(std::fs::read(dir.join("test file.bin")).unwrap(), *data);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn keeps_server_time_when_enabled() {
    let data = test_data(300 * 1024);
    let url = serve(data.clone(), Opts::default()).await;
    let dir = temp_dir("mtime");
    let engine = Engine::start(Config { keep_server_time: true, ..config(&dir, 2) }, None).unwrap();

    let id = add(&engine, &url);
    let v = wait_for(&engine, id, "completion", finished).await;
    assert_eq!(v.status, Status::Completed, "{v:?}");
    let modified = std::fs::metadata(dir.join(&v.file_name)).unwrap().modified().unwrap();
    let start_of_2016 = std::time::UNIX_EPOCH + Duration::from_secs(1_451_606_400);
    assert!(modified < start_of_2016, "file should carry the server's 2015 date");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn category_folders_sort_by_type() {
    let dir = temp_dir("cats");
    let engine = Engine::start(Config { category_folders: true, ..config(&dir, 2) }, None).unwrap();
    let paused = |url: &str| AddRequest { url: url.into(), paused: true, ..Default::default() };
    let video = engine.add(paused("http://127.0.0.1:1/a/film.mp4")).unwrap();
    let other = engine.add(paused("http://127.0.0.1:1/a/notes.xyz")).unwrap();
    assert_eq!(video.dir.parent(), Some(dir.as_path()));
    assert!(video.dir.file_name().unwrap().to_string_lossy().starts_with("Video"));
    assert_eq!(other.dir, dir, "unknown types stay in the main folder");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rejects_non_http_urls() {
    let dir = temp_dir("bad");
    let engine = Engine::start(config(&dir, 4), None).unwrap();
    assert!(engine.add(AddRequest { url: "ftp://x/y".into(), ..Default::default() }).is_err());
    assert!(engine.add(AddRequest { url: "not a url".into(), ..Default::default() }).is_err());
}
