//! Embedded compact PNG avatar data URIs and `NVE_DRAW1` telemetry sparkline builder
//! for **Aether Workspace** + **NVE Studio**.

use nve_abi::ColorRgba;
use nve_canvas::DrawList;
use nve_tables::parse_color;

/// Valid 8x8 RGBA PNG data-URI avatar for `@maya` (cyan ring/core).
pub const AVATAR_MAYA: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAJElEQVR42mOQ07T+jw8zgAgerzysGEWBxd4fKHggFOB1JD4MAKjRfBnTmv6zAAAAAElFTkSuQmCC";

/// Valid 8x8 RGBA PNG data-URI avatar for `@liam` (violet ring/core).
pub const AVATAR_LIAM: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAJElEQVR42mOQ07T+jw8zgAg9gVSsGEXB8u5fKHggFOB1JD4MAGcqfXUz+ORAAAAAAElFTkSuQmCC";

/// Valid 8x8 RGBA PNG data-URI avatar for `@sara` (green ring/core).
pub const AVATAR_SARA: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAJElEQVR42mOQ07T+jw8zgAhWPTGsGEWB0tE4FDwQCvA6Eh8GAE5RZ/09pCRNAAAAAElFTkSuQmCC";

/// Valid 8x8 RGBA PNG data-URI avatar for `@dev` (slate ring/core).
pub const AVATAR_DEV: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAHElEQVQY02OQ07T+jw8zEK1gyuIdKHggFJDtCwAdM3cR3wnOlQAAAABJRU5ErkJggg==";

/// Valid 8x8 RGBA PNG data-URI avatar for `@aria` (amber ring/core).
pub const AVATAR_ARIA: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAJElEQVR42mOQ07T+jw8zgAhXKWasGEXB13ncKHggFOB1JD4MAEs7b4ERxIxeAAAAAElFTkSuQmCC";

/// Valid 8x8 RGBA PNG data-URI avatar for `@noah` (sky ring/core).
pub const AVATAR_NOAH: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAJElEQVR42mOQ07T+jw8zgAgerzysGEVB7eU/KHggFOB1JD4MAL/Ggglv5kNyAAAAAElFTkSuQmCC";

/// Valid 8x8 RGBA PNG data-URI avatar for `@you` (bright slate/white core).
pub const AVATAR_YOU: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAJElEQVR42mOQ07T+jw8zgAh+cS2sGEXBj19/UPBAKMDrSHwYAFnZgxk5UYNxAAAAAElFTkSuQmCC";

/// Resolves an author/assignee handle (with or without leading `@`) to a valid PNG data-URI avatar.
pub fn avatar_for_author(author: &str) -> &'static str {
    let key = author.trim().trim_start_matches('@');
    if key.eq_ignore_ascii_case("maya") {
        AVATAR_MAYA
    } else if key.eq_ignore_ascii_case("liam") {
        AVATAR_LIAM
    } else if key.eq_ignore_ascii_case("sara") {
        AVATAR_SARA
    } else if key.eq_ignore_ascii_case("dev") {
        AVATAR_DEV
    } else if key.eq_ignore_ascii_case("aria") {
        AVATAR_ARIA
    } else if key.eq_ignore_ascii_case("noah") {
        AVATAR_NOAH
    } else {
        AVATAR_YOU
    }
}

/// Builds a live zero-serialization [`DrawList`] (`Arc<[CanvasOp]>`) plotting recorded
/// frame/mutation timings and snapshot history depth for the `Metrics` view `SkiaCanvas`.
pub fn build_metrics_sparkline(
    samples_us: &[u32],
    history_len: usize,
    accent_hex: &str,
) -> DrawList {
    let stroke = parse_color(accent_hex).unwrap_or(ColorRgba::rgb(56, 189, 248));
    let bg = ColorRgba::rgb(17, 24, 39);

    let mut plot_points: Vec<f32> = samples_us.iter().map(|&v| v as f32).collect();
    // Append a live point encoding current snapshot depth so time-travel / edits visibly move the curve.
    let history_pulse = 140.0 + ((history_len as f32) * 19.0).min(240.0);
    plot_points.push(history_pulse);

    DrawList::sparkline(&plot_points, 320.0, 72.0, stroke, bg)
}
