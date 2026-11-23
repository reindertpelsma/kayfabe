#!/usr/bin/env python3
"""GPU-free unit tests of the safe bind/restore logic in pcsession.py (owner rule 2026-11-17): a module in use is an ABORT, nothing ever
writes to a sysfs unbind file, nothing is unloaded before nvidia_drm is quiet, no session starts within 60 s of a restore.
run: python3 -B -m unittest test_pcsession_bind -v   (in the bench dir)"""
import os, pathlib, re, sys, tempfile, unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import pcsession as P


class Fake:
    def __init__(self, rc=None):
        self.cmds = []
        self.rc = rc or {}

    def sh(self, cmd, timeout):
        self.cmds.append(list(cmd))
        key = " ".join(cmd)
        for k, v in self.rc.items():
            if k in key:
                return v
        return 0, ""

    def text(self):
        return "\n".join(" ".join(c) for c in self.cmds)


def refcnt_factory(loaded=("nvidia_uvm", "nvidia_drm", "nvidia_modeset", "nvidia"), drm=0):
    def r(name):
        if name == "nvidia_drm" and name in loaded:
            return drm
        return 1 if name in loaded else None
    return r


class Bind(unittest.TestCase):
    def setUp(self):
        self.old = P.DRM_QUIET_S

    def tearDown(self):
        P.DRM_QUIET_S = self.old

    def run_bind(self, fake, holders=lambda: ([], ""), refcnt=None, own=lambda p: False):
        return P.bind_vfio(sh=fake.sh, holders=holders, own=own, refcnt=refcnt or refcnt_factory(), sleep=lambda s: None, driver_of=lambda d: None)

    def test_module_in_use_aborts_and_never_unbinds(self):
        f = Fake({"rmmod nvidia_drm": (1, "rmmod: ERROR: Module nvidia_drm is in use")})
        with self.assertRaises(P.Abort) as cm:
            self.run_bind(f)
        self.assertIn("module in use", str(cm.exception))
        self.assertNotIn("unbind", f.text())
        self.assertNotIn("driver_override", f.text())          # never reaches the vfio hand-over
        self.assertEqual([c for c in f.cmds if c[0] == "rmmod"], [["rmmod", "nvidia_uvm"], ["rmmod", "nvidia_drm"]])   # stops at the first failure, no retry

    def test_rmmod_timeout_aborts_no_retry(self):
        f = Fake({"rmmod nvidia_uvm": ("TIMEOUT", "timeout after 90s")})
        with self.assertRaises(P.Abort):
            self.run_bind(f)
        self.assertEqual(len([c for c in f.cmds if c[:2] == ["rmmod", "nvidia_uvm"]]), 1)
        self.assertNotIn("unbind", f.text())

    def test_nvidia_drm_refcount_nonzero_aborts_before_any_unload(self):
        P.DRM_QUIET_S = 0
        f = Fake()
        with self.assertRaises(P.Abort) as cm:
            self.run_bind(f, refcnt=refcnt_factory(drm=2))
        self.assertIn("nvidia_drm refcnt 2", str(cm.exception))
        self.assertEqual([c for c in f.cmds if c[0] == "rmmod"], [])
        self.assertNotIn("unbind", f.text())

    def test_foreign_drm_client_aborts_before_any_unload(self):
        f = Fake()
        with self.assertRaises(P.Abort) as cm:
            self.run_bind(f, holders=lambda: ([4242], "gnome-shell"), own=lambda p: False)
        self.assertIn("not ours", str(cm.exception))
        self.assertEqual([c for c in f.cmds if c[0] == "rmmod"], [])

    def test_clean_path_uses_rmmod_then_driver_override_only(self):
        f = Fake()
        try:
            self.run_bind(f)
        except P.Abort:
            pass          # on a host without the PCI device the final driver checks may abort; the commands up to there are what matters
        t = f.text()
        self.assertIn("rmmod nvidia_uvm", t)
        self.assertLess(t.index("rmmod nvidia_uvm"), t.index("rmmod nvidia ") if "rmmod nvidia " in t + " " else len(t))
        self.assertNotIn("unbind", t)

    def test_restore_never_unbinds(self):
        f = Fake({"nvidia-smi": (0, "NVIDIA GeForce RTX 4070, 595.91.07")})
        old = P.LAST_RESTORE
        with tempfile.TemporaryDirectory() as td:
            P.LAST_RESTORE = pathlib.Path(td) / "last"
            try:
                P.restore_vfio(sh=f.sh, sleep=lambda s: None, driver_of=lambda d: "snd_hda_intel", settle=False)
                self.assertTrue(P.LAST_RESTORE.exists())
            finally:
                P.LAST_RESTORE = old
        self.assertNotIn("unbind", f.text())
        self.assertIn("rmmod vfio_pci", f.text())

    def test_restore_aborts_when_nvidia_smi_does_not_answer(self):
        f = Fake({"nvidia-smi": (9, "failed")})
        with self.assertRaises(P.Abort):
            P.restore_vfio(sh=f.sh, sleep=lambda s: None, driver_of=lambda d: "snd_hda_intel", settle=False)


