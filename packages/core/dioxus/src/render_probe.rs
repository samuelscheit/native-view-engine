//! Per-component render counters for re-render regression tests.
//!
//! Components call [`record_render`] with their name at the top of their body. Nothing is counted
//! unless a [`RenderProbe`] is live on the rendering thread, so production renders pay one
//! thread-local flag check.
//!
//! ```ignore
//! let probe = RenderProbe::start();
//! harness.type_text("composer", "hi");
//! assert_eq!(probe.take().get("Sidebar"), None); // the sidebar never re-rendered
//! ```

use std::cell::RefCell;
use std::collections::BTreeMap;

thread_local! {
    static COUNTS: RefCell<Option<BTreeMap<&'static str, u32>>> = const { RefCell::new(None) };
}

/// Counts one render of component `name` while a [`RenderProbe`] is live on this thread.
#[inline]
pub fn record_render(name: &'static str) {
    COUNTS.with(|counts| {
        if let Some(counts) = counts.borrow_mut().as_mut() {
            *counts.entry(name).or_default() += 1;
        }
    });
}

/// Records renders on the current thread from [`Self::start`] until dropped.
#[must_use = "renders are only counted while the probe is alive"]
pub struct RenderProbe(());

impl RenderProbe {
    /// Starts counting (from zero) on the current thread.
    pub fn start() -> Self {
        COUNTS.with(|counts| *counts.borrow_mut() = Some(BTreeMap::new()));
        Self(())
    }

    /// Renders per component since the start or the previous `take`, then restarts from zero.
    pub fn take(&self) -> BTreeMap<&'static str, u32> {
        COUNTS.with(|counts| counts.borrow_mut().replace(BTreeMap::new()).unwrap_or_default())
    }
}

impl Drop for RenderProbe {
    fn drop(&mut self) {
        COUNTS.with(|counts| *counts.borrow_mut() = None);
    }
}
