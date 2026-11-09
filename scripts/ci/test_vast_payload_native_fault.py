"""XD1: contract-shaped host semaphore faults; synthetic 5070 log shape, no GPU."""

import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

import vast_payload as payload
from test_vast_payload import watch

ROOT = Path(__file__).resolve().parents[2]
SOURCE = """KF_NATIVE_EXEC pid=4242 name=kayfabe-rm-ladd
info  R3 bystander before = Landed
FAULT_MARK=victim_release va=0x0000000a11000000
kayfabe-isolate: DOORBELL-STORE #2 host_token=0x40000003 runlist=16384 chid=3
info  R3 victim           = Lost { saw: 3735880569 }
info  R3 notifier         = fired=true status=0xffff except_type=0x1f engine=0x0001
info  R3 bystander after  = Landed
RUNGCTL_missing_page=PASS
RUNG_missing_page=PASS
"""
BLACKWELL = (
    "[ 1193.744521] NVRM: Xid (PCI:0000:00:00): 31, pid=4242, "
    "name=kayfabe-rm-ladd, channel 0x00000003, intr 00000000. "
    "MMU Fault: ENGINE GR_HOST0 HUBCLIENT_ESC0 faulted @ 0xa_11000000. "
    "Fault is of type FAULT_PDE ACCESS_TYPE_VIRT_WRITE"
)
LEGACY = BLACKWELL.replace("GR_HOST0 HUBCLIENT_ESC0", "GR0_PBDMA0 HUBCLIENT_ESC")


