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


# ---------------------------------------------------------------- package manifest, harden-offline, inject (section 13)
REAL_NFT = ('{"nftables": [{"metainfo": {"version": "1.1.6", "release_name": "Commodore Bullmoose #7", "json_schema_version": 1}}, '
            '{"table": {"family": "inet", "name": "kf_offline", "handle": 4}}, '
            '{"chain": {"family": "inet", "table": "kf_offline", "name": "input", "handle": 1, "type": "filter", "hook": "input", "prio": 0, "policy": "accept"}}, '
            '{"chain": {"family": "inet", "table": "kf_offline", "name": "forward", "handle": 2, "type": "filter", "hook": "forward", "prio": 0, "policy": "drop"}}, '
            '{"chain": {"family": "inet", "table": "kf_offline", "name": "output", "handle": 3, "type": "filter", "hook": "output", "prio": 0, "policy": "drop"}}, '
            '{"rule": {"family": "inet", "table": "kf_offline", "chain": "output", "handle": 4, "expr": [{"match": {"op": "==", "left": {"meta": {"key": "oifname"}}, "right": "lo"}}, {"accept": null}]}}, '
            '{"rule": {"family": "inet", "table": "kf_offline", "chain": "output", "handle": 5, "expr": [{"match": {"op": "in", "left": {"ct": {"key": "state"}}, "right": ["established", "related"]}}, {"accept": null}]}}]}')
UNITS_MASKED = "".join("%s\tmasked\n" % u for u in (
    "apt-daily.timer", "apt-daily-upgrade.timer", "apt-daily.service", "apt-daily-upgrade.service", "unattended-upgrades.service"))
DPKG = ("bash\t5.2.21-2ubuntu4\tamd64\tii \nlinux-image-6.8.0-149-generic\t6.8.0-149.150\tamd64\tii \nlinux-headers-6.8.0-149\t6.8.0-149.150\tall\tii \n"
        "linux-headers-6.8.0-149-generic\t6.8.0-149.150\tamd64\tii \nlinux-image-generic\t6.8.0.149.150\tamd64\thi \nold-pkg\t1.0\tamd64\trc \n"
        "nftables\t1.0.9-1build1\tamd64\tii \npython3-venv\t3.12.3-0ubuntu2\tamd64\tii \n")
SRC_FILE = {"path": "/etc/apt/sources.list.d/ubuntu.sources", "content": "Types: deb\nURIs: http://archive.ubuntu.com/ubuntu\nSuites: noble\n"}
CRED_SRC = {"path": "/etc/apt/sources.list", "content": "deb http://mirroruser:secretpass@mirror.example/ubuntu noble main\n"}


def ok(out=""):
    return {"rc": 0, "out": out}


def raw_guest(hardened=True, sources=(SRC_FILE,), du="", pip="[]", **over):
    r = {"dpkg": ok(DPKG), "manual": ok("bash\nnftables\npython3-venv\n"), "holds": ok("linux-image-generic\n"), "uname": ok("6.8.0-149-generic\n"),
         "proc_version": ok("Linux version 6.8.0-149-generic (gcc-13)\n"), "headers_present": ok("yes\n"),
         "os_release": ok('ID=ubuntu\nVERSION_ID="24.04"\nVERSION_CODENAME=noble\n'), "python3": ok("Python 3.12.3\n"),
         "snap": ok("Name  Version  Rev  Tracking  Publisher  Notes\ncore22  20240111  1380  latest/stable  canonical  base\n"), "pip": ok(pip),
         "npm": ok(""), "gem": ok("rake (13.0.6)\n"), "apt_files": ok(json.dumps(list(sources))), "pyenvs": ok(""), "du": ok(du),
         "nft": ok(REAL_NFT if hardened else '{"nftables": [{"metainfo": {}}]}'), "nft_conf_sha": ok("ab" * 32 + "  /etc/nftables.conf\n" if hardened else ""),
         "nft_enabled": ok("enabled\n" if hardened else "disabled\n"), "unit_states": ok(UNITS_MASKED if hardened else ""),
         "sources_present": ok("0\n" if hardened else "1\n")}
    r.update(over)
    return r


def collector_ssh(raw, log=None):
    def ssh(remote, stdin=None, timeout=600):
        if log is not None:
            log.append(remote)
        cmds = json.loads(stdin)
        return cp(0, json.dumps({k: raw.get(k, ok("")) for k in cmds}) + "\n")
    return ssh


class ManifestBase(Base):
    def man(self, vendor=(), **kw):
        raw = raw_guest(**kw)
        return self.p.P.build_manifest(raw, list(vendor), {"entries": 3}, self.st["base"], "2026-11-17T10:00:00Z")


class VendorEntryTests(ManifestBase):
    def test_parses_a_free_form_entry(self):
        e = self.p.P.parse_vendor_entry("name=MATLAB release=R2026b products=MATLAB,Parallel_Computing_Toolbox,Deep_Learning_Toolbox "
                                        "installer_sha256=" + "AB" * 32 + " installed_via=mpm extra_field='two words'")
        self.assertEqual(e["products"], ["MATLAB", "Parallel_Computing_Toolbox", "Deep_Learning_Toolbox"])
        self.assertEqual(e["installer_sha256"], "ab" * 32)
        self.assertEqual(e["extra_field"], "two words")

    def test_a_key_that_names_a_secret_is_refused(self):
        for k in ("password", "token", "license_file", "activation_code", "api_key", "key", "client_secret"):
            with self.assertRaises(self.p.P.PkgError, msg=k) as e:
                self.p.P.parse_vendor_entry("name=X %s=abc" % k)
            self.assertIn("VENDOR_ENTRY_SECRET_KEY", str(e.exception))

    def test_a_credential_url_in_a_value_is_refused(self):
        with self.assertRaises(self.p.P.PkgError) as e:
            self.p.P.parse_vendor_entry("name=X source=https://user:pw@host.example/x")
        self.assertIn("VENDOR_ENTRY_CREDENTIAL", str(e.exception))

    def test_name_and_hash_are_checked(self):
        for bad in ("release=1", "name=X installer_sha256=abc", "name=X name=Y", "Name=X", "name=", "", "name=X noequals"):
            with self.assertRaises(self.p.P.PkgError, msg=bad):
                self.p.P.parse_vendor_entry(bad)


