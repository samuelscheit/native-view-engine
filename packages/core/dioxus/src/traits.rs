//! Dioxus 0.7 ergonomics for the Unified Extensible Environment & `TraitKey` System.
//!
//! Provides:
//! - [`ReactiveTraitStore`] & [`TraitProvider`] / [`use_provide_trait_store`] for host-driven
//!   breakpoint-only reactivity (1px resizes within a [`SizeClass`] never trigger re-renders).
//! - [`TraitScope`] for lexical subtree overrides with automatic shadowing and restoration.
//! - [`use_trait`] subscribing strictly to the single trait `K` read.
//! - [`use_environment`] returning the effective [`TraitEnvironment`] at the current tree position.
//! - [`PlatformOnly`] and [`TraitSwitch`] ergonomic conditional rendering components.

use std::any::TypeId;
use std::cell::RefCell;
use nve_abi::hash::HashMap;
use std::rc::Rc;

use dioxus_core::{
    current_scope_id, provide_context, try_consume_context, use_hook, Element, ScopeId, VNode,
};
use crate::rsx;
use dioxus_core_macro::{component, Props};
use dioxus_signals::{ReadableExt, Signal, WritableExt};

pub use nve_interop::traits::{
    effective_bottom_inset, keyboard_insets_from_visual_viewport, BuiltinTraitSlot, BuiltinTraits,
    CanHover, ColorScheme, ContainerSize, HasHardwareKeyboard, HighContrast, HostToolkit,
    KeyboardInsets, Locale, Orientation, PlatformOs, PointerKind, ReducedMotion, SafeAreaInsets,
    ScaleFactor, SizeClass, TextScale, TraitEnvironment, TraitKey, TraitOverrides, TraitResolver,
    TraitStore, UiDensity, VerticalSizeClass, ViewportChangeMask, BUILTIN_TRAIT_COUNT,
    VISUAL_VIEWPORT_KEYBOARD_THRESHOLD_PX,
};

struct ReactiveStoreInner {
    store: TraitStore,
    owner_scope: ScopeId,
    builtin_signals: [Option<Signal<u64>>; BUILTIN_TRAIT_COUNT],
    custom_signals: HashMap<TypeId, Signal<u64>>,
    any_signal: Option<Signal<u64>>,
}

/// Dioxus-reactive handle wrapping [`TraitStore`] with per-trait revision signals.
///
/// Updating a trait only writes to the single Dioxus [`Signal`] corresponding to that trait, so
/// components reading other traits (or reading `SizeClass`/`Orientation` during a 1px drag inside
/// the same breakpoint) are never marked dirty.
#[derive(Clone)]
pub struct ReactiveTraitStore {
    inner: Rc<RefCell<ReactiveStoreInner>>,
}

impl PartialEq for ReactiveTraitStore {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }
}

impl ReactiveTraitStore {
    /// Creates a reactive trait store anchored to `owner_scope` (typically [`ScopeId::ROOT`] or
    /// `current_scope_id()`).
    pub fn new_in_scope(store: TraitStore, owner_scope: ScopeId) -> Self {
        Self {
            inner: Rc::new(RefCell::new(ReactiveStoreInner {
                store,
                owner_scope,
                builtin_signals: [None; BUILTIN_TRAIT_COUNT],
                custom_signals: HashMap::default(),
                any_signal: None,
            })),
        }
    }

    /// Creates a reactive trait store anchored to the current Dioxus component scope.
    pub fn new(store: TraitStore) -> Self {
        Self::new_in_scope(store, current_scope_id())
    }

    /// Creates a reactive trait store anchored to [`ScopeId::ROOT`] so host callbacks can create
    /// or update it via `vdom.in_scope(ScopeId::ROOT, || ...)`.
    pub fn new_root(store: TraitStore) -> Self {
        Self::new_in_scope(store, ScopeId::ROOT)
    }

    /// Reads trait `K` without subscribing the current component.
    #[inline]
    pub fn peek<K: TraitKey>(&self) -> K::Value {
        self.inner.borrow().store.get::<K>()
    }

    /// Returns an immutable snapshot of the current [`TraitEnvironment`] without subscribing.
    #[inline]
    pub fn peek_environment(&self) -> TraitEnvironment {
        self.inner.borrow().store.environment().clone()
    }

