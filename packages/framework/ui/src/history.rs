//! Bounded, transactional, burst-coalescing [`UndoRedoStack<T>`] for editors, forms, canvas
//! tools, and multi-step UI mutations.

use std::collections::VecDeque;

#[derive(Debug, Clone, PartialEq, Eq)]
struct CoalesceMeta {
    key: String,
    timestamp_ms: u64,
}

#[derive(Debug, Clone)]
struct GroupFrame<T> {
    snapshot: T,
    had_changes: bool,
}

/// Generic bounded undo/redo history stack with:
/// - Memory-safe eviction of the oldest entries once `max_entries` is exceeded.
/// - Burst coalescing (`push_coalesced`) collapsing rapid edits with the same key inside a time
///   window into a single undoable step.
/// - Nested transaction grouping (`begin_group` / `commit_group` / `abort_group`) with full
///   rollback on abort.
#[derive(Debug, Clone)]
pub struct UndoRedoStack<T> {
    entries: VecDeque<T>,
    cursor: usize,
    max_entries: usize,
    last_coalesce: Option<CoalesceMeta>,
    groups: Vec<GroupFrame<T>>,
    pending_group_state: Option<T>,
}

impl<T: Clone> UndoRedoStack<T> {
    /// Creates a new stack initialized with `initial` as the baseline state and capped at
    /// `max_entries` total entries (clamped to at least `1`).
    pub fn new(initial: T, max_entries: usize) -> Self {
        let cap = max_entries.max(1);
        let mut entries = VecDeque::with_capacity(cap.min(64));
        entries.push_back(initial);
        Self {
            entries,
            cursor: 0,
            max_entries: cap,
            last_coalesce: None,
            groups: Vec::new(),
            pending_group_state: None,
        }
    }

    /// Maximum number of history entries retained before oldest entries are evicted.
    #[inline]
    pub fn max_entries(&self) -> usize {
        self.max_entries
    }

