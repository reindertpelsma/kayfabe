"""Run the verbatim S4 shared-guard test modules in the payload CI job."""

import importlib.util
from pathlib import Path
import unittest

TESTS = Path(__file__).resolve().parents[1] / "bench/tests"


def load_tests(loader, tests, pattern):
    result = unittest.TestSuite()
    for name in (
        "test_native_xid_guard",
        "test_native_xid_route_round2",
        "test_guest_xid_guard",
        "test_owner_copy_family_rule",
    ):
        spec = importlib.util.spec_from_file_location(name, TESTS / (name + ".py"))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        result.addTests(loader.loadTestsFromModule(module))
    return result
