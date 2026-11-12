#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Proprietary Windows image: seed -> machine ids, QEMU argv per mode, verify, R2 zip package.

STATUS: DESIGN-ONLY, 2026-11-12 (GPU-free tooling; no VM was started by it). Design:
docs/design/V3_WINDOWS_PROPRIETARY_IMAGE.md. Logic lives here; shell is glue only.

  propimage.py ids SEED | password STATE        derived ids, derived throwaway account password
  propimage.py init SEED --out STATE [--ssh-key-dir D]   state skeleton (ids, net, CPU profile, clock rule)
  propimage.py tools STATE --msi M --nvidia N --out D    the ONE install CD directory (KFTOOLS)
  propimage.py argv STATE MODE --disk D --workdir W ...  the exact qemu argv (+ swtpm, clone); refuses with
                                                CPU_PROFILE_UNSUPPORTED when the host cannot run the CPU profile
  propimage.py verify STATE [--workdir W] [--ssh-key K] [--record]   the first check: MATCH/MISMATCH/WARN per item
  propimage.py pack STATE IMAGE --out ZIP [--workdir W] | unpack ZIP --dir D [--workdir W]   one zip64 blob

Modes: setup (std VGA, no kf3, internet on, real clock), install (kf3 + broker, internet on, real
clock), offline (kf3 + broker, slirp restrict=on, faked clock), ci (clone + offline + faked clock).
The same machine ids, disk serial, MAC, subnet and swtpm state apply in every mode.
"""
import argparse
import base64
import copy
import datetime as dt
import hashlib
import json
import re
import shlex
import shutil
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



# ---------------------------------------------------------------- CPU profile
# Image-mode-only exception (2026-11-12 owner ruling) to the "CPU overrides rejected" rule of PR #302, which
# applies to win_vm.sh's DEFAULT argv and is untouched: every propimage mode uses ONE named CPU model, never
# `-cpu host`. Base model, defaults and property names were read from `qemu-system-x86_64 -cpu help` and
# `query-cpu-model-expansion` (type full) of QEMU 10.2.1: EPYC-v1 = AuthenticAMD family 23 model 1 stepping 2, with
# AVX2, BMI1/2, FMA, F16C, MOVBE, SSE4.2, AES, PCLMUL, RDRAND, POPCNT, XSAVE and no AVX-512, RTM, HLE, MPX or SGX.
# Zen 1 is the oldest supported host, so every later Zen host should support it (INFERRED; hardware validation below).
CPU_PROFILE_NAME = "kayfabe-amd-v1"
CPU_PROFILE = {
    "name": CPU_PROFILE_NAME, "base_model": "EPYC-v1", "vendor": "AuthenticAMD",
    "model_id": "AMD Virtual CPU", "family": 23, "model": 1, "stepping": 2,
    # explicit removals: redundant with the named model (QEMU exposes only the model's features) and kept so the
    # intent is visible in the argv; -vmx is the gate's setting; -svm is a decision (open item O3 in the design doc)
    "remove": ["avx512f", "rtm", "hle", "mpx", "sgx", "vmx", "svm"],
    "hv_flags": ["hv-relaxed", "hv-vapic", "hv-spinlocks=0x1fff", "hv-time", "hv-vpindex", "hv-synic", "hv-stimer", "hv-crash"],
    # host /proc/cpuinfo flag names the host must show (QEMU name to kernel name mapping INFERRED)
    "required_host_flags": ["3dnowprefetch", "abm", "adx", "aes", "avx", "avx2", "bmi1", "bmi2", "clflushopt", "clzero",
                            "cx16", "f16c", "fma", "fsgsbase", "lahf_lm", "misalignsse", "movbe", "pclmulqdq", "popcnt",
                            "rdrand", "rdseed", "rdtscp", "sha_ni", "smap", "smep", "sse4_1", "sse4_2", "sse4a",
                            "ssse3", "xsave", "xsavec", "xsaveopt"],
}
CPU_REFUSAL = "CPU_PROFILE_UNSUPPORTED"
FORBIDDEN_GUEST_FLAGS = ("avx512f", "rtm", "hle", "mpx", "sgx")


def cpu_arg(state):
    c = state["cpu"]
    return ",".join([c["base_model"], "model-id=" + c["model_id"], "family=%d" % c["family"], "model=%d" % c["model"],
                     "stepping=%d" % c["stepping"]] + ["-" + f for f in c["remove"]] + c["hv_flags"])


def parse_cpuinfo(text):
    """First processor record of /proc/cpuinfo: vendor_id and the flag set."""
    vendor, flags = None, set()
    for line in text.splitlines():
        if not line.strip() and (vendor or flags):
            break
        k, _, v = line.partition(":")
        k, v = k.strip(), v.strip()
        if k == "vendor_id" and vendor is None:
            vendor = v
        elif k == "flags" and not flags:
            flags = set(v.split())
    return vendor, flags


def cpu_preflight(state, cpuinfo_text):
    """Host-side check that KVM can run the profile. -> rows; any MISMATCH row means CPU_PROFILE_UNSUPPORTED."""
    c = state["cpu"]
    vendor, flags = parse_cpuinfo(cpuinfo_text)
    rows = [("cpu_vendor", "MATCH" if vendor == c["vendor"] else "MISMATCH",
             "profile %s needs %s, host is %s (no vendor spoofing; Intel needs its own image)" % (c["name"], c["vendor"], vendor))]
    missing = sorted(set(c["required_host_flags"]) - flags)
    rows.append(("cpu_features", "MATCH" if not missing and flags else "MISMATCH",
                 "missing host flags: %s" % (", ".join(missing) if missing else ("none" if flags else "no flags line"))))
    return rows


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
        "qemu": {"machine": "pc-q35-10.2", "smp": 8, "ram_mb": 8192, "fb_mb": 4096},   # smp fixed: never the host's
        "cpu": copy.deepcopy(CPU_PROFILE), "cpu_fingerprint": None,
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
    c = state["cpu"]
    require(c["name"] == CPU_PROFILE_NAME and c["base_model"] == "EPYC-v1", "unknown CPU profile")
    require(c["vendor"] == "AuthenticAMD", "the image CPU vendor is AuthenticAMD (no spoofing)")
    require(re.fullmatch(r"[A-Za-z0-9 ._()-]{1,48}", c["model_id"]), "bad CPU brand string")
    require(all(type(c[k]) is int and 0 <= c[k] < 256 for k in ("family", "model", "stepping")), "bad CPU family/model/stepping")
    require(all(re.fullmatch(r"[a-z0-9_.]+", f) for f in c["remove"]), "bad CPU removal flag")
    require(all(re.fullmatch(r"hv-[a-z-]+(=0x[0-9a-f]+)?", f) for f in c["hv_flags"]), "bad Hyper-V flag")
    require(not set(FORBIDDEN_GUEST_FLAGS) - set(c["remove"]), "profile must remove avx512f, rtm, hle, mpx, sgx")
    require(1 <= state["qemu"]["smp"] <= 64, "bad smp")
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
               qemu=None, uid=1000, broker_sock=None, gop_efi=None, display="broker", cdroms=()):
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
         "-smp", "%d,sockets=1,cores=%d,threads=1" % (q["smp"], q["smp"]), "-cpu", cpu_arg(state),
         "-uuid", sm["uuid"],
         "-smbios", "type=1,manufacturer=%s,product=%s,serial=%s" % (_comma(sm["manufacturer"]), _comma(sm["product"]), sm["serial"]),
         "-smbios", "type=2,manufacturer=%s,product=%s,serial=%s" % (_comma(sm["manufacturer"]), _comma(sm["product"]), sm["baseboard_serial"]),
         "-smbios", "type=3,manufacturer=%s,serial=%s" % (_comma(sm["manufacturer"]), sm["chassis_serial"]),
         "-global", "driver=cfi.pflash01,property=secure,value=on",
         "-global", "ICH9-LPC.disable_s3=1", "-global", "ICH9-LPC.disable_s4=1",
         "-drive", "if=pflash,format=raw,unit=0,readonly=on,file=" + ovmf_code,
         "-drive", "if=pflash,format=raw,unit=1,file=%s/firmware/OVMF_VARS.fd" % wd,
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
    require(len(cdroms) <= 6, "the q35 AHCI controller has 6 ports")
    for i, iso in enumerate(cdroms):
        # read-only install media on the q35 AHCI bus (Windows ISO first: it boots, then the disk)
        a += ["-drive", "if=none,id=cd%d,format=raw,media=cdrom,readonly=on,file=%s" % (i, iso),
              "-device", "ide-cd,drive=cd%d,bus=ide.%d%s" % (i, i, ",bootindex=1" if i == 0 else "")]   # q35 AHCI: one unit per port
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
$pr=Get-CimInstance Win32_Processor | Select-Object -First 1
$o.cpu_name=$pr.Name; $o.cpu_manufacturer=$pr.Manufacturer; $o.cpu_family=$pr.Family; $o.cpu_description=$pr.Description
$o.cpu_processor_id=$pr.ProcessorId; $o.cpu_cores=[int]$pr.NumberOfCores; $o.cpu_logical=[int]$pr.NumberOfLogicalProcessors
Add-Type -Namespace K -Name P -MemberDefinition '[DllImport("kernel32.dll")] public static extern bool IsProcessorFeaturePresent(uint f);'
$pf=@{ssse3=36;sse4_1=37;sse4_2=38;avx=39;avx2=40;avx512f=41;xsave=17;cx16=14;rdtscp=32}
$o.cpu_flags=@($pf.Keys | Where-Object { [K.P]::IsProcessorFeaturePresent($pf[$_]) } | Sort-Object)
$ek=Get-TpmEndorsementKeyInfo; $o.ek_pub_hash=$ek.PublicKeyHash
$o | ConvertTo-Json -Compress
"""


