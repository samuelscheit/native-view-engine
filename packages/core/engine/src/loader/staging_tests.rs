//! Unix unit tests for [`super::StagedDylib`], [`super::InPlaceClaim`], and private staging dirs.

use super::{
    InPlaceClaim, LoaderError, StagedDylib, open_private_dir, stage_private_copy, staging_dir_in,
};
use std::os::unix::fs::PermissionsExt;

#[test]
fn copies_stage_in_a_per_user_private_dir_with_0600_mode() {
    let root = scratch("root");
    std::fs::create_dir(&root).expect("test fs op");
    let src = root.join("libapp.so");
    std::fs::write(&src, b"code").expect("test fs op");
    let copy = stage_private_copy(&root, &src, "mods", "g1").expect("stage");
    let dir = staging_dir_in(&root, "mods");
    // SAFETY: `geteuid` has no preconditions.
    assert_eq!(dir, root.join(format!("mods-{}", unsafe { libc::geteuid() })));
    assert_eq!(copy.parent(), Some(dir.as_path()));
    assert_eq!(std::fs::read(&copy).expect("test fs op"), b"code");
    assert_eq!(mode(&dir), 0o700);
    assert_eq!(mode(&copy), 0o600, "staged copy created 0o600 via openat");
    std::fs::remove_dir_all(&root).expect("test fs op");
}

#[test]
fn in_place_claim_allows_unchanged_reopens_and_rejects_modified_reloads() {
    let dir = scratch("claim");
    std::fs::create_dir(&dir).expect("test fs op");
    let (file, alias, atomic_tmp, replacement) = (
        dir.join("libapp.so"),
        dir.join("libapp-alias.so"),
        dir.join("libapp.so.tmp"),
        dir.join("libapp-v2.so"),
    );
    std::fs::write(&file, b"bundle-v1").expect("test fs op");
    std::fs::hard_link(&file, &alias).expect("test fs op");

    // (a) Unchanged bundle re-opens succeed both concurrently (multi-surface) and after drop
    //     (Activity recreation / detachApp -> attachApp), including through hard-link aliases.
    let mut first = InPlaceClaim::new(&file).expect("initial in-place claim");
    let mut concurrent = InPlaceClaim::new(&file).expect("concurrent unchanged claim");
    first.seal();
    let mut concurrent_alias = InPlaceClaim::new(&alias).expect("unchanged hard-link alias");
    concurrent.seal();
    concurrent_alias.seal();
    drop(first);
    drop(concurrent);
    drop(concurrent_alias);

    let mut after_drop = InPlaceClaim::new(&file).expect("re-open unchanged file after drop");
    after_drop.seal();
    drop(after_drop);

    // (b) Modifying the in-place file on disk (same inode, changed bytes — even same length)
    //     fails with `AlreadyLoadedInPlace` on both the original path and its hard-link alias.
    std::fs::write(&file, b"bundle-v2").expect("test fs op");
    for again in [&file, &alias] {
        assert!(
            matches!(InPlaceClaim::new(again), Err(LoaderError::AlreadyLoadedInPlace(_))),
            "modified inode must be rejected after sealed load"
        );
    }

    // Atomic rename replacement at the same canonical path (new inode, changed bytes) is also
    // rejected because `dlopen` caches mapped images by path / install name.
    std::fs::write(&atomic_tmp, b"bundle-v3-longer").expect("test fs op");
    std::fs::rename(&atomic_tmp, &file).expect("test fs op");
    assert!(
        matches!(InPlaceClaim::new(&file), Err(LoaderError::AlreadyLoadedInPlace(_))),
        "atomic replacement with changed bytes at same canonical path must be rejected"
    );

    // A freshly created versioned file (distinct inode & canonical path) still loads cleanly.
    std::fs::write(&replacement, b"bundle-v3-longer").expect("test fs op");
    let mut fresh = InPlaceClaim::new(&replacement).expect("distinct versioned file loads");
    fresh.seal();
    drop(fresh);
    std::fs::remove_dir_all(&dir).expect("test fs op");
}

