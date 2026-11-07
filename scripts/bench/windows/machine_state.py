#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Design-only offline image state; never starts a VM or contacts a service."""
import argparse
import base64
import binascii
from contextlib import contextmanager
from datetime import datetime, timezone
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import sys
import tempfile
import uuid

CLASSIFICATION = "NON-SENSITIVE-BY-DESIGN: offline image guest material only"
REQUIRED = {"tpm/tpm2-00.permall", "firmware/OVMF_VARS.fd", "secrets/ssh_ed25519",
            "secrets/ssh_ed25519.pub", "secureboot/db.key", "secureboot/db.crt", "secureboot/db.guid"}
ALLOWED = REQUIRED | {"tpm/tpm2-00.savestate", "tpm/tpm2-00.volatilestate", "secrets/win_password"}
MAX_FILE = 16 * 1024 * 1024
MAX_MANIFEST = 96 * 1024 * 1024


class StateError(ValueError):
    pass


def require(condition, message):
    if not condition:
        raise StateError(message)


def fields(obj, required, optional=()):
    require(isinstance(obj, dict), "expected object")
    require(set(required) <= obj.keys() <= set(required) | set(optional), "missing or unknown fields")


def scalar(value, commas=False):
    pattern = r"[A-Za-z0-9_., :/+()-]+" if commas else r"[A-Za-z0-9_. :/+()-]+"
    require(isinstance(value, str) and 0 < len(value) <= 120 and
            re.fullmatch(pattern, value) is not None, "invalid QEMU property string")
    return value


def integer(value, low, high):
    require(type(value) is int and low <= value <= high, "invalid integer")


def digest(data):
    return hashlib.sha256(data).hexdigest()


