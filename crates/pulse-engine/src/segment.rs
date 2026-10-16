use std::sync::Mutex;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentState {
    pub start: u64,
    pub pos: u64,
    pub end: u64,
}

#[derive(Debug)]
struct Seg {
    start: u64,
    pos: u64,
    claimed: u64,
    end: u64,
    active: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Assignment {
    Work { idx: usize, from: u64 },
    Wait,
    Done,
}

pub struct SegmentTable {
    segs: Mutex<Vec<Seg>>,
    min_split: u64,
}

impl SegmentTable {
    pub fn new(total: u64, parts: usize, min_split: u64) -> Self {
        let parts = (parts.max(1) as u64).min((total / min_split.max(1)).max(1));
        let step = total / parts;
        let segs = (0..parts)
            .map(|i| {
                let start = i * step;
                let end = if i + 1 == parts { total } else { start + step };
                Seg { start, pos: start, claimed: start, end, active: false }
            })
            .collect();
        Self { segs: Mutex::new(segs), min_split }
    }

    pub fn unbounded() -> Self {
        Self {
            segs: Mutex::new(vec![Seg { start: 0, pos: 0, claimed: 0, end: u64::MAX, active: false }]),
            min_split: u64::MAX,
        }
    }

    pub fn counter() -> Self {
        Self::unbounded()
    }

    pub fn bump(&self, bytes: u64) {
        let mut segs = self.segs.lock().unwrap();
        segs[0].pos += bytes;
        segs[0].claimed = segs[0].pos;
    }

    pub fn set_counter(&self, bytes: u64) {
        let mut segs = self.segs.lock().unwrap();
        segs[0].pos = bytes;
        segs[0].claimed = bytes;
    }

    pub fn from_states(states: &[SegmentState], min_split: u64) -> Self {
        let segs = states
            .iter()
            .map(|s| Seg { start: s.start, pos: s.pos, claimed: s.pos, end: s.end, active: false })
            .collect();
        Self { segs: Mutex::new(segs), min_split }
    }

    pub fn acquire(&self) -> Assignment {
        let mut segs = self.segs.lock().unwrap();

        if let Some((idx, s)) = segs.iter_mut().enumerate().find(|(_, s)| !s.active && s.pos < s.end) {
            s.active = true;
            s.claimed = s.pos;
            return Assignment::Work { idx, from: s.pos };
        }

        let victim = segs
            .iter()
            .enumerate()
            .filter(|(_, s)| s.active)
            .map(|(i, s)| (i, s.end - s.claimed))
            .max_by_key(|&(_, remaining)| remaining);

        match victim {
            Some((i, remaining)) if remaining >= self.min_split.saturating_mul(2) => {
                let mid = segs[i].claimed + remaining / 2;
                let old_end = segs[i].end;
                segs[i].end = mid;
                segs.push(Seg { start: mid, pos: mid, claimed: mid, end: old_end, active: true });
                Assignment::Work { idx: segs.len() - 1, from: mid }
            }
            Some(_) => Assignment::Wait,
            None => Assignment::Done,
        }
    }

    pub fn claim(&self, idx: usize, len: usize) -> (u64, usize) {
        let mut segs = self.segs.lock().unwrap();
        let s = &mut segs[idx];
        let take = (len as u64).min(s.end - s.claimed);
        let off = s.claimed;
        s.claimed += take;
        (off, take as usize)
    }

    pub fn commit(&self, idx: usize, upto: u64) {
        self.segs.lock().unwrap()[idx].pos = upto;
    }

    pub fn release(&self, idx: usize) {
        let mut segs = self.segs.lock().unwrap();
        let s = &mut segs[idx];
        s.active = false;
        s.claimed = s.pos;
    }

    pub fn pos(&self, idx: usize) -> u64 {
        self.segs.lock().unwrap()[idx].pos
    }

    pub fn remaining(&self, idx: usize) -> u64 {
        let segs = self.segs.lock().unwrap();
        segs[idx].end - segs[idx].pos
    }

    pub fn close_unbounded(&self, idx: usize) {
        let mut segs = self.segs.lock().unwrap();
        let s = &mut segs[idx];
        s.end = s.pos;
        s.claimed = s.pos;
    }

