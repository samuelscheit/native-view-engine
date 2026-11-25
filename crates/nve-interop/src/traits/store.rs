//! Breakpoint-only reactive [`TraitStore`] with per-trait revision counters.

use std::any::TypeId;
use std::collections::BTreeMap;

use super::builtin::{BuiltinTraitSlot, BuiltinTraits, TraitKey, BUILTIN_TRAIT_COUNT};
use super::environment::{TraitEnvironment, TraitOverrides};
use super::types::{ContainerSize, Orientation, SizeClass, VerticalSizeClass};

/// Summary of which discrete viewport traits changed during [`TraitStore::update_viewport`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ViewportChangeMask {
    pub size_class_changed: bool,
    pub vertical_size_class_changed: bool,
    pub orientation_changed: bool,
    pub container_size_changed: bool,
}

impl ViewportChangeMask {
    /// Returns `true` if any discrete responsive breakpoint (`SizeClass`, `VerticalSizeClass`, or
    /// `Orientation`) changed.
    #[inline]
    pub const fn any_breakpoint_changed(self) -> bool {
        self.size_class_changed || self.vertical_size_class_changed || self.orientation_changed
    }

    /// Returns `true` if any viewport-derived trait changed (including continuous `ContainerSize`).
    #[inline]
    pub const fn any_changed(self) -> bool {
        self.any_breakpoint_changed() || self.container_size_changed
    }
}

/// Host-side mutable trait store tracking per-trait monotonic revision counters (`u64`) so UI
/// subscribers re-render strictly when the specific trait they read actually changes.
///
/// In particular, continuous 1px window resizes within the same breakpoint (e.g. `1200x800` ->
/// `1201x800`) do NOT bump [`SizeClass`] or [`Orientation`] revisions, while crossing a breakpoint
/// (e.g. `599x800` -> `600x800`) bumps only [`SizeClass`]'s revision.
#[derive(Debug, Clone)]
pub struct TraitStore {
    env: TraitEnvironment,
    viewport_width: f32,
    viewport_height: f32,
    builtin_revisions: [u64; BUILTIN_TRAIT_COUNT],
    custom_revisions: BTreeMap<TypeId, u64>,
    global_revision: u64,
}

impl Default for TraitStore {
    fn default() -> Self {
        Self::new()
    }
}

impl TraitStore {
    pub fn new() -> Self {
        Self::from_environment(TraitEnvironment::default())
    }

    pub fn from_builtins(builtins: BuiltinTraits) -> Self {
        Self::from_environment(TraitEnvironment::from_builtins(builtins))
    }

    pub fn from_environment(env: TraitEnvironment) -> Self {
        let cs = env.builtins.container_size;
        Self {
            env,
            viewport_width: cs.width,
            viewport_height: cs.height,
            builtin_revisions: [0; BUILTIN_TRAIT_COUNT],
            custom_revisions: BTreeMap::new(),
            global_revision: 0,
        }
    }

    /// Immutable snapshot of the current [`TraitEnvironment`].
    #[inline]
    pub fn environment(&self) -> &TraitEnvironment {
        &self.env
    }

    /// Current logical viewport dimensions `(width, height)` last reported to [`Self::update_viewport`].
    #[inline]
    pub fn viewport(&self) -> (f32, f32) {
        (self.viewport_width, self.viewport_height)
    }

    /// Reads trait `K` in O(1).
    #[inline]
    pub fn get<K: TraitKey>(&self) -> K::Value {
        self.env.get::<K>()
    }

    /// Returns the monotonic revision counter for trait `K` (`0` until changed).
    #[inline]
    pub fn revision<K: TraitKey>(&self) -> u64 {
        if let Some(slot) = K::BUILTIN_SLOT {
            return self.builtin_revisions[slot.index()];
        }
        self.custom_revisions
            .get(&TypeId::of::<K>())
            .copied()
            .unwrap_or(0)
    }

    /// Returns the revision counter for a specific built-in slot.
    #[inline]
    pub fn builtin_revision(&self, slot: BuiltinTraitSlot) -> u64 {
        self.builtin_revisions[slot.index()]
    }

    /// Returns the revision counter for a custom `TypeId` key.
    #[inline]
    pub fn custom_revision(&self, type_id: TypeId) -> u64 {
        self.custom_revisions.get(&type_id).copied().unwrap_or(0)
    }

