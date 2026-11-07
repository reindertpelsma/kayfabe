# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""WinVmVm (winapps.py): the win_vm.sh controller of the Windows apps stage, against a fake win_vm.sh (no QEMU, no GPU)."""
import os
import signal
import stat
import subprocess
import tempfile
import time
import unittest

import helpers as H  # noqa: F401
import winapps

FAKE = r'''#!/bin/bash
# a fake win_vm.sh: records its argv and environment, then plays the parts the controller waits for
VM=$WINVM_ROOT/${WINVM_NAME:-kfwin}
echo "$* | OVERLAY_BACKING=${WINVM_OVERLAY_BACKING:-} APPS_ISO=${WINVM_APPS_ISO:-}" >> "$FAKE_LOG"
case "$1" in
  run)
    tag=""
    while [ $# -gt 0 ]; do [ "$1" = --tag ] && tag=$2; shift; done
    mkdir -p "$VM/logs" "$VM/run"
    if [ -e "$FAKE_REFUSE" ]; then echo "[t] REFUSED: the GPU lock was not free in 120s" > "$VM/logs/${tag}_run.log"; exit 0; fi
    if [ ! -e "$FAKE_NOSTART" ]; then
      ( sleep 300 ) > /dev/null 2>&1 &
      echo $! > "$VM/run/qemu.pid"
      echo "[t] QEMU_START tag=$tag boot=1 pid=$! log=logs/${tag}_b1_qemu.log" > "$VM/logs/${tag}_run.log"
      printf 'GSP phase Running -> Suspending\nx\nGSP phase Running -> Suspending\n' > "$VM/logs/${tag}_b1_qemu.log"
    else
      echo "[t] WINVM_RUN_START tag=$tag" > "$VM/logs/${tag}_run.log"
    fi ;;
  stop)
    pid=$(cat "$VM/run/qemu.pid" 2>/dev/null) && kill "$pid" 2>/dev/null
    for f in "$VM"/logs/*_run.log; do echo "[t] WINVM_RUN_END tag=$(basename "$f" _run.log) boots=1 reason=x" >> "$f"; done ;;
  shot)
    mkdir -p "$VM/logs/shots"; printf 'PNG' > "$VM/logs/shots/$2.png"; echo "$VM/logs/shots/$2.png" ;;
esac
exit 0
'''
UUID = "0123abcd-0123-4abc-8abc-0123456789ab"


class WinVmVmTests(unittest.TestCase):
    def setUp(self):
        self.d = tempfile.mkdtemp()
        self.addCleanup(lambda: __import__("shutil").rmtree(self.d, ignore_errors=True))
        self.root = os.path.join(self.d, "root")
        self.vm = os.path.join(self.root, "kfwin")
        os.makedirs(os.path.join(self.vm, "secrets"))
        open(os.path.join(self.vm, "identity.env"), "w").write(f"VM_UUID={UUID}\nVM_SERIAL=x\n")
        self.script = os.path.join(self.d, "win_vm.sh")
        open(self.script, "w").write(FAKE)
        os.chmod(self.script, os.stat(self.script).st_mode | stat.S_IXUSR)
        self.log = os.path.join(self.d, "fake.log")
        for k, v in (("FAKE_LOG", self.log), ("FAKE_REFUSE", os.path.join(self.d, "refuse")), ("FAKE_NOSTART", os.path.join(self.d, "nostart"))):
            os.environ[k] = v
        self.lines = []
        self.v = winapps.WinVmVm(self.script, self.root, "kfwin", "gate_wa1", "/x/kfapps.iso", "/x/flags.env", "580.65.06", "B",
                                 self.lines.append, ssh_port=2299, boot_wait=6)
        self.addCleanup(self.cleanup)

    def cleanup(self):
        pid = self.v._pid()
        if pid:
            try:
                os.kill(pid, signal.SIGKILL)
            except OSError:
                pass

    def test_launch_passes_the_overlay_chain_the_disk_the_identity_and_the_flags(self):
        self.v.launch("g1")
        t = open(self.log).read()
        self.assertIn("run --detach --overlay apps_g1_", t)
        self.assertIn("--arm B --guest-driver 580.65.06", t)
        self.assertIn(f"--kf3-extra gpu-uuid=GPU-{UUID}", t)
        self.assertIn("--env-file /x/flags.env", t)
        self.assertIn("OVERLAY_BACKING=gate_wa1 APPS_ISO=/x/kfapps.iso", t)
        self.assertTrue(self.v.alive())

    def test_gsp_cycles_and_screendump_and_stop(self):
        self.v.launch("g2")
        self.assertEqual(self.v.gsp_cycles(), 2)
        png = os.path.join(self.d, "shots", "a-end.png")
        self.assertTrue(self.v.screendump(png))
        self.assertTrue(os.path.exists(png))
        how = self.v.stop()
        self.assertTrue(how.startswith("clean"))
        time.sleep(0.2)
        self.assertFalse(self.v.alive())

    def test_a_refused_start_is_an_error_not_a_guest(self):
        open(os.environ["FAKE_REFUSE"], "w").close()
        with self.assertRaises(RuntimeError) as e:
            self.v.launch("g3")
        self.assertIn("ended before QEMU started", str(e.exception))

    def test_a_loop_that_never_starts_qemu_is_an_error(self):
        open(os.environ["FAKE_NOSTART"], "w").close()
        v = winapps.WinVmVm(self.script, self.root, "kfwin", "gate_wa1", "/x/i.iso", "/x/f.env", "580.65.06", "B", self.lines.append, boot_wait=3)
        with self.assertRaises(RuntimeError):
            v.launch("g4")

    def test_no_identity_is_an_error_and_no_sign_in_keystrokes_exist(self):
        os.unlink(os.path.join(self.vm, "identity.env"))
        with self.assertRaises(RuntimeError):
            self.v.launch("g5")
        with self.assertRaises(NotImplementedError):
            self.v.send_key("a")

    def test_the_guest_transport_is_ssh_with_the_vms_own_key(self):
        g = self.v.guest(5.0)
        try:
            self.assertEqual(g.port, 2299)
            self.assertTrue(g.key.endswith("kfwin/secrets/ssh_ed25519"))
            self.assertEqual(g.user, "kf")
        finally:
            g.cleanup()


if __name__ == "__main__":
    unittest.main()
