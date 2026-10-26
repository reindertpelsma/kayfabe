"""Host-driver swap falsifiers: a swap that leaves the GPU unusable must be repaired or fail loudly.

Measured 2026-10-26 (live Worker gate run 40255723206, RTX 3090): provision_host_driver.sh printed
DRIVER_SWAP_DONE rc=0, and provision_full.sh ended 1382 s later with
`NOT_READY driver=No devices were found`. Probe box 57755545 (RTX 3090, same image) reproduced the
signature exactly: with the 575 packages purged there is no nouveau blacklist, so any udev "add"
event for the GPU while no nvidia module is loaded lets nouveau claim it. After
`rmmod nouveau; modprobe nvidia` the module loads (rc 0, /proc/driver/nvidia/version present, so
every check of the old script passed), RmInitAdapter fails ("the GPU is likely in a bad state and
may need to be reset") and nvidia-smi prints "No devices were found". A PCI function-level reset
plus a module reload repairs it; a nouveau blacklist followed by `udevadm control --reload`
prevents it.

The fix lives in marked blocks of provision_host_driver.sh and provision_full.sh. These tests run
those blocks against fake `nvidia-smi`, `rmmod`, `modprobe`, `dmesg`, `lsmod`, `udevadm` and a fake
sysfs tree. Path overrides (KF_SYSFS_PCI, KF_MODPROBE_D) are test seams with the real paths as
defaults.
"""

import os
import re
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DRIVER = Path(
    os.environ.get("KF_PROVISION_DRIVER") or ROOT / "scripts/bench/provision_host_driver.sh"
)
FULL = Path(os.environ.get("KF_PROVISION_FULL") or ROOT / "scripts/bench/box/provision_full.sh")

SMI_OK = "NVIDIA GeForce RTX 3090, 580.159.04"
SMI_BAD = "No devices were found"


def block(path, name):
    """The text between `# KF_<name>_BEGIN` and `# KF_<name>_END`, or a failing assertion."""
    text = path.read_text()
    begin, end = "# KF_%s_BEGIN" % name, "# KF_%s_END" % name
    if text.count(begin) != 1 or text.count(end) != 1:
        raise AssertionError("%s has no single marked block %s" % (path.name, name))
    return text.split(begin, 1)[1].split(end, 1)[0]


class Fake:
    """A scratch tree with fake system commands that log every call to one file."""

    def __init__(self, directory):
        self.root = Path(directory)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.log = self.root / "calls.log"
        self.log.write_text("")
        self.sysfs = self.root / "sys"
        self.modprobe_d = self.root / "modprobe.d"
        self.state = self.root / "state"
        self.state.mkdir()
        self.set("loaded", "")
        self.set("smi", "ok")
        self.set("smi_rc", "0")
        self.set("dmesg", "")
        self.set("healthy_after_resets", "0")
        self.command("rmmod", 'echo "rmmod $*" >> "$LOG"; exit 0')
        self.command("modprobe", 'echo "modprobe $*" >> "$LOG"; exit 0')
        self.command("systemctl", 'echo "systemctl $*" >> "$LOG"; exit 0')
        self.command("udevadm", 'echo "udevadm $*" >> "$LOG"; exit 0')
        self.command("nvidia-modprobe", 'echo "nvidia-modprobe $*" >> "$LOG"; exit 0')
        self.command("sleep", 'echo "sleep $*" >> "$LOG"; exit 0')
        self.command("timeout", 'shift; exec "$@"')
        self.command("lsmod", 'cat "$STATE/loaded"')
        self.command("dmesg", 'cat "$STATE/dmesg"')
        self.command(
            "nvidia-smi",
            'echo "nvidia-smi $*" >> "$LOG"\n'
            'n=$(grep -c . "$LOG_RESETS" 2>/dev/null); n=${n:-0}\n'
            'need=$(cat "$STATE/healthy_after_resets")\n'
            'if [ "$(cat "$STATE/smi")" = ok ] ||\n'
            '   { [ "$need" != never ] && [ "$n" -ge "$need" ]; }; then\n'
            '  case "$*" in\n'
            '    *--query-gpu=driver_version*) echo "580.159.04";;\n'
            '    *) echo "%s";;\n'
            "  esac\n"
            '  exit "$(cat "$STATE/smi_rc")"\n'
            "fi\n"
            'echo "%s"; exit "$(cat "$STATE/smi_rc_bad")"' % (SMI_OK, SMI_BAD),
        )
        self.set("smi_rc_bad", "6")

    def set(self, key, value):
        (self.state / key).write_text(str(value) + "\n")

    def command(self, name, body):
        path = self.bin / name
        path.write_text("#!/bin/bash\n" + body + "\n")
        path.chmod(path.stat().st_mode | stat.S_IXUSR)

    def pci(self, bdf, vendor, klass):
        d = self.sysfs / bdf
        d.mkdir(parents=True)
        (d / "vendor").write_text(vendor + "\n")
        (d / "class").write_text(klass + "\n")
        (d / "reset").write_text("")

    def calls(self):
        return self.log.read_text().splitlines()

    def run(self, script, extra_env=None):
        env = dict(
            os.environ,
            PATH=str(self.bin) + ":" + os.environ["PATH"],
            LOG=str(self.log),
            LOG_RESETS=str(self.root / "resets.log"),
            STATE=str(self.state),
            KF_SYSFS_PCI=str(self.sysfs),
            KF_MODPROBE_D=str(self.modprobe_d),
            KF_RECOVER_SLEEP="0",
        )
        env.update(extra_env or {})
        path = self.root / "case.sh"
        path.write_text(script)
        p = subprocess.run(["bash", str(path)], env=env, text=True, capture_output=True, timeout=60)
        return p.returncode, p.stdout + p.stderr


