//! Recorded standalone style attributes (`width`, `color`, `opacity`, ...) and the recycling
//! pool that keeps their per-node lists and `String`s allocation-free in steady state: removed
//! nodes return their lists and strings, newly styled nodes (e.g. re-instantiated templates)
//! take them back.

/// Saved value of a standalone layout/visual/typography attribute so clearing `style="..."`
/// restores standalone attributes instead of wiping them.
#[derive(Debug, Clone)]
pub(crate) enum StandaloneStyleVal {
    Float(f64),
    Int(i64),
    Str(String),
}

/// Borrowed incoming [`StandaloneStyleVal`].
#[derive(Debug, Clone, Copy)]
pub(crate) enum StandaloneStyleRef<'v> {
    Float(f64),
    Int(i64),
    Str(&'v str),
}

/// One node's standalone style attributes in canonical CSS precedence order.
pub(crate) type StandaloneStyleList = Vec<(String, StandaloneStyleVal)>;

/// Recycled [`StandaloneStyleList`]s and `String`s (keys, values, inline `style` strings).
#[derive(Debug, Default)]
pub(crate) struct StylePool {
    lists: Vec<StandaloneStyleList>,
    strings: Vec<String>,
}

impl StylePool {
    /// An owned copy of `s` in a recycled buffer.
    pub(crate) fn string(&mut self, s: &str) -> String {
        let mut out = self.strings.pop().unwrap_or_default();
        out.clear();
        out.push_str(s);
        out
    }

    pub(crate) fn value(&mut self, val: StandaloneStyleRef<'_>) -> StandaloneStyleVal {
        match val {
            StandaloneStyleRef::Float(f) => StandaloneStyleVal::Float(f),
            StandaloneStyleRef::Int(i) => StandaloneStyleVal::Int(i),
            StandaloneStyleRef::Str(s) => StandaloneStyleVal::Str(self.string(s)),
        }
    }

    /// Overwrites `slot` with `val`, reusing an existing `Str` buffer in place.
    pub(crate) fn assign(&mut self, slot: &mut StandaloneStyleVal, val: StandaloneStyleRef<'_>) {
        if let (StandaloneStyleVal::Str(s), StandaloneStyleRef::Str(v)) = (&mut *slot, val) {
            s.clear();
            s.push_str(v);
            return;
        }
        let old = core::mem::replace(slot, self.value(val));
        self.recycle_value(old);
    }

    pub(crate) fn list(&mut self) -> StandaloneStyleList {
        self.lists.pop().unwrap_or_default()
    }

    pub(crate) fn recycle_string(&mut self, s: String) {
        self.strings.push(s);
    }

    pub(crate) fn recycle_entry(&mut self, (key, val): (String, StandaloneStyleVal)) {
        self.recycle_string(key);
        self.recycle_value(val);
    }

    pub(crate) fn recycle_list(&mut self, mut list: StandaloneStyleList) {
        for entry in list.drain(..) {
            self.recycle_entry(entry);
        }
        self.lists.push(list);
    }

    fn recycle_value(&mut self, val: StandaloneStyleVal) {
        if let StandaloneStyleVal::Str(s) = val {
            self.recycle_string(s);
        }
    }
}
