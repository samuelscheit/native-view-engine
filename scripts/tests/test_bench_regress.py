"""Unit tests for scripts/bench-regress.py."""

from __future__ import annotations

import importlib.util
import json
import os
import sys
import tempfile
import time
import unittest
from pathlib import Path
from typing import Any, Dict

SCRIPT_PATH = Path(__file__).resolve().parent.parent / "bench-regress.py"


def _load_module():
    spec = importlib.util.spec_from_file_location("bench_regress", SCRIPT_PATH)
    assert spec is not None and spec.loader is not None
    mod = importlib.util.module_from_spec(spec)
    sys.modules["bench_regress"] = mod
    spec.loader.exec_module(mod)
    return mod


br = _load_module()


def _write_criterion_bench(
    criterion_dir: Path,
    rel_dir: str,
    median_ns: float,
    *,
    mean_ns: float | None = None,
    full_id: str | None = None,
) -> Path:
    new_dir = criterion_dir / rel_dir / "new"
    new_dir.mkdir(parents=True, exist_ok=True)
    estimates: Dict[str, Any] = {
        "median": {
            "point_estimate": median_ns,
            "confidence_interval": {
                "confidence_level": 0.95,
                "lower_bound": median_ns * 0.98,
                "upper_bound": median_ns * 1.02,
            },
        }
    }
    if mean_ns is not None:
        estimates["mean"] = {"point_estimate": mean_ns}

    estimates_path = new_dir / "estimates.json"
    estimates_path.write_text(json.dumps(estimates), encoding="utf-8")

    if full_id is not None:
        (new_dir / "benchmark.json").write_text(
            json.dumps({"full_id": full_id, "directory_name": rel_dir}),
            encoding="utf-8",
        )
    return estimates_path


