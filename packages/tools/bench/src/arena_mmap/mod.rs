//! Zero-copy memory layout & allocator prototypes for NVE app-state snapshotting
//! and per-frame UI tree allocation benchmarks.
//!
//! - **Workload A** (Studio Workspace app-state):
//!   - [`baseline_codec`]: JSON (`serde_json`-style) owned encode/decode baseline.
//!   - [`reloc`]: `NVERELOC` / `RelocSlice<T>` aligned zero-copy buffer (`Full`, `HeaderOnly`,
//!     and `Unchecked` mmap load modes) over production `nve_engine::reloc`.
//!   - [`workspace_arena`]: Studio Workspace adapter directly over production
//!     [`nve_engine::MmapOffsetArena`] with `TrustedUtf8Str`, double-buffered root commits,
//!     and `msync`.
//! - **Workload B** (Per-frame UI tree & prop allocation):
//!   - [`frame_arena`]: UI tree benchmark harness over production [`nve_engine::FrameArena`] vs
//!     standard `Vec` + `String` heap allocation baseline.

pub mod baseline_codec;
pub mod frame_arena;
pub mod reloc;
pub mod workspace_arena;

#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

/// Owned Studio Workspace task/message record used as the canonical input and owned baseline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedTask {
    pub id: u64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub priority: u32,
    pub status: u32,
    pub assignee_id: u32,
    pub flags: u32,
    pub title: String,
    pub body: String,
    pub tag: String,
}

const TAGS: [&str; 8] = [
    "engine/abi",
    "host/skia",
    "host/web",
    "studio/ui",
    "perf/mmap",
    "docs/mdx",
    "sync/crdt",
    "ci/verify",
];

const VERBS: [&str; 8] = [
    "Eliminate per-frame heap churn in",
    "Verify zero-copy mmap layout for",
    "Audit alignment invariants across",
    "Benchmark incremental state sync in",
    "Consolidate render pass buffers for",
    "Profile cold-start frame0 boot on",
    "Validate relocatable string pool in",
    "Tighten cache-line packing inside",
];

const SUBSYSTEMS: [&str; 8] = [
    "Studio task timeline & inspector",
    "Skia surface command submission",
    "NVERELOC scene snapshot loader",
    "WebAssembly DOM diff dispatcher",
    "Dioxus reactive signal store",
    "Cross-host scene view bridge",
    "CRDT workspace replication log",
    "Frame-arena layout prop table",
];

/// Generates `count` deterministic, realistic Studio Workspace tasks/messages.
pub fn generate_workspace_tasks(count: usize) -> Vec<OwnedTask> {
    let mut tasks = Vec::with_capacity(count);
    let base_ts: i64 = 1_760_000_000_000;

    for i in 0..count {
        let idx = i as u64;
        let verb = VERBS[i % VERBS.len()];
        let sub = SUBSYSTEMS[(i / 3 + i * 5) % SUBSYSTEMS.len()];
        let tag = TAGS[(i * 7 + 1) % TAGS.len()];

        let title = format!("TASK-{:05}: {verb} {sub}", i + 1);
        let body = format!(
            "Workspace item #{i} targeting `{tag}`: measure cold-load latency, verify 8-byte \
             alignment, and track delta updates for revision r{} across desktop and mobile hosts.",
            (i * 13) % 997
        );

        tasks.push(OwnedTask {
            id: 100_000 + idx,
            created_at_ms: base_ts + (i as i64) * 1_500,
            updated_at_ms: base_ts + (i as i64) * 1_500 + ((i % 47) as i64) * 250,
            priority: (i % 4) as u32,
            status: (i % 5) as u32,
            assignee_id: 1 + (i % 64) as u32,
            flags: ((i as u32).wrapping_mul(2654435761)) & 0xFFFF,
            title,
            body,
            tag: tag.to_owned(),
        });
    }
    tasks
}

/// Generates `read_count` deterministic pseudo-random indices across `[0, record_count)`.
pub fn deterministic_read_indices(record_count: usize, read_count: usize) -> Vec<usize> {
    assert!(record_count > 0, "record_count must be > 0");
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut out = Vec::with_capacity(read_count);
    for _ in 0..read_count {
        // SplitMix64 step for uniform deterministic access across pages.
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        out.push((z as usize) % record_count);
    }
    out
}

/// Generates `mutation_count` distinct deterministic indices spread across `[0, record_count)`.
pub fn deterministic_mutation_indices(record_count: usize, mutation_count: usize) -> Vec<usize> {
    assert!(
        mutation_count <= record_count,
        "mutation_count must not exceed record_count"
    );
    if mutation_count == 0 {
        return Vec::new();
    }
    let stride = (record_count / mutation_count).max(1);
    (0..mutation_count)
        .map(|i| (i * stride + (i * 17) % stride) % record_count)
        .collect()
}

/// Computes an identical checksum over a task's scalar fields and strings for apples-to-apples
/// random-read benchmarks across `OwnedTask`, `RelocWorkspaceView`, and `MmapOffsetArena`.
#[inline]
pub fn fold_task_fields(
    acc: u64,
    id: u64,
    updated_at_ms: i64,
    priority: u32,
    status: u32,
    title: &str,
    tag: &str,
) -> u64 {
    let mut h = acc.wrapping_mul(0x100000001B3).wrapping_add(id);
    h ^= updated_at_ms as u64;
    h = h.wrapping_add(((priority as u64) << 32) | (status as u64));
    let tb = title.as_bytes();
    let gb = tag.as_bytes();
    h = h.wrapping_add(tb.len() as u64);
    if let Some(&first) = tb.first() {
        h ^= first as u64;
    }
    if let Some(&last) = tb.last() {
        h ^= (last as u64) << 8;
    }
    h = h.wrapping_add(gb.len() as u64);
    if let Some(&first) = gb.first() {
        h ^= (first as u64) << 16;
    }
    h
}

/// Evaluates the random-read workload directly over an owned `Vec<OwnedTask>`.
pub fn random_read_owned_tasks(tasks: &[OwnedTask], indices: &[usize]) -> u64 {
    let mut acc = 0u64;
    for &idx in indices {
        let t = &tasks[idx];
        acc = fold_task_fields(
            acc,
            t.id,
            t.updated_at_ms,
            t.priority,
            t.status,
            &t.title,
            &t.tag,
        );
    }
    acc
}

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Creates a unique temp file path scoped to the current process for tests and benchmarks.
pub fn unique_temp_path(stem: &str, ext: &str) -> PathBuf {
    let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "nve_bench_{stem}_{}_{seq}.{ext}",
        std::process::id()
    ))
}

/// RAII guard that removes a temporary benchmark/test file on drop.
pub struct TempFileGuard {
    path: PathBuf,
}

impl TempFileGuard {
    pub fn new(stem: &str, ext: &str) -> Self {
        Self {
            path: unique_temp_path(stem, ext),
        }
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
