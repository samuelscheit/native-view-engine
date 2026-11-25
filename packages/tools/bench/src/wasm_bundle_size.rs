//! WASM bundle artifact size tracking for `hosts/web-shell/pkg/{studio,kitchen-sink,dev}`.
//!
//! Reads built `*.wasm`, `*.js`, `*.gz`, and `*.br` outputs (including `wasm-bindgen` inline JS
//! under `snippets/`) with zero external dependencies and zero panics when bundles are absent.
//! Budget checks report structured soft warnings (`BudgetStatus::OverBudget`) without failing
//! unit or benchmark builds.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

/// Default brotli (wasm + JS) budget in bytes for `hosts/web-shell/pkg/studio`.
pub const DEFAULT_BROTLI_BUDGET_STUDIO: u64 = 316_416;
/// Default brotli (wasm + JS) budget in bytes for `hosts/web-shell/pkg/kitchen-sink`.
pub const DEFAULT_BROTLI_BUDGET_KITCHEN_SINK: u64 = 315_392;
/// Default brotli (wasm + JS) budget in bytes for `hosts/web-shell/pkg/dev`.
pub const DEFAULT_BROTLI_BUDGET_DEV: u64 = 485_376;

/// Canonical bundle directories produced by `hosts/web-shell/build.sh`.
pub const DEFAULT_WEB_SHELL_BUNDLES: &[&str] = &["studio", "kitchen-sink", "dev"];

/// Raw, gzip, and brotli byte totals for one asset class (`wasm`, `js`, or combined).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AssetSizeBreakdown {
    pub raw_bytes: u64,
    pub gzip_bytes: u64,
    pub brotli_bytes: u64,
    pub file_count: usize,
}

impl AssetSizeBreakdown {
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.file_count == 0
            && self.raw_bytes == 0
            && self.gzip_bytes == 0
            && self.brotli_bytes == 0
    }

    #[inline]
    pub const fn combine(a: Self, b: Self) -> Self {
        Self {
            raw_bytes: a.raw_bytes.saturating_add(b.raw_bytes),
            gzip_bytes: a.gzip_bytes.saturating_add(b.gzip_bytes),
            brotli_bytes: a.brotli_bytes.saturating_add(b.brotli_bytes),
            file_count: a.file_count.saturating_add(b.file_count),
        }
    }
}

/// Budget comparison outcome for a single WASM bundle (soft-warning semantics).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetStatus {
    /// Bundle directory was missing or contained no `.wasm`/`.js` assets.
    NotBuilt,
    /// Bundle was built, but no brotli byte budget was configured.
    NoBudget,
    /// Built bundle has no `.br` artifacts yet (e.g. `--dev` debug build).
    UncompressedOnly,
    /// Total brotli bytes (`wasm + js`) are within budget (`<= budget`).
    WithinBudget { margin_bytes: u64 },
    /// Total brotli bytes (`wasm + js`) exceed budget (`> budget`).
    OverBudget { excess_bytes: u64 },
}

/// Measured artifact size breakdown and budget status for one web-shell bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmBundleMetrics {
    pub name: String,
    pub present: bool,
    pub wasm: AssetSizeBreakdown,
    pub js: AssetSizeBreakdown,
    pub total: AssetSizeBreakdown,
    pub brotli_budget_bytes: Option<u64>,
}

impl WasmBundleMetrics {
    /// Evaluates the bundle's brotli size against `brotli_budget_bytes` without panicking.
    pub fn budget_status(&self) -> BudgetStatus {
        if !self.present {
            return BudgetStatus::NotBuilt;
        }
        let Some(budget) = self.brotli_budget_bytes else {
            return BudgetStatus::NoBudget;
        };
        if self.total.brotli_bytes == 0 {
            return BudgetStatus::UncompressedOnly;
        }
        if self.total.brotli_bytes <= budget {
            BudgetStatus::WithinBudget {
                margin_bytes: budget - self.total.brotli_bytes,
            }
        } else {
            BudgetStatus::OverBudget {
                excess_bytes: self.total.brotli_bytes - budget,
            }
        }
    }

    #[inline]
    pub fn is_over_budget(&self) -> bool {
        matches!(self.budget_status(), BudgetStatus::OverBudget { .. })
    }

    /// Signed byte delta (`total.brotli_bytes - budget`) when built with `.br` files and a budget.
    pub fn brotli_delta_bytes(&self) -> Option<i64> {
        if !self.present || self.total.brotli_bytes == 0 {
            return None;
        }
        let budget = self.brotli_budget_bytes?;
        Some(self.total.brotli_bytes as i64 - budget as i64)
    }
}