class Settle(unittest.TestCase):
    def test_waits_until_60s_after_the_restore(self):
        with tempfile.TemporaryDirectory() as td:
            f = pathlib.Path(td) / "last"
            f.write_text("1000\n")
            slept = []
            w = P.settle_after_restore(log=lambda m: None, sleep=slept.append, now_fn=lambda: 1030, path=f)
            self.assertEqual(slept, [30])
            self.assertEqual(w, 30)
            self.assertEqual(P.settle_after_restore(log=lambda m: None, sleep=slept.append, now_fn=lambda: 1100, path=f), 0)
            self.assertEqual(P.settle_after_restore(log=lambda m: None, sleep=slept.append, now_fn=lambda: 1100, path=f.with_name("none")), 0)


class Audio(unittest.TestCase):
    """2026-11-18 exception (coordinator conditions): only the audio function, only through snd_hda_intel, FIRST, abort/wedge-lite on failure."""
    def bind(self, f, driver_of, refcnt=None):
        return P.bind_vfio(sh=f.sh, holders=lambda: ([], ""), own=lambda p: False, refcnt=refcnt or refcnt_factory(), sleep=lambda s: None, driver_of=driver_of)

    def test_audio_released_first_through_snd_hda_intel_only(self):
        f = Fake()
        state = {"0000:01:00.1": "snd_hda_intel"}
        orig = f.sh
        def sh(cmd, timeout):
            r = orig(cmd, timeout)
            if "snd_hda_intel/unbind" in " ".join(cmd):
                state.pop("0000:01:00.1")
            return r
        f.sh = sh
        try:
            self.bind(f, lambda d: state.get(d))
        except P.Abort:
            pass     # the final realpath check needs the real device
        t = f.text()
        unb = [c for c in f.cmds if "unbind" in " ".join(c)]
        self.assertEqual(len(unb), 1)
        self.assertIn("/sys/bus/pci/drivers/snd_hda_intel/unbind", unb[0][-1])
        self.assertLess(t.index("snd_hda_intel/unbind"), t.index("rmmod nvidia_uvm"))       # BEFORE any nvidia unload

    def test_audio_unbind_failure_aborts_before_any_unload_no_retry(self):
        f = Fake({"snd_hda_intel/unbind": (1, "fail")})
        with self.assertRaises(P.Abort) as cm:
            self.bind(f, lambda d: "snd_hda_intel" if d == "0000:01:00.1" else None)
        self.assertNotIsInstance(cm.exception, P.WedgeLite)
        self.assertEqual([c for c in f.cmds if c[0] == "rmmod"], [])
        self.assertEqual(len([c for c in f.cmds if "unbind" in " ".join(c)]), 1)

    def test_audio_unbind_child_in_D_state_is_wedge_lite_and_nvidia_untouched(self):
        f = Fake({"snd_hda_intel/unbind": ("TIMEOUT", "timeout after 60s: bash -c ... (pid 77 state D)")})
        with self.assertRaises(P.WedgeLite):
            self.bind(f, lambda d: "snd_hda_intel" if d == "0000:01:00.1" else None)
        self.assertEqual([c for c in f.cmds if c[0] in ("rmmod", "modprobe")], [])

    def test_nvidia_function_with_a_driver_is_never_unbound(self):
        f = Fake()
        with self.assertRaises(P.Abort) as cm:
            self.bind(f, lambda d: "nvidia" if d == "0000:01:00.0" else None)
        self.assertIn("still has driver nvidia", str(cm.exception))
        self.assertEqual([c for c in f.cmds if "unbind" in " ".join(c)], [])

    def test_restore_binds_audio_back_when_driverless(self):
        f = Fake({"nvidia-smi": (0, "NVIDIA GeForce RTX 4070, 595.91.07")})
        old = P.LAST_RESTORE
        with tempfile.TemporaryDirectory() as td:
            P.LAST_RESTORE = pathlib.Path(td) / "last"
            try:
                P.restore_vfio(sh=f.sh, sleep=lambda s: None, driver_of=lambda d: None, settle=False)
            finally:
                P.LAST_RESTORE = old
        self.assertIn("snd_hda_intel/bind", f.text())
        self.assertNotIn("unbind", f.text())
        t = f.text()
        self.assertLess(t.index("modprobe nvidia_uvm"), t.index("snd_hda_intel/bind"))        # GPU side first

    def test_restore_does_not_touch_audio_when_already_bound(self):
        f = Fake({"nvidia-smi": (0, "x")})
        old = P.LAST_RESTORE
        with tempfile.TemporaryDirectory() as td:
            P.LAST_RESTORE = pathlib.Path(td) / "last"
            try:
                P.restore_vfio(sh=f.sh, sleep=lambda s: None, driver_of=lambda d: "snd_hda_intel", settle=False)
            finally:
                P.LAST_RESTORE = old
        self.assertNotIn("snd_hda_intel/bind", f.text())


