//! Isolated OS-process RSS regression test proving macOS color emoji fonts use zero-copy OS
//! `mmap` (`Data::new_with_owner` / `Data::from_filename`) rather than copying ~180 MiB into heap.
//!
//! Kept in its own integration test binary so parallel tests in the same process never perturb
//! process-wide RSS readings.

#[cfg(all(feature = "skia", target_os = "macos"))]
#[test]
fn macos_emoji_mmap_typeface_keeps_rss_delta_under_10mb() {
    use nve_backend_skia::text::TextAlign;
    use nve_backend_skia::SkiaCanvasRenderer;

    let read_rss_bytes = || -> Option<u64> {
        let out = std::process::Command::new("ps")
            .args(["-o", "rss=", "-p", &std::process::id().to_string()])
            .output()
            .ok()?;
        let kb: u64 = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
        Some(kb * 1024)
    };

    // Warm Skia `FontMgr::new()` + `CosmicTextEngine::shared()` BEFORE measuring the emoji delta.
    let renderer = SkiaCanvasRenderer::new();
    let _ = renderer.build_cached_text_blob("Warm fontdb", 16.0, 400, TextAlign::Left, None, 120.0);

    let rss_before = read_rss_bytes().expect("macOS ps rss before");
    for weight in [400u16, 600u16, 700u16] {
        let blob =
            renderer.build_cached_text_blob("🚀🎉🔥", 24.0, weight, TextAlign::Left, None, 200.0);
        assert!(blob.is_some());
    }
    let rss_after = read_rss_bytes().expect("macOS ps rss after");
    let stats = renderer.cache_stats();
    assert!(
        stats.typeface_mmap_entries >= 2,
        "system + emoji fonts must be mmap-backed"
    );
    assert_eq!(stats.typeface_copied_entries, 0);
    let rss_delta = rss_after.saturating_sub(rss_before);
    eprintln!(
        "[rss] emoji mmap delta = {} KiB (typeface_bytes = {} KiB)",
        rss_delta / 1024,
        stats.typeface_bytes / 1024
    );
    assert!(
        rss_delta < 10 * 1024 * 1024,
        "mmap emoji load must keep RSS delta < 10 MiB, observed {} KiB (total typeface_bytes = {} KiB)",
        rss_delta / 1024,
        stats.typeface_bytes / 1024
    );
}
