<div align="center">

<img src="docs/images/logo.png" width="96" alt="Pulse DM logo" />

# Pulse DM

**A fast, free and open-source download manager for Windows.**
Multi-connection downloads that resume, BitTorrent and magnet links, extensions for Chrome, Edge and Firefox, video sites (YouTube & 1000+ more), queues and a scheduler, in a native app smaller than 5 MB.

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
![Platform](https://img.shields.io/badge/platform-Windows%2010%2F11-0078D6)
![Built with Rust](https://img.shields.io/badge/engine-Rust-orange)
![Tauri](https://img.shields.io/badge/UI-Tauri%202%20%2B%20Svelte%205-24C8DB)
![Status](https://img.shields.io/badge/status-early%20development-yellow)

<img src="docs/images/main-window.png" width="820" alt="Pulse DM main window" />

</div>

> 🇹🇷 **Türkçe:** Pulse DM, Windows için hızlı, ücretsiz ve açık kaynaklı bir indirme yöneticisidir. Dosyaları aynı anda 8–16 bağlantıyla indirir, kopan indirmeleri kaldığı yerden sürdürür, Chrome, Edge ve Firefox eklentileriyle indirmeleri otomatik yakalar, YouTube dahil 1000'den fazla video sitesinden indirebilir ve torrent/magnet linklerini de indirir. Arayüz Türkçe ve İngilizcedir; motor Rust ile yazılmıştır ve kurulum dosyası 5 MB'tan küçüktür. Kendini otomatik günceller. Geliştirme aşamasındadır.

---

## Contents

- [Highlights](#highlights)
- [Benchmarks](#benchmarks)
- [Features](#features)
- [How it works](#how-it-works)
- [Installation](#installation)
- [Browser extension](#browser-extension)
- [Building from source](#building-from-source)
- [Releasing a new version](#releasing-a-new-version)
- [Project structure](#project-structure)
- [Where Pulse DM keeps its files](#where-pulse-dm-keeps-its-files)
- [Roadmap](#roadmap)
- [Responsible use](#responsible-use)
- [Contributing](#contributing)
- [License and credits](#license-and-credits)

## Highlights

- **Fast:** every file is split across many connections, and idle connections take over half of the slowest one's remaining range, so all of them finish together.
- **Adapts to the server by itself:** starts with 8 connections and adds more (up to 16) only while it measurably speeds things up. When a server refuses extra connections (HTTP 503/429), Pulse quietly drops back instead of failing or getting your IP blocked.
- **Light:** about **3 MB of RAM** while waiting in the tray, and a **4.5 MB** installer.
- **Never loses progress:** pause, close the app, reboot or lose the network, and the download continues from where it stopped. Expired links can be replaced and the download keeps its progress.
- **Video sites:** YouTube, X/Twitter, Instagram, TikTok, Vimeo, Twitch, Reddit and 1000+ more, in any quality up to 8K, or audio only as MP3.
- **BitTorrent built in:** magnet links, `.torrent` files and `.torrent` links download in the same list, with the same queues, pause and resume.
- **Works like IDM in the browser:** in Chrome, Edge, Brave and Firefox, a "Download with Pulse DM" button appears on videos, and regular downloads are captured with your cookies so files behind a login work too.

## Benchmarks

Same 1 GB test file ([Tele2 speed test server](http://speedtest.tele2.net/)), same machine and network, runs interleaved (Pulse → AB → Pulse → AB), October 2026. RAM is the **private memory of the whole process tree** (working-set totals double-count memory that WebView2 processes share).

| | Avg. time | Avg. speed | Peak RAM while downloading | RAM idle in tray |
|---|---:|---:|---:|---:|
| **Pulse DM 0.1** | **29.0 s** | **35.4 MB/s** | **186 MB** | **3 MB** |
| AB Download Manager 1.11.1 | 40.9 s | 25.6 MB/s | 473 MB | 400 MB |

YouTube, 720p video (153 MB), video and audio merged into MP4: **5.7 s (≈57 MB/s)**. The same stream downloaded as one plain request is throttled by YouTube to about 3.6 MB/s.

> **Take benchmarks with a grain of salt.** This test server limits each connection to about 2.7 MB/s, which is the situation where a download manager helps most; the gap is smaller on servers without such a limit. Results are from two runs on one network, and speeds varied between runs. Measure on your own connection (the `bench` example below times the engine alone), and open an issue if you see different results.

## Features

### Download engine

- Segmented HTTP(S) downloads, with each segment on its own TCP connection (HTTP/1.1 on purpose: HTTP/2 would squeeze every segment into one connection).
- Dynamic work stealing between connections, so the last few percent never crawl on a single connection.
- **Automatic connection count**: 8 → 12 → 16 while each step adds at least 8 % speed; it freezes as soon as the server pushes back.
- Writes straight into the final file (sparse on NTFS), so there is no slow "assembling file" step at the end.
- Resume after pause, crash or reboot; every range request carries `If-Range`, so a file that changed on the server is detected and restarted instead of silently corrupted.
- Expired-link detection (the server answers with an HTML page or 401/403/404/410) and **link refresh**: paste a new link and the download continues from where it stopped.
- Speed limits: global, per download and per website, adjustable while downloading.
- Free disk space check, retries with exponential backoff, `Retry-After` support.
- Proxy (system, none, manual HTTP/SOCKS5), HTTP basic authentication, custom User-Agent and per-site rules.
- Checksum verification (MD5, SHA-1, SHA-256, SHA-512), with the algorithm detected from the hash you paste.

### Video sites

- Resolves pages with [yt-dlp](https://github.com/yt-dlp/yt-dlp), then downloads the streams with Pulse's own multi-connection engine, and merges video and audio with ffmpeg **without re-encoding**.
- Quality picker from 144p to 8K, plus **audio only as MP3** (LAME VBR, about 190 kbps).
- HLS (`.m3u8`) streams, including **AES-128 encrypted** ones; live streams and DRM-protected (SAMPLE-AES) videos are refused.
- yt-dlp, ffmpeg and a JavaScript runtime are downloaded on first use only (about 150 MB in total, about 107 MB if Node.js is already installed), and yt-dlp keeps itself up to date.
- Cookies from the browser extension are passed to yt-dlp, which helps with sites that otherwise ask you to sign in ("confirm you're not a bot") while you are signed in in your browser.

### BitTorrent and magnet links

- Built on [librqbit](https://github.com/ikatson/rqbit), a BitTorrent engine written in Rust, so there is no extra program or service to install.
- Add a **magnet link**, a **`.torrent` file** (button in the add window, double-click in Explorer, or drag the path in) or a **`.torrent` link**. Peers are found through DHT and trackers, and UPnP opens a port on your router for incoming peers.
- Torrents share the download list, queues, speed limits and pause/resume with everything else. A resumed torrent checks the pieces already on disk and continues from there.
- Multi-file torrents go into their own folder, and a single-file torrent never overwrites a file you already have.
- Optional: make Pulse DM the handler for magnet links (*Settings → Downloads*). This is off by default, so Pulse DM does not take magnet links away from another torrent program unless you ask it to.
- Tested on the official Debian image torrent: 35–41 MB/s with up to 48 peers from a `.torrent` link, and metadata from a magnet link in about 4 seconds.
- Pulse DM stops a torrent when it is complete; it does not keep seeding yet.

### Organizing

- Default folder `Downloads\Pulse DM` with sub-folders per category (Archives, Programs, Videos, Music, Pictures, Documents), named in your Windows language.
- Queues with their own concurrency limit, and a **scheduler** that starts or stops a queue at set times on chosen weekdays.
- Shut down or sleep the PC when all downloads finish (with a 60-second cancel window).
- Windows notifications, optionally a "download complete" dialog, and an option to keep the server's file date.
- **Automatic updates**: checks GitHub Releases at start-up and every 12 hours, then updates with one click. Every update is signed and verified before it is installed.

### Interface

- IDM / AB DM-style layout: categories sidebar, sortable columns, a separate compact window for adding a download and one for each download's progress.
- Starts with Windows straight into the tray; downloads from the browser show only a small confirmation window, never the main window.
- Dark, light or system theme, UI scaling, compact layout.
- English and Turkish interface. It follows the Windows display language by default and can be switched in *Settings → Appearance*. The browser extension follows the browser's language.

<p align="center">
  <img src="docs/images/add-download.png" width="400" alt="Add download window" />
  <img src="docs/images/video-dialog.png" width="400" alt="Video quality picker" />
</p>

## How it works

```mermaid
flowchart LR
    subgraph Browser
        EXT[Pulse DM extension<br/>Chromium MV3 · Firefox MV2]
    end
    subgraph Windows
        HOST[pulsedm-host.exe<br/>native messaging]
        APP[Pulse DM app<br/>Tauri + Svelte UI]
        ENG[(pulse-engine<br/>Rust, Tokio)]
        TOOLS[yt-dlp · ffmpeg<br/>fetched on first use]
        BT[librqbit<br/>BitTorrent]
    end
    EXT -- "link + cookies" --> HOST
    HOST -- "pulsedm.exe --add …" --> APP
    APP <--> ENG
    ENG -- "resolve video pages" --> TOOLS
    ENG == "8–16 HTTP/1.1 connections" ==> NET((Internet))
    ENG --> BT
    BT == "peers: DHT, trackers" ==> NET
```

- **`pulse-engine`** is a standalone Rust library with no UI: it handles segments, work stealing, the connection ramp, resume state, HLS, video sites, torrents and queues. It is tested on its own against a local test server: 49 tests cover byte-exact downloads, pause/resume, restarts, connection-limited servers, expired links, HLS decryption, queues, authentication, speed limits and torrent sources.
- **The app** (`src-tauri`) adds the tray, windows, notifications, autostart and the browser bridge. When its windows are closed, only the engine stays in memory.
- **The extension** talks to the app through the browser's native messaging (no local web server, so websites cannot reach Pulse DM). The Chromium and Firefox versions are built from the same source.

## Installation

Download `Pulse DM_x.y.z_x64-setup.exe` from the [latest release](../../releases/latest) and run it. No administrator rights are needed: it installs for the current user. After that, Pulse DM updates itself.

The installer is **not code-signed yet**, so Windows SmartScreen may warn about an unrecognized app. Choose *More info → Run anyway*. When an older version is installed, choose to install **without uninstalling first**: your settings and download list are kept either way.

Requirements: Windows 10 or 11 (x64). WebView2 is already part of Windows 11 and recent Windows 10 versions, and the installer adds it when missing.

## Browser extension

Chrome, Edge, Brave, other Chromium browsers and Firefox are supported.

**Chrome, Edge, Brave** (until the extension is on the Chrome Web Store and Edge Add-ons):

1. Open `chrome://extensions` (or `edge://extensions`).
2. Turn on **Developer mode**.
3. Click **Load unpacked** and pick the `extension` folder. Pulse DM can open this folder for you: *Settings → Browser → Open the extension folder*.

**Firefox** (142 or newer): open [`pulse-dm-firefox.xpi`](../../releases/latest/download/pulse-dm-firefox.xpi) from the latest release **in Firefox** and press *Add*. Pulse DM has a shortcut for this too: *Settings → Browser → Get the Firefox extension*.

The popup should say **Connected**. The extension:

- hands regular downloads (zip, exe, iso, pdf, torrent, video files and more; this is configurable) to Pulse DM with your cookies and referrer;
- hands **magnet links** to Pulse DM when you click them (can be turned off in the popup);
- shows a **Download with Pulse DM** button over videos on any page, including TikTok and Reddit feeds;
- adds **Download with Pulse DM** to the right-click menu for links and media;
- leaves the download to the browser if Pulse DM cannot be reached, so nothing is lost.

In Firefox, downloads are taken over as soon as the server's response arrives, before Firefox opens its own download, so no dialog or partial file is left behind. This is the same approach AB Download Manager, Gopeed and XDM use.

## Building from source

### Prerequisites

- [Rust](https://rustup.rs/) (stable, MSVC toolchain)
- [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) with the *Desktop development with C++* workload
- [Node.js](https://nodejs.org/) 20 or newer

### Commands

```powershell
git clone https://github.com/Burqk1/PulseDM.git
cd PulseDM
npm install

# Run in development mode (hot reload for the UI)
npm run tauri dev

# Engine tests (local test server, no internet needed)
cargo test -p pulse-engine

# Installer → target\release\bundle\nsis\
npm run tauri build
```

`npm run tauri dev` and `npm run tauri build` first build the browser bridge (`pulsedm-host.exe`) through `scripts/build-host.mjs`.

For everyday testing use `npm run tauri dev`: it rebuilds only what changed. `tauri build` is the full optimized release build (whole-program LTO over almost 700 crates) and takes several minutes; GitHub Actions does it for every release anyway. Keep the repository outside OneDrive or Dropbox: the build cache in `target/` grows to tens of gigabytes.

The Firefox extension is generated from the same `extension/` folder:

```powershell
node scripts/build-firefox.mjs                     # → target\firefox-extension\
npx web-ext lint -s target/firefox-extension       # Mozilla's validator
npx web-ext run -s target/firefox-extension        # try it in a temporary Firefox profile
```

> While `tauri dev` runs, the browser bridge points at the development build. Start the installed Pulse DM once afterwards to point it back.

Handy examples:

```powershell
# Engine-only speed test: URL, connections, output folder
cargo run --release -p pulse-engine --example bench -- http://speedtest.tele2.net/100MB.zip 8 .\out

# Video site end-to-end: page URL, quality (best | h1080 | audio …), output folder
cargo run --release -p pulse-engine --example media -- "<video page url>" h720 .\out

# Torrent end-to-end: magnet / .torrent link or path, stop after N MB (0 = finish), output folder
cargo run --release -p pulse-engine --example torrent -- "<magnet or .torrent>" 20 .\out
```

## Releasing a new version

Releases are built by GitHub Actions ([`.github/workflows/release.yml`](.github/workflows/release.yml)). Installed copies of Pulse DM pick them up through the updater.

**One-time setup:** under *Settings → Secrets and variables → Actions*, add these repository secrets:

| Secret | Value |
|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | contents of `keys/updater.key` |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | contents of `keys/updater.password` |
| `AMO_JWT_ISSUER` | *JWT issuer* from [addons.mozilla.org API keys](https://addons.mozilla.org/developers/addon/api/key/) |
| `AMO_JWT_SECRET` | *JWT secret* from the same page |

The matching public key is in `src-tauri/tauri.conf.json`. Keep the private key safe and out of git: if you lose it, existing installs can no longer verify (and so cannot install) new updates. The two AMO keys let the workflow get the Firefox extension signed by Mozilla; without them the `.xpi` is still built but unsigned, and Firefox only loads it temporarily.

The repository must be **public** for the updater: installed copies download `latest.json` and the installer from GitHub Releases without signing in.

**Each release:**

```powershell
npm run release -- 0.3.0   # bumps all version numbers, commits, tags v0.3.0
git push --follow-tags     # GitHub builds the installer and publishes the release
```

The workflow runs the tests, builds the NSIS installer, signs the update bundle, uploads `latest.json`, and builds (and, with the AMO keys, signs) `pulse-dm-firefox.xpi`. Running copies see the new version within 12 hours, or straight away from *Help → Check for updates*.

## Project structure

```
PulseDM/
├── crates/
│   ├── pulse-engine/        # Download engine (Rust library)
│   │   ├── src/
│   │   │   ├── engine.rs    # Downloads, queues, scheduler, persistence
│   │   │   ├── download.rs  # Workers, connection ramp, range requests
│   │   │   ├── segment.rs   # Segment table and work stealing
│   │   │   ├── hls.rs       # m3u8 parsing, AES-128, in-order append
│   │   │   ├── media.rs     # yt-dlp formats, quality choice, ffmpeg mux / MP3
│   │   │   ├── tools.rs     # On-demand yt-dlp / ffmpeg / Deno
│   │   │   ├── torrent.rs   # Magnet / .torrent sources, librqbit session
│   │   │   └── …            # probe, limiter, schedule, fsutil, errors
│   │   ├── tests/engine.rs  # Integration tests with a local HTTP server
│   │   └── examples/        # bench, media, torrent
│   └── pulse-host/          # Native messaging host (browser → app bridge)
├── src-tauri/               # Desktop app: tray, windows, notifications, autostart
├── src/                     # Svelte UI
│   ├── routes/+page.svelte  # Main window
│   ├── routes/add/          # "Add download" window
│   ├── routes/progress/     # Progress / download-complete window
│   └── lib/                 # API bindings, icons, preferences, translations
├── extension/               # Browser extension (Chromium MV3 source, Firefox build from it)
├── scripts/build-host.mjs   # Builds the bridge for the installer
├── scripts/build-firefox.mjs # Generates the Firefox (MV2) extension
├── scripts/release.mjs      # Bumps the version and tags a release
├── .github/workflows/       # Builds and publishes releases from tags
└── docs/images/             # Screenshots
```

## Where Pulse DM keeps its files

| What | Location |
|---|---|
| Downloads | `%USERPROFILE%\Downloads\Pulse DM\<category>` (configurable) |
| Unfinished downloads | `<file name>.pdm` next to the final file, renamed when complete |
| Settings, list, preferences | `%APPDATA%\com.novament.pulsedm\` (`settings.json`, `downloads.json`, `prefs.json`) |
| yt-dlp, ffmpeg, Deno | `%APPDATA%\com.novament.pulsedm\tools\` |
| Torrent metadata, DHT state | `%APPDATA%\com.novament.pulsedm\torrents\` |
| App | `%LOCALAPPDATA%\Pulse DM\` |

## Roadmap

- [x] Multi-connection engine with resume, work stealing and an automatic connection count
- [x] Queues, scheduler, speed limits, proxy, per-site rules
- [x] Browser extension for Chromium browsers, with on-video download buttons
- [x] Video sites through yt-dlp, HLS with AES-128, MP3 audio
- [x] Installer, autostart in the tray
- [x] Automatic updates (signed, from GitHub Releases)
- [x] Firefox extension
- [x] BitTorrent and magnet links
- [x] English and Turkish interface
- [ ] macOS and Linux builds (in progress)
- [ ] Code-signed installer
- [ ] Chrome Web Store / Edge Add-ons / Firefox Add-ons listings (one-click extension install)
- [ ] Seeding options for torrents (ratio, time limit)
- [ ] More languages

## Responsible use

Pulse DM is a general-purpose download tool. Downloading from video platforms may be against their terms of service, and much of their content is copyrighted. Only download what you have the right to download, for example your own uploads, Creative Commons or public-domain content, or content the platform allows you to save. You are responsible for how you use Pulse DM.

Pulse DM does not bypass DRM and refuses DRM-protected streams. The same applies to torrents: share and download only content you are allowed to.

## Contributing

Bug reports, ideas and pull requests are welcome. Before a pull request:

```powershell
cargo fmt
cargo clippy --workspace --all-targets
cargo test -p pulse-engine
npm run check
```

Please open an issue first for larger changes, so we can agree on the approach.

**Translations.** Interface text is written in English inside `t("…")` calls. Each language is a dictionary keyed by that English text: see [`src/lib/locales/tr.ts`](src/lib/locales/tr.ts). To add a language, copy that file, translate the values and register it in [`src/lib/i18n.svelte.ts`](src/lib/i18n.svelte.ts). The extension's strings are in [`extension/_locales/`](extension/_locales/), and the few messages produced by Rust (tray menu, notifications, errors) use `l("English", "Türkçe")` and `text!` in the engine.

## License and credits

Pulse DM is released under the [MIT License](LICENSE). © 2026 [Burqk1](https://github.com/Burqk1).

Built with [Rust](https://www.rust-lang.org/), [Tokio](https://tokio.rs/), [reqwest](https://github.com/seanmonstar/reqwest), [librqbit](https://github.com/ikatson/rqbit) (Apache-2.0), [Tauri](https://tauri.app/) and [Svelte](https://svelte.dev/).

Video sites are handled by [yt-dlp](https://github.com/yt-dlp/yt-dlp) (Unlicense), and streams are merged by [FFmpeg](https://ffmpeg.org/) (GPL build from [yt-dlp/FFmpeg-Builds](https://github.com/yt-dlp/FFmpeg-Builds)). Both are downloaded on first use and are **not** bundled with Pulse DM; [Deno](https://deno.com/) (MIT) is fetched the same way when Node.js is not installed.