class FakeClock:
    def __init__(self):
        self.t = 0.0
    def now(self):
        return self.t
    def sleep(self, s):
        self.t += s


class UvmSettle(unittest.TestCase):
    """2026-11-18: udev rule 71-nvidia.rules line 19 runs `modprobe -r nvidia-uvm` late after a restore; the settle must outwait it and call a 'going' module a wedge."""
    def run_settle(self, states, procs=lambda: [], f=None, **kw):
        c = FakeClock(); f = f or Fake()
        def state_fn():
            for until, st in states:
                if c.t < until:
                    return st
            return states[-1][1]
        out = P.wait_host_modules_settled(sh=f.sh, sleep=c.sleep, now_fn=c.now, state_fn=state_fn, procs_fn=procs, log=[], **kw)
        return out, c, f

    def test_live_and_stable_returns_after_60s_and_runs_udevadm_settle_first(self):
        out, c, f = self.run_settle([(1e9, ("live", 0))])
        self.assertGreaterEqual(c.t, 60)
        self.assertEqual(f.cmds[0][:2], ["udevadm", "settle"])
        self.assertIn("settled", out)

    def test_a_running_nvidia_modprobe_resets_the_stability_clock(self):
        procs = lambda: ["1 /sbin/modprobe -r nvidia-uvm"] if self_c.t < 50 else []
        self_c = FakeClock()
        c = self_c; f = Fake()
        P.wait_host_modules_settled(sh=f.sh, sleep=c.sleep, now_fn=c.now, state_fn=lambda: ("live", 0), procs_fn=procs, log=[])
        self.assertGreaterEqual(c.t, 110)           # 50 s of the running modprobe + 60 s stable

    def test_late_udev_unload_then_absent_stable_loads_uvm_once_and_settles(self):
        c = FakeClock(); f = Fake(); st = {"s": ("live", 0)}
        def state_fn():
            if c.t >= 20 and not any("modprobe nvidia_uvm" in " ".join(x) for x in f.cmds):
                return (None, None)
            return ("live", 0)
        out = P.wait_host_modules_settled(sh=f.sh, sleep=c.sleep, now_fn=c.now, state_fn=state_fn, procs_fn=lambda: [], log=[])
        self.assertEqual(len([x for x in f.cmds if x == ["modprobe", "nvidia_uvm"]]), 1)
        self.assertNotIn("rmmod", f.text())
        self.assertIn("settled", out)

    def test_going_for_more_than_120s_is_a_wedge_no_unload_attempt(self):
        c = FakeClock(); f = Fake()
        with self.assertRaises(P.UvmWedge) as cm:
            P.wait_host_modules_settled(sh=f.sh, sleep=c.sleep, now_fn=c.now, state_fn=lambda: ("going", -1), procs_fn=lambda: ["9 /sbin/modprobe -r nvidia-uvm"], log=[])
        self.assertIn("SysRq", str(cm.exception))
        self.assertLessEqual(c.t, 130)
        self.assertEqual([x for x in f.cmds if x[0] in ("rmmod", "modprobe")], [])       # never a second unload/load attempt

    def test_going_that_clears_within_the_limit_is_not_a_wedge(self):
        out, c, f = self.run_settle([(60, ("going", -1)), (1e9, ("live", 0))])
        self.assertIn("settled", out)

    def test_restore_runs_the_settle_after_nvidia_smi_and_a_wedge_propagates(self):
        f = Fake({"nvidia-smi": (0, "x")})
        old = P.LAST_RESTORE; old_w = P.wait_host_modules_settled
        def boom(**k):
            raise P.UvmWedge("wedge")
        P.wait_host_modules_settled = boom
        with tempfile.TemporaryDirectory() as td:
            P.LAST_RESTORE = pathlib.Path(td) / "last"
            try:
                with self.assertRaises(P.UvmWedge):
                    P.restore_vfio(sh=f.sh, sleep=lambda s: None, driver_of=lambda d: "snd_hda_intel")
                self.assertFalse(P.LAST_RESTORE.exists())            # no settle stamp after a wedge
            finally:
                P.LAST_RESTORE = old; P.wait_host_modules_settled = old_w


