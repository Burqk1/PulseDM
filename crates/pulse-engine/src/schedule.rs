use chrono::{Datelike, Local, Timelike};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct QueueConfig {
    pub id: String,
    pub name: String,
    pub max_active: usize,
    pub running: bool,
    pub schedule: Schedule,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self { id: String::new(), name: String::new(), max_active: 3, running: false, schedule: Schedule::default() }
    }
}

impl QueueConfig {
    pub fn main() -> Self {
        Self {
            id: crate::engine::MAIN_QUEUE.into(),
            name: if crate::lang::english() { "Main queue" } else { "Ana kuyruk" }.into(),
            running: true,
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Schedule {
    pub enabled: bool,
    pub days: Vec<u8>,
    pub start: Option<String>,
    pub stop: Option<String>,
}

impl Schedule {
    pub fn action_at(&self, weekday: u8, minute: u32) -> Option<bool> {
        if !self.enabled || !(self.days.is_empty() || self.days.contains(&weekday)) {
            return None;
        }
        if self.start.as_deref().and_then(parse_hm) == Some(minute) {
            return Some(true);
        }
        if self.stop.as_deref().and_then(parse_hm) == Some(minute) {
            return Some(false);
        }
        None
    }
}

pub fn parse_hm(s: &str) -> Option<u32> {
    let (h, m) = s.trim().split_once(':')?;
    let (h, m): (u32, u32) = (h.trim().parse().ok()?, m.trim().parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

#[derive(Default)]
pub struct ScheduleClock {
    last: Option<(u8, u32)>,
}

impl ScheduleClock {
    pub fn due(&mut self, queues: &[QueueConfig]) -> Vec<(String, bool)> {
        let now = Local::now();
        let key = (now.weekday().num_days_from_monday() as u8, now.hour() * 60 + now.minute());
        if self.last == Some(key) {
            return Vec::new();
        }
        self.last = Some(key);
        queues
            .iter()
            .filter_map(|q| {
                q.schedule.action_at(key.0, key.1).filter(|&run| run != q.running).map(|run| (q.id.clone(), run))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_times() {
        assert_eq!(parse_hm("07:30"), Some(450));
        assert_eq!(parse_hm(" 0:05 "), Some(5));
        assert_eq!(parse_hm("24:00"), None);
        assert_eq!(parse_hm("aa"), None);
    }

    #[test]
    fn fires_only_on_chosen_days_and_minutes() {
        let s = Schedule { enabled: true, days: vec![0, 4], start: Some("02:00".into()), stop: Some("07:00".into()) };
        assert_eq!(s.action_at(0, 120), Some(true));
        assert_eq!(s.action_at(4, 420), Some(false));
        assert_eq!(s.action_at(1, 120), None, "Tuesday is not selected");
        assert_eq!(s.action_at(0, 121), None);
        assert_eq!(Schedule { enabled: false, ..s.clone() }.action_at(0, 120), None);
        assert_eq!(Schedule { days: vec![], ..s }.action_at(6, 120), Some(true), "no days = every day");
    }
}
