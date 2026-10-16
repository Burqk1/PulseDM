use std::fmt;
use std::time::Duration;

use crate::lang::english;

#[derive(Debug)]
pub enum EngineError {
    Http(String),
    ConnectFailed(String),
    Io(std::io::Error),
    BadResponse(String),
    Throttled { status: u16, retry_after: Option<Duration> },
    LinkDead(u16),
    WebPage,
    Changed,
    UrlMismatch(String),
    NoSpace { need: String, free: String },
    InvalidUrl(String),
    Busy,
    NotFound,
    Cancelled,
}

macro_rules! say {
    ($f:expr, $en:literal, $tr:literal) => {
        if english() { write!($f, $en) } else { write!($f, $tr) }
    };
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http(e) => say!(f, "Network error: {e}", "Ağ hatası: {e}"),
            Self::ConnectFailed(e) => say!(f, "Could not connect: {e}", "Bağlantı kurulamadı: {e}"),
            Self::Io(e) => say!(f, "Disk error: {e}", "Disk hatası: {e}"),
            Self::BadResponse(e) => say!(f, "Unexpected server response: {e}", "Sunucu beklenmeyen yanıt verdi: {e}"),
            Self::Throttled { status, .. } => say!(
                f,
                "The server says it is getting too many requests (HTTP {status})",
                "Sunucu çok fazla istek aldığını bildirdi (HTTP {status})"
            ),
            Self::LinkDead(status) => say!(
                f,
                "The server refused the file (HTTP {status}). The link may be invalid or expired; refresh the link and resume",
                "Sunucu dosyayı vermedi (HTTP {status}). Link geçersiz ya da süresi dolmuş olabilir; bağlantıyı yenileyip sürdürün"
            ),
            Self::WebPage => say!(
                f,
                "The link returned a web page instead of the file. It may have expired or need a sign-in; refresh the link and resume",
                "Bağlantı dosya yerine bir web sayfası döndürdü. Linkin süresi dolmuş ya da giriş gerekiyor olabilir; bağlantıyı yenileyip sürdürün"
            ),
            Self::Changed => say!(
                f,
                "The file changed on the server and has to be downloaded again",
                "Dosya sunucuda değişmiş, baştan indirilmesi gerekiyor"
            ),
            Self::UrlMismatch(e) => {
                say!(
                    f,
                    "The new link does not point to the same file: {e}",
                    "Yeni bağlantı aynı dosyayı göstermiyor: {e}"
                )
            }
            Self::NoSpace { need, free } => {
                say!(
                    f,
                    "Not enough disk space: {need} needed, {free} free",
                    "Diskte yeterli alan yok: {need} gerekli, {free} boş"
                )
            }
            Self::InvalidUrl(e) => say!(f, "Invalid link: {e}", "Geçersiz bağlantı: {e}"),
            Self::Busy => say!(f, "Pause the download first", "Bu işlem için önce indirmeyi duraklatın"),
            Self::NotFound => say!(f, "Download not found", "İndirme bulunamadı"),
            Self::Cancelled => say!(f, "Cancelled", "İptal edildi"),
        }
    }
}

impl std::error::Error for EngineError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for EngineError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl EngineError {
    pub fn is_refusal(&self) -> bool {
        matches!(self, Self::ConnectFailed(_) | Self::Throttled { .. } | Self::LinkDead(403 | 429))
    }

    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::Throttled { retry_after, .. } => *retry_after,
            _ => None,
        }
    }
}

impl From<reqwest::Error> for EngineError {
    fn from(e: reqwest::Error) -> Self {
        let mut causes = String::new();
        let mut source = std::error::Error::source(&e);
        while let Some(s) = source {
            causes.push_str(": ");
            causes.push_str(&s.to_string());
            source = s.source();
        }
        let refused = e.is_connect() || (e.is_timeout() && e.status().is_none());
        let msg = format!("{}{causes}", e.without_url());
        if refused { EngineError::ConnectFailed(msg) } else { EngineError::Http(msg) }
    }
}

pub type Result<T> = std::result::Result<T, EngineError>;