/// Aggregated WASM bundle report across `hosts/web-shell/pkg/{studio,counter,dev}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmBundleReport {
    pub pkg_root: PathBuf,
    pub bundles: Vec<WasmBundleMetrics>,
}

impl WasmBundleReport {
    pub fn built_count(&self) -> usize {
        self.bundles.iter().filter(|b| b.present).count()
    }

    pub fn over_budget_bundles(&self) -> Vec<&WasmBundleMetrics> {
        self.bundles.iter().filter(|b| b.is_over_budget()).collect()
    }

    /// Returns `true` when at least one built bundle exceeds its soft brotli budget.
    pub fn has_soft_warnings(&self) -> bool {
        self.bundles.iter().any(WasmBundleMetrics::is_over_budget)
    }

    /// Formats a deterministic, aligned plain-text summary table for logs/bench output.
    pub fn format_table(&self) -> String {
        let mut out = String::with_capacity(256);
        let _ = writeln!(
            out,
            "{:<10} {:>10} {:>10} {:>10} {:>10}  STATUS",
            "BUNDLE", "RAW", "GZIP", "BROTLI", "BUDGET"
        );
        for b in &self.bundles {
            let budget_col = b
                .brotli_budget_bytes
                .map(format_bytes_kb)
                .unwrap_or_else(|| "-".to_owned());
            if !b.present {
                let _ = writeln!(
                    out,
                    "{:<10} {:>10} {:>10} {:>10} {:>10}  NOT BUILT",
                    b.name, "-", "-", "-", budget_col
                );
                continue;
            }
            let status = match b.budget_status() {
                BudgetStatus::NotBuilt => "NOT BUILT".to_owned(),
                BudgetStatus::NoBudget => "OK (no budget)".to_owned(),
                BudgetStatus::UncompressedOnly => "BUILT (no .br)".to_owned(),
                BudgetStatus::WithinBudget { margin_bytes } => {
                    format!("PASS (-{margin_bytes} B)")
                }
                BudgetStatus::OverBudget { excess_bytes } => {
                    format!("WARN (+{excess_bytes} B over budget)")
                }
            };
            let _ = writeln!(
                out,
                "{:<10} {:>10} {:>10} {:>10} {:>10}  {}",
                b.name,
                format_bytes_kb(b.total.raw_bytes),
                format_bytes_kb(b.total.gzip_bytes),
                format_bytes_kb(b.total.brotli_bytes),
                budget_col,
                status
            );
        }
        out
    }
}

fn format_bytes_kb(bytes: u64) -> String {
    if bytes == 0 {
        return "-".to_owned();
    }
    format!("{:.1} KB", bytes as f64 / 1024.0)
}

/// Returns the default (or env-overridden) brotli budget in bytes for a known web-shell bundle.
pub fn default_brotli_budget(bundle: &str) -> Option<u64> {
    let (env_key, fallback) = match bundle {
        "studio" => ("WASM_BROTLI_BUDGET_STUDIO", DEFAULT_BROTLI_BUDGET_STUDIO),
        "kitchen-sink" => ("WASM_BROTLI_BUDGET_KITCHEN_SINK", DEFAULT_BROTLI_BUDGET_KITCHEN_SINK),
        "dev" => ("WASM_BROTLI_BUDGET_DEV", DEFAULT_BROTLI_BUDGET_DEV),
        _ => return None,
    };
    let parsed = std::env::var(env_key)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok());
    Some(parsed.unwrap_or(fallback))
}

/// Recursively inspects `bundle_dir` for `*.wasm`, `*.js`, `*.gz`, and `*.br` artifacts.
///
/// Missing or unreadable directories return `present: false` with zeroed metrics and never panic.
pub fn measure_bundle_dir(
    name: impl Into<String>,
    bundle_dir: &Path,
    brotli_budget_bytes: Option<u64>,
) -> WasmBundleMetrics {
    let name = name.into();
    let mut wasm = AssetSizeBreakdown::default();
    let mut js = AssetSizeBreakdown::default();

    if bundle_dir.is_dir() {
        walk_bundle_dir(bundle_dir, &mut wasm, &mut js);
    }

    let total = AssetSizeBreakdown::combine(wasm, js);
    let present = wasm.file_count > 0 || js.file_count > 0;
    WasmBundleMetrics {
        name,
        present,
        wasm,
        js,
        total,
        brotli_budget_bytes,
    }
}

