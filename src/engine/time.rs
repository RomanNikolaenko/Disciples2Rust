//! Monotonic frame timing and periodic frame-time statistics.

use std::time::{Duration, Instant};

/// Upper bound for the time step fed into animations. After a stall (debugger, window
/// drag, focus change) animations advance by at most this much instead of jumping.
pub const MAX_ANIMATION_STEP: Duration = Duration::from_millis(100);

/// Measures the time between consecutive frames using a monotonic clock.
///
/// Callers pass `now` explicitly so the logic is deterministic in tests.
#[derive(Debug, Default)]
pub struct FrameClock {
    last: Option<Instant>,
}

impl FrameClock {
    /// Returns the time since the previous tick, or `None` on the first tick after
    /// creation or [`reset`](Self::reset).
    pub fn tick(&mut self, now: Instant) -> Option<Duration> {
        let delta = self.last.map(|last| now.saturating_duration_since(last));
        self.last = Some(now);
        delta
    }

    /// Forgets the previous tick, e.g. after rendering was paused while minimized, so the
    /// pause is not reported as one very long frame.
    pub fn reset(&mut self) {
        self.last = None;
    }
}

/// Aggregated frame times over one reporting interval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameSummary {
    pub frames: u32,
    pub average: Duration,
    pub worst: Duration,
}

/// Frames per second averaged over short windows, for an on-screen counter (updating
/// every frame would be unreadable).
#[derive(Debug)]
pub struct FpsCounter {
    window: Duration,
    elapsed: Duration,
    frames: u32,
}

impl FpsCounter {
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            elapsed: Duration::ZERO,
            frames: 0,
        }
    }

    /// Records one frame time; returns the average FPS each time a window completes.
    pub fn record(&mut self, frame_time: Duration) -> Option<u32> {
        self.elapsed += frame_time;
        self.frames += 1;
        if self.elapsed < self.window {
            return None;
        }
        let fps = (f64::from(self.frames) / self.elapsed.as_secs_f64()).round() as u32;
        self.elapsed = Duration::ZERO;
        self.frames = 0;
        Some(fps)
    }
}

/// Accumulates frame times and emits a [`FrameSummary`] once per interval.
#[derive(Debug)]
pub struct FrameStats {
    interval: Duration,
    elapsed: Duration,
    frames: u32,
    worst: Duration,
}

impl FrameStats {
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            elapsed: Duration::ZERO,
            frames: 0,
            worst: Duration::ZERO,
        }
    }

    /// Records one frame time. Returns a summary when the accumulated time reaches the
    /// interval, then starts a new interval.
    pub fn record(&mut self, frame_time: Duration) -> Option<FrameSummary> {
        self.elapsed += frame_time;
        self.frames += 1;
        self.worst = self.worst.max(frame_time);
        if self.elapsed < self.interval {
            return None;
        }

        let summary = FrameSummary {
            frames: self.frames,
            average: self.elapsed / self.frames,
            worst: self.worst,
        };
        self.elapsed = Duration::ZERO;
        self.frames = 0;
        self.worst = Duration::ZERO;
        Some(summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_tick_has_no_delta() {
        let start = Instant::now();
        let mut clock = FrameClock::default();
        assert_eq!(clock.tick(start), None);
        assert_eq!(
            clock.tick(start + Duration::from_millis(16)),
            Some(Duration::from_millis(16))
        );
    }

    #[test]
    fn reset_drops_the_pause() {
        let start = Instant::now();
        let mut clock = FrameClock::default();
        clock.tick(start);
        clock.reset();
        assert_eq!(clock.tick(start + Duration::from_secs(30)), None);
        assert_eq!(
            clock.tick(start + Duration::from_secs(30) + Duration::from_millis(7)),
            Some(Duration::from_millis(7))
        );
    }

    #[test]
    fn backwards_time_saturates_to_zero() {
        let start = Instant::now() + Duration::from_secs(1);
        let mut clock = FrameClock::default();
        clock.tick(start);
        assert_eq!(
            clock.tick(start - Duration::from_millis(5)),
            Some(Duration::ZERO)
        );
    }

    #[test]
    fn fps_counter_averages_over_its_window() {
        let mut counter = FpsCounter::new(Duration::from_millis(500));
        for _ in 0..29 {
            assert_eq!(counter.record(Duration::from_micros(16_667)), None);
        }
        assert_eq!(counter.record(Duration::from_micros(16_667)), Some(60));
        // The next window starts from scratch: 30 FPS frames.
        for _ in 0..14 {
            assert_eq!(counter.record(Duration::from_micros(33_334)), None);
        }
        assert_eq!(counter.record(Duration::from_micros(33_334)), Some(30));
    }

    #[test]
    fn summary_is_emitted_once_per_interval() {
        let mut stats = FrameStats::new(Duration::from_millis(100));
        for _ in 0..4 {
            assert_eq!(stats.record(Duration::from_millis(20)), None);
        }
        let summary = stats.record(Duration::from_millis(30));
        assert_eq!(
            summary,
            Some(FrameSummary {
                frames: 5,
                average: Duration::from_millis(22),
                worst: Duration::from_millis(30),
            })
        );
        // A new interval starts from scratch.
        assert_eq!(stats.record(Duration::from_millis(10)), None);
    }
}
