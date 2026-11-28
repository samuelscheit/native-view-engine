//! Recorded numeric standalone style attributes (`width`, `padding`, `opacity`, `z-index`, ...)
//! and the recycling pool that keeps the writer's per-node side-table buffers (standalone style
//! lists and `TableAttr` lists) allocation-free in steady state.

use crate::TableAttr;

/// Saved value of a numeric standalone layout/visual/typography attribute so updating or clearing
/// `nve_style` preserves standalone attributes in canonical precedence order.
#[derive(Debug, Clone, Copy)]
pub(crate) enum StandaloneStyleVal {
    Float(f64),
    Int(i64),
}

/// One node's numeric standalone style attributes in canonical shorthand/longhand precedence order.
/// Keys are Dioxus's static attribute names (`&'static str`), requiring zero heap strings.
pub(crate) type StandaloneStyleList = Vec<(&'static str, StandaloneStyleVal)>;

/// Lists above this many entries (capacity) are dropped on recycle.
const MAX_LIST_CAP: usize = 32;

/// One node's active `TableAttr` writers by attribute name.
pub(crate) type TableAttrList = Vec<(&'static str, TableAttr)>;

/// Pooled buffers per kind: enough to remount a few hundred styled nodes without allocating,
/// while an unmounted large view never pins its whole style footprint.
const MAX_POOLED: usize = 512;

// Canonical property bits for standalone numeric attributes, partitioned by core style table.
const BIT_WIDTH: u32 = 1 << 0;
const BIT_HEIGHT: u32 = 1 << 1;
const BIT_MIN_WIDTH: u32 = 1 << 2;
const BIT_MIN_HEIGHT: u32 = 1 << 3;
const BIT_MAX_WIDTH: u32 = 1 << 4;
const BIT_MAX_HEIGHT: u32 = 1 << 5;
const BIT_FLEX_GROW: u32 = 1 << 6;
const BIT_FLEX_SHRINK: u32 = 1 << 7;
const BIT_FLEX_BASIS: u32 = 1 << 8;
const BIT_ROW_GAP: u32 = 1 << 9;
const BIT_COLUMN_GAP: u32 = 1 << 10;
const BIT_PADDING_H: u32 = 1 << 11;
const BIT_PADDING_V: u32 = 1 << 12;
const BIT_MARGIN: u32 = 1 << 13;
const LAYOUT_BITS: u32 = (1 << 14) - 1;

const BIT_BORDER_WIDTH: u32 = 1 << 16;
const BIT_BORDER_RADIUS: u32 = 1 << 17;
const BIT_OPACITY: u32 = 1 << 18;
const BIT_Z_INDEX: u32 = 1 << 19;
const VISUAL_BITS: u32 = BIT_BORDER_WIDTH | BIT_BORDER_RADIUS | BIT_OPACITY | BIT_Z_INDEX;

const BIT_FONT_SIZE: u32 = 1 << 20;
const TYPOGRAPHY_BITS: u32 = BIT_FONT_SIZE;

/// Canonical property bitmask for a supported numeric standalone style attribute.
#[inline]
pub(crate) fn numeric_attr_bits(key: &str) -> u32 {
    match key {
        "width" => BIT_WIDTH,
        "height" => BIT_HEIGHT,
        "min-width" | "minWidth" => BIT_MIN_WIDTH,
        "min-height" | "minHeight" => BIT_MIN_HEIGHT,
        "max-width" | "maxWidth" => BIT_MAX_WIDTH,
        "max-height" | "maxHeight" => BIT_MAX_HEIGHT,
        "flex" | "flex-grow" | "flexGrow" => BIT_FLEX_GROW,
        "flex-shrink" | "flexShrink" => BIT_FLEX_SHRINK,
        "flex-basis" | "flexBasis" => BIT_FLEX_BASIS,
        "gap" => BIT_ROW_GAP | BIT_COLUMN_GAP,
        "row-gap" | "rowGap" => BIT_ROW_GAP,
        "column-gap" | "columnGap" => BIT_COLUMN_GAP,
        "padding" => BIT_PADDING_H | BIT_PADDING_V,
        "padding-horizontal" | "paddingHorizontal" => BIT_PADDING_H,
        "padding-vertical" | "paddingVertical" => BIT_PADDING_V,
        "margin" => BIT_MARGIN,
        "border-width" | "borderWidth" => BIT_BORDER_WIDTH,
        "border-radius" | "borderRadius" => BIT_BORDER_RADIUS,
        "opacity" => BIT_OPACITY,
        "z-index" | "zIndex" => BIT_Z_INDEX,
        "font-size" | "fontSize" => BIT_FONT_SIZE,
        _ => 0,
    }
}

/// Precedence rank so shorthands (`flex`, `gap`, `padding`) always sort before longhands
/// (`flexGrow`, `rowGap`, `paddingHorizontal`, ...) regardless of set/clear history.
#[inline]
pub(crate) fn numeric_attr_rank(key: &str) -> u8 {
    match key {
        "flex" | "gap" | "padding" => 0,
        _ => 1,
    }
}

/// Maps a [`numeric_attr_bits`] mask to its `nve_tables::STYLE_LAYER_*` bitmask.
#[inline]
pub(crate) const fn numeric_bits_to_layers(bits: u32) -> u8 {
    let mut layers = 0u8;
    if bits & LAYOUT_BITS != 0 {
        layers |= nve_tables::STYLE_LAYER_LAYOUT;
    }
    if bits & VISUAL_BITS != 0 {
        layers |= nve_tables::STYLE_LAYER_VISUAL;
    }
    if bits & TYPOGRAPHY_BITS != 0 {
        layers |= nve_tables::STYLE_LAYER_TYPOGRAPHY;
    }
    layers
}

/// Returns the `STYLE_LAYER_*` bitmask for a standalone numeric style attribute (`0` if unknown).
#[inline]
pub(crate) fn numeric_attr_layer(key: &str) -> u8 {
    numeric_bits_to_layers(numeric_attr_bits(key))
}

/// Recycled [`StandaloneStyleList`]s and [`TableAttrList`]s. Bounded in count and per-buffer
/// capacity; zero-capacity buffers are never pooled (they own nothing).
#[derive(Debug, Default)]
pub(crate) struct BufferPool {
    lists: Vec<StandaloneStyleList>,
    table_lists: Vec<TableAttrList>,
}

/// Returns `v` (empty) to `pool` unless it owns nothing, is oversized, or the pool is full.
fn give_list<T>(pool: &mut Vec<Vec<T>>, mut v: Vec<T>) {
    if v.capacity() != 0 && v.capacity() <= MAX_LIST_CAP && pool.len() < MAX_POOLED {
        v.clear();
        pool.push(v);
    }
}

impl BufferPool {
    pub(crate) fn list(&mut self) -> StandaloneStyleList {
        self.lists.pop().unwrap_or_default()
    }

    pub(crate) fn recycle_list(&mut self, list: StandaloneStyleList) {
        give_list(&mut self.lists, list);
    }

    pub(crate) fn table_list(&mut self) -> TableAttrList {
        self.table_lists.pop().unwrap_or_default()
    }

    pub(crate) fn recycle_table_list(&mut self, list: TableAttrList) {
        give_list(&mut self.table_lists, list);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_is_bounded_and_skips_empty_or_oversized_buffers() {
        let mut pool = BufferPool::default();
        // Zero-capacity buffers own nothing: never pooled.
        pool.recycle_list(Vec::new());
        pool.recycle_table_list(Vec::new());
        assert!(pool.lists.is_empty() && pool.table_lists.is_empty());

        // Oversized buffers are dropped, not pinned.
        pool.recycle_list(Vec::with_capacity(MAX_LIST_CAP + 1));
        pool.recycle_table_list(Vec::with_capacity(MAX_LIST_CAP + 1));
        assert!(pool.lists.is_empty() && pool.table_lists.is_empty());

        // Lists are emptied and reused on take.
        let mut list = pool.list();
        list.push(("padding", StandaloneStyleVal::Float(8.0)));
        let cap = list.capacity();
        pool.recycle_list(list);
        let reused = pool.list();
        assert!(reused.is_empty() && reused.capacity() == cap);

        let mut tlist = pool.table_list();
        tlist.push(("accent", crate::table_attr_fn(|_, _| {})));
        let tcap = tlist.capacity();
        pool.recycle_table_list(tlist);
        let treused = pool.table_list();
        assert!(treused.is_empty() && treused.capacity() == tcap);

        // Count bound.
        for _ in 0..MAX_POOLED + 10 {
            pool.recycle_list(Vec::with_capacity(4));
            pool.recycle_table_list(Vec::with_capacity(4));
        }
        assert_eq!((pool.lists.len(), pool.table_lists.len()), (MAX_POOLED, MAX_POOLED));
    }

    #[test]
    fn numeric_attr_metadata_ranks_shorthands_before_longhands_and_partitions_layers() {
        assert!(numeric_attr_rank("padding") < numeric_attr_rank("paddingHorizontal"));
        assert!(numeric_attr_rank("gap") < numeric_attr_rank("rowGap"));
        assert!(numeric_attr_rank("flex") < numeric_attr_rank("flexGrow"));
        assert_eq!(numeric_attr_layer("width"), nve_tables::STYLE_LAYER_LAYOUT);
        assert_eq!(numeric_attr_layer("opacity"), nve_tables::STYLE_LAYER_VISUAL);
        assert_eq!(numeric_attr_layer("z-index"), nve_tables::STYLE_LAYER_VISUAL);
        assert_eq!(numeric_attr_layer("font-size"), nve_tables::STYLE_LAYER_TYPOGRAPHY);
        assert_eq!(numeric_attr_layer("unknown"), 0);
    }
}
