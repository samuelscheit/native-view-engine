//! Dioxus 0.7 router components (`RouterProvider`, `TabRouterProvider`, `Outlet`, `Link`) and
//! reactive hooks (`use_navigator`, `use_route`, `use_active_modal`, `use_tab_navigator`,
//! `use_active_tab`).

use std::cell::RefCell;
use std::fmt::Debug;
use std::rc::Rc;

use dioxus_core::{consume_context, provide_context, try_consume_context};
use dioxus_core_macro::Props;
use nve_dioxus::prelude::*;

use crate::route::Routable;
use crate::stack::{NavigationError, NavigationTransition, RouterState};
use crate::tabs::{TabRouterState, TabSpec};

/// `Clone + PartialEq` wrapper around a route-rendering closure or function pointer for Dioxus
/// component props.
pub struct RouteRenderer<R: Routable>(Rc<dyn Fn(&R) -> Element>);

impl<R: Routable> RouteRenderer<R> {
    pub fn new(f: impl Fn(&R) -> Element + 'static) -> Self {
        Self(Rc::new(f))
    }

    #[inline]
    pub fn render(&self, route: &R) -> Element {
        (self.0)(route)
    }
}

impl<R: Routable> Clone for RouteRenderer<R> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

impl<R: Routable> PartialEq for RouteRenderer<R> {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl<R: Routable> From<fn(&R) -> Element> for RouteRenderer<R> {
    fn from(f: fn(&R) -> Element) -> Self {
        Self::new(f)
    }
}

trait NavDriver<R: Routable> {
    fn current_route(&self) -> R;
    fn active_modal(&self) -> Option<R>;
    fn depth(&self) -> usize;
    fn can_go_back(&self) -> bool;
    fn can_go_forward(&self) -> bool;
    fn snapshot(&self) -> NavigationTransition<R>;
    fn push(&mut self, route: R) -> Result<NavigationTransition<R>, NavigationError>;
    fn replace(&mut self, route: R) -> Result<NavigationTransition<R>, NavigationError>;
    fn pop(&mut self) -> Result<NavigationTransition<R>, NavigationError>;
    fn pop_to_root(&mut self) -> Result<NavigationTransition<R>, NavigationError>;
    fn go(&mut self, delta: isize) -> Result<NavigationTransition<R>, NavigationError>;
    fn apply_deep_link(&mut self, route: R) -> Result<NavigationTransition<R>, NavigationError>;
    fn open_modal(&mut self, route: R) -> Result<NavigationTransition<R>, NavigationError>;
    fn dismiss_modal(&mut self) -> Result<NavigationTransition<R>, NavigationError>;
    fn dismiss_all_modals(&mut self) -> usize;
}

impl<R: Routable> NavDriver<R> for RouterState<R> {
    fn current_route(&self) -> R {
        self.current_route().clone()
    }
    fn active_modal(&self) -> Option<R> {
        self.active_modal().map(|e| e.route.clone())
    }
    fn depth(&self) -> usize {
        self.depth()
    }
    fn can_go_back(&self) -> bool {
        self.can_go_back()
    }
    fn can_go_forward(&self) -> bool {
        self.can_go_forward()
    }
    fn snapshot(&self) -> NavigationTransition<R> {
        self.snapshot()
    }
    fn push(&mut self, route: R) -> Result<NavigationTransition<R>, NavigationError> {
        self.push(route)
    }
    fn replace(&mut self, route: R) -> Result<NavigationTransition<R>, NavigationError> {
        self.replace(route)
    }
    fn pop(&mut self) -> Result<NavigationTransition<R>, NavigationError> {
        self.pop()
    }
    fn pop_to_root(&mut self) -> Result<NavigationTransition<R>, NavigationError> {
        self.pop_to_root()
    }
    fn go(&mut self, delta: isize) -> Result<NavigationTransition<R>, NavigationError> {
        self.go(delta)
    }
    fn apply_deep_link(&mut self, route: R) -> Result<NavigationTransition<R>, NavigationError> {
        self.apply_deep_link(route)
    }
    fn open_modal(&mut self, route: R) -> Result<NavigationTransition<R>, NavigationError> {
        self.open_modal(route)
    }
    fn dismiss_modal(&mut self) -> Result<NavigationTransition<R>, NavigationError> {
        self.dismiss_modal()
    }
    fn dismiss_all_modals(&mut self) -> usize {
        self.dismiss_all_modals()
    }
}

struct SharedTabDriver<T: Copy + Eq + Ord + 'static, R: Routable>(Rc<RefCell<TabRouterState<T, R>>>);

impl<T: Copy + Eq + Ord + 'static, R: Routable> NavDriver<R> for SharedTabDriver<T, R> {
    fn current_route(&self) -> R {
        self.0.borrow().current_route().clone()
    }
    fn active_modal(&self) -> Option<R> {
        self.0.borrow().active_modal().map(|e| e.route.clone())
    }
    fn depth(&self) -> usize {
        self.0.borrow().depth()
    }
    fn can_go_back(&self) -> bool {
        self.0.borrow().can_go_back()
    }
    fn can_go_forward(&self) -> bool {
        self.0.borrow().can_go_forward()
    }
    fn snapshot(&self) -> NavigationTransition<R> {
        self.0.borrow().snapshot()
    }
    fn push(&mut self, route: R) -> Result<NavigationTransition<R>, NavigationError> {
        self.0.borrow_mut().push(route)
    }
    fn replace(&mut self, route: R) -> Result<NavigationTransition<R>, NavigationError> {
        self.0.borrow_mut().replace(route)
    }
    fn pop(&mut self) -> Result<NavigationTransition<R>, NavigationError> {
        self.0.borrow_mut().pop()
    }
    fn pop_to_root(&mut self) -> Result<NavigationTransition<R>, NavigationError> {
        self.0.borrow_mut().pop_to_root()
    }
    fn go(&mut self, delta: isize) -> Result<NavigationTransition<R>, NavigationError> {
        self.0.borrow_mut().go(delta)
    }
    fn apply_deep_link(&mut self, route: R) -> Result<NavigationTransition<R>, NavigationError> {
        self.0.borrow_mut().apply_deep_link(route)
    }
    fn open_modal(&mut self, route: R) -> Result<NavigationTransition<R>, NavigationError> {
        self.0.borrow_mut().open_modal(route)
    }
    fn dismiss_modal(&mut self) -> Result<NavigationTransition<R>, NavigationError> {
        self.0.borrow_mut().dismiss_modal()
    }
    fn dismiss_all_modals(&mut self) -> usize {
        self.0.borrow_mut().dismiss_all_modals()
    }
}