# Replaces the real sysfs write, so a test can count resets and see their order.
LOGGED_RESET = 'kf_pci_reset() { echo "reset $1" >> "$LOG"; echo "$1" >> "$LOG_RESETS"; }\n'


def with_gpu(fake):
    fake.pci("0000:00:08.0", "0x10de", "0x030000")
    fake.pci("0000:00:07.0", "0x10de", "0x040300")
    fake.pci("0000:00:03.0", "0x8086", "0x030000")


class NouveauGuard(unittest.TestCase):
    def guard(self, loaded=""):
        with tempfile.TemporaryDirectory() as d:
            fake = Fake(d)
            fake.set("loaded", loaded)
            script = block(DRIVER, "GPU_SETTLE") + "\nnouveau_guard\necho DONE\n"
            rc, out = fake.run(script)
            conf = fake.modprobe_d / "kf-blacklist-nouveau.conf"
            return rc, out, (conf.read_text() if conf.exists() else None), fake.calls()

    def test_the_guard_writes_a_nouveau_blacklist(self):
        rc, _, conf, _ = self.guard()
        self.assertEqual(rc, 0)
        self.assertIsNotNone(conf, "no kf-blacklist-nouveau.conf was written")
        self.assertRegex(conf, r"(?m)^blacklist nouveau$")

    def test_the_guard_keeps_nouveau_modesetting_off(self):
        _, _, conf, _ = self.guard()
        self.assertRegex(conf or "", r"(?m)^options nouveau modeset=0$")

    def test_udev_is_told_to_reload_its_module_rules(self):
        # Measured: without the reload, udev kept using the old (blacklist-free) module config
        # and nouveau still claimed the GPU.
        _, _, _, calls = self.guard()
        self.assertIn("udevadm control --reload", calls)

    def test_the_reload_comes_after_the_file_exists(self):
        with tempfile.TemporaryDirectory() as d:
            fake = Fake(d)
            fake.command(
                "udevadm",
                'if [ -s "$KF_MODPROBE_D/kf-blacklist-nouveau.conf" ]; then '
                'echo "udevadm saw-file $*" >> "$LOG"; '
                'else echo "udevadm no-file $*" >> "$LOG"; fi',
            )
            fake.run(block(DRIVER, "GPU_SETTLE") + "\nnouveau_guard\n")
            self.assertIn("udevadm saw-file control --reload", fake.calls())

    def test_a_loaded_nouveau_is_unloaded(self):
        _, _, _, calls = self.guard("nouveau              3100672  0\n")
        self.assertIn("rmmod nouveau", calls)

    def test_an_absent_nouveau_is_not_touched(self):
        _, _, _, calls = self.guard("nvidia              14422016  0\n")
        self.assertNotIn("rmmod nouveau", calls)

    def test_the_guard_runs_before_the_old_driver_is_removed(self):
        text = DRIVER.read_text()
        call = re.search(r"(?m)^nouveau_guard\b", text)
        purge = text.find("apt-get purge")
        unload = text.find("for m in nvidia_uvm")
        self.assertIsNotNone(call, "provision_host_driver.sh never calls nouveau_guard")
        self.assertLess(call.start(), purge)
        self.assertLess(call.start(), unload)


