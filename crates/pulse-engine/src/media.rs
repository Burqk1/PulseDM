use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{EngineError, Result};
use crate::hls::HlsState;
use crate::segment::SegmentState;
use crate::tools::hidden;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct MediaOption {
    pub id: String,
    pub label: String,
    pub height: Option<u32>,
    pub audio_only: bool,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MediaInfo {
    pub url: String,
    pub title: String,
    pub thumbnail: Option<String>,
    pub duration: Option<f64>,
    pub uploader: Option<String>,
    pub site: Option<String>,
    pub options: Vec<MediaOption>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaState {
    pub page_url: String,
    pub option: String,
    pub streams: Vec<StreamState>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamState {
    pub format_id: String,
    pub part: String,
    pub size: Option<u64>,
    pub segments: Vec<SegmentState>,
    pub hls: Option<HlsState>,
    pub done: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Protocol {
    Http,
    Hls,
}

#[derive(Debug, Clone)]
pub(crate) struct Pick {
    pub format_id: String,
    pub url: String,
    pub protocol: Protocol,
    pub ext: String,
    pub size: Option<u64>,
    pub headers: Vec<(String, String)>,
    pub has_video: bool,
    pub chunk_size: Option<u64>,
}

#[derive(Debug, Clone)]
struct Fmt {
    id: String,
    url: String,
    protocol: Protocol,
    ext: String,
    vcodec: Option<String>,
    height: Option<u32>,
    has_video: bool,
    has_audio: bool,
    tbr: f64,
    abr: f64,
    size: Option<u64>,
    headers: Vec<(String, String)>,
    chunk_size: Option<u64>,
}

pub(crate) async fn extract(ytdlp: &Path, js_runtime: &str, url: &str, headers: &[(String, String)]) -> Result<Value> {
    let mut cmd = hidden(ytdlp);
    cmd.args(["-J", "--no-playlist", "--no-warnings", "--js-runtimes", js_runtime]);
    for (k, v) in headers {
        if k.eq_ignore_ascii_case("referer") || k.eq_ignore_ascii_case("user-agent") {
            cmd.args(["--add-header", &format!("{k}:{v}")]);
        }
    }
    let cookie_file =
        headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("cookie")).and_then(|(_, v)| write_cookie_file(url, v));
    if let Some(file) = &cookie_file {
        cmd.arg("--cookies").arg(file);
    }
    cmd.arg("--").arg(url);
    let out = cmd.output().await;
    if let Some(file) = &cookie_file {
        let _ = std::fs::remove_file(file);
    }
    let out = out?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let msg = stderr
            .lines()
            .rev()
            .find(|l| l.contains("ERROR"))
            .unwrap_or("bilinmeyen hata")
            .trim_start_matches("ERROR:")
            .trim();
        return Err(EngineError::BadResponse(format!("video çözülemedi: {msg}")));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| EngineError::BadResponse(format!("yt-dlp çıktısı okunamadı: {e}")))
}

fn write_cookie_file(url: &str, cookie_header: &str) -> Option<std::path::PathBuf> {
    let host = reqwest::Url::parse(url).ok()?.host_str()?.to_ascii_lowercase();
    let domain = format!(".{}", host.trim_start_matches("www."));
    let mut text = String::from(
        "# Netscape HTTP Cookie File
",
    );
    for pair in cookie_header.split(';') {
        if let Some((name, value)) = pair.trim().split_once('=') {
            text.push_str(&format!(
                "{domain}	TRUE	/	TRUE	0	{}	{}
",
                name.trim(),
                value.trim()
            ));
        }
    }
    let path = std::env::temp_dir().join(format!("pulse-cookies-{}.txt", uuid::Uuid::new_v4()));
    std::fs::write(&path, text).ok()?;
    Some(path)
}

pub(crate) fn info(page_url: &str, v: &Value) -> MediaInfo {
    let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
    MediaInfo {
        url: page_url.to_string(),
        title: s("title").unwrap_or_else(|| "video".into()),
        thumbnail: s("thumbnail"),
        duration: v.get("duration").and_then(Value::as_f64),
        uploader: s("uploader").or_else(|| s("channel")),
        site: s("extractor_key"),
        options: options(v),
    }
}

fn formats(v: &Value) -> Vec<Fmt> {
    let Some(list) = v.get("formats").and_then(Value::as_array) else {
        return single_format(v).into_iter().collect();
    };
    list.iter().filter_map(parse_format).collect()
}

fn single_format(v: &Value) -> Option<Fmt> {
    parse_format(v)
}

fn parse_format(f: &Value) -> Option<Fmt> {
    let s = |k: &str| f.get(k).and_then(Value::as_str);
    let protocol = match s("protocol").unwrap_or("https") {
        "https" | "http" => Protocol::Http,
        "m3u8_native" | "m3u8" => Protocol::Hls,
        _ => return None,
    };
    let vcodec = s("vcodec").map(str::to_owned);
    let acodec = s("acodec");
    let height = f.get("height").and_then(Value::as_u64).map(|h| h as u32);
    let has_video = match vcodec.as_deref() {
        Some("none") => false,
        Some(_) => true,
        None => height.is_some() || acodec.is_none(),
    };
    let has_audio = match acodec {
        Some("none") => false,
        Some(_) => true,
        None => has_video || vcodec.is_none(),
    };
    let headers = f
        .get("http_headers")
        .and_then(Value::as_object)
        .map(|m| m.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned()))).collect())
        .unwrap_or_default();
    Some(Fmt {
        id: s("format_id")?.to_owned(),
        url: s("url")?.to_owned(),
        protocol,
        ext: s("ext").unwrap_or("mp4").to_owned(),
        vcodec,
        height: if has_video { height } else { None },
        has_video,
        has_audio,
        tbr: f.get("tbr").and_then(Value::as_f64).unwrap_or(0.0),
        abr: f.get("abr").and_then(Value::as_f64).unwrap_or(0.0),
        size: f.get("filesize").and_then(Value::as_u64).or_else(|| f.get("filesize_approx").and_then(Value::as_u64)),
        chunk_size: f.pointer("/downloader_options/http_chunk_size").and_then(Value::as_u64),
        headers,
    })
}

