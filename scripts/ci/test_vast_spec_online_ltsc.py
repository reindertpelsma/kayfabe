#!/usr/bin/env python3
"""GPU-free tests of the SPECviewperf stage's guest network and licence probe (owner decision, 2026-11-17).

STATUS: LIVE as tests, 2026-11-17. The open Windows LTSC guest of the SPECviewperf lanes gets the network and the real clock
(KF_SPEC_NET=online, the default): the Windows evaluation guest needs both to complete its own start-up validation. The tests run
the REAL scripts/bench/box/specviewperf_stage.sh against the sandbox of spec_stage_fakes.py, whose fake win_vm.sh records the
WINVM_NET of every launch and the clock the REAL scripts/bench/windows/clockrule.py picks for it. Mutants with two named killers
each: scripts/ci/spec_stage_mutants.py.

FALSIFIER: a stage that launches any of its boots offline (or on the recorded clock) without KF_SPEC_NET=offline; an online launch
on the recorded clock or an offline launch on the real one; a licence probe that is off by default, never repeats inside a viewset,
or skips the end; a status line, an evidence file or the streamed log that carries a product key, an activation id, a computer name
or an account; a guest that went down with a licence event 1074 and is reported as a bare GUEST row; a non-licence 1074 reported as
a licence shutdown.
"""

import json
import re
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

import spec_stage_fakes as fk

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts/bench/specviewperf"))
import licence as L  # noqa: E402

RECORDED_CLOCK = "base=2026-11-16T10:00:00,clock=vm,driftfix=slew"
REAL_CLOCK = "base=utc,driftfix=slew"
LEAKS = ("ABCDE", "FGHIJ", "0a1b2c3d-1111", "KFWIN", "Product Key", "Activation ID", r"kf\\", "NT AUTHORITY", "bob")
STATE = re.compile(r"^SPEC_LICENCE_STATE at=(\S+) (.*)$", re.M)


def event(when, process, reason, comment="", kind="power off", code="0x80000000"):
    """One LIC_EVT1074 line as licence_probe.ps1 writes it (the computer and the user are never in it)."""
    return "LIC_EVT1074 time=%s process='%s' reason='%s' code=%s type='%s' comment='%s'\n" % (when, process, reason, code, kind, comment)


def readout(events="", up_min=5, boot="2026-11-17T10:00:00.0000000Z"):
    base = fk.healthy_licence("x", up_min)
    return base.replace("LIC_DONE\n", events + "LIC_DONE\n").replace("lastboot=2026-11-17T10:00:00.0000000Z", "lastboot=" + boot)


WLMS = event("2026-11-17T11:03:00.0000000Z", r"C:\Windows\System32\wlms\wlms.exe (KFWIN-0)", "Other (Planned)",
             r"The evaluation period of this copy of Windows has expired; bob\kf", kind="power off")
OTHER = event("2026-11-17T11:03:00.0000000Z", r"C:\Windows\System32\svchost.exe", "Operating System: Recovery (Planned)", "", kind="restart")
OLD_WLMS = event("2026-11-17T09:00:00.0000000Z", r"C:\Windows\System32\wlms\wlms.exe", "Other (Planned)", "evaluation expired")


class Case(unittest.TestCase):
    def go(self, lane="lanea", wrapper=False, **extra):
        # a sandbox of its own for every run: the fakes keep logs
        self.dir = tempfile.mkdtemp(prefix="kfspec-net-")
        self.addCleanup(shutil.rmtree, self.dir, True)
        self.sb = fk.Sandbox(self.dir)
        scn = {k: extra.pop(k) for k in list(extra) if "__" in k}
        for k, v in scn.items():
            self.sb.set(k.replace("__", "."), v)
        self.p = self.sb.run(lane, 40000, wrapper=wrapper, **extra)
        self.rows, self.order = fk.rows_of(self.p.stdout)
        return self.p

    def runs(self):
        out = []
        for line in self.sb.runs():
            kv = dict(re.findall(r"(\w+)=(\S*)", line.split(" :: ")[0]))
            out.append(kv)
        return out

    def states(self):
        return STATE.findall(self.p.stdout)

    def evidence(self, name):
        return self.sb.read("prov/%s_spec/%s" % (self.sb.tag, name))


