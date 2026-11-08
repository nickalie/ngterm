"""Run unit-coverage.sh with stand-in tools; no Rust builds or coverage profiles are needed."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


REPO = Path(__file__).resolve().parents[2]


class UnitCoverageTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="unit-coverage-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        for name in ("scripts/unit-coverage.sh", "scripts/lib/native.sh", "build/pins.env"):
            destination = self.root / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(REPO / name, destination)
        for name in ("ngterm-proto", "ngterm-wpe", "ngterm-wpe-sys", "ngterm-ghostty-vt-sys"):
            (self.root / "crates" / name).mkdir(parents=True)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.log = self.root / "calls.jsonl"
        self.env = {
            key: value for key, value in os.environ.items()
            if not key.startswith(("NGTERM_", "AGTERM_")) and key != "PYTHON"
        }
        self.env.update(PATH=f"{self.bin}{os.pathsep}{os.environ['PATH']}", TOOL_LOG=str(self.log),
                        TEST_TARGET=str(self.root / "target"), TEST_OS="Linux")
        self.tool("uname", 'printf "%s\\n" "$TEST_OS"\n', shell=True)
        self.tool("cargo", '''
import json, os, sys
args = sys.argv[1:]
with open(os.environ["TOOL_LOG"], "a") as log:
    log.write(json.dumps(["cargo", *args]) + "\\n")
if args[0] == "metadata":
    print(json.dumps({"target_directory": os.environ["TEST_TARGET"]}))
elif args[:2] == ["llvm-cov", "show-env"]:
    pass
elif "--no-run" in args:
    sys.exit(int(os.environ.get("BUILD_STATUS", "0")))
else:
    sys.exit(int(os.environ.get("TEST_STATUS", "0")))
''')
        for name in ("python3", "python"):
            self.tool(name, '''
import json, os, sys
with open(os.environ["TOOL_LOG"], "a") as log:
    log.write(json.dumps([os.path.basename(sys.argv[0]), *sys.argv[1:]]) + "\\n")
if sys.argv[1] == "-c":
    exec(sys.argv[2])
else:
    sys.exit(int(os.environ.get("REPORT_STATUS", "0")))
''')

    def tool(self, name, body, shell=False):
        path = self.bin / name
        path.write_text(("#!/usr/bin/env bash\n" if shell else f"#!{sys.executable}\n") + body)
        path.chmod(0o755)
        return path

    def run_script(self, *args, **env):
        return subprocess.run(
            ["bash", str(self.root / "scripts/unit-coverage.sh"), str(self.root / "out"), *args],
            env={**self.env, **env}, capture_output=True, text=True, timeout=10, check=False,
        )

    def calls(self, tool):
        return [call[1:] for call in map(json.loads, self.log.read_text().splitlines()) if call[0] == tool]

    def test_default_selection_leaves_wpe_out_on_windows_and_macos(self):
        for host in ("MINGW64_NT-10.0", "Darwin"):
            with self.subTest(host=host):
                result = self.run_script(TEST_OS=host)
                self.assertEqual(result.returncode, 0, result.stderr)
                build = self.calls("cargo")[-2]
                self.assertIn("--workspace", build)
                self.assertIn(["--exclude", "ngterm-wpe"], [build[i:i + 2] for i in range(len(build))])

    def test_linux_default_selection_and_output_stay_the_same(self):
        result = self.run_script()
        self.assertEqual((result.returncode, result.stdout, result.stderr), (0, "", ""))
        self.assertEqual(self.calls("cargo")[-2:], [
            ["test", "--tests", "--no-run", "--message-format=json", "--workspace",
             "--exclude", "ngterm-ghostty-vt-sys", "--exclude", "ngterm-wpe-sys"],
            ["test", "--tests", "--no-fail-fast", "--workspace",
             "--exclude", "ngterm-ghostty-vt-sys", "--exclude", "ngterm-wpe-sys"],
        ])

    def test_a_store_alias_falls_back_to_python_for_metadata_and_report(self):
        self.tool("python3", 'echo "Python was not found" >&2\nexit 49\n', shell=True)
        result = self.run_script("-p", "ngterm-proto")
        self.assertEqual((result.returncode, result.stderr), (0, ""))
        calls = self.calls("python")
        self.assertTrue(any(call[:1] == ["-c"] and "target_directory" in call[1] for call in calls))
        self.assertTrue(any(call[:2] == ["-m", "agterm_conformance.code_coverage"] for call in calls))

    def test_python_override_is_used_for_both_calls(self):
        override = self.bin / "chosen python"
        shutil.copy2(self.bin / "python", override)
        self.tool("python3", "exit 49\n", shell=True)
        result = self.run_script("-p", "ngterm-proto", PYTHON=str(override))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(any(call[:1] == ["-c"] for call in self.calls(override.name)))
        self.assertTrue(any(call[:1] == ["-m"] for call in self.calls(override.name)))

    def test_harness_arguments_go_only_to_the_test_run(self):
        result = self.run_script("-p", "ngterm-proto", "-p", "ngterm-pty", "--",
                                 "--skip", "a test", "--test-threads=1")
        self.assertEqual(result.returncode, 0, result.stderr)
        build, run = self.calls("cargo")[-2:]
        self.assertEqual(build[-4:], ["-p", "ngterm-proto", "-p", "ngterm-pty"])
        self.assertNotIn("--exclude", build)
        self.assertEqual(run[-4:], ["--", "--skip", "a test", "--test-threads=1"])
        report = self.calls("python3")[-1]
        self.assertEqual(report[-4:], ["--crate", "ngterm-proto", "--crate", "ngterm-pty"])

    def test_harness_arguments_work_with_default_selection_and_empty_tail(self):
        for tail in (("--skip", "test_name"), ()):
            with self.subTest(tail=tail):
                result = self.run_script("--", *tail)
                self.assertEqual(result.returncode, 0, result.stderr)
                build, run = self.calls("cargo")[-2:]
                self.assertIn("--workspace", build)
                self.assertNotIn("--skip", build)
                if tail:
                    self.assertEqual(run[-3:], ["--", *tail])
                else:
                    self.assertNotIn("--", run)

    def test_bad_selection_is_a_usage_error(self):
        for args in (("-p",), ("--skip", "test_name")):
            with self.subTest(args=args):
                result = self.run_script(*args)
                self.assertEqual(result.returncode, 2)
                self.assertIn("usage:", result.stderr)
                self.assertIn("[-- TEST_ARGS...]", result.stderr)

    def test_test_failure_still_reports_but_build_failure_does_not(self):
        result = self.run_script("-p", "ngterm-proto", TEST_STATUS="1")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stderr, "unit-coverage: a test failed (the report is written)\n")
        self.assertEqual(self.calls("python3")[-1][:1], ["-m"])
        self.log.unlink()
        result = self.run_script("-p", "ngterm-proto", BUILD_STATUS="1")
        self.assertEqual(result.returncode, 2)
        self.assertEqual(result.stderr, "unit-coverage: the build failed\n")
        self.assertFalse(any(call[:1] == ["-m"] for call in self.calls("python3")))

    def test_report_failure_is_exit_two(self):
        result = self.run_script("-p", "ngterm-proto", REPORT_STATUS="1")
        self.assertEqual(result.returncode, 2)
        self.assertEqual(result.stderr, "unit-coverage: no report could be made from the run's profiles\n")


if __name__ == "__main__":
    unittest.main()
