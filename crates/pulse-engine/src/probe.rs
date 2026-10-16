use percent_encoding::percent_decode_str;
use reqwest::header::{CONTENT_DISPOSITION, CONTENT_RANGE, CONTENT_TYPE, ETAG, HeaderMap, LAST_MODIFIED, RANGE};
use reqwest::{Client, StatusCode, Url};

use crate::download::check_status;
use crate::error::{EngineError, Result};
use crate::fsutil::sanitize_file_name;

#[derive(Debug, Clone)]
pub struct ProbeInfo {
    pub final_url: String,
    pub size: Option<u64>,
    pub resumable: bool,
    pub file_name: String,
    pub validator: Option<String>,
    pub html: bool,
    pub last_modified: Option<String>,
    pub content_type: Option<String>,
}

pub async fn probe(client: &Client, url: &str, headers: &HeaderMap) -> Result<ProbeInfo> {
    let resp = client.get(url).headers(headers.clone()).header(RANGE, "bytes=0-0").send().await?;
    let status = resp.status();
    check_status(status, resp.headers())?;
    if !status.is_success() {
        return Err(EngineError::BadResponse(format!("HTTP {}", status.as_u16())));
    }
    let h = resp.headers();

    let (size, resumable) = if status == StatusCode::PARTIAL_CONTENT {
        let total = h.get(CONTENT_RANGE).and_then(|v| v.to_str().ok()).and_then(parse_content_range_total);
        (total, total.is_some())
    } else {
        (resp.content_length(), false)
    };

    let etag = h.get(ETAG).and_then(|v| v.to_str().ok()).filter(|e| !e.starts_with("W/")).map(str::to_owned);
    let validator = etag.or_else(|| h.get(LAST_MODIFIED).and_then(|v| v.to_str().ok()).map(str::to_owned));

    let final_url = resp.url().clone();
    let file_name = h
        .get(CONTENT_DISPOSITION)
        .and_then(|v| v.to_str().ok())
        .and_then(parse_content_disposition)
        .or_else(|| name_from_url(&final_url))
        .map(|n| sanitize_file_name(&n))
        .unwrap_or_else(|| "download".into());

    Ok(ProbeInfo {
        final_url: final_url.to_string(),
        size,
        resumable,
        file_name,
        validator,
        html: is_html(h),
        last_modified: h.get(LAST_MODIFIED).and_then(|v| v.to_str().ok()).map(str::to_owned),
        content_type: h.get(CONTENT_TYPE).and_then(|v| v.to_str().ok()).map(str::to_owned),
    })
}

pub fn is_html(headers: &HeaderMap) -> bool {
    headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|ct| {
            let ct = ct.trim().to_ascii_lowercase();
            ct.starts_with("text/html") || ct.starts_with("application/xhtml")
        })
        .unwrap_or(false)
}

pub fn expects_html(file_name: &str) -> bool {
    match file_name.rsplit_once('.') {
        Some((_, ext)) => matches!(
            ext.to_ascii_lowercase().as_str(),
            "html" | "htm" | "xhtml" | "php" | "asp" | "aspx" | "jsp" | "cgi"
        ),
        None => true,
    }
}

pub fn parse_content_range_total(v: &str) -> Option<u64> {
    v.rsplit('/').next()?.trim().parse().ok()
}

pub fn parse_content_range_start(v: &str) -> Option<u64> {
    v.trim().strip_prefix("bytes")?.trim().split('-').next()?.trim().parse().ok()
}

pub fn parse_content_disposition(v: &str) -> Option<String> {
    let mut plain = None;
    for part in v.split(';').map(str::trim) {
        let lower = part.to_ascii_lowercase();
        if lower.starts_with("filename*=") {
            let raw = part["filename*=".len()..].trim_matches('"');
            let value = raw.splitn(3, '\'').nth(2).unwrap_or(raw);
            let decoded = percent_decode_str(value).decode_utf8_lossy().into_owned();
            if !decoded.is_empty() {
                return Some(decoded);
            }
        } else if lower.starts_with("filename=") {
            plain = Some(part["filename=".len()..].trim().trim_matches('"').to_string());
        }
    }
    plain.filter(|s| !s.is_empty())
}

pub fn name_from_url(url: &Url) -> Option<String> {
    let last = url.path_segments()?.rfind(|s| !s.is_empty())?;
    Some(percent_decode_str(last).decode_utf8_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_content_range() {
        assert_eq!(parse_content_range_total("bytes 0-0/12345"), Some(12345));
        assert_eq!(parse_content_range_total("bytes 0-0/*"), None);
        assert_eq!(parse_content_range_start("bytes 100-199/1000"), Some(100));
    }

    #[test]
    fn parses_content_disposition() {
        assert_eq!(parse_content_disposition(r#"attachment; filename="a b.zip""#).as_deref(), Some("a b.zip"));
        assert_eq!(parse_content_disposition("attachment; filename=plain.iso").as_deref(), Some("plain.iso"));
        assert_eq!(
            parse_content_disposition(r#"attachment; filename="fallback.txt"; filename*=UTF-8''%C3%B6zel%20dosya.txt"#)
                .as_deref(),
            Some("özel dosya.txt")
        );
        assert_eq!(parse_content_disposition("inline"), None);
    }

    #[test]
    fn name_from_url_decodes_last_segment() {
        let u = Url::parse("https://x.com/files/my%20file.zip?token=1").unwrap();
        assert_eq!(name_from_url(&u).as_deref(), Some("my file.zip"));
        assert_eq!(name_from_url(&Url::parse("https://x.com/").unwrap()), None);
    }

    #[test]
    fn html_expectation_follows_extension() {
        assert!(expects_html("index.html"));
        assert!(expects_html("download"));
        assert!(!expects_html("setup.exe"));
    }
}