class CriterionParsingTests(unittest.TestCase):
    def test_parses_full_id_and_fallback_path(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            cdir = Path(tmp) / "criterion"
            _write_criterion_bench(
                cdir,
                "abi_command_buffer_encode/100",
                4380.1234,
                mean_ns=4402.5,
                full_id="abi/command_buffer_encode/100",
            )
            _write_criterion_bench(
                cdir,
                "dioxus/initial_frame_render",
                12500.0,
            )

            parsed = br.parse_criterion_dir(cdir)
            self.assertEqual(
                list(parsed.keys()),
                [
                    "rust::abi/command_buffer_encode/100",
                    "rust::dioxus/initial_frame_render",
                ],
            )
            e1 = parsed["rust::abi/command_buffer_encode/100"]
            self.assertEqual(e1["suite"], "rust")
            self.assertEqual(e1["name"], "abi/command_buffer_encode/100")
            self.assertAlmostEqual(e1["median_ns"], 4380.123)
            self.assertAlmostEqual(e1["mean_ns"], 4402.5)

            e2 = parsed["rust::dioxus/initial_frame_render"]
            self.assertEqual(e2["name"], "dioxus/initial_frame_render")
            self.assertAlmostEqual(e2["median_ns"], 12500.0)

    def test_filters_stale_files_with_min_mtime(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            cdir = Path(tmp) / "criterion"
            stale = _write_criterion_bench(cdir, "old/bench", 100.0)
            fresh = _write_criterion_bench(cdir, "fresh/bench", 200.0)

            now = time.time()
            os.utime(stale, (now - 100.0, now - 100.0))
            os.utime(fresh, (now, now))

            parsed = br.parse_criterion_dir(cdir, min_mtime=now - 10.0)
            self.assertEqual(list(parsed.keys()), ["rust::fresh/bench"])

    def test_rejects_missing_median(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            cdir = Path(tmp) / "criterion"
            p = cdir / "broken" / "new"
            p.mkdir(parents=True)
            (p / "estimates.json").write_text(json.dumps({"mean": {"point_estimate": 10.0}}), encoding="utf-8")
            with self.assertRaises(ValueError):
                br.parse_criterion_dir(cdir)


class CxxJsonParsingTests(unittest.TestCase):
    def test_parses_host_bench_json_with_p50_and_median(self) -> None:
        raw = json.dumps(
            {
                "suite": "nve_host_cxx_bench",
                "benchmarks": [
                    {
                        "name": "scene_read_all_10000",
                        "p50_ns": 13200.0,
                        "p95_ns": 13900.5,
                        "mean_ns": 13350.25,
                        "ops_per_sec": 75757.58,
                        "iters": 200,
                    },
                    {
                        "name": "custom_median_entry",
                        "median_ns": 420.5,
                        "p95_ns": 450.0,
                        "throughput": 2378121.0,
                    },
                ],
            }
        )
        parsed = br.parse_cxx_json(raw)
        self.assertEqual(
            list(parsed.keys()),
            ["cxx::custom_median_entry", "cxx::scene_read_all_10000"],
        )
        read_all = parsed["cxx::scene_read_all_10000"]
        self.assertEqual(read_all["suite"], "cxx")
        self.assertAlmostEqual(read_all["median_ns"], 13200.0)
        self.assertAlmostEqual(read_all["p95_ns"], 13900.5)
        self.assertAlmostEqual(read_all["ops_per_sec"], 75757.58)

        custom = parsed["cxx::custom_median_entry"]
        self.assertAlmostEqual(custom["median_ns"], 420.5)
        self.assertAlmostEqual(custom["ops_per_sec"], 2378121.0)

    def test_rejects_malformed_cxx_json(self) -> None:
        with self.assertRaises(ValueError):
            br.parse_cxx_json("{not valid json")
        with self.assertRaises(ValueError):
            br.parse_cxx_json(json.dumps({"benchmarks": [{"name": "no_timing"}]}))
        with self.assertRaises(ValueError):
            br.parse_cxx_json(json.dumps({"benchmarks": [{"p50_ns": 100.0}]}))


class ComparisonAndReportingTests(unittest.TestCase):
    def _make_snapshot(self, benches: Dict[str, Dict[str, Any]], *, os_name: str = "Darwin", arch: str = "arm64") -> Dict[str, Any]:
        return {
            "schema_version": 1,
            "generated_at": "2026-11-25T03:00:00Z",
            "machine": {"os": os_name, "arch": arch, "cpu_count": 8},
            "default_tolerance_pct": 15.0,
            "benchmarks": benches,
        }

    def test_regression_improvement_ok_missing_new_and_tolerance_overrides(self) -> None:
        baseline = self._make_snapshot(
            {
                # Uses global threshold (15%): +20% -> regression
                "rust::abi/encode": {
                    "suite": "rust",
                    "name": "abi/encode",
                    "median_ns": 1000.0,
                },
                # Per-metric tighter tolerance (5%): +8% -> regression even though < global 15%
                "rust::abi/scene_dirty": {
                    "suite": "rust",
                    "name": "abi/scene_dirty",
                    "median_ns": 500.0,
                    "tolerance_pct": 5.0,
                },
                # Per-metric looser tolerance (30%): +22% -> OK even though > global 15%
                "rust::abi/micro_lookup": {
                    "suite": "rust",
                    "name": "abi/micro_lookup",
                    "median_ns": 10.0,
                    "tolerance_pct": 30.0,
                },
                # Improved (-20%)
                "cxx::scene_read_all_1000": {
                    "suite": "cxx",
                    "name": "scene_read_all_1000",
                    "median_ns": 2000.0,
                    "tolerance_pct": 15.0,
                },
                # Within noise (+3%) -> OK
                "cxx::scene_reader_open_1000": {
                    "suite": "cxx",
                    "name": "scene_reader_open_1000",
                    "median_ns": 700.0,
                    "tolerance_pct": 15.0,
                },
                # Missing in current
                "cxx::removed_bench": {
                    "suite": "cxx",
                    "name": "removed_bench",
                    "median_ns": 1200.0,
                    "tolerance_pct": 15.0,
                },
            }
        )

        current = self._make_snapshot(
            {
                "rust::abi/encode": {
                    "suite": "rust",
                    "name": "abi/encode",
                    "median_ns": 1200.0,
                },
                "rust::abi/scene_dirty": {
                    "suite": "rust",
                    "name": "abi/scene_dirty",
                    "median_ns": 540.0,
                },
                "rust::abi/micro_lookup": {
                    "suite": "rust",
                    "name": "abi/micro_lookup",
                    "median_ns": 12.2,
                },
                "cxx::scene_read_all_1000": {
                    "suite": "cxx",
                    "name": "scene_read_all_1000",
                    "median_ns": 1600.0,
                },
                "cxx::scene_reader_open_1000": {
                    "suite": "cxx",
                    "name": "scene_reader_open_1000",
                    "median_ns": 721.0,
                },
                "rust::abi/brand_new": {
                    "suite": "rust",
                    "name": "abi/brand_new",
                    "median_ns": 333.0,
                },
            }
        )

        report = br.compare_snapshots(baseline, current, threshold_pct=15.0)
        self.assertFalse(report["passed"])
        self.assertFalse(report["machine_mismatch"])

        by_key = {row["key"]: row for row in report["rows"]}
        self.assertEqual(by_key["rust::abi/encode"]["status"], "regression")
        self.assertAlmostEqual(by_key["rust::abi/encode"]["delta_pct"], 20.0)
        self.assertAlmostEqual(by_key["rust::abi/encode"]["tolerance_pct"], 15.0)

        self.assertEqual(by_key["rust::abi/scene_dirty"]["status"], "regression")
        self.assertAlmostEqual(by_key["rust::abi/scene_dirty"]["delta_pct"], 8.0)
        self.assertAlmostEqual(by_key["rust::abi/scene_dirty"]["tolerance_pct"], 5.0)

        self.assertEqual(by_key["rust::abi/micro_lookup"]["status"], "ok")
        self.assertAlmostEqual(by_key["rust::abi/micro_lookup"]["delta_pct"], 22.0)
        self.assertAlmostEqual(by_key["rust::abi/micro_lookup"]["tolerance_pct"], 30.0)

        self.assertEqual(by_key["cxx::scene_read_all_1000"]["status"], "improved")
        self.assertAlmostEqual(by_key["cxx::scene_read_all_1000"]["delta_pct"], -20.0)

        self.assertEqual(by_key["cxx::scene_reader_open_1000"]["status"], "ok")
        self.assertEqual(by_key["cxx::removed_bench"]["status"], "missing")
        self.assertEqual(by_key["rust::abi/brand_new"]["status"], "new")

        self.assertEqual(report["summary"]["regressions"], 2)
        self.assertEqual(report["summary"]["improved"], 1)
        self.assertEqual(report["summary"]["ok"], 2)
        self.assertEqual(report["summary"]["missing"], 1)
        self.assertEqual(report["summary"]["new"], 1)

        md = br.render_markdown(report)
        self.assertIn("## Benchmark Regression Report — FAIL", md)
        self.assertIn("### Regressions", md)
        self.assertIn("`rust::abi/encode`", md)
        self.assertIn("`rust::abi/scene_dirty`", md)
        self.assertIn("| Suite | Benchmark | Baseline | Current | Delta | Tolerance | Status |", md)
        self.assertIn("**REGRESSION**", md)
        self.assertIn("**IMPROVED**", md)
        self.assertIn("**OK**", md)
        self.assertIn("**NEW**", md)
        self.assertIn("**MISSING**", md)

    def test_pass_and_machine_mismatch_and_fail_on_missing(self) -> None:
        baseline = self._make_snapshot(
            {
                "rust::a": {"suite": "rust", "name": "a", "median_ns": 1000.0, "tolerance_pct": 15.0},
                "rust::b": {"suite": "rust", "name": "b", "median_ns": 1000.0, "tolerance_pct": 15.0},
            },
            os_name="Darwin",
            arch="arm64",
        )
        current = self._make_snapshot(
            {
                "rust::a": {"suite": "rust", "name": "a", "median_ns": 900.0},
            },
            os_name="Linux",
            arch="x86_64",
        )

        report_default = br.compare_snapshots(baseline, current, threshold_pct=15.0)
        self.assertTrue(report_default["passed"])
        self.assertTrue(report_default["machine_mismatch"])
        md = br.render_markdown(report_default)
        self.assertIn("## Benchmark Regression Report — PASS", md)
        self.assertIn("different OS/architecture environments", md)

        report_strict = br.compare_snapshots(
            baseline, current, threshold_pct=15.0, fail_on_missing=True
        )
        self.assertFalse(report_strict["passed"])

    def test_cli_capture_and_compare_end_to_end(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            cdir = tmp_path / "criterion"
            _write_criterion_bench(
                cdir,
                "abi_value/value_direct_ffi_read",
                12.5,
                full_id="abi/value/value_direct_ffi_read",
            )
            _write_criterion_bench(
                cdir,
                "abi_nve2_wire_decode/1000",
                4200.0,
                full_id="abi/nve2_wire_decode/1000",
            )
            cxx_json_path = tmp_path / "cxx.json"
            cxx_json_path.write_text(
                json.dumps(
                    {
                        "suite": "nve_host_cxx_bench",
                        "benchmarks": [
                            {"name": "scene_read_all_1000", "p50_ns": 1900.0, "p95_ns": 1950.0, "ops_per_sec": 526315.8}
                        ],
                    }
                ),
                encoding="utf-8",
            )

            baseline_path = tmp_path / "baseline.json"
            rc = br.main(
                [
                    "capture",
                    "--baseline",
                    str(baseline_path),
                    "--criterion-dir",
                    str(cdir),
                    "--cxx-json",
                    str(cxx_json_path),
                ]
            )
            self.assertEqual(rc, 0)
            baseline_data = json.loads(baseline_path.read_text(encoding="utf-8"))
            # Deterministic sorted keys
            self.assertEqual(
                list(baseline_data["benchmarks"].keys()),
                [
                    "cxx::scene_read_all_1000",
                    "rust::abi/nve2_wire_decode/1000",
                    "rust::abi/value/value_direct_ffi_read",
                ],
            )
            # Sub-50ns microbenchmark gets 25% tolerance; larger bench gets 15%
            self.assertEqual(
                baseline_data["benchmarks"]["rust::abi/value/value_direct_ffi_read"]["tolerance_pct"],
                25.0,
            )
            self.assertEqual(
                baseline_data["benchmarks"]["rust::abi/nve2_wire_decode/1000"]["tolerance_pct"],
                15.0,
            )

            # Passing compare against itself
            md_path = tmp_path / "summary.md"
            report_path = tmp_path / "report.json"
            rc_pass = br.main(
                [
                    "compare",
                    "--baseline",
                    str(baseline_path),
                    "--current",
                    str(baseline_path),
                    "--markdown",
                    str(md_path),
                    "--report-json",
                    str(report_path),
                ]
            )
            self.assertEqual(rc_pass, 0)
            self.assertIn("PASS", md_path.read_text(encoding="utf-8"))
            self.assertTrue(json.loads(report_path.read_text(encoding="utf-8"))["passed"])

            # Failing compare when one benchmark regresses by 50%
            regressed_path = tmp_path / "regressed.json"
            regressed = json.loads(baseline_path.read_text(encoding="utf-8"))
            regressed["benchmarks"]["cxx::scene_read_all_1000"]["median_ns"] = 2850.0
            regressed_path.write_text(json.dumps(regressed), encoding="utf-8")

            rc_fail = br.main(
                [
                    "compare",
                    "--baseline",
                    str(baseline_path),
                    "--current",
                    str(regressed_path),
                    "--markdown",
                    str(md_path),
                ]
            )
            self.assertEqual(rc_fail, 1)
            self.assertIn("FAIL", md_path.read_text(encoding="utf-8"))
            self.assertIn("cxx::scene_read_all_1000", md_path.read_text(encoding="utf-8"))

    def test_zero_baseline_and_exact_threshold_boundary(self) -> None:
        baseline = self._make_snapshot(
            {
                "rust::zero_ok": {"suite": "rust", "name": "zero_ok", "median_ns": 0.0, "tolerance_pct": 15.0},
                "rust::zero_reg": {"suite": "rust", "name": "zero_reg", "median_ns": 0.0, "tolerance_pct": 15.0},
                "rust::exact_boundary": {"suite": "rust", "name": "exact_boundary", "median_ns": 100.0, "tolerance_pct": 15.0},
            }
        )
        current = self._make_snapshot(
            {
                "rust::zero_ok": {"suite": "rust", "name": "zero_ok", "median_ns": 0.0},
                "rust::zero_reg": {"suite": "rust", "name": "zero_reg", "median_ns": 5.0},
                "rust::exact_boundary": {"suite": "rust", "name": "exact_boundary", "median_ns": 115.0},
            }
        )
        report = br.compare_snapshots(baseline, current, threshold_pct=15.0)
        by_key = {row["key"]: row for row in report["rows"]}
        self.assertEqual(by_key["rust::zero_ok"]["status"], "ok")
        self.assertEqual(by_key["rust::zero_reg"]["status"], "regression")
        self.assertIsNone(by_key["rust::zero_reg"]["delta_pct"])
        self.assertEqual(by_key["rust::exact_boundary"]["status"], "ok")
        # Ensure strict JSON serialization (no NaN / Infinity)
        serialized = json.dumps(report, allow_nan=False)
        self.assertIn("zero_reg", serialized)
        self.assertIn("+∞%", br.render_markdown(report))

    def test_preserve_tolerances_and_raw_cxx_current(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            cxx_json_path = tmp_path / "cxx.json"
            cxx_json_path.write_text(
                json.dumps(
                    {
                        "suite": "nve_host_cxx_bench",
                        "benchmarks": [
                            {"name": "scene_read_all_1000", "p50_ns": 700.0, "p95_ns": 800.0, "ops_per_sec": 1428571.0}
                        ],
                    }
                ),
                encoding="utf-8",
            )
            baseline_path = tmp_path / "baseline.json"
            baseline_path.write_text(
                json.dumps(
                    self._make_snapshot(
                        {
                            "cxx::scene_read_all_1000": {
                                "suite": "cxx",
                                "name": "scene_read_all_1000",
                                "median_ns": 690.0,
                                "tolerance_pct": 7.5,
                            }
                        }
                    )
                ),
                encoding="utf-8",
            )
            rc = br.main(
                [
                    "capture",
                    "--baseline",
                    str(baseline_path),
                    "--cxx-json",
                    str(cxx_json_path),
                    "--preserve-tolerances",
                ]
            )
            self.assertEqual(rc, 0)
            updated = json.loads(baseline_path.read_text(encoding="utf-8"))
            self.assertEqual(
                updated["benchmarks"]["cxx::scene_read_all_1000"]["tolerance_pct"],
                7.5,
            )

            # Directly pass raw C++ --json output file as --current
            rc_cmp = br.main(
                [
                    "compare",
                    "--baseline",
                    str(baseline_path),
                    "--current",
                    str(cxx_json_path),
                ]
            )
            self.assertEqual(rc_cmp, 0)

    def test_fails_when_suite_has_zero_baseline_overlap(self) -> None:
        baseline = self._make_snapshot(
            {
                "rust::a": {"suite": "rust", "name": "a", "median_ns": 1000.0, "tolerance_pct": 15.0},
            }
        )
        current = self._make_snapshot(
            {
                "rust::a": {"suite": "rust", "name": "a", "median_ns": 1000.0},
                "cxx::scene_reader_open_1000": {
                    "suite": "cxx",
                    "name": "scene_reader_open_1000",
                    "median_ns": 12.0,
                },
            }
        )
        report = br.compare_snapshots(baseline, current, threshold_pct=15.0)
        self.assertFalse(report["passed"])
        self.assertEqual(report["unbaselined_suites"], ["cxx"])
        self.assertEqual(report["summary"]["unbaselined_suites"], 1)
        md = br.render_markdown(report)
        self.assertIn("## Benchmark Regression Report — FAIL", md)
        self.assertIn("### Unbaselined Suites", md)
        self.assertIn("`cxx`", md)
        console = br.render_console_summary(report)
        self.assertIn("unbaselined suites: cxx", console)

    def test_format_ns_units(self) -> None:
        self.assertEqual(br.format_ns(None), "—")
        self.assertEqual(br.format_ns(float("nan")), "—")
        self.assertEqual(br.format_ns(float("inf")), "∞")
        self.assertEqual(br.format_ns(12.345), "12.35 ns")
        self.assertEqual(br.format_ns(4_402.0), "4.40 µs")
        self.assertEqual(br.format_ns(3_520_000.0), "3.52 ms")
        self.assertEqual(br.format_ns(1_250_000_000.0), "1.25 s")


if __name__ == "__main__":
    unittest.main()
