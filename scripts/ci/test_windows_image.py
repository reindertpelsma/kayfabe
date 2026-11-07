#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""VM/GPU/network-free image tooling tests. All external executables are fixtures.

Falsifiers: no artifact from an unclean/busy/changed source, no secret in argv or
output, no transfer accepted on a hash mismatch, no default win_vm argv change.
PowerShell tests below check source contracts only; they do not execute Windows.
"""
import fcntl
import hashlib
import json
import os
import re
from pathlib import Path
import shutil
import shlex
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
WIN = ROOT / "scripts/bench/windows"
ARTIFACT = b"fixture flattened qcow2\n"


def inventory(session="fixture"):
    return dict(schema="kayfabe-windows-inventory/1", session_id=session,
                prepared=True, hibernation_disabled=True, fast_startup_disabled=True,
                time_service_disabled=True, bitlocker_decrypted=True,
                zero_fill={"method": "sdelete", "completed": True},
                windows={"build": "fixture"}, drivers=[], packages=[],
                uninstall_registry=[], app_roots=[], files=[], exclusions=[])


FAKE = r'''#!/usr/bin/env python3
import json, os, pathlib, sys
p = pathlib.Path
a = sys.argv[1:]
name = p(sys.argv[0]).name
with open(os.environ['CALLS'], 'a') as f:
    f.write(json.dumps([name, a]) + '\n')
vm = p(os.environ['WINVM_ROOT']) / os.environ['WINVM_NAME']
if name == 'win_vm.sh':
    if os.environ.get('FAIL_VERB') == a[0]: sys.exit(7)
    if a[0] == 'run':
        tag = a[a.index('--tag')+1]
        overlay = a[a.index('--overlay')+1]
        (vm/'disk'/f'{overlay}.qcow2').write_bytes(b'fixture disk')
        (vm/'identity.env').write_text('VM_UUID=00000000-0000-0000-0000-000000000000\n')
        (vm/'network.json').write_text(json.dumps({
            'restrict':os.environ.get('WINVM_NET_RESTRICT'), 'identity':os.environ.get('WINVM_VM_IDENTITY')}))
        (vm/'logs'/f'{tag}_b1_qmp_events.jsonl').write_text(json.dumps(
            {'event':'SHUTDOWN', 'data':{'guest':True, 'reason':'guest-shutdown'}})+'\n')
    if a[0] == 'ssh':
        if 'ReadToEnd' in a[1]:
            assert sys.stdin.read() == os.environ['EXPECTED_PASSWORD']
            assert "Set-LocalUser -Name 'kf'" in a[1]
        if 'IMAGE_ACCOUNT_READY' in a[1]: print('IMAGE_ACCOUNT_READY')
    if a[0] == 'scp-from':
        state = json.loads((vm/'image-session.json').read_text())
        data = json.loads(os.environ['INVENTORY'])
        data['session_id'] = state['tag']
        if os.environ.get('BAD_INVENTORY'): data['prepared'] = False
        if os.environ.get('STALE_SESSION'): data['session_id'] = 'old-session'
        if os.environ.get('WRONG_SCHEMA'): data['schema'] = 'unsupported'
        p(a[2]).write_text(json.dumps(data))
elif name == 'qemu-img':
    if os.environ.get('FAIL_QEMU') == a[0]: sys.exit(9)
    if a[0] == 'convert':
        p(a[-1]).write_bytes(b'fixture flattened qcow2\n' if '-c' in a else b'fixture uncompressed\n')
    if a[0] == 'info':
        if '--backing-chain' in a:
            chain = [{'filename':str(vm/'disk/image-build.qcow2'),'format':'qcow2'},
                     {'filename':str(vm/'disk/base.qcow2'),'format':'qcow2'}]
            if os.environ.get('BAD_CHAIN'): chain.append({'filename':'untracked-parent','format':'qcow2'})
            print(json.dumps(chain)); sys.exit(0)
        print(json.dumps({'format':'qcow2', 'virtual-size':1024,
                          **({'backing-filename':'oops'} if os.environ.get('BACKING') else {}),
                          **({'format-specific':{'data':{'data-file':'oops'}}} if os.environ.get('EXTERNAL_DATA') else {})}))
elif name == 'curl':
    config = sys.stdin.read()
    assert 'https://' in config
    assert '--retry' in a and '--retry-all-errors' in a
    if os.environ.get('FAIL_CURL'):
        print(config, file=sys.stderr); sys.exit(22)
    if '--upload-file' not in a:
        p(a[a.index('--output')+1]).write_bytes(
            b'wrong' if os.environ.get('BAD_DOWNLOAD') else b'fixture flattened qcow2\n')
    print(os.environ.get('HTTP_STATUS', '200'))
'''


class Images(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="y1-image-test-"))
        self.addCleanup(shutil.rmtree, self.tmp)
        self.win = self.tmp / "repo/scripts/bench/windows"
        self.win.mkdir(parents=True)
        for name in ("image_session.sh", "image_package.sh", "image_tools.py",
                     "image_prepare_guest.ps1"):
            shutil.copy(WIN / name, self.win / name)
        shutil.copy(WIN / "win_vm.sh", self.win / "media-pins.txt")
        self.bin = self.tmp / "bin"
        self.bin.mkdir()
        for name in ("qemu-img", "curl"):
            (self.bin / name).write_text(FAKE)
            (self.bin / name).chmod(0o700)
        (self.win / "win_vm.sh").write_text(FAKE)
        (self.win / "win_vm.sh").chmod(0o700)
        # Pins must come from the checked-in launcher, never a supplied manifest.
        self.pins = (self.win / "media-pins.txt").read_text()
        self.vm = self.tmp / "vms/build"
        for name in ("disk", "run", "logs"):
            (self.vm / name).mkdir(parents=True)
        (self.vm / "installed").touch()
        (self.vm / "disk/base.qcow2").write_bytes(b"fixture sealed base")
        self.secret = self.tmp / "password.env"
        self.password = "fixture-only-$(`no-execution`) \" password"
        self.secret.write_text("WIN_IMAGE_PASSWORD=" + self.password + "\n")
        self.secret.chmod(0o600)
        self.url = self.tmp / "signed.url"
        self.url.write_text("https://example.invalid/object?signature=fixture-only\n")
        self.url.chmod(0o600)
        self.calls_file = self.tmp / "calls.jsonl"
        self.env = dict(os.environ, PATH=f"{self.bin}:{os.environ['PATH']}",
                        WINVM_ROOT=str(self.tmp / "vms"), WINVM_NAME="build",
                        CALLS=str(self.calls_file), EXPECTED_PASSWORD=self.password,
                        INVENTORY=json.dumps(inventory()))
        for key in ("WINVM_DRY_RUN", "WINVM_VM_IDENTITY"):
            self.env.pop(key, None)
        # A tiny local repo gives the implementation a real revision and tracked pins.
        for args in (["init", "-q"], ["add", "."],
                     ["-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                      "commit", "-qm", "fixture"]):
            subprocess.run(["git", *args], cwd=self.win.parents[2], check=True,
                           stdout=subprocess.DEVNULL)
        self.out = self.tmp / "package"

    def run_tool(self, tool, *args, **extra):
        if tool == "session" and args[0] in ("start", "install") and "--wait-seconds" not in args:
            args = (*args, "--wait-seconds", "1")
        return subprocess.run(["bash", str(self.win / f"image_{tool}.sh"), *map(str, args)],
                              env=dict(self.env, **extra), text=True,
                              capture_output=True, timeout=15)

    def calls(self, name=None):
        rows = [json.loads(x) for x in self.calls_file.read_text().splitlines()] if self.calls_file.exists() else []
        return [a for n, a in rows if name is None or n == name]

    def start(self):
        r = self.run_tool("session", "start", "--env-file", self.secret,
                          "--ssh-host", "builder.example.invalid", "--host-ssh-port", "2200")
        self.assertEqual(r.returncode, 0, r.stderr)
        return r

    def ready(self):
        self.start()
        r = self.run_tool("session", "stop")
        self.assertEqual(r.returncode, 0, r.stderr)

    def package(self, *args, **extra):
        # Restore the real pinned source at the production path only during packaging.
        fake = self.win / "win_vm.sh"
        old = fake.read_text()
        fake.write_text(self.pins)
        try:
            return self.run_tool("package", "create", "--out", self.out, *args, **extra)
        finally:
            fake.write_text(old)

    def built(self):
        self.ready()
        r = self.package()
        self.assertEqual(r.returncode, 0, r.stderr)
        return json.loads((self.out / "manifest.json").read_text())

    def test_start_reuses_run_overlay_and_fixed_identity(self):
        self.start()
        run = next(x for x in self.calls("win_vm.sh") if x[0] == "run")
        self.assertIn("--detach", run)
        self.assertEqual(run[run.index("--overlay")+1], "image-build")
        self.assertNotIn("--no-kf3", run)
        self.assertEqual(json.loads((self.vm / "network.json").read_text()), {"restrict":"on", "identity":"1"})
        self.assertIn("--unix-target", self.start_output_after_status())

    def start_output_after_status(self):
        r = self.run_tool("session", "status")
        self.assertEqual(r.returncode, 0, r.stderr)
        return r.stdout

    def test_connection_commands_are_loopback_and_exact(self):
        r = self.start()
        self.assertIn("ssh -N -p 2200 -L 127.0.0.1:6080:127.0.0.1:6080", r.stdout)
        self.assertIn("websockify --web /usr/share/novnc --unix-target", r.stdout)
        self.assertIn(str(self.vm / "run/vnc.sock"), r.stdout)
        self.assertIn("127.0.0.1:6080", r.stdout)

    def test_session_network_is_restricted_by_default(self):
        self.start()
        self.assertEqual(json.loads((self.vm / "network.json").read_text()), {"restrict":"on", "identity":"1"})

    def test_online_session_requires_explicit_opt_in(self):
        r = self.run_tool("session", "start", "--env-file", self.secret,
                          "--ssh-host", "host", "--allow-internet")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(json.loads((self.vm / "network.json").read_text())["restrict"], "off")

    def test_install_forwards_online_opt_in(self):
        r = self.run_tool("session", "install", "--env-file", self.secret,
                          "--ssh-host", "host", "--allow-internet")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(json.loads((self.vm / "network.json").read_text())["restrict"], "off")

    def test_password_literal_only_on_stdin(self):
        r = self.start()
        self.assertNotIn(self.password, r.stdout + r.stderr + self.calls_file.read_text())
        account = [x for x in self.calls("win_vm.sh") if x[0] == "ssh" and "ReadToEnd" in x[1]]
        self.assertEqual(len(account), 1)
        self.assertIn("Set-LocalUser", account[0][1])

    def test_world_readable_secret_refused_before_boot(self):
        self.secret.chmod(0o644)
        r = self.run_tool("session", "start", "--env-file", self.secret, "--ssh-host", "host")
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(self.calls())

    def test_secret_inside_repo_refused_before_install(self):
        secret = self.win / "private.env"
        secret.write_text(self.secret.read_text())
        secret.chmod(0o600)
        r = self.run_tool("session", "install", "--env-file", secret, "--ssh-host", "host")
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(self.calls())

    def test_env_is_data_never_sourced(self):
        self.secret.write_text("WIN_IMAGE_PASSWORD=fixture\nexport ATTACK=1\n")
        r = self.run_tool("session", "start", "--env-file", self.secret, "--ssh-host", "host")
        self.assertNotEqual(r.returncode, 0)
        self.assertNotIn("ATTACK=1", r.stdout + r.stderr)
        self.assertFalse(self.calls())

    def test_duplicate_password_line_is_rejected(self):
        self.secret.write_text("WIN_IMAGE_PASSWORD=first\nWIN_IMAGE_PASSWORD=second\n")
        r = self.run_tool("session", "start", "--env-file", self.secret, "--ssh-host", "host")
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(self.calls())

    def test_group_readable_password_refused(self):
        self.secret.chmod(0o640)
        r = self.run_tool("session", "install", "--env-file", self.secret, "--ssh-host", "host")
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(self.calls())

    def test_password_in_repo_refused_on_start(self):
        secret = self.win.parents[2] / "password.env"
        shutil.copyfile(self.secret, secret)
        secret.chmod(0o600)
        r = self.run_tool("session", "start", "--env-file", secret, "--ssh-host", "host")
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(self.calls())

    def test_install_uses_install_then_build(self):
        r = self.run_tool("session", "install", "--env-file", self.secret, "--ssh-host", "host")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(self.calls("win_vm.sh")[0], ["install"])
        self.assertTrue(any(x[0] == "run" for x in self.calls("win_vm.sh")))

    def test_no_password_logging_when_guest_fails(self):
        r = self.run_tool("session", "start", "--env-file", self.secret,
                          "--ssh-host", "host", "--wait-seconds", "1", FAIL_VERB="ssh")
        self.assertNotEqual(r.returncode, 0)
        self.assertNotIn(self.password, r.stdout + r.stderr)
        self.assertFalse((self.vm / "image-clean.json").exists())

    def test_second_start_invalidates_no_running_session(self):
        self.start()
        (self.vm / "disk/image-build.qcow2").unlink()
        r = self.run_tool("session", "start", "--env-file", self.secret, "--ssh-host", "host")
        self.assertNotEqual(r.returncode, 0)
        self.assertEqual(len([x for x in self.calls("win_vm.sh") if x[0] == "run"]), 1)

    def test_active_session_cannot_be_reinstalled(self):
        self.start()
        # Remove only the fixture disk so this tests the independent state guard.
        (self.vm / "disk/image-build.qcow2").unlink()
        r = self.run_tool("session", "install", "--env-file", self.secret, "--ssh-host", "host")
        self.assertNotEqual(r.returncode, 0)
        self.assertNotIn(["install"], self.calls("win_vm.sh"))

    def test_existing_overlay_refused_on_start(self):
        (self.vm / "disk/image-build.qcow2").write_bytes(b"valuable old image")
        r = self.run_tool("session", "start", "--env-file", self.secret, "--ssh-host", "host")
        self.assertNotEqual(r.returncode, 0)
        self.assertEqual((self.vm / "disk/image-build.qcow2").read_bytes(), b"valuable old image")

    def test_existing_overlay_refused_on_install(self):
        (self.vm / "disk/image-build.qcow2").write_bytes(b"valuable old image")
        r = self.run_tool("session", "install", "--env-file", self.secret, "--ssh-host", "host")
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(self.calls())

    def test_busy_vm_refused_before_start(self):
        with (self.vm / "run/vm.lock").open("w") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            r = self.run_tool("session", "start", "--env-file", self.secret, "--ssh-host", "host")
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(self.calls())

    def test_stop_prepares_pulls_cleans_then_stops(self):
        self.ready()
        calls = self.calls("win_vm.sh")
        prepare = next(i for i, x in enumerate(calls) if x[0] == "ssh" and "-File" in x[1])
        pull = next(i for i, x in enumerate(calls) if x[0] == "scp-from")
        clean = next(i for i, x in enumerate(calls) if x[0] == "ssh" and "Remove-Item" in x[1])
        stop = next(i for i, x in enumerate(calls) if x[0] == "stop")
        self.assertLess(prepare, pull)
        self.assertLess(pull, clean)
        self.assertLess(clean, stop)
        self.assertTrue((self.vm / "image-clean.json").exists())

    def test_failed_inventory_never_stops_or_seals(self):
        self.start()
        r = self.run_tool("session", "stop", BAD_INVENTORY="1")
        self.assertNotEqual(r.returncode, 0)
        self.assertNotIn(["stop"], self.calls("win_vm.sh"))
        self.assertFalse((self.vm / "image-clean.json").exists())

    def test_guest_prepare_failure_leaves_no_receipt(self):
        self.start()
        r = self.run_tool("session", "stop", FAIL_VERB="scp-to")
        self.assertNotEqual(r.returncode, 0)
        self.assertNotIn(["stop"], self.calls("win_vm.sh"))
        self.assertFalse((self.vm / "image-clean.json").exists())

    def test_incomplete_zero_fill_never_stops(self):
        self.start()
        data = inventory()
        data["zero_fill"]["completed"] = False
        r = self.run_tool("session", "stop", INVENTORY=json.dumps(data))
        self.assertNotEqual(r.returncode, 0)
        self.assertNotIn(["stop"], self.calls("win_vm.sh"))

    def test_hibernation_enabled_never_stops(self):
        self.start()
        data = inventory()
        data["hibernation_disabled"] = False
        r = self.run_tool("session", "stop", INVENTORY=json.dumps(data))
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse((self.vm / "image-clean.json").exists())

    def test_zero_completion_marker_required(self):
        self.start()
        data = inventory()
        del data["zero_fill"]["completed"]
        r = self.run_tool("session", "stop", INVENTORY=json.dumps(data))
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse((self.vm / "image-clean.json").exists())

    def test_inventory_for_old_session_refused(self):
        self.start()
        r = self.run_tool("session", "stop", STALE_SESSION="1")
        self.assertNotEqual(r.returncode, 0)
        self.assertNotIn(["stop"], self.calls("win_vm.sh"))

    def test_unsupported_inventory_schema_refused(self):
        self.start()
        r = self.run_tool("session", "stop", WRONG_SCHEMA="1")
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse((self.vm / "image-clean.json").exists())

    def test_qmp_quit_not_a_clean_shutdown(self):
        self.start()
        event = next((self.vm / "logs").glob("*_qmp_events.jsonl"))
        event.write_text(json.dumps({"event":"SHUTDOWN", "data":{"reason":"host-qmp-quit", "guest":False}}))
        r = self.run_tool("session", "stop")
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse((self.vm / "image-clean.json").exists())

    def test_guest_panic_then_shutdown_not_clean(self):
        self.start()
        event = next((self.vm / "logs").glob("*_qmp_events.jsonl"))
        event.write_text(json.dumps({"event":"GUEST_PANICKED"}) + "\n" + event.read_text())
        r = self.run_tool("session", "stop")
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse((self.vm / "image-clean.json").exists())

    def test_missing_guest_origin_refused(self):
        self.start()
        event = next((self.vm / "logs").glob("*_qmp_events.jsonl"))
        event.write_text(json.dumps({"event":"SHUTDOWN", "data":{"reason":"guest-shutdown", "guest":False}}))
        r = self.run_tool("session", "stop")
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse((self.vm / "image-clean.json").exists())

    def test_panic_after_shutdown_refused(self):
        self.start()
        event = next((self.vm / "logs").glob("*_qmp_events.jsonl"))
        event.write_text(event.read_text() + json.dumps({"event":"GUEST_PANICKED"}) + "\n")
        r = self.run_tool("session", "stop")
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse((self.vm / "image-clean.json").exists())

    def test_package_requires_clean_receipt(self):
        self.start()
        r = self.package()
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(self.calls("qemu-img"))

    def test_missing_receipt_cannot_create_artifact(self):
        self.ready()
        (self.vm / "image-clean.json").unlink()
        r = self.package()
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(self.out.exists())

    def test_changed_disk_refused(self):
        self.ready()
        (self.vm / "disk/image-build.qcow2").write_bytes(b"changed")
        r = self.package()
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(self.calls("qemu-img"))

    def test_touched_disk_refused(self):
        self.ready()
        os.utime(self.vm / "disk/image-build.qcow2", ns=(1, 1))
        r = self.package()
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(self.out.exists())

    def test_base_changed_after_shutdown_refused(self):
        self.ready()
        (self.vm / "disk/base.qcow2").write_bytes(b"changed base")
        r = self.package()
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(self.calls("qemu-img"))

    def test_base_touched_after_shutdown_refused(self):
        self.ready()
        os.utime(self.vm / "disk/base.qcow2", ns=(1, 1))
        r = self.package()
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(self.out.exists())

    def test_changed_identity_refused(self):
        self.ready()
        (self.vm / "identity.env").write_text("changed identity")
        r = self.package()
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(self.out.exists())

    def test_missing_identity_refused(self):
        self.ready()
        (self.vm / "identity.env").unlink()
        r = self.package()
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(self.calls("qemu-img"))

    def test_identity_receipt_mismatch_refused(self):
        self.ready()
        path = self.vm / "image-clean.json"
        data = json.loads(path.read_text())
        data["identity_sha256"] = "0" * 64
        path.write_text(json.dumps(data))
        r = self.package()
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(self.out.exists())

    def test_event_log_changed_after_receipt_refused(self):
        self.ready()
        event = next((self.vm / "logs").glob("*_qmp_events.jsonl"))
        event.write_text(event.read_text() + "\n")
        r = self.package()
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(self.out.exists())

    def test_new_event_after_shutdown_refused(self):
        self.ready()
        event = next((self.vm / "logs").glob("*_qmp_events.jsonl"))
        event.write_text(event.read_text() + json.dumps({"event":"RESUME"}) + "\n")
        r = self.package()
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(self.calls("qemu-img"))

    def test_inventory_file_changed_after_receipt_refused(self):
        self.ready()
        path = self.vm / "image-inventory.json"
        data = json.loads(path.read_text())
        data["windows"]["build"] = "changed"
        path.write_text(json.dumps(data))
        r = self.package()
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(self.out.exists())

    def test_inventory_whitespace_change_refused(self):
        self.ready()
        path = self.vm / "image-inventory.json"
        path.write_text(path.read_text() + "\n")
        r = self.package()
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(self.calls("qemu-img"))

    def test_wrong_receipt_tag_refused(self):
        self.ready()
        path = self.vm / "image-clean.json"
        data = json.loads(path.read_text())
        data["tag"] = "wrong-session"
        path.write_text(json.dumps(data))
        r = self.package()
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(self.out.exists())

    def test_nonclean_session_phase_refused(self):
        self.ready()
        path = self.vm / "image-session.json"
        data = json.loads(path.read_text())
        data["phase"] = "interactive"
        path.write_text(json.dumps(data))
        r = self.package()
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(self.calls("qemu-img"))

    def test_existing_output_directory_is_not_replaced(self):
        self.ready()
        self.out.mkdir()
        (self.out / "owner-file").write_text("keep")
        r = self.package()
        self.assertNotEqual(r.returncode, 0)
        self.assertEqual((self.out / "owner-file").read_text(), "keep")
        self.assertFalse(self.calls("qemu-img"))

    def test_package_inside_checkout_refused(self):
        self.ready()
        self.out = self.win.parents[2] / "image-package"
        r = self.package()
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(self.out.exists())

    def test_manual_installer_short_hash_refused(self):
        self.ready()
        path = self.tmp / "installers.json"
        path.write_text(json.dumps([dict(name="fixture", version="1", official_url="https://example.invalid",
                        sha256="1234", manual_steps="fixture", throwaway_account_note="none", license_notes="fixture")]))
        r = self.package("--installers", path)
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(self.calls("qemu-img"))

    def test_manual_installer_insecure_url_refused(self):
        self.ready()
        path = self.tmp / "installers.json"
        path.write_text(json.dumps([dict(name="fixture", version="1", official_url="http://example.invalid",
                        sha256="0" * 64, manual_steps="fixture", throwaway_account_note="none", license_notes="fixture")]))
        r = self.package("--installers", path)
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(self.out.exists())

    def test_busy_vm_refused_before_qemu_img(self):
        self.ready()
        with (self.vm / "run/vm.lock").open("w") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            r = self.package()
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(self.calls("qemu-img"))

    def test_busy_vm_never_creates_output_directory(self):
        self.ready()
        with (self.vm / "run/vm.lock").open("w") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            r = self.package()
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(self.out.exists())

    def test_check_failure_does_not_convert(self):
        self.ready()
        r = self.package(FAIL_QEMU="check")
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(any(x[0] == "convert" for x in self.calls("qemu-img")))

    def test_check_failure_publishes_nothing(self):
        self.ready()
        r = self.package(FAIL_QEMU="check")
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(self.out.exists())

    def test_manifest_records_artifact_pins_inventory_revision(self):
        m = self.built()
        self.assertEqual(m["artifact"]["sha256"], hashlib.sha256(ARTIFACT).hexdigest())
        self.assertEqual(m["artifact"]["bytes"], len(ARTIFACT))
        self.assertEqual(m["qemu_img_info"]["format"], "qcow2")
        self.assertEqual(len(m["media_pins"]), 4)
        self.assertTrue(all(len(x["sha256"]) == 64 for x in m["media_pins"]))
        self.assertEqual(len(m["host_kayfabe_revision"]), 40)
        self.assertTrue(m["inventory"]["prepared"])
        self.assertRegex(m["build_date"], r"^202\d-.*Z$")

    def test_convert_is_compressed_flattened_qcow2(self):
        self.built()
        calls = self.calls("qemu-img")
        convert = next(x for x in calls if x[0] == "convert")
        self.assertIn("-c", convert)
        self.assertEqual(convert[convert.index("-O")+1], "qcow2")
        self.assertNotIn("-B", convert)
        self.assertEqual(calls[0][0], "check")
        self.assertTrue(any(x[0] == "compare" for x in calls))

    def test_exact_repo_media_pins_are_embedded(self):
        m = self.built()
        source = (WIN / "win_vm.sh").read_text()
        for pin in m["media_pins"]:
            self.assertIn(f'{pin["name"]}|{pin["official_url"]}|{pin["sha256"]}|{pin["bytes"]}', source)
        self.assertEqual({x["name"] for x in m["media_pins"]}, {
            "win11_ltsc_2024_eval_x64_en-us.iso", "virtio-win-0.1.302.iso",
            "OpenSSH-Win64-v10.0.0.0.msi", "580.88-desktop-win10-win11-64bit-international-dch-whql.exe"})

    def test_extra_backing_parent_is_refused(self):
        self.ready()
        r = self.package(BAD_CHAIN="1")
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(self.out.exists())

    def test_unknown_source_chain_never_converts(self):
        self.ready()
        r = self.package(BAD_CHAIN="1")
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(any(x[0] == "convert" for x in self.calls("qemu-img")))

    def test_output_backing_file_refused(self):
        self.ready()
        r = self.package(BACKING="1")
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(self.out.exists())

    def test_output_external_dependency_never_published(self):
        self.ready()
        r = self.package(BACKING="1")
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse((self.out / "manifest.json").exists())

    def test_nested_external_data_file_refused(self):
        self.ready()
        r = self.package(EXTERNAL_DATA="1")
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(self.out.exists())

    def test_compare_failure_removes_partial_package(self):
        self.ready()
        r = self.package(FAIL_QEMU="compare")
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(self.out.exists())

    def test_compare_failure_never_publishes_manifest(self):
        self.ready()
        r = self.package(FAIL_QEMU="compare")
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse((self.out / "manifest.json").exists())

    def test_conversion_failure_does_not_publish_manifest(self):
        self.ready()
        r = self.package(FAIL_QEMU="convert")
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse((self.out / "manifest.json").exists())

    def test_upload_put_retries_and_hides_url(self):
        self.built()
        r = self.run_tool("package", "upload", "--package", self.out, "--url-file", self.url)
        self.assertEqual(r.returncode, 0, r.stderr)
        curl = self.calls("curl")[-1]
        self.assertIn("--upload-file", curl)
        self.assertIn("--config", curl)
        self.assertNotIn("signature=", str(curl) + r.stdout + r.stderr)
        self.assertNotIn("--location", curl)

    def test_upload_corruption_refused_before_network(self):
        self.built()
        (self.out / "windows.qcow2").write_bytes(b"corruption")
        r = self.run_tool("package", "upload", "--package", self.out, "--url-file", self.url)
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(self.calls("curl"))

    def test_upload_same_size_corruption_refused(self):
        self.built()
        (self.out / "windows.qcow2").write_bytes(b"x" * len(ARTIFACT))
        r = self.run_tool("package", "upload", "--package", self.out, "--url-file", self.url)
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(self.calls("curl"))

    def test_verify_download_matches_manifest(self):
        self.built()
        r = self.run_tool("package", "verify", "--package", self.out, "--url-file", self.url)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("VERIFIED", r.stdout)

    def test_redirect_is_not_acknowledged_as_upload(self):
        self.built()
        r = self.run_tool("package", "upload", "--package", self.out, "--url-file", self.url, HTTP_STATUS="302")
        self.assertNotEqual(r.returncode, 0)
        self.assertNotIn("UPLOADED", r.stdout)

    def test_redirect_get_rejected_even_with_matching_body(self):
        self.built()
        r = self.run_tool("package", "verify", "--package", self.out, "--url-file", self.url, HTTP_STATUS="302")
        self.assertNotEqual(r.returncode, 0)
        self.assertNotIn("VERIFIED", r.stdout)

    def test_verify_hash_mismatch_fails(self):
        self.built()
        r = self.run_tool("package", "verify", "--package", self.out, "--url-file", self.url,
                          BAD_DOWNLOAD="1")
        self.assertNotEqual(r.returncode, 0)
        self.assertNotIn("VERIFIED", r.stdout)

    def test_wrong_download_is_not_retained(self):
        self.built()
        r = self.run_tool("package", "verify", "--package", self.out, "--url-file", self.url,
                          BAD_DOWNLOAD="1")
        self.assertNotEqual(r.returncode, 0)
        self.assertEqual(sorted(x.name for x in self.out.iterdir()), ["manifest.json", "windows.qcow2"])

    def test_curl_error_does_not_leak_signed_url(self):
        self.built()
        r = self.run_tool("package", "upload", "--package", self.out, "--url-file", self.url, FAIL_CURL="1")
        self.assertNotEqual(r.returncode, 0)
        self.assertNotIn("signature=", r.stderr + r.stdout)

    def test_failed_get_hides_signed_url_too(self):
        self.built()
        r = self.run_tool("package", "verify", "--package", self.out, "--url-file", self.url, FAIL_CURL="1")
        self.assertNotEqual(r.returncode, 0)
        self.assertNotIn("signature=", r.stderr + r.stdout)

    def test_upload_size_limit_before_hashing(self):
        self.built()
        path = self.out / "manifest.json"
        data = json.loads(path.read_text())
        data["artifact"]["bytes"] = 5 * 1024**3 + 1
        path.write_text(json.dumps(data))
        r = self.run_tool("package", "upload", "--package", self.out, "--url-file", self.url)
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("5 GiB", r.stderr)
        self.assertFalse(self.calls("curl"))

    def test_multipart_required_for_large_record(self):
        self.built()
        path = self.out / "manifest.json"
        data = json.loads(path.read_text())
        data["artifact"]["bytes"] = 6 * 1024**3
        path.write_text(json.dumps(data))
        r = self.run_tool("package", "upload", "--package", self.out, "--url-file", self.url)
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("multipart", r.stderr)
        self.assertFalse(self.calls("curl"))

    def test_signed_url_must_be_https(self):
        self.built()
        self.url.write_text("http://example.invalid/object?signature=fixture\n")
        r = self.run_tool("package", "upload", "--package", self.out, "--url-file", self.url)
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(self.calls("curl"))

    def test_verify_refuses_plain_http_too(self):
        self.built()
        self.url.write_text("http://example.invalid/object\n")
        r = self.run_tool("package", "verify", "--package", self.out, "--url-file", self.url)
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(self.calls("curl"))

    def test_curl_config_injection_refused(self):
        self.built()
        self.url.write_text('https://example.invalid/"\noutput = "/tmp/attack"\n')
        r = self.run_tool("package", "verify", "--package", self.out, "--url-file", self.url)
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(self.calls("curl"))

    def test_quoted_url_cannot_inject_curl_option(self):
        self.built()
        self.url.write_text('https://example.invalid/"attack\n')
        r = self.run_tool("package", "upload", "--package", self.out, "--url-file", self.url)
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(self.calls("curl"))

    def test_parent_artifact_name_refused_on_verify(self):
        self.built()
        path = self.out / "manifest.json"
        data = json.loads(path.read_text())
        data["artifact"]["name"] = "../escaped.qcow2"
        path.write_text(json.dumps(data))
        r = self.run_tool("package", "verify", "--package", self.out, "--url-file", self.url)
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(self.calls("curl"))

    def test_absolute_artifact_name_refused_on_verify(self):
        self.built()
        path = self.out / "manifest.json"
        data = json.loads(path.read_text())
        data["artifact"]["name"] = str(self.tmp / "escaped.qcow2")
        path.write_text(json.dumps(data))
        r = self.run_tool("package", "verify", "--package", self.out, "--url-file", self.url)
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(self.calls("curl"))


class GuestSourceContracts(unittest.TestCase):
    def source(self):
        return (WIN / "image_prepare_guest.ps1").read_text()

    def test_power_and_time_commands_fail_closed(self):
        s = self.source()
        for token in ("powercfg.exe /h off", "HiberbootEnabled", "Stop-Service -Name W32Time",
                      "Set-Service -Name W32Time -StartupType Disabled", "$ErrorActionPreference = 'Stop'",
                      "Set-ItemProperty -LiteralPath $power -Name HiberbootEnabled -Type DWord -Value 0"):
            self.assertIn(token, s)

    def test_inventory_records_shutdown_preconditions(self):
        s = self.source()
        for token in ("hibernation_disabled = $true", "fast_startup_disabled = $true",
                      "time_service_disabled = $true", "bitlocker_decrypted = $true",
                      "Get-BitLockerVolume", "Get-Package", "Get-FileHash", "SHA256",
                      "WOW6432Node", "Win32_PnPSignedDriver", "CurrentBuild"):
            self.assertIn(token, s)

    def test_commands_and_inventory_precede_zeroing_and_commit(self):
        s = self.source()
        for command in ("powercfg.exe /h off", "Stop-Service -Name W32Time",
                        "Set-Service -Name W32Time -StartupType Disabled", "Get-Package",
                        "Get-FileHash", "Get-BitLockerVolume",
                        "Set-ItemProperty -LiteralPath $power -Name HiberbootEnabled -Type DWord -Value 0"):
            self.assertLess(s.index(command), s.index("# ZERO_FILL_START"))
        self.assertLess(s.index("# ZERO_FILL_START"), s.index("# INVENTORY_COMMIT"))

    def test_zero_stream_is_cleaned_and_success_recorded(self):
        s = self.source()
        for token in ("sdelete64.exe", "'-z'", "FileMode]::CreateNew", "New-Object byte[]",
                      ".Flush($true)", "Remove-Item -LiteralPath $zeroPath", "completed = $true"):
            self.assertLess(s.index(token), s.index("# INVENTORY_COMMIT"))

    def test_zeroing_uses_only_local_tool_or_zero_stream(self):
        s = self.source()
        for token in ("sdelete64.exe", "'-z'", "FileMode]::CreateNew", "New-Object byte[]",
                      ".Flush($true)", "finally", "Remove-Item -LiteralPath $zeroPath"):
            self.assertIn(token, s)
        self.assertNotIn("Invoke-WebRequest", s)

    def test_inventory_is_written_after_zeroing(self):
        s = self.source()
        self.assertLess(s.index("# ZERO_FILL_START"), s.index("# INVENTORY_COMMIT"))
        self.assertIn("completed = $true", s)
        self.assertIn("session_id = $SessionId", s)
        self.assertIn("exclusions =", s)
        self.assertIn("ReparsePoint", s)
        self.assertLess(s.index(".Flush($true)"), s.index("Set-Content -LiteralPath ($OutputPath"))

    def test_guest_writes_only_successful_preconditions(self):
        s = self.source()
        for token in ("hibernation_disabled = $true", "fast_startup_disabled = $true",
                      "time_service_disabled = $true", "bitlocker_decrypted = $true"):
            self.assertGreater(s.index(token), s.index("# INVENTORY_COMMIT"))
        self.assertIn("$ErrorActionPreference = 'Stop'", s)

    def test_file_inventory_records_exclusions_and_refuses_reparse_points(self):
        s = self.source()
        for token in ("ReparsePoint", "$excluded.Add", "exclusions =", "$rule.reason",
                      "Get-FileHash", "SHA256"):
            self.assertIn(token, s)

    def test_registry_driver_and_build_inventory_coverage(self):
        s = self.source()
        for token in ("HKLM:\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
                      "WOW6432Node", "HKCU:\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
                      "Get-Package", "Win32_PnPSignedDriver", "CurrentBuild", "exclusions =", "$rule.reason"):
            self.assertIn(token, s)

    def test_guest_network_downloads_are_absent(self):
        s = self.source()
        for token in ("Invoke-WebRequest", "Invoke-RestMethod", "Start-BitsTransfer", "WebClient"):
            self.assertNotIn(token, s)
        self.assertIn("sdelete64.exe", s)


class LauncherNetworkDefaults(unittest.TestCase):
    def dry(self, restriction=None):
        with tempfile.TemporaryDirectory(prefix="y1-net-fixture-") as tmp:
            env = dict(PATH=os.environ["PATH"], HOME=tmp, WINVM_ROOT=tmp,
                       WINVM_QEMU="/fixture/qemu", WINVM_NAME="build", WINVM_DRY_RUN="1")
            if restriction is not None:
                env["WINVM_NET_RESTRICT"] = restriction
            return subprocess.run(["bash", str(WIN / "win_vm.sh"), "run", "--no-kf3", "--dry-run", "--tag", "fixture"],
                                  env=env, capture_output=True, text=True, timeout=30)

    def argv(self, r):
        self.assertEqual(r.returncode, 0, r.stderr)
        line = next(x for x in r.stdout.splitlines() if x.startswith("DRYRUN_QEMU "))
        args = shlex.split(line)[1:]
        # Each dry run uses a distinct scratch root. Normalize that path only.
        return [re.sub(r"/tmp/y1-net-fixture-[^/ ,]+", "/fixture/vms", x) for x in args]

    def test_network_default_is_byte_identical_to_explicit_on(self):
        self.assertEqual(self.argv(self.dry()), self.argv(self.dry("on")))

    def test_full_default_argv_matches_original_next_fixture(self):
        fixture = read_argv_fixture()
        self.assertEqual(self.argv(self.dry()), fixture["argv"])

    def test_unset_default_retains_exact_old_netdev(self):
        args = self.argv(self.dry())
        self.assertEqual(args[args.index("-netdev")+1],
                         "user,id=n0,restrict=on,hostfwd=tcp:127.0.0.1:2224-:22")

    def test_outbound_opt_in_changes_only_restriction(self):
        normal = self.argv(self.dry())
        online = self.argv(self.dry("off"))
        self.assertIn("restrict=off", online[online.index("-netdev")+1])
        self.assertEqual(normal, [x.replace("restrict=off", "restrict=on") for x in online])

    def test_invalid_network_restriction_never_emits_qemu(self):
        for value in ("", "OFF"):
            with self.subTest(value=value):
                r = self.dry(value)
                self.assertNotEqual(r.returncode, 0)
                self.assertNotIn("DRYRUN_QEMU", r.stdout)
    def test_network_option_injection_is_rejected(self):
        for value in ("off,hostfwd=tcp::1234-:22", "on extra"):
            with self.subTest(value=value):
                r = self.dry(value)
                self.assertNotEqual(r.returncode, 0)
                self.assertNotIn("DRYRUN_QEMU", r.stdout)


def read_argv_fixture():
    return json.loads((ROOT / "scripts/ci/fixtures/windows_image_default_argv.json").read_text())


if __name__ == "__main__":
    unittest.main()
