use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use aes::cipher::{BlockDecryptMut, KeyIvInit, block_padding::Pkcs7};
use futures_util::StreamExt;
use reqwest::header::{HeaderMap, RANGE};
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;

use crate::download::{JobControl, check_status};
use crate::error::{EngineError, Result};
use crate::limiter::RateLimiter;
use crate::segment::SegmentTable;

type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HlsState {
    pub playlist_url: String,
    pub total: usize,
    pub written: usize,
    pub out_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HlsSegment {
    pub uri: String,
    pub key: Option<Key>,
    pub range: Option<(u64, u64)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key {
    pub uri: String,
    pub iv: [u8; 16],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaPlaylist {
    pub init: Option<HlsSegment>,
    pub segments: Vec<HlsSegment>,
}

pub fn looks_like_playlist(url: &str, content_type: Option<&str>) -> bool {
    let path = url.split(['?', '#']).next().unwrap_or("").to_ascii_lowercase();
    path.ends_with(".m3u8") || content_type.is_some_and(|ct| ct.to_ascii_lowercase().contains("mpegurl"))
}

pub fn is_master(text: &str) -> bool {
    text.contains("#EXT-X-STREAM-INF")
}

pub fn best_variant(text: &str, base: &Url) -> Option<String> {
    let mut best: Option<(u64, String)> = None;
    let mut pending = None;
    for line in text.lines().map(str::trim) {
        if let Some(attrs) = line.strip_prefix("#EXT-X-STREAM-INF:") {
            pending = Some(attr(attrs, "BANDWIDTH").and_then(|b| b.parse().ok()).unwrap_or(0));
        } else if !line.is_empty()
            && !line.starts_with('#')
            && let Some(bw) = pending.take()
        {
            let uri = base.join(line).ok()?.to_string();
            if best.as_ref().is_none_or(|(b, _)| bw > *b) {
                best = Some((bw, uri));
            }
        }
    }
    best.map(|(_, uri)| uri)
}

pub fn parse_media(text: &str, base: &Url) -> Result<MediaPlaylist> {
    let bad = |m: &str| EngineError::BadResponse(m.to_string());
    if !text.trim_start().starts_with("#EXTM3U") {
        return Err(bad("geçerli bir HLS listesi değil"));
    }
    if !text.contains("#EXT-X-ENDLIST") {
        return Err(bad("canlı yayınlar henüz desteklenmiyor"));
    }
    let join = |u: &str| base.join(u).map(|u| u.to_string()).map_err(|e| bad(&e.to_string()));

    let mut seq = 0u64;
    let mut key: Option<(String, Option<[u8; 16]>)> = None;
    let mut range: Option<(u64, u64)> = None;
    let mut next_offset = 0u64;
    let mut init = None;
    let mut segments = Vec::new();

    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(v) = line.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
            seq = v.trim().parse().unwrap_or(0);
        } else if let Some(attrs) = line.strip_prefix("#EXT-X-KEY:") {
            key = match attr(attrs, "METHOD").as_deref() {
                Some("NONE") | None => None,
                Some("AES-128") => {
                    let uri = attr(attrs, "URI").ok_or_else(|| bad("şifre anahtarı adresi yok"))?;
                    Some((join(&uri)?, attr(attrs, "IV").and_then(|iv| parse_iv(&iv))))
                }
                Some(_) => return Err(bad("bu video DRM (SAMPLE-AES) ile korunuyor, indirilemez")),
            };
        } else if let Some(attrs) = line.strip_prefix("#EXT-X-MAP:") {
            let uri = attr(attrs, "URI").ok_or_else(|| bad("EXT-X-MAP adresi yok"))?;
            let range = attr(attrs, "BYTERANGE").and_then(|r| parse_byterange(&r, 0));
            init = Some(HlsSegment { uri: join(&uri)?, key: None, range });
        } else if let Some(v) = line.strip_prefix("#EXT-X-BYTERANGE:") {
            range = parse_byterange(v, next_offset);
        } else if !line.starts_with('#') {
            if let Some((_, len)) = range {
                next_offset = range.map(|(o, _)| o).unwrap_or(0) + len;
            }
            segments.push(HlsSegment {
                uri: join(line)?,
                key: key.as_ref().map(|(uri, iv)| Key { uri: uri.clone(), iv: iv.unwrap_or_else(|| seq_iv(seq)) }),
                range: range.take(),
            });
            seq += 1;
        }
    }
    if segments.is_empty() {
        return Err(bad("HLS listesinde parça yok"));
    }
    Ok(MediaPlaylist { init, segments })
}

fn attr(attrs: &str, name: &str) -> Option<String> {
    let mut rest = attrs;
    while !rest.is_empty() {
        let (key, after) = rest.split_once('=')?;
        let (value, next) = if let Some(q) = after.strip_prefix('"') {
            let end = q.find('"')?;
            (&q[..end], q[end + 1..].trim_start_matches(','))
        } else {
            match after.split_once(',') {
                Some((v, n)) => (v, n),
                None => (after, ""),
            }
        };
        if key.trim().eq_ignore_ascii_case(name) {
            return Some(value.trim().to_string());
        }
        rest = next.trim_start();
    }
    None
}

fn parse_iv(hex: &str) -> Option<[u8; 16]> {
    let hex = hex.trim().trim_start_matches("0x").trim_start_matches("0X");
    if hex.len() != 32 {
        return None;
    }
    let mut iv = [0u8; 16];
    for (i, b) in iv.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(iv)
}

fn seq_iv(seq: u64) -> [u8; 16] {
    let mut iv = [0u8; 16];
    iv[8..].copy_from_slice(&seq.to_be_bytes());
    iv
}

fn parse_byterange(v: &str, default_offset: u64) -> Option<(u64, u64)> {
    let (len, off) = match v.trim().split_once('@') {
        Some((l, o)) => (l.trim().parse().ok()?, o.trim().parse().ok()?),
        None => (v.trim().parse().ok()?, default_offset),
    };
    Some((off, len))
}

pub(crate) async fn fetch_text(client: &Client, url: &str, headers: &HeaderMap) -> Result<String> {
    let resp = client.get(url).headers(headers.clone()).send().await?;
    check_status(resp.status(), resp.headers())?;
    if !resp.status().is_success() {
        return Err(EngineError::BadResponse(format!("HTTP {}", resp.status().as_u16())));
    }
    Ok(resp.text().await?)
}

pub(crate) async fn load(client: &Client, url: &str, headers: &HeaderMap) -> Result<MediaPlaylist> {
    let text = fetch_text(client, url, headers).await?;
    let base = Url::parse(url).map_err(|e| EngineError::InvalidUrl(e.to_string()))?;
    if is_master(&text) {
        let variant =
            best_variant(&text, &base).ok_or_else(|| EngineError::BadResponse("video kalitesi bulunamadı".into()))?;
        let text = fetch_text(client, &variant, headers).await?;
        parse_media(&text, &Url::parse(&variant).map_err(|e| EngineError::InvalidUrl(e.to_string()))?)
    } else {
        parse_media(&text, &base)
    }
}

pub(crate) struct HlsJob {
    pub client: Client,
    pub headers: HeaderMap,
    pub limiters: Vec<Arc<RateLimiter>>,
    pub control: Arc<JobControl>,
    pub cancel: CancellationToken,
    pub counter: Arc<SegmentTable>,
    pub max_retries: u32,
    pub output: PathBuf,
    pub state: Arc<Mutex<HlsState>>,
}

struct Writer {
    next: usize,
    ready: HashMap<usize, Vec<u8>>,
    file: std::fs::File,
}

pub(crate) async fn run(job: Arc<HlsJob>, playlist: MediaPlaylist) -> Result<()> {
    let start = job.state.lock().unwrap().written;
    let total = playlist.segments.len();
    let file = std::fs::OpenOptions::new().append(true).open(&job.output)?;
    let writer = Arc::new(AsyncMutex::new(Writer { next: start, ready: HashMap::new(), file }));

    if start == 0
        && let Some(init) = &playlist.init
    {
        let data = fetch_segment(&job, init, &AsyncMutex::new(HashMap::new())).await?;
        let mut w = writer.lock().await;
        w.file.write_all(&data)?;
        job.state.lock().unwrap().out_size += data.len() as u64;
    }

    let playlist = Arc::new(playlist);
    let next_index = Arc::new(AtomicUsize::new(start));
    let keys = Arc::new(AsyncMutex::new(HashMap::new()));
    let workers = job.control.connections().clamp(1, 16);
    let window = workers * 2;

    let mut set = tokio::task::JoinSet::new();
    for _ in 0..workers {
        let (job, playlist, next_index, keys, writer) =
            (job.clone(), playlist.clone(), next_index.clone(), keys.clone(), writer.clone());
        set.spawn(async move {
            loop {
                let idx = next_index.fetch_add(1, Ordering::SeqCst);
                if idx >= total {
                    return Ok(());
                }
                while idx >= writer.lock().await.next + window {
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_millis(50)) => {}
                        _ = job.cancel.cancelled() => return Err(EngineError::Cancelled),
                    }
                }
                let data = fetch_with_retries(&job, &playlist.segments[idx], &keys).await?;
                let mut w = writer.lock().await;
                w.ready.insert(idx, data);
                loop {
                    let next = w.next;
                    let Some(data) = w.ready.remove(&next) else {
                        break;
                    };
                    w.file.write_all(&data)?;
                    w.next += 1;
                    let mut st = job.state.lock().unwrap();
                    st.written = w.next;
                    st.out_size += data.len() as u64;
                }
            }
        });
    }

    let mut first_error = None;
    while let Some(res) = set.join_next().await {
        if let Err(e) = res.expect("hls worker panicked") {
            job.cancel.cancel();
            if first_error.is_none() || matches!(first_error, Some(EngineError::Cancelled)) {
                first_error = Some(e);
            }
        }
    }
    writer.lock().await.file.flush()?;
    match first_error {
        Some(e) => Err(e),
        None if job.state.lock().unwrap().written == total => Ok(()),
        None => Err(EngineError::BadResponse("video tamamlanamadı".into())),
    }
}

