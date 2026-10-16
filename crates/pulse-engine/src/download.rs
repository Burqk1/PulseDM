use std::fs::File;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use reqwest::header::{CONTENT_RANGE, HeaderMap, IF_RANGE, RANGE, RETRY_AFTER};
use reqwest::{Client, StatusCode};
use tokio::sync::Notify;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::error::{EngineError, Result};
use crate::fsutil::write_all_at;
use crate::lang::text;
use crate::limiter::RateLimiter;
use crate::probe::{is_html, parse_content_range_start};
use crate::segment::{Assignment, SegmentTable};

const WRITE_BUFFER: usize = 256 * 1024;
const MAX_CONNECTIONS: usize = 32;

const RAMP_INTERVAL: Duration = Duration::from_secs(2);
const RAMP_STEP: usize = 4;
const RAMP_MIN_GAIN: f64 = 1.08;

pub struct JobControl {
    target_connections: AtomicUsize,
    wake: Notify,
    pub limiter: RateLimiter,
    auto: AtomicBool,
    auto_max: usize,
}

impl JobControl {
    pub fn new(connections: usize, speed_limit: u64) -> Self {
        Self {
            target_connections: AtomicUsize::new(connections.clamp(1, MAX_CONNECTIONS)),
            wake: Notify::new(),
            limiter: RateLimiter::new(speed_limit),
            auto: AtomicBool::new(false),
            auto_max: connections,
        }
    }

    pub fn automatic(connections: usize, max: usize, speed_limit: u64) -> Self {
        let max = max.clamp(connections, MAX_CONNECTIONS);
        Self { auto: AtomicBool::new(max > connections), auto_max: max, ..Self::new(connections, speed_limit) }
    }

    pub fn set_connections(&self, n: usize) {
        self.auto.store(false, Ordering::Relaxed);
        self.set_target(n);
    }

    fn set_target(&self, n: usize) {
        self.target_connections.store(n.clamp(1, MAX_CONNECTIONS), Ordering::Relaxed);
        self.wake.notify_one();
    }

    pub fn connections(&self) -> usize {
        self.target_connections.load(Ordering::Relaxed)
    }
}

pub(crate) struct Job {
    pub client: Client,
    pub limiters: Vec<Arc<RateLimiter>>,
    pub control: Arc<JobControl>,
    pub cancel: CancellationToken,
    pub table: Arc<SegmentTable>,
    pub file: Arc<File>,
    pub url: String,
    pub headers: HeaderMap,
    pub ranged: bool,
    pub bounded: bool,
    pub validator: Option<String>,
    pub max_retries: u32,
    pub accept_html: bool,
    pub max_request: Option<u64>,
    pub live: AtomicUsize,
    pub refusals: AtomicUsize,
}

struct Ramp {
    last_bytes: u64,
    last_at: Instant,
    trial: Option<(usize, f64)>,
    warmup: u32,
    frozen: bool,
}

impl Ramp {
    fn new(job: &Job) -> Self {
        Self { last_bytes: job.table.downloaded(), last_at: Instant::now(), trial: None, warmup: 2, frozen: false }
    }

    fn step(&mut self, job: &Job) {
        let now = Instant::now();
        let bytes = job.table.downloaded();
        let rate = bytes.saturating_sub(self.last_bytes) as f64 / now.duration_since(self.last_at).as_secs_f64();
        self.last_bytes = bytes;
        self.last_at = now;
        if self.frozen || !job.control.auto.load(Ordering::Relaxed) {
            return;
        }
        if job.refusals.swap(0, Ordering::Relaxed) > 0 {
            self.frozen = true;
            return;
        }
        if self.warmup > 0 {
            self.warmup -= 1;
            return;
        }
        if let Some((before, before_rate)) = self.trial.take() {
            if rate < before_rate * RAMP_MIN_GAIN {
                job.control.set_target(before);
                self.frozen = true;
            }
            return;
        }
        let current = job.control.connections();
        let left = job.table.remaining_total() as f64;
        if current < job.control.auto_max && rate > 0.0 && left > rate * 6.0 {
            self.trial = Some((current, rate));
            job.control.set_target((current + RAMP_STEP).min(job.control.auto_max));
        }
    }
}

enum Exit {
    Done,
    Retired,
}

enum FetchEnd {
    Finished,
    Yield,
}

impl Job {
    fn target(&self) -> usize {
        if self.ranged { self.control.connections() } else { 1 }
    }