class Recovery(unittest.TestCase):
    def settle(self, healthy_after="0", tries=None, dmesg="", smi_bad_rc="6", fake_hook=None):
        with tempfile.TemporaryDirectory() as d:
            fake = Fake(d)
            with_gpu(fake)
            fake.set("smi", "bad" if healthy_after != "0" else "ok")
            fake.set("healthy_after_resets", healthy_after)
            fake.set("smi_rc_bad", smi_bad_rc)
            fake.set("dmesg", dmesg)
            if fake_hook:
                fake_hook(fake)
            script = block(DRIVER, "GPU_SETTLE") + LOGGED_RESET + "\ngpu_settle\necho RC=$?\n"
            env = {"KF_RECOVER_TRIES": str(tries)} if tries else None
            rc, out = fake.run(script, env)
            return out, fake.calls()

    def test_a_healthy_gpu_gets_no_recovery_at_all(self):
        out, calls = self.settle("0")
        self.assertIn("RC=0", out)
        self.assertEqual([c for c in calls if c.split()[0] in ("rmmod", "reset", "modprobe")], [])

    def test_the_listing_must_name_a_gpu_not_merely_exit_zero(self):
        # nvidia-smi can exit 0 while printing the message; the printed text is what counts.
        def hook(fake):
            fake.set("smi_rc", "0")
            fake.set("smi_rc_bad", "0")

        out, calls = self.settle("never", tries=1, fake_hook=hook)
        self.assertIn("RC=8", out)
        self.assertEqual(sum(c.startswith("reset ") for c in calls), 1)

    def test_a_wedged_gpu_is_reset_and_reloaded_once_then_passes(self):
        out, calls = self.settle("1")
        self.assertIn("RC=0", out)
        self.assertEqual([c for c in calls if c.startswith("reset ")], ["reset 0000:00:08.0"])

    def test_only_the_nvidia_display_function_is_reset(self):
        _, calls = self.settle("1")
        resets = [c for c in calls if c.startswith("reset ")]
        self.assertNotIn("reset 0000:00:07.0", resets)  # the GPU's audio function
        self.assertNotIn("reset 0000:00:03.0", resets)  # another vendor's display controller

    def test_recovery_order_unload_then_reset_then_reload(self):
        _, calls = self.settle("1")
        index = {}
        for i, c in enumerate(calls):
            index.setdefault(c, i)
        order = [
            "systemctl stop nvidia-persistenced",
            "rmmod nvidia",
            "reset 0000:00:08.0",
            "modprobe nvidia",
            "modprobe nvidia_uvm",
        ]
        for name in order:
            self.assertIn(name, index, calls)
        positions = [index[name] for name in order]
        self.assertEqual(positions, sorted(positions), calls)

    def test_every_nvidia_module_and_nouveau_is_unloaded_before_the_reset(self):
        _, calls = self.settle("1")
        reset = calls.index("reset 0000:00:08.0")
        for module in ("nvidia_uvm", "nvidia_drm", "nvidia_modeset", "nvidia", "nouveau"):
            self.assertIn("rmmod " + module, calls[:reset])

    def test_the_reset_is_a_sysfs_write_of_one(self):
        # No logging override here: the real write must hit <sysfs>/<bdf>/reset.
        with tempfile.TemporaryDirectory() as d:
            fake = Fake(d)
            with_gpu(fake)
            fake.set("smi", "bad")
            fake.set("healthy_after_resets", "never")
            fake.run(block(DRIVER, "GPU_SETTLE") + "\ngpu_settle\n", {"KF_RECOVER_TRIES": "1"})
            self.assertEqual((fake.sysfs / "0000:00:08.0" / "reset").read_text(), "1\n")
            self.assertEqual((fake.sysfs / "0000:00:07.0" / "reset").read_text(), "")
            self.assertEqual((fake.sysfs / "0000:00:03.0" / "reset").read_text(), "")

    def test_recovery_is_bounded_and_the_verdict_is_eight(self):
        out, calls = self.settle("never", tries=3)
        self.assertIn("RC=8", out)
        self.assertEqual(sum(c.startswith("reset ") for c in calls), 3)

    def test_the_try_count_is_honoured(self):
        _, calls = self.settle("never", tries=1)
        self.assertEqual(sum(c.startswith("reset ") for c in calls), 1)

    def test_default_is_three_tries(self):
        _, calls = self.settle("never")
        self.assertEqual(sum(c.startswith("reset ") for c in calls), 3)

    def test_a_late_recovery_stops_retrying(self):
        out, calls = self.settle("2", tries=3)
        self.assertIn("RC=0", out)
        self.assertEqual(sum(c.startswith("reset ") for c in calls), 2)

    def test_failure_prints_the_smi_error_and_the_last_twenty_nvrm_lines(self):
        lines = "".join("[%d.0] NVRM: line %d\n" % (i, i) for i in range(1, 41))
        out, _ = self.settle("never", tries=1, dmesg=lines)
        self.assertIn(SMI_BAD, out)
        self.assertIn("NVRM: line 40", out)
        self.assertIn("NVRM: line 21", out)
        self.assertNotIn("NVRM: line 20\n", out)
        self.assertLessEqual(len(re.findall(r"NVRM: line \d+", out)), 20)

    def test_only_nvrm_lines_are_shown_and_each_is_cut(self):
        noise = "[1.0] usb 1-1: new device\n" * 30
        long = "[2.0] NVRM: " + "x" * 900 + "\n"
        out, _ = self.settle("never", tries=1, dmesg=noise + long)
        self.assertNotIn("usb 1-1", out)
        self.assertLessEqual(max(len(line) for line in out.splitlines()), 300)

    def test_success_prints_no_failure_diagnostics(self):
        lines = "[1.0] NVRM: some line\n"
        out, _ = self.settle("1", dmesg=lines)
        self.assertNotIn("NVRM: some line", out)

    def test_the_reload_pokes_the_device_nodes(self):
        _, calls = self.settle("1")
        self.assertIn("nvidia-modprobe -c 0 -u", calls)


