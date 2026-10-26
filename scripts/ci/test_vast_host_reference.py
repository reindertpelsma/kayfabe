"""Falsifier: native-host evidence is dropped or overwritten by a later guest failure."""

import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


def load(name):
    path = Path(__file__).with_name(name + ".py")
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class HostReferenceTests(unittest.TestCase):
    def execute(self, failed=None):
        payload = load("vast_payload")
        stages = []
        for name in ["provision", "cuda_host", "guest"]:

            def grade(rc, name=name):
                if name == failed:
                    raise payload.EvidenceError(name)
                return [payload.row(name, 1)]

            stages.append(dict(name=name, grade=grade, command=[], output="", progress=[]))
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "result.json"
            result = {"verdict": "FAIL", "suites": [], "hardware": {}}
            payload.execute_stages(stages, 0, result, path, runner=lambda *args: 0)
            return json.loads(path.read_text())

    def test_native_host_pass_is_green(self):
        self.assertEqual(self.execute()["host_reference"], "GREEN")

    def test_guest_failure_retains_green_host_reference(self):
        self.assertEqual(self.execute("guest")["host_reference"], "GREEN")

    def test_native_host_failure_is_not_green(self):
        self.assertEqual(self.execute("cuda_host")["host_reference"], "NOT_GREEN")

    def test_provision_failure_has_no_native_reference(self):
        self.assertEqual(self.execute("provision")["host_reference"], "MISSING")

    def test_reporter_forwards_reference(self):
        report = load("vast_report")
        env = {"KF_NONCE": "a" * 32, "KF_SHA": "b" * 40, "KF_RUNNER_NONCE": "c" * 64}
        for reference in ["GREEN", "NOT_GREEN"]:
            with self.subTest(reference=reference), tempfile.TemporaryDirectory() as folder:
                path = Path(folder) / "result.json"
                doc = {
                    "verdict": "FAIL",
                    "suites": [
                        {
                            "name": "guest",
                            "pass": 0,
                            "fail": 1,
                            "skip": 0,
                        }
                    ],
                    "host_reference": reference,
                }
                path.write_text(json.dumps(doc))
                self.assertEqual(
                    report.final_result(env, 1, False, None, path)["host_reference"], reference
                )


if __name__ == "__main__":
    unittest.main()