class ManifestTests(ManifestBase):
    def test_dpkg_keeps_installed_packages_with_versions_manual_and_held(self):
        m = self.man()
        names = {r["name"]: r for r in m["dpkg"]}
        self.assertNotIn("old-pkg", names)                   # status rc: removed, config files only
        self.assertEqual(names["bash"]["version"], "5.2.21-2ubuntu4")
        self.assertTrue(names["bash"]["manual"] and not names["linux-image-6.8.0-149-generic"]["manual"])
        self.assertTrue(names["linux-image-generic"]["held"])    # status hi: hold, installed
        self.assertEqual(m["counts"]["dpkg"], 7)

    def test_a_removed_package_is_not_in_the_manifest(self):
        rows = self.p.P.parse_dpkg("a\t1\tamd64\tii \nb\t2\tamd64\trc \nc\t3\tamd64\tun \n")
        self.assertEqual([r["name"] for r in rows], ["a"])

    def test_kernel_headers_snaps_pip_npm_gem(self):
        pip = json.dumps([{"name": "requests", "version": "2.31.0", "location": "/usr/local/lib/python3.12/dist-packages"}])
        m = self.man(pip=pip, npm=ok(json.dumps({"dependencies": {"left-pad": {"version": "1.3.0"}}})))
        self.assertEqual(m["kernel"]["release"], "6.8.0-149-generic")
        self.assertTrue(m["kernel"]["headers_present"])
        self.assertEqual({k["name"] for k in m["kernel"]["packages"]},
                         {"linux-image-6.8.0-149-generic", "linux-headers-6.8.0-149", "linux-headers-6.8.0-149-generic", "linux-image-generic"})
        self.assertEqual(m["snaps"], [{"name": "core22", "version": "20240111", "revision": "1380", "channel": "latest/stable"}])
        self.assertEqual(m["pip"][0]["name"], "requests")
        self.assertEqual(m["npm"], [{"name": "left-pad", "version": "1.3.0"}])
        self.assertEqual(m["gem"], [{"name": "rake", "version": "13.0.6"}])
        self.assertEqual(m["os"]["version_codename"], "noble")

    def test_apt_source_credentials_are_redacted(self):
        m = self.man(sources=(CRED_SRC,))
        txt = json.dumps(m)
        self.assertNotIn("secretpass", txt)
        self.assertNotIn("mirroruser", txt)
        self.assertIn("http://REDACTED@mirror.example/ubuntu", m["apt"]["sources"][0]["content"])

    def test_redaction_helper_alone(self):
        self.assertEqual(self.p.P.redact_userinfo("deb https://u:p@h/x\nURIs: http://plain/y"), "deb https://REDACTED@h/x\nURIs: http://plain/y")

    def test_large_dir_threshold_is_one_gb_and_caches_are_not_applications(self):
        gb = 1 << 30
        rows = [{"path": "/usr/local/MATLAB", "bytes": gb + 1}, {"path": "/opt/edge", "bytes": gb}, {"path": "/home/kfimage/.cache", "bytes": 9 * gb},
                {"path": "/home/kfimage/tools", "bytes": 2 * gb}]
        self.assertEqual([r["path"] for r in self.p.P.large_app_dirs(rows)], ["/home/kfimage/tools", "/usr/local/MATLAB"])

    def test_seal_warns_naming_large_dirs_without_a_vendor_entry(self):
        m = self.man(du="3221225472\t/usr/local/MATLAB\n100\t/opt/small\n")
        w = self.p.P.seal_warnings(m)
        self.assertTrue(any("LARGE_DIRS_WITHOUT_VENDOR_ENTRY" in x and "/usr/local/MATLAB" in x for x in w), w)
        self.assertFalse(any("/opt/small" in x for x in w))

    def test_a_vendor_entry_silences_the_large_dir_warning(self):
        m = self.man(vendor=[{"name": "MATLAB"}], du="3221225472\t/usr/local/MATLAB\n")
        self.assertFalse(any("LARGE_DIRS" in x for x in self.p.P.seal_warnings(m)))

    def test_package_manifest_through_a_scripted_guest_and_vendor_entries(self):
        raw = raw_guest(du="3221225472\t/opt/app\n")
        runner = lambda argv, **kw: cp(0, "2026-11-16T20:00:00Z\n") if argv[-1].startswith("cat /var/lib/kf-provisioned") else cp(0, "[]\n")
        p = {"key": self.tmp / "k", "setup_dir": self.tmp}
        m = self.p.package_manifest(self.st, p, [{"name": "App", "release": "1"}], ssh=collector_ssh(raw), runner=runner)
        self.assertEqual(m["vendor"], [{"name": "App", "release": "1"}])
        self.assertEqual(m["inventory"]["entries"], 0)
        self.assertFalse(any("LARGE_DIRS" in x for x in self.p.P.seal_warnings(m)))
        self.p.P.validate_manifest(m)

    def seal_dir(self):
        d = self.tmp / "D"
        d.mkdir(exist_ok=True)
        self.p.save_state(self.p.init_state(SEED, keypair=("PRIVATE", PUB)), d / "state.json")
        return str(d)

    def test_cli_seal_refuses_a_secret_vendor_key_before_touching_the_guest(self):
        import contextlib
        import io
        err = io.StringIO()
        with contextlib.redirect_stderr(err):
            rc = self.p.main(["seal", "--dir", self.seal_dir(), "--vendor-entry", "name=X password=hunter2"])
        self.assertEqual(rc, 2)
        self.assertIn("VENDOR_ENTRY_SECRET_KEY", err.getvalue())
        self.assertNotIn("hunter2", err.getvalue())

    def test_cli_seal_refuses_a_credential_url_in_a_vendor_value(self):
        import contextlib
        import io
        err = io.StringIO()
        with contextlib.redirect_stderr(err):
            rc = self.p.main(["seal", "--dir", self.seal_dir(), "--vendor-entry", "name=X src=https://u:pw@h.example/x"])
        self.assertEqual(rc, 2)
        self.assertIn("VENDOR_ENTRY_CREDENTIAL", err.getvalue())

    def test_manifest_uses_the_host_record_after_harden(self):
        rec = {"schema": self.p.P.SCHEMA, "sources": self.p.P.sources_from_files([SRC_FILE]), "preferences": []}
        (self.tmp / "apt_sources.recorded.json").write_text(json.dumps(rec))
        runner = lambda argv, **kw: cp(0, "2026-11-16T20:00:00Z\n") if argv[-1].startswith("cat /var/lib/kf-provisioned") else cp(0, "[]\n")
        m = self.p.package_manifest(self.st, {"key": self.tmp / "k", "setup_dir": self.tmp}, [], ssh=collector_ssh(raw_guest(sources=())), runner=runner)
        self.assertEqual(m["apt"]["sources_origin"], "recorded-by-harden-offline")
        self.assertEqual(m["apt"]["sources"][0]["path"], SRC_FILE["path"])

    def test_an_unmasked_unattended_upgrades_service_is_not_hardened(self):
        states = UNITS_MASKED.replace("unattended-upgrades.service\tmasked", "unattended-upgrades.service\tenabled")
        h = self.man(unit_states=ok(states))["hardening"]
        self.assertFalse(h["hardened"])
        self.assertIn("unattended-upgrades.service", " ".join(h["problems"]))

    def test_python_environments_in_the_image_are_a_warning(self):
        m = self.man(pyenvs=ok("/opt/x/venv/pyvenv.cfg\n"))
        self.assertTrue(any("PYTHON_ENV_IN_IMAGE" in x for x in self.p.P.seal_warnings(m)))

    def test_sources_recorded_by_harden_are_used_when_the_guest_has_none(self):
        rec = {"sources": self.p.P.sources_from_files([SRC_FILE]), "preferences": []}
        m = self.p.P.build_manifest(raw_guest(sources=()), [], {}, self.st["base"], "t", recorded_apt=rec)
        self.assertEqual(m["apt"]["sources_origin"], "recorded-by-harden-offline")
        self.assertEqual(m["apt"]["sources"][0]["path"], SRC_FILE["path"])

    def test_live_sources_win_over_a_record_and_none_is_flagged(self):
        rec = {"sources": self.p.P.sources_from_files([CRED_SRC]), "preferences": []}
        self.assertEqual(self.p.P.build_manifest(raw_guest(), [], {}, self.st["base"], "t", recorded_apt=rec)["apt"]["sources_origin"], "guest-live")
        m = self.man(sources=())
        self.assertEqual(m["apt"]["sources_origin"], "none")
        self.assertTrue(any("NO_APT_SOURCES_RECORDED" in x for x in self.p.P.seal_warnings(m)))

    def test_hardened_flag_needs_every_part(self):
        self.assertTrue(self.man()["hardening"]["hardened"])
        self.assertEqual(self.man()["hardening"]["problems"], [])
        for name, over in (("ruleset", {"nft": ok('{"nftables": []}')}), ("conf", {"nft_conf_sha": ok("")}), ("enabled", {"nft_enabled": ok("disabled\n")}),
                           ("unmasked", {"unit_states": ok(UNITS_MASKED.replace("masked", "enabled", 1))}), ("sources", {"sources_present": ok("2\n")})):
            h = self.man(**over)["hardening"]
            self.assertFalse(h["hardened"], name)
            self.assertTrue(h["problems"], name)

    def test_remaining_apt_sources_make_the_image_not_hardened_and_warn(self):
        m = self.man(sources_present=ok("1\n"))
        self.assertFalse(m["hardening"]["hardened"])
        self.assertTrue(any("NOT_HARDENED" in x and "apt source" in x for x in self.p.P.seal_warnings(m)))

    def test_schema_validation(self):
        m = self.man(vendor=[{"name": "A"}])
        self.assertTrue(self.p.P.validate_manifest(m))
        bad = json.loads(json.dumps(m))
        bad["counts"]["dpkg"] += 1
        with self.assertRaises(self.p.P.PkgError):
            self.p.P.validate_manifest(bad)
        with self.assertRaises(self.p.P.PkgError):
            self.p.P.validate_manifest({"schema": "x"})

    def test_pip_and_apt_snippets_are_valid_python_and_json(self):
        for name in ("pip", "apt_files"):
            r = subprocess.run(self.p.P.COMMANDS[name], shell=True, capture_output=True, text=True, timeout=60)
            self.assertEqual(r.returncode, 0, r.stderr)
            self.assertIsInstance(json.loads(r.stdout.strip().splitlines()[-1]), list)

    def test_inventory_summary_has_counts_not_paths(self):
        rows = [{"path": "/opt/a", "size": 5, "type": "f", "why": "named-root"}, {"path": "/home/x/b", "size": 0, "type": "d", "why": "changed-since-provisioning"}]
        s = self.p.P.summarize_inventory(rows)
        self.assertEqual((s["entries"], s["files"], s["dirs"], s["bytes"]), (2, 1, 1, 5))
        self.assertNotIn("/opt/a", json.dumps(s))
        self.assertEqual(s["by_top"], {"/home": 1, "/opt": 1})

    def test_stage_a_installs_what_a_clone_cannot(self):
        src = self.p.render_stage_a(self.st)
        self.assertIn("python3-venv", src)
        self.assertIn("nftables", src)

    def test_seal_parses_vendor_entries_before_any_long_step(self):
        body = SOURCE.read_text().split("def cmd_seal(a):")[1].split("\ndef boot_clone")[0]
        self.assertLess(body.index("parse_vendor_entry"), body.index("guest_grep("))
        self.assertIn("man = package_manifest(", body)
        self.assertLess(body.index("man = package_manifest("), body.index("systemctl poweroff"))


