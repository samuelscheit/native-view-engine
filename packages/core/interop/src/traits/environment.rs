//! Immutable copy-on-write `TraitEnvironment` and sparse scoped `TraitOverrides`.

use std::any::{Any, TypeId};
use std::fmt;
use std::sync::Arc;

use super::builtin::{BuiltinTraitSlot, BuiltinTraits, TraitKey};
use super::resolver::TraitResolver;
use super::types::{ContainerSize, Orientation, SizeClass};

/// Type-erased custom trait entry with dynamic `PartialEq` support.
#[derive(Clone)]
pub(crate) struct CustomTraitEntry {
    pub(crate) value: Arc<dyn Any + Send + Sync>,
    eq_fn: fn(&dyn Any, &dyn Any) -> bool,
}

impl CustomTraitEntry {
    #[expect(
        clippy::disallowed_methods,
        reason = "CustomTraitEntry type-erased trait value equality boundary"
    )]
    pub(crate) fn new<V: PartialEq + Send + Sync + 'static>(value: V) -> Self {
        Self {
            value: Arc::new(value),
            eq_fn: |a, b| match (a.downcast_ref::<V>(), b.downcast_ref::<V>()) {
                (Some(x), Some(y)) => x == y,
                _ => false,
            },
        }
    }

    #[inline]
    pub(crate) fn eq_entry(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.value, &other.value) || (self.eq_fn)(&*self.value, &*other.value)
    }
}

/// Persistent copy-on-write overlay map for user-defined custom [`TraitKey`]s.
///
/// Stored as a sorted `Vec<(TypeId, CustomTraitEntry)>` so small custom trait overlays avoid
/// hash-table overhead while remaining O(log N) lookup and cheap to clone-on-write.
#[derive(Clone, Default)]
pub(crate) struct CustomTraitMap {
    entries: Vec<(TypeId, CustomTraitEntry)>,
}

impl CustomTraitMap {
    #[inline]
    #[expect(
        clippy::disallowed_methods,
        reason = "CustomTraitMap resolves typed TraitKey::Value by TypeId"
    )]
    pub(crate) fn get<K: TraitKey>(&self) -> Option<&K::Value> {
        let tid = TypeId::of::<K>();
        let idx = self.entries.binary_search_by_key(&tid, |(k, _)| *k).ok()?;
        self.entries[idx].1.value.downcast_ref::<K::Value>()
    }

    #[inline]
    pub(crate) fn contains_type(&self, tid: TypeId) -> bool {
        self.entries.binary_search_by_key(&tid, |(k, _)| *k).is_ok()
    }

    #[inline]
    pub(crate) fn get_entry(&self, tid: TypeId) -> Option<&CustomTraitEntry> {
        let idx = self.entries.binary_search_by_key(&tid, |(k, _)| *k).ok()?;
        Some(&self.entries[idx].1)
    }

    /// Inserts or replaces `K`'s entry, returning `true` iff the value actually changed.
    pub(crate) fn insert<K: TraitKey>(&mut self, value: K::Value) -> bool {
        let tid = TypeId::of::<K>();
        let entry = CustomTraitEntry::new(value);
        match self.entries.binary_search_by_key(&tid, |(k, _)| *k) {
            Ok(idx) => {
                if self.entries[idx].1.eq_entry(&entry) {
                    false
                } else {
                    self.entries[idx].1 = entry;
                    true
                }
            }
            Err(idx) => {
                self.entries.insert(idx, (tid, entry));
                true
            }
        }
    }

    pub(crate) fn merge_from(&mut self, other: &Self) {
        for (tid, entry) in &other.entries {
            match self.entries.binary_search_by_key(tid, |(k, _)| *k) {
                Ok(idx) => self.entries[idx].1 = entry.clone(),
                Err(idx) => self.entries.insert(idx, (*tid, entry.clone())),
            }
        }
    }

    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[inline]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn iter_type_ids(&self) -> impl Iterator<Item = TypeId> + '_ {
        self.entries.iter().map(|(tid, _)| *tid)
    }
}

impl PartialEq for CustomTraitMap {
    fn eq(&self, other: &Self) -> bool {
        if self.entries.len() != other.entries.len() {
            return false;
        }
        self.entries
            .iter()
            .zip(other.entries.iter())
            .all(|((ka, va), (kb, vb))| ka == kb && va.eq_entry(vb))
    }
}

/// Immutable snapshot of all built-in and custom environment traits at a given point in the UI tree.
///
/// Cloning is zero-allocation when only built-in traits are used, and O(1) refcount-only when
/// custom [`TraitKey`]s are present.
#[derive(Clone, Default)]
pub struct TraitEnvironment {
    pub(crate) builtins: BuiltinTraits,
    pub(crate) custom: Option<Arc<CustomTraitMap>>,
}

