pub fn category_of(file_name: &str) -> &'static str {
    let ext = file_name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "mp4" | "mkv" | "avi" | "mov" | "webm" | "flv" | "wmv" | "m4v" | "ts" | "m3u8" => "video",
        "mp3" | "flac" | "wav" | "aac" | "ogg" | "m4a" | "opus" | "wma" => "audio",
        "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" | "zst" | "iso" => "archive",
        "exe" | "msi" | "msix" | "appx" | "dmg" | "pkg" | "deb" | "rpm" | "apk" | "appimage" => "program",
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "txt" | "epub" | "csv" | "odt" => "document",
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "svg" | "bmp" | "tiff" | "heic" | "avif" => "image",
        _ => "other",
    }
}
