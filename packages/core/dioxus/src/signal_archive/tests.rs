use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use dioxus_signals::WritableExt;
use nve_abi::{NveMapEntry, NveValue};
use nve_engine::{NativeStateStore, StateStoreConfig};

use super::codec::{write_header, HEADER_SIZE};
use super::*;

static TEST_DIR_SEQ: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let n = TEST_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "nve_sig_archive_{label}_{}_{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn roundtrip_all_signal_value_types_and_bin_value() {
    let map_entries = [
        NveMapEntry::new("theme", NveValue::from_str("midnight")),
        NveMapEntry::new("zoom", NveValue::f64(1.25)),
        NveMapEntry::new("count", NveValue::i64(-42)),
    ];
    let nve_val = NveValue::map(&map_entries);
    // SAFETY: `map_entries` and nested backing slices outlive the synchronous `from_nve_value` call.
    let bin_val = SignalValue::from_nve_value(unsafe { nve_val.as_ref() }).unwrap();

    let mut table = SignalTableSnapshot::new(7);
    table.set_named("Root", "empty", SignalValue::Null);
    table.set_named("Root", "dark_mode", SignalValue::Bool(true));
    table.set_named("Counter", "delta", SignalValue::I64(-9_876_543_210));
    table.set_named("Counter", "generation", SignalValue::U64(u64::MAX - 7));
    table.set_named("Canvas", "scale", SignalValue::F64(std::f64::consts::PI));
    table.set_named("Canvas", "nan_guard", SignalValue::F64(f64::NAN));
    table.set_named("Editor", "title", SignalValue::Str("Aether Studio 🚀".into()));
    table.set_named("Editor", "raw", SignalValue::Bytes(vec![0, 1, 2, 254, 255]));
    let bin_id = table.set_named("Settings", "prefs", bin_val);

    let encoded = SignalArchive::encode_snapshot(&table).unwrap();
    assert_eq!(&encoded[0..8], &NVE_SIGNAL_ARCHIVE_MAGIC);

    let decoded = SignalArchive::decode_snapshot(&encoded).unwrap();
    assert_eq!(decoded, table);
    assert_eq!(decoded.schema_version(), 7);

    let view = decoded.get(bin_id).unwrap().as_bin_value().unwrap().unwrap();
    assert_eq!(view.get("theme").and_then(|v| v.as_str()), Some("midnight"));
    assert_eq!(view.get("zoom").and_then(|v| v.as_f64()), Some(1.25));
    assert_eq!(view.get("count").and_then(|v| v.as_i64()), Some(-42));

    // Slot ID separator prevents prefix/suffix collisions.
    assert_ne!(
        signal_slot_id("app/nav", "tab"),
        signal_slot_id("app/navt", "ab")
    );
}

#[test]
fn incremental_delta_folding_and_arena_compaction_equivalence() {
    let dir = TempDir::new("compact_eq");
    let arena_path = dir.path().join("state.nvemmap");

    let mut base = SignalTableSnapshot::new(2);
    base.set_named("App", "count", SignalValue::I64(1));
    base.set_named("App", "tab", SignalValue::Str("board".into()));
    base.set_named("App", "ephemeral", SignalValue::Bool(true));

    let mut store = NativeStateStore::open(&arena_path, 2, StateStoreConfig::default()).unwrap();
    SignalArchive::save_to_store(&mut store, &base).unwrap();

    // Delta 1: mutate count, add filter.
    let mut step1 = base.clone();
    step1.set_named("App", "count", SignalValue::I64(2));
    step1.set_named("App", "filter", SignalValue::Str("open".into()));
    let d1 = step1.diff_since(&base);
    assert_eq!(d1.len(), 2);
    let d1_bytes = d1.encode().unwrap();

    // Delta 2: mutate count again, remove ephemeral, update tab.
    let mut step2 = step1.clone();
    step2.set_named("App", "count", SignalValue::I64(42));
    step2.set_named("App", "tab", SignalValue::Str("metrics".into()));
    step2.remove(signal_slot_id("App", "ephemeral"));
    let d2 = step2.diff_since(&step1);
    assert_eq!(d2.len(), 3);
    let d2_bytes = d2.encode().unwrap();

    // Unchanged diff produces empty delta.
    assert!(step2.diff_since(&step2).is_empty());

    // Fold deltas onto base loaded from MmapOffsetArena.
    let mut folded = SignalArchive::load_from_store(&store).unwrap();
    folded.apply_delta_bytes(&d1_bytes).unwrap();
    folded.apply_delta_bytes(&d2_bytes).unwrap();
    assert_eq!(folded, step2);

    SignalArchive::save_to_store(&mut store, &folded).unwrap();
    store.compact().unwrap();
    drop(store);

    let reopened = NativeStateStore::open(&arena_path, 2, StateStoreConfig::default()).unwrap();
    let from_compacted = SignalArchive::load_from_store(&reopened).unwrap();
    assert_eq!(from_compacted, folded);
    assert_eq!(from_compacted, step2);
}

