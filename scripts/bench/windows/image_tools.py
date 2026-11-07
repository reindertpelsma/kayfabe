#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Image session/packaging implementation (standard library only, Linux host).

Commands execute the sibling win_vm.sh, qemu-img, and curl; none rents or deploys.
Receipts are local accident-prevention records, not an attestation of a hostile guest.
"""
from __future__ import annotations

import argparse
from contextlib import contextmanager
from datetime import datetime, timezone
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import stat
import subprocess
import sys
import tempfile
import time
from urllib.parse import urlsplit

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[2]
LAUNCHER = HERE / "win_vm.sh"
OVERLAY = "image-build"
ACCOUNT_COMMAND = (
    "$ErrorActionPreference='Stop'; "
    "$p=[Console]::In.ReadToEnd(); "
    "Set-LocalUser -Name 'kf' -Password (ConvertTo-SecureString $p -AsPlainText -Force); "
    "$p=$null; Write-Output 'IMAGE_ACCOUNT_READY'"
)


class Refused(Exception):
    pass


def require(ok, message):
    if not ok:
        raise Refused(message)


def read_json(path):
    # utf-8-sig also accepts the BOM Windows PowerShell 5.1 writes.
    return json.loads(Path(path).read_text(encoding="utf-8-sig"))


def write_json(path, data):
    path = Path(path)
    temp = path.with_suffix(path.suffix + ".tmp")
    with temp.open("x", encoding="utf-8") as f:
        json.dump(data, f, indent=2, sort_keys=True)
        f.write("\n")
    temp.replace(path)


def digest(path):
    with Path(path).open("rb") as f:
        return hashlib.file_digest(f, "sha256").hexdigest()


def fingerprint(path):
    s = Path(path).stat()
    return dict(device=s.st_dev, inode=s.st_ino, bytes=s.st_size,
                mtime_ns=s.st_mtime_ns, ctime_ns=s.st_ctime_ns)


def disk_fingerprints(vm, state):
    # win_vm creates this fresh overlay with exactly base.qcow2 as its parent.
    return {"overlay": fingerprint(state["disk"]), "base": fingerprint(vm / "disk/base.qcow2")}


def command(argv, *, data=None, timeout=None, env=None):
    # Guest output and curl diagnostics may contain secrets. Report only the tool
    # and exit code, never argv, stdin, stdout, stderr, or a CalledProcessError.
    r = subprocess.run(list(map(str, argv)), input=data, text=True, capture_output=True,
                       timeout=timeout, env=env)
    require(r.returncode == 0, f"{Path(argv[0]).name} failed (exit {r.returncode}); no receipt published")
    return r.stdout


def win(*args, **kw):
    return command([str(LAUNCHER), *args], **kw)


def vm_path():
    name = os.environ.get("WINVM_NAME", "kfwin")
    require(re.fullmatch(r"[A-Za-z0-9_-]+", name), "WINVM_NAME must be a plain name")
    root = Path(os.environ.get("WINVM_ROOT", "/workspace/winvm")).resolve()
    require(not root.is_relative_to(REPO), "VM state must live outside the checkout")
    return root / name


@contextmanager
def lock(path, wait=0):
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    with path.open("a") as f:
        deadline = time.monotonic() + wait
        while True:
            try:
                fcntl.flock(f, fcntl.LOCK_EX | fcntl.LOCK_NB)
                break
            except BlockingIOError:
                require(time.monotonic() < deadline, "VM/session is busy; retry after it exits")
                time.sleep(0.1)
        yield


def private_text(path):
    path = Path(path).absolute()
    require(not path.resolve().is_relative_to(REPO), "secret input must live outside the checkout")
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    with os.fdopen(fd, encoding="utf-8") as f:
        s = os.fstat(f.fileno())
        require(stat.S_ISREG(s.st_mode) and s.st_uid == os.getuid() and s.st_mode & 0o077 == 0,
                "secret input must be an owner-only regular file (0600)")
        require(s.st_size <= 16384, "secret input exceeds size limit")
        return f.read()


def password(path):
    lines = [s for s in private_text(path).splitlines() if s and not s.startswith("#")]
    require(len(lines) == 1 and lines[0].startswith("WIN_IMAGE_PASSWORD="),
            "env file must contain only WIN_IMAGE_PASSWORD=value and optional comments")
    value = lines[0].partition("=")[2]
    require(1 <= len(value) <= 256 and not any(ord(c) < 32 for c in value),
            "password must be one nonempty line of at most 256 characters")
    return value


def console(state, vm):
    if state.get("display_mode") != "std-vga":
        print("UNTESTED DESIGN: kf3 viewing uses broker + headless X11 compositor + browser-screencast; no live kf3 VNC")
        print("Start these prerequisites in separate builder terminals; reserve display :99 first:")
        print("browser-screencast --headless --headless-display :99 --headless-wm openbox --listen 127.0.0.1 --port 6081 --capture x11grab --generate-token --no-audio")
        print("env DISPLAY=:99 picom --backend xrender")
        print(shlex.join(["env", "DISPLAY=:99", state.get("broker_bin", "/opt/nvkvm-broker/nvkvm-display-broker"),
                          "--socket", state.get("broker_socket", "<owned-broker.sock>"), "--backend", "x11", "--persist",
                          "--title", "Windows image build"]))
        print(shlex.join(["ssh", "-N", "-p", str(state["host_ssh_port"]), "-L",
                          "127.0.0.1:6081:127.0.0.1:6081", state["ssh_host"]]))
        print("Open the browser-screencast token URL privately; do not copy the token to logs or manifests.")
        print("UNTESTED fallback: poll this QMP screendump command, then use QMP input; no live-console dependency:")
        print(shlex.join(["python3", str(HERE / "qmp.py"), str(vm / "run/qmp.sock"), "cmd", "screendump",
                          json.dumps({"filename": str(vm / "logs/console.ppm")})]))
        print(shlex.join(["python3", str(HERE / "qmp.py"), str(vm / "run/qmp.sock"), "cmd", "send-key",
                          json.dumps({"keys": [{"type": "qcode", "data": "ret"}]})]))
        return
    print("STD VGA install/app stage: no kf3 device; GPU-dependent steps belong to the UNTESTED broker stage.")
    forward = "127.0.0.1:6080:127.0.0.1:6080"
    print("On the Linux builder:")
    print(shlex.join(["websockify", "--web", "/usr/share/novnc", "--unix-target",
                      str(vm / "run/vnc.sock"), "127.0.0.1:6080"]))
    print("On your workstation:")
    print(shlex.join(["ssh", "-N", "-p", str(state["host_ssh_port"]), "-L", forward, state["ssh_host"]]))
    print("Open http://127.0.0.1:6080/vnc.html?autoconnect=1 ; local Windows account: kf")


def validate_inventory(data, tag):
    require(data.get("schema") == "kayfabe-windows-inventory/1" and data.get("session_id") == tag,
            "inventory schema/session does not match this session")
    for key in ("prepared", "hibernation_disabled", "fast_startup_disabled",
                "time_service_disabled", "bitlocker_decrypted"):
        require(data.get(key) is True, f"inventory precondition failed: {key}")
    require(data.get("zero_fill", {}).get("completed") is True, "zero fill did not complete")
    for key in ("drivers", "packages", "uninstall_registry", "app_roots", "files", "exclusions"):
        require(isinstance(data.get(key), list), f"inventory is missing {key}")
    require(isinstance(data.get("windows"), dict), "inventory is missing Windows build")


def clean_shutdown(vm, state):
    events = list((vm / "logs").glob(f"{state['tag']}_b*_qmp_events.jsonl"))
    require(events, "no QMP shutdown evidence for the build session")
    event = max(events, key=lambda p: int(re.search(r"_b(\d+)_", p.name).group(1)))
    rows = [json.loads(s) for s in event.read_text().splitlines() if s.strip()]
    require(not any(x.get("event") == "GUEST_PANICKED" for x in rows), "guest panic in final boot")
    shuts = [x.get("data", {}) for x in rows if x.get("event") == "SHUTDOWN"]
    require(shuts and shuts[-1].get("guest") is True and shuts[-1].get("reason") == "guest-shutdown",
            "last QMP event is not a clean guest shutdown (QMP quit/reset is insufficient)")
    return {"path": str(event), "sha256": digest(event)}


def session_start(args, vm):
    pw = password(args.env_file)  # validate before install/boot, never source or export
    require(re.fullmatch(r"[A-Za-z0-9_.@:-]+", args.ssh_host) and not args.ssh_host.startswith("-"),
            "--ssh-host must be a plain SSH destination")
    require(1 <= args.host_ssh_port <= 65535, "invalid Linux SSH port")
    require(not (vm / "image-session.json").exists() or args.resume, "session already exists; stop and package it first")
    broker = None
    if args.display_mode == "broker":
        require(os.environ.get("WINVM_KF3_DISPLAY", "on") == "on", "broker mode needs WINVM_KF3_DISPLAY=on; off has no display")
        require(args.broker_socket is not None, "broker mode needs --broker-socket from the caller's running broker")
        broker = args.broker_socket.absolute()
        require(broker.is_absolute() and not re.search(r"[,\s]", str(broker)), "broker socket path cannot contain QEMU separators or whitespace")
        require(broker.is_socket() and broker.stat().st_uid == os.getuid(), "broker mode needs a live caller-owned Unix socket")
    with lock(vm / "run/vm.lock"):
        require(not (vm / f"disk/{OVERLAY}.qcow2").exists() or args.resume,
                "image-build overlay already exists; use a fresh WINVM_NAME")
        if args.resume:
            previous = read_json(vm / "image-session.json")
            receipt = read_json(vm / "image-clean.json")
            require(args.verb == "start" and previous.get("phase") == "clean" and receipt.get("tag") == previous["tag"],
                    "resume requires a cleanly stopped session")
            require(disk_fingerprints(vm, previous) == receipt.get("disks"), "resume source changed after shutdown")
    if args.verb == "install":
        win("install", env=dict(os.environ, WINVM_VM_IDENTITY="1",
            WINVM_NET_RESTRICT="off" if args.allow_internet else "on"), timeout=18000)
    require((vm / "installed").exists(), "install the VM first with image_session.sh install")
    require(not os.environ.get("WINVM_DRY_RUN"), "image sessions require a real run, not WINVM_DRY_RUN")
    tag = "image_" + datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")
    state = dict(tag=tag, disk=str(vm / f"disk/{OVERLAY}.qcow2"),
                 ssh_host=args.ssh_host, host_ssh_port=args.host_ssh_port, phase="starting",
                 network_restrict="off" if args.allow_internet else "on",
                 display_mode=args.display_mode, broker_socket=str(broker) if broker else None,
                 broker_bin=str(args.broker_bin))
    write_json(vm / "image-session.json", state)
    (vm / "image-clean.json").unlink(missing_ok=True)
    # win_vm takes its own vm.lock before launching. We must release ours first.
    display_args = ["--no-kf3"] if args.display_mode == "std-vga" else [
        "--kf3-extra", f"display-broker={broker},display-broker-uid={os.getuid()}"]
    win("run", "--overlay", OVERLAY, "--tag", tag, "--detach", *display_args,
        env=dict(os.environ, WINVM_VM_IDENTITY="1", WINVM_NET_RESTRICT=state["network_restrict"]), timeout=30)
    console(state, vm)
    deadline = time.monotonic() + args.wait_seconds
    while True:
        try:
            out = win("ssh", ACCOUNT_COMMAND, data=pw, timeout=30)
            require("IMAGE_ACCOUNT_READY" in out, "guest did not confirm account setup")
            break
        except (Refused, subprocess.TimeoutExpired):
            require(time.monotonic() < deadline, "guest account setup timed out; session remains available for stop")
            time.sleep(min(5, max(0, deadline - time.monotonic())))
    state["phase"] = "interactive"
    write_json(vm / "image-session.json", state)
    print("IMAGE_SESSION_READY; install through official installers, then run image_session.sh stop")


def session_stop(args, vm):
    state = read_json(vm / "image-session.json")
    require(state.get("phase") != "clean", "session already stopped cleanly")
    (vm / "image-clean.json").unlink(missing_ok=True)
    config = read_json(args.guest_config) if args.guest_config else {"app_roots": [], "exclude": []}
    require(isinstance(config.get("app_roots"), list) and isinstance(config.get("exclude"), list),
            "guest config requires app_roots and exclude arrays")
    with tempfile.TemporaryDirectory(prefix="image-prepare-", dir=vm) as tmp:
        cfg = Path(tmp) / "config.json"
        write_json(cfg, config)
        win("scp-to", HERE / "image_prepare_guest.ps1", "C:/kf/image_prepare_guest.ps1", timeout=120)
        win("scp-to", cfg, "C:/kf/image-config.json", timeout=120)
        win("ssh", "powershell -NoProfile -ExecutionPolicy Bypass -File C:\\kf\\image_prepare_guest.ps1 "
            f"-SessionId {state['tag']}", timeout=14400)
        inv = Path(tmp) / "inventory.json"
        win("scp-from", "C:/kf/image-inventory.json", inv, timeout=120)
        data = read_json(inv)
        validate_inventory(data, state["tag"])
        write_json(vm / "image-inventory.json", data)
    # The image holds installed software, not these preparation/runtime scripts.
    win("ssh", "$ErrorActionPreference='Stop'; $paths=@("
        "'C:\\kf\\image_prepare_guest.ps1','C:\\kf\\image-config.json',"
        "'C:\\kf\\image-inventory.json','C:\\kf\\check.ps1'); "
        "foreach($path in $paths){if(Test-Path -LiteralPath $path){Remove-Item -LiteralPath $path -Force}}", timeout=120)
    win("stop", timeout=360)
    with lock(vm / "run/vm.lock", wait=30):
        qmp = clean_shutdown(vm, state)
        receipt = dict(tag=state["tag"], disks=disk_fingerprints(vm, state), qmp=qmp,
                       identity_sha256=digest(vm / "identity.env"),
                       inventory_sha256=digest(vm / "image-inventory.json"))
        write_json(vm / "image-clean.json", receipt)
        state["phase"] = "clean"
        write_json(vm / "image-session.json", state)
    print("IMAGE_SESSION_CLEAN; ready for image_package.sh create")


def media_pins():
    source = LAUNCHER.read_text()
    block = source.split("MEDIA=(", 1)[1].split("\n)", 1)[0]
    rows = re.findall(r'^  "([^|"\n]+)\|(https://[^|"\n]+)\|([0-9a-f]{64})\|([0-9]+)"$', block, re.M)
    require(rows, "no media pins found in the repo launcher")
    return [dict(name=n, official_url=u, sha256=h, bytes=int(b)) for n, u, h, b in rows]


def installer_records(path):
    rows = read_json(path) if path else []
    require(isinstance(rows, list), "manual installer checklist must be a JSON array")
    for row in rows:
        require(isinstance(row, dict) and all(isinstance(row.get(k), str) and row[k].strip()
                for k in ("name", "version", "official_url", "sha256", "manual_steps",
                          "throwaway_account_note", "license_notes")), "incomplete manual installer record")
        require(re.fullmatch(r"[0-9a-f]{64}", row["sha256"]) and row["official_url"].startswith("https://"),
                "manual installer record requires full SHA-256 and official HTTPS URL")
    return rows


def no_external_data(value):
    if isinstance(value, dict):
        return not any(k in value for k in ("backing-filename", "full-backing-filename", "data-file")) and all(
            no_external_data(v) for v in value.values())
    if isinstance(value, list):
        return all(no_external_data(v) for v in value)
    return True


def create_package(args, vm):
    with lock(vm / "run/vm.lock"):
        state = read_json(vm / "image-session.json")
        receipt = read_json(vm / "image-clean.json")
        disk = Path(state["disk"])
        require(state.get("phase") == "clean" and receipt.get("tag") == state["tag"],
                "session has no completed clean-shutdown receipt")
        require(disk_fingerprints(vm, state) == receipt.get("disks"), "disk changed after clean shutdown")
        require(digest(vm / "identity.env") == receipt.get("identity_sha256"), "VM identity changed")
        require(clean_shutdown(vm, state) == receipt.get("qmp"), "shutdown evidence changed")
        require(digest(vm / "image-inventory.json") == receipt.get("inventory_sha256"), "inventory changed")
        inv = read_json(vm / "image-inventory.json")
        validate_inventory(inv, state["tag"])
        pins = media_pins()
        installers = installer_records(args.installers)
        revision = command(["git", "-C", REPO, "rev-parse", "HEAD"]).strip()
        dirty = bool(command(["git", "-C", REPO, "status", "--porcelain", "--untracked-files=no"]).strip())
        out = args.out.resolve()
        require(not out.is_relative_to(REPO) and not out.exists(), "--out must be a new directory outside the checkout")
        out.parent.mkdir(parents=True, exist_ok=True)
        # qemu-img refuses open writers too; never use -U, repair, salvage, or commit.
        command(["qemu-img", "check", "-f", "qcow2", disk])
        chain = json.loads(command(["qemu-img", "info", "--backing-chain", "--output=json", disk]))
        require(isinstance(chain, list) and len(chain) == 2 and all(x.get("format") == "qcow2" for x in chain)
                and Path(chain[0]["filename"]).resolve() == disk.resolve()
                and Path(chain[1]["filename"]).resolve() == (vm / "disk/base.qcow2").resolve(),
                "build disk must have exactly the known base.qcow2 parent")
        staging = Path(tempfile.mkdtemp(prefix=".image-package-", dir=out.parent))
        try:
            artifact = staging / "windows.qcow2"
            command(["qemu-img", "convert", "-c", "-f", "qcow2", "-O", "qcow2", disk, artifact])
            command(["qemu-img", "check", "-f", "qcow2", artifact])
            command(["qemu-img", "compare", "-f", "qcow2", "-F", "qcow2", disk, artifact])
            info = json.loads(command(["qemu-img", "info", "--output=json", "-f", "qcow2", artifact]))
            require(info.get("format") == "qcow2" and no_external_data(info),
                "converted artifact has external dependencies or unexpected format")
            require(disk_fingerprints(vm, state) == receipt["disks"], "source changed during conversion")
            # info's filename otherwise points to the temporary staging directory.
            info["filename"] = artifact.name
            manifest = dict(schema="kayfabe-windows-image/1", artifact=dict(name=artifact.name,
                            bytes=artifact.stat().st_size, sha256=digest(artifact)), qemu_img_info=info,
                            inventory=inv, media_pins=pins, host_kayfabe_revision=revision,
                            host_checkout_dirty=dirty, build_date=datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
                            identity=(vm / "identity.env").read_text(),
                            shutdown=receipt, manual_installers=installers)
            write_json(staging / "manifest.json", manifest)
            staging.rename(out)
        finally:
            if staging.exists():
                shutil.rmtree(staging)
    print(f"PACKAGED {out / 'manifest.json'}")


def artifact_record(package):
    manifest = read_json(package / "manifest.json")
    require(manifest.get("schema") == "kayfabe-windows-image/1", "unsupported manifest schema")
    a = manifest["artifact"]
    require(a.get("name") == "windows.qcow2" and re.fullmatch(r"[0-9a-f]{64}", a.get("sha256", ""))
            and type(a.get("bytes")) is int and a["bytes"] > 0, "invalid artifact record")
    return a


def matches(path, record):
    require(path.stat().st_size == record["bytes"] and digest(path) == record["sha256"],
            "artifact SHA-256/size mismatch")


def transfer(args):
    package = args.package.resolve()
    a = artifact_record(package)
    url = private_text(args.url_file).removesuffix("\n")
    require(not any(ord(c) <= 32 or c in '\\"' for c in url), "invalid presigned URL characters")
    parts = urlsplit(url)
    require(parts.scheme == "https" and parts.hostname and not parts.username and not parts.password
            and not parts.fragment, "a presigned HTTPS URL without userinfo/fragment is required")
    curl = ["curl", "-q", "--config", "-", "--proto", "=https", "--fail", "--silent",
            "--connect-timeout", "30", "--write-out", "%{http_code}",
            "--retry", "3", "--retry-all-errors", "--retry-delay", "2"]
    config = f'url = "{url}"\n'
    if args.verb == "upload":
        path = package / a["name"]
        require(a["bytes"] <= 5 * 1024**3, "single R2 PUT supports at most 5 GiB; use caller-managed multipart for larger artifacts")
        matches(path, a)
        status = command([*curl, "--upload-file", path, "--output", os.devnull], data=config)
        require(re.fullmatch(r"2[0-9]{2}", status.strip()), "PUT did not return HTTP 2xx")
        print("UPLOADED; verify with a separately supplied presigned GET")
    else:
        with tempfile.TemporaryDirectory(prefix=".image-verify-", dir=package.parent) as tmp:
            path = Path(tmp) / a["name"]
            status = command([*curl, "--output", path], data=config)
            require(re.fullmatch(r"2[0-9]{2}", status.strip()), "GET did not return HTTP 2xx")
            matches(path, a)
        print("VERIFIED SHA-256 and byte length match manifest.json")


def parser():
    p = argparse.ArgumentParser(description=__doc__)
    modes = p.add_subparsers(dest="mode", required=True)
    session = modes.add_parser("session", help="install/start/status/stop an owner image build")
    subs = session.add_subparsers(dest="verb", required=True)
    for name in ("install", "start"):
        sub = subs.add_parser(name)
        sub.add_argument("--env-file", type=Path, required=True, help="0600, outside repo: WIN_IMAGE_PASSWORD=literal value")
        sub.add_argument("--ssh-host", required=True, help="Linux builder SSH destination, e.g. root@builder")
        sub.add_argument("--host-ssh-port", type=int, default=22)
        sub.add_argument("--allow-internet", action="store_true", help="opt in to outbound guest networking for official installs/activation")
        sub.add_argument("--wait-seconds", type=int, default=600)
        sub.add_argument("--display-mode", choices=("std-vga", "broker"), default="std-vga",
                         help="std VGA for setup/installers; broker is an UNTESTED GPU stage")
        sub.add_argument("--broker-socket", type=Path, help="caller-owned running broker socket; broker stage only")
        sub.add_argument("--broker-bin", type=Path, default=Path("/opt/nvkvm-broker/nvkvm-display-broker"))
        sub.add_argument("--resume", action="store_true", help="start the same overlay again after a clean session stop")
    subs.add_parser("status")
    stop = subs.add_parser("stop", help="prepare guest, pull inventory, clean shutdown; no forced-stop receipt")
    stop.add_argument("--guest-config", type=Path, help="JSON: app_roots and exclude arrays")
    package = modes.add_parser("package", help="create/upload/verify a private image")
    subs = package.add_subparsers(dest="verb", required=True)
    create = subs.add_parser("create")
    create.add_argument("--out", type=Path, required=True)
    create.add_argument("--installers", type=Path, help="manual installer checklist JSON array to embed")
    for name in ("upload", "verify"):
        sub = subs.add_parser(name)
        sub.add_argument("--package", type=Path, required=True)
        sub.add_argument("--url-file", type=Path, required=True, help="0600 file holding caller-signed HTTPS PUT/GET URL")
    return p


def main():
    os.umask(0o077)
    args = parser().parse_args()
    try:
        if args.mode == "package" and args.verb != "create":
            transfer(args)
        else:
            vm = vm_path()
            with lock(vm / "run/image.lock"):
                if args.mode == "package":
                    create_package(args, vm)
                elif args.verb in ("install", "start"):
                    require(0 < args.wait_seconds <= 3600, "wait must be 1..3600 seconds")
                    session_start(args, vm)
                elif args.verb == "stop":
                    session_stop(args, vm)
                else:
                    print(win("status", timeout=30), end="")
                    if (vm / "image-session.json").exists():
                        state = read_json(vm / "image-session.json")
                        print(f"image phase: {state['phase']}")
                        console(state, vm)
        return 0
    except (Refused, OSError, ValueError, KeyError, IndexError, TypeError, subprocess.TimeoutExpired) as e:
        # No exception string: malformed secret/URL/guest JSON can echo its contents.
        message = str(e) if isinstance(e, Refused) else type(e).__name__
        print(f"REFUSED: {message}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