def cpu_fingerprint(guest):
    """Guest-visible CPU identity: what is recorded at backup time and compared on every run."""
    flags = sorted(guest.get("cpu_flags") or [])
    return {"vendor": guest.get("cpu_manufacturer"), "brand": guest.get("cpu_name"), "family": guest.get("cpu_family"),
            "description": guest.get("cpu_description"), "processor_id": guest.get("cpu_processor_id"),
            "cores": guest.get("cpu_cores"), "logical": guest.get("cpu_logical"), "flags": flags,
            "flags_sha256": hashlib.sha256("\n".join(flags).encode()).hexdigest()}


def fill_from_guest(state, guest):
    """Record the non-derivable facts once, from the guest, before the backup."""
    state["guest"]["machine_guid"] = guest.get("machine_guid")
    state["guest"]["ek_pub_hash"] = guest.get("ek_pub_hash")
    state["clock"]["last_time_at_stop_utc"] = guest.get("date_utc")
    state["cpu_fingerprint"] = cpu_fingerprint(guest)
    return state


def compare(state, guest, max_clock_age_s=1800):
    """-> list of (item, 'MATCH'|'MISMATCH', detail). Pure; the ssh call is separate."""
    ids = state["ids"]
    out = []

    def add(name, want, got, blocking=True):
        ok = want == got and want is not None
        out.append((name, "MATCH" if ok else ("MISMATCH" if blocking else "WARN"), "want=%r got=%r" % (want, got)))

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
    # CPU fingerprint: vendor and family/model/stepping block; brand, ProcessorId, counts and flags are
    # informational (WARN): the real strictness test is the cross-host restore with the apps started offline.
    want = state.get("cpu_fingerprint")
    got = cpu_fingerprint(guest)
    for key, item, blocking in (("vendor", "cpu_vendor", True), ("description", "cpu_family_model_stepping", True),
                                ("brand", "cpu_brand", False), ("processor_id", "cpu_processor_id", False),
                                ("cores", "cpu_cores", False), ("logical", "cpu_logical", False),
                                ("flags_sha256", "cpu_flags_sha256", False)):
        add(item, want[key] if want else None, got[key], blocking)
    bad = sorted(set(got["flags"]) & set(FORBIDDEN_GUEST_FLAGS))
    known = guest.get("cpu_flags") is not None
    out.append(("cpu_no_forbidden_flags", "WARN" if bad or not known else "MATCH",
                "guest shows: %s" % (bad or ("none" if known else "no flag list reported"))))
    out.append(("tpm_present", "MATCH" if guest.get("tpm_present") else "MISMATCH", repr(guest.get("tpm_present"))))
    add("tpm_ek_pub_hash", state["guest"]["ek_pub_hash"], guest.get("ek_pub_hash"))
    return out