class NativeFaultContract(unittest.TestCase):
    def admit(self, line=BLACKWELL, source=SOURCE, arm="--missing-page-fault", before=4):
        text = watch(line, before, before + len(line.splitlines())).replace(
            "\nfixture\n", "\n" + source + "\n"
        )
        return payload.xid_ok(text, 6, arm, text)

    def test_blackwell_host_esc_is_the_intentional_fault(self):
        self.assertTrue(self.admit())

    def test_legacy_pbdma_esc_is_the_intentional_fault(self):
        self.assertTrue(self.admit(LEGACY))

    def test_all_header_derived_host_names_accept(self):
        for engine in ("GR_HOST7", "GR7_PBDMA1"):
            for client in ("HUBCLIENT_ESC", "HUBCLIENT_ESC11"):
                with self.subTest(engine=engine, client=client):
                    self.assertTrue(
                        self.admit(
                            BLACKWELL.replace("GR_HOST0", engine).replace("HUBCLIENT_ESC0", client)
                        )
                    )

    def test_channel_is_from_this_submission_not_a_die(self):
        self.assertTrue(
            self.admit(
                BLACKWELL.replace("channel 0x00000003", "channel 0x00000017"),
                SOURCE.replace("chid=3", "chid=23"),
            )
        )

    def test_header_derived_nonzero_host_esc_accepts(self):
        self.assertTrue(
            self.admit(
                BLACKWELL.replace("GR_HOST0", "GR_HOST1").replace(
                    "HUBCLIENT_ESC0", "HUBCLIENT_ESC1"
                )
            )
        )

    def test_wrong_pid_blackwell_refuses(self):
        self.assertFalse(self.admit(BLACKWELL.replace("pid=4242", "pid=4243")))

    def test_wrong_pid_legacy_refuses(self):
        self.assertFalse(self.admit(LEGACY.replace("pid=4242", "pid=4243")))

    def test_other_xid_blackwell_refuses(self):
        self.assertFalse(self.admit(BLACKWELL.replace(": 31,", ": 32,")))

    def test_other_xid_legacy_refuses(self):
        self.assertFalse(self.admit(LEGACY.replace(": 31,", ": 79,")))

    def test_extra_fault_blackwell_refuses(self):
        self.assertFalse(self.admit(BLACKWELL + "\n" + BLACKWELL))

    def test_extra_fault_legacy_refuses(self):
        self.assertFalse(self.admit(LEGACY + "\n" + LEGACY))

    def test_wrong_va_blackwell_refuses(self):
        self.assertFalse(self.admit(BLACKWELL.replace("0xa_11000000", "0xa_11001000")))

    def test_wrong_va_legacy_refuses(self):
        self.assertFalse(self.admit(LEGACY.replace("0xa_11000000", "0xa_11000004")))

    def test_changed_contract_va_in_both_records_refuses(self):
        self.assertFalse(
            self.admit(
                BLACKWELL.replace("0xa_11000000", "0xa_12000000"),
                SOURCE.replace("0x0000000a11000000", "0x0000000a12000000"),
            )
        )

    def test_changed_contract_va_legacy_in_both_records_refuses(self):
        self.assertFalse(
            self.admit(
                LEGACY.replace("0xa_11000000", "0xa_12000000"),
                SOURCE.replace("0x0000000a11000000", "0x0000000a12000000"),
            )
        )

    def test_wrong_channel_blackwell_refuses(self):
        self.assertFalse(self.admit(BLACKWELL.replace("channel 0x00000003", "channel 0x00000002")))

    def test_wrong_channel_legacy_refuses(self):
        self.assertFalse(self.admit(LEGACY.replace("channel 0x00000003", "channel 0x00000002")))

    def test_non_fault_timer_refuses(self):
        self.assertFalse(self.admit(arm="--timer"))

    def test_non_fault_executor_refuses(self):
        self.assertFalse(self.admit(arm="--executor-vas"))

    def test_wrong_name_refuses(self):
        self.assertFalse(self.admit(BLACKWELL.replace("name=kayfabe-rm-ladd", "name=other")))

    def test_wrong_name_legacy_refuses(self):
        self.assertFalse(self.admit(LEGACY.replace("name=kayfabe-rm-ladd", "name=qemu-system-x86")))

    def test_unknown_engine_refuses(self):
        for engine in ("CE0", "GR_HOST8", "GR0_PBDMA2", "GR_HOST0garbage"):
            with self.subTest(engine=engine):
                self.assertFalse(self.admit(BLACKWELL.replace("GR_HOST0", engine)))

    def test_unknown_client_refuses(self):
        for client in (
            "HUBCLIENT_PMU",
            "HUBCLIENT_CE0",
            "HUBCLIENT_ESC12",
            "HUBCLIENT_ESC0garbage",
        ):
            with self.subTest(client=client):
                self.assertFalse(self.admit(BLACKWELL.replace("HUBCLIENT_ESC0", client)))

    def test_non_host_engine_legacy_refuses(self):
        self.assertFalse(self.admit(LEGACY.replace("GR0_PBDMA0", "CE0")))

    def test_non_esc_client_legacy_refuses(self):
        self.assertFalse(self.admit(LEGACY.replace("HUBCLIENT_ESC", "HUBCLIENT_PMU")))

    def test_wrong_fault_type_refuses(self):
        self.assertFalse(self.admit(BLACKWELL.replace("FAULT_PDE", "FAULT_PTE")))

    def test_wrong_access_type_refuses(self):
        self.assertFalse(self.admit(BLACKWELL.replace("VIRT_WRITE", "VIRT_READ")))

    def test_wrong_fault_type_legacy_refuses(self):
        self.assertFalse(self.admit(LEGACY.replace("FAULT_PDE", "FAULT_PTE")))

    def test_wrong_access_type_legacy_refuses(self):
        self.assertFalse(self.admit(LEGACY.replace("VIRT_WRITE", "VIRT_READ")))

    def test_malformed_interrupt_blackwell_refuses(self):
        self.assertFalse(self.admit(BLACKWELL.replace("intr 00000000", "intr 000000000")))

    def test_malformed_interrupt_legacy_refuses(self):
        self.assertFalse(self.admit(LEGACY.replace("intr 00000000", "intr 000000000")))

    def test_interrupt_value_is_diagnostic_blackwell(self):
        self.assertTrue(self.admit(BLACKWELL.replace("intr 00000000", "intr 00000001")))

    def test_interrupt_value_is_diagnostic_legacy(self):
        self.assertTrue(self.admit(LEGACY.replace("intr 00000000", "intr 00000001")))

    def test_missing_pid_attestation_refuses(self):
        self.assertFalse(self.admit(LEGACY, SOURCE.split("\n", 1)[1]))

    def test_duplicate_pid_attestation_refuses(self):
        self.assertFalse(self.admit(LEGACY, SOURCE.splitlines()[0] + "\n" + SOURCE))

    def test_identity_after_fault_operation_blackwell_refuses(self):
        identity, body = SOURCE.split("\n", 1)
        self.assertFalse(self.admit(BLACKWELL, body + identity + "\n"))

    def test_identity_after_fault_operation_legacy_refuses(self):
        identity, body = SOURCE.split("\n", 1)
        self.assertFalse(self.admit(LEGACY, body + identity + "\n"))

    def test_hidden_second_xid_blackwell_refuses(self):
        self.assertFalse(self.admit("NVRM: Xid hidden; " + BLACKWELL))

    def test_hidden_second_xid_legacy_refuses(self):
        self.assertFalse(self.admit("NVRM: Xid hidden; " + LEGACY))

    def test_missing_notifier_refuses(self):
        self.assertFalse(self.admit(LEGACY, SOURCE.replace("fired=true", "fired=false")))

    def test_wrong_notifier_refuses(self):
        self.assertFalse(self.admit(LEGACY, SOURCE.replace("except_type=0x1f", "except_type=0x20")))

    def test_source_before_command_window_refuses(self):
        text = SOURCE + watch(LEGACY, 4, 5)
        self.assertFalse(payload.xid_ok(text, 6, "--missing-page-fault", text))

    def test_source_after_command_window_refuses(self):
        text = watch(LEGACY, 4, 5) + SOURCE
        self.assertFalse(payload.xid_ok(text, 6, "--missing-page-fault", text))

    def test_later_matching_fault_is_fatal(self):
        with self.assertRaises(payload.xid_guard.GuardRefused):
            payload.xid_guard.check_unchanged(LEGACY + "\n" + BLACKWELL, [LEGACY])

    def test_replaced_matching_fault_is_fatal(self):
        with self.assertRaises(payload.xid_guard.GuardRefused):
            payload.xid_guard.check_unchanged(BLACKWELL, [LEGACY])


