"""Per-arm permission records only the owner's bounded Windows Xid32 pair."""

import copy
import datetime as dt
import io
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import MagicMock, patch

sys.path.insert(0, str(Path(__file__).parent))
try:
    import windows_observation as O
except ImportError:
    O = None

NOW = dt.datetime(2026, 10, 23, 12, 0, tzinfo=dt.timezone.utc)
PHASE = {
    "arm": "on_observe_b",
    "pid": 111032,
    "starttime": 191783,
    "exe": "/test/qemu-system-x86_64",
    "pci": "0000:00:07",
}
AUTHORITY = {
    "version": 1,
    "lease_id": 12345678,
    "record_limit": 8,
    "arms": ["on_observe_b", "on_observe_b1"],
    "not_before_utc": "2026-10-23T11:00:00+00:00",
    "expires_utc": "2026-10-23T13:30:00+00:00",
}
PERMIT = {
    **PHASE,
    "lease_id": 12345678,
    "profile": "on",
    "enable_count": 1,
    "armed_utc": "2026-10-23T11:59:00+00:00",
    "mode": "windows-on-xid32-observation-v1",
}


def xid(channel=0x14, *, pid=111032, code=32, intr="00200000", second=1):
    return (
        f"2026-10-23T11:59:{second:02d}.485372+0000 ubuntu kernel: "
        f"NVRM: Xid (PCI:0000:00:07): {code}, pid={pid}, "
        f"name=qemu-system-x86, channel {channel:#010x} intr {intr}"
    )


def pair(count=2):
    return [xid(0x14 if i % 2 == 0 else 0x0E00000B, second=i + 1) for i in range(count)]


