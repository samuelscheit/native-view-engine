"""Regression tests for scripts/host-runtimes.sh (argument handling, toolchain reporting, temp cleanup)."""

from __future__ import annotations

import os
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent.parent / "host-runtimes.sh"
BASH = "/bin/bash"


def _run(args: list[str], env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [BASH, str(SCRIPT), *args], capture_output=True, text=True, env=env, timeout=60, check=False
    )


def _missing(suite: str, path: str) -> str:
    """`host_runtime_missing_tool <suite>` with only `path` on PATH ("" when nothing is missing)."""
    script = f'source "{SCRIPT}"; PATH="{path}"; host_runtime_missing_tool {suite} || true'
    out = subprocess.run([BASH, "-c", script], capture_output=True, text=True, timeout=30, check=False)
    return out.stdout


class HostRuntimesScriptTest(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.tmp = Path(self._tmp.name)

    def tearDown(self) -> None:
        self._tmp.cleanup()

    def _fake_tools(self, *names: str) -> str:
        bin_dir = self.tmp / "bin"
        bin_dir.mkdir(exist_ok=True)
        for name in names:
            tool = bin_dir / name
            tool.write_text("#!/bin/sh\nexit 0\n")
            tool.chmod(tool.stat().st_mode | stat.S_IEXEC)
        return str(bin_dir)

    def test_unknown_suite_fails_cleanly_and_removes_its_temp_dir(self) -> None:
        env = dict(os.environ, TMPDIR=str(self.tmp))
        env.pop("BUILD_ROOT", None)
        result = _run(["cxx", "bogus"], env)
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn("unknown suite 'bogus'", result.stderr)
        self.assertEqual([p.name for p in self.tmp.glob("nve-host-runtimes.*")], [], "temp dir leaked")

    def test_suite_names_match_exactly(self) -> None:
        env = dict(os.environ, TMPDIR=str(self.tmp))
        env.pop("BUILD_ROOT", None)
        for arg in ("cxx swift", "c", "xx", " ts", "all "):
            result = _run([arg], env)
            self.assertEqual(result.returncode, 2, f"{arg!r}: {result.stderr}")
            self.assertIn(f"unknown suite '{arg}'", result.stderr)

    def test_reports_the_actual_missing_binary(self) -> None:
        self.assertEqual(_missing("java", self._fake_tools()), "javac")
        self.assertEqual(_missing("java", self._fake_tools("javac")), "java")
        self.assertEqual(_missing("java", self._fake_tools("javac", "java")), "")
        self.assertEqual(_missing("dart", self._fake_tools("dart")), "node")
        self.assertEqual(_missing("cxx", self._fake_tools("cmake")), "ctest")
        self.assertEqual(_missing("ts", self._fake_tools("node")), "tsc or npx")
        self.assertEqual(_missing("ts", self._fake_tools("node", "npx")), "")
        self.assertEqual(_missing("nope", self._fake_tools()), "unknown suite nope")

    def test_missing_toolchain_fails_before_running_anything(self) -> None:
        # PATH holds only a fake `javac` plus the coreutils the script itself needs.
        bin_dir = Path(self._fake_tools("javac"))
        for util in ("mktemp", "rm"):
            real = subprocess.run(["/bin/sh", "-c", f"command -v {util}"], capture_output=True, text=True, check=True)
            (bin_dir / util).symlink_to(real.stdout.strip())
        env = dict(os.environ, PATH=str(bin_dir), TMPDIR=str(self.tmp))
        env.pop("BUILD_ROOT", None)
        result = _run(["java"], env)
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("needs 'java'", result.stderr)
        self.assertEqual([p.name for p in self.tmp.glob("nve-host-runtimes.*")], [], "temp dir leaked")

    def test_java_sources_survive_spaces_in_paths(self) -> None:
        root = self.tmp / "src dir" / "dev nve"
        root.mkdir(parents=True)
        for name in ("A.java", "B C.java"):
            (root / name).write_text("class X {}\n")
        (root / "notes.txt").write_text("")
        script = f'source "{SCRIPT}"; host_runtime_java_sources "{self.tmp / "src dir"}"'
        out = subprocess.run([BASH, "-c", script], capture_output=True, timeout=30, check=True).stdout
        found = sorted(Path(p.decode()).name for p in out.split(b"\0") if p)
        self.assertEqual(found, ["A.java", "B C.java"])


if __name__ == "__main__":
    unittest.main()