fn walk_bundle_dir(dir: &Path, wasm: &mut AssetSizeBreakdown, js: &mut AssetSizeBreakdown) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_dir() {
            walk_bundle_dir(&path, wasm, js);
            continue;
        }
        if !meta.is_file() {
            continue;
        }
        let Some(fname) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let len = meta.len();
        if fname.ends_with(".wasm") {
            wasm.raw_bytes = wasm.raw_bytes.saturating_add(len);
            wasm.file_count += 1;
        } else if fname.ends_with(".wasm.gz") {
            wasm.gzip_bytes = wasm.gzip_bytes.saturating_add(len);
        } else if fname.ends_with(".wasm.br") {
            wasm.brotli_bytes = wasm.brotli_bytes.saturating_add(len);
        } else if fname.ends_with(".js") {
            js.raw_bytes = js.raw_bytes.saturating_add(len);
            js.file_count += 1;
        } else if fname.ends_with(".js.gz") {
            js.gzip_bytes = js.gzip_bytes.saturating_add(len);
        } else if fname.ends_with(".js.br") {
            js.brotli_bytes = js.brotli_bytes.saturating_add(len);
        }
    }
}

/// Measures the standard `{studio, counter, dev}` bundles under `pkg_root` (`hosts/web-shell/pkg`).
pub fn measure_web_shell_bundles(pkg_root: &Path) -> WasmBundleReport {
    let bundles = DEFAULT_WEB_SHELL_BUNDLES
        .iter()
        .map(|&name| measure_bundle_dir(name, &pkg_root.join(name), default_brotli_budget(name)))
        .collect();
    WasmBundleReport {
        pkg_root: pkg_root.to_path_buf(),
        bundles,
    }
}