def verify_host(state, tpm_dir, cpuinfo_text=None):
    """Part 1 of the first check, before the guest runs: restored swtpm/OVMF files equal the state."""
    out = []
    root = Path(tpm_dir)
    for name, rec in sorted(state["files"].items()):
        p = root / name
        got = hashlib.sha256(p.read_bytes()).hexdigest() if p.is_file() and not p.is_symlink() else None
        out.append(("file:" + name, "MATCH" if got == rec["sha256"] else "MISMATCH", "want=%s got=%s" % (rec["sha256"][:12], (got or "missing")[:12])))
    if not state["files"]:
        out.append(("files", "MISMATCH", "state embeds no swtpm/OVMF files"))
    if cpuinfo_text is not None:
        out += cpu_preflight(state, cpuinfo_text)
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


# ---------------------------------------------------------------- unattended setup media
UNATTEND_TEMPLATE = Path(__file__).with_name("propimage_autounattend.xml.in")
OPENSSH_MSI = "OpenSSH-Win64-v10.0.0.0.msi"


def derive_password(seed):
    """Throwaway local-account password, derived from the seed (the image is a local test VM)."""
    return "Kf-" + _h(seed, "guest-password").hex()[:16]


def render_unattend(template, state, computer="KFPROP"):
    """autounattend.xml for Windows 11 Pro: offline local account, virtio drivers from the virtio-win ISO,
    OpenSSH from the pinned MSI. Deterministic: the same state gives the same bytes."""
    user = state["guest"]["user"]
    require(re.fullmatch(r"[A-Za-z][A-Za-z0-9_-]{0,19}", user), "bad guest user name")
    require(re.fullmatch(r"[A-Z][A-Z0-9-]{0,14}", computer), "bad computer name")
    letters = "DEFGHIJK"
    winpe = "\n".join('    <PathAndCredentials wcm:action="add" wcm:keyValue="%d"><Path>%s:\\viostor\\w11\\amd64</Path></PathAndCredentials>' % (k + 1, L)
                      for k, L in enumerate(letters))
    offline, k = [], 1
    for drv in ("NetKVM", "vioserial", "viorng", "pvpanic", "fwcfg", "smbus"):
        for L in letters:
            offline.append('    <PathAndCredentials wcm:action="add" wcm:keyValue="%d"><Path>%s:\\%s\\w11\\amd64</Path></PathAndCredentials>' % (k, L, drv))
            k += 1
    out = template
    for key, val in (("@@WINPE_DRIVER_PATHS@@", winpe), ("@@OFFLINE_DRIVER_PATHS@@", "\n".join(offline)), ("@@USER@@", user),
                     ("@@COMPUTERNAME@@", computer), ("@@PASSWORD@@", derive_password(state["seed"]))):
        require(key in out, "template lacks " + key)
        out = out.replace(key, val)
    require("@@" not in out, "unfilled placeholder in the template")
    return out



