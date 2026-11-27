//! Thread-safe in-memory store backing `examples/fullstack-issues/server`.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use nve_example_issues_api::{
    AddCommentInput, ApiError, CloseIssueInput, Comment, CommentId, CreateIssueInput, Issue,
    IssueFilter, IssueId, IssueState, RenameIssueInput, WorkspaceStats,
};
use nve_rpc::FieldErrors;
use tokio::sync::watch;

struct StoreData {
    issues: BTreeMap<IssueId, Issue>,
    comments: BTreeMap<IssueId, Vec<Comment>>,
    next_issue_id: u64,
    next_comment_id: u64,
}

struct StreamControl {
    stats_delay_ms: AtomicU64,
    comments_delay_ms: AtomicU64,
    comments_gate_tx: watch::Sender<bool>,
    /// Retained receiver so the `watch` channel never closes when zero requests are waiting.
    _comments_gate_rx: watch::Receiver<bool>,
    comments_waiting: AtomicUsize,
}

impl StreamControl {
    fn new() -> Self {
        let (comments_gate_tx, _comments_gate_rx) = watch::channel(true);
        Self {
            stats_delay_ms: AtomicU64::new(0),
            comments_delay_ms: AtomicU64::new(0),
            comments_gate_tx,
            _comments_gate_rx,
            comments_waiting: AtomicUsize::new(0),
        }
    }
}

struct WaitingCounterGuard<'a>(&'a AtomicUsize);

impl Drop for WaitingCounterGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Clone)]
pub struct IssueStore {
    inner: Arc<RwLock<StoreData>>,
    stream_control: Arc<StreamControl>,
}

impl Default for IssueStore {
    fn default() -> Self {
        Self::seeded()
    }
}

impl IssueStore {
    pub fn seeded() -> Self {
        let mut issues = BTreeMap::new();
        let mut comments = BTreeMap::new();

        issues.insert(
            IssueId(1),
            Issue {
                id: IssueId(1),
                title: "Wire minicbor RPC over Hyper + tower::Service".into(),
                state: IssueState::InProgress,
                priority: Some(1),
                comment_count: 1,
            },
        );
        comments.insert(
            IssueId(1),
            vec![Comment {
                id: CommentId(1),
                issue_id: IssueId(1),
                author: "alice".into(),
                body: "Zero-envelope HTTP + Nve-Invalidate header is live.".into(),
            }],
        );

        issues.insert(
            IssueId(2),
            Issue {
                id: IssueId(2),
                title: "Add H0 preloaded SSR + fail-closed hydration".into(),
                state: IssueState::Open,
                priority: Some(2),
                comment_count: 0,
            },
        );

        issues.insert(
            IssueId(3),
            Issue {
                id: IssueId(3),
                title: "Verify wasm32-wasip2 SSR under wasmtime serve".into(),
                state: IssueState::Closed,
                priority: None,
                comment_count: 0,
            },
        );

        Self {
            inner: Arc::new(RwLock::new(StoreData {
                issues,
                comments,
                next_issue_id: 4,
                next_comment_id: 2,
            })),
            stream_control: Arc::new(StreamControl::new()),
        }
    }

    /// Configures an optional artificial delay on [`nve_example_issues_api::GetWorkspaceStats`] for streaming SSR tests.
    pub fn set_stats_delay_ms(&self, delay_ms: u64) {
        self.stream_control
            .stats_delay_ms
            .store(delay_ms, Ordering::Release);
    }

    /// Configures an optional artificial delay on [`nve_example_issues_api::ListComments`] for streaming SSR tests.
    pub fn set_comments_delay_ms(&self, delay_ms: u64) {
        self.stream_control
            .comments_delay_ms
            .store(delay_ms, Ordering::Release);
    }

    /// Opens (`true`) or closes (`false`) the deterministic async barrier before [`nve_example_issues_api::ListComments`].
    pub fn set_comments_gate_open(&self, open: bool) {
        let _ = self.stream_control.comments_gate_tx.send_replace(open);
    }

