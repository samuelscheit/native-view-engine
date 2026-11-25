#!/usr/bin/env python3
"""Continuous benchmark regression tracker for native-view-engine.

Covers both Rust Criterion benchmarks (`crates/nve-bench`) and the C++20
Fabric/Yoga/JSI bridge benchmark (`nve_fabric_bridge_bench --json`) using
pure Python 3 standard library modules with zero external dependencies.

Usage examples:
  # Capture a baseline from both Rust Criterion and C++ bridge benchmarks
  python3 scripts/bench-regress.py capture \\
    --baseline benchmarks/baseline.json \\
    --rust \\
    --cxx-bin /tmp/nve-bench-cxx/nve_fabric_bridge_bench

  # Compare an already-captured JSON file against baseline
  python3 scripts/bench-regress.py compare \\
    --baseline benchmarks/baseline.json \\
    --current current.json \\
    --threshold 15.0 \\
    --markdown summary.md

  # Run benchmarks live and compare against baseline
  python3 scripts/bench-regress.py compare \\
    --baseline benchmarks/baseline.json \\
    --current run \\
    --rust \\
    --cxx-bin /tmp/nve-bench-cxx/nve_fabric_bridge_bench \\
    --markdown "$GITHUB_STEP_SUMMARY" \\
    --report-json benchmark-report.json
"""

from __future__ import annotations

import argparse
import datetime
import json
import math
import os
import platform
import subprocess
import sys
import time
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple

SCHEMA_VERSION = 1
DEFAULT_THRESHOLD_PCT = 15.0
DEFAULT_IMPROVEMENT_PCT = 5.0
MICROBENCH_THRESHOLD_NS = 50.0
DEFAULT_MICROBENCH_TOLERANCE_PCT = 25.0

DEFAULT_CRITERION_SAMPLE_SIZE = 20
DEFAULT_CRITERION_WARM_UP_TIME = 0.3
DEFAULT_CRITERION_MEASUREMENT_TIME = 0.5


def repo_root_from_script() -> Path:
    return Path(__file__).resolve().parent.parent


def utc_now_iso() -> str:
    return (
        datetime.datetime.now(datetime.timezone.utc)
        .replace(microsecond=0)
        .isoformat()
        .replace("+00:00", "Z")
    )


def detect_machine_info() -> Dict[str, Any]:
    return {
        "arch": platform.machine() or "unknown",
        "os": platform.system() or "unknown",
        "cpu_count": os.cpu_count() or 0,
    }


def format_ns(ns: Optional[float]) -> str:
    """Format a nanosecond duration cleanly for human-readable tables."""
    if ns is None or math.isnan(ns):
        return "—"
    if math.isinf(ns):
        return "∞"
    abs_ns = abs(ns)
    if abs_ns >= 1_000_000_000.0:
        return f"{ns / 1_000_000_000.0:.2f} s"
    if abs_ns >= 1_000_000.0:
        return f"{ns / 1_000_000.0:.2f} ms"
    if abs_ns >= 1_000.0:
        return f"{ns / 1_000.0:.2f} µs"
    return f"{ns:.2f} ns"


def format_delta_pct(delta_pct: Optional[float]) -> str:
    if delta_pct is None or math.isnan(delta_pct):
        return "—"
    if math.isinf(delta_pct):
        return "+∞%" if delta_pct > 0 else "-∞%"
    sign = "+" if delta_pct > 0 else ""
    return f"{sign}{delta_pct:.2f}%"


def _as_finite_float(value: Any, field_name: str, context: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"{context}: expected numeric '{field_name}', got {value!r}")
    out = float(value)
    if not math.isfinite(out) or out < 0.0:
        raise ValueError(
            f"{context}: expected non-negative finite '{field_name}', got {value!r}"
        )
    return out


def _round_metric(value: float) -> float:
    return round(value, 3)


def make_benchmark_key(suite: str, name: str) -> str:
    return f"{suite}::{name}"