    /// Returns the monotonic revision counter for trait `K`.
    #[inline]
    pub fn revision<K: TraitKey>(&self) -> u64 {
        self.inner.borrow().store.revision::<K>()
    }

    /// Subscribes the currently executing Dioxus component strictly to trait `K` and returns its
    /// current value.
    pub fn read_reactive<K: TraitKey>(&self) -> K::Value {
        let (signal, value) = {
            let mut guard = self.inner.borrow_mut();
            let owner = guard.owner_scope;
            let val = guard.store.get::<K>();
            let rev = guard.store.revision::<K>();
            let sig = if let Some(slot) = K::BUILTIN_SLOT {
                let idx = slot.index();
                *guard.builtin_signals[idx].get_or_insert_with(|| Signal::new_in_scope(rev, owner))
            } else {
                let tid = TypeId::of::<K>();
                *guard
                    .custom_signals
                    .entry(tid)
                    .or_insert_with(|| Signal::new_in_scope(rev, owner))
            };
            (sig, val)
        };
        let _ = signal.read();
        value
    }

    /// Subscribes the currently executing Dioxus component to any environment change and returns a
    /// cloned [`TraitEnvironment`].
    pub fn read_environment_reactive(&self) -> TraitEnvironment {
        let (signal, env) = {
            let mut guard = self.inner.borrow_mut();
            let owner = guard.owner_scope;
            let rev = guard.store.global_revision();
            let sig = *guard
                .any_signal
                .get_or_insert_with(|| Signal::new_in_scope(rev, owner));
            (sig, guard.store.environment().clone())
        };
        let _ = signal.read();
        env
    }

    /// Updates trait `K`, notifying ONLY subscribers of `K` (and `use_environment` listeners) iff
    /// the value actually changed.
    pub fn set<K: TraitKey>(&self, value: K::Value) -> bool {
        let (changed, target_sig, any_sig, rev, global_rev) = {
            let mut guard = self.inner.borrow_mut();
            let changed = guard.store.set::<K>(value);
            if !changed {
                return false;
            }
            let rev = guard.store.revision::<K>();
            let global_rev = guard.store.global_revision();
            let target_sig = if let Some(slot) = K::BUILTIN_SLOT {
                guard.builtin_signals[slot.index()]
            } else {
                guard.custom_signals.get(&TypeId::of::<K>()).copied()
            };
            (changed, target_sig, guard.any_signal, rev, global_rev)
        };
        if let Some(mut sig) = target_sig {
            sig.set(rev);
        }
        if let Some(mut any) = any_sig {
            any.set(global_rev);
        }
        changed
    }

    /// Updates viewport dimensions with breakpoint-only reactivity:
    /// - `SizeClass` signal fires ONLY on horizontal breakpoint crossings (`<600`, `600..1024`, `>=1024`).
    /// - `VerticalSizeClass` signal fires ONLY on vertical breakpoint crossings.
    /// - `Orientation` signal fires ONLY on `Portrait <-> Landscape` changes.
    /// - `ContainerSize` signal fires when `(width, height)` changes.
    pub fn update_viewport(&self, width: f32, height: f32) -> ViewportChangeMask {
        let (mask, to_notify, any_sig, global_rev) = {
            let mut guard = self.inner.borrow_mut();
            let mask = guard.store.update_viewport(width, height);
            if !mask.any_changed() {
                return mask;
            }
            let mut notify = Vec::with_capacity(4);
            if mask.size_class_changed
                && let Some(sig) = guard.builtin_signals[BuiltinTraitSlot::SizeClass.index()]
            {
                notify.push((
                    sig,
                    guard.store.builtin_revision(BuiltinTraitSlot::SizeClass),
                ));
            }
            if mask.vertical_size_class_changed
                && let Some(sig) =
                    guard.builtin_signals[BuiltinTraitSlot::VerticalSizeClass.index()]
            {
                notify.push((
                    sig,
                    guard
                        .store
                        .builtin_revision(BuiltinTraitSlot::VerticalSizeClass),
                ));
            }
            if mask.orientation_changed
                && let Some(sig) = guard.builtin_signals[BuiltinTraitSlot::Orientation.index()]
            {
                notify.push((
                    sig,
                    guard.store.builtin_revision(BuiltinTraitSlot::Orientation),
                ));
            }
            if mask.container_size_changed
                && let Some(sig) = guard.builtin_signals[BuiltinTraitSlot::ContainerSize.index()]
            {
                notify.push((
                    sig,
                    guard
                        .store
                        .builtin_revision(BuiltinTraitSlot::ContainerSize),
                ));
            }
            (mask, notify, guard.any_signal, guard.store.global_revision())
        };
        for (mut sig, rev) in to_notify {
            sig.set(rev);
        }
        if let Some(mut any) = any_sig {
            any.set(global_rev);
        }
        mask
    }

