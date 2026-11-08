//! Lightweight runtime counters for measuring frame cost.
//!
//! Logged periodically under the `rpob::stats` target. Enable with
//! `RUST_LOG=rpob::stats=info`.

use std::time::{Duration, Instant};

pub const TARGET: &str = "rpob::stats";
const REPORT_INTERVAL: Duration = Duration::from_secs(5);

pub struct FrameStats {
    window_start: Instant,
    frames: u32,
    elided: u32,
    frame_time: Duration,
    lua_time: Duration,
    max_frame_time: Duration,
}

impl Default for FrameStats {
    fn default() -> Self {
        Self {
            window_start: Instant::now(),
            frames: 0,
            elided: 0,
            frame_time: Duration::ZERO,
            lua_time: Duration::ZERO,
            max_frame_time: Duration::ZERO,
        }
    }
}

impl FrameStats {
    /// Records one frame. `frame_time` covers stage update, frame building, and rendering.
    pub fn record(&mut self, frame_time: Duration, lua_time: Duration, elided: bool) {
        self.frames += 1;
        self.elided += u32::from(elided);
        self.frame_time += frame_time;
        self.lua_time += lua_time;
        self.max_frame_time = self.max_frame_time.max(frame_time);
    }

    /// Logs and resets the counters once per report interval. `state` describes the window
    /// state so that idle numbers can be interpreted.
    pub fn maybe_report(&mut self, state: impl FnOnce() -> String) {
        let elapsed = self.window_start.elapsed();
        if elapsed < REPORT_INTERVAL {
            return;
        }

        if log::log_enabled!(target: TARGET, log::Level::Info) && self.frames > 0 {
            let frames = f64::from(self.frames);
            log::info!(
                target: TARGET,
                "fps={:.1} elided={}/{} avg_frame={:.2}ms avg_lua={:.2}ms max_frame={:.2}ms {}",
                frames / elapsed.as_secs_f64(),
                self.elided,
                self.frames,
                self.frame_time.as_secs_f64() * 1000.0 / frames,
                self.lua_time.as_secs_f64() * 1000.0 / frames,
                self.max_frame_time.as_secs_f64() * 1000.0,
                state(),
            );
        }

        *self = Self::default();
    }
}