class Recover(unittest.TestCase):
    def test_abort_after_a_module_unload_runs_the_full_restore(self):
        f = Fake(); msgs = []
        self.assertTrue(P.recover_after_bind_abort(P.Abort("x"), sh=f.sh, refcnt=lambda n: None, driver_of=lambda d: None, active_fn=msgs.append))
        self.assertEqual(f.cmds, [])          # the restore itself is the caller's restore_vfio

    def test_abort_before_any_unload_restarts_services_and_reprobes_audio(self):
        f = Fake(); msgs = []
        self.assertFalse(P.recover_after_bind_abort(P.Abort("x"), sh=f.sh, refcnt=lambda n: 3, driver_of=lambda d: "snd_hda_intel", active_fn=msgs.append))
        t = f.text()
        self.assertIn("drivers_probe", t); self.assertIn("start gdm", t); self.assertIn("start nvidia-persistenced", t)
        self.assertNotIn("unbind", t); self.assertNotIn("rmmod", t)

    def test_wedge_lite_only_starts_gdm(self):
        f = Fake(); msgs = []
        self.assertFalse(P.recover_after_bind_abort(P.WedgeLite("x"), sh=f.sh, refcnt=lambda n: 3, driver_of=lambda d: None, active_fn=msgs.append))
        self.assertEqual(f.cmds, [["systemctl", "start", "gdm"]])
        self.assertIn("WEDGE-LITE", msgs[0])


class Source(unittest.TestCase):
    def test_only_the_audio_unbind_exists_in_the_session_driver(self):
        src = pathlib.Path(P.__file__).read_text()
        for m in re.finditer(r"[^\n]*/unbind[^\n]*", src):
            line = m.group(0)
            self.assertTrue("snd_hda_intel" in line or line.lstrip().startswith("#") or "never" in line or "NEVER" in line, line)
            self.assertNotIn("drivers/nvidia/unbind", line)
        self.assertNotIn("driver/unbind", src.replace("# unbind file", ""))


if __name__ == "__main__":
    unittest.main()
