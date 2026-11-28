//! History stack and modal overlay state machine (`RouterState<R>`).

use std::fmt;

use crate::guard::{guard_fn, GuardChain, GuardDecision, GuardOutcome, NavigationContext, NavigationGuard};
use crate::route::Routable;

/// Wire-compatible navigation transition kind (matches `NVN1` byte `[5]`).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, nve_codec::ReprEnum)]
pub enum NavigationAction {
    Snapshot = 0,
    Push = 1,
    Replace = 2,
    Pop = 3,
    PopToRoot = 4,
    SwitchTab = 5,
    DeepLink = 6,
    OpenModal = 7,
    DismissModal = 8,
}

/// Errors returned when a navigation transition is rejected or out of bounds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavigationError {
    /// A synchronous guard returned [`GuardDecision::Block`].
    Blocked,
    /// A guard redirect chain hit a cycle or exceeded the hop limit.
    RedirectCycle { hops: usize },
    /// Cannot pop when already at the root entry (`cursor == 0`).
    CannotPopRoot,
    /// `go(delta)` stepped outside `[0, history_len)`.
    HistoryOutOfBounds {
        cursor: usize,
        len: usize,
        delta: isize,
    },
    /// Attempted to dismiss a modal when the modal overlay stack is empty.
    NoActiveModal,
    /// Tab key/index/id did not match any registered tab.
    UnknownTab,
    /// Tab router was constructed with an empty tab list or unknown initial tab.
    InvalidTabConfig(&'static str),
}

impl fmt::Display for NavigationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Blocked => write!(f, "navigation blocked by guard"),
            Self::RedirectCycle { hops } => {
                write!(f, "navigation guard redirect cycle detected after {hops} hop(s)")
            }
            Self::CannotPopRoot => write!(f, "cannot pop root route entry"),
            Self::HistoryOutOfBounds { cursor, len, delta } => {
                write!(
                    f,
                    "history delta {delta} out of bounds for cursor {cursor} (len {len})"
                )
            }
            Self::NoActiveModal => write!(f, "no active modal to dismiss"),
            Self::UnknownTab => write!(f, "unknown tab key or index"),
            Self::InvalidTabConfig(msg) => write!(f, "invalid tab router configuration: {msg}"),
        }
    }
}

impl std::error::Error for NavigationError {}

/// One committed entry on a router history stack or modal overlay stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteEntry<R> {
    /// Monotonically increasing identifier unique within this router instance.
    pub entry_id: u64,
    /// Typed route payload.
    pub route: R,
}

/// Summary of a committed router transition, ready for `NVN1` channel broadcast or UI sync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavigationTransition<R> {
    pub action: NavigationAction,
    pub entry_id: u64,
    pub route: R,
    pub stack_depth: u32,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub is_modal: bool,
    pub active_tab_index: u32,
    pub active_tab_id: String,
    pub redirect_hops: usize,
}

/// Type-safe navigation history stack with cursor, forward history, isolated modal overlay stack,
/// and synchronous navigation guards.
#[derive(Debug, Clone)]
pub struct RouterState<R: Routable> {
    next_entry_id: u64,
    entries: Vec<RouteEntry<R>>,
    cursor: usize,
    modals: Vec<RouteEntry<R>>,
    guards: GuardChain<R>,
}

impl<R: Routable> RouterState<R> {
    /// Creates a new router initialized with `initial` as the root entry (`entry_id = 1`).
    pub fn new(initial: R) -> Self {
        Self::with_starting_id(initial, 1)
    }

    pub(crate) fn with_starting_id(initial: R, first_entry_id: u64) -> Self {
        let root = RouteEntry {
            entry_id: first_entry_id,
            route: initial,
        };
        Self {
            next_entry_id: first_entry_id.saturating_add(1),
            entries: vec![root],
            cursor: 0,
            modals: Vec::new(),
            guards: GuardChain::new(),
        }
    }

    pub(crate) fn alloc_id_from(&mut self, external_next_id: Option<&mut u64>) -> u64 {
        if let Some(counter) = external_next_id {
            let id = *counter;
            *counter = counter.saturating_add(1);
            self.next_entry_id = self.next_entry_id.max(*counter);
            id
        } else {
            let id = self.next_entry_id;
            self.next_entry_id = self.next_entry_id.saturating_add(1);
            id
        }
    }

