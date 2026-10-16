use std::time::{Duration, Instant};

use pulse_engine::{AddRequest, Config, Engine, Status};

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let url = args.next().expect("usage: bench <url> [connections] [out_dir]");
    let connections: usize = args.next().map(|c| c.parse().expect("connections")).unwrap_or(8);
    let dir = args.next().map(Into::into).unwrap_or_else(|| std::env::temp_dir().join("pulse-bench"));

    let engine = Engine::start(Config { download_dir: dir, connections, ..Config::default() }, None).unwrap();
    let started = Instant::now();
    let id = engine.add(AddRequest { url, ..Default::default() }).unwrap().id;

    loop {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let v = engine.get(id).unwrap();
        match v.status {
            Status::Completed => {
                let secs = started.elapsed().as_secs_f64();
                println!(
                    "{} | {} conn | {:.2} MB in {:.2}s | {:.2} MB/s | {}",
                    v.file_name,
                    connections,
                    v.downloaded as f64 / 1048576.0,
                    secs,
                    v.downloaded as f64 / 1048576.0 / secs,
                    v.dir.join(&v.file_name).display()
                );
                return;
            }
            Status::Failed => panic!("failed: {:?}", v.error),
            _ => {}
        }
    }
}