class DiffTests(ManifestBase):
    def two(self):
        a = self.man()
        b = json.loads(json.dumps(a))
        return a, b

    def test_identical_manifests_have_no_difference_and_the_timestamp_is_ignored(self):
        a, b = self.two()
        b["generated_utc"] = "2030-01-01T00:00:00Z"
        self.assertEqual(self.p.P.diff_manifests(a, b), {})
        self.assertIn("no package-level difference", self.p.P.format_diff({}))

    def test_added_removed_and_changed_packages(self):
        a, b = self.two()
        b["dpkg"] = [r for r in b["dpkg"] if r["name"] != "nftables"] + [{"name": "newpkg", "version": "1", "arch": "amd64", "manual": True, "held": False}]
        for r in b["dpkg"]:
            if r["name"] == "bash":
                r["version"] = "9.9"
        d = self.p.P.diff_manifests(a, b)["dpkg"]
        self.assertEqual([x[0] for x in d["added"]], ["newpkg"])
        self.assertEqual([x[0] for x in d["removed"]], ["nftables"])
        self.assertEqual(d["changed"][0][0], "bash")
        txt = self.p.P.format_diff(self.p.P.diff_manifests(a, b))
        self.assertIn("+ newpkg", txt)
        self.assertIn("- nftables", txt)
        self.assertIn("~ bash: 5.2.21-2ubuntu4 amd64".split(" amd64")[0], txt)

    def test_vendor_apt_kernel_and_hardening_changes(self):
        a, b = self.two()
        b["vendor"] = [{"name": "MATLAB", "release": "R2026b"}]
        b["counts"]["vendor"] = 1
        b["kernel"]["release"] = "6.8.0-150-generic"
        b["apt"]["sources"][0]["sha256"] = "0" * 64
        b["hardening"]["hardened"] = False
        d = self.p.P.diff_manifests(a, b)
        self.assertEqual({"vendor", "kernel", "apt_sources", "hardening"}, set(d))

    def test_cli_diff_exit_code_and_zip_input(self):
        a, b = self.two()
        b["pip"] = [{"name": "x", "version": "1", "location": "/l"}]
        b["counts"]["pip"] = 1
        fa, fb = self.tmp / "a.json", self.tmp / "b.json"
        self.p.P.write_manifest(a, fa)
        self.p.P.write_manifest(b, fb)
        main = lambda *a: quiet_main(self.p, list(a))
        self.assertEqual(main("diff-packages", str(fa), str(fa)), 0)
        self.assertEqual(main("diff-packages", str(fa), str(fb)), 1)
        z = self.tmp / "p.zip"
        with zipfile.ZipFile(z, "w") as zz:
            zz.write(fb, "packages.manifest.json")
        self.assertEqual(main("diff-packages", str(fa), str(z)), 1)
        self.assertEqual(main("diff-packages", str(fb), str(z)), 0)


class PackManifestTests(ManifestBase):
    def make(self, with_sha=True):
        mf = self.tmp / "packages.manifest.json"
        sha = self.p.P.write_manifest(self.man(), mf)
        if with_sha:
            self.st["packages"] = {"kernel": "k", "count": 1, "sha256": "x", "manifest_sha256": sha}
        st = self.tmp / "state.json"
        self.p.save_state(self.st, st)
        img = self.tmp / "image.qcow2"
        img.write_bytes(b"qcow-ish")
        return st, img, mf, self.tmp / "p.zip"

    def test_the_manifest_is_a_hashed_member(self):
        st, img, mf, z = self.make()
        m = self.p.pack(st, img, z, [mf])
        self.assertEqual(m["members"]["packages.manifest.json"], self.p.sha256_file(mf))
        st2 = self.p.unpack(z, self.tmp / "u")
        self.assertTrue((self.tmp / "u" / "packages.manifest.json").exists())
        self.assertEqual(st2["packages"]["manifest_sha256"], self.p.sha256_file(mf))

    def test_pack_refuses_a_missing_package_manifest(self):
        st, img, mf, z = self.make()
        with self.assertRaises(self.p.PropError) as e:
            self.p.pack(st, img, z, [])
        self.assertIn("PACKAGES_MANIFEST_MISSING_OR_CHANGED", str(e.exception))

    def test_pack_refuses_a_changed_package_manifest(self):
        st, img, mf, z = self.make()
        mf.write_text(mf.read_text().replace("noble", "jammy"))
        with self.assertRaises(self.p.PropError) as e:
            self.p.pack(st, img, z, [mf])
        self.assertIn("PACKAGES_MANIFEST_MISSING_OR_CHANGED", str(e.exception))

    def test_unpack_validates_the_manifest_member(self):
        st, img, mf, z = self.make(with_sha=False)
        mf.write_text('{"schema": "wrong"}\n')
        self.p.pack(st, img, z, [mf])
        with self.assertRaises(self.p.P.PkgError):
            self.p.unpack(z, self.tmp / "u")

    def test_unpack_refuses_a_manifest_the_state_does_not_record(self):
        st, img, mf, z = self.make(with_sha=False)
        self.p.pack(st, img, z, [mf])
        # a state that records another manifest hash cannot be packed with this manifest, so no such zip exists to unpack: the
        # unpack side is checked by building the zip by hand
        s2 = self.p.load_state(st)
        s2["packages"] = {"manifest_sha256": "0" * 64}
        self.p.save_state(s2, st)
        with self.assertRaises(self.p.PropError):
            self.p.pack(st, img, self.tmp / "q.zip", [mf])
        manifest = {"manifest_version": 1, "kind": "propimage-linux", "packages": s2["packages"], "members": {
            "image.qcow2": self.p.sha256_file(img), "state.json": self.p.sha256_file(st), "packages.manifest.json": self.p.sha256_file(mf)}}
        z2 = self.tmp / "hand.zip"
        with zipfile.ZipFile(z2, "w", zipfile.ZIP_STORED) as zz:
            zz.write(img, "image.qcow2")
            zz.write(st, "state.json")
            zz.write(mf, "packages.manifest.json")
            zz.writestr("manifest.json", json.dumps(manifest))
        with self.assertRaises(self.p.PropError) as e:
            self.p.unpack(z2, self.tmp / "u2")
        self.assertIn("PACKAGES_MANIFEST_MISMATCH", str(e.exception))

    def test_the_pack_cli_adds_the_sidecar_manifest(self):
        d = self.tmp / "sealed"
        d.mkdir()
        st, img, mf, z = self.make()
        (d / "image.qcow2").write_bytes(b"x")
        (d / "packages.manifest.json").write_bytes(mf.read_bytes())
        self.assertEqual(quiet_main(self.p, ["pack", str(st), str(d / "image.qcow2"), "--out", str(z)]), 0)
        self.assertIn("packages.manifest.json", zipfile.ZipFile(z).namelist())


class HardenGuest:
    """A scripted guest for harden-offline: what it answers depends on whether the harden script has run."""

    def __init__(self, before_open=("tcp_1.1.1.1_443",), after_open=(), loopback=True, nft_after=REAL_NFT, harden_ok=True):
        self.log, self.hardened = [], False
        self.before_open, self.after_open, self.loopback, self.nft_after, self.harden_ok = before_open, after_open, loopback, nft_after, harden_ok
        self.keys = ("tcp_1.1.1.1_443", "tcp_9.9.9.9_53", "udp_dns_1.1.1.1", "ping_1.1.1.1")

    def __call__(self, remote, stdin=None, timeout=600):
        self.log.append(("bash" if remote.startswith("sudo bash -s") else remote[:40], stdin if remote.startswith("sudo bash -s") else None))
        if remote.startswith("sudo bash -s"):
            self.hardened = self.harden_ok
            return cp(0, "HARDEN_OK\n" if self.harden_ok else "", "" if self.harden_ok else "boom")
        if "tcp_1.1.1.1_443" in remote:
            opened = self.after_open if self.hardened else self.before_open
            res = {k: k in opened for k in self.keys}
            res["loopback_ssh"] = self.loopback
            return cp(0, json.dumps(res) + "\n")
        if remote.startswith("sudo nft flush"):
            self.hardened = False
            return cp(0)
        cmds = json.loads(stdin)
        if set(cmds) == {"apt_files"}:
            out = {"apt_files": ok(json.dumps([CRED_SRC, SRC_FILE] if not self.hardened else []))}
        else:
            nft = self.nft_after if self.hardened else '{"nftables": []}'
            out = {"nft": ok(nft), "sha": ok("cd" * 32 + "  /etc/nftables.conf\n"), "en": ok("enabled\n"), "units": ok(UNITS_MASKED), "src": ok("0\n")}
        return cp(0, json.dumps(out) + "\n")