    pub fn remaining_total(&self) -> u64 {
        self.segs.lock().unwrap().iter().map(|s| s.end.saturating_sub(s.pos)).sum()
    }

    pub fn downloaded(&self) -> u64 {
        self.segs.lock().unwrap().iter().map(|s| s.pos - s.start).sum()
    }

    pub fn is_complete(&self) -> bool {
        self.segs.lock().unwrap().iter().all(|s| s.pos >= s.end)
    }

    pub fn active_connections(&self) -> usize {
        self.segs.lock().unwrap().iter().filter(|s| s.active).count()
    }

    pub fn snapshot(&self) -> Vec<SegmentState> {
        let segs = self.segs.lock().unwrap();
        let mut out: Vec<SegmentState> =
            segs.iter().map(|s| SegmentState { start: s.start, pos: s.pos, end: s.end }).collect();
        out.sort_by_key(|s| s.start);
        let mut merged: Vec<SegmentState> = Vec::with_capacity(out.len());
        for s in out {
            match merged.last_mut() {
                Some(prev) if prev.pos == prev.end && prev.end == s.start => {
                    prev.pos = s.pos;
                    prev.end = s.end;
                }
                _ => merged.push(s),
            }
        }
        merged
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: u64 = 1024 * 1024;

    #[test]
    fn splits_evenly_and_covers_whole_file() {
        let t = SegmentTable::new(10 * MB + 3, 4, MB);
        let snap = t.snapshot();
        assert_eq!(snap.len(), 4);
        assert_eq!(snap[0].start, 0);
        assert_eq!(snap.last().unwrap().end, 10 * MB + 3);
        for w in snap.windows(2) {
            assert_eq!(w[0].end, w[1].start);
        }
    }

    #[test]
    fn small_files_get_fewer_segments() {
        assert_eq!(SegmentTable::new(MB / 2, 8, MB).snapshot().len(), 1);
        assert_eq!(SegmentTable::new(3 * MB, 8, MB).snapshot().len(), 3);
    }

    #[test]
    fn idle_worker_steals_half_of_largest_range() {
        let t = SegmentTable::new(8 * MB, 1, MB);
        let Assignment::Work { idx: a, from } = t.acquire() else { panic!() };
        assert_eq!(from, 0);
        let (_, take) = t.claim(a, MB as usize);
        t.commit(a, take as u64);

        let Assignment::Work { idx: b, from } = t.acquire() else { panic!() };
        assert_eq!(from, MB + 7 * MB / 2);

        let (off, take) = t.claim(a, usize::MAX);
        assert_eq!(off, MB);
        assert_eq!(off + take as u64, from);
        assert_ne!(a, b);
    }

    #[test]
    fn tiny_remainders_are_not_split() {
        let t = SegmentTable::new(MB, 1, MB);
        assert!(matches!(t.acquire(), Assignment::Work { .. }));
        assert_eq!(t.acquire(), Assignment::Wait);
    }

    #[test]
    fn released_segment_is_picked_up_again_and_done_when_written() {
        let t = SegmentTable::new(MB, 1, MB);
        let Assignment::Work { idx, .. } = t.acquire() else { panic!() };
        let (_, take) = t.claim(idx, 1000);
        t.commit(idx, take as u64);
        t.release(idx);
        assert_eq!(t.acquire(), Assignment::Work { idx, from: 1000 });
        let (off, take) = t.claim(idx, usize::MAX);
        t.commit(idx, off + take as u64);
        t.release(idx);
        assert_eq!(t.acquire(), Assignment::Done);
        assert!(t.is_complete());
        assert_eq!(t.downloaded(), MB);
    }

    #[test]
    fn snapshot_round_trips_and_merges_finished_neighbours() {
        let t = SegmentTable::from_states(
            &[SegmentState { start: 0, pos: 10, end: 10 }, SegmentState { start: 10, pos: 15, end: 20 }],
            1,
        );
        assert_eq!(t.snapshot(), vec![SegmentState { start: 0, pos: 15, end: 20 }]);
        assert_eq!(t.downloaded(), 15);
    }
}