/// Convenience helper measuring `<workspace_root>/hosts/web-shell/pkg/{studio,counter,dev}`.
pub fn measure_workspace_web_shell_bundles(workspace_root: &Path) -> WasmBundleReport {
    measure_web_shell_bundles(&workspace_root.join("hosts").join("web-shell").join("pkg"))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT_TMP_ID: AtomicU64 = AtomicU64::new(1);

    fn temp_pkg_dir(label: &str) -> PathBuf {
        let id = NEXT_TMP_ID.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "nve_bench_wasm_size_{label}_{}_{id}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp test dir");
        dir
    }

    fn write_bytes(path: &Path, len: usize) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent dirs");
        }
        fs::write(path, vec![0xABu8; len]).expect("write test artifact");
    }

    #[test]
    fn missing_pkg_root_returns_empty_report_without_panicking() {
        let missing = std::env::temp_dir().join(format!(
            "nve_bench_nonexistent_pkg_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&missing);

        let report = measure_web_shell_bundles(&missing);
        assert_eq!(report.bundles.len(), 3);
        assert_eq!(report.built_count(), 0);
        assert!(!report.has_soft_warnings());
        assert!(report.over_budget_bundles().is_empty());
        for b in &report.bundles {
            assert!(!b.present);
            assert!(b.total.is_empty());
            assert_eq!(b.budget_status(), BudgetStatus::NotBuilt);
            assert_eq!(b.brotli_delta_bytes(), None);
        }
        let table = report.format_table();
        assert!(table.contains("studio"));
        assert!(table.contains("kitchen-sink"));
        assert!(table.contains("dev"));
        assert!(table.contains("NOT BUILT"));
    }

    #[test]
    fn aggregates_nested_snippet_js_and_wasm_raw_gzip_brotli_and_ignores_other_files() {
        let root = temp_pkg_dir("aggregate");
        let ks_dir = root.join("kitchen-sink");

        write_bytes(&ks_dir.join("nve_web_shell_bg.wasm"), 200_000);
        write_bytes(&ks_dir.join("nve_web_shell_bg.wasm.gz"), 95_000);
        write_bytes(&ks_dir.join("nve_web_shell_bg.wasm.br"), 80_000);
        write_bytes(&ks_dir.join("nve_web_shell.js"), 12_000);
        write_bytes(&ks_dir.join("nve_web_shell.js.gz"), 4_200);
        write_bytes(&ks_dir.join("nve_web_shell.js.br"), 3_500);
        // Nested wasm-bindgen inline_js snippet.
        write_bytes(
            &ks_dir.join("snippets/nve-backend-web-abc/inline0.js"),
            1_024,
        );
        write_bytes(
            &ks_dir.join("snippets/nve-backend-web-abc/inline0.js.gz"),
            400,
        );
        write_bytes(
            &ks_dir.join("snippets/nve-backend-web-abc/inline0.js.br"),
            320,
        );
        // Non-bundle artifacts must be ignored.
        write_bytes(&ks_dir.join("package.json"), 512);
        write_bytes(&ks_dir.join("nve_web_shell.d.ts"), 2_048);

        let m = measure_bundle_dir(
            "kitchen-sink",
            &ks_dir,
            Some(DEFAULT_BROTLI_BUDGET_KITCHEN_SINK),
        );
        assert!(m.present);
        assert_eq!(m.wasm.file_count, 1);
        assert_eq!(m.wasm.raw_bytes, 200_000);
        assert_eq!(m.wasm.gzip_bytes, 95_000);
        assert_eq!(m.wasm.brotli_bytes, 80_000);
        assert_eq!(m.js.file_count, 2);
        assert_eq!(m.js.raw_bytes, 13_024);
        assert_eq!(m.js.gzip_bytes, 4_600);
        assert_eq!(m.js.brotli_bytes, 3_820);
        assert_eq!(m.total.file_count, 3);
        assert_eq!(m.total.raw_bytes, 213_024);
        assert_eq!(m.total.gzip_bytes, 99_600);
        assert_eq!(m.total.brotli_bytes, 83_820);
        assert_eq!(
            m.budget_status(),
            BudgetStatus::WithinBudget {
                margin_bytes: DEFAULT_BROTLI_BUDGET_KITCHEN_SINK - 83_820
            }
        );
        assert!(!m.is_over_budget());
        assert_eq!(
            m.brotli_delta_bytes(),
            Some(83_820i64 - DEFAULT_BROTLI_BUDGET_KITCHEN_SINK as i64)
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn budget_boundaries_and_soft_warning_report_behavior() {
        let root = temp_pkg_dir("budgets");

        // `kitchen-sink`: exact budget boundary -> PASS with 0 margin.
        let ks = root.join("kitchen-sink");
        write_bytes(&ks.join("app.wasm"), 350_000);
        write_bytes(&ks.join("app.wasm.br"), 300_000);
        write_bytes(&ks.join("app.js"), 25_000);
        write_bytes(
            &ks.join("app.js.br"),
            (DEFAULT_BROTLI_BUDGET_KITCHEN_SINK - 300_000) as usize,
        );

        // `studio`: +512 B over budget -> soft warning (`OverBudget`), no panic.
        let studio = root.join("studio");
        write_bytes(&studio.join("app.wasm"), 400_000);
        write_bytes(
            &studio.join("app.wasm.br"),
            (DEFAULT_BROTLI_BUDGET_STUDIO + 512) as usize,
        );

        // `dev`: debug build without .br/.gz -> `UncompressedOnly`.
        let dev = root.join("dev");
        write_bytes(&dev.join("app.wasm"), 500_000);

        let report = measure_web_shell_bundles(&root);
        assert_eq!(report.built_count(), 3);
        assert!(report.has_soft_warnings());
        let over = report.over_budget_bundles();
        assert_eq!(over.len(), 1);
        assert_eq!(over[0].name, "studio");
        assert_eq!(
            over[0].budget_status(),
            BudgetStatus::OverBudget { excess_bytes: 512 }
        );
        assert_eq!(over[0].brotli_delta_bytes(), Some(512));

        let ks_m = report.bundles.iter().find(|b| b.name == "kitchen-sink").unwrap();
        assert_eq!(
            ks_m.budget_status(),
            BudgetStatus::WithinBudget { margin_bytes: 0 }
        );
        assert_eq!(ks_m.brotli_delta_bytes(), Some(0));

        let dev_m = report.bundles.iter().find(|b| b.name == "dev").unwrap();
        assert_eq!(dev_m.budget_status(), BudgetStatus::UncompressedOnly);
        assert_eq!(dev_m.brotli_delta_bytes(), None);

        // Custom bundle with no budget configured reports `NoBudget`.
        let custom = measure_bundle_dir("custom", &ks, None);
        assert_eq!(custom.budget_status(), BudgetStatus::NoBudget);

        let table = report.format_table();
        assert!(table.contains("PASS (-0 B)"));
        assert!(table.contains("WARN (+512 B over budget)"));
        assert!(table.contains("BUILT (no .br)"));

        let _ = fs::remove_dir_all(&root);
    }
}