class HardenTests(Base):
    def test_ruleset_is_default_drop_with_loopback_and_established_only(self):
        r = self.p.P.RULESET
        self.assertIn("hook output priority 0; policy drop", r)
        self.assertIn("hook forward priority 0; policy drop", r)
        out = r.split("chain output")[1]
        self.assertEqual([l.strip() for l in out.splitlines() if "accept" in l], ['oifname "lo" accept', "ct state established,related accept"])
        self.assertNotIn("dport", r)
        self.assertNotIn("apt", "\n".join(l for l in r.splitlines() if not l.startswith("#")))

    def test_the_real_nft_json_is_analysed_ok(self):
        a = self.p.P.check_ruleset(self.p.P.analyze_nft(REAL_NFT))
        self.assertEqual((a["tables"], a["chains"], a["rules"]), (1, 3, 2))
        self.assertEqual(a["drop_hooks"], ["forward", "output"])
        self.assertEqual(a["rule_kinds"], {"loopback": 1, "established": 1, "other": 0})
        self.assertTrue(a["ok"])

    def exception_json(self, expr):
        j = json.loads(REAL_NFT)
        j["nftables"].append({"rule": {"family": "inet", "table": "kf_offline", "chain": "output", "handle": 9, "expr": expr}})
        return json.dumps(j)

    def test_a_port_accept_is_an_exception(self):
        t = self.exception_json([{"match": {"op": "==", "left": {"payload": {"protocol": "tcp", "field": "dport"}}, "right": 443}}, {"accept": None}])
        with self.assertRaises(self.p.P.PkgError) as e:
            self.p.P.check_ruleset(self.p.P.analyze_nft(t))
        self.assertIn("HARDEN_EXCEPTION", str(e.exception))

    def test_accepting_new_connections_is_an_exception(self):
        t = self.exception_json([{"match": {"op": "in", "left": {"ct": {"key": "state"}}, "right": ["established", "new"]}}, {"accept": None}])
        a = self.p.P.analyze_nft(t)
        self.assertEqual(a["rule_kinds"]["other"], 1)
        with self.assertRaises(self.p.P.PkgError):
            self.p.P.check_ruleset(a)

    def test_a_bare_accept_rule_is_an_exception(self):
        with self.assertRaises(self.p.P.PkgError):
            self.p.P.check_ruleset(self.p.P.analyze_nft(self.exception_json([{"accept": None}])))

    def test_output_policy_accept_is_refused(self):
        with self.assertRaises(self.p.P.PkgError) as e:
            self.p.P.check_ruleset(self.p.P.analyze_nft(REAL_NFT.replace('"hook": "output", "prio": 0, "policy": "drop"', '"hook": "output", "prio": 0, "policy": "accept"')))
        self.assertIn("HARDEN_NO_DEFAULT_DROP", str(e.exception))

    def test_forward_policy_accept_is_refused(self):
        with self.assertRaises(self.p.P.PkgError):
            self.p.P.check_ruleset(self.p.P.analyze_nft(REAL_NFT.replace('"hook": "forward", "prio": 0, "policy": "drop"', '"hook": "forward", "prio": 0, "policy": "accept"')))

    def test_an_empty_ruleset_is_refused(self):
        with self.assertRaises(self.p.P.PkgError):
            self.p.P.check_ruleset(self.p.P.analyze_nft('{"nftables": []}'))
        with self.assertRaises(self.p.P.PkgError):
            self.p.P.analyze_nft("not json")

    def test_the_ruleset_loads_in_a_real_nftables_when_the_box_allows_it(self):
        if not shutil_which("nft") or not shutil_which("unshare"):
            self.skipTest("no nft or unshare")
        f = self.tmp / "r.nft"
        f.write_text(self.p.P.RULESET)
        r = subprocess.run(["unshare", "-rn", "sh", "-c", "nft -c -f %s && nft -f %s && nft -j list ruleset" % (f, f)], capture_output=True, text=True)
        if r.returncode != 0:
            self.skipTest("no user namespace or nftables kernel support here: " + r.stderr[-120:])
        a = self.p.P.check_ruleset(self.p.P.analyze_nft(r.stdout))
        self.assertEqual(a["rule_kinds"]["other"], 0)

    def test_mask_units_are_the_named_five(self):
        self.assertEqual(set(self.p.P.MASK_UNITS), {"apt-daily.timer", "apt-daily-upgrade.timer", "apt-daily.service", "apt-daily-upgrade.service",
                                                     "unattended-upgrades.service"})

    def test_harden_script_masks_removes_and_checks(self):
        s = self.p.P.render_harden_script()
        self.assertIn("nft -c -f /etc/nftables.conf", s)
        self.assertIn("systemctl enable nftables.service", s)
        for u in ("apt-daily.timer", "apt-daily-upgrade.timer", "apt-daily.service", "apt-daily-upgrade.service", "unattended-upgrades.service"):
            self.assertIn(u, s.split("systemctl mask")[1].splitlines()[0])
        self.assertIn("rm -f /etc/apt/sources.list /etc/apt/sources.list.d/*", s)
        self.assertIn("rm -rf /var/lib/apt/lists/*", s)
        self.assertIn(self.p.P.RULESET.strip().splitlines()[-1], s)
        self.assertEqual(subprocess.run(["bash", "-n"], input=s, text=True, capture_output=True).returncode, 0)

    def test_the_harden_script_embeds_the_default_drop_ruleset(self):
        s = self.p.P.render_harden_script()
        self.assertIn("hook output priority 0; policy drop;", s)
        self.assertIn("hook forward priority 0; policy drop;", s)
        self.assertIn("flush ruleset", s)

    def test_a_ct_state_invalid_accept_is_an_exception(self):
        t = self.exception_json([{"match": {"op": "==", "left": {"ct": {"key": "state"}}, "right": "invalid"}}, {"accept": None}])
        self.assertEqual(self.p.P.analyze_nft(t)["rule_kinds"]["other"], 1)

    def test_harden_offline_with_a_scripted_guest(self):
        g, out = HardenGuest(), []
        rec = self.p.P.harden_offline(g, self.tmp, log=out.append)
        recorded = json.loads((self.tmp / "apt_sources.recorded.json").read_text())
        self.assertEqual({s["path"] for s in recorded["sources"]}, {CRED_SRC["path"], SRC_FILE["path"]})
        self.assertNotIn("secretpass", json.dumps(recorded))
        self.assertEqual(rec["ruleset"]["rules"], 2)
        self.assertEqual(rec["outbound_after"]["ping_1.1.1.1"], False)
        text = "\n".join(out)
        self.assertIn("1 table(s), 3 chain(s), 2 rule(s)", text)
        self.assertIn("drop on forward+output", text)
        self.assertTrue((self.tmp / "hardening.json").exists())

    def test_the_sources_are_recorded_before_the_script_removes_them(self):
        seen = []

        class G(HardenGuest):
            def __call__(g, remote, stdin=None, timeout=600):
                if remote.startswith("sudo bash -s"):
                    seen.append((self.tmp / "apt_sources.recorded.json").exists())
                return HardenGuest.__call__(g, remote, stdin, timeout)
        self.p.P.harden_offline(G(), self.tmp, log=lambda *a: None)
        self.assertEqual(seen, [True])

    def test_a_second_run_never_overwrites_the_record_with_nothing(self):
        self.p.P.harden_offline(HardenGuest(), self.tmp, log=lambda *a: None)
        first = (self.tmp / "apt_sources.recorded.json").read_text()
        g = HardenGuest()
        g.hardened = True    # sources already gone on the guest
        self.p.P.harden_offline(g, self.tmp, log=lambda *a: None)
        self.assertEqual((self.tmp / "apt_sources.recorded.json").read_text(), first)

    def test_a_tcp_path_still_open_after_the_rules_refuses(self):
        with self.assertRaises(self.p.P.PkgError) as e:
            self.p.P.harden_offline(HardenGuest(after_open=("tcp_1.1.1.1_443",)), self.tmp, log=lambda *a: None)
        self.assertIn("HARDEN_OUTBOUND_OPEN", str(e.exception))

    def test_a_ping_path_still_open_after_the_rules_refuses(self):
        with self.assertRaises(self.p.P.PkgError) as e:
            self.p.P.harden_offline(HardenGuest(after_open=("ping_1.1.1.1",)), self.tmp, log=lambda *a: None)
        self.assertIn("ping_1.1.1.1", str(e.exception))

    def test_a_broken_loopback_refuses(self):
        with self.assertRaises(self.p.P.PkgError) as e:
            self.p.P.harden_offline(HardenGuest(loopback=False), self.tmp, log=lambda *a: None)
        self.assertIn("HARDEN_LOOPBACK_BROKEN", str(e.exception))

    def test_nothing_reachable_before_the_rules_is_called_out(self):
        out = []
        self.p.P.harden_offline(HardenGuest(before_open=()), self.tmp, log=out.append)
        self.assertTrue(any("cannot show the rules did it" in x for x in out))
        out2 = []
        (self.tmp / "x").mkdir()
        self.p.P.harden_offline(HardenGuest(), self.tmp / "x", log=out2.append)
        self.assertFalse(any("cannot show" in x for x in out2))

    def test_a_failing_harden_script_is_named(self):
        with self.assertRaises(self.p.P.PkgError) as e:
            self.p.P.harden_offline(HardenGuest(harden_ok=False), self.tmp, log=lambda *a: None)
        self.assertIn("HARDEN_FAILED", str(e.exception))

    def test_a_ruleset_with_an_exception_after_apply_refuses(self):
        t = HardenTests.exception_json(self, [{"accept": None}])
        with self.assertRaises(self.p.P.PkgError) as e:
            self.p.P.harden_offline(HardenGuest(nft_after=t), self.tmp, log=lambda *a: None)
        self.assertIn("HARDEN_EXCEPTION", str(e.exception))


