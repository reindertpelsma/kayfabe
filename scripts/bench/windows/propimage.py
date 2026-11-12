#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Proprietary Windows image: seed -> machine ids, QEMU argv per mode, verify, R2 zip package.

STATUS: DESIGN-ONLY, 2026-11-12 (GPU-free tooling; no VM was started by it). Design:
docs/design/V3_WINDOWS_PROPRIETARY_IMAGE.md. Logic lives here; shell is glue only.

  propimage.py ids   SEED                       print the ids derived from a seed string
  propimage.py init  SEED --out state.json      state skeleton (ids, net, clock rule); no secrets
  propimage.py argv  STATE MODE --disk D ...    print the exact qemu argv (+ swtpm argv, clone cmd)
  propimage.py verify-host STATE --tpm-dir D    part 1 of the first check: swtpm/OVMF files == state
  propimage.py verify STATE --ssh-key K         parts 1-3 over ssh (PowerShell sent on stdin)
  propimage.py pack  STATE IMAGE --out X.zip    one zip64 blob: image.qcow2 + state.json + manifest
  propimage.py unpack X.zip --dir D             extract and check every sha256

Modes: setup (std VGA, no kf3, internet on, real clock), install (kf3 + broker, internet on, real
clock), offline (kf3 + broker, slirp restrict=on, faked clock), ci (clone + offline + faked clock).
The same machine ids, disk serial, MAC, subnet and swtpm state apply in every mode.
"""
import argparse
import base64
import datetime as dt
import hashlib
import json
import re
import shlex
import subprocess
import sys
import zipfile
from pathlib import Path

DOMAIN = "kayfabe-propimage/v1"
MODES = ("setup", "install", "offline", "ci")
STATE_VERSION = 1


class PropError(ValueError):
    pass


def require(cond, msg):
    if not cond:
        raise PropError(msg)


# ---------------------------------------------------------------- seed -> ids
def _h(seed, field):
    require(isinstance(seed, str) and 4 <= len(seed) <= 128 and seed.isprintable(),
            "seed must be a printable string of 4..128 characters")
    return hashlib.sha256(f"{DOMAIN}\0{seed}\0{field}".encode()).digest()


def _uuid(raw):
    b = bytearray(raw[:16])
    b[6] = (b[6] & 0x0F) | 0x50          # version 5 style, RFC 4122 variant
    b[8] = (b[8] & 0x3F) | 0x80
    x = bytes(b).hex()
    return f"{x[:8]}-{x[8:12]}-{x[12:16]}-{x[16:20]}-{x[20:]}"


def derive_ids(seed, nics=1, disks=1):
    """Pure function: the same seed always gives the same ids; formats are valid for QEMU/SMBIOS."""
    require(1 <= nics <= 8 and 1 <= disks <= 8, "nics and disks must be 1..8")
    net_b = _h(seed, "net")
    gpu = _h(seed, "gpu-uuid")
    gx = gpu[:16].hex()
    return {
        "smbios": {
            "uuid": _uuid(_h(seed, "smbios-uuid")),
            "manufacturer": "kayfabe",
            "product": "propimage",
            "serial": "KF" + _h(seed, "smbios-serial").hex()[:10].upper(),
            "baseboard_serial": "KFB" + _h(seed, "baseboard-serial").hex()[:10].upper(),
            "chassis_serial": "KFC" + _h(seed, "chassis-serial").hex()[:10].upper(),
        },
        "nics": [{"mac": "52:54:00:" + ":".join("%02x" % c for c in _h(seed, f"mac/{i}")[:3])}
                 for i in range(nics)],
        "disks": [{"serial": "KFD" + _h(seed, f"disk/{i}").hex()[:12].upper(), "model": "virtio-blk-pci"}
                  for i in range(disks)],
        "gpu_uuid": "GPU-" + "-".join((gx[:8], gx[8:12], gx[12:16], gx[16:20], gx[20:32])),
        # a fixed slirp /24 inside 10.16.0.0/12; never 10.0.2.0/24 (the QEMU default)
        "net": {"subnet": f"10.{16 + net_b[0] % 16}.{net_b[1]}.0/24",
                "ssh_port": 20000 + int.from_bytes(net_b[2:4], "big") % 20000},
        # offline/ci boot at last_time_at_stop + gap (never before it: see the design, clock anomaly)
        "clock_gap_s": 600 + int.from_bytes(_h(seed, "clock-gap")[:2], "big") % 3000,
    }


# ---------------------------------------------------------------- state
def utc(text):
    require(isinstance(text, str) and re.fullmatch(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ", text), "bad UTC time: %r" % (text,))
    return dt.datetime.strptime(text, "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=dt.timezone.utc)


def fmt(t):
    return t.astimezone(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def init_state(seed, nics=1, disks=1):
    ids = derive_ids(seed, nics, disks)
    return {
        "state_version": STATE_VERSION, "seed": seed, "ids": ids,
        "clock": {"last_time_at_stop_utc": None, "gap_s": ids["clock_gap_s"]},
        "qemu": {"machine": "pc-q35-10.2", "smp": 8, "ram_mb": 8192, "fb_mb": 4096},
        "files": {},   # name -> {bytes, sha256, data_base64}: tpm/*, firmware/OVMF_VARS.fd
        "guest": {"user": "kfimage", "machine_guid": None, "ek_pub_hash": None, "ssh_private_key": None,
                  "ssh_public_key": None},
        "software": [], "offline_checks": {},
    }


def validate_state(state):
    require(state.get("state_version") == STATE_VERSION, "unknown state_version")
    ids = state["ids"]
    require(re.fullmatch(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}", ids["smbios"]["uuid"]), "bad SMBIOS uuid")
    for n in ids["nics"]:
        require(re.fullmatch(r"[0-9a-f]{2}(:[0-9a-f]{2}){5}", n["mac"]) and int(n["mac"][:2], 16) & 1 == 0, "bad MAC")
    for d in ids["disks"]:
        require(re.fullmatch(r"[A-Z0-9]{1,20}", d["serial"]), "bad disk serial (virtio limit 20)")
    require(re.fullmatch(r"GPU-[0-9a-f]{8}(-[0-9a-f]{4}){3}-[0-9a-f]{12}", ids["gpu_uuid"]), "bad GPU uuid")
    require(re.fullmatch(r"10\.\d+\.\d+\.0/24", ids["net"]["subnet"]), "slirp subnet must be a 10.x.y.0/24")
    require(1024 <= ids["net"]["ssh_port"] <= 65535, "bad ssh port")
    for name, rec in state["files"].items():
        data = base64.b64decode(rec["data_base64"], validate=True)
        require(len(data) == rec["bytes"] and hashlib.sha256(data).hexdigest() == rec["sha256"], "blob mismatch: " + name)
        require(not name.startswith("/") and ".." not in Path(name).parts, "bad blob path " + name)
    return True


def seed_matches(state):
    """The state's ids must be exactly what the seed derives (no hand-edited identity)."""
    want = derive_ids(state["seed"], len(state["ids"]["nics"]), len(state["ids"]["disks"]))
    return want == state["ids"]