    fn try_retire(&self, keep: usize) -> bool {
        let mut cur = self.live.load(Ordering::SeqCst);
        loop {
            if cur <= keep.max(1) {
                return false;
            }
            match self.live.compare_exchange_weak(cur, cur - 1, Ordering::SeqCst, Ordering::SeqCst) {
                Ok(_) => return true,
                Err(actual) => cur = actual,
            }
        }
    }

    async fn throttle(&self, bytes: usize) {
        for l in &self.limiters {
            l.acquire(bytes).await;
        }
        self.control.limiter.acquire(bytes).await;
    }
}

fn spawn_up_to_target(job: &Arc<Job>, set: &mut JoinSet<Result<Exit>>) {
    while job.live.load(Ordering::SeqCst) < job.target() && !job.cancel.is_cancelled() {
        job.live.fetch_add(1, Ordering::SeqCst);
        set.spawn(worker(job.clone()));
    }
}

pub(crate) async fn run(job: Arc<Job>) -> Result<()> {
    let mut set = JoinSet::new();
    spawn_up_to_target(&job, &mut set);
    let mut ramp = Ramp::new(&job);
    let mut ramp_tick = tokio::time::interval_at(tokio::time::Instant::now() + RAMP_INTERVAL, RAMP_INTERVAL);

    let mut first_error = None;
    loop {
        tokio::select! {
            res = set.join_next() => {
                let Some(res) = res else { break };
                match res.expect("download worker panicked") {
                    Ok(Exit::Retired) => {}
                    Ok(Exit::Done) => {
                        job.live.fetch_sub(1, Ordering::SeqCst);
                    }
                    Err(e) => {
                        job.live.fetch_sub(1, Ordering::SeqCst);
                        if matches!(e, EngineError::Changed) {
                            job.cancel.cancel();
                            first_error = Some(e);
                        } else if first_error.is_none() {
                            first_error = Some(e);
                        }
                    }
                }
            }
            _ = job.control.wake.notified() => spawn_up_to_target(&job, &mut set),
            _ = ramp_tick.tick(), if job.ranged => ramp.step(&job),
        }
    }

    if job.cancel.is_cancelled() && !matches!(first_error, Some(EngineError::Changed)) {
        return Err(EngineError::Cancelled);
    }
    if job.table.is_complete() {
        return Ok(());
    }
    Err(first_error.unwrap_or_else(|| {
        EngineError::BadResponse(text!("the download could not be finished", "indirme tamamlanamadı"))
    }))
}

async fn worker(job: Arc<Job>) -> Result<Exit> {
    let mut failures = 0u32;
    loop {
        if job.cancel.is_cancelled() {
            return Err(EngineError::Cancelled);
        }
        if job.live.load(Ordering::SeqCst) > job.target() && job.try_retire(job.target()) {
            return Ok(Exit::Retired);
        }
        match job.table.acquire() {
            Assignment::Done => return Ok(Exit::Done),
            Assignment::Wait => {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_millis(250)) => {}
                    _ = job.cancel.cancelled() => return Err(EngineError::Cancelled),
                }
            }
            Assignment::Work { idx, from } => {
                let res = fetch(&job, idx, from).await;
                let progressed = job.table.pos(idx) > from;
                job.table.release(idx);
                match res {
                    Ok(FetchEnd::Finished) => failures = 0,
                    Ok(FetchEnd::Yield) => {
                        if job.try_retire(job.target()) {
                            return Ok(Exit::Retired);
                        }
                    }
                    Err(e @ (EngineError::Cancelled | EngineError::Changed | EngineError::WebPage)) => return Err(e),
                    Err(e) => {
                        if progressed {
                            failures = 0;
                        }
                        if e.is_refusal() {
                            job.refusals.fetch_add(1, Ordering::Relaxed);
                        }
                        if e.is_refusal() && job.try_retire(1) {
                            return Ok(Exit::Retired);
                        }
                        if matches!(e, EngineError::LinkDead(_)) {
                            return Err(e);
                        }
                        failures += 1;
                        if failures > job.max_retries {
                            return Err(e);
                        }
                        let backoff =
                            e.retry_after().unwrap_or_else(|| Duration::from_millis(500 * 2u64.pow(failures.min(5))));
                        tokio::select! {
                            _ = tokio::time::sleep(backoff) => {}
                            _ = job.cancel.cancelled() => return Err(EngineError::Cancelled),
                        }
                    }
                }
            }
        }
    }
}