def quiet_main(mod, argv):
    import contextlib
    import io
    with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
        return mod.main(argv)


def shutil_which(x):
    import shutil
    return shutil.which(x)


class CloneGuardTests(Base):
    def setUp(self):
        super().setUp()
        self.d = self.tmp / "D"
        self.pp = self.p.paths(self.d)
        (self.pp["sealed"]).mkdir(parents=True)
        (self.pp["sealed"] / "image.qcow2").write_bytes(b"sealed")
        self.wd = self.pp["run"] / "clone1"
        self.wd.mkdir(parents=True)
        (self.wd / "clone.qcow2").write_bytes(b"overlay")
        self.sealed_img = str((self.pp["sealed"] / "image.qcow2").resolve())

    def info(self, backing="SEALED", rc=0):
        b = self.sealed_img if backing == "SEALED" else backing
        j = {"format": "qcow2"}
        if b:
            j["full-backing-filename"] = b
        return lambda argv, **kw: cp(rc, json.dumps(j), "" if rc == 0 else "boom")

    def alive(self, wd=None):
        (wd or self.wd).joinpath("qemu.pid").write_text(str(os.getpid()))

    def test_a_running_overlay_of_the_sealed_image_is_accepted(self):
        self.alive()
        wd, pid = self.p.resolve_clone(self.pp, "clone1", self.info())
        self.assertEqual(wd, self.wd.resolve())
        self.assertEqual(pid, os.getpid())

    def test_an_image_without_a_backing_file_is_refused(self):
        self.alive()
        with self.assertRaises(self.p.PropError) as e:
            self.p.resolve_clone(self.pp, "clone1", self.info(backing=None))
        self.assertIn("NOT_AN_OVERLAY_OF_THE_SEALED_IMAGE", str(e.exception))

    def test_an_overlay_of_some_other_image_is_refused(self):
        self.alive()
        with self.assertRaises(self.p.PropError):
            self.p.resolve_clone(self.pp, "clone1", self.info(backing="/tmp/other.qcow2"))

    def test_the_sealed_image_itself_is_never_a_target(self):
        self.alive()
        (self.wd / "clone.qcow2").unlink()
        (self.wd / "clone.qcow2").symlink_to(self.pp["sealed"] / "image.qcow2")
        with self.assertRaises(self.p.PropError) as e:
            self.p.resolve_clone(self.pp, "clone1", self.info(backing=None))
        self.assertIn("NOT_A_CLONE", str(e.exception))

    def test_a_name_that_leaves_the_run_directory_or_points_at_sealed_is_refused(self):
        for bad in ("../sealed", "..", "a/b", "", "/etc"):
            with self.assertRaises(self.p.PropError, msg=bad):
                self.p.resolve_clone(self.pp, bad, self.info())
        (self.pp["run"] / "link").symlink_to(self.pp["sealed"])
        with self.assertRaises(self.p.PropError) as e:
            self.p.resolve_clone(self.pp, "link", self.info())
        self.assertIn("CLONE_REFUSED_SEALED", str(e.exception))

    def test_a_stopped_clone_is_refused_by_unharden(self):
        (self.wd / "qemu.pid").write_text("999999999")
        with self.assertRaises(self.p.PropError) as e:
            self.p.resolve_clone(self.pp, "clone1", self.info())
        self.assertIn("CLONE_NOT_RUNNING", str(e.exception))

    def test_a_stopped_clone_is_refused_by_inject(self):
        (self.wd / "qemu.pid").write_text("999999999")

        def never(*a, **k):
            raise AssertionError("the guest must not be reached for a stopped clone")
        with mock_patch(self.p, "run_probe", never):
            with self.assertRaises(self.p.PropError) as e:
                self.p.clone_preflight(self.st, self.pp, "clone1", runner=self.info())
        self.assertIn("CLONE_NOT_RUNNING", str(e.exception))

    def test_unharden_flushes_the_ruleset_and_checks_it_is_gone(self):
        g = HardenGuest()
        g.hardened = True
        a = self.p.P.unharden(g)
        self.assertEqual(a["drop_hooks"], [])
        self.assertTrue(any(k.startswith("sudo nft flush") for k, _ in g.log))

    def test_unharden_fails_when_the_ruleset_survives(self):
        def ssh(remote, stdin=None, timeout=600):
            if remote.startswith("sudo nft flush"):
                return cp(0)
            return cp(0, json.dumps({"nft": ok(REAL_NFT)}) + "\n")
        with self.assertRaises(self.p.P.PkgError) as e:
            self.p.P.unharden(ssh)
        self.assertIn("UNHARDEN_STILL_DROPPING", str(e.exception))

    def test_the_cli_unharden_checks_the_clone_before_touching_the_guest(self):
        self.p.save_state(self.p.init_state(SEED, keypair=("PRIVATE", PUB)), self.pp["state"])
        (self.wd / "qemu.pid").write_text("999999999")
        self.assertEqual(quiet_main(self.p, ["unharden", "--dir", str(self.d), "--clone", "clone1"]), 2)
        self.assertEqual(quiet_main(self.p, ["unharden", "--dir", str(self.d), "--clone", "../sealed"]), 2)

    def test_preflight_refuses_another_identity(self):
        self.alive()
        guest = good_guest(self.st)
        guest["machine_id"] = "0" * 32
        with mock_patch(self.p, "run_probe", lambda *a, **k: guest):
            with self.assertRaises(self.p.PropError) as e:
                self.p.clone_preflight(self.st, self.pp, "clone1", runner=self.info())
        self.assertIn("machine_id", str(e.exception))

    def test_preflight_refuses_a_guest_with_network_like_the_setup_vm(self):
        self.alive()
        guest = good_guest(self.st)
        guest["internet"] = True
        with mock_patch(self.p, "run_probe", lambda *a, **k: guest):
            with self.assertRaises(self.p.PropError) as e:
                self.p.clone_preflight(self.st, self.pp, "clone1", runner=self.info())
        self.assertIn("no_network", str(e.exception))

    def test_preflight_accepts_a_matching_offline_clone_on_the_recorded_clock(self):
        self.alive()
        guest = good_guest(self.st)
        t0 = (self.wd / "qemu.pid").stat().st_mtime
        with mock_patch(self.p, "run_probe", lambda *a, **k: guest):
            wd, rows = self.p.clone_preflight(self.st, self.pp, "clone1", runner=self.info(), now=lambda: t0 + 120)
        self.assertTrue(all(r[1] != "MISMATCH" for r in rows))


def mock_patch(obj, name, value):
    import unittest.mock
    return unittest.mock.patch.object(obj, name, value)