def boot_clock(state, mode):
    """RTC base for a mode, or None = the host's real clock (internet modes need valid TLS dates)."""
    if mode in ("setup", "install"):
        return None
    last = state["clock"]["last_time_at_stop_utc"]
    require(last, "offline/ci need clock.last_time_at_stop_utc (record it at the first clean stop)")
    return utc(last) + dt.timedelta(seconds=state["clock"]["gap_s"])


# ---------------------------------------------------------------- argv
def _comma(v):
    return str(v).replace(",", ",,")


def slirp_net(subnet):
    return subnet


def build_argv(state, mode, disk, workdir, ovmf_code="/usr/share/OVMF/OVMF_CODE_4M.secboot.fd",
               qemu=None, uid=1000, broker_sock=None, gop_efi=None, display="broker"):
    """The exact qemu argv. disk: the image path (install/offline) or the clone (ci)."""
    require(mode in MODES, "mode must be one of %s" % (MODES,))
    require(display in ("broker", "off"), "display must be broker or off")
    ids, q = state["ids"], state["qemu"]
    wd = str(workdir)
    qemu = qemu or ("/usr/bin/qemu-system-x86_64" if mode == "setup" else "qemu-system-x86_64")
    base = boot_clock(state, mode)
    restrict = "off" if mode in ("setup", "install") else "on"
    rtc = "base=utc" if base is None else "base=%s,clock=vm" % base.strftime("%Y-%m-%dT%H:%M:%S")
    sm = ids["smbios"]
    a = [qemu, "-name", "propimage-" + mode,
         "-machine", q["machine"] + ",accel=kvm,smm=on", "-m", str(q["ram_mb"]),
         "-smp", "%d,sockets=1,cores=%d,threads=1" % (q["smp"], q["smp"]), "-cpu", "host,-vmx",
         "-uuid", sm["uuid"],
         "-smbios", "type=1,manufacturer=%s,product=%s,serial=%s" % (_comma(sm["manufacturer"]), _comma(sm["product"]), sm["serial"]),
         "-smbios", "type=2,manufacturer=%s,product=%s,serial=%s" % (_comma(sm["manufacturer"]), _comma(sm["product"]), sm["baseboard_serial"]),
         "-smbios", "type=3,manufacturer=%s,serial=%s" % (_comma(sm["manufacturer"]), sm["chassis_serial"]),
         "-global", "driver=cfi.pflash01,property=secure,value=on",
         "-drive", "if=pflash,format=raw,unit=0,readonly=on,file=" + ovmf_code,
         "-drive", "if=pflash,format=raw,unit=1,file=%s/OVMF_VARS.fd" % wd,
         "-rtc", rtc + ",driftfix=slew",
         "-chardev", "socket,id=chrtpm,path=%s/swtpm.sock" % wd, "-tpmdev", "emulator,id=tpm0,chardev=chrtpm",
         "-device", "tpm-crb,tpmdev=tpm0",
         "-drive", "if=none,id=sys0,format=qcow2,cache=none,aio=threads,file=" + str(disk),
         "-device", "virtio-blk-pci,drive=sys0,serial=" + ids["disks"][0]["serial"],
         "-netdev", "user,id=n0,net=%s,ipv6=off,restrict=%s,hostfwd=tcp:127.0.0.1:%d-:22"
         % (ids["net"]["subnet"], restrict, ids["net"]["ssh_port"]),
         "-device", "virtio-net-pci,netdev=n0,mac=" + ids["nics"][0]["mac"],
         "-object", "rng-random,id=rng0,filename=/dev/urandom", "-device", "virtio-rng-pci,rng=rng0",
         "-qmp", "unix:%s/qmp.sock,server=on,wait=off" % wd, "-msg", "timestamp=on"]
    if mode == "setup":
        # Windows setup first on std VGA, as the gate does; kayfabe's QEMU has no GTK front end, so the
        # distro QEMU (gtk, measured on the PC) shows it on the physical desktop.
        a += ["-vga", "std", "-display", "gtk", "-device", "qemu-xhci,id=xhci", "-device", "usb-tablet,bus=xhci.0",
              "-device", "usb-kbd,bus=xhci.0"]
    else:
        kf3 = ["kf3-gpu", "id=kf0", "fb-mb=%d" % q["fb_mb"], "bar1-size=134217728", "bar2-size=33554432",
               "gpu-uuid=" + ids["gpu_uuid"]]
        a += ["-vga", "none"]
        if display == "broker":
            require(broker_sock and gop_efi, "broker display needs broker_sock and gop_efi")
            kf3 += ["display=on", "gop=on", "gop-efi=" + str(gop_efi), "display-broker=" + str(broker_sock),
                    "display-broker-uid=%d" % uid]
        a += ["-device", ",".join(kf3), "-display", "none",
              "-device", "qemu-xhci,id=xhci", "-device", "usb-tablet,bus=xhci.0", "-device", "usb-kbd,bus=xhci.0"]
    return a


