use std::path::PathBuf;
use std::time::{Duration, Instant};

use pulse_engine::{AddRequest, Config, Engine, Status};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let source = args.next().ok_or("usage: torrent <magnet | .torrent url/path> [stop-after-MB] [out-dir]")?;
    let cap_mb: u64 = args.next().map(|a| a.parse()).transpose()?.unwrap_or(0);
    let out = PathBuf::from(args.next().unwrap_or_else(|| "torrent-out".into()));
    std::fs::create_dir_all(&out)?;

    let config = Config { download_dir: out.clone(), category_folders: false, ..Config::default() };
    let engine = Engine::start(config, None)?;
    let id = engine.add(AddRequest { url: source, ..Default::default() })?.id;
    let started = Instant::now();

    loop {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let d = engine.get(id).ok_or("download vanished")?;
        println!(
            "{:>5.0}s  {:?}  {}  {:.1}/{:.1} MB  {:.2} MB/s  peers {}",
            started.elapsed().as_secs_f64(),
            d.status,
            d.file_name,
            d.downloaded as f64 / 1e6,
            d.size.unwrap_or(0) as f64 / 1e6,
            d.speed as f64 / 1e6,
            d.connections,
        );
        match d.status {
            Status::Completed => break,
            Status::Failed => return Err(d.error.unwrap_or_default().into()),
            _ => {}
        }
        if cap_mb > 0 && d.downloaded >= cap_mb * 1_000_000 {
            println!("reached {cap_mb} MB, stopping");
            engine.pause(id)?;
            tokio::time::sleep(Duration::from_secs(1)).await;
            let d = engine.get(id).ok_or("download vanished")?;
            let kept = d.downloaded;
            println!("after pause: {:?}, {:.1} MB kept for resume", d.status, kept as f64 / 1e6);
            engine.resume(id)?;
            for _ in 0..60 {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let d = engine.get(id).ok_or("download vanished")?;
                println!("resumed: {:?}  {:.1} MB  peers {}", d.status, d.downloaded as f64 / 1e6, d.connections);
                if d.status == Status::Downloading && d.speed > 0 {
                    break;
                }
            }
            let d = engine.get(id).ok_or("download vanished")?;
            if d.downloaded < kept {
                return Err(format!("resume lost data: {} < {kept}", d.downloaded).into());
            }
            engine.pause(id)?;
            tokio::time::sleep(Duration::from_secs(1)).await;
            engine.remove(id, true)?;
            tokio::time::sleep(Duration::from_millis(500)).await;
            break;
        }
    }
    engine.shutdown().await;
    Ok(())
}