pub(crate) fn options(v: &Value) -> Vec<MediaOption> {
    let fmts = formats(v);
    let mut heights: Vec<u32> =
        fmts.iter().filter(|f| f.has_video).filter_map(|f| f.height).filter(|&h| h >= 144).collect();
    heights.sort_unstable_by(|a, b| b.cmp(a));
    heights.dedup();

    let size_of = |id: &str| select_from(&fmts, id).ok().and_then(|p| p.iter().map(|p| p.size).sum::<Option<u64>>());
    let mut out = Vec::new();
    if fmts.iter().any(|f| f.has_video) {
        let label = match heights.first() {
            Some(h) => format!("En iyi kalite ({})", height_label(*h)),
            None => "En iyi kalite".into(),
        };
        out.push(MediaOption {
            id: "best".into(),
            label,
            height: heights.first().copied(),
            audio_only: false,
            size: size_of("best"),
        });
        for &h in &heights {
            let id = format!("h{h}");
            out.push(MediaOption {
                label: height_label(h),
                height: Some(h),
                audio_only: false,
                size: size_of(&id),
                id,
            });
        }
    }
    if fmts.iter().any(|f| f.has_audio) {
        out.push(MediaOption {
            id: "audio".into(),
            label: "Sadece ses (MP3)".into(),
            height: None,
            audio_only: true,
            size: size_of("audio"),
        });
    }
    out
}

fn height_label(h: u32) -> String {
    match h {
        4320.. => format!("{h}p (8K)"),
        2160.. => format!("{h}p (4K)"),
        1440.. => format!("{h}p (2K)"),
        720.. => format!("{h}p (HD)"),
        _ => format!("{h}p"),
    }
}

pub(crate) fn select(v: &Value, option: &str) -> Result<Vec<Pick>> {
    select_from(&formats(v), option)
}