class NativeExecHook(unittest.TestCase):
    """Run the actual hook, watcher, native-exec and grader with fake dmesg only."""

    def invoke(self, line=BLACKWELL, *, wrong_pid=False, arm="--missing-page-fault", rc=0):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            bins = root / "bin"
            bins.mkdir()
            kernel = root / "kernel"
            kernel.write_text("kernel boot\nreadable log\n")
            scripts = {
                "dmesg": '#!/bin/sh\ncat "$KF_FIXTURE_KERNEL"\n',
                "nvidia-smi": '#!/bin/sh\necho "fixture GPU, fixture driver"\n',
                "sleep": "#!/bin/sh\nexit 0\n",
                "kayfabe-rm-ladder": (
                    "#!" + sys.executable + "\nimport os,sys\nfrom pathlib import Path\n"
                    "assert sys.argv[1:] == [" + repr(arm) + "]\n"
                    "print(" + repr(SOURCE.split("\n", 1)[1]) + ", flush=True)\n"
                    "line = "
                    + repr(line)
                    + ".replace('pid=4242', 'pid=' + str(os.getpid() + "
                    + str(int(wrong_pid))
                    + "))\n"
                    "with Path(os.environ['KF_FIXTURE_KERNEL']).open('a') as f: f.write(line+'\\n')\n"
                    "sys.exit(" + str(rc) + ")\n"
                ),
            }
            for name, body in scripts.items():
                path = bins / name
                path.write_text(body)
                path.chmod(0o700)
            env = {
                **os.environ,
                "PATH": str(bins) + ":" + os.environ["PATH"],
                "KF_FIXTURE_KERNEL": str(kernel),
                "KF_PAYLOAD_ROOT": str(ROOT),
                "BASH_ENV": str(ROOT / "scripts/ci/vast_payload_hooks.sh"),
                "BENCH_DIR": str(root),
                "XID_OUTDIR": str(root),
                "KF_PAYLOAD_XID_ALLOWED": str(root / "allowed"),
                "KF_PAYLOAD_XID_FAILED": str(root / "failed"),
            }
            # Isolate from receipt/diagnostic settings of a caller running this test.
            for name in (
                "KF_PAYLOAD_CONTROL_DIR",
                "KF_PAYLOAD_NATIVE_RECEIPT",
                "KF_PAYLOAD_DIAGNOSTIC_STATE",
                "KF_PAYLOAD_LOG",
            ):
                env.pop(name, None)
            result = subprocess.run(
                [
                    "/bin/bash",
                    "-c",
                    'timeout --kill-after=1 5 "$1" "$2"',
                    "fixture",
                    str(bins / "kayfabe-rm-ladder"),
                    arm,
                ],
                env=env,
                capture_output=True,
                text=True,
                timeout=10,
            )
            return (
                result.returncode,
                (root / "failed").exists(),
                (root / "allowed").exists(),
                result.stdout,
            )

    def test_blackwell_through_real_hook_exec_and_grader(self):
        rc, failed, allowed, output = self.invoke()
        self.assertEqual((rc, failed, allowed), (0, False, True), output)
        self.assertIn("KF_NATIVE_EXEC pid=", output)

    def test_legacy_through_real_hook_exec_and_grader(self):
        rc, failed, allowed, output = self.invoke(LEGACY)
        self.assertEqual((rc, failed, allowed), (0, False, True), output)

    def test_blackwell_other_process_through_hook_voids(self):
        rc, failed, allowed, output = self.invoke(wrong_pid=True)
        self.assertEqual((rc, failed, allowed), (6, True, False), output)

    def test_legacy_other_process_through_hook_voids(self):
        rc, failed, allowed, output = self.invoke(LEGACY, wrong_pid=True)
        self.assertEqual((rc, failed, allowed), (6, True, False), output)

    def test_non_fault_timer_through_hook_voids(self):
        rc, failed, allowed, output = self.invoke(arm="--timer")
        self.assertEqual((rc, failed, allowed), (6, True, False), output)

    def test_non_fault_executor_through_hook_voids(self):
        rc, failed, allowed, output = self.invoke(arm="--executor-vas")
        self.assertEqual((rc, failed, allowed), (6, True, False), output)

    def test_fault_with_failed_command_voids(self):
        rc, failed, allowed, output = self.invoke(rc=1)
        self.assertEqual((rc, failed, allowed), (6, True, False), output)

    def test_fault_with_timed_out_command_voids(self):
        rc, failed, allowed, output = self.invoke(rc=124)
        self.assertEqual((rc, failed, allowed), (6, True, False), output)


if __name__ == "__main__":
    unittest.main()
