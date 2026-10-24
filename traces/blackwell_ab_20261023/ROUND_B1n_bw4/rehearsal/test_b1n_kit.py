"""BW4 round B1n run kit: arm plan, per-die authority, capture and collection boots (tests first).

Owner 2026-10-24: (1) per-die pinned Xid32 pairs under the unchanged exact rules; (2) every arm
copies guest dumps, the full event log and the full kf3/QEMU log off the box before teardown,
and a fatal stop gets one bounded standard-VGA collection boot with no kf3 device and no GPU work.
"""

import copy
import datetime as dt
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
K = Path(os.environ.get("KF_BW4_K", HERE.parent / "attempt1" / "run_kit"))
BW = Path(os.environ.get("KF_BW4_BW", HERE.parents[3] / "scripts/bench/blackwell"))
sys.path.insert(0, str(BW))
sys.path.insert(0, str(K))


def load(name, path):
    if not Path(path).exists():
        return None
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


A = load("b1n_arms", K / "b1n_arms.py")
AUTH = load("b1n_authority", K / "b1n_authority.py")
PACK = load("capture_pack", K / "capture_pack.py")
PULL = load("capture_pull", K / "capture_pull.py")
FATAL = load("fatal_collect", K / "fatal_collect.py")
O = load("saved_xid32_observation_b1n", BW / "saved_xid32_observation.py")
NOW = dt.datetime(2026, 10, 24, 12, 0, tzinfo=dt.timezone.utc)
START = NOW - dt.timedelta(minutes=30)
END = START + dt.timedelta(minutes=150)


def text(name):
    return (K / name).read_text()


class Present(unittest.TestCase):
    def test_new_kit_modules_exist(self):
        for module, name in [
            (A, "b1n_arms.py"),
            (AUTH, "b1n_authority.py"),
            (PACK, "capture_pack.py"),
            (PULL, "capture_pull.py"),
            (FATAL, "fatal_collect.py"),
        ]:
            self.assertIsNotNone(module, name)

    def test_new_kit_scripts_exist(self):
        for name in [
            "enable580_cycle.ps1",
            "prep_tdr60.ps1",
            "capture_guest.ps1",
            "prep_boot.sh",
            "linux_reload.sh",
            "reload_hook.sh",
            "fatal_collect.sh",
        ]:
            self.assertTrue((K / name).is_file(), name)