# ---------------------------------------------------------------- tools CD (read-only second CD-ROM)
NVIDIA_EXE = "580.88-desktop-win10-win11-64bit-international-dch-whql.exe"
PINS = {  # sha256 pins copied from win_vm.sh's MEDIA list (CODE READ)
    OPENSSH_MSI: "ddec9c53864280759cf9f74791cefd387100e3946aa849a1c138a4ed1b96b7d9",
    NVIDIA_EXE: "90c49f925b41ee062e02cc8094a16b6d057abfde4e85235a76d63ffe267f63a9",
}
PUBKEY_RE = r"ssh-ed25519 [A-Za-z0-9+/=]+( [A-Za-z0-9._@-]+)?"


def render_setup_ssh(state):
    """setup_ssh.ps1: deterministic, contains only the VM-specific PUBLIC key. Idempotent; works from its own path.
    UNVERIFIED until its first run: the MSI feature names (ADDLOCAL) and the service name sshd."""
    pub = (state["guest"].get("ssh_public_key") or "").strip()
    require(re.fullmatch(PUBKEY_RE, pub), "state has no valid ssh public key")
    require("'" not in pub, "bad key")
    return f"""# setup_ssh.ps1 - generated by propimage.py (docs/design/V3_WINDOWS_PROPRIETARY_IMAGE.md). Idempotent.
# Run once in an ADMIN PowerShell:  powershell -ExecutionPolicy Bypass -File <drive>:\\setup_ssh.ps1
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$msi = Join-Path $here '{OPENSSH_MSI}'
$want = '{PINS[OPENSSH_MSI]}'
$key = '{pub}'
$got = (Get-FileHash -Algorithm SHA256 -LiteralPath $msi).Hash.ToLower()
if ($got -ne $want) {{ throw "OpenSSH MSI sha256 mismatch: $got" }}
if (-not (Get-Service sshd -ErrorAction SilentlyContinue)) {{
    $p = Start-Process msiexec.exe -Wait -PassThru -ArgumentList '/i', ('"' + $msi + '"'), 'ADDLOCAL=Client,Server', '/qn', '/norestart'
    if ($p.ExitCode -ne 0 -and $p.ExitCode -ne 3010) {{ throw "msiexec exit $($p.ExitCode)" }}
}}
if (-not (Get-Service sshd -ErrorAction SilentlyContinue)) {{ throw 'sshd service missing after the MSI (check the ADDLOCAL feature names)' }}
Set-Service -Name sshd -StartupType Automatic
if (-not (Get-NetFirewallRule -Name 'OpenSSH-Server-In-TCP' -ErrorAction SilentlyContinue)) {{
    New-NetFirewallRule -Name 'OpenSSH-Server-In-TCP' -DisplayName 'OpenSSH Server (sshd)' -Enabled True -Direction Inbound -Protocol TCP -Action Allow -LocalPort 22 | Out-Null
}}
New-Item -Path 'HKLM:\\SOFTWARE\\OpenSSH' -Force | Out-Null
New-ItemProperty -Path 'HKLM:\\SOFTWARE\\OpenSSH' -Name DefaultShell -PropertyType String -Force -Value 'C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe' | Out-Null
Start-Service sshd
$dir = Join-Path $env:ProgramData 'ssh'
New-Item -ItemType Directory -Path $dir -Force | Out-Null
$f = Join-Path $dir 'administrators_authorized_keys'
Set-Content -LiteralPath $f -Value $key -Encoding ascii
icacls.exe $f /inheritance:r /grant 'Administrators:F' /grant 'SYSTEM:F' | Out-Null
Restart-Service sshd
Write-Output 'SSH_READY'
Write-Output ('SMBIOS_UUID ' + (Get-CimInstance Win32_ComputerSystemProduct).UUID)
Write-Output ('MACHINE_GUID ' + (Get-ItemProperty 'HKLM:\\SOFTWARE\\Microsoft\\Cryptography').MachineGuid)
"""