    /// Applies a batch of [`TraitOverrides`] onto the store, notifying only changed traits.
    pub fn apply_overrides(&self, overrides: &TraitOverrides) {
        let (to_notify, any_sig, global_rev) = {
            let mut guard = self.inner.borrow_mut();
            let (changed_builtins, changed_custom) = guard.store.apply_overrides(overrides);
            if changed_builtins == 0 && changed_custom.is_empty() {
                return;
            }
            let mut notify = Vec::new();
            if changed_builtins != 0 {
                for slot in BuiltinTraitSlot::ALL {
                    if (changed_builtins & slot.bit()) != 0
                        && let Some(sig) = guard.builtin_signals[slot.index()]
                    {
                        notify.push((sig, guard.store.builtin_revision(slot)));
                    }
                }
            }
            for tid in changed_custom {
                if let Some(&sig) = guard.custom_signals.get(&tid) {
                    notify.push((sig, guard.store.custom_revision(tid)));
                }
            }
            (notify, guard.any_signal, guard.store.global_revision())
        };
        for (mut sig, rev) in to_notify {
            sig.set(rev);
        }
        if let Some(mut any) = any_sig {
            any.set(global_rev);
        }
    }
}

/// Lexical scope context stored in Dioxus context, linking an optional root [`ReactiveTraitStore`]
/// with the stack of enclosing `<TraitScope>` override signals (outermost -> innermost).
#[derive(Clone, Default)]
struct TraitScopeContext {
    root_store: Option<ReactiveTraitStore>,
    layers: Vec<Signal<TraitOverrides>>,
}

/// Provides a live [`ReactiveTraitStore`] for the current component subtree.
pub fn use_provide_trait_store(init: impl FnOnce() -> TraitStore) -> ReactiveTraitStore {
    let ctx = use_hook(|| {
        let store = ReactiveTraitStore::new(init());
        provide_context(TraitScopeContext {
            root_store: Some(store),
            layers: Vec::new(),
        })
    });
    ctx.root_store
        .expect("use_provide_trait_store always initializes root_store")
}

/// Provides an already-constructed [`ReactiveTraitStore`] for the current subtree.
pub fn use_provide_existing_trait_store(store: ReactiveTraitStore) -> ReactiveTraitStore {
    let ctx = use_hook(|| {
        provide_context(TraitScopeContext {
            root_store: Some(store),
            layers: Vec::new(),
        })
    });
    ctx.root_store
        .expect("use_provide_existing_trait_store always initializes root_store")
}

/// Returns the active root [`ReactiveTraitStore`], if a provider is mounted in an ancestor scope.
pub fn use_trait_store() -> Option<ReactiveTraitStore> {
    try_consume_context::<TraitScopeContext>().and_then(|ctx| ctx.root_store)
}

/// Reads trait `K` from the nearest enclosing `<TraitScope>` or root [`ReactiveTraitStore`].
///
/// Subscribes strictly to `K`'s single per-trait revision counter when inherited from the root
/// store, guaranteeing breakpoint-only reactivity.
pub fn use_trait<K: TraitKey>() -> K::Value {
    let Some(ctx) = try_consume_context::<TraitScopeContext>() else {
        return K::default_value();
    };
    for layer_sig in ctx.layers.iter().rev() {
        let layer = layer_sig.read();
        if let Some(val) = layer.get::<K>() {
            return val;
        }
    }
    if let Some(ref store) = ctx.root_store {
        return store.read_reactive::<K>();
    }
    K::default_value()
}