class Observation(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(O, "per-arm observation implementation missing")

    def judge(self, lines=None, **changes):
        args = dict(phase=PHASE, permit=PERMIT, authority=AUTHORITY, now=NOW)
        args.update(changes)
        return O.judge_observation(pair() if lines is None else lines, **args)

    def test_actual_pair_is_recorded_with_identity_intr_and_timestamps(self):
        rows = self.judge()
        self.assertEqual([r["channel"] for r in rows], ["0x14", "0xe00000b"])
        self.assertEqual(rows[0]["pid"], 111032)
        self.assertEqual(rows[0]["intr"], "00200000")
        self.assertEqual(rows[0]["timestamp_utc"], "2026-10-23T11:59:01.485372+00:00")

    def test_single_record_can_precede_the_second_half_of_the_pair(self):
        self.assertEqual(len(self.judge(pair(1))), 1)

    def test_eight_records_from_the_same_pair_are_recorded(self):
        self.assertEqual(len(self.judge(pair(8))), 8)

    def test_permit_is_bound_to_each_fresh_arm_and_pid_life(self):
        phase = {**PHASE, "arm": "on_observe_b1", "pid": 222}
        permit = {**PERMIT, **phase}
        rows = self.judge([xid(pid=222)], phase=phase, permit=permit)
        self.assertEqual(rows[0]["pid"], 222)

    def test_absent_permit_does_not_enable_the_mode(self):
        self.assertFalse(O.scope_enabled(PHASE, {}, AUTHORITY, NOW))

    def test_previous_arm_permit_does_not_enable_the_next_arm(self):
        phase = {**PHASE, "arm": "on_observe_b1"}
        self.assertFalse(O.scope_enabled(phase, PERMIT, AUTHORITY, NOW))

    def test_ninth_record_stops(self):
        with self.assertRaises(ValueError):
            self.judge(pair(9))

    def test_tenth_record_stops(self):
        with self.assertRaises(ValueError):
            self.judge(pair(10))

    def test_second_plain_channel_stops(self):
        with self.assertRaises(ValueError):
            self.judge(pair() + [xid(0x15, second=3)])

    def test_second_high_channel_stops(self):
        with self.assertRaises(ValueError):
            self.judge(pair() + [xid(0x0E00000C, second=3)])

    def test_kernel_error_beside_the_allowed_pair_stops(self):
        rows = [{"priority": 3, "line": "NVRM: GPU has fallen off the bus"}]
        with self.assertRaises(ValueError):
            O.check_kernel_errors(rows, pair())

    def test_error_text_at_warning_priority_also_stops(self):
        rows = [{"priority": 4, "line": "NVRM: RmInitAdapter failed"}]
        with self.assertRaises(ValueError):
            O.check_kernel_errors(rows, pair())

    def test_only_exact_recorded_xids_are_excluded_from_new_kernel_errors(self):
        rows = [{"priority": 3, "line": line} for line in pair()]
        rows.append({"priority": 6, "line": "normal informational kernel message"})
        O.check_kernel_errors(rows, pair())

    def test_replaced_kernel_history_stops(self):
        with self.assertRaises(ValueError):
            O.new_kernel_rows([{"cursor": "changed"}], ["original"])

    def test_lost_kernel_history_stops(self):
        with self.assertRaises(ValueError):
            O.new_kernel_rows([], ["original"])


def install_negative(name, mutate):
    def test(self):
        lines, args = pair(), {
            "phase": copy.deepcopy(PHASE),
            "permit": copy.deepcopy(PERMIT),
            "authority": copy.deepcopy(AUTHORITY),
            "now": NOW,
        }
        changed = mutate(lines, args)
        with self.assertRaises(ValueError):
            self.judge(lines if changed is None else changed, **args)

    setattr(Observation, "test_" + name, test)


NEGATIVES = {
    "foreign_pid_stops": lambda lines, args: [xid(pid=999)],
    "foreign_pid_beside_owned_record_stops": lambda lines, args: [
        xid(),
        xid(0x0E00000B, pid=222, second=2),
    ],
    "reused_pid_life_stops": lambda lines, args: args["permit"].update(starttime=191784),
    "different_executable_stops": lambda lines, args: args["permit"].update(exe="/other/qemu"),
    "wrong_pci_stops": lambda lines, args: [xid().replace("00:07", "00:08")],
    "native_31_stops": lambda lines, args: [xid(code=31)],
    "fallen_bus_79_stops": lambda lines, args: [xid(code=79)],
    "xid_61_stops": lambda lines, args: [xid(code=61)],
    "xid_62_stops": lambda lines, args: [xid(code=62)],
    "xid_45_stops": lambda lines, args: [xid(code=45)],
    "wrong_intr_stops": lambda lines, args: [xid(intr="00800000")],
    "zero_intr_stops": lambda lines, args: [xid(intr="00000000")],
    "wrong_high_prefix_stops": lambda lines, args: [xid(0x0F00000B)],
    "high_suffix_overflow_stops": lambda lines, args: [xid(0x0E000100)],
    "malformed_xid_stops": lambda lines, args: ["NVRM: Xid malformed"],
    "missing_timestamp_stops": lambda lines, args: [xid().split(" ubuntu ", 1)[1]],
    "off_profile_stops": lambda lines, args: args["permit"].update(profile="off"),
    "native_profile_stops": lambda lines, args: args["permit"].update(profile="native"),
    "second_enable_stops": lambda lines, args: args["permit"].update(enable_count=2),
    "wrong_lease_stops": lambda lines, args: args["permit"].update(lease_id=87654321),
    "expired_authority_stops": lambda lines, args: args["authority"].update(
        expires_utc="2026-10-23T11:59:00+00:00"
    ),
    "future_authority_stops": lambda lines, args: args["authority"].update(
        not_before_utc="2026-10-23T12:01:00+00:00"
    ),
    "limit_cannot_be_raised_stops": lambda lines, args: args["authority"].update(record_limit=9),
    "authority_over_150_minutes_stops": lambda lines, args: args["authority"].update(
        expires_utc="2026-10-23T13:31:00+00:00"
    ),
    "permanent_authority_stops": lambda lines, args: args["authority"].update(
        expires_utc="2027-10-23T13:30:00+00:00"
    ),
    "fault_before_enable_stops": lambda lines, args: args["permit"].update(
        armed_utc="2026-10-23T11:59:30+00:00"
    ),
    "future_fault_stops": lambda lines, args: [xid().replace("11:59", "12:01")],
}
for case, mutate in NEGATIVES.items():
    install_negative(case, mutate)


class HostHealth(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(O, "per-arm observation implementation missing")

    def probe(self, smi_rc=0, cuda_rc=0, cuda_text=None, smi_text=None, timeout=False):
        answers = [
            SimpleNamespace(
                returncode=smi_rc,
                stdout="00000000:00:07.0, 595.91.07\n" if smi_text is None else smi_text,
            ),
            SimpleNamespace(
                returncode=cuda_rc,
                stdout=(
                    '{"cuInit":0,"cuDeviceGetCount":0,"devices":1}'
                    if cuda_text is None
                    else cuda_text
                ),
            ),
        ]
        calls = []

        def run(argv, **kwargs):
            calls.append((argv, kwargs))
            if timeout:
                raise subprocess.TimeoutExpired(argv, 3)
            return answers[len(calls) - 1]

        return O.check_host_health("0000:00:07", runner=run), calls

    def test_stock_smi_and_cuda_health_must_both_pass(self):
        result, calls = self.probe()
        self.assertEqual(result["devices"], 1)
        self.assertEqual(len(calls), 2)
        self.assertTrue(all(call[1]["timeout"] <= 3 for call in calls))

    def test_nvidia_smi_failure_stops(self):
        with self.assertRaises(ValueError):
            self.probe(smi_rc=1)

    def test_cuinit_failure_stops(self):
        with self.assertRaises(ValueError):
            self.probe(cuda_text='{"cuInit":999,"cuDeviceGetCount":0,"devices":1}')

    def test_cuda_process_failure_stops(self):
        with self.assertRaises(ValueError):
            self.probe(cuda_rc=1)

    def test_gpu_fallen_off_bus_text_stops(self):
        with self.assertRaises(ValueError):
            self.probe(smi_text="GPU has fallen off the bus")

    def test_probe_timeout_stops(self):
        with self.assertRaises(ValueError):
            self.probe(timeout=True)

    def test_no_cuda_device_stops(self):
        with self.assertRaises(ValueError):
            self.probe(cuda_text='{"cuInit":0,"cuDeviceGetCount":0,"devices":0}')


class WatchFinished(BaseException):
    pass


class KernelSnapshot(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(O, "per-arm observation implementation missing")

    def snapshot(self, require_outer_deadline=False):
        messages = ["native record one", "native record two"]
        raw = b"".join(
            (
                json.dumps(
                    {
                        "__REALTIME_TIMESTAMP": str(1792756741000000 + index),
                        "__CURSOR": str(index),
                        "_HOSTNAME": "ubuntu",
                        "PRIORITY": "6",
                        "MESSAGE": message,
                    }
                )
                + "\n"
            ).encode()
            for index, message in enumerate(messages)
        )
        calls = []

        def popen(argv, **kwargs):
            calls.append(argv)
            if require_outer_deadline and argv[0] != "timeout":
                raise RuntimeError("pipe read has no external wall bound")
            proc = MagicMock()
            proc.stdout = io.BytesIO(raw)
            proc.wait.return_value = 0
            proc.poll.return_value = 0
            return proc

        with patch.object(O.subprocess, "Popen", side_effect=popen):
            rows = O.kernel_rows()
        return rows, calls

    def test_kernel_pipe_has_an_external_deadline_before_read(self):
        _, calls = self.snapshot()
        self.assertEqual(calls[0][0], "timeout")
        self.assertLessEqual(float(calls[0][1]), 3)

    def test_blocking_pipe_cannot_escape_the_external_deadline(self):
        rows, _ = self.snapshot(require_outer_deadline=True)
        self.assertEqual(len(rows), 2)


class WatcherLifecycle(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(O, "per-arm observation implementation missing")

    def exercise(self, new_fault_after_exit=False, owner_error="owned QEMU exited"):
        class Clock(dt.datetime):
            @classmethod
            def now(cls, tz=None):
                return NOW

        initial = [
            {"cursor": "a", "priority": 6, "line": "kernel initial one"},
            {"cursor": "b", "priority": 6, "line": "kernel initial two"},
        ]
        one = initial + [{"cursor": "c", "priority": 3, "line": xid()}]
        two = one + [{"cursor": "d", "priority": 3, "line": xid(0x0E00000B, second=2)}]
        first, second = (one, two) if new_fault_after_exit else (two, two)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name, value in [
                ("native.json", {"approved_native_xid_lines": [], "pci": PHASE["pci"]}),
                ("phase.json", PHASE),
                ("permit.json", PERMIT),
                ("authority.json", AUTHORITY),
            ]:
                (root / name).write_text(json.dumps(value))
            with (
                patch.object(O, "kernel_rows", side_effect=[initial, first, second]),
                patch.object(O, "check_host_health", return_value={"devices": 1}),
                patch.object(
                    O.G, "validate_phase_owner", side_effect=[True, True, O.Refused(owner_error)]
                ) as validate,
                patch.object(O.G, "abort_owned_qemu", return_value=False) as abort,
                patch.object(O.G.N, "terminate_owned_tree") as terminate,
                patch.object(O.os, "getpgid", return_value=1234),
                patch.object(O.os, "getpgrp", return_value=1234),
                patch.object(O.dt, "datetime", Clock),
                patch.object(
                    O.time,
                    "sleep",
                    side_effect=[
                        None,
                        WatchFinished if new_fault_after_exit else KeyboardInterrupt,
                    ],
                ),
            ):
                arguments = [
                    str(root / "native.json"),
                    1234,
                    str(root / "phase.json"),
                    str(root / "fatal.json"),
                    str(root / "authority.json"),
                    str(root / "permit.json"),
                ]
                if new_fault_after_exit:
                    with self.assertRaises(O.Refused):
                        O.watch(*arguments)
                    self.assertTrue((root / "fatal.json").exists())
                    abort.assert_called_once()
                    terminate.assert_called_once_with(1234)
                else:
                    with self.assertRaises(KeyboardInterrupt):
                        O.watch(*arguments)
                    self.assertFalse((root / "fatal.json").exists())
                    abort.assert_not_called()
                    terminate.assert_not_called()
                    self.assertEqual(validate.call_count, 2)

    def test_guest_exit_without_new_faults_can_reach_collection(self):
        self.exercise()

    def test_pid_reuse_without_new_faults_does_not_signal_the_replacement(self):
        self.exercise(owner_error="starttime changed after guest exit")

    def test_new_fault_after_owned_qemu_exits_is_fatal(self):
        self.exercise(new_fault_after_exit=True)

    def test_new_fault_after_pid_reuse_is_fatal(self):
        self.exercise(new_fault_after_exit=True, owner_error="starttime changed")

    def exercise_phase_gap(self, count):
        class Clock(dt.datetime):
            @classmethod
            def now(cls, tz=None):
                return NOW

        initial = [
            {"cursor": "a", "priority": 6, "line": "initial kernel one"},
            {"cursor": "b", "priority": 6, "line": "initial kernel two"},
        ]

        def rows(number):
            return initial + [
                {"cursor": f"x{i}", "priority": 3, "line": line}
                for i, line in enumerate(pair(number))
            ]

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name, value in [
                ("native.json", {"approved_native_xid_lines": [], "pci": PHASE["pci"]}),
                ("phase.json", PHASE),
                ("permit.json", PERMIT),
                ("authority.json", AUTHORITY),
            ]:
                (root / name).write_text(json.dumps(value))
            slept = []

            def sleep(_):
                slept.append(True)
                if len(slept) == 1:
                    (root / "phase.json").write_text("{}")
                elif len(slept) == 2:
                    (root / "phase.json").write_text(json.dumps(PHASE))
                else:
                    raise WatchFinished

            with (
                patch.object(
                    O, "kernel_rows", side_effect=[initial, rows(8), rows(8), rows(count)]
                ),
                patch.object(O, "check_host_health", return_value={"devices": 1}),
                patch.object(O.G, "validate_phase_owner", return_value=True),
                patch.object(O.G, "abort_owned_qemu", return_value=True) as abort,
                patch.object(O.G.N, "terminate_owned_tree") as terminate,
                patch.object(O.os, "getpgid", return_value=1234),
                patch.object(O.os, "getpgrp", return_value=1234),
                patch.object(O.dt, "datetime", Clock),
                patch.object(O.time, "sleep", side_effect=sleep),
            ):
                with self.assertRaises(O.Refused):
                    O.watch(
                        str(root / "native.json"),
                        1234,
                        str(root / "phase.json"),
                        str(root / "fatal.json"),
                        str(root / "authority.json"),
                        str(root / "permit.json"),
                    )
                self.assertIn("eight", json.loads((root / "fatal.json").read_text())["STOP"])
                abort.assert_called_once()
                terminate.assert_called_once_with(1234)

    def test_phase_gap_does_not_reset_the_ninth_record_limit(self):
        self.exercise_phase_gap(9)

    def test_phase_gap_does_not_reset_the_tenth_record_limit(self):
        self.exercise_phase_gap(10)


if __name__ == "__main__":
    unittest.main()