fn select_from(fmts: &[Fmt], option: &str) -> Result<Vec<Pick>> {
    let none = || EngineError::BadResponse("bu kalite için indirilebilir akış bulunamadı".into());
    let audio_score = |f: &Fmt, prefer_m4a: bool| {
        let m4a = (prefer_m4a && (f.ext == "m4a" || f.ext == "mp4")) as u8;
        (m4a, (f.abr.max(f.tbr) * 1000.0) as u64)
    };

    if option == AUDIO_OPTION {
        let best = fmts
            .iter()
            .filter(|f| f.has_audio && !f.has_video)
            .max_by_key(|f| audio_score(f, false))
            .or_else(|| fmts.iter().filter(|f| f.has_audio).min_by_key(|f| f.height.unwrap_or(0)))
            .ok_or_else(none)?;
        return Ok(vec![pick(best)]);
    }

    let max_h = option.strip_prefix('h').and_then(|h| h.parse().ok()).unwrap_or(u32::MAX);
    let video = fmts
        .iter()
        .filter(|f| f.has_video && f.height.unwrap_or(0) <= max_h)
        .max_by_key(|f| {
            let h = f.height.unwrap_or(0);
            let compatible = (h <= 1080 && f.vcodec.as_deref().is_some_and(|c| c.starts_with("avc"))) as u8;
            let http = (f.protocol == Protocol::Http) as u8;
            (h, compatible, f.has_audio as u8, http, (f.tbr * 1000.0) as u64)
        })
        .ok_or_else(none)?;
    if video.has_audio {
        return Ok(vec![pick(video)]);
    }
    let mp4 = video.ext == "mp4";
    let audio = fmts.iter().filter(|f| f.has_audio && !f.has_video).max_by_key(|f| audio_score(f, mp4));
    Ok(match audio {
        Some(a) => vec![pick(video), pick(a)],
        None => vec![pick(video)],
    })
}

fn pick(f: &Fmt) -> Pick {
    Pick {
        format_id: f.id.clone(),
        url: f.url.clone(),
        protocol: f.protocol,
        ext: f.ext.clone(),
        size: f.size,
        headers: f.headers.clone(),
        has_video: f.has_video,
        chunk_size: f.chunk_size,
    }
}

pub(crate) const AUDIO_OPTION: &str = "audio";

pub(crate) fn output_ext(picks: &[Pick], audio_only: bool) -> String {
    if audio_only {
        return "mp3".into();
    }
    match picks {
        [single] if single.protocol == Protocol::Hls => "mp4".into(),
        [single] if !single.has_video && single.ext == "mp4" => "m4a".into(),
        [single] => single.ext.clone(),
        _ if picks.iter().all(|p| matches!(p.ext.as_str(), "mp4" | "m4a")) => "mp4".into(),
        _ => "mkv".into(),
    }
}

pub(crate) fn needs_ffmpeg(picks: &[Pick], audio_only: bool) -> bool {
    audio_only || picks.len() > 1 || picks.iter().any(|p| p.protocol == Protocol::Hls)
}

