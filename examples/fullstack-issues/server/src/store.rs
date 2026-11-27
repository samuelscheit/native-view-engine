//! Thread-safe in-memory store backing `examples/fullstack-issues/server`.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use nve_example_issues_api::{
    AddCommentInput, ApiError, CloseIssueInput, Comment, CommentId, CreateIssueInput, Issue,
    IssueFilter, IssueId, IssueState, RenameIssueInput,
};
use nve_rpc::FieldErrors;

struct StoreData {
    issues: BTreeMap<IssueId, Issue>,
    comments: BTreeMap<IssueId, Vec<Comment>>,
    next_issue_id: u64,
    next_comment_id: u64,
}

#[derive(Clone)]
pub struct IssueStore {
    inner: Arc<RwLock<StoreData>>,
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
        }
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
