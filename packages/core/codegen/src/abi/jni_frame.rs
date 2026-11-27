//! The packed JNI scene frame: one `long[]` header + one `ByteBuffer[]` of slabs that
//! `libnve_android_shell` fills per `acquireScene` and `dev.nve.host.RuntimeSceneReader` reads
//! (one JNI crossing per frame). Both sides are generated from this one layout
//! (`NveJniFrame.java`, `nve_jni_frame.generated.hpp`), so their indices cannot drift; the
//! `LAYOUT_HASH` both carry lets the JNI build reject a stale Java half.

use nve_abi::table::MAX_TABLE_SLOTS;
use nve_hash::xxh64;

/// Frame-wide `long[]` header words (`H_*`), in order.
pub const HEADER: &[&str] = &[
    "schema",
    "frame_id",
    "node_capacity",
    "dirty_count",
    "moved_count",
    "removed_count",
    "text_len",
    "children_len",
    "table_count",
    "scroll_count",
    "scene_epoch",
    "dirty_resource_count",
];
/// Per-table header words (`TH_*`), repeated `MAX_TABLES` times after `HEADER`.
pub const TABLE_HEADER: &[&str] =
    &["key", "slot", "words", "row_count", "dirty_count", "row_index_dir_len", "row_index_page_count"];
/// Frame-wide slabs (`S_*`), in order.
pub const SLABS: &[&str] = &[
    "rows",
    "rects",
    "text",
    "children",
    "dirty",
    "moved",
    "removed",
    "dirty_resources",
    "scroll_offsets",
];
/// Per-table slabs (`TS_*`), repeated `MAX_TABLES` times after `SLABS`.
pub const TABLE_SLABS: &[&str] = &["rows", "owners", "dirty_rows", "row_index_dir", "row_index_pages"];

/// One named index of a group (`H_SCENE_EPOCH = 10`).
#[derive(Debug)]
pub struct Slot {
    pub name: String,
    pub index: usize,
}

/// The rendered layout.
#[derive(Debug)]
pub struct JniFrameModel {
    pub header: Vec<Slot>,
    pub table_header: Vec<Slot>,
    pub slabs: Vec<Slot>,
    pub table_slabs: Vec<Slot>,
    pub max_tables: usize,
    pub header_len: usize,
    pub slabs_len: usize,
    /// `xxh64` of every group's names in order plus `max_tables`.
    pub layout_hash: u64,
}

fn group(prefix: &str, names: &[&str]) -> Vec<Slot> {
    names
        .iter()
        .enumerate()
        .map(|(index, n)| Slot { name: format!("{prefix}_{}", n.to_ascii_uppercase()), index })
        .collect()
}

impl JniFrameModel {
    pub fn new() -> Self {
        let mut desc = String::new();
        for g in [HEADER, TABLE_HEADER, SLABS, TABLE_SLABS] {
            desc.push_str(&g.join(","));
            desc.push(';');
        }
        desc.push_str(&MAX_TABLE_SLOTS.to_string());
        Self {
            header: group("H", HEADER),
            table_header: group("TH", TABLE_HEADER),
            slabs: group("S", SLABS),
            table_slabs: group("TS", TABLE_SLABS),
            max_tables: MAX_TABLE_SLOTS,
            header_len: HEADER.len() + MAX_TABLE_SLOTS * TABLE_HEADER.len(),
            slabs_len: SLABS.len() + MAX_TABLE_SLOTS * TABLE_SLABS.len(),
            layout_hash: xxh64(desc.as_bytes(), 0),
        }
    }
}

impl Default for JniFrameModel {
    fn default() -> Self {
        Self::new()
    }
}
