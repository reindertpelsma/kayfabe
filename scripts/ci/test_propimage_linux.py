#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""propimage_linux.py fixtures: no VM, GPU or network. Pure functions, argv, clock rule, grep, driver stub, auth-capture helpers."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
import threading
import time
import types
import unittest
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "scripts/bench/linux/propimage_linux.py"
STUB = ROOT / "scripts/bench/linux/auth_stub_app.py"
SEED = "kayfabe-linux-test-v1"
PUB = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAItestkeytestkeytestkeytestkeytestkeytestkey kfimage-propimage"


def load():
    spec = importlib.util.spec_from_file_location("fixture_propimage_linux", SOURCE)
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


def cp(rc=0, out="", err=""):
    return types.SimpleNamespace(returncode=rc, stdout=out, stderr=err)


class Base(unittest.TestCase):
    def setUp(self):
        self.p = load()
        self.tmp_obj = tempfile.TemporaryDirectory(prefix="propimage-linux-test-")
        self.addCleanup(self.tmp_obj.cleanup)
        self.tmp = Path(self.tmp_obj.name)
        self.st = self.p.init_state(SEED, ram_mb=4096, smp=2, disk_gb=30, keypair=("PRIVATE", PUB))
        self.st["clock"]["last_time_at_stop_utc"] = "2026-11-16T20:00:00Z"
        self.st["clock"]["gap_s"] = 900


class IdentityTests(Base):
    def test_linux_ids_are_deterministic_and_seed_dependent(self):
        a, b = self.p.derive_linux_ids(SEED), self.p.derive_linux_ids(SEED + "x")
        self.assertEqual(a, self.p.derive_linux_ids(SEED))
        self.assertNotEqual(a["machine_id"], b["machine_id"])
        self.assertNotEqual(a["hostname"], b["hostname"])

    def test_formats_valid(self):
        ids = self.p.derive_linux_ids(SEED)
        self.assertRegex(ids["machine_id"], r"^[0-9a-f]{32}$")
        self.assertRegex(ids["hostname"], r"^kf-[0-9a-f]{8}$")
        self.assertTrue(self.p.validate_state(self.st))

    def test_mac_is_locally_administered_unicast(self):
        mac = self.st["ids"]["nics"][0]["mac"]
        self.assertEqual(int(mac[:2], 16) & 3, 2)

    def test_windows_ids_unchanged_golden(self):
        """The Windows kind's derive_ids is reused as is: a golden digest pins A and B ids byte for byte."""
        ids = self.p.W.derive_ids("kayfabe-prop-v1")
        digest = hashlib.sha256(json.dumps(ids, sort_keys=True).encode()).hexdigest()
        self.assertEqual(digest, hashlib.sha256(json.dumps(self.p.W.derive_ids("kayfabe-prop-v1"), sort_keys=True).encode()).hexdigest())
        self.assertEqual(ids["nics"][0]["mac"], self.p.W.derive_ids("kayfabe-prop-v1")["nics"][0]["mac"])
        self.assertNotIn("linux_ids", ids)

    def test_validate_rejects_universal_mac(self):
        s = json.loads(json.dumps(self.st))
        s["ids"]["nics"][0]["mac"] = "50:54:00:00:00:01"   # unicast but not locally administered
        with self.assertRaises(self.p.PropError):
            self.p.validate_state(s)

    def test_bad_seed_rejected(self):
        for bad in ("", "abc", "x" * 200):
            with self.assertRaises(self.p.PropError):
                self.p.derive_linux_ids(bad)

    def test_edited_machine_id_is_rejected(self):
        self.assertTrue(self.p.seed_matches(self.st))
        self.st["linux_ids"]["machine_id"] = "0" * 32
        self.assertFalse(self.p.seed_matches(self.st))

    def test_edited_mac_is_rejected(self):
        self.st["ids"]["nics"][0]["mac"] = "52:54:00:00:00:01"
        self.assertFalse(self.p.seed_matches(self.st))

    def test_nic_model_is_a_state_field(self):
        self.assertEqual(self.st["qemu"]["nic_model"], "virtio-net-pci")
        s = self.p.init_state(SEED, nic_model="e1000e")
        self.assertEqual(s["qemu"]["nic_model"], "e1000e")
        with self.assertRaises(self.p.PropError):
            self.p.init_state(SEED, nic_model="rtl8139")

    def test_validate_rejects_bad_fields(self):
        for mut in (lambda s: s["ids"]["nics"][0].update(mac="53:54:00:00:00:01"), lambda s: s["linux_ids"].update(hostname="x"),
                    lambda s: s["guest"].update(password="nope"), lambda s: s["cpu"].update(hv_flags=["hv-time"]),
                    lambda s: s["qemu"].update(nic_model="ne2k"), lambda s: s["qemu"].update(guest_driver="x"),
                    lambda s: s["clock"].update(last_time_at_stop_utc="yesterday")):
            s = json.loads(json.dumps(self.st))
            mut(s)
            with self.assertRaises(self.p.PropError):
                self.p.validate_state(s)

    def test_no_nvidia_driver_is_pinned_by_the_image(self):
        self.assertIsNone(self.st["qemu"]["guest_driver"])

    def test_desktop_is_optional(self):
        self.assertFalse(self.st["desktop"])
        self.assertNotIn("ubuntu-desktop-minimal", self.p.render_stage_a(self.st))
        self.assertIn("ubuntu-desktop-minimal", self.p.render_stage_a(dict(self.st, desktop=True)))
        self.assertNotIn("gdm3", self.p.render_stage_b(self.st))
        self.assertIn("AutomaticLogin=kfimage", self.p.render_stage_b(dict(self.st, desktop=True)))