class GuestNetwork(Case):
    """KF_SPEC_NET: the WINVM_NET of every boot of the stage, and the clock the clock rule then picks."""

    def test_every_boot_is_online_on_the_real_clock_by_default(self):
        self.go()
        runs = self.runs()
        self.assertGreaterEqual(len(runs), 2, self.p.stdout)  # the copy boot and the lane's boot
        self.assertEqual({r["net"] for r in runs}, {"online"})
        self.assertEqual({r["rtc"] for r in runs}, {REAL_CLOCK})

    def test_an_empty_setting_is_the_default(self):
        self.go(KF_SPEC_NET="")
        self.assertEqual({r["net"] for r in self.runs()}, {"online"})

    def test_offline_override_boots_offline_on_the_recorded_clock(self):
        self.go(KF_SPEC_NET="offline")
        runs = self.runs()
        self.assertGreaterEqual(len(runs), 2)
        self.assertEqual({r["net"] for r in runs}, {"offline"})
        self.assertEqual({r["rtc"] for r in runs}, {RECORDED_CLOCK})

    def test_the_stage_setting_wins_over_a_wrongly_inherited_winvm_net(self):
        self.go(WINVM_NET="offline")
        self.assertEqual({r["net"] for r in self.runs()}, {"online"})
        self.go(KF_SPEC_NET="offline", WINVM_NET="online")
        self.assertEqual({r["net"] for r in self.runs()}, {"offline"})

    def test_any_other_value_is_a_usage_error_and_boots_nothing(self):
        for value in ("sideways", "ONLINE", "on", "restrict=off"):
            with self.subTest(value):
                p = self.go(KF_SPEC_NET=value)
                self.assertEqual(p.returncode, 2, p.stdout)
                self.assertEqual(p.stdout.strip(), "SPECVIEWPERF_STAGE_RETRY class=USAGE")
                self.assertEqual(self.sb.runs(), [])

    def test_a_lane_run_passes_in_both_modes(self):
        for net in ("online", "offline"):
            with self.subTest(net):
                p = self.go(KF_SPEC_NET=net)
                self.assertEqual(p.returncode, 0, p.stdout)
                self.assertEqual(set(r["result"] for r in self.rows.values()), {"PASS"})

    def test_the_stage_says_which_network_the_guest_has(self):
        self.go()
        self.assertIn("SPEC_NOTE guest network=online", self.p.stdout)
        self.go(KF_SPEC_NET="offline")
        self.assertIn("SPEC_NOTE guest network=offline", self.p.stdout)

    def test_the_network_is_recorded_in_the_stage_log_of_the_evidence(self):
        self.go()
        self.assertIn("guest network=online", self.evidence("stage.log"))
        self.assertIn("the Windows evaluation guest needs the network and the real time", self.evidence("stage.log"))


