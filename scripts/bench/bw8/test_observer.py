"""Synthetic observer checks only; never a production RED or hardware result."""

import copy
import datetime as dt
import json
from pathlib import Path
import unittest

try:
    from observer import evaluate, read_jsonl
except ModuleNotFoundError:
    # Tests-first checkpoint: no observer implementation exists yet.
    def evaluate(rows, readiness_seconds=120):
        return {"verdict": "UNIMPLEMENTED"}

    def read_jsonl(text):
        return []


def record(kind, second, code=0, **changes):
    row = {
        "kind": kind,
        "mono": second,
        "utc": (dt.datetime(2026, 10, 24, tzinfo=dt.timezone.utc)
                + dt.timedelta(seconds=second)).isoformat(),
        "boot": "2026-10-23T23:59:00+00:00",
        "code": code,
        "driver": "32.0.15.8088",
        "events": [],
        "dumps": [],
        "complete": True,
    }
    row.update(changes)
    return row


def control():
    start = record("start", 0, 22, cursors={"System": 100, "Application": 200})
    return [start, record("enable", 1, 22), record("sample", 2)] + [
        record("end" if n == 62 else "sample", n) for n in range(7, 63, 5)
    ]


def event(channel="System", provider="Microsoft-Windows-WER-SystemErrorReporting",
          number=1001, rid=101):
    return {"channel": channel, "provider": provider, "id": number, "record_id": rid}