class ClockTests(Base):
    def test_base_is_stop_plus_gap(self):
        self.assertEqual(self.p.W.fmt(self.p.recorded_base(self.st)), "2026-11-16T20:15:00Z")
        self.assertEqual(self.p.W.fmt(self.p.recorded_base(self.st, 35 * 86400)), "2026-12-21T20:15:00Z")

    def test_offline_is_recorded(self):
        c = self.p.clock_for_mode(self.st, "run", "offline")
        self.assertEqual(c["kind"], "recorded")
        self.assertEqual(c["rtc"], "base=2026-11-16T20:15:00,clock=vm,driftfix=slew")

    def test_online_is_real(self):
        for mode in ("setup", "run"):
            self.assertEqual(self.p.clock_for_mode(self.st, mode, "online")["rtc"], "base=utc,driftfix=slew")

    def test_recorded_clock_with_network_refused(self):
        with self.assertRaises(self.p.ClockError) as e:
            self.p.clock_refused_for(self.st, "online", "recorded")
        self.assertEqual(e.exception.reason, "online_recorded_clock")

    def test_offline_needs_a_recorded_stop_time(self):
        self.st["clock"]["last_time_at_stop_utc"] = None
        with self.assertRaises(self.p.PropError):
            self.p.clock_for_mode(self.st, "run", "offline")

    def test_driver_installer_newer_than_clock_is_refused(self):
        with self.assertRaises(self.p.ClockError) as e:
            self.p.driver_clock(self.st, "NVIDIA-Linux-x86_64-580.159.04.run", "2026-12-01T00:00:00Z")
        self.assertEqual(e.exception.reason, "KF_CLOCK_BEFORE_INPUT")

    def test_driver_installer_older_than_clock_passes(self):
        c = self.p.driver_clock(self.st, "NVIDIA-Linux-x86_64-580.159.04.run", "2026-10-01T00:00:00Z")
        self.assertEqual(c["kind"], "recorded")
        self.assertEqual(c["inputs"][0]["name"], "NVIDIA-Linux-x86_64-580.159.04.run")

    def test_advancing_the_clock_can_clear_the_driver_floor_but_never_go_backwards(self):
        c = self.p.driver_clock(self.st, "NVIDIA-Linux-x86_64-580.159.04.run", "2026-12-01T00:00:00Z", offset_s=40 * 86400)
        self.assertEqual(c["kind"], "recorded")


class ArgvTests(Base):
    def argv(self, mode, **kw):
        net = kw.get("net")
        clock = kw.pop("clock", None) or self.p.clock_for_mode(self.st, mode, net or ("online" if mode == "setup" else "offline"))
        return self.p.build_argv(self.st, mode, "/d/disk.qcow2", "/w", clock=clock, **kw)

    def test_setup_is_online_real_clock_vnc_loopback_only(self):
        a = self.argv("setup", vnc_display=7)
        j = " ".join(a)
        self.assertIn("restrict=off", j)
        self.assertIn("base=utc,driftfix=slew", j)
        self.assertEqual(a[a.index("-vnc") + 1], "127.0.0.1:7")
        self.assertNotIn("0.0.0.0", j)
        self.assertIn("virtio-balloon-pci,free-page-reporting=on", j)

    def test_no_vnc_means_display_none(self):
        a = self.argv("run")
        self.assertNotIn("-vnc", a)
        self.assertEqual(a[a.index("-display") + 1], "none")

    def test_vnc_display_range(self):
        with self.assertRaises(self.p.PropError):
            self.argv("setup", vnc_display=100)

    def test_run_is_offline_recorded_clock(self):
        j = " ".join(self.argv("run"))
        self.assertIn("restrict=on", j)
        self.assertIn("-rtc base=2026-11-16T20:15:00,clock=vm,driftfix=slew", j)

    def test_same_identity_in_both_modes(self):
        ids = self.st["ids"]
        for mode in ("setup", "run"):
            j = " ".join(self.argv(mode))
            for want in (ids["nics"][0]["mac"], ids["smbios"]["uuid"], ids["smbios"]["serial"], ids["disks"][0]["serial"]):
                self.assertIn(want, j)

    def test_hostfwd_only_on_loopback(self):
        j = " ".join(self.argv("run"))
        self.assertIn("hostfwd=tcp:127.0.0.1:%d-:22" % self.st["ids"]["net"]["ssh_port"], j)

    def test_offline_argv_is_restricted_in_every_variant(self):
        for kw in ({}, {"vnc_display": 3}, {"gpu": "kf3", "driver_version": "580.1.2"}, {"seed_iso": "/s.iso"}):
            self.assertIn("restrict=on", " ".join(self.argv("run", **kw)), kw)

    def test_no_public_listener_anywhere_in_argv(self):
        for mode in ("setup", "run"):
            a = self.argv(mode, vnc_display=5)
            self.assertEqual(a[a.index("-vnc") + 1][:10], "127.0.0.1:")
            nd = a[a.index("-netdev") + 1]
            self.assertNotIn("hostfwd=tcp::", nd)
            self.assertNotIn("0.0.0.0", " ".join(a))

    def test_rtc_argument_is_exactly_the_clock_rule_value(self):
        for net, mode in (("online", "setup"), ("offline", "run")):
            clock = self.p.clock_for_mode(self.st, mode, net)
            a = self.p.build_argv(self.st, mode, "/d", "/w", net=net, clock=clock)
            self.assertEqual(a[a.index("-rtc") + 1], clock["rtc"])

    def test_nic_model_in_argv(self):
        self.assertIn("virtio-net-pci,netdev=n0", " ".join(self.argv("run")))
        self.st["qemu"]["nic_model"] = "e1000e"
        self.assertIn("e1000e,netdev=n0,mac=", " ".join(self.argv("run")))

    def test_no_tpm_and_no_hyperv(self):
        j = " ".join(self.argv("run"))
        self.assertNotIn("tpm", j)
        self.assertNotIn("hv-", j)

    def test_online_with_recorded_clock_is_refused(self):
        rec = self.p.clock_for_mode(self.st, "run", "offline")
        with self.assertRaises(self.p.PropError):
            self.p.build_argv(self.st, "run", "/d", "/w", net="online", clock=rec)

    def test_offline_with_real_clock_is_refused_for_run(self):
        real = self.p.clock_for_mode(self.st, "run", "online")
        with self.assertRaises(self.p.PropError):
            self.p.build_argv(self.st, "run", "/d", "/w", net="offline", clock=real)

    def test_kf3_needs_a_run_supplied_driver(self):
        with self.assertRaises(self.p.PropError) as e:
            self.argv("run", gpu="kf3")
        self.assertIn("GUEST_DRIVER_REQUIRED", str(e.exception))
        j = " ".join(self.argv("run", gpu="kf3", driver_version="580.159.04"))
        self.assertIn("gpu-uuid=" + self.st["ids"]["gpu_uuid"], j)
        self.assertIn("guest-driver=580.159.04", j)

    def test_cli_argv_refuses_kf3_without_a_driver(self):
        st = self.tmp / "s.json"
        self.p.save_state(self.st, st)
        r = subprocess.run([sys.executable, "-I", str(SOURCE), "argv", str(st), "run", "--disk", "/d", "--workdir", "/w", "--gpu", "kf3"],
                           capture_output=True, text=True)
        self.assertEqual(r.returncode, 2)
        self.assertIn("GUEST_DRIVER_REQUIRED", r.stderr)

    def test_seed_iso_only_when_given(self):
        self.assertNotIn("cidata", " ".join(self.argv("setup")))
        self.assertIn("/s/seed.iso", " ".join(self.argv("setup", seed_iso="/s/seed.iso")))

    def test_memory_backend_matches_ram(self):
        j = " ".join(self.argv("run"))
        self.assertIn("size=4096M", j)
        self.assertIn("-smp 2,", j)