async fn fetch_with_retries(
    job: &HlsJob,
    seg: &HlsSegment,
    keys: &AsyncMutex<HashMap<String, [u8; 16]>>,
) -> Result<Vec<u8>> {
    let mut failures = 0;
    loop {
        match fetch_segment(job, seg, keys).await {
            Ok(data) => return Ok(data),
            Err(e @ (EngineError::Cancelled | EngineError::LinkDead(_))) => return Err(e),
            Err(e) => {
                failures += 1;
                if failures > job.max_retries {
                    return Err(e);
                }
                let backoff = e.retry_after().unwrap_or_else(|| Duration::from_millis(500 * 2u64.pow(failures.min(5))));
                tokio::select! {
                    _ = tokio::time::sleep(backoff) => {}
                    _ = job.cancel.cancelled() => return Err(EngineError::Cancelled),
                }
            }
        }
    }
}

async fn fetch_segment(
    job: &HlsJob,
    seg: &HlsSegment,
    keys: &AsyncMutex<HashMap<String, [u8; 16]>>,
) -> Result<Vec<u8>> {
    let mut req = job.client.get(&seg.uri).headers(job.headers.clone());
    if let Some((off, len)) = seg.range {
        req = req.header(RANGE, format!("bytes={off}-{}", off + len - 1));
    }
    let resp = tokio::select! {
        r = req.send() => r?,
        _ = job.cancel.cancelled() => return Err(EngineError::Cancelled),
    };
    check_status(resp.status(), resp.headers())?;
    if !resp.status().is_success() {
        return Err(EngineError::BadResponse(format!("HTTP {}", resp.status().as_u16())));
    }
    let mut data = Vec::with_capacity(resp.content_length().unwrap_or(0) as usize);
    let mut stream = resp.bytes_stream();
    loop {
        let chunk = tokio::select! {
            c = stream.next() => c,
            _ = job.cancel.cancelled() => return Err(EngineError::Cancelled),
        };
        let Some(chunk) = chunk else { break };
        let chunk = chunk?;
        for l in &job.limiters {
            l.acquire(chunk.len()).await;
        }
        job.control.limiter.acquire(chunk.len()).await;
        job.counter.bump(chunk.len() as u64);
        data.extend_from_slice(&chunk);
    }

    match &seg.key {
        None => Ok(data),
        Some(key) => {
            let key_bytes = {
                let mut cache = keys.lock().await;
                match cache.get(&key.uri) {
                    Some(k) => *k,
                    None => {
                        let resp = job.client.get(&key.uri).headers(job.headers.clone()).send().await?;
                        check_status(resp.status(), resp.headers())?;
                        let bytes = resp.bytes().await?;
                        let k: [u8; 16] = bytes
                            .as_ref()
                            .try_into()
                            .map_err(|_| EngineError::BadResponse("şifre anahtarı 16 bayt değil".into()))?;
                        cache.insert(key.uri.clone(), k);
                        k
                    }
                }
            };
            Aes128CbcDec::new(&key_bytes.into(), &key.iv.into())
                .decrypt_padded_vec_mut::<Pkcs7>(&data)
                .map_err(|_| EngineError::BadResponse("video parçası çözülemedi (yanlış anahtar)".into()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Url {
        Url::parse("https://cdn.example.com/video/index.m3u8").unwrap()
    }

    #[test]
    fn picks_best_variant() {
        let master = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360\nlow/index.m3u8\n\
                      #EXT-X-STREAM-INF:BANDWIDTH=5000000,CODECS=\"avc1.640028,mp4a.40.2\"\nhigh/index.m3u8\n";
        assert!(is_master(master));
        assert_eq!(best_variant(master, &base()).unwrap(), "https://cdn.example.com/video/high/index.m3u8");
    }

    #[test]
    fn parses_encrypted_media_playlist() {
        let text = "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:7\n#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\"\n#EXTINF:4,\nseg7.ts\n\
                    #EXT-X-KEY:METHOD=AES-128,URI=\"https://k.example.com/k2\",IV=0x000102030405060708090a0b0c0d0e0f\n#EXTINF:4,\nseg8.ts\n\
                    #EXT-X-KEY:METHOD=NONE\n#EXTINF:4,\nseg9.ts\n#EXT-X-ENDLIST\n";
        let p = parse_media(text, &base()).unwrap();
        assert_eq!(p.segments.len(), 3);
        let k0 = p.segments[0].key.as_ref().unwrap();
        assert_eq!(k0.uri, "https://cdn.example.com/video/key.bin");
        assert_eq!(k0.iv, seq_iv(7));
        assert_eq!(p.segments[1].key.as_ref().unwrap().iv[15], 0x0f);
        assert!(p.segments[2].key.is_none());
    }

    #[test]
    fn parses_byteranges_and_init_map() {
        let text = "#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXT-X-BYTERANGE:100@0\nall.mp4\n#EXT-X-BYTERANGE:50\nall.mp4\n#EXT-X-ENDLIST\n";
        let p = parse_media(text, &base()).unwrap();
        assert!(p.init.is_some());
        assert_eq!(p.segments[0].range, Some((0, 100)));
        assert_eq!(p.segments[1].range, Some((100, 50)));
    }

    #[test]
    fn rejects_drm_and_live() {
        let drm = "#EXTM3U\n#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://x\"\n#EXTINF:4,\na.ts\n#EXT-X-ENDLIST\n";
        assert!(parse_media(drm, &base()).unwrap_err().to_string().contains("DRM"));
        let live = "#EXTM3U\n#EXTINF:4,\na.ts\n";
        assert!(parse_media(live, &base()).unwrap_err().to_string().contains("canlı"));
    }

    #[test]
    fn detects_playlists() {
        assert!(looks_like_playlist("https://x/a/master.m3u8?token=1", None));
        assert!(looks_like_playlist("https://x/a/play", Some("application/vnd.apple.mpegurl")));
        assert!(!looks_like_playlist("https://x/a.mp4", Some("video/mp4")));
    }
}