class LicenceProbe(Case):
    def test_the_probe_is_on_by_default_at_boot_after_each_viewset_and_at_the_end(self):
        self.go()
        labels = [a for a, _ in self.states()]
        self.assertTrue(labels[0].endswith("_start"), labels)
        self.assertEqual([x for x in labels if x.startswith("after_")], ["after_%s" % n for n in ("energy_04", "creo_04", "blender_01", "enscape_01")])
        self.assertEqual(len([x for x in labels if x.startswith("after_")]), 4, labels)
        self.assertEqual(labels[-1], "end", labels)

    def test_one_probe_per_moment_and_no_more(self):
        self.go()
        # the boot, one after each of the four viewsets, the end
        self.assertEqual(len(re.findall(r"^probe run", self.sb.calls(), re.M)), 6, self.sb.calls())
        self.go(KF_SPEC_LICENCE_PROBE="1", KF_SPEC_NET="offline")
        self.assertEqual(len(re.findall(r"^probe run", self.sb.calls(), re.M)), 6)

    def test_every_state_line_says_state_and_network(self):
        self.go()
        self.assertGreaterEqual(len(self.states()), 6)
        for label, rest in self.states():
            self.assertIn("status=Licensed", rest)
            self.assertIn("up_min=5", rest)
            self.assertIn("wlms=Running", rest)
            self.assertTrue(rest.endswith("net=online"), rest)
        self.go(KF_SPEC_NET="offline")
        self.assertGreaterEqual(len(self.states()), 6)
        self.assertTrue(all(rest.endswith("net=offline") for _, rest in self.states()))

    def test_zero_and_off_switch_the_probe_off(self):
        for value in ("0", "off", ""):
            with self.subTest(value):
                self.go(KF_SPEC_LICENCE_PROBE=value)
                self.assertEqual(self.states(), [])
                self.assertNotIn("probe ", self.sb.calls())
                self.assertEqual(self.p.returncode, 0, self.p.stdout)

    def test_a_long_viewset_is_probed_at_least_every_15_minutes(self):
        # 100 polls of 20 s: 2000 s inside one viewset; the settle wait and the boot probe come first
        self.go(**{"polls__blender-01": 100})
        ups = [int(m) for a, _ in self.states() for m in re.findall(r"_up(\d+)m$", a)]
        self.assertGreaterEqual(len(ups), 2, self.p.stdout)
        # the first is due 15 minutes after the boot probe, the next 15 after that (the poll adds up to 20 s)
        self.assertTrue(all(15 <= b - a <= 16 for a, b in zip(ups, ups[1:])), ups)
        self.assertGreaterEqual(ups[0], 15)

    def test_the_periodic_probes_are_state_lines_like_the_others(self):
        self.go(**{"polls__blender-01": 100})
        periodic = [rest for a, rest in self.states() if "_up" in a]
        self.assertGreaterEqual(len(periodic), 2)
        self.assertTrue(all("status=Licensed" in r and r.endswith("net=online") for r in periodic), periodic)

    def test_the_wrapper_passes_every_state_line_the_stage_keeps(self):
        self.go(wrapper=True)
        kept = self.evidence("licence_states.txt").splitlines()
        self.assertGreaterEqual(len(kept), 6)
        self.assertEqual([l for l in self.p.stdout.splitlines() if l.startswith("SPEC_LICENCE_STATE ")], kept)

    def test_a_short_viewset_needs_no_repeat(self):
        self.go()
        self.assertEqual([a for a, _ in self.states() if "_up" in a], [])

    def test_the_states_are_kept_in_the_evidence_as_printed(self):
        self.go()
        printed = [l for l in self.p.stdout.splitlines() if l.startswith("SPEC_LICENCE_STATE ")]
        self.assertGreaterEqual(len(printed), 5)
        self.assertEqual(self.evidence("licence_states.txt").splitlines(), printed)

    def test_no_key_id_computer_or_account_leaves_the_readout(self):
        self.go(wrapper=True)
        blob = self.p.stdout + "".join(
            f.read_text(errors="replace") for f in (self.sb.prov / (self.sb.tag + "_spec")).rglob("*") if f.is_file())
        for leak in LEAKS:
            self.assertNotRegex(blob, leak)
        self.assertIn("SPEC_LICENCE_STATE", self.p.stdout)  # the wrapper lets the state lines through

    def test_the_raw_readout_stays_in_the_private_directory(self):
        self.go(wrapper=True)
        private = list((self.sb.root / "private").rglob("licence_*.txt"))
        self.assertGreaterEqual(len(private), 5)
        self.assertTrue(any("Partial Product Key" in f.read_text() for f in private))
        kept = [f.name for f in (self.sb.prov / (self.sb.tag + "_spec")).rglob("licence_*.txt")]
        self.assertEqual(kept, ["licence_states.txt"])  # the state lines only; the raw readouts are in the private directory

    def test_the_probe_script_never_writes_the_computer_or_the_account_of_an_event(self):
        ps = (ROOT / "scripts/bench/windows/spec/licence_probe.ps1").read_text()
        body = ps[ps.index("Get-WinEvent"):]
        self.assertIn("LIC_EVT1074", body)
        for index in (1, 6):
            self.assertNotRegex(body, r"\$p\[%d\]" % index)
        self.assertNotRegex(ps, r"Write-Output[^\n]*(UserName|\$env:COMPUTERNAME|hostname)")

    def test_an_unreadable_guest_is_unavailable_not_an_error(self):
        p = self.go(licence__default="")
        self.assertEqual(p.returncode, 0, p.stdout)
        self.assertTrue(all(rest.startswith("status=unavailable") for _, rest in self.states()), self.states())


