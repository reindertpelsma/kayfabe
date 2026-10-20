#!/usr/bin/env python3
"""GPU-free checks of desktop_baton.sh / desktop_baton_guest.sh: syntax, `plan` output (touches
nothing), and static safety rules (no SIGKILL, no pkill of QEMU, owner restore recipe intact)."""
import os
import re
import subprocess
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))


def text(name):
    return open(os.path.join(HERE, name)).read()


class Scripts(unittest.TestCase):
    def test_bash_syntax(self):
        for n in ("desktop_baton.sh", "desktop_baton_guest.sh"):
            subprocess.run(["bash", "-n", os.path.join(HERE, n)], check=True)

    def plan(self, **env):
        e = dict(os.environ, **env)
        r = subprocess.run(["bash", os.path.join(HERE, "desktop_baton.sh"), "plan"], env=e,
                           capture_output=True, text=True, timeout=30)
        self.assertEqual(r.returncode, 0, r.stderr)
        return r.stdout

    def test_plan_names_the_required_flags(self):
        out = self.plan()
        for needle in ("display=on", "display-broker=", "gop=on", "guest-driver=580.159.04",
                       "/data/claude-cursor6/nvkvm-display-broker", "KF3_TSG_MIRROR=0",
                       "touch /tmp/kf-stop-claude-manual", "flock -w 90 -o /tmp/kayfabe-fastguest.lock",
                       "REV=0d7ba743", "solo:30 pair:300 canary:300"):
            self.assertIn(needle, out)

    def test_plan_honours_arm_and_extra_flags(self):
        out = self.plan(ARM="plain", KF3_FLAGS="KF3_DEBUG_FORCE_MIGRATABLE=1")
        self.assertIn("env: KF3_DEBUG_FORCE_MIGRATABLE=1", out)
        self.assertNotIn("KF3_TSG_MIRROR=0 ", out.split("guest image")[0])

    def test_bad_arm_refused(self):
        r = subprocess.run(["bash", os.path.join(HERE, "desktop_baton.sh"), "plan"],
                           env=dict(os.environ, ARM="nope"), capture_output=True, text=True)
        self.assertNotEqual(r.returncode, 0)

    def test_no_sigkill_and_no_qemu_pkill(self):
        t = text("desktop_baton.sh") + text("desktop_baton_guest.sh")
        code = "\n".join(l for l in t.splitlines() if not l.lstrip().startswith("#"))
        self.assertIsNone(re.search(r"kill\s+-(9|KILL)|kill\s+-s\s+KILL", code))
        self.assertIsNone(re.search(r"pkill|killall", code))

    def test_owner_restore_recipe_matches_campaign_restore(self):
        t = text("desktop_baton.sh")
        self.assertIn('rm -f "$OWNER_STOP"', t)
        self.assertIn('flock -w 90 -o "$LOCK" bash "$OWNER_LAUNCHER"', t)
        self.assertIn("OWNER_SESSION_READY", t)
        self.assertIn("max(3094", t)

    def test_unknown_subcommand(self):
        r = subprocess.run(["bash", os.path.join(HERE, "desktop_baton.sh"), "frob"], capture_output=True, text=True)
        self.assertEqual(r.returncode, 2)


if __name__ == "__main__":
    unittest.main()