def parse_criterion_dir(
    criterion_dir: Path,
    *,
    min_mtime: Optional[float] = None,
) -> Dict[str, Dict[str, Any]]:
    """Parse Criterion `**/new/estimates.json` files into normalized entries."""
    criterion_dir = Path(criterion_dir)
    if not criterion_dir.is_dir():
        raise FileNotFoundError(f"Criterion directory not found: {criterion_dir}")

    entries: Dict[str, Dict[str, Any]] = {}
    estimates_files = sorted(criterion_dir.glob("**/new/estimates.json"))

    for estimates_path in estimates_files:
        if min_mtime is not None:
            try:
                if estimates_path.stat().st_mtime < min_mtime:
                    continue
            except OSError:
                continue

        try:
            data = json.loads(estimates_path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as exc:
            raise ValueError(f"Failed to parse Criterion estimates {estimates_path}: {exc}") from exc

        if not isinstance(data, dict):
            raise ValueError(f"Invalid Criterion estimates object in {estimates_path}")

        median_obj = data.get("median")
        if not isinstance(median_obj, dict) or "point_estimate" not in median_obj:
            raise ValueError(
                f"Missing 'median.point_estimate' in Criterion estimates: {estimates_path}"
            )
        median_ns = _as_finite_float(
            median_obj["point_estimate"],
            "median.point_estimate",
            str(estimates_path),
        )

        # Prefer Criterion's canonical `full_id` from sibling `benchmark.json` when available,
        # since directory names replace `/` with `_` on disk.
        bench_name: Optional[str] = None
        benchmark_meta_path = estimates_path.parent / "benchmark.json"
        if benchmark_meta_path.is_file():
            try:
                meta = json.loads(benchmark_meta_path.read_text(encoding="utf-8"))
                if isinstance(meta, dict) and isinstance(meta.get("full_id"), str):
                    candidate = meta["full_id"].strip()
                    if candidate:
                        bench_name = candidate
            except (OSError, json.JSONDecodeError):
                pass

        if bench_name is None:
            rel_parent = estimates_path.parent.parent.relative_to(criterion_dir)
            bench_name = rel_parent.as_posix()

        entry: Dict[str, Any] = {
            "suite": "rust",
            "name": bench_name,
            "median_ns": _round_metric(median_ns),
        }

        mean_obj = data.get("mean")
        if isinstance(mean_obj, dict) and "point_estimate" in mean_obj:
            try:
                entry["mean_ns"] = _round_metric(
                    _as_finite_float(
                        mean_obj["point_estimate"],
                        "mean.point_estimate",
                        str(estimates_path),
                    )
                )
            except ValueError:
                pass

        key = make_benchmark_key("rust", bench_name)
        entries[key] = entry

    return dict(sorted(entries.items()))


def parse_cxx_json(raw_json: str) -> Dict[str, Dict[str, Any]]:
    """Parse structured JSON output from `nve_fabric_bridge_bench --json`."""
    try:
        payload = json.loads(raw_json)
    except json.JSONDecodeError as exc:
        raise ValueError(f"Failed to parse C++ benchmark JSON: {exc}") from exc

    if isinstance(payload, dict):
        items = payload.get("benchmarks")
        if not isinstance(items, list):
            raise ValueError("C++ benchmark JSON object must contain a 'benchmarks' list")
    elif isinstance(payload, list):
        items = payload
    else:
        raise ValueError("C++ benchmark JSON must be an object with 'benchmarks' or a list")

    entries: Dict[str, Dict[str, Any]] = {}
    for idx, item in enumerate(items):
        ctx = f"cxx benchmark[{idx}]"
        if not isinstance(item, dict):
            raise ValueError(f"{ctx}: expected object, got {type(item).__name__}")
        name = item.get("name")
        if not isinstance(name, str) or not name.strip():
            raise ValueError(f"{ctx}: missing non-empty string 'name'")
        name = name.strip()

        if "median_ns" in item and item["median_ns"] is not None:
            median_ns = _as_finite_float(item["median_ns"], "median_ns", f"{ctx} ({name})")
        elif "p50_ns" in item and item["p50_ns"] is not None:
            median_ns = _as_finite_float(item["p50_ns"], "p50_ns", f"{ctx} ({name})")
        else:
            raise ValueError(f"{ctx} ({name}): missing 'median_ns' or 'p50_ns'")

        entry: Dict[str, Any] = {
            "suite": "cxx",
            "name": name,
            "median_ns": _round_metric(median_ns),
        }

        if item.get("p95_ns") is not None:
            entry["p95_ns"] = _round_metric(
                _as_finite_float(item["p95_ns"], "p95_ns", f"{ctx} ({name})")
            )
        if item.get("p99_ns") is not None:
            entry["p99_ns"] = _round_metric(
                _as_finite_float(item["p99_ns"], "p99_ns", f"{ctx} ({name})")
            )
        if item.get("mean_ns") is not None:
            entry["mean_ns"] = _round_metric(
                _as_finite_float(item["mean_ns"], "mean_ns", f"{ctx} ({name})")
            )
        if item.get("ops_per_sec") is not None:
            entry["ops_per_sec"] = _round_metric(
                _as_finite_float(item["ops_per_sec"], "ops_per_sec", f"{ctx} ({name})")
            )
        elif item.get("throughput") is not None:
            entry["ops_per_sec"] = _round_metric(
                _as_finite_float(item["throughput"], "throughput", f"{ctx} ({name})")
            )

        key = make_benchmark_key("cxx", name)
        entries[key] = entry

    return dict(sorted(entries.items()))


def build_snapshot(
    benchmarks: Dict[str, Dict[str, Any]],
    *,
    default_tolerance_pct: float = DEFAULT_THRESHOLD_PCT,
    microbench_tolerance_pct: Optional[float] = DEFAULT_MICROBENCH_TOLERANCE_PCT,
    existing_tolerances: Optional[Dict[str, float]] = None,
    machine: Optional[Dict[str, Any]] = None,
    generated_at: Optional[str] = None,
) -> Dict[str, Any]:
    """Construct a deterministic, sorted snapshot/baseline JSON payload."""
    normalized: Dict[str, Dict[str, Any]] = {}
    existing_tolerances = existing_tolerances or {}

    for key in sorted(benchmarks.keys()):
        raw = benchmarks[key]
        entry = dict(raw)
        median_ns = _as_finite_float(entry["median_ns"], "median_ns", key)
        entry["median_ns"] = _round_metric(median_ns)

        if "tolerance_pct" in entry and entry["tolerance_pct"] is not None:
            tol = _as_finite_float(entry["tolerance_pct"], "tolerance_pct", key)
        elif key in existing_tolerances:
            tol = float(existing_tolerances[key])
        elif (
            microbench_tolerance_pct is not None
            and median_ns < MICROBENCH_THRESHOLD_NS
        ):
            tol = max(float(default_tolerance_pct), float(microbench_tolerance_pct))
        else:
            tol = float(default_tolerance_pct)
        entry["tolerance_pct"] = _round_metric(tol)

        normalized[key] = dict(sorted(entry.items()))

    return {
        "schema_version": SCHEMA_VERSION,
        "generated_at": generated_at or utc_now_iso(),
        "machine": machine or detect_machine_info(),
        "default_tolerance_pct": _round_metric(float(default_tolerance_pct)),
        "benchmarks": normalized,
    }


def write_json_deterministic(path: Path, payload: Dict[str, Any]) -> None:
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    text = json.dumps(payload, indent=2, sort_keys=False) + "\n"
    path.write_text(text, encoding="utf-8")


def load_snapshot(path: Path) -> Dict[str, Any]:
    """Load a snapshot/baseline JSON file (or raw C++ --json output)."""
    path = Path(path)
    try:
        raw_text = path.read_text(encoding="utf-8")
        data = json.loads(raw_text)
    except (OSError, json.JSONDecodeError) as exc:
        raise ValueError(f"Failed to read snapshot {path}: {exc}") from exc

    if isinstance(data, list) or (
        isinstance(data, dict) and isinstance(data.get("benchmarks"), list)
    ):
        entries = parse_cxx_json(raw_text)
        return build_snapshot(entries, microbench_tolerance_pct=None)

    if not isinstance(data, dict) or not isinstance(data.get("benchmarks"), dict):
        raise ValueError(
            f"Invalid benchmark snapshot format in {path}: expected object with 'benchmarks' map"
        )

    normalized: Dict[str, Dict[str, Any]] = {}
    for key, raw_entry in sorted(data["benchmarks"].items()):
        if not isinstance(raw_entry, dict):
            raise ValueError(f"{path}: benchmark '{key}' must be an object")
        entry = dict(raw_entry)
        entry["median_ns"] = _as_finite_float(
            entry.get("median_ns"), "median_ns", f"{path}:{key}"
        )
        if "suite" not in entry:
            entry["suite"] = key.split("::", 1)[0] if "::" in key else "unknown"
        if "name" not in entry:
            entry["name"] = key.split("::", 1)[1] if "::" in key else key
        if "tolerance_pct" in entry and entry["tolerance_pct"] is not None:
            entry["tolerance_pct"] = _as_finite_float(
                entry["tolerance_pct"], "tolerance_pct", f"{path}:{key}"
            )
        normalized[key] = entry

    return {
        "schema_version": data.get("schema_version", SCHEMA_VERSION),
        "generated_at": data.get("generated_at", ""),
        "machine": data.get("machine", {}),
        "default_tolerance_pct": data.get("default_tolerance_pct", DEFAULT_THRESHOLD_PCT),
        "benchmarks": normalized,
    }


def compare_snapshots(
    baseline: Dict[str, Any],
    current: Dict[str, Any],
    *,
    threshold_pct: float = DEFAULT_THRESHOLD_PCT,
    improvement_pct: float = DEFAULT_IMPROVEMENT_PCT,
    fail_on_missing: bool = False,
) -> Dict[str, Any]:
    """Compare current benchmarks against baseline and classify every entry."""
    base_benches: Dict[str, Dict[str, Any]] = baseline.get("benchmarks", {})
    curr_benches: Dict[str, Dict[str, Any]] = current.get("benchmarks", {})

    all_keys = sorted(set(base_benches.keys()) | set(curr_benches.keys()))

    rows: List[Dict[str, Any]] = []
    regressions: List[Dict[str, Any]] = []
    improvements: List[Dict[str, Any]] = []
    missing: List[Dict[str, Any]] = []
    new_benches: List[Dict[str, Any]] = []
    ok_count = 0

    for key in all_keys:
        in_base = key in base_benches
        in_curr = key in curr_benches

        if in_base and in_curr:
            b = base_benches[key]
            c = curr_benches[key]
            base_ns = float(b["median_ns"])
            curr_ns = float(c["median_ns"])
            tol_pct = (
                float(b["tolerance_pct"])
                if b.get("tolerance_pct") is not None
                else float(threshold_pct)
            )

            if base_ns > 0.0:
                delta_pct: Optional[float] = ((curr_ns - base_ns) / base_ns) * 100.0
            elif curr_ns == 0.0:
                delta_pct = 0.0
            else:
                delta_pct = None

            if delta_pct is None or delta_pct > tol_pct:
                status = "regression"
            elif delta_pct <= -float(improvement_pct):
                status = "improved"
            else:
                status = "ok"

            row = {
                "key": key,
                "suite": c.get("suite") or b.get("suite") or key.split("::", 1)[0],
                "name": c.get("name") or b.get("name") or key,
                "baseline_ns": _round_metric(base_ns),
                "current_ns": _round_metric(curr_ns),
                "delta_pct": round(delta_pct, 2) if delta_pct is not None else None,
                "tolerance_pct": _round_metric(tol_pct),
                "status": status,
            }
            rows.append(row)
            if status == "regression":
                regressions.append(row)
            elif status == "improved":
                improvements.append(row)
            else:
                ok_count += 1

        elif in_base and not in_curr:
            b = base_benches[key]
            tol_pct = (
                float(b["tolerance_pct"])
                if b.get("tolerance_pct") is not None
                else float(threshold_pct)
            )
            row = {
                "key": key,
                "suite": b.get("suite") or key.split("::", 1)[0],
                "name": b.get("name") or key,
                "baseline_ns": _round_metric(float(b["median_ns"])),
                "current_ns": None,
                "delta_pct": None,
                "tolerance_pct": _round_metric(tol_pct),
                "status": "missing",
            }
            rows.append(row)
            missing.append(row)

        else:
            c = curr_benches[key]
            row = {
                "key": key,
                "suite": c.get("suite") or key.split("::", 1)[0],
                "name": c.get("name") or key,
                "baseline_ns": None,
                "current_ns": _round_metric(float(c["median_ns"])),
                "delta_pct": None,
                "tolerance_pct": _round_metric(float(threshold_pct)),
                "status": "new",
            }
            rows.append(row)
            new_benches.append(row)

    base_machine = baseline.get("machine") or {}
    curr_machine = current.get("machine") or {}
    machine_mismatch = bool(
        base_machine
        and curr_machine
        and (
            base_machine.get("os") != curr_machine.get("os")
            or base_machine.get("arch") != curr_machine.get("arch")
        )
    )

    passed = len(regressions) == 0 and (not fail_on_missing or len(missing) == 0)

    return {
        "schema_version": SCHEMA_VERSION,
        "compared_at": utc_now_iso(),
        "threshold_pct": _round_metric(float(threshold_pct)),
        "improvement_pct": _round_metric(float(improvement_pct)),
        "passed": passed,
        "machine_mismatch": machine_mismatch,
        "baseline": {
            "generated_at": baseline.get("generated_at", ""),
            "machine": base_machine,
        },
        "current": {
            "generated_at": current.get("generated_at", ""),
            "machine": curr_machine,
        },
        "summary": {
            "total": len(rows),
            "compared": ok_count + len(regressions) + len(improvements),
            "regressions": len(regressions),
            "improved": len(improvements),
            "ok": ok_count,
            "new": len(new_benches),
            "missing": len(missing),
        },
        "regressions": regressions,
        "improvements": improvements,
        "new": new_benches,
        "missing": missing,
        "rows": rows,
    }


def _status_badge(status: str) -> str:
    return {
        "regression": "REGRESSION",
        "improved": "IMPROVED",
        "ok": "OK",
        "new": "NEW",
        "missing": "MISSING",
    }.get(status, status.upper())


def _format_machine(machine: Dict[str, Any]) -> str:
    if not machine:
        return "unknown"
    os_name = machine.get("os", "unknown")
    arch = machine.get("arch", "unknown")
    cpus = machine.get("cpu_count")
    if cpus:
        return f"{os_name}/{arch} ({cpus} CPUs)"
    return f"{os_name}/{arch}"


def render_markdown(report: Dict[str, Any]) -> str:
    """Render a GitHub-flavored Markdown summary table for `$GITHUB_STEP_SUMMARY`."""
    summary = report["summary"]
    passed = report["passed"]
    verdict = "PASS" if passed else "FAIL"

    lines: List[str] = [
        f"## Benchmark Regression Report — {verdict}",
        "",
        (
            f"**Summary:** {summary['compared']} compared · "
            f"{summary['regressions']} regressions · "
            f"{summary['improved']} improved · "
            f"{summary['ok']} ok · "
            f"{summary['new']} new · "
            f"{summary['missing']} missing "
            f"(default threshold: `+{report['threshold_pct']:.1f}%`)"
        ),
        "",
        (
            f"- **Baseline:** `{_format_machine(report.get('baseline', {}).get('machine', {}))}` "
            f"({report.get('baseline', {}).get('generated_at') or 'n/a'})"
        ),
        (
            f"- **Current:** `{_format_machine(report.get('current', {}).get('machine', {}))}` "
            f"({report.get('current', {}).get('generated_at') or 'n/a'})"
        ),
    ]

    if report.get("machine_mismatch"):
        lines.extend(
            [
                "",
                (
                    "> **Note:** Baseline and current measurements were captured on "
                    "different OS/architecture environments; interpret absolute deltas accordingly."
                ),
            ]
        )

    if report.get("regressions"):
        lines.extend(["", "### Regressions", ""])
        for row in report["regressions"]:
            delta_str = (
                format_delta_pct(row["delta_pct"])
                if row["delta_pct"] is not None
                else "+∞%"
            )
            lines.append(
                f"- `{row['suite']}::{row['name']}`: "
                f"{format_ns(row['baseline_ns'])} -> {format_ns(row['current_ns'])} "
                f"(**{delta_str}**, limit `+{row['tolerance_pct']:.1f}%`)"
            )

    lines.extend(
        [
            "",
            "| Suite | Benchmark | Baseline | Current | Delta | Tolerance | Status |",
            "| :--- | :--- | ---: | ---: | ---: | ---: | :---: |",
        ]
    )

    for row in report.get("rows", []):
        delta_str = (
            "+∞%"
            if row["status"] == "regression" and row["delta_pct"] is None
            else format_delta_pct(row["delta_pct"])
        )
        tol_str = f"+{row['tolerance_pct']:.1f}%"
        lines.append(
            f"| `{row['suite']}` | `{row['name']}` | "
            f"{format_ns(row['baseline_ns'])} | "
            f"{format_ns(row['current_ns'])} | "
            f"{delta_str} | {tol_str} | **{_status_badge(row['status'])}** |"
        )

    lines.append("")
    return "\n".join(lines)


def render_console_summary(report: Dict[str, Any]) -> str:
    """Compact terminal table for local developer runs."""
    s = report["summary"]
    verdict = "PASS" if report["passed"] else "FAIL"
    header = (
        f"Benchmark Comparison [{verdict}]: "
        f"{s['compared']} compared, {s['regressions']} regressions, "
        f"{s['improved']} improved, {s['ok']} ok, {s['new']} new, {s['missing']} missing"
    )
    lines = [header, "-" * 108]
    lines.append(
        f"{'Suite':<6} {'Benchmark':<46} {'Baseline':>12} {'Current':>12} "
        f"{'Delta':>9} {'Limit':>8} {'Status':>10}"
    )
    lines.append("-" * 108)
    for row in report.get("rows", []):
        delta_str = (
            "+inf%"
            if row["status"] == "regression" and row["delta_pct"] is None
            else format_delta_pct(row["delta_pct"])
        )
        tol_str = f"+{row['tolerance_pct']:.1f}%"
        lines.append(
            f"{row['suite']:<6} {row['name']:<46} "
            f"{format_ns(row['baseline_ns']):>12} "
            f"{format_ns(row['current_ns']):>12} "
            f"{delta_str:>9} "
            f"{tol_str:>8} "
            f"{_status_badge(row['status']):>10}"
        )
    lines.append("-" * 108)
    return "\n".join(lines)


def run_rust_benchmarks(
    repo_root: Path,
    *,
    package: str = "nve-bench",
    sample_size: int = DEFAULT_CRITERION_SAMPLE_SIZE,
    warm_up_time: float = DEFAULT_CRITERION_WARM_UP_TIME,
    measurement_time: float = DEFAULT_CRITERION_MEASUREMENT_TIME,
    bench_filter: Optional[str] = None,
) -> float:
    """Execute Rust Criterion benchmarks with the fast statistical profile."""
    start_mtime = time.time() - 1.0
    cmd = [
        "cargo",
        "bench",
        "-p",
        package,
        "--",
        "--sample-size",
        str(sample_size),
        "--warm-up-time",
        str(warm_up_time),
        "--measurement-time",
        str(measurement_time),
        "--noplot",
    ]
    if bench_filter:
        cmd.append(bench_filter)

    print(f"[bench-regress] Running Rust benchmarks: {' '.join(cmd)}", file=sys.stderr)
    proc = subprocess.run(cmd, cwd=str(repo_root), check=False)
    if proc.returncode != 0:
        raise RuntimeError(
            f"Rust Criterion benchmark command failed with exit code {proc.returncode}"
        )
    return start_mtime


def run_cxx_benchmarks(
    cxx_bin: Path,
    *,
    iters: Optional[int] = None,
    warmup: Optional[int] = None,
    bench_filter: Optional[str] = None,
) -> str:
    """Execute `nve_fabric_bridge_bench --json` and return raw JSON stdout."""
    cxx_bin = Path(cxx_bin)
    if not cxx_bin.is_file():
        raise FileNotFoundError(f"C++ benchmark binary not found: {cxx_bin}")

    cmd = [str(cxx_bin), "--json"]
    if warmup is not None:
        cmd.extend(["--warmup", str(warmup)])
    if iters is not None:
        cmd.extend(["--iters", str(iters)])
    if bench_filter:
        cmd.extend(["--filter", bench_filter])

    print(f"[bench-regress] Running C++ benchmarks: {' '.join(cmd)}", file=sys.stderr)
    proc = subprocess.run(
        cmd,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        check=False,
    )
    if proc.returncode != 0:
        raise RuntimeError(
            f"C++ benchmark binary failed with exit code {proc.returncode}: {proc.stderr.strip()}"
        )
    return proc.stdout


def collect_live_benchmarks(
    *,
    repo_root: Path,
    run_rust: bool,
    criterion_dir: Path,
    read_criterion_only: bool,
    cxx_bin: Optional[Path],
    cxx_json: Optional[Path],
    package: str,
    sample_size: int,
    warm_up_time: float,
    measurement_time: float,
    cxx_iters: Optional[int],
    cxx_warmup: Optional[int],
    bench_filter: Optional[str],
) -> Dict[str, Dict[str, Any]]:
    combined: Dict[str, Dict[str, Any]] = {}

    if run_rust:
        start_mtime = run_rust_benchmarks(
            repo_root,
            package=package,
            sample_size=sample_size,
            warm_up_time=warm_up_time,
            measurement_time=measurement_time,
            bench_filter=bench_filter,
        )
        rust_entries = parse_criterion_dir(criterion_dir, min_mtime=start_mtime)
        if not rust_entries:
            raise RuntimeError(
                f"No fresh Criterion estimates.json files found under {criterion_dir}"
            )
        combined.update(rust_entries)
    elif read_criterion_only:
        rust_entries = parse_criterion_dir(criterion_dir)
        combined.update(rust_entries)

    if cxx_bin is not None:
        raw_cxx = run_cxx_benchmarks(
            cxx_bin,
            iters=cxx_iters,
            warmup=cxx_warmup,
            bench_filter=bench_filter,
        )
        combined.update(parse_cxx_json(raw_cxx))
    elif cxx_json is not None:
        raw_cxx = Path(cxx_json).read_text(encoding="utf-8")
        combined.update(parse_cxx_json(raw_cxx))

    if not combined:
        raise RuntimeError(
            "No benchmarks collected. Pass --rust, --criterion-dir, --cxx-bin, or --cxx-json."
        )

    return dict(sorted(combined.items()))


def _add_runner_args(parser: argparse.ArgumentParser) -> None:
    parser.add_argument(
        "--rust",
        action="store_true",
        help="Run Rust Criterion benchmarks (`cargo bench -p nve-bench`).",
    )
    parser.add_argument(
        "--criterion-dir",
        type=Path,
        default=None,
        help="Path to Criterion output directory (default: <repo>/target/criterion).",
    )
    parser.add_argument(
        "--cxx-bin",
        type=Path,
        default=None,
        help="Path to compiled `nve_fabric_bridge_bench` binary.",
    )
    parser.add_argument(
        "--cxx-json",
        type=Path,
        default=None,
        help="Path to pre-captured `nve_fabric_bridge_bench --json` output file.",
    )
    parser.add_argument(
        "--package",
        default="nve-bench",
        help="Cargo package for Rust benchmarks (default: nve-bench).",
    )
    parser.add_argument(
        "--sample-size",
        type=int,
        default=DEFAULT_CRITERION_SAMPLE_SIZE,
        help=f"Criterion --sample-size (default: {DEFAULT_CRITERION_SAMPLE_SIZE}).",
    )
    parser.add_argument(
        "--warm-up-time",
        type=float,
        default=DEFAULT_CRITERION_WARM_UP_TIME,
        help=f"Criterion --warm-up-time seconds (default: {DEFAULT_CRITERION_WARM_UP_TIME}).",
    )
    parser.add_argument(
        "--measurement-time",
        type=float,
        default=DEFAULT_CRITERION_MEASUREMENT_TIME,
        help=f"Criterion --measurement-time seconds (default: {DEFAULT_CRITERION_MEASUREMENT_TIME}).",
    )
    parser.add_argument(
        "--cxx-iters",
        type=int,
        default=None,
        help="Optional --iters override passed to `nve_fabric_bridge_bench`.",
    )
    parser.add_argument(
        "--cxx-warmup",
        type=int,
        default=None,
        help="Optional --warmup override passed to `nve_fabric_bridge_bench`.",
    )
    parser.add_argument(
        "--filter",
        dest="bench_filter",
        default=None,
        help="Optional benchmark name substring filter.",
    )


def build_arg_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="bench-regress.py",
        description="Zero-dependency benchmark regression tracker for Rust Criterion + C++20 Fabric bridge.",
    )
    subparsers = parser.add_subparsers(dest="command", required=True)

    capture_p = subparsers.add_parser(
        "capture",
        help="Run benchmarks and write a deterministic baseline JSON file.",
    )
    capture_p.add_argument(
        "--baseline",
        type=Path,
        required=True,
        help="Output baseline JSON path (e.g. benchmarks/baseline.json).",
    )
    capture_p.add_argument(
        "--tolerance",
        type=float,
        default=DEFAULT_THRESHOLD_PCT,
        help=f"Default per-benchmark tolerance percentage (default: {DEFAULT_THRESHOLD_PCT}).",
    )
    capture_p.add_argument(
        "--microbench-tolerance",
        type=float,
        default=DEFAULT_MICROBENCH_TOLERANCE_PCT,
        help=(
            f"Tolerance percentage for sub-{int(MICROBENCH_THRESHOLD_NS)}ns microbenchmarks "
            f"(default: {DEFAULT_MICROBENCH_TOLERANCE_PCT})."
        ),
    )
    capture_p.add_argument(
        "--preserve-tolerances",
        action="store_true",
        help="Preserve custom per-benchmark tolerance_pct values from an existing baseline file.",
    )
    _add_runner_args(capture_p)

    compare_p = subparsers.add_parser(
        "compare",
        help="Compare current benchmarks (file or live run) against a baseline.",
    )
    compare_p.add_argument(
        "--baseline",
        type=Path,
        required=True,
        help="Baseline JSON path (e.g. benchmarks/baseline.json).",
    )
    compare_p.add_argument(
        "--current",
        required=True,
        help="Path to current snapshot JSON file, or 'run' to execute benchmarks now.",
    )
    compare_p.add_argument(
        "--threshold",
        type=float,
        default=DEFAULT_THRESHOLD_PCT,
        help=f"Global regression threshold percentage when per-metric tolerance is absent (default: {DEFAULT_THRESHOLD_PCT}).",
    )
    compare_p.add_argument(
        "--improvement-threshold",
        type=float,
        default=DEFAULT_IMPROVEMENT_PCT,
        help=f"Minimum speedup percentage to mark as IMPROVED (default: {DEFAULT_IMPROVEMENT_PCT}).",
    )
    compare_p.add_argument(
        "--markdown",
        type=Path,
        default=None,
        help="Write GitHub Markdown summary table to this path (use '-' for stdout).",
    )
    compare_p.add_argument(
        "--report-json",
        type=Path,
        default=None,
        help="Optional output path for the structured JSON comparison report.",
    )
    compare_p.add_argument(
        "--save-current",
        type=Path,
        default=None,
        help="When --current run is used, also write the captured current snapshot to this path.",
    )
    compare_p.add_argument(
        "--fail-on-missing",
        action="store_true",
        help="Exit non-zero if baseline benchmarks are missing from current results.",
    )
    _add_runner_args(compare_p)

    return parser


