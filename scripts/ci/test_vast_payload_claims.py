#!/usr/bin/env python3
"""Setup-picker host claims on the box side (2026-11-06), GPU-free.

FALSIFIER (written first): a claim carries an identifier (UUID, serial, address, hostname, machine
id); a bad or missing claim fails or changes a result; the reporter sends host_claims to a Worker
that did not ask for it (an older Worker refuses the unknown key); the box and the Worker disagree
on a clean claim; the harness's guest OS and driver constants drift from the scripts that install
those guests.
"""

import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]


def load(name, file):
    spec = importlib.util.spec_from_file_location(name, HERE / file)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


payload = load("claims_payload", "vast_payload.py")
report = load("claims_report", "vast_report.py")

INTEL = """processor\t: 0
vendor_id\t: GenuineIntel
cpu family\t: 6
model\t\t: 106
model name\t: Intel(R) Xeon(R) Gold 6338 CPU @ 2.00GHz
stepping\t: 6
processor\t: 1
vendor_id\t: GenuineIntel
model name\t: Intel(R) Xeon(R) Gold 6338 CPU @ 2.00GHz
"""
AMD = "vendor_id\t: AuthenticAMD\nmodel\t\t: 1\nmodel name\t: AMD EPYC 7763 64-Core Processor\n"


class Cpu(unittest.TestCase):
    def claims(self, text, machine="x86_64"):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "cpuinfo"
            if isinstance(text, bytes):
                path.write_bytes(text)
            else:
                path.write_text(text)
            return payload.cpu_claims(str(path), machine)

    def test_intel_and_amd(self):
        self.assertEqual(
            self.claims(INTEL),
            {"cpu_vendor": "intel", "cpu_arch": "x86_64",
             "cpu_model": "Intel(R) Xeon(R) Gold 6338 CPU @ 2.00GHz"},
        )
        self.assertEqual(
            self.claims(AMD),
            {"cpu_vendor": "amd", "cpu_arch": "x86_64", "cpu_model": "AMD EPYC 7763 64-Core Processor"},
        )

    def test_the_numeric_model_line_is_not_the_model_name(self):
        self.assertEqual(self.claims("model\t: 106\nvendor_id: GenuineIntel\n")["cpu_model"], None)

    def test_other_vendor_and_arm(self):
        self.assertEqual(self.claims("vendor_id: HygonGenuine\nmodel name: Hygon C86 7285\n")["cpu_vendor"], "other")
        arm = self.claims("processor : 0\nCPU implementer : 0x41\n", machine="aarch64")
        self.assertEqual((arm["cpu_vendor"], arm["cpu_arch"], arm["cpu_model"]), ("arm", "aarch64", None))
        self.assertEqual(self.claims("", machine="arm64")["cpu_arch"], "aarch64")
        self.assertEqual(self.claims("", machine="AMD64")["cpu_arch"], "x86_64")

    def test_missing_or_hostile_cpuinfo_is_empty_not_an_error(self):
        empty = {"cpu_vendor": None, "cpu_model": None, "cpu_arch": "x86_64"}
        self.assertEqual(payload.cpu_claims("/nonexistent/cpuinfo", "x86_64"), empty)
        self.assertEqual(self.claims(b"\xff\xfe\x00 model name\t: \xff\n")["cpu_vendor"], None)
        self.assertEqual(self.claims("")["cpu_model"], None)

    def test_the_model_is_sanitised_and_bounded(self):
        got = self.claims("vendor_id: GenuineIntel\nmodel name: Xeon;\x07 `rm -rf /` $(id)  " + "x" * 200 + "\n")
        self.assertLessEqual(len(got["cpu_model"]), 64)
        self.assertNotRegex(got["cpu_model"], r"[;`$\x07]")
        self.assertNotIn("  ", got["cpu_model"])