#[test]
fn registry_handles_schema_evolution_added_removed_and_retyped_slots() {
    let dir = TempDir::new("schema_evo");
    let arena_path = dir.path().join("evo.nvemmap");

    // App v1 state:
    let v1_count = Rc::new(RefCell::new(10i32));
    let v1_theme = Rc::new(RefCell::new("dark".to_string()));
    let v1_legacy_flag = Rc::new(RefCell::new(true));
    let v1_retyped_zoom = Rc::new(RefCell::new(2i64)); // i64 in v1, becomes f64 in v2!

    let mut reg_v1 = SignalRegistry::new(1);
    {
        let c = Rc::clone(&v1_count);
        let c2 = Rc::clone(&v1_count);
        reg_v1.register_slot(
            "App",
            "count",
            move || *c.borrow(),
            move |v| *c2.borrow_mut() = v,
        );
    }
    {
        let t = Rc::clone(&v1_theme);
        let t2 = Rc::clone(&v1_theme);
        reg_v1.register_slot(
            "App",
            "theme",
            move || t.borrow().clone(),
            move |v| *t2.borrow_mut() = v,
        );
    }
    {
        let f = Rc::clone(&v1_legacy_flag);
        let f2 = Rc::clone(&v1_legacy_flag);
        reg_v1.register_slot(
            "App",
            "legacy_flag",
            move || *f.borrow(),
            move |v| *f2.borrow_mut() = v,
        );
    }
    {
        let z = Rc::clone(&v1_retyped_zoom);
        let z2 = Rc::clone(&v1_retyped_zoom);
        reg_v1.register_slot(
            "Canvas",
            "zoom",
            move || *z.borrow(),
            move |v| *z2.borrow_mut() = v,
        );
    }

    let mut store = NativeStateStore::open(&arena_path, 1, StateStoreConfig::default()).unwrap();
    reg_v1.persist_to_store(&mut store).unwrap();

    // Mutate count and persist incrementally.
    *v1_count.borrow_mut() = 77;
    assert!(reg_v1.persist_if_dirty_to_store(&mut store).unwrap().is_some());
    // Clean persist returns None.
    assert!(reg_v1.persist_if_dirty_to_store(&mut store).unwrap().is_none());

    // App v2 upgrades schema:
    // - keeps `App/count` (i32) and `App/theme` (String)
    // - removes `App/legacy_flag`
    // - re-types `Canvas/zoom` from i64 -> f64 (default 1.5)
    // - adds new `App/sidebar_open` (bool, default true)
    let v2_count = Rc::new(RefCell::new(0i32));
    let v2_theme = Rc::new(RefCell::new("light".to_string()));
    let v2_zoom = Rc::new(RefCell::new(1.5f64));
    let v2_sidebar = Rc::new(RefCell::new(true));

    let mut reg_v2 = SignalRegistry::new(2);
    {
        let c = Rc::clone(&v2_count);
        let c2 = Rc::clone(&v2_count);
        reg_v2.register_slot(
            "App",
            "count",
            move || *c.borrow(),
            move |v| *c2.borrow_mut() = v,
        );
    }
    {
        let t = Rc::clone(&v2_theme);
        let t2 = Rc::clone(&v2_theme);
        reg_v2.register_slot(
            "App",
            "theme",
            move || t.borrow().clone(),
            move |v| *t2.borrow_mut() = v,
        );
    }
    {
        let z = Rc::clone(&v2_zoom);
        let z2 = Rc::clone(&v2_zoom);
        reg_v2.register_slot(
            "Canvas",
            "zoom",
            move || *z.borrow(),
            move |v| *z2.borrow_mut() = v,
        );
    }
    {
        let s = Rc::clone(&v2_sidebar);
        let s2 = Rc::clone(&v2_sidebar);
        reg_v2.register_slot(
            "App",
            "sidebar_open",
            move || *s.borrow(),
            move |v| *s2.borrow_mut() = v,
        );
    }

    let report = reg_v2.hydrate_from_store(&store).unwrap();

    assert_eq!(report.source_schema_version, 1);
    assert_eq!(report.hydrated_slots, 2);
    assert_eq!(report.skipped_unknown_slots, 1); // legacy_flag
    assert_eq!(report.skipped_type_mismatches, 1); // zoom i64 -> f64
    assert_eq!(report.untouched_new_slots, 1); // sidebar_open

    // Matching slots restored (including updated count=77); incompatible/new slots kept safe defaults.
    assert_eq!(*v2_count.borrow(), 77);
    assert_eq!(*v2_theme.borrow(), "dark");
    assert_eq!(*v2_zoom.borrow(), 1.5);
    assert!(*v2_sidebar.borrow());
}

