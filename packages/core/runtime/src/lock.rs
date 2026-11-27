//! Per-surface reader/writer lock supporting both safe Rust RAII guards and split FFI
//! `scene_acquire` / `scene_release` calls without self-referential lifetimes.

use std::sync::{Condvar, Mutex};
#[cfg(not(target_arch = "wasm32"))]
use std::time::{Duration, Instant};

#[derive(Debug, Default)]
struct LockState {
    /// Active `scene_acquire` / `acquire_scene` readers (released ONLY by `release_read`).
    scene_readers: u32,
    /// Active internal RAII readers (`metrics`, `layout_engine_name`), isolated from FFI
    /// `scene_release` so a spurious release can never corrupt internal borrows.
    internal_readers: u32,
    /// Exclusive guard for `LaidOutScene::hit_scratch` `Cell` (safe alongside `scene_readers`
    /// and `internal_readers`, never alongside another `hit_test` or a writer).
    hit_busy: bool,
    writer: bool,
}

impl LockState {
    #[inline]
    fn has_any_reader(&self) -> bool {
        self.scene_readers > 0 || self.internal_readers > 0 || self.hit_busy
    }
}

/// Non-reentrant-write / multi-reader lock guarding zero-copy `NveRuntimeSceneView` borrows across
/// Rust and C FFI boundaries, plus exclusive serialization for `hit_test`'s internal `Cell` scratch.
pub struct SceneLock {
    state: Mutex<LockState>,
    cv: Condvar,
}

impl Default for SceneLock {
    fn default() -> Self {
        Self::new()
    }
}

impl SceneLock {
    pub const fn new() -> Self {
        Self {
            state: Mutex::new(LockState {
                scene_readers: 0,
                internal_readers: 0,
                hit_busy: false,
                writer: false,
            }),
            cv: Condvar::new(),
        }
    }

    /// Attempts to acquire a shared scene reader lock without blocking (`scene_acquire`).
    pub fn try_acquire_read(&self) -> bool {
        let mut g = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if g.writer || g.scene_readers == u32::MAX {
            return false;
        }
        g.scene_readers += 1;
        true
    }

    /// Releases one scene reader lock (`scene_release`); returns `false` if none was held.
    pub fn release_read(&self) -> bool {
        let mut g = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if g.scene_readers == 0 {
            return false;
        }
        g.scene_readers -= 1;
        if !g.has_any_reader() {
            self.cv.notify_all();
        }
        true
    }

    /// Attempts to acquire an internal RAII reader lock (`metrics`, `layout_engine_name`).
    fn try_acquire_internal_read(&self) -> bool {
        let mut g = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if g.writer || g.internal_readers == u32::MAX {
            return false;
        }
        g.internal_readers += 1;
        true
    }

    fn release_internal_read(&self) {
        let mut g = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if g.internal_readers > 0 {
            g.internal_readers -= 1;
        }
        if !g.has_any_reader() {
            self.cv.notify_all();
        }
    }

    /// Attempts to acquire a hit-test reader lock guarding `hit_test`'s `Cell` scratch buffer.
    pub fn try_acquire_hit_read(&self) -> bool {
        let mut g = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if g.writer || g.hit_busy {
            return false;
        }
        g.hit_busy = true;
        true
    }

    /// Releases the hit-test reader lock.
    pub fn release_hit_read(&self) {
        let mut g = self.state.lock().unwrap_or_else(|e| e.into_inner());
        g.hit_busy = false;
        if !g.has_any_reader() {
            self.cv.notify_all();
        }
    }

    /// Attempts to acquire exclusive write access without blocking (fails if any reader or writer
    /// holds the lock — prevents mutating borrowed scene slabs while a host holds `scene_acquire`).
    pub fn try_acquire_write(&self) -> bool {
        let mut g = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if g.writer || g.has_any_reader() {
            return false;
        }
        g.writer = true;
        true
    }

    /// Waits up to `timeout` for active readers/writers to drain and acquires exclusive write
    /// access (used by FFI `destroy` so a leaked `scene_acquire` cannot deadlock indefinitely).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn try_acquire_write_timeout(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut g = self.state.lock().unwrap_or_else(|e| e.into_inner());
        while g.writer || g.has_any_reader() {
            let Some(rem) = deadline.checked_duration_since(Instant::now()) else {
                return false;
            };
            let (next, res) = self
                .cv
                .wait_timeout(g, rem)
                .unwrap_or_else(|e| e.into_inner());
            g = next;
            if res.timed_out() && (g.writer || g.has_any_reader()) {
                return false;
            }
        }
        g.writer = true;
        true
    }

    /// Single-threaded WASM fallback for `destroy` write lock acquisition.
    #[cfg(target_arch = "wasm32")]
    pub fn try_acquire_write_timeout(&self, _timeout: core::time::Duration) -> bool {
        self.try_acquire_write()
    }

    /// Releases exclusive write access.
    pub fn release_write(&self) {
        let mut g = self.state.lock().unwrap_or_else(|e| e.into_inner());
        g.writer = false;
        self.cv.notify_all();
    }

    pub fn write_guard(&self) -> Option<WriteGuard<'_>> {
        self.try_acquire_write().then_some(WriteGuard(self))
    }

    pub fn read_guard(&self) -> Option<ReadGuard<'_>> {
        self.try_acquire_internal_read().then_some(ReadGuard(self))
    }

    pub fn hit_read_guard(&self) -> Option<HitReadGuard<'_>> {
        self.try_acquire_hit_read().then_some(HitReadGuard(self))
    }
}

pub struct WriteGuard<'a>(&'a SceneLock);
impl Drop for WriteGuard<'_> {
    fn drop(&mut self) {
        self.0.release_write();
    }
}

pub struct ReadGuard<'a>(&'a SceneLock);
impl Drop for ReadGuard<'_> {
    fn drop(&mut self) {
        self.0.release_internal_read();
    }
}

pub struct HitReadGuard<'a>(&'a SceneLock);
impl Drop for HitReadGuard<'_> {
    fn drop(&mut self) {
        self.0.release_hit_read();
    }
}