def write_tools_dir(state, outdir, msi, nvidia, template_path=UNATTEND_TEMPLATE):
    """The ONE install CD (label KFTOOLS): autounattend.xml, the pinned OpenSSH MSI, setup_ssh.ps1, the pinned NVIDIA
    installer. Nothing else; the virtio-win CD is attached separately."""
    out = Path(outdir)
    out.mkdir(parents=True, exist_ok=True)
    for src, name in ((msi, OPENSSH_MSI), (nvidia, NVIDIA_EXE)):
        require(Path(src).name == name, "unexpected file name: %s (want %s)" % (Path(src).name, name))
        got = sha256_file(src)
        require(got == PINS[name], "sha256 mismatch for %s: %s" % (name, got))
        dst = out / name
        if dst.exists():
            dst.unlink()
        try:
            dst.hardlink_to(src)
        except OSError:
            shutil.copyfile(src, dst)
    (out / "setup_ssh.ps1").write_text(render_setup_ssh(state), encoding="utf-8", newline="\r\n")
    (out / "autounattend.xml").write_text(render_unattend(Path(template_path).read_text(encoding="utf-8"), state), encoding="utf-8")
    return sorted(p.name for p in out.iterdir())


STATE_FILES = ("tpm/tpm2-00.permall", "tpm/tpm2-00.savestate", "tpm/tpm2-00.volatilestate", "firmware/OVMF_VARS.fd")