class ArmPlan(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(A, "b1n_arms.py missing")

    def test_priority_order_is_the_owner_order(self):
        runs = [arm["run"] for arm in A.ARMS]
        self.assertEqual(runs, ["on_urt", "on_relay_off", "on_tdr60", "lx_reload", "on_diag_pnp"])
        self.assertTrue(A.validate())

    def test_each_arm_changes_exactly_the_named_thing(self):
        arms = {arm["run"]: arm for arm in A.ARMS}
        self.assertEqual(
            arms["on_urt"]["env"], {"KF3_USER_RING_TRACE": "/root/prov/arms/on_urt/urt"}
        )
        self.assertEqual(arms["on_urt"]["label"], "MEASUREMENT")
        self.assertEqual(arms["on_relay_off"]["env"], {"KF3_WIN_USER_RING_RELAY": "0"})
        self.assertEqual(arms["on_tdr60"]["prep"], "prep_tdr60.ps1")
        self.assertEqual(arms["on_tdr60"]["env"], {})
        self.assertEqual(arms["on_diag_pnp"]["enable"], "enable580_cycle.ps1")
        self.assertEqual(arms["on_diag_pnp"]["enable_count"], 2)
        self.assertEqual(arms["lx_reload"]["kind"], "linux")
        for run in ("on_relay_off", "on_tdr60", "lx_reload", "on_diag_pnp"):
            self.assertEqual(arms[run]["label"], "DIAGNOSTIC", run)

    def bad(self, **change):
        arms = copy.deepcopy(list(A.ARMS))
        index = change.pop("index", 0)
        arms[index].update(change)
        with self.assertRaises(ValueError):
            A.validate(arms)

    def test_unknown_flag_refuses(self):
        self.bad(env={"KF3_USER_RING_TRACE": "/root/prov/arms/on_urt/urt", "KF3_X": "1"})

    def test_trace_dir_must_be_the_arm_dir(self):
        self.bad(env={"KF3_USER_RING_TRACE": "/tmp/urt"})

    def test_relay_flag_only_zero(self):
        self.bad(index=1, env={"KF3_WIN_USER_RING_RELAY": "1"})

    def test_two_changes_in_one_arm_refuse(self):
        self.bad(index=2, env={"KF3_WIN_USER_RING_RELAY": "0"})

    def test_second_enable_only_with_cycle_script_and_diagnostic(self):
        self.bad(index=0, enable_count=2)
        self.bad(index=4, label="MEASUREMENT")
        self.bad(index=4, enable="enable580_once.ps1")

    def test_prep_is_diagnostic_and_pinned(self):
        self.bad(index=2, label="MEASUREMENT")
        self.bad(index=2, prep="other.ps1")

    def test_linux_arm_has_no_permit_shape(self):
        self.bad(index=3, run="on_lx")
        self.bad(index=3, enable="enable580_once.ps1")
        self.bad(index=3, env={"KF3_WIN_USER_RING_RELAY": "0"})

    def test_repeated_name_refuses(self):
        self.bad(index=1, run="on_urt")

    def test_trace_dir_of_another_arm_refuses(self):
        self.bad(env={"KF3_USER_RING_TRACE": "/root/prov/arms/on_relay_off/urt"})

    def test_relay_empty_value_refuses(self):
        self.bad(index=1, env={"KF3_WIN_USER_RING_RELAY": ""})

    def test_cycle_with_an_extra_flag_refuses(self):
        self.bad(index=4, env={"KF3_WIN_USER_RING_RELAY": "0"})

    def test_flag_arm_with_a_prep_refuses(self):
        self.bad(index=1, prep="prep_tdr60.ps1")

    def test_measurement_label_cannot_carry_the_cycle(self):
        self.bad(index=1, env={}, enable="enable580_cycle.ps1", enable_count=2, label="MEASUREMENT")

    def test_linux_kind_with_windows_name_refuses(self):
        self.bad(index=3, run="on_linux")

    def test_probed_die_mismatch_refuses(self):
        with self.assertRaises(ValueError):
            A.probed_die("PROBE_DIE arch=0x1b0 impl=0x2 die=GB202\n", "GB205")

    def test_two_probe_lines_refuse(self):
        line = "PROBE_DIE arch=0x1b0 impl=0x5 die=GB205\n"
        with self.assertRaises(ValueError):
            A.probed_die(line + line, "GB205")

    def test_prepare_writes_env_and_config(self):
        with tempfile.TemporaryDirectory() as tmp:
            line = A.prepare("on_urt", prov=tmp)
            self.assertEqual(line, "enable580_once.ps1 1 - MEASUREMENT")
            env = (Path(tmp) / "arm_env_on_urt.sh").read_text()
            self.assertEqual(env, "export KF3_USER_RING_TRACE=/root/prov/arms/on_urt/urt\n")
            self.assertEqual(
                A.prepare("on_diag_pnp", prov=tmp), "enable580_cycle.ps1 2 - DIAGNOSTIC"
            )
            self.assertEqual((Path(tmp) / "arm_env_on_diag_pnp.sh").read_text(), "")
            self.assertEqual(
                A.prepare("on_tdr60", prov=tmp), "enable580_once.ps1 1 prep_tdr60.ps1 DIAGNOSTIC"
            )

    def test_prepare_refuses_an_arm_outside_the_plan(self):
        with tempfile.TemporaryDirectory() as tmp, self.assertRaises(ValueError):
            A.prepare("on_observe_b", prov=tmp)

    def test_probed_die_parses_exactly_one_matching_line(self):
        log = "noise\nPROBE_DIE arch=0x1b0 impl=0x5 die=GB205\nmore\n"
        self.assertEqual(A.probed_die(log, "GB205"), "GB205")
        with self.assertRaises(ValueError):
            A.probed_die(log, "GB202")
        with self.assertRaises(ValueError):
            A.probed_die(log + "PROBE_DIE arch=0x1b0 impl=0x2 die=GB202\n", "GB205")
        with self.assertRaises(ValueError):
            A.probed_die("PROBE_DIE arch=0x1b0 impl=0x5 die=GB205 extra\n", "GB205")
        with self.assertRaises(ValueError):
            A.probed_die("", "GB205")

    def test_permit_fields_carry_die_count_and_label(self):
        with tempfile.TemporaryDirectory() as tmp:
            probed = Path(tmp) / "probed_die.json"
            probed.write_text(json.dumps({"die": "GB205"}))
            self.assertEqual(
                A.permit_fields("on_diag_pnp", probed),
                {"probed_die": "GB205", "enable_count": 2, "label": "DIAGNOSTIC"},
            )
            with self.assertRaises(ValueError):
                A.permit_fields("lx_reload", probed)


class Authority(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(AUTH, "b1n_authority.py missing")
        self.assertIsNotNone(O)

    def build(self, die="GB205"):
        return AUTH.build(lease_id=57500001, die=die, start=START, end=END, table=O.DIE_PAIRS)

    def test_gb205_authority_uses_the_table_pair_and_windows_arms(self):
        a = self.build()
        self.assertEqual(a["exact_channels"], ["0x14", "0x1000000b"])
        self.assertEqual(a["candidate_die"], "GB205")
        self.assertEqual(a["arms"], ["on_urt", "on_relay_off", "on_tdr60", "on_diag_pnp"])
        self.assertEqual(a["diagnostic_enable_arms"], {"on_diag_pnp": 2})
        self.assertEqual(a["mode"], O.MODE)
        self.assertIs(a["observation_only"], True)
        self.assertIs(a["attestation_allowed"], False)
        self.assertEqual(a["record_limit"], 8)

    def test_gb202_authority_uses_its_own_pair(self):
        self.assertEqual(self.build("GB202")["exact_channels"], ["0x14", "0x0e00000b"])

    def test_die_outside_the_table_refuses_before_any_launch(self):
        for die in ("GB206", "GB203", "GB207"):
            with self.subTest(die=die), self.assertRaises(ValueError):
                self.build(die)

    def test_every_windows_arm_permit_is_accepted_by_the_real_guard(self):
        a = self.build()
        with tempfile.TemporaryDirectory() as tmp:
            probed = Path(tmp) / "probed_die.json"
            probed.write_text(json.dumps({"die": "GB205"}))
            for run in a["arms"]:
                phase = {
                    "arm": run,
                    "pid": 4242,
                    "starttime": 99,
                    "exe": "/x/qemu-system-x86_64",
                    "pci": "0000:00:07",
                }
                permit = {**phase, "mode": O.MODE, "profile": "on", "lease_id": 57500001}
                permit.update(armed_utc=(NOW - dt.timedelta(minutes=1)).isoformat())
                permit.update(A.permit_fields(run, probed))
                self.assertTrue(O.scope_enabled(phase, permit, a, NOW), run)
                line = (
                    "2026-10-24T11:59:30.000000+0000 box kernel: NVRM: Xid (PCI:0000:00:07): 32, "
                    "pid=4242, name=qemu-system-x86, channel 0x1000000b intr 00200000"
                )
                rec = O.judge_observation([line], phase=phase, permit=permit, authority=a, now=NOW)
                self.assertEqual(rec[0]["verdict"], "OBSERVATION")

    def test_linux_arm_is_never_an_observation_arm(self):
        self.assertNotIn("lx_reload", self.build()["arms"])


class CapturePack(unittest.TestCase):
    """Box side: the per-arm private capture directory and its sha256 manifest."""

    def setUp(self):
        self.assertIsNotNone(PACK, "capture_pack.py missing")

    def tree(self, root):
        root = Path(root)
        guest = root / "cap" / "guest"
        (guest / "LiveKernelReports" / "WATCHDOG").mkdir(parents=True)
        (guest / "Minidump").mkdir()
        (guest / "Minidump" / "a.dmp").write_bytes(b"PAGEDU64" + b"\0" * 100)
        (guest / "LiveKernelReports" / "WATCHDOG" / "w.dmp").write_bytes(b"x" * 50)
        (guest / "evtx").mkdir()
        (guest / "evtx" / "System.evtx").write_bytes(b"ElfFile\0")
        rows = []
        for p in sorted(guest.rglob("*")):
            if p.is_file():
                digest = hashlib.sha256(p.read_bytes()).hexdigest().upper()
                rows.append(f"GUESTFILE {digest} {p.stat().st_size} {p.relative_to(guest)}")
        (guest / "GUEST_MANIFEST.txt").write_text("\n".join(rows) + "\nCAPTURE_GUEST_DONE\n")
        logs = root / "logs"
        logs.mkdir()
        (logs / "on_urt_b1_qemu.log").write_text("kf3: one\n")
        (logs / "on_urt_collect_b1_qemu.log").write_text("collect\n")
        (logs / "on_relay_off_b1_qemu.log").write_text("other arm\n")
        prov = root / "prov"
        (prov / "arms" / "on_urt" / "urt").mkdir(parents=True)
        (prov / "arms" / "on_urt" / "urt" / "user-ring-refusal-1-2-3.txt").write_text("t\n")
        (prov / "arm_on_urt.log").write_text("arm\n")
        (prov / "collect_on_urt.log").write_text("collect\n")
        return root

    def test_manifest_lists_every_file_with_sha256_and_verifies_guest_hashes(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self.tree(tmp)
            out = root / "capture"
            summary = PACK.pack(
                "on_urt",
                guest_dir=root / "cap" / "guest",
                vm_logs=root / "logs",
                prov=root / "prov",
                out=out,
            )
            manifest = (out / "on_urt" / "MANIFEST.sha256").read_text().splitlines()
            names = {line.split("  ", 1)[1] for line in manifest}
            self.assertIn("guest/Minidump/a.dmp", names)
            self.assertIn("guest/LiveKernelReports/WATCHDOG/w.dmp", names)
            self.assertIn("vm_logs/on_urt_b1_qemu.log", names)
            self.assertIn("vm_logs/on_urt_collect_b1_qemu.log", names)
            self.assertNotIn("vm_logs/on_relay_off_b1_qemu.log", names)
            self.assertIn("prov/arms/on_urt/urt/user-ring-refusal-1-2-3.txt", names)
            self.assertIn("prov/arm_on_urt.log", names)
            self.assertEqual(summary["guest_hash_mismatches"], [])
            self.assertTrue(summary["guest_manifest_complete"])
            for line in manifest:
                digest, name = line.split("  ", 1)
                data = (out / "on_urt" / name).read_bytes()
                self.assertEqual(hashlib.sha256(data).hexdigest(), digest)

    def test_manifest_is_ordered_smallest_first(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self.tree(tmp)
            PACK.pack(
                "on_urt",
                guest_dir=root / "cap" / "guest",
                vm_logs=root / "logs",
                prov=root / "prov",
                out=root / "capture",
            )
            arm = root / "capture" / "on_urt"
            names = [
                l.split("  ", 1)[1] for l in (arm / "MANIFEST.sha256").read_text().splitlines()
            ]
            sizes = [(arm / n).stat().st_size for n in names]
            self.assertEqual(sizes, sorted(sizes))

    def test_guest_hash_mismatch_is_recorded_not_hidden(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self.tree(tmp)
            (root / "cap" / "guest" / "Minidump" / "a.dmp").write_bytes(b"changed")
            summary = PACK.pack(
                "on_urt",
                guest_dir=root / "cap" / "guest",
                vm_logs=root / "logs",
                prov=root / "prov",
                out=root / "capture",
            )
            self.assertEqual(summary["guest_hash_mismatches"], ["Minidump/a.dmp"])

    def test_nested_watchdog_mismatch_is_recorded(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self.tree(tmp)
            (root / "cap" / "guest" / "LiveKernelReports" / "WATCHDOG" / "w.dmp").write_bytes(b"y")
            summary = PACK.pack(
                "on_urt",
                guest_dir=root / "cap" / "guest",
                vm_logs=root / "logs",
                prov=root / "prov",
                out=root / "capture",
            )
            self.assertEqual(summary["guest_hash_mismatches"], ["LiveKernelReports/WATCHDOG/w.dmp"])

    def test_another_arm_takes_only_its_own_logs(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self.tree(tmp)
            summary = PACK.pack(
                "on_relay_off",
                guest_dir=root / "absent",
                vm_logs=root / "logs",
                prov=root / "prov",
                out=root / "capture",
            )
            logs = sorted(n for n in summary["files"] if n.startswith("vm_logs/"))
            self.assertEqual(logs, ["vm_logs/on_relay_off_b1_qemu.log"])

    def test_missing_guest_capture_still_packs_logs(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self.tree(tmp)
            summary = PACK.pack(
                "on_urt",
                guest_dir=root / "absent",
                vm_logs=root / "logs",
                prov=root / "prov",
                out=root / "capture",
            )
            self.assertFalse(summary["guest_manifest_complete"])
            self.assertIn("vm_logs/on_urt_b1_qemu.log", summary["files"])

    def test_public_summary_has_no_content_only_names_sizes_hashes(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self.tree(tmp)
            summary = PACK.pack(
                "on_urt",
                guest_dir=root / "cap" / "guest",
                vm_logs=root / "logs",
                prov=root / "prov",
                out=root / "capture",
            )
            public = PACK.public_summary(summary)
            blob = json.dumps(public)
            self.assertNotIn("kf3: one", blob)
            self.assertEqual(public["private"], True)
            for row in public["files"].values():
                self.assertEqual(set(row), {"bytes", "sha256"})

    def test_run_name_is_validated(self):
        with tempfile.TemporaryDirectory() as tmp, self.assertRaises(ValueError):
            PACK.pack("../x", guest_dir=tmp, vm_logs=tmp, prov=tmp, out=Path(tmp) / "c")


class CapturePull(unittest.TestCase):
    """Operator side: copy the box capture to the private dev-box directory, verify sha256."""

    def setUp(self):
        self.assertIsNotNone(PULL, "capture_pull.py missing")

    def remote_tree(self, root):
        arm = Path(root) / "remote" / "on_urt"
        (arm / "guest").mkdir(parents=True)
        (arm / "guest" / "d.dmp").write_bytes(b"dump")
        (arm / "vm_logs").mkdir()
        (arm / "vm_logs" / "q.log").write_bytes(b"log")
        lines = []
        for p in sorted(arm.rglob("*")):
            if p.is_file():
                lines.append(
                    hashlib.sha256(p.read_bytes()).hexdigest() + "  " + str(p.relative_to(arm))
                )
        (arm / "MANIFEST.sha256").write_text("\n".join(lines) + "\n")
        return arm

    def fetch_local(self, base):
        def fetch(remote_name, dest, timeout):
            data = (base / remote_name).read_bytes()
            Path(dest).parent.mkdir(parents=True, exist_ok=True)
            Path(dest).write_bytes(data)

        return fetch

    def test_pull_verifies_every_file_and_writes_a_content_free_receipt(self):
        with tempfile.TemporaryDirectory() as tmp:
            arm = self.remote_tree(tmp)
            dest = Path(tmp) / "private"
            manifest = (arm / "MANIFEST.sha256").read_text()
            receipt = PULL.pull_arm("on_urt", manifest, dest, self.fetch_local(arm), deadline_s=60)
            self.assertEqual(receipt["verified"], 2)
            self.assertEqual(receipt["failed"], [])
            self.assertEqual((dest / "on_urt" / "guest" / "d.dmp").read_bytes(), b"dump")
            self.assertNotIn("dump", json.dumps(receipt["files"]).replace("d.dmp", ""))

    def stream_local(self, base):
        def stream(names, dest, timeout):
            Path(dest).mkdir(parents=True, exist_ok=True)
            pack = subprocess.Popen(
                ["tar", "-C", str(base), "-cf", "-", "-T", "-"],
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
            )
            unpack = subprocess.Popen(["tar", "-C", str(dest), "-xf", "-"], stdin=pack.stdout)
            pack.stdin.write("".join(n + "\n" for n in names).encode())
            pack.stdin.close()
            unpack.wait(timeout=timeout)
            pack.wait(timeout=timeout)

        return stream

    def test_stream_pull_verifies_every_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            arm = self.remote_tree(tmp)
            manifest = (arm / "MANIFEST.sha256").read_text()
            dest = Path(tmp) / "private"
            receipt = PULL.pull_arm_stream(
                "on_urt", manifest, dest, self.stream_local(arm), self.fetch_local(arm), 60
            )
            self.assertEqual((receipt["verified"], receipt["failed"]), (2, []))
            self.assertEqual((dest / "on_urt" / "vm_logs" / "q.log").read_bytes(), b"log")

    def test_stream_pull_falls_back_per_file_for_a_missing_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            arm = self.remote_tree(tmp)
            manifest = (arm / "MANIFEST.sha256").read_text()

            def partial(names, dest, timeout):
                self.stream_local(arm)(names[:1], dest, timeout)

            receipt = PULL.pull_arm_stream(
                "on_urt", manifest, Path(tmp) / "p", partial, self.fetch_local(arm), 60
            )
            self.assertEqual(receipt["verified"], 2)
            self.assertEqual(receipt["stream_missing_or_bad"], 1)

    def test_stream_pull_reports_a_tampered_file_after_fallback(self):
        with tempfile.TemporaryDirectory() as tmp:
            arm = self.remote_tree(tmp)
            manifest = (arm / "MANIFEST.sha256").read_text()
            (arm / "guest" / "d.dmp").write_bytes(b"tampered")
            receipt = PULL.pull_arm_stream(
                "on_urt",
                manifest,
                Path(tmp) / "p",
                self.stream_local(arm),
                self.fetch_local(arm),
                60,
            )
            self.assertEqual(receipt["failed"], ["guest/d.dmp"])

    def test_hash_mismatch_is_a_failed_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            arm = self.remote_tree(tmp)
            manifest = (arm / "MANIFEST.sha256").read_text()
            (arm / "guest" / "d.dmp").write_bytes(b"tampered")
            receipt = PULL.pull_arm(
                "on_urt", manifest, Path(tmp) / "p", self.fetch_local(arm), deadline_s=60
            )
            self.assertEqual(receipt["failed"], ["guest/d.dmp"])
            self.assertEqual(receipt["verified"], 1)

    def test_changed_file_content_is_a_failed_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            arm = self.remote_tree(tmp)
            manifest = (arm / "MANIFEST.sha256").read_text()
            (arm / "vm_logs" / "q.log").write_bytes(b"lo")
            receipt = PULL.pull_arm(
                "on_urt", manifest, Path(tmp) / "p", self.fetch_local(arm), deadline_s=60
            )
            self.assertEqual(receipt["failed"], ["vm_logs/q.log"])

    def test_deadline_reached_midway_keeps_the_rest_unpulled(self):
        ticks = iter([0, 0, 100, 100, 100, 100])
        with tempfile.TemporaryDirectory() as tmp:
            arm = self.remote_tree(tmp)
            manifest = (arm / "MANIFEST.sha256").read_text()
            receipt = PULL.pull_arm(
                "on_urt",
                manifest,
                Path(tmp) / "p",
                self.fetch_local(arm),
                deadline_s=50,
                clock=lambda: next(ticks),
            )
            self.assertEqual(receipt["verified"], 1)
            self.assertEqual(len(receipt["not_pulled"]), 1)

    def test_manifest_paths_cannot_escape_the_private_dir(self):
        for bad in ("../x", "/etc/passwd", "a/../../b"):
            manifest = "0" * 64 + "  " + bad + "\n"
            with self.subTest(bad=bad), tempfile.TemporaryDirectory() as tmp:
                with self.assertRaises(ValueError):
                    PULL.pull_arm("on_urt", manifest, Path(tmp), lambda *a, **k: None, 60)

    def test_private_dir_is_never_inside_the_repo(self):
        with self.assertRaises(ValueError):
            PULL.private_root(Path("/workspace/bw4-wt/traces/x"), lease=1)
        self.assertEqual(
            PULL.private_root(Path("/data/bw4_private_capture"), lease=57500001),
            Path("/data/bw4_private_capture/57500001"),
        )

    def test_worktree_or_traces_dirs_under_data_are_refused(self):
        for bad in ("/data/x-wt/cap", "/data/repo/traces/cap"):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                PULL.private_root(Path(bad), lease=1)

    def test_deadline_stops_further_fetches_and_records_them(self):
        with tempfile.TemporaryDirectory() as tmp:
            arm = self.remote_tree(tmp)
            manifest = (arm / "MANIFEST.sha256").read_text()
            receipt = PULL.pull_arm(
                "on_urt", manifest, Path(tmp) / "p", self.fetch_local(arm), deadline_s=0
            )
            self.assertEqual(receipt["verified"], 0)
            self.assertEqual(len(receipt["not_pulled"]), 2)


class FatalCollect(unittest.TestCase):
    """After a fatal stop: one bounded standard-VGA collection boot, never a kf3 device."""

    def setUp(self):
        self.assertIsNotNone(FATAL, "fatal_collect.py missing")

    def prov(self, tmp, *, fatal=True, phase=("on_relay_off", 4242), started=("on_urt",)):
        prov = Path(tmp)
        if fatal:
            (prov / "XID_STOP").write_text(json.dumps({"STOP": "x", "phase": list(phase)}))
        for run in started:
            (prov / f"arm_{run}.started").write_text("t\n")
        (prov / "sweep_controller.pid").write_text("999999\n")
        return prov

    def test_requires_a_fatal_marker(self):
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaises(ValueError):
                FATAL.choose_run(self.prov(tmp, fatal=False), disks=set())

    def test_no_marker_never_boots_even_with_started_arms_and_disks(self):
        with tempfile.TemporaryDirectory() as tmp:
            prov = self.prov(tmp, fatal=False, started=("on_urt", "on_relay_off"))
            with self.assertRaises(ValueError):
                FATAL.choose_run(prov, disks={"on_urt", "on_relay_off"})

    def test_chooses_the_fatal_arm_when_its_overlay_exists(self):
        with tempfile.TemporaryDirectory() as tmp:
            prov = self.prov(tmp)
            self.assertEqual(
                FATAL.choose_run(prov, disks={"on_relay_off", "on_urt"}), "on_relay_off"
            )

    def test_falls_back_to_the_latest_started_arm_with_an_overlay(self):
        with tempfile.TemporaryDirectory() as tmp:
            prov = self.prov(tmp, phase=(None, None), started=("on_urt", "on_relay_off"))
            os.utime(prov / "arm_on_urt.started", (1, 1))
            self.assertEqual(
                FATAL.choose_run(prov, disks={"on_urt", "on_relay_off"}), "on_relay_off"
            )

    def test_fallback_skips_an_arm_whose_collection_already_completed(self):
        with tempfile.TemporaryDirectory() as tmp:
            prov = self.prov(tmp, phase=(None, None), started=("on_urt",))
            (prov / "collect_on_urt.log").write_text("COLLECT_ALL_DONE 2026\n")
            self.assertIsNone(FATAL.choose_run(prov, disks={"on_urt"}))

    def test_fatal_arm_without_overlay_collects_nothing(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.assertIsNone(FATAL.choose_run(self.prov(tmp), disks={"on_urt"}))

    def test_no_overlay_means_logs_only(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.assertIsNone(FATAL.choose_run(self.prov(tmp, started=()), disks=set()))

    def test_linux_or_bind_phase_never_boots_windows_collection(self):
        with tempfile.TemporaryDirectory() as tmp:
            prov = self.prov(tmp, phase=("lx_reload", 1), started=("on_urt", "lx_reload"))
            self.assertIsNone(FATAL.choose_run(prov, disks={"on_urt"}))
            prov = self.prov(tmp, phase=("bind", 1), started=())
            self.assertIsNone(FATAL.choose_run(prov, disks=set()))

    def test_preconditions_refuse_live_qemu_or_live_controller(self):
        with tempfile.TemporaryDirectory() as tmp:
            prov = self.prov(tmp)
            with self.assertRaises(ValueError):
                FATAL.preconditions(prov, qemu_running=True, controller_alive=False)
            with self.assertRaises(ValueError):
                FATAL.preconditions(prov, qemu_running=False, controller_alive=True)
            self.assertTrue(FATAL.preconditions(prov, qemu_running=False, controller_alive=False))

    def test_preconditions_need_the_marker(self):
        with tempfile.TemporaryDirectory() as tmp:
            prov = self.prov(tmp, fatal=False)
            with self.assertRaises(ValueError):
                FATAL.preconditions(prov, qemu_running=False, controller_alive=False)

    def test_preconditions_refuse_a_live_qemu_alone(self):
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaises(ValueError):
                FATAL.preconditions(self.prov(tmp), qemu_running=True, controller_alive=False)

    def test_collection_uses_exactly_the_collect_script(self):
        self.assertEqual(FATAL.collect_command("on_urt")[3], "/root/sweep2/collect_bw.sh")

    def test_dropped_xid_history_refuses(self):
        with self.assertRaises(ValueError):
            FATAL.xid_delta(["a", "b"], ["a"])

    def test_collection_command_is_standard_vga_bounded_and_kf3_free(self):
        command = FATAL.collect_command("on_relay_off")
        self.assertEqual(command[:3], ["timeout", "900", "bash"])
        self.assertIn("/root/sweep2/collect_bw.sh", command)
        self.assertNotIn("--arm", command)
        self.assertEqual(command[-1], "on_relay_off")

    def test_new_xid_during_collection_is_a_recorded_finding(self):
        before = ["a NVRM: Xid 1"]
        self.assertEqual(FATAL.xid_delta(before, before), [])
        self.assertEqual(FATAL.xid_delta(before, before + ["b NVRM: Xid 2"]), ["b NVRM: Xid 2"])
        with self.assertRaises(ValueError):
            FATAL.xid_delta(before + ["c"], ["c"])


class ScriptShapes(unittest.TestCase):
    """Static checks on the bash and PowerShell pieces (they run only on the rental)."""

    def assertIn(self, needle, haystack, msg=None):  # short failure text, never the file
        self.assertTrue(needle in haystack, msg or needle)

    def assertNotIn(self, needle, haystack, msg=None):
        self.assertFalse(needle in haystack, msg or needle)

    def test_bash_syntax(self):
        for name in [
            "arm.sh",
            "controller.sh",
            "collect_bw.sh",
            "prep_boot.sh",
            "linux_reload.sh",
            "reload_hook.sh",
            "fatal_collect.sh",
        ]:
            r = subprocess.run(["bash", "-n", str(K / name)], capture_output=True, text=True)
            self.assertEqual(r.returncode, 0, name + r.stderr)

    def test_collection_and_prep_boots_have_no_kf3_device_and_no_flags(self):
        for name in ("collect_bw.sh", "prep_boot.sh"):
            source = text(name)
            self.assertIn("--no-kf3", source, name)
            self.assertIn("compgen -A variable KF3_", source, name)
            self.assertNotIn("p19.flags", source, name)
            self.assertNotIn("--arm B", source, name)
            self.assertNotIn("--guest-driver", source, name)

    def test_collection_boot_is_bounded_to_ten_minutes_of_guest_work(self):
        source = text("collect_bw.sh")
        self.assertIn("COLLECT_BUDGET=600", source)
        self.assertIn("COLLECT_DEADLINE_SKIP", source)
        self.assertIn("capture_guest.ps1", source)
        self.assertIn("capture_pack.py", source)

    def test_arm_uses_the_pinned_plan_and_carries_the_probed_die(self):
        source = text("arm.sh")
        self.assertIn("/root/sweep2/b1n_arms.py prepare", source)
        self.assertIn("permit_fields", source)
        self.assertIn("prep_boot.sh", source)
        self.assertNotIn("enable_count=1,", source)
        self.assertIn("C:\\\\kf\\\\$enable_script", source)

    def test_controller_records_the_probed_die_and_runs_the_plan(self):
        source = text("controller.sh")
        self.assertIn("b1n_arms.py probed-die", source)
        self.assertIn("b1n_arms.py list", source)
        self.assertIn("linux_reload.sh", source)
        self.assertNotIn("run_cell on_observe_b ", source)

    def test_cycle_script_enables_twice_and_disables_once_with_bounded_waits(self):
        source = text("enable580_cycle.ps1")
        self.assertEqual(source.count("pnputil /enable-device"), 2)
        self.assertEqual(source.count("pnputil /disable-device"), 1)
        self.assertIn("enable580_cycle.started", source)
        self.assertIn("DIAGNOSTIC", source)
        self.assertNotIn("Start-Sleep -Seconds 600", source)
        self.assertEqual(len(re.findall(r"WaitProblem (?:0|22) \d{2,3}\n", source)), 3)
        self.assertIn("AddSeconds($seconds)", source)

    def test_prep_sets_only_tdrddidelay_60(self):
        source = text("prep_tdr60.ps1")
        self.assertIn("GraphicsDrivers", source)
        self.assertEqual(re.findall(r"-Name (\w+)", source), ["TdrDdiDelay"])
        self.assertIn("-Value 60", source)
        self.assertIn("PREP_TDR60_DONE", source)

    def test_guest_capture_reads_and_copies_locally_only(self):
        source = text("capture_guest.ps1")
        for needle in ("LiveKernelReports", "Minidump", "WER", "wevtutil epl", "Get-FileHash"):
            self.assertIn(needle, source)
        for forbidden in (
            "Invoke-WebRequest",
            "http",
            "Remove-Item C:\\Windows",
            "Set-ItemProperty",
        ):
            self.assertNotIn(forbidden, source)

    def test_linux_arm_registers_its_qemu_for_the_guard_and_is_bounded(self):
        source = text("linux_reload.sh")
        self.assertIn("phase.json", source)
        self.assertIn("timeout 900", source)
        self.assertIn("POST_CAPTURE_HOOK=/root/sweep2/reload_hook.sh", source)
        self.assertRegex(source, r"flock (-w \d+ )?-o /tmp/kayfabe-fastguest.lock")
        hook = text("reload_hook.sh")
        self.assertIn("rmmod", hook)
        self.assertIn("modprobe nvidia", hook)
        self.assertIn("RELOAD_VERDICT", hook)

    def test_fatal_collect_never_starts_a_kf3_boot(self):
        source = text("fatal_collect.sh")
        self.assertIn("fatal_collect.py", source)
        self.assertNotIn("arm.sh", source)
        self.assertNotIn("p19.flags", source)


if __name__ == "__main__":
    unittest.main()