class ShutdownClassification(Case):
    """The guest's licence shutdown: reason GUEST_SHUTDOWN_LICENCE with the 1074 text, never a bare GUEST row."""

    def die(self, viewset="blender-01", fixture=WLMS, boot="run3", lane="lanea", **extra):
        # boot numbers count the copy boot: copy1, run2 (first run boot), run3 (the fresh boot after the guest went down)
        return self.go(lane, **{"viewset__" + viewset: "die_mid", "polls__" + viewset: 2, "licence__" + boot: readout(fixture, 5), **extra})

    def line(self):
        return [l for l in self.p.stdout.splitlines() if l.startswith("SPEC_GUEST_SHUTDOWN_LICENCE ")]

    def test_a_licence_1074_makes_the_row_guest_shutdown_licence_with_the_text(self):
        p = self.die()
        self.assertEqual(self.rows["blender-01"]["reason"], "GUEST_SHUTDOWN_LICENCE", p.stdout)
        self.assertEqual(self.rows["blender-01"]["result"], "FAIL")
        (line,) = self.line()
        self.assertIn("viewset=blender-01 event=1074", line)
        self.assertIn("process=wlms.exe", line)
        self.assertIn("The_evaluation_period_of_this_copy_of_Windows_has_expired", line)
        self.assertIn("time=2026-11-17T11:03:00Z", line)
        self.assertEqual(self.sb.read("prov/%s_spec/stage.log" % self.sb.tag).count("SPEC_GUEST_SHUTDOWN_LICENCE "), 1)

    def test_the_account_and_the_computer_of_the_event_text_are_not_printed(self):
        self.die()
        (line,) = self.line()
        for leak in ("bob", "KFWIN"):
            self.assertNotIn(leak, line)
        self.assertNotRegex(line.split("text=", 1)[1], r"(?<![A-Za-z])kf(?![A-Za-z])")

    def test_the_shutdown_line_comes_before_its_row_and_the_order_of_the_rows_is_the_lanes(self):
        self.die()
        lines = self.p.stdout.splitlines()
        shutdown = next(i for i, l in enumerate(lines) if l.startswith("SPEC_GUEST_SHUTDOWN_LICENCE "))
        row = next(i for i, l in enumerate(lines) if l.startswith("SPEC_ROW name=blender-01 "))
        self.assertLess(shutdown, row)
        self.assertEqual(self.order, ["energy-04", "creo-04", "blender-01", "enscape-01"])

    def test_the_next_viewset_still_gets_its_fresh_boot_and_runs(self):
        self.die()
        self.assertEqual(self.rows["enscape-01"]["result"], "PASS")
        self.assertEqual(len(self.runs()), 3)  # the copy boot, the lane's first boot, the fresh boot
        self.assertEqual(fk.verdict_of(self.p.stdout), "SPECVIEWPERF_STAGE_FAIL")

    def test_a_guest_that_went_down_for_another_reason_stays_a_bare_guest_row(self):
        self.die(fixture=OTHER)
        self.assertEqual(self.rows["blender-01"]["reason"], "GUEST")
        self.assertEqual(self.line(), [])

    def test_a_guest_without_any_event_1074_stays_a_bare_guest_row(self):
        self.die(fixture="")
        self.assertEqual(self.rows["blender-01"]["reason"], "GUEST")
        self.assertEqual(self.line(), [])

    def test_a_licence_1074_older_than_the_boot_that_went_down_is_not_the_cause(self):
        # the first boot started at 10:00 (guest clock): an event of 09:00 belongs to an earlier life of the image
        self.die(fixture=OLD_WLMS)
        self.assertEqual(self.rows["blender-01"]["reason"], "GUEST")
        self.assertEqual(self.line(), [])

    def test_the_forensic_boot_is_probed_like_any_other_and_says_so(self):
        self.die(viewset="enscape-01", boot="run3")
        self.assertIn("forensic_boot2", [a for a, _ in STATE.findall(self.p.stdout)])
        self.assertEqual(self.p.stdout.count("SPEC_ROW name=enscape-01 "), 1)

    def test_an_old_1074_is_not_the_cause_of_the_last_viewset_either(self):
        self.die(viewset="enscape-01", fixture=OLD_WLMS)
        self.assertEqual(self.rows["enscape-01"]["reason"], "GUEST")
        self.assertEqual(self.line(), [])
        self.assertEqual(self.p.stdout.count("SPEC_ROW name=enscape-01 "), 1)

    def test_the_last_viewset_that_dies_gets_one_more_boot_to_be_read(self):
        # enscape-01 is the last viewset of lane A: boots copy1, run2 (energy, creo, blender), the forensic boot is run3
        p = self.die(viewset="enscape-01", boot="run3")
        self.assertEqual(self.rows["enscape-01"]["reason"], "GUEST_SHUTDOWN_LICENCE", p.stdout)
        self.assertEqual(len(self.runs()), 3)
        self.assertIn("one more boot to read why the guest went down during enscape-01", p.stdout)
        self.assertEqual(fk.verdict_of(p.stdout), "SPECVIEWPERF_STAGE_FAIL")

    def limit_run(self):
        # max_guest_boots is 3: energy dies on the 1st lane boot (read by the 2nd), creo on the 2nd (read by the 3rd), blender passes
        # on the 3rd, enscape (the last) dies on it, and no boot is left to read why
        scn = {"viewset__%s" % n: "die_mid" for n in ("energy-04", "creo-04", "enscape-01")}
        scn.update({"polls__%s" % n: 1 for n in ("energy-04", "creo-04", "enscape-01")})
        scn.update({"licence__run3": readout(WLMS, 5), "licence__run4": readout(WLMS, 5), "licence__run5": readout(WLMS, 5)})
        return self.go(**scn)

    def test_no_forensic_boot_beyond_the_boot_limit(self):
        self.limit_run()
        self.assertEqual(len(self.runs()) - 1, 3, self.p.stdout)  # the copy boot is not a lane boot
        self.assertNotIn("one more boot", self.p.stdout)

    def test_a_row_without_a_boot_left_to_read_it_stays_a_bare_guest_row_once(self):
        self.limit_run()
        self.assertEqual({n: r["reason"] for n, r in self.rows.items()},
                         {"energy-04": "GUEST_SHUTDOWN_LICENCE", "creo-04": "GUEST_SHUTDOWN_LICENCE", "blender-01": "-", "enscape-01": "GUEST"})
        self.assertEqual(self.order, ["energy-04", "creo-04", "blender-01", "enscape-01"])

    def test_without_the_probe_the_row_is_emitted_at_once_as_a_bare_guest(self):
        self.die(KF_SPEC_LICENCE_PROBE="0")
        self.assertEqual(self.rows["blender-01"]["reason"], "GUEST")
        self.assertEqual(self.line(), [])
        self.assertNotIn("one more boot", self.p.stdout)

    def test_the_wrapper_keeps_the_licence_row_and_puts_the_shutdown_line_before_it(self):
        self.die(wrapper=True)
        lines = self.p.stdout.splitlines()
        row = next(i for i, l in enumerate(lines) if l.startswith("SPEC_ROW name=blender-01 result=FAIL subtests=0/0 reason=GUEST_SHUTDOWN_LICENCE "))
        self.assertTrue(lines[row - 1].startswith("SPEC_GUEST_SHUTDOWN_LICENCE viewset=blender-01 event=1074 "), lines[row - 1])

    def test_the_classification_passes_the_wrapper_and_the_payload_pattern(self):
        self.die(wrapper=True)
        self.assertIn("reason=GUEST_SHUTDOWN_LICENCE", self.p.stdout)
        self.assertEqual(len(self.line()), 1)
        sys.path.insert(0, str(Path(__file__).resolve().parent))
        import vast_payload as vp

        rows = vp.SPEC_ROW.findall(self.p.stdout)
        self.assertIn(("blender-01", "FAIL"), rows)


