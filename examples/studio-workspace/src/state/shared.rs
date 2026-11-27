//! Copy-on-write collections for the workspace snapshot.
//!
//! Clones share one allocation, so history snapshots and reactive store projections are O(1),
//! and a collection nobody edited compares equal by pointer without walking its elements.
//! Edits go through [`Shared::make_mut`], which clones the elements only while shared.

use std::fmt;
use std::ops::Deref;
use std::sync::Arc;

/// A copy-on-write `Vec<T>` (see the module docs).
pub struct Shared<T>(Arc<Vec<T>>);

impl<T> Shared<T> {
    /// `true` when both handles share one allocation (unchanged since one was cloned from the
    /// other). Never walks the elements.
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl<T: Clone> Shared<T> {
    /// Mutable access, cloning the elements first if another handle shares them.
    pub fn make_mut(&mut self) -> &mut Vec<T> {
        Arc::make_mut(&mut self.0)
    }
}

impl<T> Clone for Shared<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T> Default for Shared<T> {
    fn default() -> Self {
        Self(Arc::new(Vec::new()))
    }
}

impl<T> Deref for Shared<T> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        &self.0
    }
}

impl<T> From<Vec<T>> for Shared<T> {
    fn from(items: Vec<T>) -> Self {
        Self(Arc::new(items))
    }
}

impl<'a, T> IntoIterator for &'a Shared<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl<T> FromIterator<T> for Shared<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self(Arc::new(iter.into_iter().collect()))
    }
}

/// Pointer equality first; element equality only for distinct allocations.
impl<T: PartialEq> PartialEq for Shared<T> {
    fn eq(&self, other: &Self) -> bool {
        self.ptr_eq(other) || self.0 == other.0
    }
}

impl<T: Eq> Eq for Shared<T> {}

impl<T: fmt::Debug> fmt::Debug for Shared<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.0.iter()).finish()
    }
}
