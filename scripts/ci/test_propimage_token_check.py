#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free tests of scripts/bench/windows/propimage_token_check.py (2026-11-18): no VM, no ssh, no GPU, no real image.

The pure planners and guards are always tested. The tests that create overlays use a tiny FAKE qcow2 chain made with `qemu-img create`
and are skipped, with the reason, when qemu-img is not installed. Nothing here starts a guest; the guest paths (ssh, the PowerShell
report, the firewall lift) are untested and the tool says so."""
import contextlib
import hashlib
import io
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
WIN = ROOT / "scripts/bench/windows"
TOOLS = WIN / "token_check"
sys.path.insert(0, str(WIN))
sys.dont_write_bytecode = True
import propimage as PI  # noqa: E402
import propimage_token_check as T  # noqa: E402

HAVE_QEMU_IMG = shutil.which("qemu-img") is not None
NEEDS_QEMU_IMG = unittest.skipUnless(HAVE_QEMU_IMG, "qemu-img is not installed: the overlay tests need `qemu-img create` for a fake chain")

GOOD_CHECKLIST = (TOOLS / "checklist_template.md").read_text() + "\nHosts: a.example.net, b.example.net\n"
SIGNED_IN_ONLY = "# Checklist\n1. Open the app. PASS: it shows no account. FAIL: it shows signed in.\n"


def fake_state():
    st = PI.init_state("token-check-test-seed")
    files = {"tpm/tpm2-00.permall": b"fake-tpm", "firmware/OVMF_VARS.fd": b"fake-vars"}
    with tempfile.TemporaryDirectory() as d:
        for n, data in files.items():
            (Path(d) / n).parent.mkdir(parents=True, exist_ok=True)
            (Path(d) / n).write_bytes(data)
        PI.capture_files(st, d)
    st["guest"]["ssh_private_key"] = "-----BEGIN FAKE KEY-----\nZmFrZQ==\n-----END FAKE KEY-----\n"
    st["guest"]["ssh_public_key"] = "ssh-ed25519 AAAAfake fake"
    st["clock"]["last_time_at_stop_utc"] = "2026-11-12T17:17:00Z"
    return st


def make_sealed(base, real_qcow2):
    """base/sealed_one/{image.qcow2,state.json,manifest.json}, read-only files like a sealed image."""
    sd = base / "sealed_one"
    sd.mkdir(parents=True)
    img = sd / "image.qcow2"
    if real_qcow2:
        subprocess.run(["qemu-img", "create", "-q", "-f", "qcow2", str(base / "base.qcow2"), "10M"], check=True)
        subprocess.run(["qemu-img", "create", "-q", "-f", "qcow2", "-b", str(base / "base.qcow2"), "-F", "qcow2", str(img)], check=True)
    else:
        img.write_bytes(b"not a real qcow2")
    (sd / "state.json").write_text(json.dumps(fake_state(), indent=1))
    (sd / "manifest.json").write_text(json.dumps({"manifest_version": 1, "members": {"image.qcow2": hashlib.sha256(img.read_bytes()).hexdigest()}}))
    for p in sd.iterdir():
        p.chmod(0o444)
    return sd


def tree_state(path):
    out = {}
    for p in sorted(Path(path).rglob("*")):
        st = p.stat()
        out[str(p)] = (st.st_size, st.st_mtime_ns, st.st_mode, hashlib.sha256(p.read_bytes()).hexdigest() if p.is_file() else "")
    return out


class Fixture(unittest.TestCase):
    real_qcow2 = False

    def setUp(self):
        self.tmp_obj = tempfile.TemporaryDirectory(prefix="token-check-test-")
        self.addCleanup(self.tmp_obj.cleanup)
        self.tmp = Path(self.tmp_obj.name)
        self.sealed_dir = make_sealed(self.tmp / "images", self.real_qcow2)
        self.root = self.tmp / "work"
        self.checklist = self.tmp / "checklist.md"
        self.checklist.write_text(GOOD_CHECKLIST)

    def args(self, cmd, *extra, root=None):
        return [cmd, "--sealed", str(self.sealed_dir), "--checklist", str(self.checklist), "--run-root", str(root or self.root), "--no-gpu", *extra]

    def main(self, *argv):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            rc = T.main(list(argv))
        return rc, out.getvalue(), err.getvalue()

    def ctx(self, cmd="start", *extra):
        return T.context(T.parser().parse_args(self.args(cmd, *extra)))


class ScrubTests(unittest.TestCase):
    def test_emails_are_hidden(self):
        for raw in ("signed in as someone.name+tag@example.org ok", "user@sub.example.co.uk", "A@B.CO"):
            out = T.scrub(raw)
            self.assertNotIn("@", out)
            self.assertIn("<account>", out)

    def test_long_numbers_are_hidden_but_sizes_stay(self):
        out = T.scrub("id=76561198000000000 phone 0612345678 bytes=12345678901 small=12345678 date=2026-11-18")
        self.assertNotIn("76561198000000000", out)
        self.assertNotIn("0612345678", out)
        self.assertIn("bytes=12345678901", out)
        self.assertIn("small=12345678", out)
        self.assertIn("2026-11-18", out)

    def test_token_like_strings_are_hidden(self):
        tok = "A" * 45
        jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.abcdefghijk"
        out = T.scrub("t=%s j=%s g=123e4567-e89b-12d3-a456-426614174000 GPU-12345678-1234-1234-1234-123456789012" % (tok, jwt))
        for secret in (tok, jwt, "123e4567-e89b-12d3-a456-426614174000", "12345678-1234-1234-1234-123456789012"):
            self.assertNotIn(secret, out)

    def test_powershell_scrubbers_match_the_python_ones(self):
        ps = (TOOLS / "indicator_report.ps1").read_text()
        for needle in ("<account>", r"(bytes=\d+)|\d{9,}", "eyJ", "Mskl"):
            self.assertIn(needle, ps)


class ChecklistTests(unittest.TestCase):
    def test_the_template_is_accepted(self):
        self.assertEqual(T.checklist_problems(GOOD_CHECKLIST), [])

    def test_a_signed_in_look_is_not_a_checklist(self):
        probs = T.checklist_problems(SIGNED_IN_ONLY)
        self.assertTrue(any("ROUND-TRIP" in p for p in probs), probs)

    def test_template_teaches_the_measured_lesson(self):
        t = (TOOLS / "checklist_template.md").read_text()
        self.assertIn("MEASURED on 2026-11-18", t)
        self.assertIn('"Shows signed in" is NOT proof', t)
        for must in ("Go Online", "share link", "PASS", "FAIL", "SERVER ROUND-TRIP"):
            self.assertIn(must, t)
        self.assertTrue(t.splitlines()[2].startswith("STATUS: LIVE, 2026-11-18"))

    def test_hosts_line(self):
        self.assertEqual(T.checklist_hosts(GOOD_CHECKLIST), ["a.example.net", "b.example.net"])
        self.assertEqual(T.checklist_hosts("Hosts: <vendor-host-1>, x.example.com"), ["x.example.com"])
        with self.assertRaises(T.Refused):
            T.check_hosts(["bad host;rm"])
        self.assertEqual(T.hosts_prefix(["a.example.net", "b.example.net"]), "$Hosts = @('a.example.net','b.example.net')")


class FirewallRecordTests(unittest.TestCase):
    OPEN = "FW_KF_OFFLINE_GROUP_RULES=0\nFW_KAYFABE_BLOCK_RULES_ENABLED=0\nFW_OUTBOUND_DEFAULT_BLOCK_PROFILES=0\nFW_SEAL_TCP443_1.1.1.1=OPEN\nGUEST_UTC=x\n"
    BLOCKED = ("FW_KF_OFFLINE_GROUP_RULES=3\nFW_KAYFABE_BLOCK_RULES_ENABLED=1\nFW_OUTBOUND_DEFAULT_BLOCK_PROFILES=0\n"
               "FW_SEAL_TCP443_1.1.1.1=FAILED(timeout-or-blocked)\n")

    def test_no_firewall(self):
        line, present = T.firewall_in_seal(self.OPEN)
        self.assertTrue(line.startswith("firewall in seal: no"), line)
        self.assertFalse(present)

    def test_firewall_in_seal(self):
        line, present = T.firewall_in_seal(self.BLOCKED)
        self.assertTrue(line.startswith("firewall in seal: yes"), line)
        self.assertIn("kf-offline group with 3", line)
        self.assertTrue(present)

    def test_only_a_failed_probe_counts_as_yes_but_names_nothing_to_lift(self):
        line, present = T.firewall_in_seal("FW_SEAL_TCP443_1.1.1.1=FAILED(refused-or-error)\n")
        self.assertTrue(line.startswith("firewall in seal: yes"))
        self.assertFalse(present)

    def test_lift_script_only_names_kayfabe_rules_unless_lift_all(self):
        lift = (TOOLS / "fw_lift.ps1").read_text()
        self.assertIn("^(kayfabe|kf-)", lift)
        self.assertIn("kf-offline", lift)
        self.assertRegex(lift, r"if \(\$LiftAll\) \{[^}]*Set-NetFirewallProfile")
        self.assertNotRegex(lift.split("if ($LiftAll)")[0], r"Set-NetFirewallProfile")
        self.assertIn("kf-offline-ssh-in", lift)


class FirewallStepOrder(Fixture):
    """The state of the seal is recorded (evidence file) BEFORE any lift is sent; the lift is optional."""

    def run_step(self, *extra, detect=FirewallRecordTests.BLOCKED):
        c = self.ctx("start", *extra)
        calls = []

        def fake_ssh(st, key, script, prefix="", timeout=900):
            if "FW_PROFILES" in script:
                calls.append("detect")
                return 0, T.scrub(detect), ""
            calls.append("lift")
            ev = list((self.root / "evidence" / c["name"]).glob("*firewall_in_seal.txt"))
            self.assertEqual(len(ev), 1, "the seal's firewall state must be on disk before the lift")
            self.assertTrue(ev[0].read_text().startswith("firewall in seal: "))
            return 0, "LIFTED_RULE=kf-offline-x\n", ""
        out = io.StringIO()
        with mock.patch.object(T, "ssh_ps", fake_ssh), contextlib.redirect_stdout(out):
            T.firewall_step(c)
        return calls, out.getvalue(), c

    def test_default_records_and_does_not_lift(self):
        calls, out, c = self.run_step()
        self.assertEqual(calls, ["detect"])
        self.assertIn("firewall in seal: yes", out)
        self.assertIn("not lifted", out)
        self.assertEqual(len(list((self.root / "evidence" / c["name"]).glob("*firewall_in_seal.txt"))), 1)

    def test_lift_after_record(self):
        calls, out, c = self.run_step("--lift-firewall")
        self.assertEqual(calls, ["detect", "lift"])

    def test_lift_flag_without_a_named_block_lifts_nothing(self):
        calls, out, c = self.run_step("--lift-firewall", detect=FirewallRecordTests.OPEN)
        self.assertEqual(calls, ["detect"])
        self.assertIn("firewall in seal: no", out)

    def test_lift_all_lifts_even_without_a_named_block(self):
        calls, out, c = self.run_step("--lift-all", detect=FirewallRecordTests.OPEN)
        self.assertEqual(calls, ["detect", "lift"])


class ReportTests(Fixture):
    def test_report_output_and_evidence_never_contain_emails_or_long_numbers(self):
        c = self.ctx("report")
        raw = "## Steam\nloginusers.vdf=present users=1\nconnection_log.last=[U:1:123456789] mail me@example.org id 76561198012345678\nKF_TC_REPORT_END\n"
        out = io.StringIO()
        with mock.patch.object(T, "ssh_ps", lambda *a, **k: (0, raw, "")), contextlib.redirect_stdout(out):
            T.run_report(c, "report")
        saved = list((self.root / "evidence" / c["name"]).glob("*_report_report.txt"))
        self.assertEqual(len(saved), 1)
        for text in (out.getvalue(), saved[0].read_text()):
            self.assertNotIn("me@example.org", text)
            self.assertNotIn("76561198012345678", text)
            self.assertNotIn("123456789", text)
            self.assertIn("KF_TC_REPORT_END", text)

    def test_script_is_sent_with_the_hosts_prefix(self):
        c = self.ctx("report")
        seen = {}

        def fake(st, key, script, prefix="", timeout=900):
            seen.update(script=script, prefix=prefix)
            return 0, "KF_TC_REPORT_END\n", ""
        with mock.patch.object(T, "ssh_ps", fake), contextlib.redirect_stdout(io.StringIO()):
            T.run_report(c, "report")
        self.assertEqual(seen["prefix"], "$Hosts = @('a.example.net','b.example.net')")
        self.assertEqual(seen["script"], (TOOLS / "indicator_report.ps1").read_text())

    def test_indicator_report_covers_the_promised_indicators_and_prints_no_values(self):
        ps = (TOOLS / "indicator_report.ps1").read_text()
        ps.encode("ascii")                                                   # Windows PowerShell 5.1, no BOM issues
        for needle in ("ConnectCache", "Accounts", "RememberPassword", "local.vdf", "config.vdf", "loginusers.vdf", "connection_log",
                       "access ?denied", "Adobe", "Autodesk", "Esri", "Siemens", "Edge", "Preferences", "KF_TC_REPORT_END"):
            self.assertIn(needle, ps)
        self.assertNotRegex(ps, r"(?i)get-content[^\n]*\|\s*write-output")
        self.assertNotRegex(ps, r"(?i)\bP\s+'[^']*'\s+\$t\b")                 # a whole file text is never printed
        # the vdf files are only searched for key names (KB) or counted; never printed
        self.assertEqual(len(re.findall(r"function KB\(", ps)), 1)


class GuardTests(Fixture):
    def test_checklist_is_required_and_checked(self):
        self.checklist.write_text(SIGNED_IN_ONLY)
        rc, out, err = self.main(*self.args("status"))
        self.assertEqual(rc, 2)
        self.assertIn("ROUND-TRIP", err)
        rc, out, err = self.main("status", "--sealed", str(self.sealed_dir), "--checklist", str(self.tmp / "nope.md"), "--run-root", str(self.root))
        self.assertEqual(rc, 2)
        self.assertIn("checklist file missing", err)

    def test_run_root_is_required(self):
        with mock.patch.dict(os.environ, {}, clear=False):
            os.environ.pop("KF_TOKENCHECK_ROOT", None)
            rc, out, err = self.main("status", "--sealed", str(self.sealed_dir), "--checklist", str(self.checklist))
        self.assertEqual(rc, 2)
        self.assertIn("--run-root", err)

    def test_refuses_a_run_root_inside_the_sealed_directory(self):
        before = tree_state(self.sealed_dir)
        for root in (self.sealed_dir, self.sealed_dir / "sub", self.sealed_dir.parent / "sealed_one" / "deep" / "er"):
            rc, out, err = self.main(*self.args("prepare", "--dry-run", root=root))
            self.assertEqual(rc, 2, err)
            self.assertIn("refusing to write into the sealed image's directory", err)
        self.assertEqual(tree_state(self.sealed_dir), before)
        self.assertFalse((self.sealed_dir / "sub").exists())
        self.assertFalse((self.sealed_dir / "runs").exists())

    def test_run_root_may_be_a_sibling_of_the_sealed_directory(self):
        rc, out, err = self.main(*self.args("status", root=self.sealed_dir.parent / "work"))
        self.assertEqual(rc, 0, err)

    def test_bad_run_name_is_refused(self):
        for bad in ("../x", "a/b", ".."):
            rc, out, err = self.main(*self.args("status", "--name", bad))
            self.assertNotEqual(rc, 0)

    def test_state_with_a_hand_edited_identity_is_refused(self):
        st = json.loads((self.sealed_dir / "state.json").read_text())
        st["ids"]["net"]["ssh_port"] = 2222
        (self.sealed_dir / "state.json").chmod(0o644)
        (self.sealed_dir / "state.json").write_text(json.dumps(st))
        rc, out, err = self.main(*self.args("status"))
        self.assertEqual(rc, 2)

    def test_remove_run_dir_only_deletes_a_direct_child_of_runs(self):
        (self.root / "runs").mkdir(parents=True)
        outside = self.tmp / "precious"
        outside.mkdir()
        (outside / "f").write_text("keep")
        link = self.root / "runs" / "link"
        link.symlink_to(outside)
        with self.assertRaises(T.Refused):
            T.remove_run_dir(link, self.root)
        with self.assertRaises(T.Refused):
            T.remove_run_dir(outside, self.root)
        with self.assertRaises(T.Refused):
            T.remove_run_dir(self.root / "runs" / "a" / "b", self.root)
        self.assertTrue((outside / "f").exists())

    def test_state_file_alone_works_with_its_image_next_to_it(self):
        s = T.resolve_sealed(self.sealed_dir / "state.json")
        self.assertEqual(s.image, self.sealed_dir / "image.qcow2")
        self.assertIsNotNone(s.manifest_p)

    def test_sha_mismatch_is_refused_and_a_match_is_cached(self):
        s = T.resolve_sealed(self.sealed_dir)
        calls = []

        def hasher(p):
            calls.append(p)
            return hashlib.sha256(Path(p).read_bytes()).hexdigest()
        self.assertEqual(T.verify_sha(s, self.root, True, hasher)[0], "MATCH")
        self.assertEqual(T.verify_sha(s, self.root, True, hasher)[0], "MATCH")
        self.assertEqual(len(calls), 1, "a verified image is hashed once")
        self.assertEqual(T.verify_sha(s, self.tmp / "other", False, hasher)[0], "NOT-CHECKED")
        with self.assertRaises(T.Refused):
            T.verify_sha(s, self.tmp / "third", True, lambda p: "0" * 64)


class ArgvTests(Fixture):
    def test_no_gpu_argv_is_online_real_clock_and_names_only_the_overlay(self):
        c = self.ctx("start")
        argv, banner = T.argv_for(c)
        j = " ".join(argv)
        self.assertIn("CLOCKNET_OK", banner)
        self.assertIn("restrict=off", j)
        self.assertIn("-rtc base=utc", j)
        self.assertIn(str(c["run"] / "clone.qcow2"), j)
        self.assertNotIn(str(c["sealed"].image), j)
        self.assertEqual(argv[argv.index("-name") + 1], "token-check-" + c["name"])
        self.assertEqual(argv[0], T.DISTRO_QEMU)
        self.assertNotIn("kf3-gpu", j)

    def test_kf3_argv_uses_the_broker_and_the_run_copy_of_the_gop(self):
        c = self.ctx("start")
        argv, banner = T.build_guest_argv(c["st"], c["sealed"], c["run"], c["name"], "/opt/kf3/qemu-system-x86_64", False,
                                          sock=Path("/run/user/1/nvkvm/s.sock"), gop_efi=c["run"] / "firmware/kf-gop.signed.efi")
        j = " ".join(argv)
        self.assertIn("kf3-gpu", j)
        self.assertIn("display-broker=/run/user/1/nvkvm/s.sock", j)
        self.assertIn("restrict=off", j)
        with self.assertRaises(T.Refused):
            T.build_guest_argv(c["st"], c["sealed"], c["run"], c["name"], None, False, sock=None, gop_efi=None)

    def test_a_faked_clock_or_offline_argv_is_refused(self):
        c = self.ctx("start")
        argv, _ = T.argv_for(c)
        faked = [("base=2026-01-01T00:00:00,clock=vm,driftfix=slew" if a.startswith("base=utc") else a) for a in argv]
        with self.assertRaises(T.Refused):
            T.assert_online_real_clock(faked, c["sealed"], c["run"])
        offline = [a.replace("restrict=off", "restrict=on") for a in argv]
        with self.assertRaises(T.Refused):
            T.assert_online_real_clock(offline, c["sealed"], c["run"])

    def test_the_sealed_path_in_the_argv_is_refused(self):
        c = self.ctx("start")
        argv, _ = T.argv_for(c)
        bad = [a.replace(str(c["run"] / "clone.qcow2"), str(c["sealed"].image)) for a in argv]
        with self.assertRaises(T.Refused):
            T.assert_online_real_clock(bad, c["sealed"], c["run"])

    def test_overlay_command_has_the_sealed_image_as_backing_file_only(self):
        cmd = T.overlay_cmd("/s/image.qcow2", "/r/clone.qcow2")
        self.assertEqual(cmd[:2], ["qemu-img", "create"])
        self.assertEqual(cmd[cmd.index("-b") + 1], "/s/image.qcow2")
        self.assertEqual(cmd[cmd.index("-F") + 1], "qcow2")
        self.assertEqual(cmd[-1], "/r/clone.qcow2")
        self.assertEqual(cmd.count("/s/image.qcow2"), 1)

    def test_swtpm_state_lives_in_the_run_dir(self):
        cmd = T.swtpm_cmd(Path("/r/x"), ["runuser", "-u", "u", "--"])
        self.assertEqual(cmd[:4], ["runuser", "-u", "u", "--"])
        self.assertIn("dir=/r/x/tpm,mode=0600", cmd)

    def test_start_dry_run_writes_nothing(self):
        before = tree_state(self.tmp)
        rc, out, err = self.main(*self.args("start", "--dry-run"))
        self.assertEqual(rc, 0, err)
        self.assertIn("CLOCKNET_OK", out)
        self.assertEqual(tree_state(self.tmp), before)
        self.assertFalse(self.root.exists())

    def test_the_output_never_contains_the_key_or_the_password(self):
        st = json.loads((self.sealed_dir / "state.json").read_text())
        rc, out, err = self.main(*self.args("start", "--dry-run"))
        for secret in (st["guest"]["password"], "ZmFrZQ==", "BEGIN FAKE KEY"):
            self.assertNotIn(secret, out + err)


@NEEDS_QEMU_IMG
class OverlayTests(Fixture):
    real_qcow2 = True

    def prepare(self, *extra):
        with mock.patch.object(T, "free_gb", lambda p: 100.0), mock.patch.object(T, "apparmor_covers", lambda p: (False, True)):
            return self.main(*self.args("prepare", "--skip-sha", *extra))

    def test_prepare_creates_a_thin_overlay_over_the_read_only_sealed_image(self):
        before = tree_state(self.sealed_dir)
        rc, out, err = self.prepare()
        self.assertEqual(rc, 0, err + out)
        run = self.root / "runs" / "sealed_one"
        info = json.loads(subprocess.run(["qemu-img", "info", "-U", "--output=json", str(run / "clone.qcow2")], capture_output=True, text=True, check=True).stdout)
        self.assertEqual(Path(info["full-backing-filename"]).resolve(), (self.sealed_dir / "image.qcow2").resolve())
        self.assertEqual(info["backing-filename-format"], "qcow2")
        self.assertEqual((run / "tpm/tpm2-00.permall").read_bytes(), b"fake-tpm")
        self.assertEqual((run / "firmware/OVMF_VARS.fd").read_bytes(), b"fake-vars")
        self.assertEqual(oct((run / "id_kf").stat().st_mode & 0o777), "0o600")
        self.assertEqual(tree_state(self.sealed_dir), before, "the sealed directory must be untouched")
        self.assertEqual(json.loads((run / "plan.json").read_text())["name"], "sealed_one")
        self.assertNotIn("ZmFrZQ", out + err)

    def test_prepare_refuses_when_the_run_dir_exists_and_leaves_it_alone(self):
        self.assertEqual(self.prepare()[0], 0)
        run = self.root / "runs" / "sealed_one"
        marker = run / "marker"
        marker.write_text("x")
        rc, out, err = self.prepare()
        self.assertEqual(rc, 2)
        self.assertIn("exists", err)
        self.assertEqual(marker.read_text(), "x")
        rc, out, err = self.main(*self.args("prepare", "--dry-run"))
        self.assertEqual(rc, 2)

    def test_every_prepare_is_a_fresh_overlay_with_fresh_copies(self):
        self.assertEqual(self.prepare()[0], 0)
        run = self.root / "runs" / "sealed_one"
        (run / "tpm/tpm2-00.permall").write_bytes(b"modified by a guest run")
        self.assertEqual(self.main(*self.args("end"))[0], 0)
        self.assertEqual(self.prepare()[0], 0)
        self.assertEqual((run / "tpm/tpm2-00.permall").read_bytes(), b"fake-tpm")

    def test_prepare_dry_run_writes_nothing(self):
        before = tree_state(self.tmp)
        with mock.patch.object(T, "apparmor_covers", lambda p: (False, True)):
            rc, out, err = self.main(*self.args("prepare", "--dry-run"))
        self.assertEqual(rc, 0, err)
        self.assertIn("CLOCKNET_OK", out)
        self.assertEqual(tree_state(self.tmp), before)

    def test_end_deletes_the_whole_run_dir_and_keeps_scrubbed_evidence_only(self):
        self.assertEqual(self.prepare()[0], 0)
        run = self.root / "runs" / "sealed_one"
        (run / "run").mkdir(exist_ok=True)
        (run / "run/qemu.log").write_text("log")
        (run / "extra_dir").mkdir()
        (run / "extra_dir/file").write_text("x")
        ev = self.root / "evidence" / "sealed_one"
        ev.mkdir(parents=True)
        (ev / "r.txt").write_text("kept")
        before = tree_state(self.sealed_dir)
        rc, out, err = self.main(*self.args("end"))
        self.assertEqual(rc, 0, err)
        self.assertFalse(run.exists())
        self.assertEqual(sorted(p.name for p in (self.root / "runs").iterdir()), [])
        self.assertTrue((ev / "r.txt").exists())
        self.assertEqual(tree_state(self.sealed_dir), before)
        self.assertTrue((self.sealed_dir / "image.qcow2").exists())
        rc, out, err = self.main(*self.args("end"))          # a second end has nothing to do
        self.assertEqual(rc, 0)
        self.assertIn("nothing to end", out)

    def test_status_reports_the_run(self):
        self.assertEqual(self.prepare()[0], 0)
        rc, out, err = self.main(*self.args("status"))
        self.assertIn("exists=True", out)


class LintTests(unittest.TestCase):
    """No image names, no personal data and no institution names in the tool or its templates (owner rule, 2026-11-18)."""

    FILES = [WIN / "propimage_token_check.py"] + sorted(TOOLS.iterdir())
    ID_PATTERNS = [
        r"(?i)\b(?:image|propimage|checklist|sealed|seal)[ _-]*[ABCD]\b",
        r"(?i)\bpropimage_[abcd]\b",
        r"(?i)\bchecklist_[abcd]\b",
        r"""["'][ABCD]["']\s*:""",
        r"""\(\s*["']?A["']?\s*\|\s*B""",
        r"\bA\|B\|D\b",
        r"(?i)\bimages?\s+[ABCD]\s*(?:,|/|and|or)\s*[ABCD]\b",
        r"(?i)\bD check\b",
    ]
    PERSONAL = [
        "rein" + "der", "pels" + "ma",                                   # the maintainer's name
        r"[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+\.[A-Za-z]{2,}",               # any e-mail address
        r"(?i)univers" + "it", r"(?i)\bcam" + r"pus\b", r"(?i)hoch" + "schule", r"(?i)\.e" + r"du\b", r"(?i)\.ac\.[a-z]{2}\b",
        r"(?i)\bTU[ -][A-Z]", r"(?i)\binsti" + r"tute of\b", r"(?i)\bvpn\b",
    ]
    PC_SPECIFIC = [r"/workspace/", r"/root/prov", r"\bpc1\b", r"PC_ACTIVE", r"\bwayland-0\b.*ubuntu", r"/run/user/1000", r'"ubuntu"']

    def test_files_exist(self):
        names = {p.name for p in self.FILES}
        for need in ("propimage_token_check.py", "indicator_report.ps1", "checklist_template.md", "fw_detect.ps1", "fw_lift.ps1"):
            self.assertIn(need, names)

    def scan(self, patterns, label):
        bad = []
        for f in self.FILES:
            text = f.read_text()
            for pat in patterns:
                for m in re.finditer(pat, text):
                    bad.append("%s: %s: %r" % (f.name, label, m.group(0)))
        self.assertEqual(bad, [])

    def test_no_image_ids(self):
        self.scan(self.ID_PATTERNS, "image id")

    def test_no_personal_data_or_institutions(self):
        self.scan(self.PERSONAL, "personal")

    def test_no_machine_specific_paths(self):
        self.scan(self.PC_SPECIFIC, "pc-specific")

    def test_lint_catches_what_it_should(self):
        for sample in ("image A is sealed", "CHECKLIST_D.md", "propimage_b/bin", '"D": {', "token_check.py A|B|D prepare"):
            self.assertTrue(any(re.search(p, sample) for p in self.ID_PATTERNS), sample)
        for sample in ("someone@example.org", "a Univer" + "sity", "TU Xyz"):
            self.assertTrue(any(re.search(p, sample) for p in self.PERSONAL), sample)

    def test_status_line_and_generic_cli(self):
        src = (WIN / "propimage_token_check.py").read_text()
        self.assertIn("STATUS: LIVE, 2026-11-18", src.split('"""')[1])
        helptext = T.parser().format_help()
        for opt in ("--sealed", "--checklist", "--run-root", "--hosts", "--lift-firewall", "--gpu-lock", "--desktop-user"):
            self.assertIn(opt, helptext)
        self.assertNotRegex(helptext, r"\bchoices\b.*\b[ABD]\b")


if __name__ == "__main__":
    unittest.main()