class Claims(unittest.TestCase):
    def build(self, windows, cpuinfo=INTEL, receipt=None):
        with tempfile.TemporaryDirectory() as d:
            bench = Path(d)
            (bench / "cpuinfo").write_text(cpuinfo)
            if receipt is not None:
                (bench / "guest.qcow2.STAGED").write_text(receipt)
            return payload.host_claims(windows, bench=bench, cpuinfo=str(bench / "cpuinfo"), machine="x86_64")

    def test_linux_only_run_has_no_windows_claim(self):
        got = self.build(False, receipt="version=580.159.04\nimage=/workspace/bench/guest.qcow2\nstaged=2026-11-06\n")
        self.assertEqual(
            got,
            {"cpu_vendor": "intel", "cpu_model": "Intel(R) Xeon(R) Gold 6338 CPU @ 2.00GHz",
             "cpu_arch": "x86_64", "linux_os_version": "ubuntu-24.04", "linux_driver": "580.159.04"},
        )

    def test_windows_run_adds_the_windows_guest(self):
        got = self.build(True, receipt="version=580.159.04\n")
        self.assertEqual((got["windows_os_version"], got["windows_driver"]), ("11-ltsc-2024-26100", "580.88"))

    def test_a_missing_or_bad_receipt_leaves_the_driver_out(self):
        for receipt in (None, "", "version=\n", "version=unknown\n", "version=58.1\n", "image=x\n",
                        "version=580.159.04.9.9\n"):
            with self.subTest(receipt=receipt):
                self.assertNotIn("linux_driver", self.build(False, receipt=receipt))

    def test_only_claim_keys_and_no_host_driver_or_identifier(self):
        got = self.build(True, receipt="version=580.159.04\n")
        self.assertLessEqual(set(got), set(report.CLAIM_KEYS))
        # host_driver and gpu_name travel in `hardware`, not here.
        self.assertNotIn("host_driver", got)
        self.assertNotRegex(json.dumps(got), r"(?i)serial|uuid|machine|hostname|[0-9a-f]{12}")

    def test_a_model_with_stray_characters_is_cleaned_not_lost(self):
        got = self.build(False, cpuinfo="vendor_id: GenuineIntel\nmodel name: Intel(R) Xeon(R) CPU\x00 @ 2.00GHz;\n")
        self.assertEqual(got["cpu_model"], "Intel(R) Xeon(R) CPU @ 2.00GHz")

    def test_a_uuid_or_serial_in_the_model_is_dropped(self):
        got = self.build(False, cpuinfo="vendor_id: GenuineIntel\nmodel name: Xeon 0123456789abcdef0123\n")
        self.assertNotIn("cpu_model", got)
        self.assertEqual(got["cpu_vendor"], "intel")

    def test_refresh_survives_file_errors(self):
        result = {"verdict": "PASS"}
        for error in (OSError("x"), ValueError("x")):
            with mock.patch.object(payload, "host_claims", side_effect=error):
                payload.refresh_claims(result, True)
            self.assertEqual(result, {"verdict": "PASS"})

    def test_refresh_survives_decoding_and_evidence_errors(self):
        result = {"verdict": "PASS"}
        for error in (UnicodeError("x"), payload.EvidenceError("infrastructure")):
            with mock.patch.object(payload, "host_claims", side_effect=error):
                payload.refresh_claims(result, True)
            self.assertEqual(result, {"verdict": "PASS"})

    def test_refresh_records_a_block_and_skips_an_empty_one(self):
        result = {"verdict": "PASS"}
        with mock.patch.object(payload, "host_claims", return_value={}):
            payload.refresh_claims(result, True)
        self.assertEqual(result, {"verdict": "PASS"}, "an empty block is not recorded")
        with mock.patch.object(payload, "host_claims", return_value={"cpu_vendor": "amd"}):
            payload.refresh_claims(result, True)
        self.assertEqual(result["host_claims"], {"cpu_vendor": "amd"})

    def test_guest_constants_match_the_scripts_that_install_the_guests(self):
        bench = (ROOT / "scripts/bench/provision_bench_tree.sh").read_text()
        self.assertIn("noble-server-cloudimg-amd64.img", bench)
        self.assertEqual(payload.LINUX_GUEST_OS, "ubuntu-24.04")
        win_vm = (ROOT / "scripts/bench/windows/win_vm.sh").read_text()
        gate = (ROOT / "scripts/bench/box/windows_gate.sh").read_text()
        self.assertIn("win11_ltsc_2024_eval_x64", win_vm)
        self.assertIn("580.88-desktop-win10-win11-64bit", win_vm)
        self.assertIn("Windows 11 Enterprise LTSC 2024", gate)
        self.assertIn("build 26100", gate)
        self.assertRegex(gate, r"Driver Version \+: 580\\\.88")
        self.assertEqual((payload.WINDOWS_GUEST_OS, payload.WINDOWS_GUEST_DRIVER), ("11-ltsc-2024-26100", "580.88"))
        # The fat guest's receipt is the one provision_bench_tree.sh writes.
        self.assertIn('guest.qcow2.STAGED', bench)
        self.assertIn('echo "version=$GUEST_VERSION"', bench)


class SharedVectors(unittest.TestCase):
    def test_clean_claims_agrees_with_the_shared_vectors(self):
        vectors = json.loads((HERE / "host_claims_vectors.json").read_text())
        self.assertGreaterEqual(len(vectors["cases"]), 25)
        for case in vectors["cases"]:
            want = {k: None for k in report.CLAIM_KEYS}
            want.update(case["expect"])
            with self.subTest(case["name"]):
                self.assertEqual(report.clean_claims(case["input"]), want)

    def test_clean_claims_is_total(self):
        for hostile in (None, 0, "", [], {}, {"cpu_model": object()}, {"cpu_vendor": ["amd"]}):
            report.clean_claims(hostile)