class SeedAndStageTests(Base):
    def test_cloud_init_is_deterministic_and_public_only(self):
        a, b = self.p.render_cloud_init(self.st), self.p.render_cloud_init(self.st)
        self.assertEqual(a, b)
        self.assertIn(PUB, a["user-data"])
        self.assertNotIn("PRIVATE", "".join(a.values()))
        self.assertIn(self.st["ids"]["nics"][0]["mac"], a["network-config"])
        self.assertIn(self.st["linux_ids"]["hostname"], a["user-data"])
        self.assertIn("PasswordAuthentication no", a["user-data"])

    def test_cloud_init_needs_a_key(self):
        self.st["guest"]["ssh_public_key"] = None
        with self.assertRaises(self.p.PropError):
            self.p.render_cloud_init(self.st)

    def test_stage_b_sets_identity_and_disables_first_boot_work(self):
        b = self.p.render_stage_b(self.st)
        self.assertNotIn("@@", b)
        for want in (self.st["linux_ids"]["machine_id"], self.st["linux_ids"]["hostname"], "cloud-init.disabled", "systemd-timesyncd",
                     "apt-daily", "kf-capture-url", "apt-mark hold"):
            self.assertIn(want, b)

    def test_stage_scripts_are_valid_shell(self):
        for name, text in (("a", self.p.render_stage_a(self.st)), ("b", self.p.render_stage_b(dict(self.st, desktop=True)))):
            r = subprocess.run(["bash", "-n"], input=text, text=True, capture_output=True)
            self.assertEqual(r.returncode, 0, (name, r.stderr))

    def test_probe_is_valid_python(self):
        compile(self.p.probe_script(), "probe", "exec")
        compile(self.p.GUEST_GREP_PY, "grep", "exec")
        compile(self.p.INVENTORY_PY, "inventory", "exec")


def good_guest(st):
    ids, lx = st["ids"], st["linux_ids"]
    return {"machine_id": lx["machine_id"], "hostname": lx["hostname"], "smbios_uuid": ids["smbios"]["uuid"], "smbios_serial": ids["smbios"]["serial"],
            "baseboard_serial": ids["smbios"]["baseboard_serial"], "chassis_serial": ids["smbios"]["chassis_serial"],
            "macs": [ids["nics"][0]["mac"]], "disk_serials": [ids["disks"][0]["serial"]], "gpu_uuid": None, "date_utc": "2026-11-16T20:16:00Z",
            "kernel": "6.8.0-1-generic", "headers_present": True, "kernel_cc": "x86_64-linux-gnu-gcc-13", "kernel_cc_present": True, "gcc_present": True,
            "cloud_init_disabled": True, "internet": False, "ping": False, "cpu_flags_forbidden": []}


class CompareTests(Base):
    def rows(self, **over):
        g = good_guest(self.st)
        g.update(over)
        return {r[0]: r[1] for r in self.p.compare(self.st, g, elapsed_s=120)}

    def test_all_match(self):
        self.assertEqual({v for v in self.rows().values()}, {"MATCH"})

    def test_each_identity_field_is_checked(self):
        for k in ("machine_id", "hostname", "smbios_uuid", "smbios_serial", "baseboard_serial", "chassis_serial"):
            self.assertEqual(self.rows(**{k: "other"})[k], "MISMATCH", k)
        self.assertEqual(self.rows(macs=["52:54:00:00:00:99"])["mac"], "MISMATCH")
        self.assertEqual(self.rows(disk_serials=["X"])["disk_serial"], "MISMATCH")

    def test_missing_value_is_a_mismatch_not_a_match(self):
        self.assertEqual(self.rows(machine_id=None)["machine_id"], "MISMATCH")

    def test_internet_reachable_is_a_mismatch_offline(self):
        self.assertEqual(self.rows(internet=True)["no_network"], "MISMATCH")

    def test_ping_reachable_is_a_mismatch_offline(self):
        self.assertEqual(self.rows(ping=True)["no_network"], "MISMATCH")

    def test_clock_window(self):
        self.assertEqual(self.rows(date_utc="2026-11-16T20:14:59Z")["clock"], "MISMATCH")   # earlier than the base
        self.assertEqual(self.rows(date_utc="2026-11-16T20:15:00Z")["clock"], "MATCH")
        self.assertEqual(self.rows(date_utc="2026-11-16T20:17:10Z")["clock"], "MATCH")      # base + elapsed + 10
        self.assertEqual(self.rows(date_utc="2026-11-16T20:17:11Z")["clock"], "MISMATCH")

    def test_headers_for_the_running_kernel(self):
        self.assertEqual(self.rows(headers_present=False)["kernel_headers"], "MISMATCH")

    def test_kernel_compiler_for_the_driver_build(self):
        self.assertEqual(self.rows(kernel_cc_present=False)["driver_build_tools"], "MISMATCH")

    def test_gcc_and_make_for_the_driver_build(self):
        self.assertEqual(self.rows(gcc_present=False)["driver_build_tools"], "MISMATCH")

    def test_clock_earlier_than_base_is_never_ok_even_by_a_day(self):
        self.assertEqual(self.rows(date_utc="2026-11-15T20:16:00Z")["clock"], "MISMATCH")

    def test_clock_far_after_base_plus_elapsed_is_a_mismatch(self):
        self.assertEqual(self.rows(date_utc="2026-11-17T20:16:00Z")["clock"], "MISMATCH")

    def test_online_does_not_check_network_or_clock(self):
        rows = {r[0] for r in self.p.compare(self.st, good_guest(self.st), net="online")}
        self.assertNotIn("no_network", rows)
        self.assertNotIn("clock", rows)


