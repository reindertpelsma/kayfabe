"""GPU-free tests of the per-stage transferred bytes the payload records (vast_payload.net_counters / record_net) and the report side
that validates them (vast_report.validate_net_bytes, read_result_file). 2026-11-16: calibration data for ci/vast-broker/bandwidth.json."""

import json
from pathlib import Path
import tempfile
import unittest

import vast_payload as payload
import vast_report as report

ROUTE = """Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT
docker0\t000011AC\t00000000\t0001\t0\t0\t0\t0000FFFF\t0\t0\t0
eth1\t00000000\t0100000A\t0003\t0\t0\t200\t00000000\t0\t0\t0
eth0\t00000000\t0100A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0
"""
DEV = """Inter-|   Receive                                                |  Transmit
 face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed
    lo:  111  1 0 0 0 0 0 0  222 1 0 0 0 0 0 0
  eth0: 1000 10 0 0 0 0 0 0  2000 20 0 0 0 0 0 0
  eth1: 9999 10 0 0 0 0 0 0  8888 20 0 0 0 0 0 0
"""


class Counters(unittest.TestCase):
    def files(self, route=ROUTE, dev=DEV):
        d = tempfile.TemporaryDirectory()
        self.addCleanup(d.cleanup)
        (Path(d.name) / "route").write_text(route)
        (Path(d.name) / "dev").write_text(dev)
        return str(Path(d.name) / "dev"), str(Path(d.name) / "route")

    def test_the_default_route_interface_with_the_lowest_metric_is_read(self):
        dev, route = self.files()
        self.assertEqual(payload.net_counters(dev, route), ("eth0", 1000, 2000))

    def test_receive_is_the_first_column_and_transmit_the_ninth(self):
        dev, route = self.files(dev=DEV.replace("1000 10 0 0 0 0 0 0  2000", "5 10 0 0 0 0 0 0  7"))
        self.assertEqual(payload.net_counters(dev, route)[1:], (5, 7))

    def test_no_default_route_is_none(self):
        dev, route = self.files(route=ROUTE.split("eth1")[0])
        self.assertIsNone(payload.net_counters(dev, route))

    def test_unreadable_files_are_none_and_never_an_error(self):
        self.assertIsNone(payload.net_counters("/nonexistent/dev", "/nonexistent/route"))

    def test_garbage_is_none(self):
        dev, route = self.files(dev="x\ny\n eth0: not numbers\n")
        self.assertIsNone(payload.net_counters(dev, route))

    def test_a_stage_records_the_difference(self):
        result = {}
        payload.record_net(result, "provision", ("eth0", 100, 50), ("eth0", 1100, 80))
        self.assertEqual(result["net_bytes"], {"interface": "eth0", "stages": {"provision": {"rx": 1000, "tx": 30}}})

    def test_a_counter_that_went_backwards_is_zero(self):
        result = {}
        payload.record_net(result, "apps", ("eth0", 100, 50), ("eth0", 10, 5))
        self.assertEqual(result["net_bytes"]["stages"]["apps"], {"rx": 0, "tx": 0})

    def test_a_missing_reading_or_a_changed_interface_records_nothing(self):
        for before, after in ((None, ("eth0", 1, 1)), (("eth0", 1, 1), None), (("eth0", 1, 1), ("eth1", 5, 5))):
            result = {}
            payload.record_net(result, "apps", before, after)
            self.assertEqual(result, {})

    def test_a_changed_interface_does_not_overwrite_an_earlier_record(self):
        result = {}
        payload.record_net(result, "provision", ("eth0", 0, 0), ("eth0", 5, 5))
        payload.record_net(result, "apps", ("eth0", 0, 0), ("eth1", 9, 9))
        self.assertEqual(result["net_bytes"], {"interface": "eth0", "stages": {"provision": {"rx": 5, "tx": 5}}})

    def test_every_stage_of_a_run_is_recorded_by_execute_stages(self):
        seen = iter([("eth0", 0, 0), ("eth0", 10, 4), ("eth0", 10, 4), ("eth0", 30, 9)])
        original = payload.net_counters
        payload.net_counters = lambda *a: next(seen)
        self.addCleanup(setattr, payload, "net_counters", original)
        d = tempfile.TemporaryDirectory()
        self.addCleanup(d.cleanup)
        stages = [dict(name=n, command=["true"], output=Path(d.name) / "o", progress=[], grade=lambda rc: [], detail=[]) for n in ("provision", "apps")]
        result = {"verdict": "FAIL", "suites": [], "hardware": {}}
        payload.execute_stages(stages, 2**62, result, Path(d.name) / "result.json", runner=lambda *a: 0)
        self.assertEqual(result["net_bytes"]["stages"], {"provision": {"rx": 10, "tx": 4}, "apps": {"rx": 20, "tx": 5}})


class Interface(unittest.TestCase):
    def test_a_stage_that_changes_the_interface_records_nothing(self):
        seen = iter([("eth0", 0, 0), ("eth1", 10, 4)])
        original = payload.net_counters
        payload.net_counters = lambda *a: next(seen)
        self.addCleanup(setattr, payload, "net_counters", original)
        d = tempfile.TemporaryDirectory()
        self.addCleanup(d.cleanup)
        stages = [dict(name="provision", command=["true"], output=Path(d.name) / "o", progress=[], grade=lambda rc: [], detail=[])]
        result = {"verdict": "FAIL", "suites": [], "hardware": {}}
        payload.execute_stages(stages, 2**62, result, Path(d.name) / "result.json", runner=lambda *a: 0)
        self.assertNotIn("net_bytes", result)


class Report(unittest.TestCase):
    GOOD = {"interface": "eth0", "stages": {"provision": {"rx": 1, "tx": 2}}}

    def test_a_good_record_is_accepted(self):
        report.validate_net_bytes(self.GOOD)

    def test_bad_records_are_refused(self):
        bad = [
            {"interface": "eth0"},
            dict(self.GOOD, extra=1),
            {"interface": "", "stages": {}},
            {"interface": "eth0", "stages": {"Bad Name": {"rx": 1, "tx": 1}}},
            {"interface": "eth0", "stages": {"a": {"rx": -1, "tx": 1}}},
            {"interface": "eth0", "stages": {"a": {"rx": 1.5, "tx": 1}}},
            {"interface": "eth0", "stages": {"a": {"rx": True, "tx": 1}}},
            {"interface": "eth0", "stages": {"a": {"rx": 1}}},
            {"interface": "eth0", "stages": {"a": {"rx": 1 << 51, "tx": 1}}},
        ]
        for value in bad:
            with self.assertRaises(ValueError, msg=str(value)):
                report.validate_net_bytes(value)

    def test_the_result_file_may_carry_it_and_it_is_checked(self):
        d = tempfile.TemporaryDirectory()
        self.addCleanup(d.cleanup)
        path = Path(d.name) / "r.json"
        doc = {"verdict": "FAIL", "suites": [{"name": "provision", "pass": 0, "fail": 1, "skip": 0}], "net_bytes": self.GOOD}
        path.write_text(json.dumps(doc))
        self.assertEqual(report.read_result_file(str(path))["net_bytes"], self.GOOD)
        path.write_text(json.dumps(dict(doc, net_bytes={"interface": "eth0", "stages": {"a": {"rx": -1, "tx": 0}}})))
        with self.assertRaises(report.ResultError):
            report.read_result_file(str(path))


if __name__ == "__main__":
    unittest.main()