def cmd_capture(args: argparse.Namespace) -> int:
    repo_root = repo_root_from_script()
    criterion_dir = args.criterion_dir or (repo_root / "target" / "criterion")

    # Determine whether to run Rust live, read an explicit criterion_dir, or skip Rust.
    run_rust = args.rust
    read_criterion_only = (not args.rust) and (args.criterion_dir is not None)
    if not run_rust and not read_criterion_only and args.cxx_bin is None and args.cxx_json is None:
        run_rust = True

    benchmarks = collect_live_benchmarks(
        repo_root=repo_root,
        run_rust=run_rust,
        criterion_dir=criterion_dir,
        read_criterion_only=read_criterion_only,
        cxx_bin=args.cxx_bin,
        cxx_json=args.cxx_json,
        package=args.package,
        sample_size=args.sample_size,
        warm_up_time=args.warm_up_time,
        measurement_time=args.measurement_time,
        cxx_iters=args.cxx_iters,
        cxx_warmup=args.cxx_warmup,
        bench_filter=args.bench_filter,
    )

    existing_tolerances: Dict[str, float] = {}
    if args.preserve_tolerances and args.baseline.is_file():
        prev = load_snapshot(args.baseline)
        for key, entry in prev.get("benchmarks", {}).items():
            if entry.get("tolerance_pct") is not None:
                existing_tolerances[key] = float(entry["tolerance_pct"])

    snapshot = build_snapshot(
        benchmarks,
        default_tolerance_pct=args.tolerance,
        microbench_tolerance_pct=args.microbench_tolerance,
        existing_tolerances=existing_tolerances,
    )
    write_json_deterministic(args.baseline, snapshot)

    rust_count = sum(1 for e in snapshot["benchmarks"].values() if e.get("suite") == "rust")
    cxx_count = sum(1 for e in snapshot["benchmarks"].values() if e.get("suite") == "cxx")
    print(
        f"Captured {len(snapshot['benchmarks'])} benchmarks "
        f"(rust={rust_count}, cxx={cxx_count}) -> {args.baseline}"
    )
    return 0