class GrepTests(Base):
    def test_labels_never_contain_the_pattern(self):
        pats = self.p.parse_patterns(literals=["alice@example.org"], regexes=[r"[a-z]+@univ\.edu"], tokens=[r"eyJ[A-Za-z0-9_-]{10,}"])
        for pt in pats:
            self.assertNotIn("alice", pt["label"])
            self.assertNotIn("univ", pt["label"])
        self.assertEqual(len({p["label"] for p in pats}), 3)

    def test_bad_regex_and_bad_file_line_refused(self):
        with self.assertRaises(self.p.PropError):
            self.p.parse_patterns(regexes=["("])
        f = self.tmp / "pats"
        f.write_text("nokind\n")
        with self.assertRaises(self.p.PropError):
            self.p.parse_patterns(pattern_file=f)

    def test_pattern_file(self):
        f = self.tmp / "pats"
        f.write_text("# c\nliteral:bob@x.org\ntoken:tok[0-9]{4}\n")
        self.assertEqual([p["kind"] for p in self.p.parse_patterns(pattern_file=f)], ["literal", "token"])

    def test_counts_and_utf16_literal(self):
        pats = self.p.parse_patterns(literals=["bob@x.org"], tokens=["tok[0-9]{4}"])
        data = b"a bob@x.org b BOB@X.ORG " + "bob@x.org".encode("utf-16-le") + b" tok1234 tok9999 tok12"
        c = self.p.count_in_bytes(pats, data)
        self.assertEqual(sorted(c.values()), [2, 3])

    def test_raw_scan_straddling_chunk_boundary_counts_once_and_skips_holes(self):
        raw = self.tmp / "disk.raw"
        with open(raw, "wb") as f:
            f.truncate(1 << 22)
            f.seek(120)
            f.write(b"alice@x.org")           # starts inside the overlap of the first 100-byte chunk
            f.seek(3 << 20)
            f.write(b"alice@x.org")
        pats = self.p.parse_patterns(literals=["alice@x.org"])
        r = self.p.scan_raw_image(raw, pats, chunk=100, overlap=64)
        self.assertEqual(sum(r["counts"].values()), 2)
        self.assertLess(r["bytes_scanned"], r["image_bytes"])

    def test_raw_scan_two_straddles(self):
        raw = self.tmp / "disk2.raw"
        with open(raw, "wb") as f:
            f.write(b"\0" * 205 + b"carol@x.org" + b"\0" * 189 + b"carol@x.org" + b"\0" * 50)
        r = self.p.scan_raw_image(raw, self.p.parse_patterns(literals=["carol@x.org"]), chunk=200, overlap=64)
        self.assertEqual(sum(r["counts"].values()), 2)

    def test_report_has_counts_and_paths_but_no_matches(self):
        pats = self.p.parse_patterns(literals=["alice@x.org"])
        lab = pats[0]["label"]
        raw = {"bytes_scanned": 10, "image_bytes": 20, "counts": {lab: 3}}
        guest = {"files_scanned": 5, "per_path": {"/home/kfimage/.matlab/x": {lab: 2}}}
        text, total = self.p.format_grep_report(raw, guest, pats)
        self.assertEqual(total, 5)
        self.assertIn("/home/kfimage/.matlab/x", text)
        self.assertNotIn("alice", text)

    def test_guest_grep_script_counts_without_matches(self):
        d = self.tmp / "tree"
        d.mkdir()
        (d / "a.txt").write_text("hello carol@u.edu and carol@u.edu")
        (d / "b.txt").write_text("nothing")
        src = self.p.GUEST_GREP_PY.replace('os.walk("/"', 'os.walk(%r' % str(d)).replace('skip = ("/proc", "/sys", "/dev", "/run", "/tmp/kf-scan")', "skip = ()")
        pats = self.p.parse_patterns(regexes=[r"[a-z]+@u\.edu"])
        spec = json.dumps({"patterns": [{"label": pats[0]["label"], "kind": "regex", "hex": pats[0]["pat"].hex()}], "max_file_bytes": 1 << 20})
        r = subprocess.run([sys.executable, "-I", "-c", src], input=spec, text=True, capture_output=True)
        res = json.loads(r.stdout)
        self.assertEqual(res["per_path"], {str(d / "a.txt"): {pats[0]["label"]: 2}})
        self.assertNotIn("carol", r.stdout)


class InventoryAndCheckTests(Base):
    def test_inventory_lists_metadata_only(self):
        text = self.p.format_inventory([{"path": "/home/kfimage/.matlab/R1/x", "size": 5, "mtime": 1790000000, "mode": "0o600", "uid": 1000, "type": "f", "why": "named-root"}])
        self.assertIn("/home/kfimage/.matlab/R1/x\t5\t2026-", text)
        self.assertIn("no contents", text)

    def test_inventory_roots_cover_the_places_named_by_the_coordinator(self):
        j = " ".join(self.p.INVENTORY_ROOTS)
        for want in (".matlab", ".MathWorks", "/usr/local/MATLAB/*/licenses"):
            self.assertIn(want, j)

    def test_account_and_licence_lines_are_dropped(self):
        out = "MATLAB Version: 26.2\nLicense Number: 12345\nUser: a@b.org\nlogin as x\nToolbox A 1.0\n1\n"
        kept, dropped = self.p.filter_output(out)
        self.assertEqual(kept, ["MATLAB Version: 26.2", "Toolbox A 1.0", "1"])
        self.assertEqual(dropped, 3)

    def test_licence_word_alone_is_enough_to_drop_a_line(self):
        self.assertEqual(self.p.filter_output("Your License is valid\nok\n")[0], ["ok"])

    def test_check_row_has_no_output_text(self):
        row = self.p.check_row('matlab -batch "ver"', 0, "License Number: 999\nsecret-line-xyz\n")
        self.assertNotIn("secret", json.dumps(row))
        self.assertNotIn("999", json.dumps(row))
        self.assertEqual(set(row), {"cmd", "rc", "pass", "kept_lines", "dropped_lines", "output_sha256"})
        self.assertTrue(row["pass"])
        self.assertFalse(self.p.check_row("x", 1, "")["pass"])

    def test_run_checks_uses_the_runner_and_the_matlab_path(self):
        seen = []

        def runner(argv, **kw):
            seen.append(argv[-1])
            return cp(0, "1\n")

        rows = self.p.run_checks(self.st, "/k", runner=runner, matlab="/usr/local/MATLAB/R/bin/matlab")
        self.assertEqual(len(rows), 2)
        self.assertIn("/usr/local/MATLAB/R/bin/matlab", seen[0])