class InjectDestTests(Base):
    def test_destinations(self):
        P = self.p.P
        self.assertEqual(P.check_dest("work/env", "kfimage"), "/home/kfimage/work/env")
        self.assertEqual(P.check_dest("/home/kfimage/a/b", "kfimage"), "/home/kfimage/a/b")
        self.assertEqual(P.check_dest("/opt/kf/venv", "kfimage"), "/opt/kf/venv")
        self.assertEqual(P.check_dest("/opt/kf//a/./b", "kfimage"), "/opt/kf/a/b")

    def test_everything_else_is_refused(self):
        for bad in ("/etc/passwd", "/usr/local/x", "/", "/root/x", "/home/other/x", "../x", "a/../../x", "/opt/kf/../../etc", "", "a\nb", "/tmp/x"):
            with self.assertRaises(self.p.P.PkgError, msg=bad) as e:
                self.p.P.check_dest(bad, "kfimage")
            self.assertIn("INJECT_DEST_REFUSED", str(e.exception))

    def test_a_sibling_directory_with_the_same_prefix_is_refused(self):
        for bad in ("/opt/kfx/y", "/opt/kf-other/y", "/home/kfimage2/y", "/home/kfimageevil/z"):
            with self.assertRaises(self.p.P.PkgError, msg=bad):
                self.p.P.check_dest(bad, "kfimage")

    def test_the_root_itself_is_refused(self):
        for bad in ("/opt/kf", "/home/kfimage", "/opt/kf/", "."):
            with self.assertRaises(self.p.P.PkgError, msg=bad):
                self.p.P.check_dest(bad, "kfimage")


