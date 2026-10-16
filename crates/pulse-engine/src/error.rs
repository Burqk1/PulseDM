use std::time::Duration;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("Ağ hatası: {0}")]
    Http(String),
    #[error("Bağlantı kurulamadı: {0}")]
    ConnectFailed(String),
    #[error("Disk hatası: {0}")]
    Io(#[from] std::io::Error),
    #[error("Sunucu beklenmeyen yanıt verdi: {0}")]
    BadResponse(String),
    #[error("Sunucu çok fazla istek aldığını bildirdi (HTTP {status})")]
    Throttled { status: u16, retry_after: Option<Duration> },
    #[error(
        "Sunucu dosyayı vermedi (HTTP {0}). Link geçersiz ya da süresi dolmuş olabilir; bağlantıyı yenileyip sürdürün"
    )]
    LinkDead(u16),
    #[error(
        "Bağlantı dosya yerine bir web sayfası döndürdü. Linkin süresi dolmuş ya da giriş gerekiyor olabilir; bağlantıyı yenileyip sürdürün"
    )]
    WebPage,
    #[error("Dosya sunucuda değişmiş, baştan indirilmesi gerekiyor")]
    Changed,
    #[error("Yeni bağlantı aynı dosyayı göstermiyor: {0}")]
    UrlMismatch(String),
    #[error("Diskte yeterli alan yok: {need} gerekli, {free} boş")]
    NoSpace { need: String, free: String },
    #[error("Geçersiz bağlantı: {0}")]
    InvalidUrl(String),
    #[error("Bu işlem için önce indirmeyi duraklatın")]
    Busy,
    #[error("İndirme bulunamadı")]
    NotFound,
    #[error("İptal edildi")]
    Cancelled,
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