class ObserverTests(unittest.TestCase):
    def verdict(self, rows, expected="FAIL", **kwargs):
        self.assertEqual(evaluate(rows, **kwargs)["verdict"], expected)

    def test_pass_control(self):
        self.verdict(control(), "REGRESSION_PASS")

    def test_nonfatal_dumps(self):
        rows = control()
        rows[2]["dumps"] = [{"id": "display-a", "code": "0x1a8"}]
        rows[3]["dumps"] = [{"id": "display-b", "code": "0x1b8"},
                             {"id": "display-c", "code": "0x193"}]
        self.verdict(rows, "REGRESSION_PASS")

    def test_application_wer_is_separate(self):
        rows = control()
        rows[2]["events"] = [event("Application", "Windows Error Reporting", rid=201)]
        self.verdict(rows, "REGRESSION_PASS")

    def test_old_fatal_dump_is_baselined(self):
        rows = control()
        for row in rows:
            row["dumps"] = [{"id": "old", "code": "0x141"}]
        self.verdict(rows, "REGRESSION_PASS")

    def test_early_141_before_code0(self):
        rows = control()
        rows[1]["dumps"] = [{"id": "engine", "code": "0x141"}]
        self.verdict(rows)

    def test_early_113_before_hold(self):
        rows = control()
        rows[2]["dumps"] = [{"id": "fatal", "code": "0x113"}]
        self.verdict(rows)

    def test_system_bugcheck_before_code0(self):
        rows = control()
        rows[1]["events"] = [event()]
        self.verdict(rows)

    def test_system_bugcheck_at_end(self):
        rows = control()
        rows[-1]["events"] = [event(provider="BugCheck")]
        self.verdict(rows)

    def test_kernel_power(self):
        rows = control()
        rows[2]["events"] = [event(provider="Microsoft-Windows-Kernel-Power", number=41)]
        self.verdict(rows)

    def test_eventlog_reboot(self):
        rows = control()
        rows[2]["events"] = [event(provider="EventLog", number=6008)]
        self.verdict(rows)

    def test_application_provider_cannot_mask_system_bugcheck(self):
        rows = control()
        rows[2]["events"] = [event(provider="Windows Error Reporting")]
        self.verdict(rows)

    def test_wrong_channel_cannot_be_bugcheck(self):
        rows = control()
        rows[2]["events"] = [event("Application", rid=201)]
        self.verdict(rows)

    def test_utc_rollback(self):
        rows = control()
        rows[-1]["utc"] = rows[2]["utc"]
        self.verdict(rows)

    def test_monotonic_rollback(self):
        rows = control()
        rows[-1]["mono"] = 1
        self.verdict(rows)

    def test_boot_change(self):
        rows = control()
        rows[-1]["boot"] = "2026-10-24T00:00:30+00:00"
        self.verdict(rows)

    def test_boot_missing(self):
        rows = control()
        del rows[3]["boot"]
        self.verdict(rows)

    def test_short_hold(self):
        rows = control()[:-1]
        rows[-1]["kind"] = "end"
        self.verdict(rows)

    def test_just_under_sixty(self):
        rows = control()
        rows[-1] = record("end", 61.999)
        self.verdict(rows)

    def test_forward_utc_jump(self):
        rows = control()
        rows[-1]["utc"] = record("sample", 120)["utc"]
        self.verdict(rows)

    def test_second_heartbeat_gap(self):
        rows = control()
        del rows[5:8]
        self.verdict(rows)

    def test_unknown_fatal_classification(self):
        rows = control()
        rows[-1]["dumps"] = [{"id": "new", "code": "not-parsed"}]
        self.verdict(rows)

    def test_partial_final_query(self):
        rows = control()
        rows[-1]["complete"] = False
        self.verdict(rows)

    def test_initial_device_missing(self):
        rows = control()
        rows[0]["code"] = 43
        self.verdict(rows)

    def test_second_driver_change(self):
        rows = control()
        rows[3]["driver"] = "32.0.15.9999"
        self.verdict(rows)

    def test_final_code_lost(self):
        rows = control()
        rows[-1]["code"] = 43
        self.verdict(rows)

    def test_repeated_enable_at_end(self):
        rows = control()
        rows[-1]["kind"] = "enable"
        self.verdict(rows)

    def test_application_cursor_stale(self):
        rows = control()
        rows[2]["events"] = [event("Application", "Windows Error Reporting", rid=200)]
        self.verdict(rows)

    def test_late_first_code0(self):
        self.verdict(control(), readiness_seconds=0.75)

    def test_no_code0(self):
        rows = control()
        for row in rows[2:]:
            row["code"] = 43
        self.verdict(rows)

    def test_late_readiness(self):
        self.verdict(control(), readiness_seconds=0.5)

    def test_readiness_deadline_zero(self):
        self.verdict(control(), readiness_seconds=0)

    def test_driver_changed(self):
        rows = control()
        rows[-1]["driver"] = "other"
        self.verdict(rows)

    def test_code_lost(self):
        rows = control()
        rows[5]["code"] = 43
        self.verdict(rows)

    def test_missing_end(self):
        self.verdict(control()[:-1])

    def test_partial_query(self):
        rows = control()
        rows[2]["complete"] = False
        self.verdict(rows)

    def test_heartbeat_gap(self):
        rows = control()
        del rows[3:6]
        self.verdict(rows)

    def test_unknown_dump(self):
        rows = control()
        rows[2]["dumps"] = [{"id": "unknown", "code": ""}]
        self.verdict(rows)

    def test_baseline_not_disabled(self):
        rows = control()
        rows[0]["code"] = 0
        self.verdict(rows)

    def test_second_enable(self):
        rows = control()
        rows[4]["kind"] = "enable"
        self.verdict(rows)

    def test_event_cursor_reset(self):
        rows = control()
        rows[2]["events"] = [event(rid=99)]
        self.verdict(rows)

    def test_missing_baseline_cursor(self):
        rows = control()
        del rows[0]["cursors"]["Application"]
        self.verdict(rows)

    def test_boolean_clock(self):
        rows = control()
        rows[2]["mono"] = True
        self.verdict(rows)

    def test_naive_utc(self):
        rows = control()
        rows[2]["utc"] = "2026-10-24T00:00:02"
        self.verdict(rows)

    def test_empty_input(self):
        self.verdict([])

    def test_partial_jsonl(self):
        with self.assertRaises(ValueError):
            read_jsonl(json.dumps(control()[0]))

    def test_malformed_jsonl(self):
        with self.assertRaises(ValueError):
            read_jsonl('{broken}\n')

    def test_jsonl_roundtrip(self):
        rows = control()
        self.assertEqual(read_jsonl(''.join(json.dumps(r) + '\n' for r in rows)), rows)


if __name__ == "__main__":
    unittest.main()