class LicenceModule(unittest.TestCase):
    def test_state_line_of_a_healthy_readout(self):
        line = L.state_line(readout(), "boot2_start", "online")
        self.assertRegex(line, r"^SPEC_LICENCE_STATE at=boot2_start up_min=5 status=Licensed remaining=129600_minute\(s\)_\(90_day\(s\)\) ")
        self.assertTrue(line.endswith(" net=online"))

    def test_nothing_secret_survives_the_alphabet(self):
        for text in ("key ABCDE-FGHIJ-KLMNO-PQRST-UVWXY", "id 0a1b2c3d-1111-4222-8333-444455556666", r"user KFWIN\kf", r"NT AUTHORITY\SYSTEM x"):
            with self.subTest(text):
                out = L.clean(text, 100)
                self.assertNotRegex(out, r"ABCDE|0a1b2c3d|KFWIN|kf$|AUTHORITY|SYSTEM")
        self.assertEqual(L.clean("", 10), "-")
        self.assertEqual(len(L.clean("x" * 500, 40)), 40)

    def test_unavailable_and_bad_labels(self):
        self.assertEqual(L.state_line("", "end", "online"), "SPEC_LICENCE_STATE at=end status=unavailable net=online")
        self.assertTrue(L.state_line(readout(), "bad label!", "x").startswith("SPEC_LICENCE_STATE at=probe "))
        self.assertTrue(L.state_line(readout(), "end", "x").endswith("net=unknown"))

    def test_a_licence_event_is_found_and_the_process_is_its_basename(self):
        line = L.shutdown_line(readout(WLMS), "blender-01")
        self.assertIn("process=wlms.exe", line)
        self.assertNotIn("System32", line)
        self.assertNotIn("KFWIN", line)  # the process of a 1074 carries "(<computer name>)": only the exe's name is kept

    def test_an_event_that_names_a_licence_without_wlms_counts(self):
        ev = event("2026-11-17T11:00:00Z", r"C:\x\other.exe", "Other (Planned)", "Windows licensing: the evaluation has ended")
        self.assertIsNotNone(L.shutdown_line(readout(ev), "enscape-01"))

    def test_other_events_do_not(self):
        self.assertIsNone(L.shutdown_line(readout(OTHER), "blender-01"))
        self.assertIsNone(L.shutdown_line(readout(""), "blender-01"))
        self.assertIsNone(L.shutdown_line(readout(WLMS), "../etc/passwd"))

    def test_since_is_a_floor_on_the_guest_clock(self):
        self.assertIsNone(L.shutdown_line(readout(OLD_WLMS), "blender-01", "2026-11-17T10:00:00Z"))
        self.assertIsNotNone(L.shutdown_line(readout(OLD_WLMS), "blender-01", "2026-11-17T08:00:00Z"))
        self.assertIsNotNone(L.shutdown_line(readout(WLMS), "blender-01", "2026-11-17T10:00:00Z"))

    def test_the_newest_licence_event_is_reported(self):
        older = event("2026-11-17T10:30:00Z", r"C:\w\wlms.exe", "Other", "evaluation first")
        newer = event("2026-11-17T11:30:00Z", r"C:\w\wlms.exe", "Other", "evaluation second")
        self.assertIn("evaluation_second", L.shutdown_line(readout(newer + older), "blender-01"))

    def test_the_newest_licence_event_is_reported_whatever_the_order_of_the_readout(self):
        older = event("2026-11-17T10:30:00Z", r"C:\w\wlms.exe", "Other", "evaluation first")
        newer = event("2026-11-17T11:30:00Z", r"C:\w\wlms.exe", "Other", "evaluation second")
        self.assertIn("evaluation_second", L.shutdown_line(readout(older + newer), "blender-01"))

    def test_the_newest_of_three_licence_events_is_reported(self):
        mid = event("2026-11-17T10:45:00Z", r"C:\w\wlms.exe", "Other", "evaluation middle")
        old = event("2026-11-17T10:30:00Z", r"C:\w\wlms.exe", "Other", "evaluation oldest")
        new = event("2026-11-17T11:30:00Z", r"C:\w\wlms.exe", "Other", "evaluation newest")
        self.assertIn("evaluation_newest", L.shutdown_line(readout(old + new + mid), "blender-01"))
        self.assertIn("evaluation_newest", L.shutdown_line(readout(mid + old + new), "blender-01"))

    def test_a_product_key_in_the_expiry_text_is_dropped(self):
        raw = readout().replace("LIC_DLV_BEGIN", "  Key ABCDE-FGHIJ-KLMNO-PQRST-UVWXY activation 0a1b2c3d-1111-4222-8333-444455556666\nLIC_DLV_BEGIN")
        line = L.state_line(raw, "end", "online")
        self.assertNotRegex(line, "ABCDE|FGHIJ|0a1b2c3d")
        self.assertIn("expiry=Windows", line)

    def test_last_boot(self):
        self.assertEqual(L.last_boot(readout()), "2026-11-17T10:00:00Z")
        self.assertEqual(L.last_boot(""), "")


if __name__ == "__main__":
    unittest.main()