def capture_files(state, workdir):
    """Embed the swtpm state files and the OVMF variables (written by a stopped VM; layout tpm/*, firmware/OVMF_VARS.fd under the work directory) into the state."""
    wd = Path(workdir)
    files = {}
    for name in STATE_FILES:
        src = wd / name
        if src.is_symlink() or not src.is_file():
            require(name.endswith(("savestate", "volatilestate")), "missing required file " + name)
            continue
        data = src.read_bytes()
        files[name] = {"bytes": len(data), "sha256": hashlib.sha256(data).hexdigest(),
                       "data_base64": base64.b64encode(data).decode()}
    state["files"] = files
    return sorted(files)


def restore_files(state, workdir):
    """Write the embedded blobs back (same layout as capture)."""
    wd = Path(workdir)
    for name, rec in state["files"].items():
        dst = wd / name
        dst.parent.mkdir(parents=True, exist_ok=True)
        dst.write_bytes(base64.b64decode(rec["data_base64"], validate=True))
        dst.chmod(0o600)


# ---------------------------------------------------------------- cli
def _print_rows(rows):
    for item, verdict, detail in rows:
        print("%-9s %-18s %s" % (verdict, item, detail))
    bad = [r for r in rows if r[1] == "MISMATCH"]
    print("RESULT %s" % ("MATCH" if not bad else "MISMATCH (%d)" % len(bad)))
    return 0 if not bad else 1