async fn fetch(job: &Job, idx: usize, from: u64) -> Result<FetchEnd> {
    let mut req = job.client.get(&job.url).headers(job.headers.clone());
    let mut request_end = u64::MAX;
    if job.ranged {
        let seg_end = from + job.table.remaining(idx);
        request_end = job.max_request.map_or(seg_end, |max| seg_end.min(from + max));
        req = req.header(RANGE, format!("bytes={from}-{}", request_end - 1));
        if let Some(v) = &job.validator {
            req = req.header(IF_RANGE, v);
        }
    }

    let resp = tokio::select! {
        r = req.send() => r?,
        _ = job.cancel.cancelled() => return Err(EngineError::Cancelled),
    };

    let status = resp.status();
    check_status(status, resp.headers())?;
    if job.ranged {
        if status == StatusCode::OK && job.validator.is_some() {
            return Err(EngineError::Changed);
        }
        if status != StatusCode::PARTIAL_CONTENT {
            return Err(EngineError::BadResponse(format!("HTTP {}", status.as_u16())));
        }
        let start = resp.headers().get(CONTENT_RANGE).and_then(|v| v.to_str().ok()).and_then(parse_content_range_start);
        if start != Some(from) {
            return Err(EngineError::BadResponse(text!(
                "the server sent the wrong range",
                "sunucu yanlış aralık gönderdi"
            )));
        }
    } else if !status.is_success() {
        return Err(EngineError::BadResponse(format!("HTTP {}", status.as_u16())));
    }
    if !job.accept_html && is_html(resp.headers()) {
        return Err(EngineError::WebPage);
    }

    let mut stream = resp.bytes_stream();
    let mut buf = Vec::with_capacity(WRITE_BUFFER);
    loop {
        let next = tokio::select! {
            biased;
            _ = job.cancel.cancelled() => {
                flush(job, idx, &mut buf).await?;
                return Err(EngineError::Cancelled);
            }
            next = stream.next() => next,
        };
        match next {
            Some(Ok(chunk)) => {
                job.throttle(chunk.len()).await;
                buf.extend_from_slice(&chunk);
                if buf.len() >= WRITE_BUFFER {
                    if flush(job, idx, &mut buf).await? {
                        return Ok(FetchEnd::Finished);
                    }
                    if job.live.load(Ordering::SeqCst) > job.target() {
                        return Ok(FetchEnd::Yield);
                    }
                }
            }
            Some(Err(e)) => {
                flush(job, idx, &mut buf).await?;
                return Err(e.into());
            }
            None => {
                flush(job, idx, &mut buf).await?;
                if !job.bounded {
                    job.table.close_unbounded(idx);
                    return Ok(FetchEnd::Finished);
                }
                return if job.table.remaining(idx) == 0 || job.table.pos(idx) >= request_end {
                    Ok(FetchEnd::Finished)
                } else {
                    Err(EngineError::BadResponse(text!("the connection closed early", "bağlantı erken kapandı")))
                };
            }
        }
    }
}

pub(crate) fn check_status(status: StatusCode, headers: &HeaderMap) -> Result<()> {
    match status.as_u16() {
        429 | 503 => {
            let retry_after = headers
                .get(RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<u64>().ok())
                .map(|s| Duration::from_secs(s.min(60)));
            Err(EngineError::Throttled { status: status.as_u16(), retry_after })
        }
        code @ (401 | 403 | 404 | 410) => Err(EngineError::LinkDead(code)),
        _ => Ok(()),
    }
}

async fn flush(job: &Job, idx: usize, buf: &mut Vec<u8>) -> Result<bool> {
    if buf.is_empty() {
        return Ok(job.table.remaining(idx) == 0);
    }
    let len = buf.len();
    let (offset, take) = job.table.claim(idx, len);
    if take > 0 {
        let data = std::mem::take(buf);
        let file = job.file.clone();
        let (mut data, res) = tokio::task::spawn_blocking(move || {
            let res = write_all_at(&file, &data[..take], offset);
            (data, res)
        })
        .await
        .expect("disk writer panicked");
        res?;
        job.table.commit(idx, offset + take as u64);
        data.clear();
        *buf = data;
    } else {
        buf.clear();
    }
    Ok(take < len || job.table.remaining(idx) == 0)
}