impl fmt::Debug for TraitEnvironment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TraitEnvironment")
            .field("builtins", &self.builtins)
            .field("custom_count", &self.custom.as_ref().map_or(0, |m| m.len()))
            .finish()
    }
}

impl PartialEq for TraitEnvironment {
    fn eq(&self, other: &Self) -> bool {
        if !BuiltinTraitSlot::ALL
            .iter()
            .all(|&slot| self.builtins.slot_eq(&other.builtins, slot))
        {
            return false;
        }
        match (&self.custom, &other.custom) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b) || a == b,
            (Some(a), None) | (None, Some(a)) => a.is_empty(),
        }
    }
}

impl TraitEnvironment {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn from_builtins(builtins: BuiltinTraits) -> Self {
        Self {
            builtins,
            custom: None,
        }
    }

    /// Direct reference to the compact built-in traits struct (zero hashing/boxing).
    #[inline]
    pub fn builtins(&self) -> &BuiltinTraits {
        &self.builtins
    }

    /// Reads trait `K` in O(1). Built-in traits compile down to a direct struct field read.
    #[inline]
    pub fn get<K: TraitKey>(&self) -> K::Value {
        if K::BUILTIN_SLOT.is_some()
            && let Some(val) = K::read_builtin(&self.builtins)
        {
            return val;
        }
        if let Some(ref map) = self.custom
            && let Some(val) = map.get::<K>()
        {
            return val.clone();
        }
        K::default_value()
    }

    /// Returns a derived [`TraitEnvironment`] with trait `K` overridden to `value`.
    /// Unchanged custom maps are shared zero-copy via `Arc`.
    #[inline]
    pub fn with<K: TraitKey>(&self, value: K::Value) -> Self {
        let mut next = self.clone();
        next.set_in_place::<K>(value);
        next
    }

    /// Mutates trait `K` in place, returning `true` iff the effective value changed.
    #[inline]
    pub(crate) fn set_in_place<K: TraitKey>(&mut self, value: K::Value) -> bool {
        if K::BUILTIN_SLOT.is_some()
            && let Some(changed) = K::write_builtin(&mut self.builtins, &value)
        {
            return changed;
        }
        let map =
            Arc::make_mut(self.custom.get_or_insert_with(|| Arc::new(CustomTraitMap::default())));
        map.insert::<K>(value)
    }

    /// Returns a derived environment updated with viewport width/height breakpoints & container size.
    #[inline]
    pub fn with_viewport(&self, width: f32, height: f32) -> Self {
        let mut next = self.clone();
        next.builtins.size_class = SizeClass::from_width(width);
        next.builtins.vertical_size_class = SizeClass::from_height(height);
        next.builtins.orientation = Orientation::from_dimensions(width, height);
        next.builtins.container_size = ContainerSize::new(width, height);
        next
    }

    /// Returns a derived environment with an `@container`-style scoped [`ContainerSize`].
    #[inline]
    pub fn with_container_size(&self, width: f32, height: f32) -> Self {
        self.with::<ContainerSize>(ContainerSize::new(width, height))
    }

    /// Applies a [`TraitOverrides`] layer on top of `self`.
    #[inline]
    pub fn with_overrides(&self, overrides: &TraitOverrides) -> Self {
        overrides.apply_onto(self)
    }

    /// Starts a zero-allocation conditional trait resolver chain.
    ///
    /// First matching predicate wins; subsequent predicates are short-circuited.
    #[inline]
    pub fn resolve<T>(&self) -> TraitResolver<'_, T> {
        TraitResolver {
            env: self,
            resolved: None,
        }
    }

    /// Number of custom user-defined traits overridden in this environment.
    #[inline]
    pub fn custom_count(&self) -> usize {
        self.custom.as_ref().map_or(0, |m| m.len())
    }
}

/// Sparse set of trait overrides applied by a `<TraitScope>` or surface subtree.
///
/// Built-in overrides are tracked via a 32-bit bitmask (`O(1)` lookup and merge); custom user
/// overrides use a persistent copy-on-write `Arc<CustomTraitMap>`.
#[derive(Clone, Default)]
pub struct TraitOverrides {
    builtins: BuiltinTraits,
    builtin_mask: u32,
    custom: Option<Arc<CustomTraitMap>>,
}

impl fmt::Debug for TraitOverrides {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TraitOverrides")
            .field("builtin_mask", &self.builtin_mask)
            .field("custom_count", &self.custom.as_ref().map_or(0, |m| m.len()))
            .finish()
    }
}