def _load(path):
    state = json.loads(Path(path).read_text())
    validate_state(state)
    require(seed_matches(state), "state ids do not match its seed")
    return state


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="verb", required=True)
    p = sub.add_parser("ids"); p.add_argument("seed")
    p = sub.add_parser("password"); p.add_argument("state")
    p = sub.add_parser("init"); p.add_argument("seed"); p.add_argument("--out", required=True)
    p.add_argument("--ssh-key-dir", help="directory with id_ed25519 and id_ed25519.pub for the guest")
    p = sub.add_parser("tools"); p.add_argument("state"); p.add_argument("--msi", required=True)
    p.add_argument("--nvidia", required=True); p.add_argument("--out", required=True)
    p = sub.add_parser("argv"); p.add_argument("state"); p.add_argument("mode", choices=MODES)
    p.add_argument("--disk", required=True); p.add_argument("--workdir", required=True)
    p.add_argument("--broker-sock"); p.add_argument("--gop-efi"); p.add_argument("--uid", type=int, default=1000)
    p.add_argument("--display", default="broker"); p.add_argument("--qemu"); p.add_argument("--cdrom", action="append", default=[])
    p.add_argument("--cpuinfo", default="/proc/cpuinfo")
    p.add_argument("--no-cpu-preflight", action="store_true", help="print the argv on a host that cannot run it; never for a start")
    p = sub.add_parser("verify"); p.add_argument("state")
    p.add_argument("--ssh-key", help="guest checks over ssh; without it only the host-side checks run")
    p.add_argument("--workdir", help="host-side check of the swtpm and OVMF files in this work directory")
    p.add_argument("--cpuinfo", default="/proc/cpuinfo")
    p.add_argument("--record", action="store_true", help="fill MachineGuid, EK hash, stop time and CPU fingerprint from the guest")
    p = sub.add_parser("pack"); p.add_argument("state"); p.add_argument("image"); p.add_argument("--out", required=True)
    p.add_argument("--workdir", help="embed the swtpm and OVMF files of this work directory first")
    p = sub.add_parser("unpack"); p.add_argument("zip"); p.add_argument("--dir", required=True)
    p.add_argument("--workdir", help="restore the swtpm and OVMF files into this work directory")
    a = ap.parse_args(argv)
    try:
        if a.verb == "ids":
            print(json.dumps(derive_ids(a.seed), indent=2))
        elif a.verb == "password":
            print(derive_password(_load(a.state)["seed"]))
        elif a.verb == "init":
            st = init_state(a.seed)
            if a.ssh_key_dir:
                kd = Path(a.ssh_key_dir)
                st["guest"]["ssh_private_key"] = (kd / "id_ed25519").read_text()
                st["guest"]["ssh_public_key"] = (kd / "id_ed25519.pub").read_text().strip()
            Path(a.out).write_text(json.dumps(st, indent=2) + "\n")
        elif a.verb == "tools":
            print(" ".join(write_tools_dir(_load(a.state), a.out, a.msi, a.nvidia)))
        elif a.verb == "argv":
            state = _load(a.state)
            if not a.no_cpu_preflight:
                bad = [r for r in cpu_preflight(state, Path(a.cpuinfo).read_text()) if r[1] != "MATCH"]
                if bad:
                    _print_rows(bad)
                    print(CPU_REFUSAL, file=sys.stderr)
                    return 3
            print(shlex.join(build_argv(state, a.mode, a.disk, a.workdir, qemu=a.qemu, uid=a.uid, broker_sock=a.broker_sock,
                                        gop_efi=a.gop_efi, display=a.display, cdroms=a.cdrom)))
            print("# swtpm: " + shlex.join(swtpm_argv(a.workdir)))
            print("# clone: " + shlex.join(clone_cmd(a.disk, str(a.disk) + ".clone")))
        elif a.verb == "verify":
            state = _load(a.state)
            cpuinfo = Path(a.cpuinfo).read_text()
            if a.record:
                require(a.ssh_key, "--record needs --ssh-key")
                fill_from_guest(state, run_guest_probe(state, a.ssh_key))
                Path(a.state).write_text(json.dumps(state, indent=2) + "\n")
                print("recorded machine_guid, ek_pub_hash, last_time_at_stop_utc, cpu_fingerprint")
                return 0
            rows = verify_host(state, a.workdir, cpuinfo) if a.workdir else cpu_preflight(state, cpuinfo)
            if a.ssh_key:
                rows += compare(state, run_guest_probe(state, a.ssh_key))
            return _print_rows(rows)
        elif a.verb == "pack":
            state = _load(a.state)
            if a.workdir:
                capture_files(state, a.workdir)
                Path(a.state).write_text(json.dumps(state, indent=2) + "\n")
            print(json.dumps(pack(a.state, a.image, a.out)))
        else:
            state = unpack(a.zip, a.dir)
            if a.workdir:
                restore_files(state, a.workdir)
            print("unpacked and verified")
    except (PropError, OSError, KeyError, json.JSONDecodeError) as e:
        print("propimage: " + str(e), file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