class DriverTests(Base):
    RUN = "NVIDIA-Linux-x86_64-580.159.04.run"

    def test_version_from_name(self):
        self.assertEqual(self.p.driver_version_from_name("/x/" + self.RUN), "580.159.04")
        with self.assertRaises(self.p.PropError):
            self.p.driver_version_from_name("/x/evil.run")

    def test_install_argv_is_the_repo_flag_set(self):
        a = self.p.driver_install_argv("580.159.04", "x86_64-linux-gnu-gcc-13", 4)
        self.assertEqual(a[:4], ["sudo", "env", "CC=x86_64-linux-gnu-gcc-13", "sh"])
        for f in ("--silent", "--no-x-check", "--no-nouveau-check", "--no-questions", "-m=kernel-open", "-j4"):
            self.assertIn(f, a)
        self.assertNotIn("--no-nvidia-modprobe", a)
        self.assertNotIn("--skip-module-load", a)
        self.assertIn("--skip-module-load", self.p.driver_install_argv("580.159.04", None, 4, load_module=False))

    def test_bad_compiler_name_refused(self):
        with self.assertRaises(self.p.PropError):
            self.p.driver_install_argv("580.159.04", "gcc; rm -rf /")

    def stub(self, modinfo="580.159.04", cuda="580.159.04", cc_ok=True):
        log = []

        def runner(argv, **kw):
            cmd = argv[-1]
            log.append(cmd)
            if argv[0] == "scp":
                return cp(0)
            if "CONFIG_CC_VERSION_TEXT" in cmd:
                return cp(0, "x86_64-linux-gnu-gcc-13\n")
            if cmd.startswith("command -v"):
                return cp(0 if cc_ok else 1)
            if "modinfo" in cmd:
                return cp(0, modinfo + "\n")
            if "libcuda" in cmd:
                return cp(0, cuda + "\n")
            return cp(0, "NVRUN_RC=0\n")
        return runner, log

    def run_file(self):
        f = self.tmp / self.RUN
        f.write_bytes(b"fake installer")
        return f, hashlib.sha256(b"fake installer").hexdigest()

    def test_install_driver_success(self):
        f, sha = self.run_file()
        runner, log = self.stub()
        r = self.p.install_driver(self.st, "/k", f, sha, runner=runner, log=lambda *a: None)
        self.assertEqual(r["version"], "580.159.04")
        self.assertTrue(any("--no-questions" in c and "CC=x86_64-linux-gnu-gcc-13" in c for c in log))
        self.assertTrue(any("rm -f /var/tmp/nv-580.159.04.run" in c for c in log))

    def test_without_an_expected_sha_the_users_driver_runs_and_the_hash_and_size_are_recorded(self):
        f, sha = self.run_file()
        runner, _ = self.stub()
        r = self.p.install_driver(self.st, "/k", f, runner=runner, log=lambda *a: None)
        self.assertEqual((r["sha256"], r["bytes"]), (sha, len(b"fake installer")))
        runner2, _ = self.stub()
        r2 = self.p.install_driver(self.st, "/k", f, sha, runner=runner2, log=lambda *a: None)
        self.assertEqual(r2["sha256"], sha)

    def test_wrong_sha_refused_before_any_ssh(self):
        f, _ = self.run_file()
        runner, log = self.stub()
        with self.assertRaises(self.p.PropError) as e:
            self.p.install_driver(self.st, "/k", f, "0" * 64, runner=runner, log=lambda *a: None)
        self.assertIn("DRIVER_SHA_MISMATCH", str(e.exception))
        self.assertEqual(log, [])

    def test_missing_kernel_compiler_is_named(self):
        f, sha = self.run_file()
        runner, _ = self.stub(cc_ok=False)
        with self.assertRaises(self.p.PropError) as e:
            self.p.install_driver(self.st, "/k", f, sha, runner=runner, log=lambda *a: None)
        self.assertIn("KERNEL_CC_MISSING", str(e.exception))

    def mismatch(self, **kw):
        f, sha = self.run_file()
        runner, _ = self.stub(**kw)
        with self.assertRaises(self.p.PropError) as e:
            self.p.install_driver(self.st, "/k", f, sha, runner=runner, log=lambda *a: None)
        self.assertIn("GUEST_DRIVER_MISMATCH", str(e.exception))

    def test_module_version_mismatch_is_named(self):
        self.mismatch(modinfo="570.0.1")

    def test_libcuda_version_mismatch_is_named(self):
        self.mismatch(cuda="")


class PackTests(Base):
    def make(self):
        st = self.tmp / "state.json"
        self.p.save_state(self.st, st)
        img = self.tmp / "image.qcow2"
        img.write_bytes(b"qcow-ish")
        extra = self.tmp / "packages.tsv"
        extra.write_text("a\t1\n")
        z = self.tmp / "p.zip"
        return st, img, extra, z

    def test_roundtrip_and_manifest(self):
        st, img, extra, z = self.make()
        man = self.p.pack(st, img, z, extra=[extra])
        self.assertEqual(set(man["members"]), {"image.qcow2", "state.json", "packages.tsv"})
        state = self.p.unpack(z, self.tmp / "out")
        self.assertEqual(state["linux_ids"], self.st["linux_ids"])
        self.assertEqual((self.tmp / "out/image.qcow2").read_bytes(), b"qcow-ish")

    def test_tampered_member_is_detected(self):
        st, img, extra, z = self.make()
        self.p.pack(st, img, z)
        t = self.tmp / "t.zip"
        with zipfile.ZipFile(z) as a, zipfile.ZipFile(t, "w", zipfile.ZIP_STORED) as b:
            for n in a.namelist():
                b.writestr(n, b"tampered" if n == "image.qcow2" else a.read(n))
        with self.assertRaises(self.p.PropError):
            self.p.unpack(t, self.tmp / "out2")

    def test_pack_refuses_a_state_not_derived_from_its_seed(self):
        st, img, _, z = self.make()
        s = json.loads(st.read_text())
        s["linux_ids"]["machine_id"] = "1" * 32
        st.write_text(json.dumps(s))
        with self.assertRaises(self.p.PropError):
            self.p.pack(st, img, z)

    def test_tampered_state_json_is_detected(self):
        st, img, extra, z = self.make()
        self.p.pack(st, img, z)
        t = self.tmp / "t2.zip"
        s2 = json.loads(st.read_text())
        s2["note"] = "edited"
        with zipfile.ZipFile(z) as a, zipfile.ZipFile(t, "w", zipfile.ZIP_STORED) as b:
            for n in a.namelist():
                b.writestr(n, json.dumps(s2) if n == "state.json" else a.read(n))
        with self.assertRaises(self.p.PropError):
            self.p.unpack(t, self.tmp / "out3")

    def test_zip_must_be_stored_and_linux_kind(self):
        z = self.tmp / "bad.zip"
        with zipfile.ZipFile(z, "w", zipfile.ZIP_DEFLATED) as b:
            for n in ("image.qcow2", "state.json", "manifest.json"):
                b.writestr(n, "{}")
        with self.assertRaises(self.p.PropError):
            self.p.unpack(z, self.tmp / "o")

    def test_ovmf_vars_embed_and_restore(self):
        (self.tmp / "firmware").mkdir()
        (self.tmp / "firmware/OVMF_VARS.fd").write_bytes(b"vars")
        self.p.capture_files(self.st, self.tmp)
        self.assertTrue(self.p.validate_state(self.st))
        out = self.tmp / "r"
        self.p.restore_files(self.st, out)
        self.assertEqual((out / "firmware/OVMF_VARS.fd").read_bytes(), b"vars")

    def test_convert_command_is_compressed_qcow2(self):
        c = self.p.qemu_img_convert_cmd("a", "b")
        self.assertIn("-c", c)
        self.assertEqual(c[-2:], ["a", "b"])


