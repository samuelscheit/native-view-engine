//! Multi-tab navigation state machine (`TabRouterState<T, R>`) with independent per-tab back-stacks,
//! re-tap pop-to-root semantics, and a shared modal overlay stack.

use std::collections::BTreeMap;

use crate::guard::{guard_fn, GuardChain, GuardDecision, NavigationContext, NavigationGuard};
use crate::route::Routable;
use crate::stack::{NavigationAction, NavigationError, NavigationTransition, RouteEntry, RouterState};

/// Registration descriptor for one tab inside [`TabRouterState`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabSpec<T, R> {
    pub key: T,
    pub id: String,
    pub root: R,
}

impl<T, R> TabSpec<T, R> {
    pub fn new(key: T, id: impl Into<String>, root: R) -> Self {
        Self {
            key,
            id: id.into(),
            root,
        }
    }
}

#[derive(Debug, Clone)]
struct TabSlot<T, R: Routable> {
    key: T,
    id: String,
    stack: RouterState<R>,
}

/// Expo Router / `UITabBarController`-style tab router with independent history stacks per tab `T`
/// and a shared modal overlay stack.
#[derive(Debug, Clone)]
pub struct TabRouterState<T: Copy + Eq + Ord + 'static, R: Routable> {
    tabs: Vec<TabSlot<T, R>>,
    index_by_key: BTreeMap<T, usize>,
    active_index: usize,
    next_entry_id: u64,
    modals: Vec<RouteEntry<R>>,
    guards: GuardChain<R>,
}