def cmd_compare(args: argparse.Namespace) -> int:
    repo_root = repo_root_from_script()
    baseline = load_snapshot(args.baseline)

    if args.current == "run":
        criterion_dir = args.criterion_dir or (repo_root / "target" / "criterion")
        run_rust = args.rust
        read_criterion_only = (not args.rust) and (args.criterion_dir is not None)
        if not run_rust and not read_criterion_only and args.cxx_bin is None and args.cxx_json is None:
            run_rust = True

        benchmarks = collect_live_benchmarks(
            repo_root=repo_root,
            run_rust=run_rust,
            criterion_dir=criterion_dir,
            read_criterion_only=read_criterion_only,
            cxx_bin=args.cxx_bin,
            cxx_json=args.cxx_json,
            package=args.package,
            sample_size=args.sample_size,
            warm_up_time=args.warm_up_time,
            measurement_time=args.measurement_time,
            cxx_iters=args.cxx_iters,
            cxx_warmup=args.cxx_warmup,
            bench_filter=args.bench_filter,
        )
        current = build_snapshot(
            benchmarks,
            default_tolerance_pct=args.threshold,
            microbench_tolerance_pct=None,
        )
        if args.save_current is not None:
            write_json_deterministic(args.save_current, current)
    else:
        current = load_snapshot(Path(args.current))

    report = compare_snapshots(
        baseline,
        current,
        threshold_pct=args.threshold,
        improvement_pct=args.improvement_threshold,
        fail_on_missing=args.fail_on_missing,
    )

    if args.report_json is not None:
        write_json_deterministic(args.report_json, report)

    markdown_text = render_markdown(report)
    if args.markdown is not None:
        if str(args.markdown) == "-":
            sys.stdout.write(markdown_text)
        else:
            args.markdown.parent.mkdir(parents=True, exist_ok=True)
            args.markdown.write_text(markdown_text, encoding="utf-8")

    if str(args.markdown) != "-":
        print(render_console_summary(report))

    return 0 if report["passed"] else 1


def main(argv: Optional[List[str]] = None) -> int:
    parser = build_arg_parser()
    args = parser.parse_args(argv)
    try:
        if args.command == "capture":
            return cmd_capture(args)
        if args.command == "compare":
            return cmd_compare(args)
        parser.error(f"Unknown command: {args.command}")
        return 2
    except Exception as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