class Reporter(unittest.TestCase):
    ENV = {
        "KF_NONCE": "ab" * 16,
        "KF_SHA": "1234abcd" * 5,
        "KF_RUNNER_NONCE": "cd" * 32,
    }
    HW = {"gpu_name": "NVIDIA GeForce RTX 3060", "compute_cap": 860, "driver_version": "580.159.04"}

    def final(self, doc, env=None):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "r.json"
            path.write_text(json.dumps(doc))
            return report.final_result({**self.ENV, **(env or {})}, 0, False, None, str(path))

    def doc(self, **extra):
        return {"verdict": "PASS", "suites": [{"name": "gates", "pass": 1, "fail": 0, "skip": 0}],
                "hardware": self.HW, **extra}

    def test_claims_are_sent_only_to_a_worker_that_asked(self):
        claims = {"cpu_vendor": "amd", "cpu_arch": "x86_64", "linux_os_version": "ubuntu-24.04"}
        asked = self.final(self.doc(host_claims=claims), {"KF_HOST_CLAIMS": "1"})
        self.assertEqual(asked["host_claims"], claims)
        for env in ({}, {"KF_HOST_CLAIMS": "0"}, {"KF_HOST_CLAIMS": ""}, {"KF_HOST_CLAIMS": "true"}):
            with self.subTest(env=env):
                self.assertNotIn("host_claims", self.final(self.doc(host_claims=claims), env))

    def test_without_the_request_the_result_has_exactly_the_old_keys(self):
        # An older Worker refuses any other top-level key of a result.
        claims = {"cpu_vendor": "amd", "cpu_arch": "x86_64"}
        self.assertEqual(
            set(self.final(self.doc(host_claims=claims))),
            {"nonce", "sha", "runner_nonce", "verdict", "suites", "hardware"},
        )
        self.assertEqual(
            set(self.final(self.doc(host_claims=claims, host_reference="GREEN"))),
            {"nonce", "sha", "runner_nonce", "verdict", "suites", "hardware", "host_reference"},
        )

    def test_claims_are_cleaned(self):
        env = {"KF_HOST_CLAIMS": "1"}
        got = self.final(self.doc(host_claims={"cpu_vendor": "amd", "cpu_model": "root@box.example",
                                                "machine_id": "9", "linux_driver": "x"}), env)
        self.assertEqual(got["host_claims"], {"cpu_vendor": "amd"})

    def test_a_bad_block_is_dropped_not_an_error(self):
        env = {"KF_HOST_CLAIMS": "1"}
        for bad in ("amd", ["amd"], 7, None, {}, {"cpu_vendor": "via"}):
            with self.subTest(bad=bad):
                result = self.final(self.doc(host_claims=bad), env)
                self.assertNotIn("host_claims", result)
                self.assertEqual(result["verdict"], "PASS")

    def test_a_claim_never_changes_the_verdict(self):
        env = {"KF_HOST_CLAIMS": "1"}
        unknown_hw = {"gpu_name": "unknown", "compute_cap": 0, "driver_version": "unknown"}
        got = self.final(self.doc(hardware=unknown_hw, host_claims={"cpu_vendor": "amd"}), env)
        self.assertEqual(got["verdict"], "FAIL", "the old PASS conditions are untouched")

    def test_the_env_contract_accepts_only_1(self):
        base = {
            "KF_PROTOCOL": "7", "KF_BROKER_URL": "https://h/p", "KF_RUN_ID": "4242-7",
            "KF_RUN_TOKEN": ".".join(["eyJhbGciOiJIUzI1NiJ9", "eyJydW4iOiI0MjQyIn0", "A" * 43]),
            "KF_NONCE": "ab" * 16, "KF_SHA": "1234abcd" * 5, "KF_REPO": "a/b", "KF_RUNNER_NONCE": "cd" * 32,
            "KF_SOFT_DEADLINE": str(2_000_000_000), "KF_HEARTBEAT_SECONDS": "30", "KF_GRACE_FAIL": "600",
        }
        self.assertEqual(report.validate_env(base), [])
        self.assertEqual(report.validate_env({**base, "KF_HOST_CLAIMS": "1"}), [])
        for bad in ("0", "2", "yes", "11", " 1"):
            self.assertEqual(report.validate_env({**base, "KF_HOST_CLAIMS": bad}), ["KF_HOST_CLAIMS: malformed"], bad)

    def test_the_variable_reaches_the_reporter_from_the_provider_environment(self):
        with tempfile.TemporaryDirectory() as d:
            env_file = Path(d) / "environment"
            env_file.write_text("KF_HOST_CLAIMS=1\nKF_NONCE=ignored\n")
            got = report.load_provider_env({"KF_NONCE": "ab" * 16}, str(env_file))
            self.assertEqual(got["KF_HOST_CLAIMS"], "1")
            out = Path(d) / "run" / "env"
            report.write_env_file(str(out), {"KF_HOST_CLAIMS": "1", "KF_NONCE": "ab" * 16})
            self.assertIn("export KF_HOST_CLAIMS=1", out.read_text())

    def test_the_result_file_may_carry_claims(self):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "r.json"
            path.write_text(json.dumps(self.doc(host_claims={"cpu_vendor": "amd"})))
            self.assertEqual(report.read_result_file(str(path))["host_claims"], {"cpu_vendor": "amd"})
            path.write_text(json.dumps(self.doc(host_claim={"cpu_vendor": "amd"})))
            with self.assertRaises(report.ResultError):
                report.read_result_file(str(path))


if __name__ == "__main__":
    unittest.main()
