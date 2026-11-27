//! Bounded deterministic time-travel history scrubber and read-only derived queries
//! for [`WorkspaceState`].

use crate::ui_palette::fuzzy_score;

use super::models::{
    BoardFilter, TaskItem, TaskStatus, WorkspaceSnapshot, MAX_HISTORY, MAX_TELEMETRY_SAMPLES,
};
use super::WorkspaceState;

impl WorkspaceState {
    /// Commits a deterministic state transition onto the bounded time-travel history stack.
    pub(super) fn commit_action<F>(&mut self, label: impl Into<String>, mutate: F)
    where
        F: FnOnce(&mut WorkspaceSnapshot),
    {
        // Truncate any speculative redo future if we branched from an older cursor.
        if self.cursor + 1 < self.history.len() {
            self.history.truncate(self.cursor + 1);
        }
        mutate(&mut self.current);
        self.current.action_label = label.into();
        // Record a deterministic mutation-cost timing sample for the live SkiaCanvas sparkline.
        let sample_us = 155u32
            + ((self.current.tasks.len() as u32) * 11
                + (self.current.stress_rows as u32) * 7
                + (self.frame_count as u32 % 9) * 19)
                % 220;
        self.current.telemetry_samples_us.push(sample_us);
        if self.current.telemetry_samples_us.len() > MAX_TELEMETRY_SAMPLES {
            let extra = self.current.telemetry_samples_us.len() - MAX_TELEMETRY_SAMPLES;
            self.current.telemetry_samples_us.drain(0..extra);
        }
        self.history.push(self.current.clone());
        if self.history.len() > MAX_HISTORY {
            let overflow = self.history.len() - MAX_HISTORY;
            self.history.drain(0..overflow);
        }
        self.cursor = self.history.len().saturating_sub(1);
        self.frame_count = self.frame_count.saturating_add(1);
    }

    pub fn scrub_first(&mut self) {
        if self.cursor == 0 || self.history.is_empty() {
            return;
        }
        self.cursor = 0;
        self.current = self.history[0].clone();
        self.sync_derived_after_scrub();
        self.frame_count = self.frame_count.saturating_add(1);
    }

    pub fn scrub_undo(&mut self) {
        if self.cursor == 0 || self.history.is_empty() {
            return;
        }
        self.cursor -= 1;
        self.current = self.history[self.cursor].clone();
        self.sync_derived_after_scrub();
        self.frame_count = self.frame_count.saturating_add(1);
    }

    pub fn scrub_redo(&mut self) {
        if self.cursor + 1 >= self.history.len() {
            return;
        }
        self.cursor += 1;
        self.current = self.history[self.cursor].clone();
        self.sync_derived_after_scrub();
        self.frame_count = self.frame_count.saturating_add(1);
    }

    pub fn scrub_live(&mut self) {
        let last = self.history.len().saturating_sub(1);
        if self.cursor == last || self.history.is_empty() {
            return;
        }
        self.cursor = last;
        self.current = self.history[last].clone();
        self.sync_derived_after_scrub();
        self.frame_count = self.frame_count.saturating_add(1);
    }

    pub fn scrub_to(&mut self, index: usize) {
        if self.history.is_empty() {
            return;
        }
        let target = index.min(self.history.len() - 1);
        if self.cursor == target {
            return;
        }
        self.cursor = target;
        self.current = self.history[target].clone();
        self.sync_derived_after_scrub();
        self.frame_count = self.frame_count.saturating_add(1);
    }

    pub fn can_undo(&self) -> bool {
        self.cursor > 0
    }

    pub fn can_redo(&self) -> bool {
        self.cursor + 1 < self.history.len()
    }

    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    pub fn snapshot_index_1based(&self) -> usize {
        self.cursor + 1
    }

    pub fn active_task_count(&self) -> usize {
        self.current
            .tasks
            .iter()
            .filter(|t| t.status != TaskStatus::Done)
            .count()
    }

    pub fn done_task_count(&self) -> usize {
        self.current
            .tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Done)
            .count()
    }

    pub fn total_message_count(&self) -> usize {
        self.current.eng_messages.len() + self.current.design_messages.len()
    }

}

/// Board tasks visible under `filter` and the fuzzy `query` (key, title, assignee, tag).
pub fn filter_tasks<'a>(
    tasks: &'a [TaskItem],
    filter: BoardFilter,
    query: &'a str,
) -> impl Iterator<Item = &'a TaskItem> + 'a {
    let search = query.trim();
    tasks.iter().filter(move |t| {
        let status_ok = match filter {
            BoardFilter::All => true,
            BoardFilter::Active => t.status != TaskStatus::Done,
            BoardFilter::Done => t.status == TaskStatus::Done,
        };
        status_ok
            && (search.is_empty() || {
                let hay = format!("{} {} {} {}", t.key, t.title, t.assignee, t.tag);
                fuzzy_score(search, &hay).is_some()
            })
    })
}