def image_hash(path):
    require(Path(path).is_file() and not Path(path).is_symlink(), "image must be a regular file")
    h = hashlib.sha256()
    with open(path, "rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def utc(value):
    require(isinstance(value, str) and re.fullmatch(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ", value), "invalid UTC datetime")
    try:
        return datetime.strptime(value, "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=timezone.utc)
    except ValueError as error:
        raise StateError("invalid UTC datetime") from error


def decode(record):
    fields(record, {"bytes", "sha256", "data_base64"})
    integer(record["bytes"], 1, MAX_FILE)
    require(isinstance(record["data_base64"], str) and len(record["data_base64"]) <= MAX_FILE * 2,
            "invalid blob encoding")
    try:
        data = base64.b64decode(record["data_base64"], validate=True)
    except (ValueError, binascii.Error) as error:
        raise StateError("invalid blob encoding") from error
    require(len(data) == record["bytes"], "blob length mismatch")
    require(digest(data) == record["sha256"], "blob sha256 mismatch")
    return data


def verify(state, image=None):
    fields(state, {"schema_version", "classification", "created_utc", "image_sha256", "capture_window",
                   "rtc_base_utc", "time_service_disabled", "smbios", "nics", "disk", "machine",
                   "monitor", "cpu_observed", "files"}, {"cpu_pin"})
    require(type(state["schema_version"]) is int and state["schema_version"] == 1, "unsupported schema version")
    require(state["classification"] == CLASSIFICATION, "offline classification required")
    utc(state["created_utc"])
    require(isinstance(state["image_sha256"], str) and re.fullmatch(r"[0-9a-f]{64}", state["image_sha256"]),
            "invalid image sha256")
    fields(state["capture_window"], {"start_utc", "end_utc"})
    start = utc(state["capture_window"]["start_utc"])
    end = utc(state["capture_window"]["end_utc"])
    require(start <= utc(state["rtc_base_utc"]) <= end, "RTC outside capture window")
    require(state["time_service_disabled"] is True, "time service must be disabled")
    sm = state["smbios"]
    fields(sm, {"uuid", "serial", "manufacturer", "product"})
    try:
        require(str(uuid.UUID(sm["uuid"])) == sm["uuid"], "invalid SMBIOS UUID")
    except (ValueError, TypeError, AttributeError) as error:
        raise StateError("invalid SMBIOS UUID") from error
    for key in ("serial", "manufacturer", "product"):
        scalar(sm[key], commas=True)
    require(isinstance(state["nics"], list) and 1 <= len(state["nics"]) <= 8, "invalid NIC count")
    macs = []
    for nic in state["nics"]:
        fields(nic, {"mac"})
        mac = nic["mac"]
        require(isinstance(mac, str) and re.fullmatch(r"[0-9a-f]{2}(?::[0-9a-f]{2}){5}", mac), "invalid MAC")
        require(int(mac[:2], 16) & 1 == 0 and mac != "00:00:00:00:00:00", "invalid unicast MAC")
        macs.append(mac)
    require(len(set(macs)) == len(macs), "duplicate NIC MAC")
    fields(state["disk"], {"serial", "model"})
    scalar(state["disk"]["serial"])
    require(len(state["disk"]["serial"]) <= 20, "virtio disk serial exceeds 20 bytes")
    require(state["disk"]["model"] == "virtio-blk-pci", "unsupported disk device model")
    fields(state["machine"], {"type", "cores", "ram_mb"})
    require(isinstance(state["machine"]["type"], str) and
            re.fullmatch(r"pc-q35-\d+\.\d+", state["machine"]["type"]), "unsupported machine type")
    integer(state["machine"]["cores"], 1, 256)
    integer(state["machine"]["ram_mb"], 512, 1048576)
    fields(state["monitor"], {"edid_id"})
    require(state["monitor"]["edid_id"] == "KFB-0001-00000001", "unsupported fixed KF3 monitor ID")
    cpu = state["cpu_observed"]
    fields(cpu, {"brand", "family", "model", "stepping", "flags"})
    require(isinstance(cpu["brand"], str) and 0 < len(cpu["brand"]) <= 256, "invalid observed brand")
    for key in ("family", "model", "stepping"):
        integer(cpu[key], 0, 65535)
    require(isinstance(cpu["flags"], list) and len(cpu["flags"]) <= 1024, "invalid observed flags")
    require(all(isinstance(flag, str) and re.fullmatch(r"[a-zA-Z0-9_]+", flag) for flag in cpu["flags"]),
            "invalid observed flag")
    require(cpu["flags"] == sorted(set(cpu["flags"])), "observed flags must be unique and sorted")
    if "cpu_pin" in state:
        fields(state["cpu_pin"], {"brand"})
        scalar(state["cpu_pin"]["brand"])
        require(len(state["cpu_pin"]["brand"]) <= 48, "CPUID brand exceeds 48 bytes")
    require(isinstance(state["files"], dict) and REQUIRED <= state["files"].keys() <= ALLOWED,
            "missing or unrequested state file")
    require(sum(len(decode(record)) for record in state["files"].values()) <= MAX_MANIFEST // 2,
            "state blobs too large")
    if image is not None:
        require(image_hash(image) == state["image_sha256"], "image sha256 mismatch")
    return state


def no_duplicates(pairs):
    out = {}
    for key, value in pairs:
        require(key not in out, "duplicate JSON field")
        out[key] = value
    return out


def load_manifest(path):
    require(Path(path).stat().st_size <= MAX_MANIFEST, "manifest too large")
    try:
        return verify(json.loads(Path(path).read_text(), object_pairs_hook=no_duplicates))
    except (ValueError, UnicodeError) as error:
        raise StateError("invalid machine-state JSON") from error


def canonical(state):
    return json.dumps(state, sort_keys=True, separators=(",", ":")).encode()


def safe_path(root, relative):
    path = Path(root)
    require(not any(p.is_symlink() for p in (path, *path.parents)), "state root contains symlink")
    for part in Path(relative).parts:
        path /= part
        require(not path.is_symlink(), "state path contains symlink")
    return path


@contextmanager
def locked(vm, lock_fd=None):
    vm = Path(vm)
    require(not vm.is_symlink(), "VM directory must not be a symlink")
    run = safe_path(vm, "run")
    run.mkdir(parents=True, exist_ok=True)
    lock = safe_path(vm, "run/vm.lock")
    stream = None
    try:
        if lock_fd is None:
            stream = open(lock, "a+b")
            fd = stream.fileno()
        else:
            fd = lock_fd
            require(lock.is_file(), "VM lock not initialized")
            require(os.fstat(fd).st_ino == lock.stat().st_ino and
                    os.fstat(fd).st_dev == lock.stat().st_dev, "wrong inherited VM lock")
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        for name in ("qemu.pid", "swtpm.pid"):
            path = safe_path(vm, "run/" + name)
            if path.exists():
                pid = path.read_text().strip()
                require(not (pid.isdigit() and Path("/proc", pid).exists()), "VM or TPM service still running")
        yield
    except BlockingIOError as error:
        raise StateError("VM directory is locked") from error
    finally:
        if stream is not None:
            stream.close()


def capture(vm, image, facts, window_start, window_end, rtc_base, cpu_info,
            secureboot_dir=None, password_file=None):
    vm = Path(vm)
    fields(facts, {"smbios", "nics", "disk", "machine", "monitor"})
    with locked(vm):
        inventory = json.loads(safe_path(vm, "image-inventory.json").read_text())
        require(inventory.get("time_service_disabled") is True, "inventory must record disabled time service")
        observed = {}
        for line in Path(cpu_info).read_text().splitlines():
            key, sep, value = line.partition(":")
            if sep and key.strip() not in observed:
                observed[key.strip()] = value.strip()
        try:
            cpu = {"brand": observed["model name"], "family": int(observed["cpu family"]),
                   "model": int(observed["model"]), "stepping": int(observed["stepping"]),
                   "flags": sorted(set(observed["flags"].split()))}
        except (KeyError, ValueError) as error:
            raise StateError("CPU observation incomplete") from error
        state = dict(facts, schema_version=1, classification=CLASSIFICATION,
                     created_utc=datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
                     image_sha256=image_hash(image), capture_window={"start_utc": window_start, "end_utc": window_end},
                     rtc_base_utc=rtc_base, time_service_disabled=True, cpu_observed=cpu, files={})
        sb = Path(secureboot_dir) if secureboot_dir else (vm / "secureboot" if (vm / "secureboot").exists() else vm.parent / "secureboot")
        for name in sorted(ALLOWED):
            path = safe_path(sb, name.split("/", 1)[1]) if name.startswith("secureboot/") else safe_path(vm, name)
            if name == "secrets/win_password" and password_file:
                path = Path(password_file)
            if not path.exists() and name not in REQUIRED:
                continue
            require(path.is_file() and not path.is_symlink() and path.stat().st_size <= MAX_FILE,
                    "missing or invalid state file")
            data = path.read_bytes()
            state["files"][name] = {"bytes": len(data), "sha256": digest(data),
                                      "data_base64": base64.b64encode(data).decode("ascii")}
        return verify(state)


def restore_files(state, target):
    # Preflight the complete allowlist before replacing any state. Old optional TPM files
    # absent from the pristine manifest must also disappear on the next logical run.
    for name in ALLOWED:
        path = safe_path(target, name)
        require(not path.exists() or path.is_file(), "state destination is not a regular file")
    tpm = safe_path(target, "tpm")
    extras = list(tpm.iterdir()) if tpm.exists() else []
    require(all(p.is_file() and not p.is_symlink() for p in extras), "unexpected TPM directory entry")
    for path in extras:
        if "tpm/" + path.name not in state["files"]:
            path.unlink()
    for name in sorted(ALLOWED):
        path = safe_path(target, name)
        path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        if name in state["files"]:
            data = decode(state["files"][name])
            fd, temp = tempfile.mkstemp(prefix=".state-", dir=path.parent)
            try:
                with os.fdopen(fd, "wb") as stream:
                    stream.write(data)
                os.replace(temp, path)
            finally:
                if os.path.exists(temp):
                    os.unlink(temp)
        elif path.exists():
            path.unlink()


def apply(state, target, image, refresh=False, lock_fd=None):
    verify(state, image)
    target = Path(target)
    marker = digest(canonical(state))
    if refresh:
        require(target.is_dir() and not target.is_symlink(), "managed VM directory required")
        with locked(target, lock_fd):
            saved = safe_path(target, "machine-state.sha256")
            require(saved.is_file() and saved.read_text().strip() == marker, "VM is not managed by this state")
            require(image_hash(safe_path(target, "disk/base.qcow2")) == state["image_sha256"], "working base image changed")
            restore_files(state, target)
        return
    require(not target.exists() and not target.is_symlink(), "initial apply requires a new VM directory")
    target.parent.mkdir(parents=True, exist_ok=True)
    staging = Path(tempfile.mkdtemp(prefix=".machine-state-", dir=target.parent))
    try:
        for name in ("disk", "run", "logs/shots"):
            (staging / name).mkdir(parents=True, mode=0o700)
        shutil.copyfile(image, staging / "disk/base.qcow2")
        (staging / "disk/base.qcow2").chmod(0o400)
        restore_files(state, staging)
        (staging / "installed").touch(mode=0o600)
        (staging / "run/vm.lock").touch(mode=0o600)
        (staging / "machine-state.sha256").write_text(marker + "\n")
        os.rename(staging, target)
    finally:
        if staging.exists():
            shutil.rmtree(staging)


def qemu_env(state):
    verify(state)
    sm, machine = state["smbios"], state["machine"]
    escaped = {key: sm[key].replace(",", ",,") for key in ("serial", "manufacturer", "product")}
    values = {"MS_UUID": sm["uuid"], "MS_SERIAL": escaped["serial"], "MS_MANUFACTURER": escaped["manufacturer"],
              "MS_PRODUCT": escaped["product"], "MS_MAC": state["nics"][0]["mac"], "MS_MACHINE_TYPE": machine["type"],
              "MS_CORES": machine["cores"], "MS_RAM_MB": machine["ram_mb"], "MS_RTC_BASE": state["rtc_base_utc"][:-1],
              "MS_DISK_SERIAL": state["disk"]["serial"], "MS_CPU_BRAND": state.get("cpu_pin", {}).get("brand", "")}
    return "".join(f"{key}={value}\n" for key, value in values.items()) + "".join(
        f"MS_NIC={nic['mac']}\n" for nic in state["nics"][1:])


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="verb", required=True)
    cap = sub.add_parser("capture")
    cap.add_argument("--vm", type=Path, required=True)
    cap.add_argument("--image", type=Path, required=True)
    cap.add_argument("--facts", type=Path, required=True)
    cap.add_argument("--window-start", required=True)
    cap.add_argument("--window-end", required=True)
    cap.add_argument("--rtc-base", required=True)
    cap.add_argument("--cpu-info", type=Path, default=Path("/proc/cpuinfo"))
    cap.add_argument("--secureboot-dir", type=Path)
    cap.add_argument("--password-file", type=Path)
    cap.add_argument("--cpu-brand")
    cap.add_argument("--output", type=Path, required=True)
    for verb in ("verify", "apply", "qemu-env"):
        cmd = sub.add_parser(verb)
        cmd.add_argument("manifest", type=Path)
        if verb != "qemu-env":
            cmd.add_argument("--image", type=Path, required=verb == "apply")
        if verb == "apply":
            cmd.add_argument("--vm", type=Path, required=True)
            cmd.add_argument("--refresh", action="store_true")
            cmd.add_argument("--lock-fd", type=int)
    args = parser.parse_args(argv)
    try:
        if args.verb == "capture":
            require(not args.output.exists(), "output already exists")
            state = capture(args.vm, args.image, json.loads(args.facts.read_text(), object_pairs_hook=no_duplicates),
                            args.window_start, args.window_end, args.rtc_base, args.cpu_info,
                            args.secureboot_dir, args.password_file)
            if args.cpu_brand is not None:
                state["cpu_pin"] = {"brand": args.cpu_brand}
            verify(state)
            with open(args.output, "x", opener=lambda p, flags: os.open(p, flags, 0o600)) as stream:
                stream.write(json.dumps(state, indent=2) + "\n")
        else:
            state = load_manifest(args.manifest)
            if args.verb == "verify":
                verify(state, args.image)
            elif args.verb == "qemu-env":
                print(qemu_env(state), end="")
                return 0
            else:
                require(not args.manifest.resolve().is_relative_to(args.vm.resolve()), "pristine manifest must be outside working VM")
                apply(state, args.vm, args.image, args.refresh, args.lock_fd)
        print("machine-state: " + args.verb + " complete")
        return 0
    except (StateError, OSError, ValueError) as error:
        # No JSON contents, blob bytes, passwords or signing material in diagnostics.
        detail = str(error) if isinstance(error, StateError) else type(error).__name__
        print("machine-state: operation refused: " + detail, file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
