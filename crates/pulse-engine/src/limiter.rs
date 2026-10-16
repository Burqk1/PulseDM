use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub struct RateLimiter {
    rate: AtomicU64,
    state: Mutex<Bucket>,
}

struct Bucket {
    available: f64,
    last: Instant,
}

impl RateLimiter {
    pub fn new(bytes_per_sec: u64) -> Self {
        Self {
            rate: AtomicU64::new(bytes_per_sec),
            state: Mutex::new(Bucket { available: bytes_per_sec as f64, last: Instant::now() }),
        }
    }

    pub fn set_rate(&self, bytes_per_sec: u64) {
        self.rate.store(bytes_per_sec, Ordering::Relaxed);
    }

    pub async fn acquire(&self, bytes: usize) {
        let rate = self.rate.load(Ordering::Relaxed);
        if rate == 0 {
            return;
        }
        let wait = {
            let mut b = self.state.lock().unwrap();
            let now = Instant::now();
            let refill = now.duration_since(b.last).as_secs_f64() * rate as f64;
            b.available = (b.available + refill).min(rate as f64);
            b.last = now;
            b.available -= bytes as f64;
            if b.available >= 0.0 { None } else { Some(-b.available / rate as f64) }
        };
        if let Some(secs) = wait {
            tokio::time::sleep(Duration::from_secs_f64(secs)).await;
        }
    }
}