    /// Sets the maximum redirect hops permitted for guards on this router.
    pub fn with_max_redirect_hops(mut self, max_hops: usize) -> Self {
        self.guards.set_max_redirect_hops(max_hops);
        self
    }

    /// Registers a synchronous [`NavigationGuard`] evaluated before every transition.
    pub fn add_guard(&mut self, guard: impl NavigationGuard<R> + 'static) {
        self.guards.add(guard);
    }

    /// Registers a synchronous closure guard evaluated before every transition.
    pub fn add_guard_fn<F>(&mut self, f: F)
    where
        F: Fn(&NavigationContext<'_, R>) -> GuardDecision<R> + Send + Sync + 'static,
    {
        self.guards.add(guard_fn(f));
    }

    /// Returns a reference to the guard chain.
    pub fn guards(&self) -> &GuardChain<R> {
        &self.guards
    }

    /// Returns a mutable reference to the guard chain.
    pub fn guards_mut(&mut self) -> &mut GuardChain<R> {
        &mut self.guards
    }

    /// Returns the active entry on the main navigation stack (ignoring modal overlays).
    #[inline]
    pub fn current(&self) -> &RouteEntry<R> {
        &self.entries[self.cursor]
    }

    /// Returns the active route on the main navigation stack.
    #[inline]
    pub fn current_route(&self) -> &R {
        &self.entries[self.cursor].route
    }

    /// Returns the top modal if one is open, otherwise the active stack entry.
    #[inline]
    pub fn top(&self) -> &RouteEntry<R> {
        self.modals.last().unwrap_or(&self.entries[self.cursor])
    }

    /// Returns the entire history slice (including forward entries after `cursor`).
    #[inline]
    pub fn entries(&self) -> &[RouteEntry<R>] {
        &self.entries
    }

    /// Zero-based index of the active entry within `entries()`.
    #[inline]
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Active stack depth from root through cursor (`cursor + 1`, always `>= 1`).
    #[inline]
    pub fn depth(&self) -> usize {
        self.cursor + 1
    }

    /// Total history entries including forward entries reachable via `go(+delta)`.
    #[inline]
    pub fn history_len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` when at least one back entry exists (`cursor > 0`).
    #[inline]
    pub fn can_go_back(&self) -> bool {
        self.cursor > 0
    }

    /// Returns `true` when at least one forward entry exists (`cursor + 1 < entries.len()`).
    #[inline]
    pub fn can_go_forward(&self) -> bool {
        self.cursor + 1 < self.entries.len()
    }

    /// Active modal overlay stack (bottom to top).
    #[inline]
    pub fn modals(&self) -> &[RouteEntry<R>] {
        &self.modals
    }

    /// Top-most active modal, if any.
    #[inline]
    pub fn active_modal(&self) -> Option<&RouteEntry<R>> {
        self.modals.last()
    }

    /// Returns `true` if at least one modal overlay is open.
    #[inline]
    pub fn has_modal(&self) -> bool {
        !self.modals.is_empty()
    }

    /// Produces a [`NavigationTransition`] snapshot of the current state without mutating it.
    pub fn snapshot(&self) -> NavigationTransition<R> {
        let top = self.top();
        NavigationTransition {
            action: NavigationAction::Snapshot,
            entry_id: top.entry_id,
            route: top.route.clone(),
            stack_depth: self.depth() as u32,
            can_go_back: self.can_go_back(),
            can_go_forward: self.can_go_forward(),
            is_modal: !self.modals.is_empty(),
            active_tab_index: 0,
            active_tab_id: String::new(),
            redirect_hops: 0,
        }
    }

    fn resolve_guard(
        guards: &GuardChain<R>,
        from: Option<&R>,
        target: R,
        action: NavigationAction,
    ) -> Result<(R, usize), NavigationError> {
        match guards.evaluate(from, target, action) {
            GuardOutcome::Allowed(r) => Ok((r, 0)),
            GuardOutcome::Redirected { route, hops } => Ok((route, hops)),
            GuardOutcome::Blocked => Err(NavigationError::Blocked),
            GuardOutcome::RedirectCycle { hops } => Err(NavigationError::RedirectCycle { hops }),
        }
    }

    fn make_transition(
        &self,
        action: NavigationAction,
        entry: &RouteEntry<R>,
        is_modal: bool,
        redirect_hops: usize,
    ) -> NavigationTransition<R> {
        NavigationTransition {
            action,
            entry_id: entry.entry_id,
            route: entry.route.clone(),
            stack_depth: self.depth() as u32,
            can_go_back: self.can_go_back(),
            can_go_forward: self.can_go_forward(),
            is_modal,
            active_tab_index: 0,
            active_tab_id: String::new(),
            redirect_hops,
        }
    }

    /// Pushes `route` onto the stack, truncating any forward history after `cursor`.
    pub fn push(&mut self, route: R) -> Result<NavigationTransition<R>, NavigationError> {
        self.push_with_action(route, NavigationAction::Push, None, None)
    }

    /// Applies an inbound deep-link route onto the stack (`NavigationAction::DeepLink`).
    pub fn apply_deep_link(&mut self, route: R) -> Result<NavigationTransition<R>, NavigationError> {
        self.push_with_action(route, NavigationAction::DeepLink, None, None)
    }

    pub(crate) fn push_with_action(
        &mut self,
        route: R,
        action: NavigationAction,
        extra_guards: Option<&GuardChain<R>>,
        external_id: Option<&mut u64>,
    ) -> Result<NavigationTransition<R>, NavigationError> {
        let from = Some(self.current_route());
        let (target, hops) = match extra_guards {
            Some(g) => Self::resolve_guard(g, from, route, action)?,
            None => Self::resolve_guard(&self.guards, from, route, action)?,
        };
        self.entries.truncate(self.cursor + 1);
        let entry_id = self.alloc_id_from(external_id);
        self.entries.push(RouteEntry {
            entry_id,
            route: target,
        });
        self.cursor = self.entries.len() - 1;
        Ok(self.make_transition(action, &self.entries[self.cursor], false, hops))
    }

    /// Replaces the current entry at `cursor` with `route` without altering stack depth.
    pub fn replace(&mut self, route: R) -> Result<NavigationTransition<R>, NavigationError> {
        self.replace_inner(route, None, None)
    }

    pub(crate) fn replace_inner(
        &mut self,
        route: R,
        extra_guards: Option<&GuardChain<R>>,
        external_id: Option<&mut u64>,
    ) -> Result<NavigationTransition<R>, NavigationError> {
        let from = Some(self.current_route());
        let (target, hops) = match extra_guards {
            Some(g) => Self::resolve_guard(g, from, route, NavigationAction::Replace)?,
            None => Self::resolve_guard(&self.guards, from, route, NavigationAction::Replace)?,
        };
        let entry_id = self.alloc_id_from(external_id);
        self.entries[self.cursor] = RouteEntry {
            entry_id,
            route: target,
        };
        Ok(self.make_transition(
            NavigationAction::Replace,
            &self.entries[self.cursor],
            false,
            hops,
        ))
    }

    /// Pops the top stack entry (`cursor -= 1`) and removes popped entries above the new cursor.
    pub fn pop(&mut self) -> Result<NavigationTransition<R>, NavigationError> {
        self.pop_inner(None, None)
    }

    pub(crate) fn pop_inner(
        &mut self,
        extra_guards: Option<&GuardChain<R>>,
        external_id: Option<&mut u64>,
    ) -> Result<NavigationTransition<R>, NavigationError> {
        if self.cursor == 0 {
            return Err(NavigationError::CannotPopRoot);
        }
        let candidate = self.entries[self.cursor - 1].route.clone();
        let from = Some(self.current_route());
        let (resolved, hops) = match extra_guards {
            Some(g) => Self::resolve_guard(g, from, candidate.clone(), NavigationAction::Pop)?,
            None => Self::resolve_guard(&self.guards, from, candidate.clone(), NavigationAction::Pop)?,
        };
        self.cursor -= 1;
        self.entries.truncate(self.cursor + 1);
        if hops > 0 && resolved != candidate {
            let entry_id = self.alloc_id_from(external_id);
            self.entries[self.cursor] = RouteEntry {
                entry_id,
                route: resolved,
            };
        }
        Ok(self.make_transition(
            NavigationAction::Pop,
            &self.entries[self.cursor],
            false,
            hops,
        ))
    }

    /// Pops all entries above the root entry (`cursor = 0`), truncating the stack to `[root]`.
    pub fn pop_to_root(&mut self) -> Result<NavigationTransition<R>, NavigationError> {
        self.pop_to_root_inner(None, None)
    }

    pub(crate) fn pop_to_root_inner(
        &mut self,
        extra_guards: Option<&GuardChain<R>>,
        external_id: Option<&mut u64>,
    ) -> Result<NavigationTransition<R>, NavigationError> {
        if self.cursor == 0 {
            self.entries.truncate(1);
            return Ok(self.make_transition(
                NavigationAction::PopToRoot,
                &self.entries[0],
                false,
                0,
            ));
        }
        let root_route = self.entries[0].route.clone();
        let from = Some(self.current_route());
        let (resolved, hops) = match extra_guards {
            Some(g) => {
                Self::resolve_guard(g, from, root_route.clone(), NavigationAction::PopToRoot)?
            }
            None => Self::resolve_guard(
                &self.guards,
                from,
                root_route.clone(),
                NavigationAction::PopToRoot,
            )?,
        };
        self.entries.truncate(1);
        self.cursor = 0;
        if hops > 0 && resolved != root_route {
            let entry_id = self.alloc_id_from(external_id);
            self.entries[0] = RouteEntry {
                entry_id,
                route: resolved,
            };
        }
        Ok(self.make_transition(
            NavigationAction::PopToRoot,
            &self.entries[0],
            false,
            hops,
        ))
    }

    /// Moves the history cursor by `delta` (negative = back, positive = forward) without truncating
    /// forward history.
    pub fn go(&mut self, delta: isize) -> Result<NavigationTransition<R>, NavigationError> {
        self.go_inner(delta, None, None)
    }

    pub(crate) fn go_inner(
        &mut self,
        delta: isize,
        extra_guards: Option<&GuardChain<R>>,
        external_id: Option<&mut u64>,
    ) -> Result<NavigationTransition<R>, NavigationError> {
        let Some(target_idx) = self.cursor.checked_add_signed(delta) else {
            return Err(NavigationError::HistoryOutOfBounds {
                cursor: self.cursor,
                len: self.entries.len(),
                delta,
            });
        };
        if target_idx >= self.entries.len() {
            return Err(NavigationError::HistoryOutOfBounds {
                cursor: self.cursor,
                len: self.entries.len(),
                delta,
            });
        }
        if delta == 0 {
            return Ok(self.make_transition(
                NavigationAction::Snapshot,
                &self.entries[self.cursor],
                false,
                0,
            ));
        }
        let action = if delta < 0 {
            NavigationAction::Pop
        } else {
            NavigationAction::Push
        };
        let candidate = self.entries[target_idx].route.clone();
        let from = Some(self.current_route());
        let (resolved, hops) = match extra_guards {
            Some(g) => Self::resolve_guard(g, from, candidate.clone(), action)?,
            None => Self::resolve_guard(&self.guards, from, candidate.clone(), action)?,
        };
        self.cursor = target_idx;
        if hops > 0 && resolved != candidate {
            let entry_id = self.alloc_id_from(external_id);
            self.entries[self.cursor] = RouteEntry {
                entry_id,
                route: resolved,
            };
        }
        Ok(self.make_transition(action, &self.entries[self.cursor], false, hops))
    }

    /// Pushes `route` onto the isolated modal overlay stack without modifying main history.
    pub fn open_modal(&mut self, route: R) -> Result<NavigationTransition<R>, NavigationError> {
        let from = Some(&self.top().route);
        let (resolved, hops) =
            Self::resolve_guard(&self.guards, from, route, NavigationAction::OpenModal)?;
        let entry_id = self.alloc_id_from(None);
        self.modals.push(RouteEntry {
            entry_id,
            route: resolved,
        });
        let top = self.modals.last().expect("just pushed modal");
        Ok(self.make_transition(NavigationAction::OpenModal, top, true, hops))
    }

    /// Dismisses the top-most modal without altering main history.
    pub fn dismiss_modal(&mut self) -> Result<NavigationTransition<R>, NavigationError> {
        if self.modals.is_empty() {
            return Err(NavigationError::NoActiveModal);
        }
        self.modals.pop();
        let is_modal = !self.modals.is_empty();
        let exposed = self.top();
        Ok(self.make_transition(NavigationAction::DismissModal, exposed, is_modal, 0))
    }

    /// Dismisses all open modals without altering main history, returning the number dismissed.
    pub fn dismiss_all_modals(&mut self) -> usize {
        let count = self.modals.len();
        self.modals.clear();
        count
    }
}