#[test]
fn in_place_claim_rolls_back_on_failed_library_load() {
    let dir = scratch("claim-rollback");
    std::fs::create_dir(&dir).expect("test fs op");
    let file = dir.join("libbroken.so");
    std::fs::write(&file, b"not-a-valid-dylib-v1").expect("test fs op");

    // Unsealed claim rolls back on drop so modified bytes can subsequently be claimed.
    let unsealed = InPlaceClaim::new(&file).expect("unsealed claim");
    drop(unsealed);
    std::fs::write(&file, b"not-a-valid-dylib-v2").expect("test fs op");
    let unsealed2 = InPlaceClaim::new(&file).expect("retry after unsealed rollback");
    drop(unsealed2);

    // Real `StagedDylib::load_with_root(..., None, ...)` failing inside `Library::new` must
    // roll back its `InPlaceClaim` cleanly so a subsequent load of new bytes at the same path
    // is NOT rejected with `AlreadyLoadedInPlace`.
    assert!(matches!(
        StagedDylib::load_with_root(&file, None, "mods", "g1"),
        Err(LoaderError::Load(_))
    ));
    std::fs::write(&file, b"still-not-a-dylib-but-different-bytes").expect("test fs op");
    let mut retry = InPlaceClaim::new(&file).expect("failed Library::new rolled back claim");
    retry.seal();
    drop(retry);
    std::fs::remove_dir_all(&dir).expect("test fs op");
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("nve-loader-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_file(&dir);
    dir
}

fn mode(path: &std::path::Path) -> u32 {
    std::fs::symlink_metadata(path).expect("test fs op").permissions().mode() & 0o777
}

#[test]
fn staging_dir_is_created_private() {
    let dir = scratch("fresh");
    drop(open_private_dir(&dir).expect("create"));
    assert_eq!(mode(&dir), 0o700);
    drop(open_private_dir(&dir).expect("reuse"));
    std::fs::remove_dir_all(&dir).expect("test fs op");
}

#[test]
fn an_own_shared_staging_dir_is_tightened_via_fchmod() {
    let dir = scratch("loose");
    std::fs::create_dir(&dir).expect("test fs op");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).expect("test fs op");
    drop(open_private_dir(&dir).expect("own dir"));
    assert_eq!(mode(&dir), 0o700, "group / other access removed via fchmod");
    std::fs::remove_dir_all(&dir).expect("test fs op");
}

#[test]
fn symlinked_or_file_staging_paths_are_rejected() {
    let target = scratch("target");
    std::fs::create_dir(&target).expect("test fs op");
    let link = scratch("link");
    std::os::unix::fs::symlink(&target, &link).expect("test fs op");
    assert!(open_private_dir(&link).is_err(), "a symlink is never followed");
    std::fs::remove_file(&link).expect("test fs op");
    std::fs::write(&link, b"x").expect("test fs op");
    assert!(open_private_dir(&link).is_err(), "a file is not a directory");
    std::fs::remove_file(&link).expect("test fs op");
    std::fs::remove_dir_all(&target).expect("test fs op");
}

#[test]
fn in_place_symlink_repoint_loads_new_target_via_canonical_path() {
    let dir = scratch("symlink");
    std::fs::create_dir(&dir).expect("test fs op");
    let build = |name: &str, tag: usize| {
        let (src, out) = (
            dir.join(format!("{name}.rs")),
            dir.join(format!("lib{name}.{}", std::env::consts::DLL_EXTENSION)),
        );
        let code = format!(
            "#[unsafe(no_mangle)] pub extern \"C\" fn nve_get_interface(_: u32) -> *const core::ffi::c_void {{ {tag} as *const _ }}"
        );
        std::fs::write(&src, code).expect("test fs op");
        let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
        let args = ["--crate-type=cdylib", "--edition=2024", "-C", "opt-level=0", "-o"];
        assert!(
            std::process::Command::new(rustc)
                .args(args)
                .arg(&out)
                .arg(&src)
                .status()
                .expect("rustc")
                .success()
        );
        out
    };
    let (v1, v2, link) = (build("sym_v1", 1), build("sym_v2", 2), dir.join("current.so"));
    std::os::unix::fs::symlink(&v1, &link).expect("test fs op");
    let d1 = StagedDylib::load_with_root(&link, None, "mods", "g1").expect("load v1 via symlink");
    assert_eq!(d1.loaded_copy_path(), std::fs::canonicalize(&v1).expect("canon v1"));
    // SAFETY: Test fixture returns its integer tag without dereferencing.
    assert_eq!(unsafe { (d1.get_interface())(nve_abi::InterfaceId(0)) as usize }, 1);
    std::fs::remove_file(&link).expect("test fs op");
    std::os::unix::fs::symlink(&v2, &link).expect("test fs op");
    let d2 = StagedDylib::load_with_root(&link, None, "mods", "g2")
        .expect("load v2 via repointed symlink");
    assert_eq!(d2.loaded_copy_path(), std::fs::canonicalize(&v2).expect("canon v2"));
    // SAFETY: Test fixture returns its integer tag without dereferencing.
    assert_eq!(unsafe { (d2.get_interface())(nve_abi::InterfaceId(0)) as usize }, 2);
    drop((d1, d2));
    std::fs::write(&v1, b"overwritten-v1").expect("test fs op");
    assert!(matches!(
        StagedDylib::load_with_root(&v1, None, "mods", "g3"),
        Err(LoaderError::AlreadyLoadedInPlace(_))
    ));
    std::fs::remove_dir_all(&dir).expect("test fs op");
}