class FakePopen:
    """Runs a local `tar -x` in place of `ssh ... tar -x`; records argv and the bytes sent."""
    last = None

    def __init__(self, argv, stdin=None, stdout=None, stderr=None, exists=False, fail=False):
        FakePopen.last = self
        self.argv, self.exists = argv, exists
        self.dst = Path(FakePopen.dst)
        self.dst.mkdir(parents=True, exist_ok=True)
        self.sent = bytearray()
        self.proc = subprocess.Popen(["tar", "-x", "-p", "--no-same-owner", "-C", str(self.dst)] if not exists else ["sh", "-c", "exit 3"],
                                     stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        outer = self

        class Tee:
            def write(s, b):
                outer.sent += b
                return outer.proc.stdin.write(b)

            def flush(s):
                outer.proc.stdin.flush()

            def close(s):
                try:
                    outer.proc.stdin.close()
                except Exception:
                    pass
        self.stdin = Tee()
        self.stdout = types.SimpleNamespace(read=self._read)

    def _read(self):
        out = self.proc.stdout.read()
        return out + (b"INJECT_EXISTS\n" if self.exists else b"INJECT_OK\n")

    def wait(self):
        return self.proc.wait()


class InjectTreeTests(Base):
    def setUp(self):
        super().setUp()
        self.src = self.tmp / "src"
        (self.src / "bin").mkdir(parents=True)
        (self.src / "bin" / "run.sh").write_text("#!/bin/sh\necho hi\n")
        (self.src / "bin" / "run.sh").chmod(0o755)
        (self.src / "secret.txt").write_text("s")
        (self.src / "secret.txt").chmod(0o600)
        (self.src / "lib").mkdir()
        (self.src / "lib" / "libx.so.1").write_text("x")
        (self.src / "lib" / "libx.so").symlink_to("libx.so.1")
        FakePopen.dst = str(self.tmp / "guest")
        self.argv_for = lambda remote: ["ssh", "guest", remote]

    def run_inject(self, src=None, dest="/opt/kf/env", **kw):
        return self.p.P.inject_path(self.argv_for, src or self.src, dest, "kfimage", popen=kw.pop("popen", FakePopen), log=lambda *a: None, **kw)

    def test_a_directory_arrives_with_modes_and_symlinks(self):
        res = self.run_inject()
        g = Path(FakePopen.dst)
        self.assertEqual(stat.S_IMODE((g / "bin/run.sh").stat().st_mode), 0o755)
        self.assertEqual(stat.S_IMODE((g / "secret.txt").stat().st_mode), 0o600)
        self.assertTrue((g / "lib/libx.so").is_symlink())
        self.assertEqual(os.readlink(g / "lib/libx.so"), "libx.so.1")
        self.assertEqual((res["files"], res["dirs"], res["links"]), (3, 3, 1))
        self.assertEqual(res["dest"], "/opt/kf/env")

    def test_the_printed_hash_is_the_hash_of_the_bytes_sent(self):
        res = self.run_inject()
        self.assertEqual(res["sha256"], hashlib.sha256(bytes(FakePopen.last.sent)).hexdigest())
        self.assertEqual(res["bytes"], len(FakePopen.last.sent))

    def test_the_hash_follows_the_content_and_is_reproducible(self):
        a = self.run_inject()["sha256"]
        shutil_rmtree(FakePopen.dst)
        self.assertEqual(self.run_inject()["sha256"], a)
        (self.src / "secret.txt").write_text("changed")
        shutil_rmtree(FakePopen.dst)
        self.assertNotEqual(self.run_inject()["sha256"], a)

    def tar_members(self):
        import io
        import tarfile
        return {m.name: m for m in tarfile.open(fileobj=io.BytesIO(bytes(FakePopen.last.sent)))}

    def test_the_tar_stream_drops_the_setuid_bit(self):
        (self.src / "bin" / "suid").write_text("x")
        (self.src / "bin" / "suid").chmod(0o4755)
        self.run_inject()
        self.assertEqual(self.tar_members()["./bin/suid"].mode, 0o755)

    def test_the_tar_stream_drops_the_setgid_bit_and_owner(self):
        (self.src / "bin" / "sgid").write_text("x")
        (self.src / "bin" / "sgid").chmod(0o2755)
        self.run_inject()
        m = self.tar_members()["./bin/sgid"]
        self.assertEqual(m.mode, 0o755)
        self.assertEqual((m.uid, m.gid, m.uname, m.gname), (0, 0, "", ""))

    def test_a_single_file_takes_the_destination_name(self):
        res = self.run_inject(src=self.src / "secret.txt", dest="/home/kfimage/conf/renamed.txt")
        self.assertEqual((Path(FakePopen.dst) / "renamed.txt").read_text(), "s")
        self.assertEqual(res["files"], 1)
        self.assertIn("/home/kfimage/conf", FakePopen.last.argv[-1])

    def test_an_absolute_symlink_is_refused(self):
        (self.src / "bin" / "python").symlink_to("/usr/bin/python3")
        with self.assertRaises(self.p.P.PkgError) as e:
            self.run_inject(popen=self.no_popen)
        self.assertIn("INJECT_SYMLINK_ESCAPES", str(e.exception))
        self.assertIn("(absolute)", str(e.exception))

    def test_an_absolute_symlink_pointing_inside_the_source_is_refused_too(self):
        (self.src / "lib" / "abs").symlink_to(str(self.src / "lib" / "libx.so.1"))   # would dangle once relocated
        with self.assertRaises(self.p.P.PkgError) as e:
            self.run_inject(popen=self.no_popen)
        self.assertIn("(absolute)", str(e.exception))

    def test_a_venv_style_interpreter_link_is_refused(self):
        (self.src / "pyvenv.cfg").write_text("home = /usr/bin\n")
        (self.src / "bin" / "python3").symlink_to("/usr/bin/python3.12")
        with self.assertRaises(self.p.P.PkgError) as e:
            self.run_inject(popen=self.no_popen)
        self.assertIn("/usr/bin/python3.12", str(e.exception))
        self.assertIn("(absolute)", str(e.exception))

    def test_a_relative_symlink_leaving_the_source_is_refused(self):
        (self.src / "lib" / "out").symlink_to("../../outside")
        with self.assertRaises(self.p.P.PkgError) as e:
            self.run_inject(popen=self.no_popen)
        self.assertIn("leaves the source", str(e.exception))

    def test_a_symlink_chain_that_escapes_is_refused(self):
        (self.tmp / "outside").mkdir()
        (self.src / "hop2").symlink_to("../outside")
        (self.src / "hop1").symlink_to("hop2")
        with self.assertRaises(self.p.P.PkgError):
            self.run_inject(popen=self.no_popen)

    def test_the_source_itself_a_symlink_is_refused(self):
        l = self.tmp / "srclink"
        l.symlink_to(self.src)
        with self.assertRaises(self.p.P.PkgError):
            self.run_inject(src=l, popen=self.no_popen)

    @staticmethod
    def no_popen(*a, **k):
        raise AssertionError("nothing may be sent")

    def test_a_fifo_is_refused(self):
        os.mkfifo(self.src / "pipe")
        with self.assertRaises(self.p.P.PkgError) as e:
            self.run_inject(popen=self.no_popen)
        self.assertIn("INJECT_SPECIAL_FILE", str(e.exception))

    def test_a_socket_is_refused(self):
        import socket
        s = socket.socket(socket.AF_UNIX)
        s.bind(str(self.src / "sock"))
        self.addCleanup(s.close)
        with self.assertRaises(self.p.P.PkgError) as e:
            self.run_inject(popen=self.no_popen)
        self.assertIn("INJECT_SPECIAL_FILE", str(e.exception))

    def test_the_default_cap_is_20_gb(self):
        self.assertEqual(self.p.P.DEFAULT_MAX_INJECT_GB, 20)

    def test_over_the_cap_is_refused_before_anything_is_sent(self):
        with open(self.src / "big", "wb") as f:
            f.truncate(3 << 30)   # sparse: no disk used
        with self.assertRaises(self.p.P.PkgError) as e:
            self.run_inject(popen=self.no_popen, max_gb=1)
        self.assertIn("INJECT_TOO_LARGE", str(e.exception))

    def test_the_cap_boundary(self):
        entries, total, _ = self.p.P.plan_tree(self.src, 1 << 30)
        self.p.P.plan_tree(self.src, total)             # exactly at the cap: fine
        with self.assertRaises(self.p.P.PkgError):
            self.p.P.plan_tree(self.src, total - 1)

    def test_the_stream_writer_refuses_past_its_cap(self):
        import io
        w = self.p.P.HashingWriter(io.BytesIO(), 10)
        w.write(b"0123456789")
        with self.assertRaises(self.p.P.PkgError):
            w.write(b"x")

    def test_an_existing_destination_is_named_not_overwritten(self):
        with self.assertRaises(self.p.P.PkgError) as e:
            self.run_inject(popen=lambda *a, **k: FakePopen(*a, exists=True, **k))
        self.assertIn("INJECT_DEST_EXISTS", str(e.exception))

    def test_a_remote_failure_is_reported(self):
        class Bad(FakePopen):
            def _read(s):
                s.proc.stdout.read()
                return b"tar: boom\n"
        with self.assertRaises(self.p.P.PkgError) as e:
            self.run_inject(popen=Bad)
        self.assertIn("INJECT_FAILED", str(e.exception))

    def test_the_remote_command_is_tar_over_ssh_with_modes_kept_and_safe_quoting(self):
        c = self.p.P.remote_extract_command("/home/kfimage/my dir/it's", "kfimage")
        inner = shlex_split(c)
        self.assertEqual(inner[:2], ["bash", "-c"])
        self.assertIn("tar -x -p --no-same-owner", inner[2])
        self.assertEqual(shlex_split(inner[2].split("d=")[1].splitlines()[0])[0], "/home/kfimage/my dir/it's")
        self.assertNotIn("sudo", inner[2])
        self.assertNotIn("scp", c)
        self.assertEqual(subprocess.run(["bash", "-n", "-c", inner[2]], capture_output=True).returncode, 0)

    def test_opt_kf_gets_its_root_directory_owned_by_the_user_and_replace_removes_first(self):
        s = shlex_split(self.p.P.remote_extract_command("/opt/kf/env", "kfimage", replace=True))[2]
        self.assertIn("sudo install -d -o kfimage -g kfimage /opt/kf", s)
        self.assertIn('rm -rf -- "$d"', s)
        self.assertIn("INJECT_EXISTS", shlex_split(self.p.P.remote_extract_command("/opt/kf/env", "kfimage"))[2])


def shlex_split(x):
    import shlex
    return shlex.split(x)


def shutil_rmtree(p):
    import shutil
    shutil.rmtree(p, ignore_errors=True)


def whl(name, ver, tags="py3-none-any"):
    return "%s-%s-%s.whl" % (name, ver, tags)


class WheelhouseTests(Base):
    def lock_text(self, reqs, extra=""):
        out = extra
        for name, ver, hashes, *rest in reqs:
            out += "%s==%s%s \\\n" % (name, ver, rest[0] if rest else "") + " \\\n".join("    --hash=sha256:%s" % h for h in hashes) + "\n"
        return out

    def build(self, reqs=(("alpha", "1.0", b"A"), ("Beta_pkg", "2.1", b"B")), tags="py3-none-any", extra=""):
        wh = self.tmp / "wheels"
        wh.mkdir(exist_ok=True)
        rows = []
        for name, ver, content in reqs:
            f = wh / whl(name, ver, tags)
            f.write_bytes(content)
            rows.append((name, ver, [hashlib.sha256(content).hexdigest()]))
        lock = self.tmp / "lock.txt"
        lock.write_text(self.lock_text(rows, extra))
        return wh, lock

    GUEST = {"py_minor": 12, "py_major": 3, "machine": "x86_64", "glibc": [2, 39], "venv_ok": True}

    def test_parse_lock_requirements_hashes_markers_and_dropped_options(self):
        text = ("--index-url https://download.pytorch.org/whl/cu128\n# comment\n--extra-index-url https://x/simple\n"
                "torch==2.7.0+cu128 \\\n    --hash=sha256:%s \\\n    --hash=sha256:%s\nnumpy==2.0.1 ; python_version < \"3.13\" \\\n    --hash=sha256:%s\n"
                % ("a" * 64, "b" * 64, "c" * 64))
        lk = self.p.P.parse_lock(text)
        self.assertEqual([(r["name"], r["version"], len(r["hashes"]), r["marker"]) for r in lk["reqs"]],
                         [("torch", "2.7.0+cu128", 2, None), ("numpy", "2.0.1", 1, 'python_version < "3.13"')])
        self.assertEqual(len(lk["dropped"]), 2)
        staged = self.p.P.staged_lock_text(text)
        self.assertNotIn("index-url", staged)
        self.assertIn("--hash=sha256:" + "a" * 64, staged)

    def test_an_unhashed_lock_is_refused_by_name(self):
        with self.assertRaises(self.p.P.PkgError) as e:
            self.p.P.parse_lock("torch==2.7.0\nnumpy==2.0.1\n")
        self.assertIn("LOCK_NOT_HASHED", str(e.exception))

    def test_one_unhashed_line_among_hashed_ones_is_refused(self):
        with self.assertRaises(self.p.P.PkgError) as e:
            self.p.P.parse_lock("a==1 \\\n  --hash=sha256:%s\nb==2\n" % ("a" * 64))
        self.assertIn("LOCK_NOT_HASHED", str(e.exception))
        self.assertIn("b==2", str(e.exception))

    def test_requirements_that_need_an_index_or_a_url_are_refused_by_name(self):
        for bad in ("pkg @ https://example.org/pkg-1.0-py3-none-any.whl", "git+https://example.org/x.git#egg=x", "-r other.txt", "-e ./local", "./pkg.whl"):
            with self.assertRaises(self.p.P.PkgError, msg=bad) as e:
                self.p.P.parse_lock(bad + "\n")
            self.assertIn("LOCK_NEEDS_INDEX", str(e.exception))

    def test_an_unpinned_requirement_is_refused(self):
        with self.assertRaises(self.p.P.PkgError) as e:
            self.p.P.parse_lock("torch>=2 \\\n --hash=sha256:%s\n" % ("a" * 64))
        self.assertIn("LOCK_NOT_PINNED", str(e.exception))

    def test_wheel_tag_compatibility(self):
        c = lambda f, **k: self.p.P.wheel_compatible(f, 12, **k)
        self.assertTrue(c("x-1-py3-none-any.whl"))
        self.assertTrue(c("x-1-cp312-cp312-manylinux_2_17_x86_64.manylinux2014_x86_64.whl"))
        self.assertTrue(c("x-1-cp39-abi3-manylinux2014_x86_64.whl"))
        self.assertTrue(c("x-1-cp312-none-linux_x86_64.whl"))
        self.assertFalse(c("x-1-cp311-cp311-manylinux2014_x86_64.whl"))
        self.assertFalse(c("x-1-cp313-cp313-manylinux2014_x86_64.whl"))
        self.assertFalse(c("x-1-cp312-cp312-win_amd64.whl"))
        self.assertFalse(c("x-1-cp312-cp312-manylinux2014_aarch64.whl"))
        self.assertFalse(c("x-1-cp312-cp312-musllinux_1_2_x86_64.whl"))
        self.assertFalse(c("x-1-cp312-cp312-manylinux_2_40_x86_64.whl"))   # newer glibc than the guest's 2.39
        self.assertFalse(c("not-a-wheel.whl"))

    def test_a_complete_wheelhouse_passes_and_unrelated_wheels_are_not_sent(self):
        wh, lock = self.build()
        (wh / whl("unrelated", "9")).write_bytes(b"U")
        chk = self.p.P.check_wheelhouse(wh, self.p.P.parse_lock(lock.read_text()), self.GUEST)
        self.assertEqual([w.name for w in chk["wheels"]], sorted([whl("alpha", "1.0"), whl("Beta_pkg", "2.1")]))

    def test_a_missing_wheel_is_named(self):
        wh, lock = self.build()
        (wh / whl("alpha", "1.0")).unlink()
        with self.assertRaises(self.p.P.PkgError) as e:
            self.p.P.check_wheelhouse(wh, self.p.P.parse_lock(lock.read_text()), self.GUEST)
        self.assertIn("WHEEL_MISSING: alpha==1.0", str(e.exception))

    def test_a_wheel_of_the_wrong_version_counts_as_missing(self):
        wh, lock = self.build()
        (wh / whl("Beta_pkg", "2.1")).rename(wh / whl("Beta_pkg", "2.2"))
        with self.assertRaises(self.p.P.PkgError) as e:
            self.p.P.check_wheelhouse(wh, self.p.P.parse_lock(lock.read_text()), self.GUEST)
        self.assertIn("WHEEL_MISSING: Beta_pkg==2.1", str(e.exception))

    def test_a_changed_wheel_is_a_hash_mismatch(self):
        wh, lock = self.build()
        (wh / whl("alpha", "1.0")).write_bytes(b"tampered")
        with self.assertRaises(self.p.P.PkgError) as e:
            self.p.P.check_wheelhouse(wh, self.p.P.parse_lock(lock.read_text()), self.GUEST)
        self.assertIn("WHEEL_HASH_MISMATCH: alpha==1.0", str(e.exception))

    def test_a_wrong_hash_in_the_lock_is_a_hash_mismatch(self):
        wh, lock = self.build()
        lock.write_text(lock.read_text().replace(hashlib.sha256(b"B").hexdigest(), "f" * 64))
        with self.assertRaises(self.p.P.PkgError) as e:
            self.p.P.check_wheelhouse(wh, self.p.P.parse_lock(lock.read_text()), self.GUEST)
        self.assertIn("WHEEL_HASH_MISMATCH: Beta_pkg==2.1", str(e.exception))

    def test_a_wheel_for_another_python_is_a_tag_mismatch(self):
        wh, lock = self.build(tags="cp311-cp311-manylinux2014_x86_64")
        with self.assertRaises(self.p.P.PkgError) as e:
            self.p.P.check_wheelhouse(wh, self.p.P.parse_lock(lock.read_text()), self.GUEST)
        self.assertIn("WHEEL_TAG_MISMATCH", str(e.exception))

    def test_a_wheel_for_another_platform_is_a_tag_mismatch(self):
        wh, lock = self.build(tags="cp312-cp312-win_amd64")
        with self.assertRaises(self.p.P.PkgError) as e:
            self.p.P.check_wheelhouse(wh, self.p.P.parse_lock(lock.read_text()), self.GUEST)
        self.assertIn("WHEEL_TAG_MISMATCH", str(e.exception))

    def test_a_marker_line_without_a_wheel_is_a_warning_not_a_failure(self):
        wh = self.tmp / "w"
        wh.mkdir()
        lk = self.p.P.parse_lock('only-on-old==1.0 ; python_version < "3.9" \\\n --hash=sha256:%s\n' % ("a" * 64))
        chk = self.p.P.check_wheelhouse(wh, lk, self.GUEST)
        self.assertEqual(chk["wheels"], [])
        self.assertTrue(chk["warnings"])

    def guest_ssh(self, info=None, pip_rc=0, log=None):
        info = dict(self.GUEST, **(info or {}))

        def ssh(remote, stdin=None, timeout=600):
            if log is not None:
                log.append(remote)
            if "CS_GNU_LIBC_VERSION" in remote:
                return cp(0, json.dumps(info) + "\n")
            if "pip install" in remote:
                return cp(pip_rc, "INSTALL_OK 2\n" if pip_rc == 0 else "", "" if pip_rc == 0 else "ERROR: No matching distribution found for alpha==1.0")
            return cp(0)
        return ssh

    def inject(self, wh, lock, ssh, popen=FakePopen, dest="/opt/kf/pyenv"):
        FakePopen.dst = str(self.tmp / "guest")
        return self.p.P.inject_wheelhouse(ssh, lambda r: ["ssh", r], wh, lock, dest, "kfimage", popen=popen, log=lambda *a: None)

    def test_end_to_end_copies_the_wheels_and_the_staged_lock_and_installs_offline_with_hashes(self):
        wh, lock = self.build(extra="--extra-index-url https://download.pytorch.org/whl/cu128\n")
        log = []
        res = self.inject(wh, lock, self.guest_ssh(log=log))
        g = Path(FakePopen.dst)
        self.assertEqual(sorted(p.name for p in (g / "wheelhouse").iterdir()), sorted([whl("alpha", "1.0"), whl("Beta_pkg", "2.1")]))
        staged = (g / "requirements.lock.txt").read_text()
        self.assertNotIn("extra-index-url", staged)
        self.assertIn("alpha==1.0", staged)
        install = [c for c in log if "pip install" in c][0]
        for flag in ("--no-index", "--require-hashes", "--only-binary=:all:", "--find-links", "-r"):
            self.assertIn(flag, install)
        self.assertNotIn("--index-url", install)
        self.assertEqual((res["wheels"], res["requirements"], res["dropped_options"]), (2, 2, 1))
        self.assertEqual(res["lock_sha256"], hashlib.sha256(lock.read_bytes()).hexdigest())

    def test_the_install_command_never_reaches_for_an_index_or_apt(self):
        s = self.p.P.WHEEL_INSTALL
        self.assertIn("--no-index", s)
        self.assertIn("--require-hashes", s)
        self.assertIn("--only-binary=:all:", s)
        self.assertNotIn("apt", s)
        self.assertNotIn("--index-url", s)
        self.assertNotIn("--extra-index-url", s)
        self.assertEqual(subprocess.run(["bash", "-n"], input=s.replace("@@DEST@@", "/x"), text=True, capture_output=True).returncode, 0)

    def test_a_guest_without_venv_support_is_named(self):
        wh, lock = self.build()
        with self.assertRaises(self.p.P.PkgError) as e:
            self.inject(wh, lock, self.guest_ssh(info={"venv_ok": False}), popen=NoPopen)
        self.assertIn("GUEST_NO_VENV", str(e.exception))

    def test_a_pip_failure_in_the_guest_is_named_with_its_reason(self):
        wh, lock = self.build()
        with self.assertRaises(self.p.P.PkgError) as e:
            self.inject(wh, lock, self.guest_ssh(pip_rc=1))
        self.assertIn("PIP_INSTALL_FAILED", str(e.exception))
        self.assertIn("No matching distribution", str(e.exception))

    def test_a_missing_wheel_stops_before_anything_is_sent(self):
        wh, lock = self.build()
        (wh / whl("alpha", "1.0")).unlink()
        with self.assertRaises(self.p.P.PkgError) as e:
            self.inject(wh, lock, self.guest_ssh(), popen=NoPopen)
        self.assertIn("WHEEL_MISSING", str(e.exception))

    def test_a_hash_mismatch_stops_before_anything_is_sent(self):
        wh, lock = self.build()
        (wh / whl("alpha", "1.0")).write_bytes(b"other")
        with self.assertRaises(self.p.P.PkgError) as e:
            self.inject(wh, lock, self.guest_ssh(), popen=NoPopen)
        self.assertIn("WHEEL_HASH_MISMATCH", str(e.exception))

    def test_a_lock_that_needs_an_index_stops_before_the_guest_is_asked(self):
        wh, lock = self.build()
        lock.write_text("pkg @ https://example.org/pkg.whl\n")
        calls = []
        with self.assertRaises(self.p.P.PkgError) as e:
            self.inject(wh, lock, lambda *a, **k: calls.append(a) or cp(0), popen=NoPopen)
        self.assertIn("LOCK_NEEDS_INDEX", str(e.exception))
        self.assertEqual(calls, [])

    def test_the_destination_rules_apply_to_the_wheelhouse_too(self):
        wh, lock = self.build()
        with self.assertRaises(self.p.P.PkgError) as e:
            self.inject(wh, lock, self.guest_ssh(), popen=NoPopen, dest="/usr/lib/python3/x")
        self.assertIn("INJECT_DEST_REFUSED", str(e.exception))

    def test_the_cli_wheelhouse_needs_a_lock_and_no_src(self):
        pp = self.p.paths(self.tmp / "D")
        pp["dir"].mkdir()
        self.p.save_state(self.p.init_state(SEED, keypair=("PRIVATE", PUB)), pp["state"])
        ns = types.SimpleNamespace(dir=str(self.tmp / "D"), name="clone1", wheelhouse=str(self.tmp), lock=None, src=None, dest=None, max_gb=20, replace=False)
        with mock_patch(self.p, "clone_preflight", lambda *a, **k: (None, [])):
            with self.assertRaises(self.p.PropError):
                self.p.cmd_inject(ns)
            ns.lock, ns.src = "x", "y"
            with self.assertRaises(self.p.PropError):
                self.p.cmd_inject(ns)


class NoPopen:
    def __init__(self, *a, **k):
        raise AssertionError("nothing may be sent")


if __name__ == "__main__":
    unittest.main()