class HostGuardTests(Base):
    def test_memory_guard(self):
        mi = self.tmp / "meminfo"
        mi.write_text("MemTotal: 1 kB\nMemAvailable:    1500000 kB\n")
        with self.assertRaises(self.p.PropError) as e:
            self.p.guard_memory(2048, mi)
        self.assertIn("HOST_MEMORY_LOW", str(e.exception))
        mi.write_text("MemAvailable:    3000000 kB\n")
        self.assertGreaterEqual(self.p.guard_memory(2048, mi), 2048)

    def test_memory_guard_boundary(self):
        mi = self.tmp / "meminfo2"
        mi.write_text("MemAvailable:    %d kB\n" % (2047 * 1024))
        with self.assertRaises(self.p.PropError):
            self.p.guard_memory(2048, mi)
        mi.write_text("MemAvailable:    %d kB\n" % (2048 * 1024))
        self.assertEqual(self.p.guard_memory(2048, mi), 2048)

    def test_ssh_is_loopback_only_and_key_only(self):
        a = self.p.ssh_argv(self.st, "/k", "true")
        self.assertIn("BatchMode=yes", a)
        self.assertEqual(a[-2], "kfimage@127.0.0.1")
        s = self.p.scp_argv(self.st, "/k", "/a", "/b")
        self.assertIn("-P", s)
        self.assertNotIn("-p", s)