/// Returns the full effective [`TraitEnvironment`] at the current point in the component tree.
pub fn use_environment() -> TraitEnvironment {
    let Some(ctx) = try_consume_context::<TraitScopeContext>() else {
        return TraitEnvironment::default();
    };
    let mut env = match ctx.root_store {
        Some(ref store) => store.read_environment_reactive(),
        None => TraitEnvironment::default(),
    };
    for layer_sig in &ctx.layers {
        let layer = layer_sig.read();
        env = layer.apply_onto(&env);
    }
    env
}

/// Root provider component supplying a [`ReactiveTraitStore`] to all descendants.
#[component]
pub fn TraitProvider(store: ReactiveTraitStore, children: Element) -> Element {
    let _ = use_provide_existing_trait_store(store);
    children
}

/// Overrides one or more built-in or custom [`TraitKey`]s for `children` without mutating parent
/// or sibling scopes (SwiftUI `.environment` / Compose `CompositionLocalProvider`).
#[component]
pub fn TraitScope(overrides: TraitOverrides, children: Element) -> Element {
    let parent_ctx = try_consume_context::<TraitScopeContext>().unwrap_or_default();
    let mut layer_sig = use_hook(|| Signal::new(overrides.clone()));
    if *layer_sig.peek() != overrides {
        layer_sig.set(overrides);
    }
    let _ = use_hook(|| {
        let mut layers = parent_ctx.layers.clone();
        layers.push(layer_sig);
        provide_context(TraitScopeContext {
            root_store: parent_ctx.root_store.clone(),
            layers,
        })
    });
    children
}

/// Renders `children` only when the active [`PlatformOs`] and/or [`HostToolkit`] match; otherwise
/// renders `fallback` (or an empty node).
#[component]
pub fn PlatformOnly(
    #[props(default)] os: Option<PlatformOs>,
    #[props(default)] toolkit: Option<HostToolkit>,
    #[props(into, default)] fallback: Option<Element>,
    children: Element,
) -> Element {
    let os_ok = match os {
        Some(expected) => use_trait::<PlatformOs>() == expected,
        None => true,
    };
    let toolkit_ok = match toolkit {
        Some(expected) => use_trait::<HostToolkit>() == expected,
        None => true,
    };
    if os_ok && toolkit_ok {
        children
    } else {
        fallback.unwrap_or_else(VNode::empty)
    }
}

/// Properties for [`TraitSwitch`].
#[derive(Props)]
pub struct TraitSwitchProps<K: TraitKey> {
    #[props(default)]
    pub cases: Vec<(K::Value, Element)>,
    #[props(into, default)]
    pub fallback: Option<Element>,
}

impl<K: TraitKey> Clone for TraitSwitchProps<K> {
    fn clone(&self) -> Self {
        Self {
            cases: self.cases.clone(),
            fallback: self.fallback.clone(),
        }
    }
}

impl<K: TraitKey> PartialEq for TraitSwitchProps<K> {
    fn eq(&self, other: &Self) -> bool {
        self.cases == other.cases && self.fallback == other.fallback
    }
}

/// Selects the first branch whose key value matches `use_trait::<K>()`, falling back to `fallback`.
#[allow(non_snake_case)]
pub fn TraitSwitch<K: TraitKey>(props: TraitSwitchProps<K>) -> Element {
    let current = use_trait::<K>();
    for (expected, elem) in props.cases {
        if expected == current {
            return elem;
        }
    }
    props.fallback.unwrap_or_else(VNode::empty)
}

/// Properties for [`TraitValueScope`].
#[derive(Props)]
pub struct TraitValueScopeProps<K: TraitKey> {
    pub value: K::Value,
    pub children: Element,
}

impl<K: TraitKey> Clone for TraitValueScopeProps<K> {
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
            children: self.children.clone(),
        }
    }
}

impl<K: TraitKey> PartialEq for TraitValueScopeProps<K> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value && self.children == other.children
    }
}

/// Convenience helper wrapping a single `TraitScope` override inline in `rsx!`.
#[allow(non_snake_case)]
pub fn TraitValueScope<K: TraitKey>(props: TraitValueScopeProps<K>) -> Element {
    let overrides = TraitOverrides::single::<K>(props.value);
    let children = props.children;
    rsx! {
        TraitScope {
            overrides,
            {children}
        }
    }
}
