"""Include the GPU-free launcher contracts in CI's existing unittest discovery."""
import importlib.util
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]


def load_tests(loader, _tests, _pattern):
    suite = unittest.TestSuite()
    for name, relative in (
        ('hitch_launch_defaults', 'scripts/bench/hitch/test_launch_h.py'),
        ('windows_launch_defaults', 'scripts/bench/windows/test_launcher_defaults.py'),
        ('windows_trace_consumers', 'scripts/bench/windows/test_diagnostic_consumers.py'),
    ):
        spec = importlib.util.spec_from_file_location(name, ROOT / relative)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        suite.addTests(loader.loadTestsFromModule(module))
    return suite