def swtpm_argv(workdir):
    wd = str(workdir)
    return ["swtpm", "socket", "--tpm2", "--tpmstate", "dir=%s/tpm,mode=0600" % wd,
            "--ctrl", "type=unixio,path=%s/swtpm.sock,mode=0600" % wd, "--terminate", "--daemon"]


def clone_cmd(image, clone):
    return ["qemu-img", "create", "-f", "qcow2", "-b", str(image), "-F", "qcow2", str(clone)]


# ---------------------------------------------------------------- verify
GUEST_PS = r"""
$ErrorActionPreference='SilentlyContinue'
$o=[ordered]@{}
$o.machine_guid=(Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Cryptography').MachineGuid
$o.smbios_uuid=(Get-CimInstance Win32_ComputerSystemProduct).UUID
$o.smbios_serial=(Get-CimInstance Win32_BIOS).SerialNumber
$o.baseboard_serial=(Get-CimInstance Win32_BaseBoard).SerialNumber
$o.chassis_serial=(Get-CimInstance Win32_SystemEnclosure).SerialNumber
$o.macs=@(Get-NetAdapter -Physical | ForEach-Object { $_.MacAddress.ToLower().Replace('-',':') })
$o.disk_serials=@(Get-CimInstance Win32_DiskDrive | ForEach-Object { $_.SerialNumber.Trim() })
$o.gpu_uuid=((& nvidia-smi --query-gpu=uuid --format=csv,noheader) | Select-Object -First 1)
$o.date_utc=(Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
$t=Get-Tpm; $o.tpm_present=[bool]$t.TpmPresent; $o.tpm_ready=[bool]$t.TpmReady
$ek=Get-TpmEndorsementKeyInfo; $o.ek_pub_hash=$ek.PublicKeyHash
$o | ConvertTo-Json -Compress
"""


