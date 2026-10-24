//! Latency and frame rate, printed once a second and at the end.

use std::time::{Duration, Instant};

/// Samples of a window, in milliseconds
#[derive(Default, Clone, Debug)]
pub struct Samples(Vec<f32>);

impl Samples {
    pub fn push(&mut self, ms: f32) {
        self.0.push(ms);
    }

    pub fn mean(&self) -> f32 {
        if self.0.is_empty() {
            return 0.0;
        }
        self.0.iter().sum::<f32>() / self.0.len() as f32
    }

    /// The `q`-quantile (0 to 1) by nearest rank
    pub fn quantile(&self, q: f32) -> f32 {
        if self.0.is_empty() {
            return 0.0;
        }
        let mut v = self.0.clone();
        v.sort_by(f32::total_cmp);
        let i = ((v.len() as f32 * q).ceil() as usize).clamp(1, v.len()) - 1;
        v[i]
    }

    pub fn max(&self) -> f32 {
        self.0.iter().copied().fold(0.0, f32::max)
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    pub fn extend(&mut self, other: &Samples) {
        self.0.extend_from_slice(&other.0);
    }
}

#[derive(Debug)]
pub struct Stats {
    started: Instant,
    window_start: Instant,
    /// Cameras received in this window
    pub cameras: u32,
    /// Cameras that were dropped because a newer one arrived before a render slot was free
    pub skipped: u32,
    /// Frames sent in this window
    pub frames: u32,
    /// Camera received to frame sent, the guest's own clock
    pub latency: Samples,
    /// Host's camera sent (its timestamp) to frame sent
    pub host_latency: Samples,
    /// Camera received to render requested
    pub queue: Samples,
    totals: Totals,
}

#[derive(Default, Debug)]
struct Totals {
    cameras: u64,
    skipped: u64,
    frames: u64,
    latency: Samples,
    host_latency: Samples,
}

impl Stats {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            started: now,
            window_start: now,
            cameras: 0,
            skipped: 0,
            frames: 0,
            latency: Samples::default(),
            host_latency: Samples::default(),
            queue: Samples::default(),
            totals: Totals::default(),
        }
    }

    /// The line for the window so far, if a second has passed (and the window starts again)
    pub fn tick(&mut self) -> Option<String> {
        let dt = self.window_start.elapsed();
        if dt < Duration::from_secs(1) {
            return None;
        }
        let secs = dt.as_secs_f32();
        let line = format!(
            "[{:6.1}s] cameras {:5.1}/s (skipped {}), frames {:5.1}/s, latency ms: mean {:5.1}  p50 {:5.1}  p95 {:5.1}  max {:5.1}  (host clock: mean {:5.1})",
            self.started.elapsed().as_secs_f32(),
            self.cameras as f32 / secs,
            self.skipped,
            self.frames as f32 / secs,
            self.latency.mean(),
            self.latency.quantile(0.5),
            self.latency.quantile(0.95),
            self.latency.max(),
            self.host_latency.mean(),
        );
        self.totals.cameras += u64::from(self.cameras);
        self.totals.skipped += u64::from(self.skipped);
        self.totals.frames += u64::from(self.frames);
        self.totals.latency.extend(&self.latency);
        self.totals.host_latency.extend(&self.host_latency);
        self.cameras = 0;
        self.skipped = 0;
        self.frames = 0;
        self.latency.clear();
        self.host_latency.clear();
        self.queue.clear();
        self.window_start = Instant::now();
        Some(line)
    }

    /// Everything since the start
    pub fn summary(&mut self) -> String {
        // Fold the open window in
        let _ = self.tick_force();
        let t = &self.totals;
        let secs = self.started.elapsed().as_secs_f32().max(1e-3);
        format!(
            "summary: {} cameras, {} skipped, {} frames in {:.1}s = {:.1} fps; latency ms (camera received to frame sent): mean {:.1}  p50 {:.1}  p95 {:.1}  max {:.1}  (host clock: mean {:.1}  p95 {:.1})",
            t.cameras,
            t.skipped,
            t.frames,
            secs,
            t.frames as f32 / secs,
            t.latency.mean(),
            t.latency.quantile(0.5),
            t.latency.quantile(0.95),
            t.latency.max(),
            t.host_latency.mean(),
            t.host_latency.quantile(0.95),
        )
    }

    fn tick_force(&mut self) -> Option<String> {
        self.window_start = Instant::now() - Duration::from_secs(2);
        self.tick()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantiles_use_the_nearest_rank() {
        let mut s = Samples::default();
        for v in 1..=100 {
            s.push(v as f32);
        }
        assert_eq!(s.quantile(0.5), 50.0);
        assert_eq!(s.quantile(0.95), 95.0);
        assert_eq!(s.max(), 100.0);
        assert!((s.mean() - 50.5).abs() < 1e-4);
        assert_eq!(Samples::default().quantile(0.5), 0.0);
    }

    #[test]
    fn the_summary_counts_frames() {
        let mut s = Stats::new();
        s.frames = 3;
        s.cameras = 5;
        s.latency.push(10.0);
        let text = s.summary();
        assert!(
            text.contains("3 frames") && text.contains("5 cameras"),
            "{text}"
        );
    }
}