/// Type-erased outlet context allowing `<Outlet />` to render cleanly without generic parameters.
#[derive(Clone)]
struct OutletContext {
    rev: Signal<u64>,
    render: Rc<dyn Fn() -> Element>,
}

/// Reactive navigation controller provided by [`RouterProvider`] and [`TabRouterProvider`].
///
/// Implements [`Copy`] so it can be captured effortlessly by multiple Dioxus event handlers.
pub struct Navigator<R: Routable> {
    driver: Signal<Rc<RefCell<dyn NavDriver<R>>>>,
    rev: Signal<u64>,
}

impl<R: Routable> Copy for Navigator<R> {}

impl<R: Routable> Clone for Navigator<R> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<R: Routable> Navigator<R> {
    fn bump(&self) {
        let mut rev = self.rev;
        let next = (*rev.peek()).wrapping_add(1);
        rev.set(next);
    }

    fn with_driver<Out>(&self, f: impl FnOnce(&(dyn NavDriver<R> + 'static)) -> Out) -> Out {
        let rc = Rc::clone(&*self.driver.peek());
        let borrowed = rc.borrow();
        f(&*borrowed)
    }

    fn with_driver_mut<Out>(
        &self,
        f: impl FnOnce(&mut (dyn NavDriver<R> + 'static)) -> Out,
    ) -> Out {
        let rc = Rc::clone(&*self.driver.peek());
        let mut borrowed = rc.borrow_mut();
        f(&mut *borrowed)
    }

    pub fn current_route(&self) -> R {
        self.with_driver(NavDriver::current_route)
    }

    pub fn active_modal(&self) -> Option<R> {
        self.with_driver(NavDriver::active_modal)
    }

    pub fn has_modal(&self) -> bool {
        self.with_driver(|d| d.active_modal().is_some())
    }

    pub fn depth(&self) -> usize {
        self.with_driver(NavDriver::depth)
    }

    pub fn can_go_back(&self) -> bool {
        self.with_driver(NavDriver::can_go_back)
    }

    pub fn can_go_forward(&self) -> bool {
        self.with_driver(NavDriver::can_go_forward)
    }

    pub fn snapshot(&self) -> NavigationTransition<R> {
        self.with_driver(NavDriver::snapshot)
    }

    pub fn push(&self, route: R) -> Result<NavigationTransition<R>, NavigationError> {
        let res = self.with_driver_mut(|d| d.push(route));
        if res.is_ok() {
            self.bump();
        }
        res
    }

    pub fn replace(&self, route: R) -> Result<NavigationTransition<R>, NavigationError> {
        let res = self.with_driver_mut(|d| d.replace(route));
        if res.is_ok() {
            self.bump();
        }
        res
    }

    pub fn pop(&self) -> Result<NavigationTransition<R>, NavigationError> {
        let res = self.with_driver_mut(NavDriver::pop);
        if res.is_ok() {
            self.bump();
        }
        res
    }

    pub fn pop_to_root(&self) -> Result<NavigationTransition<R>, NavigationError> {
        let res = self.with_driver_mut(NavDriver::pop_to_root);
        if res.is_ok() {
            self.bump();
        }
        res
    }

    pub fn go(&self, delta: isize) -> Result<NavigationTransition<R>, NavigationError> {
        let res = self.with_driver_mut(|d| d.go(delta));
        if res.is_ok() {
            self.bump();
        }
        res
    }

    pub fn apply_deep_link(&self, route: R) -> Result<NavigationTransition<R>, NavigationError> {
        let res = self.with_driver_mut(|d| d.apply_deep_link(route));
        if res.is_ok() {
            self.bump();
        }
        res
    }

    pub fn open_modal(&self, route: R) -> Result<NavigationTransition<R>, NavigationError> {
        let res = self.with_driver_mut(|d| d.open_modal(route));
        if res.is_ok() {
            self.bump();
        }
        res
    }

    pub fn dismiss_modal(&self) -> Result<NavigationTransition<R>, NavigationError> {
        let res = self.with_driver_mut(NavDriver::dismiss_modal);
        if res.is_ok() {
            self.bump();
        }
        res
    }

    pub fn dismiss_all_modals(&self) -> usize {
        let count = self.with_driver_mut(NavDriver::dismiss_all_modals);
        if count > 0 {
            self.bump();
        }
        count
    }
}

/// Tab-aware reactive navigation handle provided by [`TabRouterProvider`].
///
/// Implements [`Copy`] so it can be captured effortlessly by multiple Dioxus event handlers.
pub struct TabNavigator<T: Copy + Eq + Ord + 'static, R: Routable> {
    state: Signal<Rc<RefCell<TabRouterState<T, R>>>>,
    rev: Signal<u64>,
}

impl<T: Copy + Eq + Ord + 'static, R: Routable> Copy for TabNavigator<T, R> {}

impl<T: Copy + Eq + Ord + 'static, R: Routable> Clone for TabNavigator<T, R> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: Copy + Eq + Ord + 'static, R: Routable> TabNavigator<T, R> {
    fn bump(&self) {
        let mut rev = self.rev;
        let next = (*rev.peek()).wrapping_add(1);
        rev.set(next);
    }

    pub fn active_tab(&self) -> T {
        let rc = Rc::clone(&*self.state.peek());
        let borrowed = rc.borrow();
        borrowed.active_tab()
    }

    pub fn active_tab_index(&self) -> usize {
        let rc = Rc::clone(&*self.state.peek());
        let borrowed = rc.borrow();
        borrowed.active_tab_index()
    }

    pub fn select_tab(&self, tab: T) -> Result<NavigationTransition<R>, NavigationError> {
        let rc = Rc::clone(&*self.state.peek());
        let res = rc.borrow_mut().select_tab(tab);
        if res.is_ok() {
            self.bump();
        }
        res
    }
}

/// Returns the [`Navigator<R>`] from the nearest enclosing [`RouterProvider`] or
/// [`TabRouterProvider`].
pub fn use_navigator<R: Routable>() -> Navigator<R> {
    consume_context::<Navigator<R>>()
}

/// Fallible variant of [`use_navigator`].
pub fn try_use_navigator<R: Routable>() -> Option<Navigator<R>> {
    try_consume_context::<Navigator<R>>()
}

/// Subscribes the calling component to router transitions and returns the active route `R`.
pub fn use_route<R: Routable>() -> R {
    let nav = use_navigator::<R>();
    let _rev = (nav.rev)();
    nav.current_route()
}

/// Subscribes the calling component to router transitions and returns the top-most modal route.
pub fn use_active_modal<R: Routable>() -> Option<R> {
    let nav = use_navigator::<R>();
    let _rev = (nav.rev)();
    nav.active_modal()
}

/// Returns the [`TabNavigator<T, R>`] from the nearest enclosing [`TabRouterProvider`].
pub fn use_tab_navigator<T: Copy + Eq + Ord + 'static, R: Routable>() -> TabNavigator<T, R> {
    consume_context::<TabNavigator<T, R>>()
}

/// Subscribes the calling component to tab transitions and returns the active tab key `T`.
pub fn use_active_tab<T: Copy + Eq + Ord + 'static, R: Routable>() -> T {
    let tabs = use_tab_navigator::<T, R>();
    let _rev = (tabs.rev)();
    tabs.active_tab()
}

/// Provides a stack + modal [`RouterState<R>`] and [`Outlet`] renderer to its subtree.
#[component]
pub fn RouterProvider<R: Routable>(
    initial: R,
    #[props(into)] render: RouteRenderer<R>,
    #[props(default)] render_modal: Option<RouteRenderer<R>>,
    #[props(default)] children: Option<Element>,
) -> Element {
    let rev = use_signal(|| 0u64);
    let driver_sig = use_signal(|| {
        Rc::new(RefCell::new(RouterState::new(initial))) as Rc<RefCell<dyn NavDriver<R>>>
    });
    let driver = Rc::clone(&*driver_sig.peek());

    let nav = Navigator {
        driver: driver_sig,
        rev,
    };
    provide_context(nav);

    let render_for_outlet = render.clone();
    let modal_for_outlet = render_modal.clone();
    let driver_for_outlet = Rc::clone(&driver);
    provide_context(OutletContext {
        rev,
        render: Rc::new(move || {
            let active = driver_for_outlet.borrow().current_route();
            let modal = driver_for_outlet.borrow().active_modal();
            let main_el = render_for_outlet.render(&active);
            match (modal, &modal_for_outlet) {
                (Some(m), Some(mr)) => {
                    let modal_el = mr.render(&m);
                    rsx! {
                        view {
                            {main_el}
                            view {
                                accessibility_role: "dialog",
                                {modal_el}
                            }
                        }
                    }
                }
                _ => main_el,
            }
        }),
    });

    match children {
        Some(child) => child,
        None => rsx! { Outlet {} },
    }
}

/// Provides a multi-tab [`TabRouterState<T, R>`], [`Navigator<R>`], [`TabNavigator<T, R>`], and
/// [`Outlet`] renderer to its subtree.
#[component]
pub fn TabRouterProvider<T: Copy + Eq + Ord + Debug + 'static, R: Routable>(
    tabs: Vec<TabSpec<T, R>>,
    initial_tab: T,
    #[props(into)] render: RouteRenderer<R>,
    #[props(default)] render_modal: Option<RouteRenderer<R>>,
    #[props(default)] children: Option<Element>,
) -> Element {
    let rev = use_signal(|| 0u64);
    let tab_state_sig = use_signal(|| {
        Rc::new(RefCell::new(
            TabRouterState::with_specs(tabs, initial_tab)
                .expect("TabRouterProvider requires valid non-empty tabs containing initial_tab"),
        ))
    });
    let tab_state = Rc::clone(&*tab_state_sig.peek());
    let driver_sig = use_signal(|| {
        Rc::new(RefCell::new(SharedTabDriver(Rc::clone(&tab_state))))
            as Rc<RefCell<dyn NavDriver<R>>>
    });
    let driver = Rc::clone(&*driver_sig.peek());

    provide_context(Navigator {
        driver: driver_sig,
        rev,
    });
    provide_context(TabNavigator {
        state: tab_state_sig,
        rev,
    });

    let render_for_outlet = render.clone();
    let modal_for_outlet = render_modal.clone();
    let driver_for_outlet = Rc::clone(&driver);
    provide_context(OutletContext {
        rev,
        render: Rc::new(move || {
            let active = driver_for_outlet.borrow().current_route();
            let modal = driver_for_outlet.borrow().active_modal();
            let main_el = render_for_outlet.render(&active);
            match (modal, &modal_for_outlet) {
                (Some(m), Some(mr)) => {
                    let modal_el = mr.render(&m);
                    rsx! {
                        view {
                            {main_el}
                            view {
                                accessibility_role: "dialog",
                                {modal_el}
                            }
                        }
                    }
                }
                _ => main_el,
            }
        }),
    });

    match children {
        Some(child) => child,
        None => rsx! { Outlet {} },
    }
}

