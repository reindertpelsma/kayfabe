#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free tests of the clock/network rule (2026-11-16): a faked guest clock needs the network off, and a guest with internet needs the real
clock. The rule is pinned in the assert itself, in every launch path that builds a QEMU argv for the Windows images (propimage.py build_argv for
all modes, propimage_plain.py, the harness proof of propimage_images.py, propimage_run.sh, win_vm.sh), with fake QEMU command lines only. A
test fails when the assert is removed or inverted in any of them; scripts/ci/clocknet_assert_mutants.py proves it."""
import datetime as dt
import io
import re
import sys
import tempfile
import unittest
import unittest.mock
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
WIN = ROOT / "scripts/bench/windows"
sys.path.insert(0, str(WIN))
sys.path.insert(0, str(HERE))
import clocknet_assert as cna  # noqa: E402
import propimage as PI  # noqa: E402

NOW = dt.datetime(2026, 11, 16, 12, 0, 0, tzinfo=dt.timezone.utc)
STALE = dt.datetime(2026, 10, 1, 0, 0, 0, tzinfo=dt.timezone.utc)
OFF = cna.Net(True, "restrict=on")
ON = cna.Net(False, "internet")


def argv(rtc=None, net=("-netdev", "user,id=n0,restrict=on,hostfwd=tcp:127.0.0.1:2222-:22")):
    out = ["qemu-system-x86_64", "-name", "t"]
    if rtc:
        out += ["-rtc", rtc]
    return out + list(net) + ["-device", "virtio-net-pci,netdev=n0"]


class Plan(unittest.TestCase):
    def code(self, clock, net):
        with self.assertRaises(cna.ClockNetViolation) as c:
            cna.assert_clock_net(clock, net, "t", now=NOW)
        return c.exception.code

    def test_faked_clock_with_internet_is_refused_by_the_fake_clock_check(self):
        # declared faked, base looks like now: only the first check can see it
        self.assertEqual(self.code(cna.Clock(faked=True, base=NOW), ON), cna.CODE_FAKE)

    def test_internet_with_a_stale_base_is_refused_by_the_real_clock_check(self):
        # not declared faked but the base is a recorded time: only the second check can see it
        self.assertEqual(self.code(cna.Clock(faked=False, base=STALE), ON), cna.CODE_REAL)

    def test_a_vm_time_clock_is_not_real_with_internet(self):
        self.assertEqual(self.code(cna.Clock(faked=False, vm_clock=True), ON), cna.CODE_REAL)

    def test_faked_clock_with_the_network_off_is_fine(self):
        cna.assert_clock_net(cna.Clock(faked=True, base=STALE), OFF, "t", now=NOW)

    def test_real_clock_with_the_network_off_or_on_is_fine(self):
        cna.assert_clock_net(cna.Clock(faked=False), OFF, "t", now=NOW)
        cna.assert_clock_net(cna.Clock(faked=False), ON, "t", now=NOW)

    def test_a_base_within_the_tolerance_counts_as_now(self):
        cna.assert_clock_net(cna.Clock(faked=False, base=NOW - dt.timedelta(seconds=299)), ON, "t", now=NOW)
        self.assertEqual(self.code(cna.Clock(faked=False, base=NOW - dt.timedelta(seconds=360)), ON), cna.CODE_REAL)
        self.assertEqual(self.code(cna.Clock(faked=False, base=NOW + dt.timedelta(seconds=360)), ON), cna.CODE_REAL)

    def test_the_violation_names_the_launcher(self):
        with self.assertRaises(cna.ClockNetViolation) as c:
            cna.assert_clock_net(cna.Clock(faked=True, base=STALE), ON, "my launcher", now=NOW)
        self.assertIn("my launcher", str(c.exception))


class Argv(unittest.TestCase):
    def check(self, words):
        return cna.assert_argv(words, "t", NOW)

    def refuses(self, words, code):
        with self.assertRaises(cna.ClockNetViolation) as c:
            self.check(words)
        self.assertEqual(c.exception.code, code)

    def test_real_clock_with_restrict_on_passes(self):
        self.check(argv("base=utc,driftfix=slew"))

    def test_no_rtc_option_is_the_real_clock(self):
        self.check(argv(None))

    def test_a_recorded_base_with_restrict_on_passes(self):
        clock, net = self.check(argv("base=2026-11-01T01:00:00,clock=vm,driftfix=slew"))
        self.assertTrue(clock.faked and net.off)

    def test_a_recorded_base_with_restrict_off_is_refused(self):
        self.refuses(argv("base=2026-11-01T01:00:00,clock=vm", ("-netdev", "user,id=n0,restrict=off")), cna.CODE_FAKE)

    def test_a_recorded_base_with_no_restrict_option_is_refused(self):
        self.refuses(argv("base=2026-11-01T01:00:00", ("-netdev", "user,id=n0")), cna.CODE_FAKE)

    def test_a_recorded_date_only_base_is_refused_with_internet(self):
        self.refuses(argv("base=2026-11-01", ("-netdev", "user,id=n0,restrict=off")), cna.CODE_FAKE)

    def test_clock_vm_with_internet_is_refused(self):
        self.refuses(argv("base=utc,clock=vm", ("-netdev", "user,id=n0,restrict=off")), cna.CODE_FAKE)

    def test_an_unparsable_base_with_internet_is_refused(self):
        self.refuses(argv("base=garbage", ("-netdev", "user,id=n0,restrict=off")), cna.CODE_FAKE)

    def test_internet_with_the_real_clock_passes(self):
        self.check(argv("base=utc,driftfix=slew", ("-netdev", "user,id=n0,restrict=off")))
        self.check(argv("base=localtime", ("-netdev", "user,id=n0,restrict=off")))

    def test_restrict_spellings(self):
        for on in ("on", "yes", "y", "true", "ON"):
            self.check(argv("base=2026-11-01T01:00:00", ("-netdev", "user,id=n0,restrict=" + on)))
        for off in ("off", "no", "n", "false", ""):
            self.refuses(argv("base=2026-11-01T01:00:00", ("-netdev", "user,id=n0,restrict=" + off)), cna.CODE_FAKE)

    def test_the_default_nic_is_internet(self):
        self.refuses(["qemu", "-rtc", "base=2026-11-01T01:00:00"], cna.CODE_FAKE)

    def test_nic_none_and_nodefaults_are_no_network(self):
        self.check(["qemu", "-rtc", "base=2026-11-01T01:00:00", "-nic", "none"])
        self.check(["qemu", "-rtc", "base=2026-11-01T01:00:00", "-net", "none"])
        self.check(["qemu", "-rtc", "base=2026-11-01T01:00:00", "-nodefaults"])

    def test_nic_user_restrict_forms(self):
        self.check(["qemu", "-rtc", "base=2026-11-01T01:00:00", "-nic", "user,restrict=on"])
        self.refuses(["qemu", "-rtc", "base=2026-11-01T01:00:00", "-nic", "user"], cna.CODE_FAKE)

    def test_other_backends_are_internet(self):
        for backend in ("tap,id=n0", "bridge,id=n0,br=br0", "socket,id=n0,listen=:1", "vde,id=n0"):
            self.refuses(argv("base=2026-11-01T01:00:00", ("-netdev", backend)), cna.CODE_FAKE)

    def test_one_open_backend_among_restricted_ones_is_internet(self):
        self.refuses(["qemu", "-rtc", "base=2026-11-01T01:00:00", "-netdev", "user,id=a,restrict=on", "-netdev", "user,id=b,restrict=off"],
                     cna.CODE_FAKE)

    def test_net_nic_alone_is_not_provably_off(self):
        self.refuses(["qemu", "-rtc", "base=2026-11-01T01:00:00", "-net", "nic"], cna.CODE_FAKE)
        self.check(["qemu", "-rtc", "base=2026-11-01T01:00:00", "-net", "nic", "-net", "user,restrict=on"])

    def test_a_non_user_backend_is_internet_even_with_a_restrict_option(self):
        self.refuses(argv("base=2026-11-01T01:00:00", ("-netdev", "tap,id=n0,restrict=on")), cna.CODE_FAKE)

    def test_nic_with_another_backend_is_internet(self):
        self.refuses(["qemu", "-rtc", "base=2026-11-01T01:00:00", "-nic", "tap,model=virtio-net-pci"], cna.CODE_FAKE)
        self.refuses(["qemu", "-rtc", "base=2026-11-01T01:00:00", "-nic", "tap,restrict=on"], cna.CODE_FAKE)

    def test_double_dash_spellings(self):
        self.refuses(["qemu", "--rtc", "base=2026-11-01T01:00:00", "--netdev", "user,id=n0,restrict=off"], cna.CODE_FAKE)

    def test_hostfwd_with_commas_in_other_options_is_parsed(self):
        self.check(argv("base=2026-11-01T01:00:00", ("-netdev", "user,id=n0,net=10.20.30.0/24,ipv6=off,restrict=on,hostfwd=tcp:127.0.0.1:20001-:22")))


class Cli(unittest.TestCase):
    def run_cli(self, *args):
        err = io.StringIO()
        with redirect_stderr(err), redirect_stdout(io.StringIO()):
            rc = cna.main(list(args))
        return rc, err.getvalue()

    def test_a_line_without_any_network_option_is_internet(self):
        self.assertEqual(self.run_cli("line", "qemu -rtc base=2026-11-01T01:00:00")[0], 3)
        self.assertEqual(self.run_cli("line", "qemu -rtc base=utc")[0], 0)

    def test_argv_form_ok_and_refused(self):
        self.assertEqual(self.run_cli("argv", "--who", "x", "--", "qemu", "-rtc", "base=utc", "-netdev", "user,restrict=off")[0], 0)
        rc, err = self.run_cli("argv", "--who", "x", "--", "qemu", "-rtc", "base=2026-11-01T01:00:00", "-netdev", "user,restrict=off")
        self.assertEqual(rc, 3)
        self.assertIn("CLOCKNET_REFUSED FAKE_CLOCK_NEEDS_OFFLINE", err)

    def test_line_form_with_the_cmd_prefix_and_quotes(self):
        good = "CMD: qemu -name 'a b' -rtc base=utc -netdev user,restrict=off"
        bad = "qemu -rtc base=2026-11-01T01:00:00 -netdev user,restrict=off"
        self.assertEqual(self.run_cli("line", good)[0], 0)
        self.assertEqual(self.run_cli("line", bad)[0], 3)

    def test_file_form_and_an_unreadable_file(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "last_cmd.txt"
            p.write_text("qemu -rtc base=2026-11-01T01:00:00 -netdev user,restrict=off\n")
            self.assertEqual(self.run_cli("file", str(p))[0], 3)
            p.write_text("qemu -rtc base=2026-11-01T01:00:00 -netdev user,restrict=on\n")
            self.assertEqual(self.run_cli("file", str(p))[0], 0)
            rc, err = self.run_cli("file", str(Path(d) / "missing"))
            self.assertEqual(rc, 3)
            self.assertIn("UNREADABLE", err)


class PropimageBuildArgv(unittest.TestCase):
    """propimage.py build_argv: every mode, plain, and the refusal when the plan is broken."""

    def state(self):
        st = PI.init_state("seed-for-the-test")
        st["clock"]["last_time_at_stop_utc"] = "2026-11-01T00:00:00Z"
        return st

    def test_every_mode_builds_and_passes_the_rule(self):
        for mode in PI.MODES:
            words = PI.build_argv(self.state(), mode, "/d/disk", "/w", display="off")
            cna.assert_argv(words, mode)

    def test_the_modes_have_the_expected_shapes(self):
        shapes = {}
        for mode in PI.MODES:
            clock, net = cna.assert_argv(PI.build_argv(self.state(), mode, "/d", "/w", display="off"), mode)
            shapes[mode] = (clock.faked, net.off)
        self.assertEqual(shapes, {"setup": (False, False), "install": (False, False), "offline": (True, True), "ci": (True, True)})

    def test_plain_default_is_real_clock_and_network_off(self):
        st = PI.plain_state("plainseed1", "user")
        clock, net = cna.assert_argv(PI.build_argv(st, PI.PLAIN, "/d", "/w", display="off"), "plain")
        self.assertEqual((clock.faked, net.off), (False, True))

    def test_plain_with_net_on_is_real_clock_and_internet(self):
        st = PI.plain_state("plainseed1", "user")
        words = PI.build_argv(st, PI.PLAIN, "/d", "/w", display="off", net="on")
        clock, net = cna.assert_argv(words, "plain")
        self.assertEqual((clock.faked, net.off), (False, False))
        self.assertIn("-rtc", words)
        self.assertEqual(words[words.index("-rtc") + 1], "base=utc,driftfix=slew")

    def test_net_on_is_only_for_plain(self):
        for mode in PI.MODES:
            with self.assertRaises(PI.PropError):
                PI.build_argv(self.state(), mode, "/d", "/w", display="off", net="on")

    def test_a_recorded_clock_in_a_networked_mode_is_refused_install(self):
        stale = dt.datetime(2026, 11, 1, 0, 10, 0, tzinfo=dt.timezone.utc)
        with unittest.mock.patch.object(PI, "boot_clock", return_value=stale):
            with self.assertRaises(PI.PropError) as c:
                PI.build_argv(self.state(), "install", "/d", "/w", display="off")
        self.assertIn(cna.CODE_FAKE, str(c.exception))

    def test_a_recorded_clock_in_plain_with_internet_is_refused(self):
        stale = dt.datetime(2026, 11, 1, 0, 10, 0, tzinfo=dt.timezone.utc)
        with unittest.mock.patch.object(PI, "boot_clock", return_value=stale):
            with self.assertRaises(PI.PropError) as c:
                PI.build_argv(PI.plain_state("plainseed1", "user"), PI.PLAIN, "/d", "/w", display="off", net="on")
        self.assertIn(cna.CODE_FAKE, str(c.exception))

    def test_a_recorded_clock_in_plain_with_the_network_off_is_allowed_by_the_rule(self):
        stale = dt.datetime(2026, 11, 1, 0, 10, 0, tzinfo=dt.timezone.utc)
        with unittest.mock.patch.object(PI, "boot_clock", return_value=stale):
            PI.build_argv(PI.plain_state("plainseed1", "user"), PI.PLAIN, "/d", "/w", display="off", net="off")

    def test_a_clock_that_does_not_follow_the_host_in_a_networked_mode_is_refused(self):
        real = PI.build_argv

        def patched(*a, **k):
            words = real(*a, **k)
            return words
        # the same builder with an rtc option that keeps the base real but pins the clock to VM time
        with unittest.mock.patch.object(PI, "boot_clock", return_value=None):
            words = real(self.state(), "install", "/d", "/w", display="off")
        words[words.index("-rtc") + 1] = "base=utc,clock=vm"
        with self.assertRaises(cna.ClockNetViolation):
            cna.assert_argv(words, "t")


class PlainLauncher(unittest.TestCase):
    def test_launch_builds_the_plain_argv_and_the_overlay_command(self):
        import propimage_plain as plain
        import argparse
        a = argparse.Namespace(ssh_user="u", ssh_port=None, qemu="qemu-x", net="off")
        words, clone, state = plain.launch(a, "/w", seed="deterministic-seed")
        self.assertEqual(str(clone), "/w/run/clone.qcow2")
        self.assertTrue(cna.net_from_argv(words).off)
        a.net = "on"
        words, _, _ = plain.launch(a, "/w", seed="deterministic-seed")
        self.assertFalse(cna.net_from_argv(words).off)
        self.assertFalse(cna.clock_from_argv(words).faked)

    def test_a_recorded_clock_cannot_reach_a_plain_launch_with_internet(self):
        import propimage_plain as plain
        import argparse
        a = argparse.Namespace(ssh_user="u", ssh_port=None, qemu="qemu-x", net="on")
        stale = dt.datetime(2026, 11, 1, tzinfo=dt.timezone.utc)
        with unittest.mock.patch.object(plain.PI, "boot_clock", return_value=stale):    # propimage_check loads its own copy of propimage.py
            with self.assertRaises(plain.PI.PropError):
                plain.launch(a, "/w", seed="deterministic-seed")


class HarnessProof(unittest.TestCase):
    """propimage_images.proof reads run/last_cmd.txt, the line the harness really started, and parses it again."""

    def proof(self, line, mode, want="off"):
        import propimage_images as images
        with tempfile.TemporaryDirectory() as d:
            work = Path(d) / "w"
            (work / "run").mkdir(parents=True)
            (work / "run/last_cmd.txt").write_text(line + "\n")
            return images.proof(work, mode, want)

    def clone(self, d="/w"):
        return "-drive file=%s/run/clone.qcow2" % d

    def test_state_shape_needs_restrict_on_and_the_overlay(self):
        with tempfile.TemporaryDirectory() as d:
            work = Path(d) / "w"
            (work / "run").mkdir(parents=True)
            import propimage_images as images
            (work / "run/last_cmd.txt").write_text("qemu -rtc base=2026-11-01T01:00:00,clock=vm -netdev user,id=n0,restrict=on -drive file=%s/run/clone.qcow2\n" % work)
            self.assertTrue(images.proof(work, "ci")["ok"])
            (work / "run/last_cmd.txt").write_text("qemu -rtc base=2026-11-01T01:00:00,clock=vm -netdev user,id=n0,restrict=off -drive file=%s/run/clone.qcow2\n" % work)
            self.assertFalse(images.proof(work, "ci")["ok"])

    def test_plain_shape_needs_the_real_clock_and_the_requested_network(self):
        self.assertTrue(self.proof("qemu -rtc base=utc -netdev user,restrict=on", "plain", "off")["ok"])
        self.assertFalse(self.proof("qemu -rtc base=utc -netdev user,restrict=off", "plain", "off")["ok"])
        self.assertTrue(self.proof("qemu -rtc base=utc -netdev user,restrict=off", "plain", "on")["ok"])
        self.assertFalse(self.proof("qemu -rtc base=utc -netdev user,restrict=on", "plain", "on")["ok"])

    def test_a_forbidden_combination_in_the_file_is_not_proven(self):
        got = self.proof("qemu -rtc base=2026-11-01T01:00:00,clock=vm -netdev user,restrict=off", "plain", "on")
        self.assertFalse(got["ok"])
        self.assertIsNone(got["clock"])

    def test_plain_with_internet_when_off_was_requested_is_not_proven(self):
        got = self.proof("qemu -rtc base=utc -netdev user,restrict=off", "plain", "off")
        self.assertEqual((got["clock"], got["net"], got["ok"]), ("real", "on", False))

    def test_state_shape_with_restrict_off_names_no_clock(self):
        got = self.proof("qemu -rtc base=2026-11-01T01:00:00,clock=vm -netdev user,restrict=off -drive file=/w/run/clone.qcow2", "ci")
        self.assertEqual((got["clock"], got["net"], got["ok"]), (None, None, False))

    def test_a_missing_file_is_not_proven(self):
        import propimage_images as images
        self.assertFalse(images.proof("/nonexistent/dir", "plain")["ok"])


class ShellLaunchers(unittest.TestCase):
    """The shell launchers call the same assert on the line that is about to start (the shell stays glue)."""

    def test_propimage_run_sh_checks_the_line_before_it_execs(self):
        text = (WIN / "propimage_run.sh").read_text()
        call = text.index('"${PYA[@]}" line "$QCMD"')
        self.assertLess(call, text.index('eval "exec $QCMD"'))
        self.assertIn("clocknet_assert.py", text)
        self.assertIn("exit 3", text[call:call + 300])

    def run_script(self, qcmd):
        """propimage_run.sh in mode setup with a python3 shim that answers `propimage.py argv` with `qcmd` and runs everything else for real."""
        import os
        import shutil
        import stat
        import subprocess
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            (d / "bin").mkdir()
            (d / "media").mkdir()
            shutil.copyfile(WIN / "clocknet_assert.py", d / "bin/clocknet_assert.py")
            (d / "bin/propimage.py").write_text("")
            (d / "state.json").write_text("{}")
            for name in ("win.iso", "virtio-win.iso", "tools.iso"):
                (d / "media" / name).write_text("")
            shim = d / "shim"
            shim.mkdir()
            real = shutil.which("python3")
            (shim / "python3").write_text('#!/bin/sh\ncase "$*" in *propimage.py*) echo "$QCMD_FOR_TEST"; echo "# swtpm"; exit 0;; esac\nexec %s "$@"\n' % real)
            (shim / "python3").chmod((shim / "python3").stat().st_mode | stat.S_IXUSR)
            (shim / "swtpm").write_text("#!/bin/sh\nexit 9\n")      # never start a real swtpm from a test
            (shim / "swtpm").chmod(0o755)
            env = dict(os.environ, PROP_DIR=str(d), PATH=str(shim) + ":" + os.environ["PATH"], QCMD_FOR_TEST=qcmd, PROP_SESSION_USER="nobody")
            return subprocess.run(["bash", str(WIN / "propimage_run.sh"), "setup"], env=env, capture_output=True, text=True, timeout=60)

    def test_propimage_run_sh_refuses_a_forbidden_line_before_starting_anything(self):
        done = self.run_script("qemu -rtc base=2026-11-01T01:00:00,clock=vm -netdev user,id=n0,restrict=off")
        self.assertEqual(done.returncode, 3, done.stderr)
        self.assertIn("clock/network rule refused", done.stderr)
        self.assertNotIn("swtpm", done.stderr.replace("clock/network", ""))

    def test_propimage_run_sh_lets_an_allowed_line_through_to_the_next_step(self):
        done = self.run_script("qemu -rtc base=utc -netdev user,id=n0,restrict=off")
        self.assertNotIn("clock/network rule refused", done.stderr)
        self.assertNotEqual(done.returncode, 0)          # it goes on to the (shimmed, failing) swtpm start: that is all this test needs

    def test_win_vm_sh_start_passes_the_words_it_launches(self):
        text = (WIN / "win_vm.sh").read_text()
        start = text.index("vm_start() {")
        body = text[start:text.index("\n}\n", start)]
        call = body[body.index("clocknet_assert.py") - 40:body.index("clocknet_assert.py") + 200]
        self.assertIn('-- "$q" "${QA[@]}" "$@"', call)

    def test_win_vm_sh_checks_every_start_in_vm_start(self):
        text = (WIN / "win_vm.sh").read_text()
        start = text.index("vm_start() {")
        body = text[start:text.index("\n}\n", start)]
        call = body.index('python3 -B -I "$HERE/clocknet_assert.py"')
        self.assertLess(call, body.index('"$q" "${QA[@]}" "$@" > "$qlog"'))
        self.assertIn("die ", body[call:call + 250])
        self.assertIn('"${QA[@]}"', body[call:body.index("\n", call)])      # on the check's own line, not the launch line after it

    def test_the_cli_agrees_with_what_win_vm_sh_builds(self):
        # the shape qemu_common builds: WINVM_NET=offline (restrict=on) or online (restrict=off) with the default real clock
        ok = ["qemu", "-rtc", "base=utc,driftfix=slew", "-netdev", "user,id=n0,restrict=off,hostfwd=tcp:127.0.0.1:2224-:22"]
        cna.assert_argv(ok, "win_vm.sh")
        with self.assertRaises(cna.ClockNetViolation):
            cna.assert_argv(["qemu", "-rtc", "base=2026-11-01T00:00:00,driftfix=slew", "-netdev", "user,id=n0,restrict=off"], "win_vm.sh")


if __name__ == "__main__":
    unittest.main()
