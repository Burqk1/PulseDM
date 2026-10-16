use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::Command;

use base64::Engine as _;
use serde::Deserialize;
use serde_json::{Value, json};

const MAX_MESSAGE: u32 = 1024 * 1024;

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Message {
    Ping,
    Add(AddMessage),
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct AddMessage {
    url: String,
    filename: Option<String>,
    referrer: Option<String>,
    cookies: Option<String>,
    user_agent: Option<String>,
    headers: Vec<(String, String)>,
    media: bool,
}

fn main() {
    let mut stdin = io::stdin().lock();
    let mut stdout = io::stdout().lock();
    while let Some(msg) = read_message(&mut stdin) {
        let reply = match serde_json::from_slice::<Message>(&msg) {
            Ok(Message::Ping) => json!({ "ok": true, "version": env!("CARGO_PKG_VERSION") }),
            Ok(Message::Add(add)) => match hand_off(add) {
                Ok(()) => json!({ "ok": true }),
                Err(e) => json!({ "ok": false, "error": e }),
            },
            Err(e) => json!({ "ok": false, "error": format!("bad message: {e}") }),
        };
        if write_message(&mut stdout, &reply).is_err() {
            break;
        }
    }
}

fn read_message(input: &mut impl Read) -> Option<Vec<u8>> {
    let mut len = [0u8; 4];
    input.read_exact(&mut len).ok()?;
    let len = u32::from_le_bytes(len);
    if len > MAX_MESSAGE {
        return None;
    }
    let mut buf = vec![0u8; len as usize];
    input.read_exact(&mut buf).ok()?;
    Some(buf)
}

fn write_message(out: &mut impl Write, value: &Value) -> io::Result<()> {
    let bytes = serde_json::to_vec(value)?;
    out.write_all(&(bytes.len() as u32).to_le_bytes())?;
    out.write_all(&bytes)?;
    out.flush()
}

fn hand_off(add: AddMessage) -> Result<(), String> {
    if !(add.url.starts_with("http://") || add.url.starts_with("https://")) {
        return Err("only http(s) links are supported".into());
    }
    let mut headers = add.headers;
    let mut push = |k: &str, v: Option<String>| {
        if let Some(v) = v.filter(|v| !v.is_empty()) {
            headers.retain(|(name, _)| !name.eq_ignore_ascii_case(k));
            headers.push((k.to_string(), v));
        }
    };
    push("Cookie", add.cookies);
    push("Referer", add.referrer);
    push("User-Agent", add.user_agent);

    let mut request = json!({ "url": add.url, "file_name": add.filename, "headers": headers });
    if add.media {
        request["media_option"] = json!("ask");
    }
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(serde_json::to_vec(&request).unwrap());
    launch(&app_path()?, &payload)
}

fn app_path() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let app = exe.with_file_name(if cfg!(windows) { "pulsedm.exe" } else { "pulsedm" });
    if app.exists() { Ok(app) } else { Err(format!("{} bulunamadı", app.display())) }
}

fn launch(app: &PathBuf, payload: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        let spawned = Command::new(app)
            .args(["--add", payload])
            .creation_flags(DETACHED_PROCESS | CREATE_BREAKAWAY_FROM_JOB)
            .spawn()
            .or_else(|_| Command::new(app).args(["--add", payload]).creation_flags(DETACHED_PROCESS).spawn());
        spawned.map(|_| ()).map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        Command::new(app).args(["--add", payload]).spawn().map(|_| ()).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framing_round_trips() {
        let mut buf = Vec::new();
        write_message(&mut buf, &json!({"type": "ping"})).unwrap();
        let msg = read_message(&mut buf.as_slice()).unwrap();
        assert!(matches!(serde_json::from_slice::<Message>(&msg).unwrap(), Message::Ping));
    }

    #[test]
    fn parses_add_message() {
        let raw = br#"{"type":"add","url":"https://x/a.zip","cookies":"a=1","userAgent":"UA"}"#;
        let Message::Add(m) = serde_json::from_slice(raw).unwrap() else { panic!() };
        assert_eq!(m.url, "https://x/a.zip");
        assert_eq!(m.cookies.as_deref(), Some("a=1"));
        assert_eq!(m.user_agent.as_deref(), Some("UA"));
    }
}