/// Renders the currently matched route (and active modal overlay, if configured) from the enclosing
/// [`RouterProvider`] or [`TabRouterProvider`].
#[component]
pub fn Outlet() -> Element {
    let ctx = consume_context::<OutletContext>();
    let _rev = (ctx.rev)();
    (ctx.render)()
}

/// Accessible navigation link rendering a native `pressable` with `accessibility_role: "link"` and
/// `onpress` navigation into the enclosing [`Navigator<R>`].
#[component]
pub fn Link<R: Routable>(
    to: R,
    #[props(default)] replace: bool,
    #[props(default)] modal: bool,
    #[props(default)] style: Option<String>,
    #[props(default)] test_id: Option<String>,
    #[props(default)] accessibility_label: Option<String>,
    #[props(default)] accessibility_hint: Option<String>,
    children: Element,
) -> Element {
    let nav = use_navigator::<R>();
    let href = to.to_path();
    let label = accessibility_label
        .or_else(|| to.title().map(str::to_owned))
        .unwrap_or_else(|| href.clone());
    let hint = accessibility_hint.unwrap_or(href);
    let target = to;

    rsx! {
        pressable {
            accessibility_role: "link",
            accessibility_label: "{label}",
            accessibility_hint: "{hint}",
            test_id: test_id,
            style: style,
            onpress: move |_| {
                if modal {
                    let _ = nav.open_modal(target.clone());
                } else if replace {
                    let _ = nav.replace(target.clone());
                } else {
                    let _ = nav.push(target.clone());
                }
            },
            {children}
        }
    }
}
