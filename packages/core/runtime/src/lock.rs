//! Per-surface reader/writer lock supporting both safe Rust RAII guards and split FFI
//! `scene_acquire` / `scene_release` calls without self-referential lifetimes.

use std::sync::{Condvar, Mutex};

#[derive(Debug, Default)]
struct LockState {
    readers: u32,
    hit_busy: bool,
    writer: bool,
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
                readers: 0,
                hit_busy: false,
                writer: false,
            }),
            cv: Condvar::new(),
        }
    }

    /// Attempts to acquire a shared reader lock without blocking.
    /// Returns `false` if a writer currently holds exclusive access or reader count saturated.
    pub fn try_acquire_read(&self) -> bool {
        let mut g = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if g.writer || g.readers == u32::MAX {
            return false;
        }
        g.readers += 1;
        true
    }

    /// Attempts to acquire a shared reader lock that also exclusively guards `hit_test`'s internal
    /// `Cell` scratch buffer (safe to overlap with `scene_acquire`, never with another `hit_test`
    /// or writer).
    pub fn try_acquire_hit_read(&self) -> bool {
        let mut g = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if g.writer || g.hit_busy || g.readers == u32::MAX {
            return false;
        }
        g.readers += 1;
        g.hit_busy = true;
        true
    }

    /// Releases one hit-test reader lock.
    pub fn release_hit_read(&self) {
        let mut g = self.state.lock().unwrap_or_else(|e| e.into_inner());
        g.hit_busy = false;
        if g.readers > 0 {
            g.readers -= 1;
        }
        if g.readers == 0 {
            self.cv.notify_all();
        }
    }

    /// Releases one shared reader lock; returns `false` if no reader lock was held.
    pub fn release_read(&self) -> bool {
        let mut g = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if g.readers == 0 {
            return false;
        }
        g.readers -= 1;
        if g.readers == 0 {
            self.cv.notify_all();
        }
        true
    }

    /// Attempts to acquire exclusive write access without blocking (fails if any reader or writer
    /// holds the lock — prevents mutating borrowed scene slabs while a host holds `scene_acquire`).
    pub fn try_acquire_write(&self) -> bool {
        let mut g = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if g.writer || g.readers > 0 || g.hit_busy {
            return false;
        }
        g.writer = true;
        true
    }

    /// Blocks until all active readers/writers finish and acquires exclusive write access (used on
    /// `destroy`).
    pub fn acquire_write_blocking(&self) {
        let mut g = self.state.lock().unwrap_or_else(|e| e.into_inner());
        while g.writer || g.readers > 0 || g.hit_busy {
            g = self.cv.wait(g).unwrap_or_else(|e| e.into_inner());
        }
        g.writer = true;
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
        self.try_acquire_read().then_some(ReadGuard(self))
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
        let _ = self.0.release_read();
    }
}

pub struct HitReadGuard<'a>(&'a SceneLock);
impl Drop for HitReadGuard<'_> {
    fn drop(&mut self) {
        self.0.release_hit_read();
    }
}
