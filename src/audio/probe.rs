//! Pure signal decisions. Only the startup/probing path scans samples.
use super::Frame;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    Pending,
    Dead,
    Alive,
}

pub(super) struct Health {
    first: Option<Instant>,
    alive: bool,
}
impl Health {
    pub fn new() -> Self {
        Self {
            first: None,
            alive: false,
        }
    }
    pub fn callback(&mut self, now: Instant) {
        self.first.get_or_insert(now);
    }
    pub fn frame(&mut self, frame: &Frame, now: Instant, source: Source) -> Signal {
        if self.alive {
            return Signal::Alive;
        }
        let peak = frame.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
        self.alive |= peak > 2;
        if self.alive {
            Signal::Alive
        } else if self.first.is_some_and(|first| {
            now.duration_since(first)
                >= match source {
                    Source::Primary => Duration::from_millis(300),
                    Source::Candidate => Duration::from_secs(2),
                }
        }) {
            Signal::Dead
        } else {
            Signal::Pending
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Source {
    Primary,
    Candidate,
}

#[derive(Default)]
pub(super) struct Choice {
    pub chosen: Option<Source>,
    pub primary_dead: bool,
}
impl Choice {
    pub fn observe(&mut self, source: Source, signal: Signal) -> Option<Source> {
        if self.chosen.is_none() {
            if source == Source::Primary && signal == Signal::Dead {
                self.primary_dead = true;
            }
            if signal == Signal::Alive {
                self.chosen = Some(source);
            }
        }
        self.chosen
    }
}

pub(super) fn prefer_candidate(primary: &[i16], candidate: &[i16]) -> bool {
    fn energy(samples: &[i16]) -> u64 {
        samples
            .iter()
            .map(|&s| {
                let s = i64::from(s);
                (s * s) as u64
            })
            .sum()
    }
    energy(candidate) > energy(primary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::FRAME;
    #[test]
    fn exact_dead_threshold_starts_at_first_callback() {
        let start = Instant::now();
        let mut health = Health::new();
        health.callback(start);
        let mut frame = [2; FRAME];
        frame[0] = -2;
        assert_eq!(
            health.frame(&frame, start + Duration::from_millis(299), Source::Primary),
            Signal::Pending
        );
        assert_eq!(
            health.frame(&frame, start + Duration::from_millis(300), Source::Primary),
            Signal::Dead
        );
        frame[0] = -3;
        assert_eq!(
            health.frame(&frame, start + Duration::from_millis(301), Source::Primary),
            Signal::Alive
        );
        frame[0] = i16::MIN;
        assert_eq!(
            health.frame(&frame, start + Duration::from_millis(330), Source::Primary),
            Signal::Alive
        );
    }
    #[test]
    fn candidate_grace_starts_at_first_callback_and_allows_late_recovery() {
        let first = Instant::now();
        let mut health = Health::new();
        health.callback(first);
        // Later callbacks must not restart the warm-up grace.
        for millis in [299, 300, 1100, 1999] {
            health.callback(first + Duration::from_millis(millis));
            assert_eq!(
                health.frame(
                    &[0; FRAME],
                    first + Duration::from_millis(millis),
                    Source::Candidate
                ),
                Signal::Pending
            );
        }
        assert_eq!(
            health.frame(
                &[0; FRAME],
                first + Duration::from_secs(2),
                Source::Candidate
            ),
            Signal::Dead
        );
        assert_eq!(
            health.frame(
                &[3; FRAME],
                first + Duration::from_secs(3),
                Source::Candidate
            ),
            Signal::Alive
        );
    }
    #[test]
    fn dead_primary_loses_to_first_live_candidate() {
        let mut choice = Choice::default();
        assert_eq!(choice.observe(Source::Primary, Signal::Dead), None);
        assert!(choice.primary_dead);
        assert_eq!(
            choice.observe(Source::Candidate, Signal::Alive),
            Some(Source::Candidate)
        );
        assert_eq!(
            choice.observe(Source::Primary, Signal::Alive),
            Some(Source::Candidate)
        );
    }
    #[test]
    fn quiet_primary_can_recover_and_win_during_probing() {
        let start = Instant::now();
        let mut health = Health::new();
        health.callback(start);
        let mut choice = Choice::default();
        for n in 0..=10 {
            choice.observe(
                Source::Primary,
                health.frame(
                    &[0; FRAME],
                    start + Duration::from_millis(n * 30),
                    Source::Primary,
                ),
            );
        }
        assert!(choice.primary_dead && choice.chosen.is_none());
        assert_eq!(choice.observe(Source::Candidate, Signal::Pending), None);
        assert_eq!(
            choice.observe(
                Source::Primary,
                health.frame(
                    &[3; FRAME],
                    start + Duration::from_millis(330),
                    Source::Primary
                )
            ),
            Some(Source::Primary)
        );
        assert_eq!(
            choice.observe(Source::Candidate, Signal::Alive),
            Some(Source::Primary)
        );
    }
    #[test]
    fn every_device_dead_leaves_choice_pending_and_finish_uses_energy() {
        let mut choice = Choice::default();
        for source in [Source::Primary, Source::Candidate, Source::Candidate] {
            assert_eq!(choice.observe(source, Signal::Dead), None);
        }
        assert!(prefer_candidate(&[0; FRAME], &[2; FRAME]));
        assert!(!prefer_candidate(&[2; FRAME], &[1; FRAME]));
        assert!(!prefer_candidate(&[0; FRAME], &[0; FRAME]));
    }
    #[test]
    #[ignore = "Offline health-scan allocation check; run alone in release mode"]
    fn microphone_probe_allocates_nothing_per_frame() {
        let start = Instant::now();
        for source in [Source::Primary, Source::Candidate] {
            let mut health = Health::new();
            health.callback(start);
            let (allocations, peak) = crate::audio::tests::allocations(|| {
                for n in 0..1000 {
                    std::hint::black_box(health.frame(
                        std::hint::black_box(&[2; FRAME]),
                        start + Duration::from_millis(n * 30),
                        source,
                    ));
                }
            });
            assert_eq!((allocations, peak), (0, 0));
        }
        println!("1,000 startup/dead-probe frames per source: zero Rust heap allocations; healthy committed callbacks bypass peak scanning.");
    }
}