def compare(state, guest, max_clock_age_s=1800):
    """-> list of (item, 'MATCH'|'MISMATCH', detail). Pure; the ssh call is separate."""
    ids = state["ids"]
    out = []

    def add(name, want, got):
        out.append((name, "MATCH" if want == got and want is not None else "MISMATCH", "want=%r got=%r" % (want, got)))

    sm = ids["smbios"]
    add("smbios_uuid", sm["uuid"], (guest.get("smbios_uuid") or "").lower())
    add("smbios_serial", sm["serial"], guest.get("smbios_serial"))
    add("baseboard_serial", sm["baseboard_serial"], guest.get("baseboard_serial"))
    add("chassis_serial", sm["chassis_serial"], guest.get("chassis_serial"))
    add("mac", sorted(n["mac"] for n in ids["nics"]), sorted(guest.get("macs") or []))
    add("disk_serial", sorted(d["serial"] for d in ids["disks"]), sorted(guest.get("disk_serials") or []))
    add("gpu_uuid", ids["gpu_uuid"], guest.get("gpu_uuid"))
    add("machine_guid", state["guest"]["machine_guid"], guest.get("machine_guid"))
    # clock: the guest must be at or after the recorded stop time (never before: floor behaviour) and
    # within max_clock_age_s of the boot base. Real-clock modes are not checked here.
    last = state["clock"]["last_time_at_stop_utc"]
    if last:
        base = utc(last) + dt.timedelta(seconds=state["clock"]["gap_s"])
        try:
            g = utc(guest.get("date_utc"))
            ok = base <= g <= base + dt.timedelta(seconds=max_clock_age_s)
            out.append(("clock", "MATCH" if ok else "MISMATCH", "base=%s guest=%s window=%ds" % (fmt(base), guest.get("date_utc"), max_clock_age_s)))
        except PropError as e:
            out.append(("clock", "MISMATCH", str(e)))
    else:
        out.append(("clock", "MISMATCH", "state has no last_time_at_stop_utc"))
    out.append(("tpm_present", "MATCH" if guest.get("tpm_present") else "MISMATCH", repr(guest.get("tpm_present"))))
    add("tpm_ek_pub_hash", state["guest"]["ek_pub_hash"], guest.get("ek_pub_hash"))
    return out


def verify_host(state, tpm_dir):
    """Part 1 of the first check, before the guest runs: restored swtpm/OVMF files equal the state."""
    out = []
    root = Path(tpm_dir)
    for name, rec in sorted(state["files"].items()):
        p = root / name
        got = hashlib.sha256(p.read_bytes()).hexdigest() if p.is_file() and not p.is_symlink() else None
        out.append(("file:" + name, "MATCH" if got == rec["sha256"] else "MISMATCH", "want=%s got=%s" % (rec["sha256"][:12], (got or "missing")[:12])))
    if not state["files"]:
        out.append(("files", "MISMATCH", "state embeds no swtpm/OVMF files"))
    return out


def ssh_argv(state, key, host="127.0.0.1"):
    return ["ssh", "-i", str(key), "-p", str(state["ids"]["net"]["ssh_port"]), "-o", "IdentitiesOnly=yes",
            "-o", "StrictHostKeyChecking=no", "-o", "UserKnownHostsFile=/dev/null", "-o", "BatchMode=yes",
            "-o", "ConnectTimeout=10", "%s@%s" % (state["guest"]["user"], host),
            "powershell", "-NoProfile", "-NonInteractive", "-Command", "-"]


def run_guest_probe(state, key, runner=subprocess.run):
    p = runner(ssh_argv(state, key), input=GUEST_PS, text=True, capture_output=True, timeout=120)
    require(p.returncode == 0, "ssh probe failed: " + (p.stderr or "")[-300:])
    line = [l for l in p.stdout.splitlines() if l.strip().startswith("{")]
    require(line, "no JSON from the guest probe")
    return json.loads(line[-1])