impl<T: Copy + Eq + Ord + 'static, R: Routable> TabRouterState<T, R> {
    /// Creates a tab router from `(tab_key, root_route)` pairs, assigning deterministic string IDs
    /// (`"tab-0"`, `"tab-1"`, ...).
    pub fn new(tabs: Vec<(T, R)>, initial_tab: T) -> Result<Self, NavigationError> {
        let specs = tabs
            .into_iter()
            .enumerate()
            .map(|(idx, (key, root))| TabSpec::new(key, format!("tab-{idx}"), root))
            .collect();
        Self::with_specs(specs, initial_tab)
    }

    /// Creates a tab router from explicit [`TabSpec`] descriptors preserving registration order.
    pub fn with_specs(
        specs: Vec<TabSpec<T, R>>,
        initial_tab: T,
    ) -> Result<Self, NavigationError> {
        if specs.is_empty() {
            return Err(NavigationError::InvalidTabConfig(
                "at least one tab is required",
            ));
        }

        let mut tabs = Vec::with_capacity(specs.len());
        let mut index_by_key = BTreeMap::new();
        let mut next_entry_id = 1u64;

        for (idx, spec) in specs.into_iter().enumerate() {
            if index_by_key.insert(spec.key, idx).is_some() {
                return Err(NavigationError::InvalidTabConfig("duplicate tab key"));
            }
            let stack = RouterState::with_starting_id(spec.root, next_entry_id);
            next_entry_id = next_entry_id.saturating_add(1);
            tabs.push(TabSlot {
                key: spec.key,
                id: spec.id,
                stack,
            });
        }

        let Some(&active_index) = index_by_key.get(&initial_tab) else {
            return Err(NavigationError::InvalidTabConfig(
                "initial tab not found in tab specs",
            ));
        };

        Ok(Self {
            tabs,
            index_by_key,
            active_index,
            next_entry_id,
            modals: Vec::new(),
            guards: GuardChain::new(),
        })
    }

    /// Configures the maximum redirect hops allowed for shared guards.
    pub fn with_max_redirect_hops(mut self, max_hops: usize) -> Self {
        self.guards.set_max_redirect_hops(max_hops);
        self
    }

    /// Registers a synchronous [`NavigationGuard`] shared across all tabs and modals.
    pub fn add_guard(&mut self, guard: impl NavigationGuard<R> + 'static) {
        self.guards.add(guard);
    }

    /// Registers a synchronous closure guard shared across all tabs and modals.
    pub fn add_guard_fn<F>(&mut self, f: F)
    where
        F: Fn(&NavigationContext<'_, R>) -> GuardDecision<R> + Send + Sync + 'static,
    {
        self.guards.add(guard_fn(f));
    }

    /// Currently active tab key.
    #[inline]
    pub fn active_tab(&self) -> T {
        self.tabs[self.active_index].key
    }

    /// Zero-based registration index of the active tab.
    #[inline]
    pub fn active_tab_index(&self) -> usize {
        self.active_index
    }

    /// Wire identifier string of the active tab.
    #[inline]
    pub fn active_tab_id(&self) -> &str {
        &self.tabs[self.active_index].id
    }

    /// Number of registered tabs.
    #[inline]
    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    /// Ordered tab keys in registration order.
    pub fn tab_keys(&self) -> Vec<T> {
        self.tabs.iter().map(|slot| slot.key).collect()
    }

    /// Returns the active tab's underlying [`RouterState`].
    #[inline]
    pub fn active_stack(&self) -> &RouterState<R> {
        &self.tabs[self.active_index].stack
    }

    /// Looks up the independent [`RouterState`] for `tab`.
    pub fn tab_stack(&self, tab: T) -> Option<&RouterState<R>> {
        let &idx = self.index_by_key.get(&tab)?;
        Some(&self.tabs[idx].stack)
    }

    /// Active route entry on the active tab's stack (ignoring modal overlays).
    #[inline]
    pub fn current(&self) -> &RouteEntry<R> {
        self.active_stack().current()
    }

    /// Active route on the active tab's stack.
    #[inline]
    pub fn current_route(&self) -> &R {
        self.active_stack().current_route()
    }

    /// Top-most shared modal if any, otherwise the active tab's current entry.
    #[inline]
    pub fn top(&self) -> &RouteEntry<R> {
        self.modals.last().unwrap_or_else(|| self.current())
    }

    #[inline]
    pub fn depth(&self) -> usize {
        self.active_stack().depth()
    }

    #[inline]
    pub fn can_go_back(&self) -> bool {
        self.active_stack().can_go_back()
    }

    #[inline]
    pub fn can_go_forward(&self) -> bool {
        self.active_stack().can_go_forward()
    }

    #[inline]
    pub fn modals(&self) -> &[RouteEntry<R>] {
        &self.modals
    }

    #[inline]
    pub fn active_modal(&self) -> Option<&RouteEntry<R>> {
        self.modals.last()
    }

    #[inline]
    pub fn has_modal(&self) -> bool {
        !self.modals.is_empty()
    }

    fn stamp_tab(&self, mut tr: NavigationTransition<R>) -> NavigationTransition<R> {
        tr.active_tab_index = self.active_index as u32;
        tr.active_tab_id = self.tabs[self.active_index].id.clone();
        tr
    }

    /// Returns a snapshot of the active tab and modal state without mutating state.
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
            active_tab_index: self.active_index as u32,
            active_tab_id: self.tabs[self.active_index].id.clone(),
            redirect_hops: 0,
        }
    }

    /// Switches to `tab`, preserving both tabs' independent back-stacks.
    ///
    /// Re-selecting the already-active tab pops its stack back to root (Expo Router /
    /// `UITabBarController` convention).
    pub fn select_tab(&mut self, tab: T) -> Result<NavigationTransition<R>, NavigationError> {
        let Some(&target_idx) = self.index_by_key.get(&tab) else {
            return Err(NavigationError::UnknownTab);
        };
        self.select_tab_index(target_idx)
    }

    /// Selects a tab by its zero-based registration index.
    pub fn switch_tab_by_index(
        &mut self,
        index: usize,
    ) -> Result<NavigationTransition<R>, NavigationError> {
        if index >= self.tabs.len() {
            return Err(NavigationError::UnknownTab);
        }
        self.select_tab_index(index)
    }

    /// Selects a tab by its wire `tab_id` string.
    pub fn switch_tab_by_id(
        &mut self,
        tab_id: &str,
    ) -> Result<NavigationTransition<R>, NavigationError> {
        let Some(index) = self.tabs.iter().position(|slot| slot.id == tab_id) else {
            return Err(NavigationError::UnknownTab);
        };
        self.select_tab_index(index)
    }

    fn select_tab_index(
        &mut self,
        target_idx: usize,
    ) -> Result<NavigationTransition<R>, NavigationError> {
        if target_idx == self.active_index {
            return self.pop_to_root();
        }

        let from_route = self.current_route().clone();
        let target_route = self.tabs[target_idx].stack.current_route().clone();

        let (resolved, hops) = self
            .guards
            .evaluate(
                Some(&from_route),
                target_route.clone(),
                NavigationAction::SwitchTab,
            )
            .into_result()?;

        self.active_index = target_idx;
        if hops > 0 && resolved != target_route {
            let empty_guards = GuardChain::new();
            let tr = self.tabs[self.active_index].stack.push_with_action(
                resolved,
                NavigationAction::SwitchTab,
                Some(&empty_guards),
                Some(&mut self.next_entry_id),
            )?;
            let mut tr = self.stamp_tab(tr);
            tr.redirect_hops = hops;
            return Ok(tr);
        }

        let current = self.tabs[self.active_index].stack.current();
        Ok(NavigationTransition {
            action: NavigationAction::SwitchTab,
            entry_id: current.entry_id,
            route: current.route.clone(),
            stack_depth: self.depth() as u32,
            can_go_back: self.can_go_back(),
            can_go_forward: self.can_go_forward(),
            is_modal: false,
            active_tab_index: self.active_index as u32,
            active_tab_id: self.tabs[self.active_index].id.clone(),
            redirect_hops: hops,
        })
    }

    /// Pushes `route` onto the active tab's stack.
    pub fn push(&mut self, route: R) -> Result<NavigationTransition<R>, NavigationError> {
        let tr = self.tabs[self.active_index].stack.push_with_action(
            route,
            NavigationAction::Push,
            Some(&self.guards),
            Some(&mut self.next_entry_id),
        )?;
        Ok(self.stamp_tab(tr))
    }

    /// Applies a deep-link route onto the active tab's stack.
    pub fn apply_deep_link(&mut self, route: R) -> Result<NavigationTransition<R>, NavigationError> {
        let tr = self.tabs[self.active_index].stack.push_with_action(
            route,
            NavigationAction::DeepLink,
            Some(&self.guards),
            Some(&mut self.next_entry_id),
        )?;
        Ok(self.stamp_tab(tr))
    }

    /// Replaces the active tab's current route entry.
    pub fn replace(&mut self, route: R) -> Result<NavigationTransition<R>, NavigationError> {
        let tr = self.tabs[self.active_index].stack.replace_inner(
            route,
            Some(&self.guards),
            Some(&mut self.next_entry_id),
        )?;
        Ok(self.stamp_tab(tr))
    }

    /// Pops one entry from the active tab's stack.
    pub fn pop(&mut self) -> Result<NavigationTransition<R>, NavigationError> {
        let tr = self.tabs[self.active_index]
            .stack
            .pop_inner(Some(&self.guards), Some(&mut self.next_entry_id))?;
        Ok(self.stamp_tab(tr))
    }

    /// Pops the active tab's stack back to its root entry.
    pub fn pop_to_root(&mut self) -> Result<NavigationTransition<R>, NavigationError> {
        let tr = self.tabs[self.active_index]
            .stack
            .pop_to_root_inner(Some(&self.guards), Some(&mut self.next_entry_id))?;
        Ok(self.stamp_tab(tr))
    }

    /// Moves the active tab's history cursor by `delta`.
    pub fn go(&mut self, delta: isize) -> Result<NavigationTransition<R>, NavigationError> {
        let tr = self.tabs[self.active_index].stack.go_inner(
            delta,
            Some(&self.guards),
            Some(&mut self.next_entry_id),
        )?;
        Ok(self.stamp_tab(tr))
    }

    /// Opens a modal overlay shared across all tabs without mutating any tab's back-stack.
    pub fn open_modal(&mut self, route: R) -> Result<NavigationTransition<R>, NavigationError> {
        let from = Some(self.top().route.clone());
        let (resolved, hops) = self
            .guards
            .evaluate(from.as_ref(), route, NavigationAction::OpenModal)
            .into_result()?;

        let entry_id = self.next_entry_id;
        self.next_entry_id = self.next_entry_id.saturating_add(1);
        self.modals.push(RouteEntry {
            entry_id,
            route: resolved,
        });
        let top = self.modals.last().expect("just pushed modal");
        Ok(NavigationTransition {
            action: NavigationAction::OpenModal,
            entry_id: top.entry_id,
            route: top.route.clone(),
            stack_depth: self.depth() as u32,
            can_go_back: self.can_go_back(),
            can_go_forward: self.can_go_forward(),
            is_modal: true,
            active_tab_index: self.active_index as u32,
            active_tab_id: self.tabs[self.active_index].id.clone(),
            redirect_hops: hops,
        })
    }

    /// Dismisses the top-most shared modal overlay.
    pub fn dismiss_modal(&mut self) -> Result<NavigationTransition<R>, NavigationError> {
        if self.modals.is_empty() {
            return Err(NavigationError::NoActiveModal);
        }
        self.modals.pop();
        let is_modal = !self.modals.is_empty();
        let exposed = self.top();
        Ok(NavigationTransition {
            action: NavigationAction::DismissModal,
            entry_id: exposed.entry_id,
            route: exposed.route.clone(),
            stack_depth: self.depth() as u32,
            can_go_back: self.can_go_back(),
            can_go_forward: self.can_go_forward(),
            is_modal,
            active_tab_index: self.active_index as u32,
            active_tab_id: self.tabs[self.active_index].id.clone(),
            redirect_hops: 0,
        })
    }

    /// Dismisses all open shared modals without touching any tab's back-stack.
    pub fn dismiss_all_modals(&mut self) -> usize {
        let count = self.modals.len();
        self.modals.clear();
        count
    }
}
