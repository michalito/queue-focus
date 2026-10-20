#!/usr/bin/env python3
"""The preflight must reject unsupported runtimes before the installer writes."""
import importlib.machinery
import importlib.util
from pathlib import Path
import subprocess
import sys
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
loader = importlib.machinery.SourceFileLoader(
    "runtime", str(Path(__file__).with_name("check-runtime"))
)
spec = importlib.util.spec_from_loader(loader.name, loader)
runtime = importlib.util.module_from_spec(spec)
loader.exec_module(runtime)


class RuntimeTests(unittest.TestCase):
    def test_supported_baseline_and_shell_majors(self):
        for major in (46, 48, 49, 50):
            with self.subTest(shell=major), patch.object(
                runtime, "library_version", side_effect=[(4, 14, 0), (1, 5, 0), (2, 80, 0)]
            ), patch.object(runtime.subprocess, "run", return_value=subprocess.CompletedProcess(
                [], 0, stdout=f"GNOME Shell {major}.1\n"
            )):
                runtime.check(shell=True)

    def test_old_libraries_are_rejected_individually(self):
        for versions, label in (
            ([(4, 12, 5)], "GTK 4.14.0"),
            ([(4, 14, 5), (1, 4, 0)], "libadwaita 1.5.0"),
            ([(4, 14, 5), (1, 5, 0), (2, 78, 0)], "GLib 2.80.0"),
            ([(5, 0, 0)], "GTK 4.14.0"),
        ):
            with self.subTest(label=label), patch.object(
                runtime, "library_version", side_effect=versions
            ), self.assertRaisesRegex(RuntimeError, label):
                runtime.check()

    def test_unavailable_library_is_actionable(self):
        with patch.object(runtime, "library_version", side_effect=OSError("not found")), \
                self.assertRaisesRegex(RuntimeError, "cannot query GTK runtime"):
            runtime.check()

    def test_untested_shell_is_not_silently_enabled(self):
        with patch.object(runtime, "library_version", side_effect=[(4, 14, 5), (1, 5, 0), (2, 80, 0)]), \
                patch.object(runtime.subprocess, "run", return_value=subprocess.CompletedProcess(
                    [], 0, stdout="GNOME Shell 47.0\n"
                )), self.assertRaisesRegex(RuntimeError, "GNOME Shell 46, 48, 49, 50"):
            runtime.check(shell=True)


if __name__ == "__main__":
    unittest.main()