#[test]
fn corrupt_and_truncated_archives_are_rejected() {
    let mut table = SignalTableSnapshot::new(1);
    table.set_named("A", "x", SignalValue::I64(123));
    table.set_named("A", "s", SignalValue::Str("hello".into()));
    let valid = table.encode().unwrap();

    // Every strict prefix must fail decoding.
    for len in 0..valid.len() {
        assert!(
            SignalTableSnapshot::decode(&valid[..len]).is_err(),
            "prefix of len {len} unexpectedly decoded"
        );
    }

    // Trailing bytes rejected.
    let mut with_trailing = valid.clone();
    with_trailing.push(0);
    assert!(SignalTableSnapshot::decode(&with_trailing).is_err());

    // Single bit flip in payload triggers checksum mismatch.
    let mut flipped = valid.clone();
    let last = flipped.len() - 1;
    flipped[last] ^= 0x01;
    assert!(matches!(
        SignalTableSnapshot::decode(&flipped),
        Err(SignalArchiveError::ChecksumMismatch { .. })
    ));

    // Bad magic rejected.
    let mut bad_magic = valid.clone();
    bad_magic[0..8].copy_from_slice(b"BADMAGIC");
    assert!(matches!(
        SignalTableSnapshot::decode(&bad_magic),
        Err(SignalArchiveError::InvalidMagic(_))
    ));

    // Corrupt BinValue inside archive rejected even if checksum is recomputed.
    let mut bad_bin = vec![0u8; HEADER_SIZE];
    bad_bin.extend_from_slice(&42u64.to_le_bytes());
    bad_bin.push(SignalValueTag::BinValue as u8);
    bad_bin.extend_from_slice(&2u32.to_le_bytes());
    bad_bin.extend_from_slice(&[0xFF, 0x00]); // invalid NveValueTag
    write_header(&mut bad_bin, NVE_SIGNAL_ARCHIVE_MAGIC, 1, 1);
    assert!(matches!(
        SignalTableSnapshot::decode(&bad_bin),
        Err(SignalArchiveError::InvalidBinValue(_))
    ));

    // Truncated/corrupted delta rejected.
    let mut next = table.clone();
    next.set_named("A", "x", SignalValue::I64(999));
    let delta_bytes = next.diff_since(&table).encode().unwrap();
    for len in 0..delta_bytes.len() {
        assert!(SignalDelta::decode(&delta_bytes[..len]).is_err());
    }
    let mut bad_delta = delta_bytes.clone();
    let last_d = bad_delta.len() - 1;
    bad_delta[last_d] ^= 0x80;
    assert!(matches!(
        SignalDelta::decode(&bad_delta),
        Err(SignalArchiveError::ChecksumMismatch { .. })
    ));
}

#[test]
fn live_dioxus_signal_registers_snapshots_and_hydrates() {
    use dioxus_core::Element;
    use dioxus_core_macro::rsx;
    use std::sync::Mutex;

    static CAPTURED: Mutex<Option<Vec<u8>>> = Mutex::new(None);
    static HYDRATED_VAL: Mutex<Option<i32>> = Mutex::new(None);

    fn app() -> Element {
        let mut count = crate::use_signal(|| 5i32);
        let mut reg = SignalRegistry::new(1);
        reg.register_signal("CounterApp", "count", count);

        count.set(19);
        let snap = reg.encode_full().unwrap();
        *CAPTURED.lock().unwrap() = Some(snap.clone());

        count.set(0);
        let report = reg.hydrate_from_bytes(&snap).unwrap();
        assert_eq!(report.hydrated_slots, 1);
        *HYDRATED_VAL.lock().unwrap() = Some(count());

        rsx! {}
    }

    let mut surface = crate::DioxusSurface::new(1, app);
    surface.reconcile(1);

    assert!(CAPTURED.lock().unwrap().is_some());
    assert_eq!(*HYDRATED_VAL.lock().unwrap(), Some(19));
}
