#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free tests of the pc's safe VFIO bind (scripts/bench/pc_bind.py), its guards (pc_guard.py) and the tick probe
(pc_safe_probe.py).

Falsifiers, stated before the guards were written (each fails on the unguarded bind of pcsession.py, see the PR):
- Guard (a): a bind that unloads nvidia before a modprobe block for all four nvidia modules is in place, or that goes on
  after the hand-over while the nvidia module is loaded, or that leaves the block behind on an abort before any unload.
- Restore: a restore that leaves vfio_iommu_type1 or vfio loaded (the first guarded restore on the pc did).
- Guard (b): a restore that runs `modprobe nvidia` while the udev queue is not empty or an nvidia modprobe runs; a restore
  that times out without restarting systemd-udevd; a restore that loads the driver after the ladder failed; a settle that
  calls a timed-out `udevadm settle` settled (the restore of 2026-11-24 logged exactly that).
- Probe: an nvidia tool planned or run while a bind may be active.

The safety invariants of the earlier rewrite (no NVIDIA unbind write, abort on a module in use, no retry) are kept as tests.
"""
import pathlib
import re
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1] / "bench"))
import pc_bind as P  # noqa: E402
import pc_guard as G  # noqa: E402
import pc_safe_probe as S  # noqa: E402


class Fake:
    def __init__(self, rc=None, on_cmd=None):
        self.cmds = []
        self.rc = rc or {}
        self.on_cmd = on_cmd

    def sh(self, cmd, timeout):
        self.cmds.append(list(cmd))
        if self.on_cmd:
            self.on_cmd(list(cmd))
        key = " ".join(cmd)
        for k, v in self.rc.items():
            if k in key:
                return v
        return 0, ""

    def text(self):
        return "\n".join(" ".join(c) for c in self.cmds)


class Mods:
    """Loaded-module state that follows rmmod/modprobe of the fake shell."""

    def __init__(self, loaded=("nvidia_uvm", "nvidia_drm", "nvidia_modeset", "nvidia"), drm=0):
        self.loaded = set(loaded)
        self.drm = drm

    def on_cmd(self, cmd):
        if cmd[0] == "rmmod":
            self.loaded.discard(cmd[1])
        elif cmd[0] == "modprobe" and cmd[1].startswith("nvidia"):
            self.loaded.add(cmd[1])

    def refcnt(self, name):
        if name not in self.loaded:
            return None
        return self.drm if name == "nvidia_drm" else 1


class Block:
    def __init__(self):
        self.events = []
        self.present = False

    def install(self):
        self.events.append("install")
        self.present = True
        return "blockfile"

    def remove(self):
        self.events.append("remove")
        had, self.present = self.present, False
        return had


def bind(fake, mods, block, holders=lambda: ([], ""), driver_of=None, own=lambda p: False):
    state = {"0000:01:00.0": None, "0000:01:00.1": None}

    def drv(d):
        return state.get(d)

    orig = fake.sh

    def sh(cmd, timeout):
        r = orig(cmd, timeout)
        m = re.search(r"echo (\S+) > /sys/bus/pci/drivers_probe", " ".join(cmd))
        if m and "vfio-pci" in " ".join(cmd):
            state[m.group(1)] = "vfio-pci"
        return r

    fake.sh = sh
    return P.bind_vfio(sh=fake.sh, holders=holders, own=own, refcnt=mods.refcnt, sleep=lambda s: None,
                       driver_of=driver_of or drv, block_install=block.install, block_remove=block.remove)


class GuardA(unittest.TestCase):
    def test_block_text_blocks_all_four_modules_in_both_spellings(self):
        t = G.block_conf_text()
        self.assertTrue(G.block_is_complete(t))
        for m in ("nvidia", "nvidia-modeset", "nvidia_uvm", "nvidia-drm"):
            self.assertTrue(G.block_is_complete(t, (m,)), m)
        self.assertFalse(G.block_is_complete(t.replace("install nvidia_drm /bin/false\n", "")))
        self.assertFalse(G.block_is_complete(""))

    def test_block_lives_in_run_so_a_reboot_drops_it(self):
        self.assertEqual(str(G.BLOCK_CONF.parent), "/run/modprobe.d")

    def test_install_and_remove_block_file(self):
        with tempfile.TemporaryDirectory() as td:
            p = pathlib.Path(td) / "modprobe.d" / "b.conf"
            G.install_block(p)
            self.assertTrue(G.block_is_complete(p.read_text()))
            self.assertTrue(G.remove_block(p))
            self.assertFalse(G.remove_block(p))

    def test_bind_installs_the_block_before_the_first_unload(self):
        mods, block = Mods(), Block()
        f = Fake(on_cmd=mods.on_cmd)
        order = []
        f.on_cmd = lambda c: (order.append(" ".join(c)), mods.on_cmd(c))
        block_install = block.install
        block.install = lambda: (order.append("BLOCK"), block_install())[1]
        ok, _ = bind(f, mods, block)
        self.assertTrue(ok)
        self.assertTrue(block.present)
        self.assertLess(order.index("BLOCK"), order.index("rmmod nvidia_uvm"))

    def test_bind_refuses_to_go_on_when_nvidia_is_loaded_after_the_hand_over(self):
        mods, block = Mods(), Block()

        def on_cmd(c):
            mods.on_cmd(c)
            if c[:2] == ["modprobe", "vfio-pci"]:
                mods.loaded.add("nvidia")          # something loaded nvidia behind the session's back
        f = Fake(on_cmd=on_cmd)
        with self.assertRaises(P.Abort) as cm:
            bind(f, mods, block)
        self.assertIn("loaded after the hand-over", str(cm.exception))
        self.assertTrue(block.present)             # modules were unloaded: the block stays for the restore

    def test_abort_before_any_unload_removes_the_block(self):
        mods, block = Mods(), Block()
        f = Fake({"rmmod nvidia_uvm": (1, "Module nvidia_uvm is in use")}, on_cmd=mods.on_cmd)
        with self.assertRaises(P.Abort):
            bind(f, mods, block)
        self.assertEqual(block.events, ["install", "remove"])
        self.assertFalse(block.present)

    def test_abort_after_an_unload_keeps_the_block(self):
        mods, block = Mods(), Block()
        f = Fake({"rmmod nvidia_drm": (1, "Module nvidia_drm is in use")}, on_cmd=mods.on_cmd)
        with self.assertRaises(P.Abort):
            bind(f, mods, block)
        self.assertEqual(block.events, ["install"])

    def test_recover_before_any_unload_removes_the_block(self):
        block = Block()
        block.present = True
        f = Fake()
        self.assertFalse(P.recover_after_bind_abort(P.Abort("x"), sh=f.sh, refcnt=lambda n: 3, driver_of=lambda d: "snd_hda_intel",
                                                    active_fn=lambda m: None, block_remove=block.remove))
        self.assertFalse(block.present)


class Clock:
    def __init__(self):
        self.t = 0.0

    def now(self):
        return self.t

    def sleep(self, s):
        self.t += s


class GuardB(unittest.TestCase):
    def test_udev_queue_empty_truth_table(self):
        self.assertTrue(G.udev_queue_empty(0, False))
        self.assertFalse(G.udev_queue_empty(1, False))
        self.assertFalse(G.udev_queue_empty(0, True))
        self.assertFalse(G.udev_queue_empty("TIMEOUT", False))

    def test_nvidia_load_pending(self):
        self.assertTrue(G.nvidia_load_pending(["/sbin/modprobe nvidia-drm"]))
        self.assertTrue(G.nvidia_load_pending(["/sbin/modprobe -r nvidia-uvm"]))
        self.assertTrue(G.nvidia_load_pending(["/usr/bin/nvidia-modprobe -c0 -u"]))
        self.assertTrue(G.nvidia_load_pending(["rmmod nvidia_modeset"]))
        self.assertFalse(G.nvidia_load_pending(["/sbin/modprobe vfio-pci", "bash -c echo nvidia", "/usr/bin/gnome-shell", ""]))

    def gate(self, busy_until, restart_clears=False, sh_log=None):
        c = Clock()
        f = Fake()
        state = {"busy_until": busy_until}

        def on_cmd(cmd):
            if cmd[:3] == ["systemctl", "restart", "systemd-udevd"] and restart_clears:
                state["busy_until"] = 0
        f.on_cmd = on_cmd
        ok, ladder, log = G.restore_gate(f.sh, settle_rc_fn=lambda: 1 if c.t < state["busy_until"] else 0, flag_fn=lambda: False,
                                         procs_fn=lambda: [], sleep=c.sleep, now_fn=c.now, timeout_s=180, ladder_timeout_s=120, stable_s=10)
        return ok, ladder, log, f, c

    def test_gate_waits_for_the_queue_then_passes_without_the_ladder(self):
        ok, ladder, log, f, c = self.gate(busy_until=50)
        self.assertTrue(ok)
        self.assertFalse(ladder)
        self.assertGreaterEqual(c.t, 60)
        self.assertNotIn("systemd-udevd", f.text())

    def test_gate_restarts_udevd_on_timeout_and_passes_when_that_drains_the_queue(self):
        ok, ladder, log, f, c = self.gate(busy_until=1e9, restart_clears=True)
        self.assertTrue(ok)
        self.assertTrue(ladder)
        self.assertEqual(f.cmds, [["systemctl", "restart", "systemd-udevd"]])

    def test_gate_fails_when_the_queue_stays_busy_after_the_ladder(self):
        ok, ladder, log, f, c = self.gate(busy_until=1e9)
        self.assertFalse(ok)
        self.assertTrue(ladder)
        self.assertIn("NOT loaded", log[-1])

    def test_a_pending_nvidia_modprobe_keeps_the_gate_closed(self):
        c = Clock()
        ok, ladder, log = G.restore_gate(Fake().sh, settle_rc_fn=lambda: 0, flag_fn=lambda: False,
                                         procs_fn=lambda: ["/sbin/modprobe nvidia-drm"] if c.t < 40 else [],
                                         sleep=c.sleep, now_fn=c.now, stable_s=10)
        self.assertTrue(ok)
        self.assertGreaterEqual(c.t, 50)

    def restore(self, gate_result, mods=None, block=None):
        mods = mods or Mods(loaded=())
        block = block or Block()
        block.present = True
        order = []
        f = Fake({"nvidia-smi": (0, "NVIDIA GeForce RTX 4070, 595.91.07")})
        f.on_cmd = lambda c: (order.append(" ".join(c)), mods.on_cmd(c))
        rm = block.remove
        block.remove = lambda: (order.append("UNBLOCK"), rm())[1]
        gate = lambda: (order.append("GATE"), gate_result)[1]
        old = P.LAST_RESTORE
        with tempfile.TemporaryDirectory() as td:
            P.LAST_RESTORE = pathlib.Path(td) / "last"
            try:
                out = P.restore_vfio(sh=f.sh, sleep=lambda s: None, driver_of=lambda d: "snd_hda_intel", settle=False, gate=gate,
                                     block_remove=block.remove, refcnt=mods.refcnt)
            finally:
                P.LAST_RESTORE = old
        return out, order, f, block

    def test_restore_gates_then_unblocks_then_loads_nvidia(self):
        out, order, f, block = self.restore((True, False, ["quiet"]))
        self.assertLess(order.index("GATE"), order.index("UNBLOCK"))
        self.assertLess(order.index("UNBLOCK"), order.index("modprobe nvidia"))
        self.assertFalse(block.present)
        self.assertIn("systemctl start gdm", f.text())
        self.assertNotIn("udevadm trigger", f.text())

    def test_restore_does_not_load_nvidia_when_the_gate_fails_and_keeps_the_block(self):
        block = Block()
        with self.assertRaises(P.Abort) as cm:
            self.restore((False, True, ["busy"]), block=block)
        self.assertIn("not quiet", str(cm.exception))
        self.assertTrue(block.present)

    def test_restore_after_the_ladder_retriggers_events_and_restarts_gdm(self):
        out, order, f, block = self.restore((True, True, ["ladder"]))
        t = f.text()
        self.assertIn("udevadm trigger --action=add /sys/bus/pci/devices/0000:01:00.0", t)
        self.assertIn("--subsystem-match=drm", t)
        self.assertIn("systemctl restart gdm", t)
        self.assertLess(t.index("modprobe nvidia_uvm"), t.index("udevadm trigger"))

    def test_restore_unloads_every_vfio_module(self):
        out, order, f, block = self.restore((True, False, []))
        rm = [c[1] for c in f.cmds if c[0] == "rmmod"]
        self.assertEqual(rm, ["vfio_pci", "vfio_pci_core", "vfio_iommu_type1", "vfio"])
        self.assertLess(order.index("rmmod vfio"), order.index("modprobe nvidia"))

    def test_restore_reports_nvidia_loaded_during_the_bind(self):
        out, order, f, block = self.restore((True, False, []), mods=Mods(loaded=("nvidia",)))
        self.assertIn("UNEXPECTED", out)

    def test_settle_never_calls_a_timed_out_udevadm_settle_settled(self):
        c = Clock()
        f = Fake({"udevadm settle": (1, "Timed out while waiting for udev queue to empty.")})
        with self.assertRaises(P.Abort) as cm:
            P.wait_host_modules_settled(sh=f.sh, sleep=c.sleep, now_fn=c.now, state_fn=lambda: ("live", 2), procs_fn=lambda: [], log=[], timeout_s=300)
        self.assertIn("settled=False", str(cm.exception))

    def test_settle_passes_once_udevadm_settle_succeeds(self):
        c = Clock()
        calls = {"n": 0}

        def rc(cmd, timeout):
            if cmd[:2] == ["udevadm", "settle"]:
                calls["n"] += 1
                return (1, "timeout") if calls["n"] < 3 else (0, "")
            return 0, ""
        out = P.wait_host_modules_settled(sh=rc, sleep=c.sleep, now_fn=c.now, state_fn=lambda: ("live", 2), procs_fn=lambda: [], log=[])
        self.assertIn("settled", out)
        self.assertEqual(calls["n"], 3)


ACTIVE_1124 = """\
2026-11-22T03:26:36Z propimage-b2 END candidate: image B2 candidate session rc=0 (overlay KEPT in b2_addon_cand)
2026-11-24T00:54:09Z vfio-s3 START VFIO S3 control (plan A:/usr/bin/qemu-system-x86_64:5): GPU 01:00.0/.1 to vfio-pci, desktop dark until the restore
2026-11-24T00:54:17Z vfio-s3 BOUND: both functions on vfio-pci
2026-11-24T00:54:17Z vfio-s3 BOOT A_b1 qemu=/usr/bin/qemu-system-x86_64 cpu=host,-vmx
2026-11-24T02:14:35Z vfio-s3 RESTORE-FIX udev add/remove ping-pong of the nvidia modules (71-nvidia.rules) after the restore
2026-11-24T02:20:37Z vfio-s3 END VFIO S3 control: 17 cycle records; host GPU: NVIDIA GeForce RTX 4070, 595.91.07
2026-11-24T02:21:30Z vfio-s3 START restore completion (lock re-taken): waiting for the udev backlog to drain, then gdm restart and holder release
2026-11-24T02:28:03Z vfio-s3 END restore completion: nvidia 595.91.07 on 01:00.0, snd_hda_intel on 01:00.1
2026-11-24T02:45:00Z pc-camera-raw NOGPU-INFO START build_kf3 (CPU only, nice), no GPU/VM
""".splitlines()


class BindActive(unittest.TestCase):
    def facts(self, lines, **kw):
        d = dict(active_lines=lines, lock_held=False, block_file_present=False, gpu_driver="nvidia")
        d.update(kw)
        return d

    def test_during_the_s3_session_a_bind_is_active(self):
        active, why = G.bind_active(**self.facts(ACTIVE_1124[:4]))
        self.assertTrue(active)
        self.assertIn("vfio-s3", why[0])

    def test_after_the_session_end_and_a_build_start_no_bind_is_active(self):
        self.assertEqual(G.bind_active(**self.facts(ACTIVE_1124)), (False, []))

    def test_each_host_fact_alone_makes_it_active(self):
        for kw in (dict(lock_held=True), dict(block_file_present=True), dict(gpu_driver="vfio-pci"), dict(gpu_driver=None)):
            self.assertTrue(G.bind_active(**self.facts(ACTIVE_1124, **kw))[0], kw)

    def test_a_kf3_start_does_not_open_a_bind(self):
        lines = ["2026-11-20T01:10:36Z propimage-b2 START offline test of sealed image (restrict=on, kf3, throwaway clone)"]
        self.assertFalse(G.bind_active(**self.facts(lines))[0])


class Probe(unittest.TestCase):
    def run_probe(self, argv, **facts):
        d = dict(active_lines=ACTIVE_1124, lock_held=False, block_file_present=False, gpu_driver="nvidia")
        d.update(facts)
        ran, out = [], []
        rc = S.main(argv, facts=d, runner=lambda c: (ran.append(c), (0, "ok"))[1], out=out.append)
        return rc, ran, "\n".join(out)

    def test_no_nvidia_tool_while_a_bind_may_be_active(self):
        for kw in (dict(lock_held=True), dict(gpu_driver="vfio-pci"), dict(block_file_present=True), dict(active_lines=ACTIVE_1124[:3])):
            rc, ran, out = self.run_probe(["--gpu"], **kw)
            self.assertEqual(rc, 3, kw)
            self.assertEqual(ran, [["uptime"]], kw)
            self.assertIn("REFUSED", out)
            self.assertIn("PC_ACTIVE.log tail", out)

    def test_plan_never_holds_an_nvidia_tool_when_active(self):
        for want in (False, True):
            self.assertFalse(any(S.touches_nvidia(c) for c in S.plan(True, want)))

    def test_gpu_query_runs_only_when_idle_and_asked(self):
        rc, ran, out = self.run_probe(["--gpu"])
        self.assertEqual(rc, 0)
        self.assertEqual(ran[1][0], "nvidia-smi")
        rc, ran, out = self.run_probe([])
        self.assertEqual(ran, [["uptime"]])


def refcnt_factory(loaded=("nvidia_uvm", "nvidia_drm", "nvidia_modeset", "nvidia"), drm=0):
    def r(name):
        if name == "nvidia_drm" and name in loaded:
            return drm
        return 1 if name in loaded else None
    return r


class KeptInvariants(unittest.TestCase):
    """The rules of the earlier safe-bind rewrite, unchanged."""

    def setUp(self):
        self.old = P.DRM_QUIET_S

    def tearDown(self):
        P.DRM_QUIET_S = self.old

    def run_bind(self, fake, holders=lambda: ([], ""), refcnt=None, own=lambda p: False, driver_of=lambda d: None):
        b = Block()
        return P.bind_vfio(sh=fake.sh, holders=holders, own=own, refcnt=refcnt or refcnt_factory(), sleep=lambda s: None,
                           driver_of=driver_of, block_install=b.install, block_remove=b.remove)

    def test_module_in_use_aborts_and_never_unbinds(self):
        f = Fake({"rmmod nvidia_drm": (1, "rmmod: ERROR: Module nvidia_drm is in use")})
        with self.assertRaises(P.Abort) as cm:
            self.run_bind(f)
        self.assertIn("module in use", str(cm.exception))
        self.assertNotIn("unbind", f.text())
        self.assertNotIn("driver_override", f.text())
        self.assertEqual([c for c in f.cmds if c[0] == "rmmod"], [["rmmod", "nvidia_uvm"], ["rmmod", "nvidia_drm"]])

    def test_rmmod_timeout_aborts_no_retry(self):
        f = Fake({"rmmod nvidia_uvm": ("TIMEOUT", "timeout after 90s")})
        with self.assertRaises(P.Abort):
            self.run_bind(f)
        self.assertEqual(len([c for c in f.cmds if c[:2] == ["rmmod", "nvidia_uvm"]]), 1)

    def test_nvidia_drm_refcount_nonzero_aborts_before_any_unload(self):
        P.DRM_QUIET_S = 0
        f = Fake()
        with self.assertRaises(P.Abort) as cm:
            self.run_bind(f, refcnt=refcnt_factory(drm=2))
        self.assertIn("nvidia_drm refcnt 2", str(cm.exception))
        self.assertEqual([c for c in f.cmds if c[0] == "rmmod"], [])

    def test_foreign_drm_client_aborts_before_any_unload(self):
        f = Fake()
        with self.assertRaises(P.Abort) as cm:
            self.run_bind(f, holders=lambda: ([4242], "gnome-shell"))
        self.assertIn("not ours", str(cm.exception))
        self.assertEqual([c for c in f.cmds if c[0] == "rmmod"], [])

    def test_audio_released_first_through_snd_hda_intel_only(self):
        state = {"0000:01:00.1": "snd_hda_intel"}

        def on_cmd(c):
            if "snd_hda_intel/unbind" in " ".join(c):
                state.pop("0000:01:00.1")
        f = Fake(on_cmd=on_cmd)
        try:
            self.run_bind(f, driver_of=lambda d: state.get(d))
        except P.Abort:
            pass
        unb = [c for c in f.cmds if "unbind" in " ".join(c)]
        self.assertEqual(len(unb), 1)
        self.assertIn("/sys/bus/pci/drivers/snd_hda_intel/unbind", unb[0][-1])
        self.assertLess(f.text().index("snd_hda_intel/unbind"), f.text().index("rmmod nvidia_uvm"))

    def test_audio_unbind_child_in_d_state_is_wedge_lite_and_nvidia_untouched(self):
        f = Fake({"snd_hda_intel/unbind": ("TIMEOUT", "timeout after 60s: bash -c ... (pid 77 state D)")})
        with self.assertRaises(P.WedgeLite):
            self.run_bind(f, driver_of=lambda d: "snd_hda_intel" if d == "0000:01:00.1" else None)
        self.assertEqual([c for c in f.cmds if c[0] in ("rmmod", "modprobe")], [])

    def test_nvidia_function_with_a_driver_is_never_unbound(self):
        f = Fake()
        with self.assertRaises(P.Abort) as cm:
            self.run_bind(f, driver_of=lambda d: "nvidia" if d == "0000:01:00.0" else None)
        self.assertIn("still has driver nvidia", str(cm.exception))
        self.assertEqual([c for c in f.cmds if "unbind" in " ".join(c)], [])

    def test_going_for_more_than_120s_is_a_wedge_no_unload_attempt(self):
        c = Clock()
        f = Fake()
        with self.assertRaises(P.UvmWedge):
            P.wait_host_modules_settled(sh=f.sh, sleep=c.sleep, now_fn=c.now, state_fn=lambda: ("going", -1),
                                        procs_fn=lambda: ["9 /sbin/modprobe -r nvidia-uvm"], log=[])
        self.assertEqual([x for x in f.cmds if x[0] in ("rmmod", "modprobe")], [])

    def test_settle_after_restore_waits_60s(self):
        with tempfile.TemporaryDirectory() as td:
            p = pathlib.Path(td) / "last"
            p.write_text("1000\n")
            slept = []
            self.assertEqual(P.settle_after_restore(log=lambda m: None, sleep=slept.append, now_fn=lambda: 1030, path=p), 30)
            self.assertEqual(slept, [30])

    def test_only_the_audio_unbind_exists_in_the_bind_code(self):
        for mod in (P, G):
            src = pathlib.Path(mod.__file__).read_text()
            for m in re.finditer(r"[^\n]*/unbind[^\n]*", src):
                line = m.group(0)
                self.assertTrue("snd_hda_intel" in line or "unbind file" in line, line)
                self.assertNotIn("drivers/nvidia/unbind", line)


if __name__ == "__main__":
    unittest.main()