    /// Total number of committed entries currently stored in the history timeline.
    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` if no entries exist (always `false` because a baseline state is always held).
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Number of undo steps currently available.
    #[inline]
    pub fn undo_count(&self) -> usize {
        self.cursor
    }

    /// Number of redo steps currently available.
    #[inline]
    pub fn redo_count(&self) -> usize {
        self.entries.len().saturating_sub(self.cursor + 1)
    }

    /// Returns `true` when an undo step can be performed (disabled while inside an open group).
    #[inline]
    pub fn can_undo(&self) -> bool {
        self.groups.is_empty() && self.cursor > 0
    }

    /// Returns `true` when a redo step can be performed (disabled while inside an open group).
    #[inline]
    pub fn can_redo(&self) -> bool {
        self.groups.is_empty() && self.cursor + 1 < self.entries.len()
    }

    /// Returns `true` when one or more transaction groups are currently open.
    #[inline]
    pub fn in_group(&self) -> bool {
        !self.groups.is_empty()
    }

    /// Current nesting depth of open transaction groups (`0` when outside any group).
    #[inline]
    pub fn group_depth(&self) -> usize {
        self.groups.len()
    }

    /// Returns the active state (the live uncommitted state while inside a group, or the entry at
    /// the current history cursor otherwise).
    #[inline]
    pub fn current(&self) -> &T {
        if let Some(ref pending) = self.pending_group_state {
            pending
        } else {
            &self.entries[self.cursor]
        }
    }

    /// Pushes a new committed state, clearing any redo branch and evicting the oldest entry if
    /// `max_entries` is exceeded. Inside an open transaction group, updates the uncommitted group
    /// working state instead.
    pub fn push(&mut self, state: T) {
        if let Some(frame) = self.groups.last_mut() {
            self.pending_group_state = Some(state);
            frame.had_changes = true;
            return;
        }
        self.last_coalesce = None;
        self.commit_entry(state);
    }

    /// Pushes a state that coalesces into the current history entry when:
    /// - No transaction group is open,
    /// - The previous push was also coalesced with the exact same `coalesce_key`,
    /// - `timestamp_ms >= prev_timestamp_ms` and `timestamp_ms - prev_timestamp_ms <= window_ms`,
    /// - The current cursor is past the baseline (`cursor > 0`) with no redo branch.
    ///
    /// Otherwise starts a fresh history entry with the given `coalesce_key`.
    pub fn push_coalesced(
        &mut self,
        state: T,
        coalesce_key: impl Into<String>,
        timestamp_ms: u64,
        window_ms: u64,
    ) {
        if !self.groups.is_empty() {
            self.push(state);
            return;
        }

        let key = coalesce_key.into();
        let can_coalesce = self.cursor > 0
            && self.cursor + 1 == self.entries.len()
            && self.last_coalesce.as_ref().is_some_and(|meta| {
                meta.key == key
                    && timestamp_ms >= meta.timestamp_ms
                    && timestamp_ms - meta.timestamp_ms <= window_ms
            });

        if can_coalesce {
            self.entries[self.cursor] = state;
            if let Some(ref mut meta) = self.last_coalesce {
                meta.timestamp_ms = timestamp_ms;
            }
        } else {
            self.commit_entry(state);
            self.last_coalesce = Some(CoalesceMeta { key, timestamp_ms });
        }
    }

    /// Begins a transaction group (supports arbitrary nesting). All `push` calls until the
    /// matching outermost [`Self::commit_group`] collapse into a single undoable step.
    pub fn begin_group(&mut self) {
        self.last_coalesce = None;
        let snapshot = self.current().clone();
        self.groups.push(GroupFrame {
            snapshot,
            had_changes: false,
        });
    }

    /// Commits the innermost transaction group. Returns `false` if no group is open.
    ///
    /// When committing a nested group, its changes merge into the enclosing parent group.
    /// When committing the outermost group, the final state is pushed as a single history entry
    /// (provided at least one push occurred inside the group).
    pub fn commit_group(&mut self) -> bool {
        let Some(frame) = self.groups.pop() else {
            return false;
        };
        if let Some(parent) = self.groups.last_mut() {
            parent.had_changes |= frame.had_changes;
            return true;
        }
        let pending = self.pending_group_state.take();
        if frame.had_changes
            && let Some(final_state) = pending
        {
            self.last_coalesce = None;
            self.commit_entry(final_state);
        }
        true
    }

    /// Aborts the innermost transaction group and rolls the working state back to the snapshot
    /// captured when that group began. Returns `false` if no group is open.
    pub fn abort_group(&mut self) -> bool {
        let Some(frame) = self.groups.pop() else {
            return false;
        };
        if self.groups.is_empty() {
            self.pending_group_state = None;
        } else {
            self.pending_group_state = Some(frame.snapshot);
        }
        true
    }

    /// Moves one step backward in history and returns the restored state, or `None` if no undo
    /// step is available (or a transaction group is open).
    pub fn undo(&mut self) -> Option<&T> {
        if !self.can_undo() {
            return None;
        }
        self.cursor -= 1;
        self.last_coalesce = None;
        self.entries.get(self.cursor)
    }

    /// Moves one step forward in history and returns the restored state, or `None` if no redo
    /// step is available (or a transaction group is open).
    pub fn redo(&mut self) -> Option<&T> {
        if !self.can_redo() {
            return None;
        }
        self.cursor += 1;
        self.last_coalesce = None;
        self.entries.get(self.cursor)
    }

    /// Resets the stack to a single baseline `state`, discarding all history and open groups.
    pub fn clear(&mut self, state: T) {
        self.entries.clear();
        self.entries.push_back(state);
        self.cursor = 0;
        self.last_coalesce = None;
        self.groups.clear();
        self.pending_group_state = None;
    }

    fn commit_entry(&mut self, state: T) {
        // Discard any redo tail strictly after `cursor`.
        let keep_len = self.cursor + 1;
        while self.entries.len() > keep_len {
            self.entries.pop_back();
        }
        self.entries.push_back(state);
        self.cursor = self.entries.len() - 1;

        // Evict oldest entries when exceeding max_entries.
        while self.entries.len() > self.max_entries {
            self.entries.pop_front();
            self.cursor = self.cursor.saturating_sub(1);
        }
    }
}
