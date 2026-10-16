use std::path::Path;

pub const HOST_NAME: &str = "com.novament.pulsedm";

const EXTENSION_IDS: &[&str] = &["ndbpcgcclegmdfdoliknajfcfiblbdlk"];

const FIREFOX_ID: &str = "pulsedm@burqk1";

const BROWSER_KEYS: &[&str] = &[
    r"Software\Google\Chrome\NativeMessagingHosts",
    r"Software\Microsoft\Edge\NativeMessagingHosts",
    r"Software\BraveSoftware\Brave-Browser\NativeMessagingHosts",
    r"Software\Chromium\NativeMessagingHosts",
];

pub fn register(data_dir: &Path) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let host = exe.with_file_name(if cfg!(windows) { "pulsedm-host.exe" } else { "pulsedm-host" });
    if !host.exists() {
        return Err(format!("{} {}", crate::l("not found:", "bulunamadı:"), host.display()));
    }

    let manifest = serde_json::json!({
        "name": HOST_NAME,
        "description": "Pulse DM download manager",
        "path": host,
        "type": "stdio",
        "allowed_origins": EXTENSION_IDS.iter().map(|id| format!("chrome-extension://{id}/")).collect::<Vec<_>>(),
    });
    let manifest_path = data_dir.join(format!("{HOST_NAME}.json"));
    std::fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest).unwrap()).map_err(|e| e.to_string())?;

    let firefox = serde_json::json!({
        "name": HOST_NAME,
        "description": "Pulse DM download manager",
        "path": host,
        "type": "stdio",
        "allowed_extensions": [FIREFOX_ID],
    });
    let firefox_path = data_dir.join(format!("{HOST_NAME}.firefox.json"));
    std::fs::write(&firefox_path, serde_json::to_vec_pretty(&firefox).unwrap()).map_err(|e| e.to_string())?;

    #[cfg(windows)]
    {
        use winreg::RegKey;
        use winreg::enums::HKEY_CURRENT_USER;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        for base in BROWSER_KEYS {
            let (key, _) = hkcu.create_subkey(format!(r"{base}\{HOST_NAME}")).map_err(|e| e.to_string())?;
            key.set_value("", &manifest_path.to_string_lossy().to_string()).map_err(|e| e.to_string())?;
        }
        let (key, _) = hkcu
            .create_subkey(format!(r"Software\Mozilla\NativeMessagingHosts\{HOST_NAME}"))
            .map_err(|e| e.to_string())?;
        key.set_value("", &firefox_path.to_string_lossy().to_string()).map_err(|e| e.to_string())?;
    }
    Ok(())
}