pub(crate) async fn mux(ffmpeg_dir: &Path, inputs: &[std::path::PathBuf], output: &Path, ext: &str) -> Result<()> {
    let mut cmd = hidden(&ffmpeg_dir.join("ffmpeg.exe"));
    cmd.args(["-y", "-hide_banner", "-loglevel", "error"]);
    for input in inputs {
        cmd.arg("-i").arg(input);
    }
    if ext == "mp3" {
        cmd.args(["-vn", "-map", "0:a:0", "-c:a", "libmp3lame", "-q:a", "2", "-f", "mp3"]).arg(output);
        let out = cmd.output().await?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            return Err(EngineError::BadResponse(format!(
                "ses MP3'e dönüştürülemedi: {}",
                err.lines().last().unwrap_or("ffmpeg hatası")
            )));
        }
        return Ok(());
    }
    if inputs.len() > 1 {
        cmd.args(["-map", "0:v:0", "-map", "1:a:0"]);
    }
    cmd.args(["-c", "copy"]);
    let format = match ext {
        "mp4" | "m4a" => {
            cmd.args(["-movflags", "+faststart"]);
            if inputs.len() == 1 {
                cmd.args(["-bsf:a", "aac_adtstoasc"]);
            }
            "mp4"
        }
        "webm" => "webm",
        _ => "matroska",
    };
    cmd.args(["-f", format]).arg(output);
    let out = cmd.output().await?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(EngineError::BadResponse(format!(
            "video birleştirilemedi: {}",
            err.lines().last().unwrap_or("ffmpeg hatası")
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn youtube_like() -> Value {
        json!({
            "title": "Test video", "extractor_key": "Youtube",
            "formats": [
                {"format_id": "sb0", "protocol": "mhtml", "url": "u", "vcodec": "none", "acodec": "none", "height": 45},
                {"format_id": "18", "protocol": "https", "url": "u18", "ext": "mp4", "vcodec": "avc1.42", "acodec": "mp4a.40.2", "height": 360, "tbr": 500.0, "filesize": 1000},
                {"format_id": "137", "protocol": "https", "url": "u137", "ext": "mp4", "vcodec": "avc1.640028", "acodec": "none", "height": 1080, "tbr": 4000.0, "filesize": 50000},
                {"format_id": "248", "protocol": "https", "url": "u248", "ext": "webm", "vcodec": "vp9", "acodec": "none", "height": 1080, "tbr": 3000.0, "filesize": 40000},
                {"format_id": "313", "protocol": "https", "url": "u313", "ext": "webm", "vcodec": "vp9", "acodec": "none", "height": 2160, "tbr": 16000.0, "filesize": 300000},
                {"format_id": "140", "protocol": "https", "url": "u140", "ext": "m4a", "vcodec": "none", "acodec": "mp4a.40.2", "abr": 128.0, "filesize": 4000},
                {"format_id": "251", "protocol": "https", "url": "u251", "ext": "webm", "vcodec": "none", "acodec": "opus", "abr": 160.0, "filesize": 5000}
            ]
        })
    }

    #[test]
    fn offers_qualities_from_best_down() {
        let opts = options(&youtube_like());
        let ids: Vec<&str> = opts.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(ids, ["best", "h2160", "h1080", "h360", "audio"]);
        assert_eq!(opts[0].label, "En iyi kalite (2160p (4K))");
        assert_eq!(opts[2].size, Some(54000), "1080p H.264 + m4a audio");
    }

    #[test]
    fn prefers_h264_mp4_up_to_1080p() {
        let p = select(&youtube_like(), "h1080").unwrap();
        assert_eq!(p.iter().map(|p| p.format_id.as_str()).collect::<Vec<_>>(), ["137", "140"]);
        assert_eq!(output_ext(&p, false), "mp4");
        assert!(needs_ffmpeg(&p, false));
    }

    #[test]
    fn best_takes_4k_vp9_into_mkv() {
        let p = select(&youtube_like(), "best").unwrap();
        assert_eq!(p[0].format_id, "313");
        assert_eq!(p[1].format_id, "251", "opus pairs with a webm video");
        assert_eq!(output_ext(&p, false), "mkv");
    }

    #[test]
    fn audio_only_takes_best_audio_and_makes_mp3() {
        let p = select(&youtube_like(), "audio").unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].format_id, "251", "160 kbps opus beats 128 kbps m4a before encoding");
        assert_eq!(output_ext(&p, true), "mp3");
        assert!(needs_ffmpeg(&p, true));
    }

    #[test]
    fn writes_netscape_cookie_file() {
        let path = write_cookie_file("https://www.youtube.com/watch?v=x", "SID=abc; PREF=f6=40000000").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert!(text.starts_with("# Netscape HTTP Cookie File"));
        assert!(text.contains(".youtube.com	TRUE	/	TRUE	0	SID	abc"));
        assert!(text.contains("PREF	f6=40000000"), "values may contain '='");
    }

    #[test]
    fn audio_only_without_audio_stream_uses_smallest_video() {
        let v = json!({"title": "t", "formats": [
            {"format_id": "http-320", "protocol": "https", "url": "a", "ext": "mp4", "height": 320, "tbr": 600.0},
            {"format_id": "http-720", "protocol": "https", "url": "b", "ext": "mp4", "height": 720, "tbr": 2000.0}
        ]});
        let p = select(&v, "audio").unwrap();
        assert_eq!(p[0].format_id, "http-320");
        assert_eq!(output_ext(&p, true), "mp3");
    }

    #[test]
    fn progressive_and_hls_sites() {
        let v = json!({"title": "t", "formats": [
            {"format_id": "hls-720", "protocol": "m3u8_native", "url": "h", "ext": "mp4", "height": 720, "tbr": 2000.0},
            {"format_id": "http-720", "protocol": "https", "url": "m", "ext": "mp4", "height": 720, "tbr": 2000.0}
        ]});
        let p = select(&v, "best").unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].format_id, "http-720", "plain HTTP beats HLS at equal quality");
        assert_eq!(output_ext(&p, false), "mp4");
        assert!(!needs_ffmpeg(&p, false));
    }
}
