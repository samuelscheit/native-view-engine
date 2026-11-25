use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use nve_abi::FrameId;

/// Exact microsecond breakdown across every pipeline stage so performance stays measurable.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct FrameTiming {
    pub frame_id: FrameId,
    pub op_count: u32,
    pub mutation_count: u32,
    pub reconcile_us: u64,
    pub encode_us: u64,
    pub decode_us: u64,
    pub layout_us: u64,
    pub diff_us: u64,
    pub mount_us: u64,
}

impl FrameTiming {
    pub fn total_us(&self) -> u64 {
        self.reconcile_us
            + self.encode_us
            + self.decode_us
            + self.layout_us
            + self.diff_us
            + self.mount_us
    }
}

pub struct FrameProfiler {
    capacity: usize,
    frames: VecDeque<FrameTiming>,
}

impl FrameProfiler {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            frames: VecDeque::with_capacity(capacity.max(1)),
        }
    }

    pub fn record(&mut self, timing: FrameTiming) {
        if self.frames.len() == self.capacity {
            self.frames.pop_front();
        }
        self.frames.push_back(timing);
    }

    pub fn frames(&self) -> &VecDeque<FrameTiming> {
        &self.frames
    }

    pub fn p50_total_us(&self) -> u64 {
        self.percentile(0.50)
    }

    pub fn p99_total_us(&self) -> u64 {
        self.percentile(0.99)
    }

    fn percentile(&self, p: f64) -> u64 {
        if self.frames.is_empty() {
            return 0;
        }
        let mut totals: Vec<u64> = self.frames.iter().map(FrameTiming::total_us).collect();
        totals.sort_unstable();
        let idx = ((totals.len() - 1) as f64 * p).round() as usize;
        totals[idx.min(totals.len() - 1)]
    }

    /// Exports recorded frames in Chrome Trace Event Format (loadable directly in Perfetto UI).
    pub fn to_chrome_trace_json(&self) -> String {
        #[derive(Serialize)]
        struct Event<'a> {
            name: &'a str,
            cat: &'a str,
            ph: &'a str,
            ts: u64,
            dur: u64,
            pid: u32,
            tid: u32,
        }

        let mut events = Vec::with_capacity(self.frames.len() * 6);
        let mut cursor_ts = 0u64;
        for f in &self.frames {
            let stages = [
                ("rust.reconcile", f.reconcile_us, 1),
                ("abi.encode", f.encode_us, 1),
                ("backend.decode", f.decode_us, 2),
                ("backend.layout", f.layout_us, 2),
                ("backend.diff", f.diff_us, 2),
                ("backend.mount", f.mount_us, 3),
            ];
            for (name, dur, tid) in stages {
                events.push(Event {
                    name,
                    cat: "nve.frame",
                    ph: "X",
                    ts: cursor_ts,
                    dur,
                    pid: 1,
                    tid,
                });
                cursor_ts += dur.max(1);
            }
            cursor_ts += 100;
        }
        serde_json::to_string_pretty(&events).unwrap_or_else(|_| "[]".into())
    }
}
