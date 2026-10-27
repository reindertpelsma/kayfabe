"""Bounded per-arm logs belong to the local result, without a new Worker protocol."""

import hashlib
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
from contextlib import redirect_stdout
from unittest.mock import patch

import vast_payload as payload
import vast_report as report
from test_vast_box import HARDWARE, good_env


def record(text="source evidence", admitted=True):
    raw = text.encode()
    return {
        "arm": "--defer-liveness",
        "mode": "native",
        "rc": 0 if admitted else 6,
        "admitted": admitted,
        "text": text,
        "sha256": hashlib.sha256(raw).hexdigest(),
        "source_bytes": len(raw),
        "dropped_bytes": 0,
    }


class ResultLogs(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.state = self.root / "state.json"
        self.state.write_text(json.dumps({"bytes": 0, "events": []}))
        self.env = patch.dict(
            os.environ,
            {
                "KF_PAYLOAD_DIAGNOSTIC_STATE": str(self.state),
                "KF_PAYLOAD_LOG": str(self.root / "output.log"),
            },
        )
        self.env.start()
        self.addCleanup(self.env.stop)
        self.path = self.root / "result.json"

    def result(self, verdict="PASS"):
        return {
            "verdict": verdict,
            "suites": [payload.row("raw_suite", 30)],
            "hardware": dict(HARDWARE),
        }

    def persisted(self, text="evidence", admitted=True, mode="native"):
        payload.arm_diagnostic("--defer-liveness", text, 0 if admitted else 6, admitted, mode)
        payload.persist_result(self.path, self.result())
        return json.loads(self.path.read_text())

    def test_clean_native_log_is_in_the_structured_file(self):
        self.assertEqual(self.persisted("native proof")["diagnostics"][0]["text"], "native proof")

    def test_guest_log_is_in_the_structured_file(self):
        item = self.persisted("guest serial and QEMU proof", mode="guest")["diagnostics"][0]
        self.assertEqual(item["text"], "guest serial and QEMU proof")
        self.assertEqual(item["mode"], "guest")

    def test_source_hash_and_size_are_for_the_complete_original(self):
        text = "HEAD\n" + "x" * 100000 + "\nTAIL"
        item = self.persisted(text, admitted=False)["diagnostics"][0]
        self.assertEqual(item["sha256"], hashlib.sha256(text.encode()).hexdigest())
        self.assertEqual(item["source_bytes"], len(text.encode()))
        self.assertGreater(item["dropped_bytes"], 0)
        self.assertTrue(item["text"].startswith("HEAD"))
        self.assertTrue(item["text"].endswith("TAIL"))
        self.assertLessEqual(len(item["text"].encode()), 2048)

    def test_guest_source_hash_covers_text_beyond_its_preview(self):
        text = "QEMU_HEAD\n" + "\N{SNOWMAN}" * 10000 + "\nSERIAL_TAIL"
        item = self.persisted(text, admitted=False, mode="guest")["diagnostics"][0]
        self.assertEqual(item["sha256"], hashlib.sha256(text.encode()).hexdigest())
        self.assertTrue(item["text"].startswith("QEMU_HEAD"))
        self.assertTrue(item["text"].endswith("SERIAL_TAIL"))

    def test_failure_keeps_both_the_log_and_named_suite_row(self):
        payload.arm_diagnostic("--defer-liveness", "EXACT_FAULT_RECORD", 6, False)

        def refuse(_):
            raise payload.EvidenceError("raw_suite")

        stage = dict(name="raw", command=[], output=self.root / "out", progress=[], grade=refuse)
        with redirect_stdout(io.StringIO()):
            rc = payload.execute_stages(
                [stage], 1, self.result("FAIL"), self.path, runner=lambda *_: 6
            )
        doc = json.loads(self.path.read_text())
        self.assertEqual(rc, 1)
        self.assertEqual(doc["diagnostics"][0]["text"], "EXACT_FAULT_RECORD")
        self.assertFalse(doc["diagnostics"][0]["admitted"])
        self.assertIn(payload.row("arm_defer_liveness", failed=1), doc["suites"])

    def test_large_manifest_stays_bounded_and_keeps_the_late_fault(self):
        for _ in range(127):
            payload.arm_diagnostic("--timer", "\x01" * 4000, 0, True)
        payload.arm_diagnostic(
            "--defer-liveness", "HEAD_FAULT\n" + "z" * 50000 + "\nTAIL_FAULT", 6, False
        )
        payload.persist_result(self.path, self.result("FAIL"))
        doc = json.loads(self.path.read_text())
        self.assertEqual(len(doc["diagnostics"]), 128)
        self.assertLessEqual(
            len(json.dumps(doc["diagnostics"], separators=(",", ":")).encode()), 49152
        )
        self.assertLessEqual(self.path.stat().st_size, 65536)
        self.assertIn("HEAD_FAULT", doc["diagnostics"][-1]["text"])
        self.assertIn("TAIL_FAULT", doc["diagnostics"][-1]["text"])

    def test_malformed_state_is_not_silently_ignored(self):
        for item in (dict(record(), rc=True), dict(record(), sha256="not-a-hash")):
            with self.subTest(item=item):
                self.state.write_text(json.dumps({"bytes": 0, "events": [item]}))
                with self.assertRaises(ValueError):
                    payload.persist_result(self.path, self.result())


class ReporterResultLogs(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.path = Path(self.temp.name) / "result.json"

    def doc(self, diagnostics):
        return {
            "verdict": "PASS",
            "suites": [{"name": "raw_suite", "pass": 30, "fail": 0, "skip": 0}],
            "hardware": dict(HARDWARE),
            "diagnostics": diagnostics,
        }

    def test_local_reader_accepts_bounded_native_records(self):
        doc = self.doc([record()])
        self.path.write_text(json.dumps(doc))
        self.assertEqual(report.read_result_file(self.path), doc)

    def test_local_reader_accepts_bounded_guest_records(self):
        doc = self.doc([dict(record(), mode="guest")])
        self.path.write_text(json.dumps(doc))
        self.assertEqual(report.read_result_file(self.path), doc)

    def test_wire_result_stays_exactly_the_worker_contract(self):
        self.path.write_text(json.dumps(self.doc([record()])))
        result = report.final_result(good_env("https://h/p"), 0, False, None, self.path)
        self.assertEqual(result["verdict"], "PASS")
        self.assertEqual(
            set(result), {"nonce", "sha", "runner_nonce", "verdict", "suites", "hardware"}
        )

    def test_guest_diagnostics_also_stay_out_of_the_wire_schema(self):
        self.path.write_text(json.dumps(self.doc([dict(record(), mode="guest")])))
        result = report.final_result(good_env("https://h/p"), 0, False, None, self.path)
        self.assertEqual(result["verdict"], "PASS")
        self.assertNotIn("diagnostics", result)

    def test_bad_native_rc_is_refused_before_wire_serialization(self):
        self.path.write_text(json.dumps(self.doc([dict(record(), rc=True)])))
        result = report.final_result(good_env("https://h/p"), 0, False, None, self.path)
        self.assertEqual(result["verdict"], "FAIL")

    def test_unknown_guest_field_is_refused_before_wire_serialization(self):
        self.path.write_text(json.dumps(self.doc([dict(record(), mode="guest", extra=1)])))
        result = report.final_result(good_env("https://h/p"), 0, False, None, self.path)
        self.assertEqual(result["verdict"], "FAIL")

    def test_validator_caps_an_ascii_manifest_below_the_outer_file_cap(self):
        with self.assertRaises(ValueError):
            report.validate_diagnostics([record("x" * 1000)] * 50)

    def test_validator_counts_json_escapes_in_its_budget(self):
        with self.assertRaises(ValueError):
            report.validate_diagnostics([record("\x01" * 700)] * 13)

    def first_refused(self):
        records = [
            report.diagnostic_record(
                "--defer-liveness",
                "native",
                6,
                False,
                "FIRST_HEAD\n" + "\x01" * 4000 + "\nFIRST_TAIL",
            )
        ]
        records += [report.diagnostic_record("--timer", "guest", 6, False, "\x01" * 4000)] * 127
        return report.compact_diagnostics(records)[0]

    def test_compaction_preserves_the_first_refusal_head(self):
        self.assertIn("FIRST_HEAD", self.first_refused()["text"])

    def test_compaction_preserves_the_first_refusal_tail(self):
        self.assertIn("FIRST_TAIL", self.first_refused()["text"])

    def test_diagnostic_text_never_changes_a_fail_verdict(self):
        doc = self.doc([record("PASS PASS PASS")])
        doc["verdict"] = "FAIL"
        doc["suites"][0]["fail"] = 1
        self.path.write_text(json.dumps(doc))
        result = report.final_result(good_env("https://h/p"), 0, False, None, self.path)
        self.assertEqual(result["verdict"], "FAIL")
        self.assertEqual(result["suites"], doc["suites"])

    def test_malformed_records_fail_closed(self):
        cases = [
            {},
            [dict(record(), unexpected=1)],
            [dict(record(), rc=True)],
            [dict(record(), admitted=1)],
            [dict(record(), mode="other")],
            [dict(record(), sha256="x" * 64)],
            [dict(record(), text="x" * 2049)],
            [dict(record(), source_bytes=-1)],
            [dict(record(), dropped_bytes=999)],
            [record()] * 129,
            [record("\x01" * 1000)] * 128,
        ]
        for records in cases:
            with self.subTest(records=str(records)[:80]):
                self.path.write_text(json.dumps(self.doc(records)))
                result = report.final_result(good_env("https://h/p"), 0, False, None, self.path)
                self.assertEqual(result["verdict"], "FAIL")
                self.assertIn("result_invalid", [row["name"] for row in result["suites"]])