class SettleCall(unittest.TestCase):
    """The tail of provision_host_driver.sh: a dead GPU is rc 8, never `rc=0`."""

    def tail(self, settle_rc):
        with tempfile.TemporaryDirectory() as d:
            fake = Fake(d)
            script = (
                "gpu_settle() { echo SETTLE_RAN; return %s; }\n" % settle_rc
                + "( "
                + block(DRIVER, "GPU_SETTLE_CALL")
                + "\n ); echo EXIT_STATUS=$?\n"
            )
            return fake.run(script)[1]

    def test_a_dead_gpu_ends_the_swap_with_rc_eight(self):
        out = self.tail(8)
        self.assertRegex(out, r"(?m)^DRIVER_SWAP_DONE rc=8\b")
        self.assertIn("EXIT_STATUS=8", out)

    def test_a_dead_gpu_never_prints_rc_zero(self):
        self.assertNotRegex(self.tail(8), r"DRIVER_SWAP_DONE rc=0")

    def test_a_working_gpu_ends_with_rc_zero(self):
        out = self.tail(0)
        self.assertRegex(out, r"(?m)^DRIVER_SWAP_DONE rc=0 ")
        self.assertIn("EXIT_STATUS=0", out)

    def test_the_settle_runs_before_the_device_node_probe(self):
        text = DRIVER.read_text()
        call = text.find("# KF_GPU_SETTLE_CALL_BEGIN")
        probe = text.find("nvidia-modprobe -c 0 -u >/dev/null")
        self.assertGreater(call, probe)
        self.assertGreater(probe, 0)


