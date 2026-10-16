use std::time::{Duration, Instant};

use pulse_engine::{AddRequest, Config, Engine, EngineEvent, Status};

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let url = args.next().expect("usage: media <page url> [option] [out_dir]");
    let option = args.next();
    let dir = args.next().map(Into::into).unwrap_or_else(|| std::env::temp_dir().join("pulse-media"));
    let state = std::env::temp_dir().join("pulse-media-state");

    let engine = Engine::start(Config { download_dir: dir, ..Config::default() }, Some(state)).unwrap();
    let mut events = engine.subscribe();
    tokio::spawn(async move {
        while let Ok(ev) = events.recv().await {
            if let EngineEvent::Tool(t) = ev
                && t.done
            {
                println!("tool ready: {} ({} bytes)", t.name, t.downloaded);
            }
        }
    });

    let t = Instant::now();
    let info = engine.probe_media(&url, vec![]).await.expect("probe");
    println!("probe {:.1}s: {} [{}]", t.elapsed().as_secs_f64(), info.title, info.site.as_deref().unwrap_or("?"));
    for o in &info.options {
        println!(
            "  {:<6} {:<24} {:>10}",
            o.id,
            o.label,
            o.size.map(|s| format!("{:.1} MB", s as f64 / 1048576.0)).unwrap_or_default()
        );
    }
    let option = option.unwrap_or_else(|| info.options[0].id.clone());

    let t = Instant::now();
    let id = engine
        .add(AddRequest {
            url,
            media_option: Some(option.clone()),
            title: Some(info.title.clone()),
            ..Default::default()
        })
        .unwrap()
        .id;
    let mut last = Instant::now();
    loop {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let v = engine.get(id).unwrap();
        if last.elapsed() > Duration::from_secs(2) {
            last = Instant::now();
            println!(
                "  {:?} {:.1}/{:.1} MB  {:.1} MB/s  {} conn",
                v.status,
                v.downloaded as f64 / 1048576.0,
                v.size.unwrap_or(0) as f64 / 1048576.0,
                v.speed as f64 / 1048576.0,
                v.connections
            );
        }
        match v.status {
            Status::Completed => {
                let secs = t.elapsed().as_secs_f64();
                println!(
                    "done [{option}] {} | {:.1} MB in {:.1}s | {}",
                    v.file_name,
                    v.size.unwrap_or(0) as f64 / 1048576.0,
                    secs,
                    v.dir.join(&v.file_name).display()
                );
                return;
            }
            Status::Failed => panic!("failed: {:?}", v.error),
            _ => {}
        }
    }
}