    /// Global revision counter incremented whenever any trait value changes.
    #[inline]
    pub fn global_revision(&self) -> u64 {
        self.global_revision
    }

    /// Sets trait `K` to `value`, bumping its revision counter ONLY if the value actually changed.
    /// Returns `true` iff the value changed.
    #[inline]
    pub fn set<K: TraitKey>(&mut self, value: K::Value) -> bool {
        let changed = self.env.set_in_place::<K>(value);
        if changed {
            self.bump_key_revision::<K>();
        }
        changed
    }

    /// Updates viewport dimensions with breakpoint-only change tracking:
    /// - `SizeClass` revision bumps ONLY when `SizeClass::from_width(w)` changes.
    /// - `VerticalSizeClass` revision bumps ONLY when `SizeClass::from_height(h)` changes.
    /// - `Orientation` revision bumps ONLY when `Orientation::from_dimensions(w, h)` changes.
    /// - `ContainerSize` revision bumps when `(w, h)` changes (isolated to `ContainerSize` readers).
    pub fn update_viewport(&mut self, width: f32, height: f32) -> ViewportChangeMask {
        let w = if width.is_finite() { width.max(0.0) } else { 0.0 };
        let h = if height.is_finite() { height.max(0.0) } else { 0.0 };
        self.viewport_width = w;
        self.viewport_height = h;

        let size_class_changed = self.set::<SizeClass>(SizeClass::from_width(w));
        let vertical_size_class_changed = self.set::<VerticalSizeClass>(SizeClass::from_height(h));
        let orientation_changed = self.set::<Orientation>(Orientation::from_dimensions(w, h));
        let container_size_changed = self.set::<ContainerSize>(ContainerSize::new(w, h));

        ViewportChangeMask {
            size_class_changed,
            vertical_size_class_changed,
            orientation_changed,
            container_size_changed,
        }
    }

    /// Updates only [`ContainerSize`] (useful for `@container`-scoped surfaces).
    pub fn update_container_size(&mut self, width: f32, height: f32) -> bool {
        self.set::<ContainerSize>(ContainerSize::new(width, height))
    }

    /// Applies a batch of [`TraitOverrides`] onto the store, bumping revisions strictly for traits
    /// whose effective values changed. Returns `(changed_builtin_mask, changed_custom_type_ids)`.
    pub fn apply_overrides(&mut self, overrides: &TraitOverrides) -> (u32, Vec<TypeId>) {
        if overrides.is_empty() {
            return (0, Vec::new());
        }
        let next_env = overrides.apply_onto(&self.env);
        let mut changed_builtin_mask = 0u32;
        let mask = overrides.builtin_mask();
        if mask != 0 {
            for slot in BuiltinTraitSlot::ALL {
                if (mask & slot.bit()) != 0
                    && self.env.builtins.copy_slot_from(slot, &next_env.builtins)
                {
                    self.builtin_revisions[slot.index()] =
                        self.builtin_revisions[slot.index()].wrapping_add(1);
                    changed_builtin_mask |= slot.bit();
                }
            }
        }
        let mut changed_custom = Vec::new();
        for tid in overrides.custom_type_ids() {
            let old_entry = self.env.custom.as_ref().and_then(|m| m.get_entry(tid));
            let new_entry = next_env.custom.as_ref().and_then(|m| m.get_entry(tid));
            let same = match (old_entry, new_entry) {
                (Some(a), Some(b)) => a.eq_entry(b),
                (None, None) => true,
                _ => false,
            };
            if !same {
                let rev = self.custom_revisions.entry(tid).or_insert(0);
                *rev = rev.wrapping_add(1);
                changed_custom.push(tid);
            }
        }
        self.env.custom = next_env.custom;
        if changed_builtin_mask != 0 || !changed_custom.is_empty() {
            self.global_revision = self.global_revision.wrapping_add(1);
        }
        (changed_builtin_mask, changed_custom)
    }

    #[inline]
    fn bump_key_revision<K: TraitKey>(&mut self) {
        if let Some(slot) = K::BUILTIN_SLOT {
            let r = &mut self.builtin_revisions[slot.index()];
            *r = r.wrapping_add(1);
        } else {
            let r = self.custom_revisions.entry(TypeId::of::<K>()).or_insert(0);
            *r = r.wrapping_add(1);
        }
        self.global_revision = self.global_revision.wrapping_add(1);
    }
}
