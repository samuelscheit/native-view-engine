//! Synchronous navigation guards with redirect-chain cycle detection.

use std::fmt;
use std::sync::Arc;

use crate::route::Routable;
use crate::stack::NavigationAction;

/// Default maximum redirect hops permitted during a single guard evaluation pass.
pub const DEFAULT_MAX_REDIRECT_HOPS: usize = 8;

/// Decision returned synchronously by a [`NavigationGuard`] before router state is mutated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardDecision<R> {
    /// Permit navigation to the target route.
    Allow,
    /// Redirect navigation to another route (re-evaluated through all guards).
    Redirect(R),
    /// Reject navigation without mutating router state.
    Block,
}

/// Read-only context supplied to [`NavigationGuard::check`].
#[derive(Debug, Clone, Copy)]
pub struct NavigationContext<'a, R> {
    /// Currently active route before this navigation attempt (if any).
    pub from: Option<&'a R>,
    /// Candidate destination route being evaluated.
    pub to: &'a R,
    /// Navigation action initiating this transition.
    pub action: NavigationAction,
    /// Zero-based redirect hop counter (`0` for the original requested destination).
    pub hop: usize,
}

/// Synchronous navigation guard evaluated prior to any router state mutation.
pub trait NavigationGuard<R>: Send + Sync {
    fn check(&self, ctx: &NavigationContext<'_, R>) -> GuardDecision<R>;
}

/// Closure adapter implementing [`NavigationGuard`].
pub struct FnGuard<F>(F);

impl<R, F> NavigationGuard<R> for FnGuard<F>
where
    F: Fn(&NavigationContext<'_, R>) -> GuardDecision<R> + Send + Sync,
{
    fn check(&self, ctx: &NavigationContext<'_, R>) -> GuardDecision<R> {
        (self.0)(ctx)
    }
}

/// Wraps a synchronous closure as a [`NavigationGuard`].
pub fn guard_fn<R, F>(f: F) -> FnGuard<F>
where
    F: Fn(&NavigationContext<'_, R>) -> GuardDecision<R> + Send + Sync,
{
    FnGuard(f)
}

/// Outcome of evaluating a [`GuardChain`] against a target route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardOutcome<R> {
    /// Navigation permitted to the original target route.
    Allowed(R),
    /// Navigation permitted after `hops` redirect(s) to `route`.
    Redirected { route: R, hops: usize },
    /// Navigation rejected by a guard.
    Blocked,
    /// Redirect chain exceeded `max_hops` or revisited a route in the same transition.
    RedirectCycle { hops: usize },
}

impl<R> GuardOutcome<R> {
    #[inline]
    pub(crate) fn into_result(self) -> Result<(R, usize), crate::stack::NavigationError> {
        match self {
            Self::Allowed(r) => Ok((r, 0)),
            Self::Redirected { route, hops } => Ok((route, hops)),
            Self::Blocked => Err(crate::stack::NavigationError::Blocked),
            Self::RedirectCycle { hops } => Err(crate::stack::NavigationError::RedirectCycle { hops }),
        }
    }
}

impl<R> GuardOutcome<R> {
    /// Consumes the outcome and returns the resolved destination route if navigation is permitted.
    pub fn into_route(self) -> Option<R> {
        match self {
            Self::Allowed(r) | Self::Redirected { route: r, .. } => Some(r),
            Self::Blocked | Self::RedirectCycle { .. } => None,
        }
    }
}

/// Ordered list of synchronous [`NavigationGuard`]s with redirect cycle & hop-limit protection.
pub struct GuardChain<R> {
    guards: Vec<Arc<dyn NavigationGuard<R>>>,
    max_redirect_hops: usize,
}

impl<R> Clone for GuardChain<R> {
    fn clone(&self) -> Self {
        Self {
            guards: self.guards.clone(),
            max_redirect_hops: self.max_redirect_hops,
        }
    }
}

impl<R> Default for GuardChain<R> {
    fn default() -> Self {
        Self::new()
    }
}

impl<R> fmt::Debug for GuardChain<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GuardChain")
            .field("count", &self.guards.len())
            .field("max_redirect_hops", &self.max_redirect_hops)
            .finish()
    }
}

impl<R> GuardChain<R> {
    pub fn new() -> Self {
        Self {
            guards: Vec::new(),
            max_redirect_hops: DEFAULT_MAX_REDIRECT_HOPS,
        }
    }

    pub fn with_max_redirect_hops(mut self, max_hops: usize) -> Self {
        self.max_redirect_hops = max_hops;
        self
    }

    pub fn max_redirect_hops(&self) -> usize {
        self.max_redirect_hops
    }

    pub fn set_max_redirect_hops(&mut self, max_hops: usize) {
        self.max_redirect_hops = max_hops;
    }

    pub fn is_empty(&self) -> bool {
        self.guards.is_empty()
    }

    pub fn len(&self) -> usize {
        self.guards.len()
    }

    pub fn add(&mut self, guard: impl NavigationGuard<R> + 'static) {
        self.guards.push(Arc::new(guard));
    }

    pub fn add_arc(&mut self, guard: Arc<dyn NavigationGuard<R>>) {
        self.guards.push(guard);
    }

    pub fn clear(&mut self) {
        self.guards.clear();
    }
}

impl<R: Routable> GuardChain<R> {
    /// Evaluates every registered guard in registration order against `target`.
    ///
    /// On [`GuardDecision::Redirect`], restarts evaluation from the first guard with the new
    /// target, rejecting cycles (revisited target in the same chain) and exceeding
    /// `max_redirect_hops`.
    pub fn evaluate(
        &self,
        from: Option<&R>,
        target: R,
        action: NavigationAction,
    ) -> GuardOutcome<R> {
        if self.guards.is_empty() {
            return GuardOutcome::Allowed(target);
        }

        let mut current = target;
        let mut visited: Vec<R> = Vec::new();
        let mut hops = 0usize;

        'outer: loop {
            for guard in &self.guards {
                let ctx = NavigationContext {
                    from,
                    to: &current,
                    action,
                    hop: hops,
                };
                match guard.check(&ctx) {
                    GuardDecision::Allow => {}
                    GuardDecision::Block => return GuardOutcome::Blocked,
                    GuardDecision::Redirect(next) => {
                        if hops >= self.max_redirect_hops || next == current || visited.contains(&next)
                        {
                            return GuardOutcome::RedirectCycle { hops: hops + 1 };
                        }
                        visited.push(current);
                        current = next;
                        hops += 1;
                        continue 'outer;
                    }
                }
            }
            return if hops == 0 {
                GuardOutcome::Allowed(current)
            } else {
                GuardOutcome::Redirected {
                    route: current,
                    hops,
                }
            };
        }
    }
}