    /// Returns the number of [`nve_example_issues_api::ListComments`] RPC calls currently parked on the closed barrier.
    #[must_use]
    pub fn comments_waiting(&self) -> usize {
        self.stream_control.comments_waiting.load(Ordering::Acquire)
    }

    /// Awaits any configured delay before resolving [`nve_example_issues_api::GetWorkspaceStats`].
    pub async fn wait_stats_hook(&self) {
        let delay_ms = self.stream_control.stats_delay_ms.load(Ordering::Acquire);
        if delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        }
    }

    /// Awaits the deterministic barrier (when closed) and optional delay before [`nve_example_issues_api::ListComments`].
    pub async fn wait_comments_hook(&self) {
        if !*self.stream_control.comments_gate_tx.borrow() {
            let mut rx = self.stream_control.comments_gate_tx.subscribe();
            self.stream_control
                .comments_waiting
                .fetch_add(1, Ordering::AcqRel);
            let _guard = WaitingCounterGuard(&self.stream_control.comments_waiting);
            let _ = rx.wait_for(|&open| open).await;
        }
        let delay_ms = self
            .stream_control
            .comments_delay_ms
            .load(Ordering::Acquire);
        if delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        }
    }

    /// Computes live aggregate issue and comment counters across the workspace.
    #[must_use]
    pub fn stats(&self) -> WorkspaceStats {
        let guard = self.inner.read().expect("issue store lock poisoned");
        let mut out = WorkspaceStats::default();
        for issue in guard.issues.values() {
            out.total = out.total.saturating_add(1);
            match issue.state {
                IssueState::Open => out.open = out.open.saturating_add(1),
                IssueState::InProgress => out.in_progress = out.in_progress.saturating_add(1),
                IssueState::Closed => out.closed = out.closed.saturating_add(1),
                IssueState::Unknown(_) => {}
            }
            out.total_comments = out.total_comments.saturating_add(issue.comment_count);
        }
        out
    }

    pub fn list(&self, filter: &IssueFilter) -> Vec<Issue> {
        let query_lower = filter
            .query
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_lowercase);
        let guard = self.inner.read().expect("issue store lock poisoned");
        guard
            .issues
            .values()
            .filter(|issue| {
                if let Some(state) = filter.state
                    && issue.state != state
                {
                    return false;
                }
                if let Some(q_lower) = query_lower.as_deref()
                    && !issue.title.to_lowercase().contains(q_lower)
                {
                    return false;
                }
                true
            })
            .cloned()
            .collect()
    }

    pub fn get(&self, id: IssueId) -> Result<Issue, ApiError> {
        let guard = self.inner.read().expect("issue store lock poisoned");
        guard
            .issues
            .get(&id)
            .cloned()
            .ok_or(ApiError::NotFound(id))
    }

    pub fn list_comments(&self, id: IssueId) -> Result<Vec<Comment>, ApiError> {
        let guard = self.inner.read().expect("issue store lock poisoned");
        if !guard.issues.contains_key(&id) {
            return Err(ApiError::NotFound(id));
        }
        Ok(guard.comments.get(&id).cloned().unwrap_or_default())
    }

    pub fn create(&self, input: &CreateIssueInput) -> Result<Issue, ApiError> {
        input.validate().map_err(ApiError::Validation)?;
        let trimmed = input.title.trim();
        let trimmed_lower = trimmed.to_lowercase();
        let mut guard = self.inner.write().expect("issue store lock poisoned");
        if guard
            .issues
            .values()
            .any(|existing| existing.title.to_lowercase() == trimmed_lower)
        {
            let mut errs = FieldErrors::new();
            errs.push("title", "an issue with this title already exists");
            return Err(ApiError::Validation(errs));
        }
        let id = IssueId(guard.next_issue_id);
        guard.next_issue_id += 1;
        let issue = Issue {
            id,
            title: trimmed.to_owned(),
            state: IssueState::Open,
            priority: input.priority,
            comment_count: 0,
        };
        guard.issues.insert(id, issue.clone());
        Ok(issue)
    }

    pub fn close(&self, input: &CloseIssueInput) -> Result<Issue, ApiError> {
        let mut guard = self.inner.write().expect("issue store lock poisoned");
        let issue = guard
            .issues
            .get_mut(&input.id)
            .ok_or(ApiError::NotFound(input.id))?;
        issue.state = IssueState::Closed;
        Ok(issue.clone())
    }

    pub fn rename(&self, input: &RenameIssueInput) -> Result<Issue, ApiError> {
        input.validate().map_err(ApiError::Validation)?;
        let trimmed = input.title.trim();
        let trimmed_lower = trimmed.to_lowercase();
        let mut guard = self.inner.write().expect("issue store lock poisoned");
        if !guard.issues.contains_key(&input.id) {
            return Err(ApiError::NotFound(input.id));
        }
        if guard
            .issues
            .values()
            .any(|existing| existing.id != input.id && existing.title.to_lowercase() == trimmed_lower)
        {
            let mut errs = FieldErrors::new();
            errs.push("title", "an issue with this title already exists");
            return Err(ApiError::Validation(errs));
        }
        let issue = guard
            .issues
            .get_mut(&input.id)
            .ok_or(ApiError::NotFound(input.id))?;
        issue.title = trimmed.to_owned();
        Ok(issue.clone())
    }

    pub fn add_comment(&self, author: &str, input: &AddCommentInput) -> Result<Comment, ApiError> {
        input.validate().map_err(ApiError::Validation)?;
        let mut guard = self.inner.write().expect("issue store lock poisoned");
        if !guard.issues.contains_key(&input.issue_id) {
            return Err(ApiError::NotFound(input.issue_id));
        }
        let cid = CommentId(guard.next_comment_id);
        guard.next_comment_id += 1;
        let comment = Comment {
            id: cid,
            issue_id: input.issue_id,
            author: author.to_owned(),
            body: input.body.trim().to_owned(),
        };
        guard
            .comments
            .entry(input.issue_id)
            .or_default()
            .push(comment.clone());
        if let Some(issue) = guard.issues.get_mut(&input.issue_id) {
            issue.comment_count = issue.comment_count.saturating_add(1);
        }
        Ok(comment)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_reflects_seeded_and_mutated_counts() {
        let store = IssueStore::seeded();
        assert_eq!(
            store.stats(),
            WorkspaceStats {
                total: 3,
                open: 1,
                in_progress: 1,
                closed: 1,
                total_comments: 1,
            }
        );
        let created = store
            .create(&CreateIssueInput {
                title: "Stats test".into(),
                priority: Some(2),
            })
            .unwrap();
        store
            .add_comment(
                "bob",
                &AddCommentInput {
                    issue_id: created.id,
                    body: "hello".into(),
                },
            )
            .unwrap();
        assert_eq!(
            store.stats(),
            WorkspaceStats {
                total: 4,
                open: 2,
                in_progress: 1,
                closed: 1,
                total_comments: 2,
            }
        );
    }

    #[tokio::test]
    async fn comments_gate_blocks_releases_and_decrements_on_cancel() {
        let store = IssueStore::seeded();
        store.set_comments_gate_open(false);

        let store_cancel = store.clone();
        let h_cancel = tokio::spawn(async move {
            store_cancel.wait_comments_hook().await;
        });
        let store_release = store.clone();
        let h_release = tokio::spawn(async move {
            store_release.wait_comments_hook().await;
        });

        while store.comments_waiting() < 2 {
            tokio::task::yield_now().await;
        }
        h_cancel.abort();
        let _ = h_cancel.await;
        assert_eq!(store.comments_waiting(), 1);

        store.set_comments_gate_open(true);
        h_release.await.unwrap();
        assert_eq!(store.comments_waiting(), 0);
    }
}