impl PartialEq for TraitOverrides {
    fn eq(&self, other: &Self) -> bool {
        if self.builtin_mask != other.builtin_mask {
            return false;
        }
        if self.builtin_mask != 0 {
            for slot in BuiltinTraitSlot::ALL {
                if (self.builtin_mask & slot.bit()) != 0
                    && !self.builtins.slot_eq(&other.builtins, slot)
                {
                    return false;
                }
            }
        }
        match (&self.custom, &other.custom) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b) || a == b,
            (Some(a), None) | (None, Some(a)) => a.is_empty(),
        }
    }
}

impl TraitOverrides {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Convenience constructor for overriding a single trait key.
    #[inline]
    pub fn single<K: TraitKey>(value: K::Value) -> Self {
        Self::new().with::<K>(value)
    }

    /// Adds or replaces an override for trait `K` in builder style.
    #[inline]
    pub fn with<K: TraitKey>(mut self, value: K::Value) -> Self {
        self.set::<K>(value);
        self
    }

    /// Sets an override for trait `K` in place.
    #[inline]
    pub fn set<K: TraitKey>(&mut self, value: K::Value) {
        if let Some(slot) = K::BUILTIN_SLOT {
            let _ = K::write_builtin(&mut self.builtins, &value);
            self.builtin_mask |= slot.bit();
            return;
        }
        let map =
            Arc::make_mut(self.custom.get_or_insert_with(|| Arc::new(CustomTraitMap::default())));
        map.insert::<K>(value);
    }

    /// Convenience builder for overriding scoped `@container` dimensions.
    #[inline]
    pub fn with_container_size(self, width: f32, height: f32) -> Self {
        self.with::<ContainerSize>(ContainerSize::new(width, height))
    }

    /// Returns `true` if no traits are overridden.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.builtin_mask == 0 && self.custom.as_ref().is_none_or(|m| m.is_empty())
    }

    /// Returns `true` if trait `K` is explicitly overridden in this layer.
    #[inline]
    pub fn contains<K: TraitKey>(&self) -> bool {
        if let Some(slot) = K::BUILTIN_SLOT {
            return (self.builtin_mask & slot.bit()) != 0;
        }
        self.custom
            .as_ref()
            .is_some_and(|m| m.contains_type(TypeId::of::<K>()))
    }

    /// Reads the overridden value for trait `K`, or `None` if inherited from parent scope.
    #[inline]
    pub fn get<K: TraitKey>(&self) -> Option<K::Value> {
        if let Some(slot) = K::BUILTIN_SLOT {
            return ((self.builtin_mask & slot.bit()) != 0)
                .then(|| K::read_builtin(&self.builtins))
                .flatten();
        }
        self.custom.as_ref()?.get::<K>().cloned()
    }

    /// Merges a child override layer onto `self` (child entries shadow parent entries).
    pub fn merge(&self, child: &Self) -> Self {
        if child.is_empty() {
            return self.clone();
        }
        if self.is_empty() {
            return child.clone();
        }
        let mut out = self.clone();
        if child.builtin_mask != 0 {
            for slot in BuiltinTraitSlot::ALL {
                if (child.builtin_mask & slot.bit()) != 0 {
                    out.builtins.copy_slot_from(slot, &child.builtins);
                }
            }
            out.builtin_mask |= child.builtin_mask;
        }
        if let Some(ref child_custom) = child.custom
            && !child_custom.is_empty()
        {
            match out.custom {
                None => out.custom = Some(Arc::clone(child_custom)),
                Some(ref mut map_arc) => {
                    Arc::make_mut(map_arc).merge_from(child_custom);
                }
            }
        }
        out
    }

    /// Applies these overrides onto `base`, returning a derived [`TraitEnvironment`].
    pub fn apply_onto(&self, base: &TraitEnvironment) -> TraitEnvironment {
        if self.is_empty() {
            return base.clone();
        }
        let mut out = base.clone();
        if self.builtin_mask != 0 {
            for slot in BuiltinTraitSlot::ALL {
                if (self.builtin_mask & slot.bit()) != 0 {
                    out.builtins.copy_slot_from(slot, &self.builtins);
                }
            }
        }
        if let Some(ref custom) = self.custom
            && !custom.is_empty()
        {
            match out.custom {
                None => out.custom = Some(Arc::clone(custom)),
                Some(ref mut map_arc) => {
                    Arc::make_mut(map_arc).merge_from(custom);
                }
            }
        }
        out
    }

    #[inline]
    pub(crate) fn builtin_mask(&self) -> u32 {
        self.builtin_mask
    }

    pub(crate) fn custom_type_ids(&self) -> Vec<TypeId> {
        self.custom
            .as_ref()
            .map(|m| m.iter_type_ids().collect())
            .unwrap_or_default()
    }
}