class DriverGate(unittest.TestCase):
    """provision_full.sh stops before the 15-minute builds when the swap left a dead GPU."""

    def gate(self, driver_rc):
        with tempfile.TemporaryDirectory() as d:
            fake = Fake(d)
            prov = fake.root / "prov"
            prov.mkdir()
            script = (
                "DRIVER_RC=%s\n" % driver_rc
                + block(FULL, "NOT_READY_DIAG")
                + block(FULL, "DRIVER_GATE").replace("/root/prov", str(prov))
                + "\necho REACHED_THE_BUILDS\n"
            )
            fake.set("dmesg", "[1.0] NVRM: GPU is in a bad state\n")
            fake.set("smi", "bad")
            fake.set("healthy_after_resets", "never")
            rc, out = fake.run(script)
            return rc, out, prov

    def test_rc_eight_stops_before_the_builds(self):
        rc, out, _ = self.gate(8)
        self.assertEqual(rc, 1)
        self.assertNotIn("REACHED_THE_BUILDS", out)

    def test_rc_eight_prints_a_not_ready_line_with_the_reason(self):
        _, out, _ = self.gate(8)
        self.assertRegex(out, r"(?m)^NOT_READY driver_rc=8\b")
        self.assertIn(SMI_BAD, out)

    def test_rc_eight_never_leaves_a_ready_marker(self):
        _, _, prov = self.gate(8)
        self.assertFalse((prov / "READY").exists())

    def test_rc_zero_goes_on_to_the_builds(self):
        rc, out, _ = self.gate(0)
        self.assertEqual(rc, 0)
        self.assertIn("REACHED_THE_BUILDS", out)
        self.assertNotIn("NOT_READY", out)

    def test_other_driver_failures_keep_their_old_behaviour(self):
        # A stock driver can still be READY after e.g. a failed download (rc 3).
        for code in (3, 4, 7):
            rc, out, _ = self.gate(code)
            self.assertIn("REACHED_THE_BUILDS", out, code)

    def test_the_gate_sits_between_the_driver_step_and_the_tree_build(self):
        text = FULL.read_text()
        gate = text.find("# KF_DRIVER_GATE_BEGIN")
        self.assertGreater(gate, text.find('echo "DRIVER_RC=$?"'))
        self.assertLess(gate, text.find("provision_bench_tree.sh"))


class ReadyCheck(unittest.TestCase):
    """A NOT_READY verdict explains itself: the nvidia-smi text and the last NVRM lines."""

    def verdict(self, smi="bad", kf3="0", fg="0", dmesg="", initrd=True):
        with tempfile.TemporaryDirectory() as d:
            fake = Fake(d)
            prov = fake.root / "prov"
            prov.mkdir()
            fake.set("smi", smi)
            fake.set("healthy_after_resets", "never")
            fake.set("dmesg", dmesg)
            fastguest = fake.root / "fastguest"
            fastguest.mkdir()
            if initrd:
                (fastguest / "initrd.cpio.gz").write_text("x")
            script = (
                "KF3_RC=%s FG_RC=%s\n" % (kf3, fg)
                + block(FULL, "NOT_READY_DIAG")
                + block(FULL, "READY_CHECK")
                .replace("/root/prov", str(prov))
                .replace("/workspace/bench/fastguest", str(fastguest))
                + "\necho AFTER_BLOCK\n"
            )
            rc, out = fake.run(script)
            return out, prov

    def test_ready_when_driver_and_builds_are_right(self):
        out, prov = self.verdict(smi="ok")
        self.assertIn("READY driver=580.159.04", out)
        self.assertTrue((prov / "READY").exists())

    def test_ready_prints_no_diagnostics(self):
        out, _ = self.verdict(smi="ok", dmesg="[1.0] NVRM: noise\n")
        self.assertNotIn("NVRM", out)

    def test_not_ready_keeps_the_machine_readable_line(self):
        out, prov = self.verdict()
        self.assertRegex(
            out, r"(?m)^NOT_READY driver=No devices were found KF3_RC=0 FG_RC=0$"
        )
        self.assertFalse((prov / "READY").exists())

    def test_not_ready_prints_the_nvidia_smi_status(self):
        out, _ = self.verdict()
        self.assertRegex(out, r"(?m)^NVIDIA_SMI_RC=6$")

    def test_not_ready_prints_the_last_twenty_nvrm_lines_only(self):
        lines = "".join("[%d.0] NVRM: line %d\n" % (i, i) for i in range(1, 61))
        out, _ = self.verdict(dmesg=lines)
        self.assertIn("NVRM: line 60", out)
        self.assertIn("NVRM: line 41", out)
        self.assertNotIn("NVRM: line 40\n", out)
        self.assertEqual(len(re.findall(r"NVRM: line \d+", out)), 20)

    def test_not_ready_cuts_long_dmesg_lines(self):
        out, _ = self.verdict(dmesg="[1.0] NVRM: " + "y" * 5000 + "\n")
        self.assertLessEqual(max(len(line) for line in out.splitlines()), 300)

    def test_not_ready_for_a_failed_build_with_a_good_driver_is_still_not_ready(self):
        out, prov = self.verdict(smi="ok", kf3="2")
        self.assertRegex(out, r"(?m)^NOT_READY driver=580\.159\.04 KF3_RC=2 FG_RC=0$")
        self.assertFalse((prov / "READY").exists())

    def test_a_missing_initrd_is_not_ready(self):
        out, _ = self.verdict(smi="ok", initrd=False)
        self.assertIn("NOT_READY", out)


if __name__ == "__main__":
    unittest.main()