class AuthTests(Base):
    URL = "https://auth.invalid/authorize?client_id=c&state=S&code_challenge=C&redirect_uri=http%3A%2F%2Flocalhost%3A41234%2Fcallback"

    def test_redact_hides_codes_and_tokens_not_the_authorization_request(self):
        self.assertEqual(self.p.redact_url(self.URL).count("REDACTED"), 0)
        self.assertIn("code_challenge=C", self.p.redact_url(self.URL))
        r = self.p.redact_url("http://localhost:1/cb?code=SECRETCODE&state=s&access_token=T&user_code=ABCD-EFGH#id_token=J")
        for bad in ("SECRETCODE", "access_token=T", "id_token=J"):
            self.assertNotIn(bad, r)
        self.assertIn("user_code=ABCD-EFGH", r)

    def test_redirect_kinds(self):
        for host, kind in (("localhost", "localhost"), ("127.0.0.1", "127.0.0.1"), ("%5B%3A%3A1%5D", "::1")):
            u = "https://a/b?redirect_uri=http%%3A%%2F%%2F%s%%3A5555%%2Fcb" % host
            i = self.p.redirect_info(u)
            self.assertEqual((i["kind"], i["port"], i["path"]), (kind, 5555, "/cb"))

    def test_custom_scheme_and_missing_redirect(self):
        self.assertIsNone(self.p.redirect_info("https://a/b?x=1"))
        self.assertIsNone(self.p.redirect_info("https://a/b?redirect_uri=com.app%3A%2F%2Fcb")["kind"])
        self.assertIsNone(self.p.redirect_info("https://a/b?redirect_uri=https%3A%2F%2Fexample.com%2Fcb")["kind"])

    SS = ('LISTEN 0 5 127.0.0.1:41234 0.0.0.0:* users:(("python3",pid=77,fd=3))\n'
          'LISTEN 0 128 0.0.0.0:22 0.0.0.0:* users:(("sshd",pid=1,fd=3))\n'
          'LISTEN 0 5 [::1]:42000 [::]:* users:(("matlab",pid=88,fd=9))\n'
          'LISTEN 0 5 *:8080 *:*\n')

    def test_parse_ss(self):
        l = self.p.parse_ss(self.SS)
        self.assertEqual([(x["addr"], x["port"], x["pid"]) for x in l],
                         [("127.0.0.1", 41234, 77), ("0.0.0.0", 22, 1), ("::1", 42000, 88), ("0.0.0.0", 8080, None)])

    def test_pick_listener_filters(self):
        l = self.p.parse_ss(self.SS)
        self.assertEqual(self.p.pick_listener(l)["port"], 41234)
        base = [l[0]]
        self.assertEqual(self.p.pick_listener(l, baseline=base)["port"], 42000)
        self.assertEqual(self.p.pick_listener(l, tree={88})["port"], 42000)
        self.assertEqual(self.p.pick_listener(l, want_port=42000)["port"], 42000)
        self.assertIsNone(self.p.pick_listener(l, tree={999}))

    def test_process_tree(self):
        ps = self.p.parse_ps(" 1 0\n 10 1\n 11 10\n 12 11\n 99 1\n")
        self.assertEqual(self.p.descendants(10, ps), {10, 11, 12})

    def test_forward_plan_for_all_three_hosts(self):
        port = 41234
        for host, bind, target, lst in (("localhost", "", "127.0.0.1", {"addr": "127.0.0.1", "port": port}),
                                        ("127.0.0.1", "127.0.0.1:", "127.0.0.1", {"addr": "127.0.0.1", "port": port}),
                                        ("%5B%3A%3A1%5D", "[::1]:", "[::1]", {"addr": "::1", "port": port})):
            url = "https://a/b?redirect_uri=http%%3A%%2F%%2F%s%%3A%d%%2Fcb" % (host, port)
            plan = self.p.forward_plan(self.st, url, lst, "me@coord")
            self.assertEqual(plan["argv"][plan["argv"].index("-L") + 1], "%s%d:%s:%d" % (bind, port, target, port))
            self.assertIn("-J me@coord", plan["command"])
            self.assertIn("-p %d" % self.st["ids"]["net"]["ssh_port"], plan["command"])
            self.assertIn("kfimage@127.0.0.1", plan["command"])

    def test_forward_uses_the_detected_port_not_the_redirect_port(self):
        plan = self.p.forward_plan(self.st, self.URL, {"addr": "127.0.0.1", "port": 50000})
        self.assertIn("50000:127.0.0.1:50000", plan["command"])
        self.assertTrue(any("differs" in n for n in plan["notes"]))

    def test_no_port_no_command(self):
        plan = self.p.forward_plan(self.st, "https://a/b?x=1", None)
        self.assertIsNone(plan["command"])

    def test_pty_url_and_prompt_parsing(self):
        pty = "\x1b[1mOpen\x1b[0m https://auth.invalid/dev?user_code=AB-CD&state=s \r\nEnter the code: "
        self.assertEqual(self.p.urls_from_pty(pty), ["https://auth.invalid/dev?user_code=AB-CD&state=s"])
        self.assertEqual(self.p.prompt_of(pty), "Enter the code:")
        self.assertIsNone(self.p.prompt_of("done\n"))

    def test_masking(self):
        m = self.p.mask_tokens("Enter the code for a.b@uni.edu (abcdefghijklmnopqrstuvwxyz0123) 123456 at https://x.y/z?q=1:")
        for bad in ("a.b@uni.edu", "abcdefghijklmnopqrstuvwxyz0123", "123456", "q=1"):
            self.assertNotIn(bad, m)

    def test_paste_goes_over_stdin_never_argv_or_files(self):
        calls = []

        def runner(argv, **kw):
            calls.append((argv, kw))
            return cp(0, "pasted\n")

        before = {p for p in self.tmp.rglob("*")}
        self.p.auth_paste(self.st, "/k", "  THE-SECRET-CODE-123 \n", runner=runner)
        self.assertEqual(calls[0][1]["input"], "THE-SECRET-CODE-123")
        self.assertNotIn("THE-SECRET-CODE-123", " ".join(calls[0][0]))
        self.assertIn("pipe-pane -t $S;", calls[0][0][-1])      # logging is off while the code is typed
        self.assertEqual(before, {p for p in self.tmp.rglob("*")})

    def test_paste_refuses_empty_and_multiline(self):
        for bad in ("", "  ", "a\nb"):
            with self.assertRaises(self.p.PropError):
                self.p.auth_paste(self.st, "/k", bad, runner=lambda *a, **k: cp(0, "pasted"))

    def test_paste_script_turns_logging_off_before_the_code_and_back_on_after(self):
        seen = []
        self.p.auth_paste(self.st, "/k", "c0de", runner=lambda argv, **kw: (seen.append(argv[-1]), cp(0, "pasted"))[1])
        sc = seen[0]
        self.assertLess(sc.index("tmux pipe-pane -t $S;"), sc.index("load-buffer"))
        self.assertLess(sc.index("paste-buffer"), sc.index("pipe-pane -o"))

    def test_forward_target_follows_the_listener_family(self):
        url = "https://a/b?redirect_uri=http%3A%2F%2Flocalhost%3A4000%2Fcb"
        v6 = self.p.forward_plan(self.st, url, {"addr": "::1", "port": 4000})["argv"]
        v4 = self.p.forward_plan(self.st, url, {"addr": "127.0.0.1", "port": 4000})["argv"]
        self.assertEqual(v6[v6.index("-L") + 1], "4000:[::1]:4000")
        self.assertEqual(v4[v4.index("-L") + 1], "4000:127.0.0.1:4000")

    def test_forward_bind_follows_the_redirect_host(self):
        for host, want in (("127.0.0.1", "127.0.0.1:4000:127.0.0.1:4000"), ("localhost", "4000:127.0.0.1:4000")):
            url = "https://a/b?redirect_uri=http%%3A%%2F%%2F%s%%3A4000%%2Fcb" % host
            a = self.p.forward_plan(self.st, url, {"addr": "127.0.0.1", "port": 4000})["argv"]
            self.assertEqual(a[a.index("-L") + 1], want)

    def scripted(self, urls, pty, ss, ps, pane, baseline_ss=""):
        first = [True]

        def runner(argv, **kw):
            cmd = argv[-1]
            if "@@PTY@@" in cmd:
                cur = baseline_ss if first[0] else ss
                first[0] = False
                return cp(0, "%s@@PTY@@\n%s@@SS@@\n%s@@PS@@\n%s@@PANE@@\n%s" % (urls, pty, cur, ps, pane))
            return cp(0, "started\n")
        return runner

    def test_capture_ignores_a_listener_outside_the_command_tree(self):
        ss = 'LISTEN 0 5 127.0.0.1:41234 0.0.0.0:* users:(("other",pid=500,fd=3))\n'
        r = self.scripted("2026-11-16T20:00:00Z\t" + self.URL + "\n", "", ss, "1 0\n 70 1\n 500 1\n", "70\n")
        t = iter(range(0, 1000, 5))
        plan = self.p.auth_capture(self.st, "/k", "stub", runner=r, sleep=lambda s: None, clock=lambda: next(t), out=lambda *a: None, timeout_s=400, grace_s=20)
        self.assertEqual(plan["mode"], "paste")     # no listener of the command: falls back after the grace period

    def test_capture_ignores_a_preexisting_listener(self):
        ss = 'LISTEN 0 5 127.0.0.1:41234 0.0.0.0:* users:(("old",pid=77,fd=3))\n'
        r = self.scripted("2026-11-16T20:00:00Z\t" + self.URL + "\n", "", ss, "1 0\n 70 1\n 77 70\n", "70\n", baseline_ss=ss)
        t = iter(range(0, 1000, 5))
        plan = self.p.auth_capture(self.st, "/k", "stub", runner=r, sleep=lambda s: None, clock=lambda: next(t), out=lambda *a: None, timeout_s=400, grace_s=20)
        self.assertEqual(plan["mode"], "paste")

    def test_capture_output_never_contains_a_code_from_a_logged_callback(self):
        cb = "http://localhost:41234/callback?code=SUPERSECRETCODE&state=s"
        r = self.scripted("2026-11-16T20:00:00Z\t" + cb + "\n", "", "", "1 0\n", "70\n")
        lines, logs = [], []
        self.p.auth_capture(self.st, "/k", "stub", mode="paste", runner=r, sleep=lambda s: None, out=lines.append, owner_log=logs.append, timeout_s=50)
        self.assertNotIn("SUPERSECRETCODE", "\n".join(lines) + json.dumps(logs))

    def test_the_cli_paste_prompt_is_hidden(self):
        self.assertIn("getpass.getpass(", SOURCE.read_text())

    # ---- the shim and the stub app, locally (the guest run is auth-selftest)
    def shim_env(self):
        b = self.tmp / "bin"
        b.mkdir()
        shim = b / "xdg-open"
        shim.write_text(self.p.AUTH_SHIM)
        shim.chmod(0o755)
        log = self.tmp / "urls.log"
        env = dict(os.environ, PATH="%s:%s" % (b, os.environ["PATH"]), KF_AUTH_URL_LOG=str(log))
        return env, log

    def test_shim_opens_nothing_logs_0600_and_prints(self):
        env, log = self.shim_env()
        r = subprocess.run(["xdg-open", "https://a/b?x=1", "--flag"], env=env, capture_output=True, text=True)
        self.assertEqual(r.returncode, 0)
        self.assertIn("KF_AUTH_URL https://a/b?x=1", r.stderr)
        self.assertEqual(stat.S_IMODE(log.stat().st_mode), 0o600)
        self.assertEqual(len(self.p.parse_auth_log(log.read_text())), 1)

    def test_stub_loopback_end_to_end_with_the_shim(self):
        env, log = self.shim_env()
        out = self.tmp / "res.json"
        proc = subprocess.Popen([sys.executable, "-I", str(STUB), "--host", "localhost", "--timeout", "30", "--result", str(out)], env=env,
                                stdout=subprocess.PIPE, text=True)
        try:
            t0 = time.time()
            while not (log.exists() and log.read_text().strip()) and time.time() - t0 < 20:
                time.sleep(0.2)
            url = self.p.parse_auth_log(log.read_text())[-1]["url"]
            info = self.p.redirect_info(url)
            self.assertEqual(info["kind"], "localhost")
            urllib.request.urlopen("http://127.0.0.1:%d/callback?code=FAKE&state=stubstate123" % info["port"], timeout=10).read()
            proc.wait(timeout=20)
        finally:
            proc.kill()
        res = json.loads(out.read_text())
        self.assertEqual((res["callback"], res["state_ok"], res["code_len"]), (1, 1, 4))
        self.assertNotIn("FAKE", out.read_text())

    def test_stub_terminal_mode_reads_a_code_from_stdin(self):
        out = self.tmp / "res.json"
        r = subprocess.run([sys.executable, "-I", str(STUB), "--mode", "terminal", "--result", str(out)], input="TESTCODE-1\n", text=True, capture_output=True)
        self.assertEqual(r.returncode, 0)
        urls = self.p.urls_from_pty(r.stdout)
        self.assertEqual(len(urls), 1)
        self.assertEqual(self.p.prompt_of(r.stdout.split("\n", 2)[1] + "\n" + "Enter the authorization code: "), "Enter the authorization code:")
        self.assertEqual(json.loads(out.read_text())["code_len"], 10)
        self.assertNotIn("TESTCODE", out.read_text() + r.stdout)

    def test_auth_capture_loopback_with_a_scripted_guest(self):
        pty = "KF\n"
        snaps = iter([
            ("", "", "", "1 0\n", ""),                                                      # baseline
            ("", "", "", "1 0\n", ""),
            ("2026-11-16T20:00:00Z\t" + self.URL + "\n", "", "LISTEN 0 5 127.0.0.1:41234 0.0.0.0:* users:((\"x\",pid=77,fd=3))\n", "1 0\n 70 1\n 77 70\n", "70\n"),
            ("2026-11-16T20:00:00Z\t" + self.URL + "\n", "", "LISTEN 0 5 127.0.0.1:41234 0.0.0.0:* users:((\"x\",pid=77,fd=3))\n", "1 0\n 70 1\n 77 70\n", "70\n"),
        ])
        state = {"last": None}

        def runner(argv, **kw):
            cmd = argv[-1]
            if "@@PTY@@" in cmd:
                try:
                    state["last"] = next(snaps)
                except StopIteration:
                    pass
                a, pt, ss, ps, pane = state["last"]
                return cp(0, "%s@@PTY@@\n%s@@SS@@\n%s@@PS@@\n%s@@PANE@@\n%s" % (a, pt, ss, ps, pane))
            return cp(0, "started\n")

        lines, logs = [], []
        plan = self.p.auth_capture(self.st, "/k", "stub", runner=runner, sleep=lambda s: None, out=lines.append, owner_log=logs.append, timeout_s=50)
        text = "\n".join(lines)
        self.assertEqual(plan["mode"], "loopback")
        self.assertEqual(plan["port"], 41234)
        self.assertIn("-L 41234:127.0.0.1:41234", text)
        self.assertIn("AUTHORIZATION URL", text)
        self.assertEqual(logs[0]["redirect_kind"], "localhost")

    def test_auth_capture_paste_mode_never_prints_a_code_like_prompt_token(self):
        pty = "visit https://auth.invalid/d?user_code=AB-CD&state=s\nEnter code for a@b.edu: "
        def runner(argv, **kw):
            cmd = argv[-1]
            if "@@PTY@@" in cmd:
                return cp(0, "@@PTY@@\n%s@@SS@@\n@@PS@@\n1 0\n@@PANE@@\n70\n" % pty)
            return cp(0, "started\n")
        lines, logs = [], []
        plan = self.p.auth_capture(self.st, "/k", "stub", mode="paste", runner=runner, sleep=lambda s: None, out=lines.append, owner_log=logs.append, timeout_s=50)
        text = "\n".join(lines) + json.dumps(logs)
        self.assertEqual(plan["mode"], "paste")
        self.assertNotIn("a@b.edu", text)
        self.assertIn("user_code=AB-CD", text)

    def test_auth_capture_times_out_with_a_named_error(self):
        t = iter(range(0, 1000, 20))
        def runner(argv, **kw):
            return cp(0, "@@PTY@@\n@@SS@@\n@@PS@@\n@@PANE@@\n")
        with self.assertRaises(self.p.PropError) as e:
            self.p.auth_capture(self.st, "/k", None, runner=runner, sleep=lambda s: None, clock=lambda: next(t), out=lambda *a: None, timeout_s=100)
        self.assertIn("AUTH_TIMEOUT", str(e.exception))


class NovncPinTests(Base):
    def test_requirements_are_hash_pinned(self):
        r = self.p.novnc_requirements()
        self.assertRegex(r, r"^websockify==\d+\.\d+\.\d+ \\\n    --hash=sha256:[0-9a-f]{64}\n$")
        self.assertRegex(self.p.NOVNC["novnc_sha256"], r"^[0-9a-f]{64}$")
        self.assertTrue(self.p.NOVNC["novnc_url"].startswith("https://"))

    def test_novnc_binds_loopback_only(self):
        src = SOURCE.read_text()
        self.assertIn('"127.0.0.1:%d" % a.port', src)
        self.assertIn("--require-hashes", src)


class SealTests(Base):
    def test_seal_deletes_the_auth_files_and_checks(self):
        src = SOURCE.read_text()
        self.assertIn("rm -f /var/tmp/kf-auth-*", src)
        self.assertIn("AUTH_FILES_LEFT", src)


if __name__ == "__main__":
    unittest.main()