# ---------------------------------------------------------------- package
def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def pack(state_path, image, out):
    """One zip64 blob, members STORED (the qcow2 is already `qemu-img convert -c` compressed)."""
    state = json.loads(Path(state_path).read_text())
    validate_state(state)
    require(seed_matches(state), "state ids do not match its seed")
    members = {"image.qcow2": sha256_file(image), "state.json": hashlib.sha256(Path(state_path).read_bytes()).hexdigest()}
    manifest = {"manifest_version": 1, "members": members}
    with zipfile.ZipFile(out, "w", compression=zipfile.ZIP_STORED, allowZip64=True) as z:
        z.write(image, "image.qcow2")
        z.write(state_path, "state.json")
        z.writestr("manifest.json", json.dumps(manifest, sort_keys=True, indent=1))
    return manifest


def unpack(zpath, dest):
    dest = Path(dest)
    with zipfile.ZipFile(zpath) as z:
        require(sorted(z.namelist()) == ["image.qcow2", "manifest.json", "state.json"], "unexpected zip members")
        require(all(i.compress_type == zipfile.ZIP_STORED for i in z.infolist()), "members must be STORED")
        manifest = json.loads(z.read("manifest.json"))
        dest.mkdir(parents=True, exist_ok=True)
        for name, want in manifest["members"].items():
            z.extract(name, dest)
            require(sha256_file(dest / name) == want, "sha256 mismatch for " + name)
    state = json.loads((dest / "state.json").read_text())
    validate_state(state)
    require(seed_matches(state), "state ids do not match its seed")
    return state


# ---------------------------------------------------------------- cli
def _print_rows(rows):
    for item, verdict, detail in rows:
        print("%-9s %-18s %s" % (verdict, item, detail))
    bad = [r for r in rows if r[1] != "MATCH"]
    print("RESULT %s" % ("MATCH" if not bad else "MISMATCH (%d)" % len(bad)))
    return 0 if not bad else 1


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="verb", required=True)
    p = sub.add_parser("ids"); p.add_argument("seed")
    p = sub.add_parser("init"); p.add_argument("seed"); p.add_argument("--out", required=True)
    p = sub.add_parser("argv"); p.add_argument("state"); p.add_argument("mode", choices=MODES)
    p.add_argument("--disk", required=True); p.add_argument("--workdir", required=True)
    p.add_argument("--broker-sock"); p.add_argument("--gop-efi"); p.add_argument("--uid", type=int, default=1000)
    p.add_argument("--display", default="broker"); p.add_argument("--qemu")
    p = sub.add_parser("verify-host"); p.add_argument("state"); p.add_argument("--tpm-dir", required=True)
    p = sub.add_parser("verify"); p.add_argument("state"); p.add_argument("--ssh-key", required=True)
    p.add_argument("--tpm-dir")
    p = sub.add_parser("pack"); p.add_argument("state"); p.add_argument("image"); p.add_argument("--out", required=True)
    p = sub.add_parser("unpack"); p.add_argument("zip"); p.add_argument("--dir", required=True)
    a = ap.parse_args(argv)
    try:
        if a.verb == "ids":
            print(json.dumps(derive_ids(a.seed), indent=2))
        elif a.verb == "init":
            Path(a.out).write_text(json.dumps(init_state(a.seed), indent=2) + "\n")
        elif a.verb in ("argv", "verify-host", "verify"):
            state = json.loads(Path(a.state).read_text())
            validate_state(state)
            require(seed_matches(state), "state ids do not match its seed")
            if a.verb == "argv":
                print(shlex.join(build_argv(state, a.mode, a.disk, a.workdir, qemu=a.qemu, uid=a.uid,
                                            broker_sock=a.broker_sock, gop_efi=a.gop_efi, display=a.display)))
                print("# swtpm: " + shlex.join(swtpm_argv(a.workdir)))
                print("# clone: " + shlex.join(clone_cmd(a.disk, str(a.disk) + ".clone")))
            elif a.verb == "verify-host":
                return _print_rows(verify_host(state, a.tpm_dir))
            else:
                rows = verify_host(state, a.tpm_dir) if a.tpm_dir else []
                return _print_rows(rows + compare(state, run_guest_probe(state, a.ssh_key)))
        elif a.verb == "pack":
            print(json.dumps(pack(a.state, a.image, a.out)))
        else:
            unpack(a.zip, a.dir)
            print("unpacked and verified")
    except (PropError, OSError, KeyError, json.JSONDecodeError) as e:
        print("propimage: " + str(e), file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
