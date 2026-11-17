#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Proprietary Linux image (image C): seed -> identity, setup VM, seal, offline clone, R2 zip.

STATUS: LIVE as tooling, 2026-11-17 (GPU-free; what ran on a box is listed in docs/design/V3_PROPIMAGE_LINUX.md, section 12,
and nothing else is measured; the commands of section 13 are unit-tested with scripted guests and have NOT run on a guest). Design: docs/design/V3_PROPIMAGE_LINUX.md. Logic lives here; shell is glue only.

  propimage_linux.py ids SEED
  propimage_linux.py init --seed S --dir D [--ram-mb N --smp N --disk-gb N --nic virtio-net-pci|e1000e]
  propimage_linux.py fetch-base --dir D                   pinned Ubuntu cloud image (sha256 checked)
  propimage_linux.py argv STATE MODE ...                  the exact qemu argv (setup | run)
  propimage_linux.py setup --dir D                        boot the setup VM (pinned identity, REAL clock, network ON,
                                                          VNC on 127.0.0.1 only), unattended provisioning on the first call
  propimage_linux.py novnc --dir D                        websockify (hash-pinned venv) bound to 127.0.0.1 only
  propimage_linux.py inventory --dir D                    read-only listing of files created after provisioning (no contents)
  propimage_linux.py harden-offline --dir D               SETUP VM, before the seal: nftables default drop on output and forward (loopback and
                                                          established/related only), apt timers masked, apt sources recorded then removed
  propimage_linux.py unharden --dir D [--clone N]         a THROWAWAY clone only (never the sealed image): flush the ruleset (online logout step)
  propimage_linux.py seal --dir D [--grep-*] [--vendor-entry 'name=X release=Y products=A,B installer_sha256=H installed_via=V']...
                          [--compare PREV.json] [--require-hardened]
                                                          clean stop, record the guest UTC, trim, convert, packages.manifest.json, grep REPORT
  propimage_linux.py diff-packages A.json|A.zip B.json|B.zip   what changed between two seals' package manifests (exit 1 if anything)
  propimage_linux.py inject --dir D [--name N] --src PATH --dest GUESTPATH [--max-gb 20] [--replace]
                                                          tar over ssh into a RUNNING offline clone (dest under /home/kfimage or /opt/kf)
  propimage_linux.py inject --dir D [--name N] --wheelhouse DIR --lock FILE [--dest GUESTPATH]
                                                          copy a wheelhouse, then pip install --no-index --require-hashes INSIDE the clone
  propimage_linux.py scan-image IMAGE|ZIP [--grep-*]      independent credential grep over a sealed image (counts only)
  propimage_linux.py run --dir D [--check]                offline clone: restrict=on, RECORDED clock, same identity
  propimage_linux.py offline-check --dir D                MATLAB-free command set in an offline clone; exit codes and hashes only
  propimage_linux.py auth-capture --dir D [--cmd C]       sign-in helper: URL + forward command (loopback) or paste-code; never a code
  propimage_linux.py auth-paste --dir D                   hidden prompt; sends the code over ssh stdin to the guest pane, stores nothing
  propimage_linux.py auth-selftest --dir D                the stub app end to end on an offline clone (no MathWorks traffic)
  propimage_linux.py install-driver --dir D --run F ...   stage and build an NVIDIA .run on a running clone, offline
  propimage_linux.py pack STATE IMAGE --out ZIP | unpack ZIP --dir D

Owner rules, 2026-11-17: no apt (and no other network) in a clone; every package of the sealed image is documented in
packages.manifest.json; no Python dependencies in the image (environments are built at run time and copied into the clone).

The clock rule (docs/design/V3_CLOCK_RULE.md, scripts/bench/windows/clockrule.py) is the only clock logic: network off ->
recorded clock from state.json, network on -> real clock, a recorded clock with network on is refused.
"""
import argparse
import base64
import getpass
import copy
import datetime as dt
import hashlib
import importlib.util
import json
import os
import re
import shlex
import shutil
import signal
import subprocess
import sys
import time
import urllib.parse
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent


def _load(name, rel):
    spec = importlib.util.spec_from_file_location(name, HERE / rel)
    m = importlib.util.module_from_spec(spec)
    sys.modules.setdefault(name, m)
    spec.loader.exec_module(m)
    return m


W = _load("kf_propimage_windows", "../windows/propimage.py")  # derive_ids, derive_password, CPU profile: reused unchanged
CR = _load("kf_clockrule", "../windows/clockrule.py")          # the one clock rule
P = _load("kf_propimage_linux_pkgs", "propimage_linux_pkgs.py")  # package manifest, harden-offline, inject (pure; ssh is passed in)

PropError = W.PropError
require = W.require
ClockError = CR.ClockError
DOMAIN = "kayfabe-propimage-linux/v1"
STATE_VERSION = 1
MODES = ("setup", "run")
NIC_MODELS = ("virtio-net-pci", "e1000e")  # OPEN QUESTION: a licensed application may only accept a physical-looking adapter
GUEST_USER = "kfimage"
BASE = {
    "name": "ubuntu-24.04-server-cloudimg-amd64.img", "release": "release-20261017",
    "url": "https://cloud-images.ubuntu.com/releases/noble/release-20261017/ubuntu-24.04-server-cloudimg-amd64.img",
    "sha256": "04e8f2461a16a44e78a370284471f29616cdd9baf1893df25807b71f985b6c94",  # SHA256SUMS of that release dir
    "sha256_source": "https://cloud-images.ubuntu.com/releases/noble/release-20261017/SHA256SUMS",
}
OVMF_CODE = "/usr/share/OVMF/OVMF_CODE_4M.fd"
OVMF_VARS_TEMPLATE = "/usr/share/OVMF/OVMF_VARS_4M.fd"
STATE_FILES = ("firmware/OVMF_VARS.fd",)
MIN_AVAILABLE_MB_BOOT = 2048   # the owner's rule for this coordinator box: do not boot below it
MIN_AVAILABLE_MB_RUNNING = 600  # a running guest is killed (never the host's other work) below it
NOVNC = {  # both pinned by sha256 (read from PyPI's JSON API and from a download on 2026-11-16); pip runs with --require-hashes
    "websockify": "0.13.0",
    "websockify_wheel_sha256": "ac497a8dafc7f51d28ca989f01cdf5f8add663a497d425a56b159dcb8d6a314c",
    "novnc_url": "https://github.com/novnc/noVNC/archive/refs/tags/v1.6.0.tar.gz",
    "novnc_sha256": "5066103959ef4e9b10f37e5a148627360dd8414e4cf8a7db92bdbd022e728aaa",
}


# ---------------------------------------------------------------- identity
def derive_linux_ids(seed):
    """Linux-only identity, in its own key: derive_ids() of the Windows kind stays byte for byte unchanged."""
    require(isinstance(seed, str) and 4 <= len(seed) <= 128 and seed.isprintable(), "seed must be a printable string of 4..128 characters")

    def h(field):
        return hashlib.sha256(f"{DOMAIN}\0{seed}\0{field}".encode()).digest()

    return {"machine_id": h("machine-id")[:16].hex(), "hostname": "kf-" + h("hostname").hex()[:8]}


def init_state(seed, ram_mb=8192, smp=4, disk_gb=60, nic_model="virtio-net-pci", keypair=None, desktop=False):
    require(nic_model in NIC_MODELS, "nic_model must be one of %s" % (NIC_MODELS,))
    require(1024 <= ram_mb <= 262144 and 1 <= smp <= 64 and 8 <= disk_gb <= 2048, "bad ram_mb, smp or disk_gb")
    ids = W.derive_ids(seed)
    cpu = copy.deepcopy(W.CPU_PROFILE)
    cpu["hv_flags"] = []  # Linux: no Hyper-V enlightenments
    priv, pub = keypair or (None, None)
    return {
        "state_version": STATE_VERSION, "os": "linux", "seed": seed, "ids": ids, "linux_ids": derive_linux_ids(seed), "desktop": bool(desktop),
        "clock": {"last_time_at_stop_utc": None, "gap_s": ids["clock_gap_s"]},
        "qemu": {"machine": "pc-q35-10.2", "smp": smp, "ram_mb": ram_mb, "disk_gb": disk_gb, "fb_mb": 4096,
                 "nic_model": nic_model, "guest_driver": None},   # guest_driver null: the NVIDIA driver is a run parameter
        "cpu": cpu, "cpu_fingerprint": None, "files": {}, "base": copy.deepcopy(BASE),
        "guest": {"user": GUEST_USER, "password": W.derive_password(seed), "ssh_private_key": priv, "ssh_public_key": pub},
        "packages": None, "seal": None, "created_utc": None,
    }


def validate_state(state):
    require(state.get("state_version") == STATE_VERSION and state.get("os") == "linux", "not a Linux propimage state")
    ids, lx, q = state["ids"], state["linux_ids"], state["qemu"]
    require(re.fullmatch(r"[0-9a-f]{32}", lx["machine_id"]), "bad machine-id")
    require(re.fullmatch(r"kf-[0-9a-f]{8}", lx["hostname"]), "bad hostname")
    require(re.fullmatch(r"[0-9a-f]{8}(-[0-9a-f]{4}){3}-[0-9a-f]{12}", ids["smbios"]["uuid"]), "bad SMBIOS uuid")
    for n in ids["nics"]:
        require(re.fullmatch(r"[0-9a-f]{2}(:[0-9a-f]{2}){5}", n["mac"]) and int(n["mac"][:2], 16) & 3 == 2,
                "bad MAC (must be unicast and locally administered)")
    for d in ids["disks"]:
        require(re.fullmatch(r"[A-Z0-9]{1,20}", d["serial"]), "bad disk serial (virtio limit 20)")
    require(re.fullmatch(r"GPU-[0-9a-f]{8}(-[0-9a-f]{4}){3}-[0-9a-f]{12}", ids["gpu_uuid"]), "bad GPU uuid")
    require(re.fullmatch(r"10\.\d+\.\d+\.0/24", ids["net"]["subnet"]), "slirp subnet must be a 10.x.y.0/24")
    require(1024 <= ids["net"]["ssh_port"] <= 65535, "bad ssh port")
    require(q["nic_model"] in NIC_MODELS, "bad nic_model")
    require(1 <= q["smp"] <= 64 and 1024 <= q["ram_mb"] <= 262144 and 8 <= q["disk_gb"] <= 2048, "bad smp, ram_mb or disk_gb")
    require(q["guest_driver"] is None or re.fullmatch(r"\d+\.\d+(\.\d+)?", q["guest_driver"]), "bad guest_driver")
    require(state["guest"]["password"] == W.derive_password(state["seed"]), "guest.password is not the seed-derived password")
    require(state["cpu"]["name"] == W.CPU_PROFILE_NAME and state["cpu"]["hv_flags"] == [], "unknown CPU profile or Hyper-V flags in a Linux state")
    require(not set(W.FORBIDDEN_GUEST_FLAGS) - set(state["cpu"]["remove"]), "profile must remove avx512f, rtm, hle, mpx, sgx")
    last = state["clock"]["last_time_at_stop_utc"]
    require(last is None or re.fullmatch(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ", last), "bad last_time_at_stop_utc")
    for name, rec in state["files"].items():
        data = base64.b64decode(rec["data_base64"], validate=True)
        require(len(data) == rec["bytes"] and hashlib.sha256(data).hexdigest() == rec["sha256"], "blob mismatch: " + name)
        require(not name.startswith("/") and ".." not in Path(name).parts, "bad blob path " + name)
    return True


def seed_matches(state):
    """Hand-edited identity is refused: both id groups must be what the seed derives."""
    return (W.derive_ids(state["seed"], len(state["ids"]["nics"]), len(state["ids"]["disks"])) == state["ids"]
            and derive_linux_ids(state["seed"]) == state["linux_ids"])


def load_state(path):
    state = json.loads(Path(path).read_text())
    validate_state(state)
    require(seed_matches(state), "state ids do not match its seed")
    return state


def save_state(state, path):
    tmp = Path(str(path) + ".tmp")
    tmp.write_text(json.dumps(state, indent=2) + "\n")
    tmp.chmod(0o600)
    os.replace(tmp, path)


# ---------------------------------------------------------------- clock (the shared rule; no logic of its own)
def recorded_base(state, offset_s=0):
    last = state["clock"]["last_time_at_stop_utc"]
    require(last, "the state has no clock.last_time_at_stop_utc (seal records it at the clean stop)")
    return W.utc(last) + dt.timedelta(seconds=state["clock"]["gap_s"] + offset_s)


def clock_for_mode(state, mode, net, loaded_software=(), offset_s=0):
    """-> the clockrule decision. setup (network on): real clock. run offline: the recorded clock, never earlier than any
    loaded software (KF_CLOCK_BEFORE_INPUT). run online: real clock. A recorded clock with network on is refused."""
    require(mode in MODES and net in ("online", "offline"), "bad mode or net")
    if net == "online":
        return CR.clock_for("online", "propimage-linux", phase="build" if mode == "setup" else "run")
    base = CR.fmt_utc(recorded_base(state, offset_s))
    return CR.clock_for("offline", "propimage-linux", base=base, loaded_software=list(loaded_software))


def clock_refused_for(state, net, clock_kind):
    """The assert for an explicit pairing (a CLI `--clock recorded` with `--net online` must refuse)."""
    CR.assert_rule(net, clock_kind)


# ---------------------------------------------------------------- argv
def _comma(v):
    return str(v).replace(",", ",,")


def build_argv(state, mode, disk, workdir, net=None, clock=None, qemu="/usr/bin/qemu-system-x86_64", ovmf_code=OVMF_CODE,
               seed_iso=None, vnc_display=None, gpu="none", driver_version=None, daemonize=True, extra_iso=()):
    """The exact qemu argv. mode setup: network ON, real clock, VNC (127.0.0.1 only), the pinned identity.
    mode run: network OFF (restrict=on), the recorded clock from `clock` (a clockrule decision), the same identity.
    gpu: none | kf3 (the later clone on a GPU box; the NVIDIA version is the run's, never the image's)."""
    require(mode in MODES, "mode must be one of %s" % (MODES,))
    net = net or ("online" if mode == "setup" else "offline")
    ids, q = state["ids"], state["qemu"]
    wd = str(workdir)
    clock = clock or clock_for_mode(state, mode, net)
    require(not (net == "online" and clock["kind"] == "recorded"), "online with a recorded clock is refused")
    require(not (net == "offline" and clock["kind"] != "recorded") or mode == "setup", "an offline run needs the recorded clock")
    restrict = "off" if net == "online" else "on"
    sm = ids["smbios"]
    nic = q["nic_model"]
    a = [qemu, "-name", "propimage-linux-" + mode,
         "-object", "memory-backend-memfd,id=ram0,size=%dM,share=on" % q["ram_mb"],
         "-machine", q["machine"] + ",accel=kvm,memory-backend=ram0", "-m", str(q["ram_mb"]),
         "-smp", "%d,sockets=1,cores=%d,threads=1" % (q["smp"], q["smp"]), "-cpu", W.cpu_arg(state),
         "-uuid", sm["uuid"],
         "-smbios", "type=1,manufacturer=%s,product=%s,serial=%s" % (_comma(sm["manufacturer"]), _comma(sm["product"]), sm["serial"]),
         "-smbios", "type=2,manufacturer=%s,product=%s,serial=%s" % (_comma(sm["manufacturer"]), _comma(sm["product"]), sm["baseboard_serial"]),
         "-smbios", "type=3,manufacturer=%s,serial=%s" % (_comma(sm["manufacturer"]), sm["chassis_serial"]),
         "-global", "ICH9-LPC.disable_s3=1", "-global", "ICH9-LPC.disable_s4=1",
         "-drive", "if=pflash,format=raw,unit=0,readonly=on,file=" + ovmf_code,
         "-drive", "if=pflash,format=raw,unit=1,file=%s/firmware/OVMF_VARS.fd" % wd,
         "-rtc", clock["rtc"],
         "-drive", "if=none,id=sys0,format=qcow2,cache=none,aio=threads,discard=unmap,detect-zeroes=unmap,file=" + str(disk),
         "-device", "virtio-blk-pci,drive=sys0,serial=" + ids["disks"][0]["serial"],
         "-netdev", "user,id=n0,net=%s,ipv6=off,restrict=%s,hostfwd=tcp:127.0.0.1:%d-:22" % (ids["net"]["subnet"], restrict, ids["net"]["ssh_port"]),
         "-device", "%s,netdev=n0,mac=%s" % (nic, ids["nics"][0]["mac"]),
         "-object", "rng-random,id=rng0,filename=/dev/urandom", "-device", "virtio-rng-pci,rng=rng0",
         "-serial", "file:%s/serial.log" % wd,
         "-qmp", "unix:%s/qmp.sock,server=on,wait=off" % wd, "-msg", "timestamp=on"]
    if mode == "setup":
        # the setup VM only: balloon free-page reporting hands the guest's freed pages back to this (shared) coordinator box
        a += ["-device", "virtio-balloon-pci,free-page-reporting=on,deflate-on-oom=on"]
    media = ([seed_iso] if seed_iso else []) + list(extra_iso)
    for i, iso in enumerate(media):
        a += ["-drive", "if=none,id=cd%d,format=raw,media=cdrom,readonly=on,file=%s" % (i, iso),
              "-device", "ide-cd,drive=cd%d,bus=ide.%d" % (i, i)]
    if vnc_display is not None:
        require(isinstance(vnc_display, int) and 0 <= vnc_display <= 99, "vnc display must be 0..99")
        a += ["-vnc", "127.0.0.1:%d" % vnc_display]   # NEVER a public bind: there is no code path that builds another address
    else:
        a += ["-display", "none"]
    if gpu == "kf3":
        require(driver_version or q["guest_driver"], "GUEST_DRIVER_REQUIRED: the image pins no NVIDIA driver; pass the run's guest driver")
        a += ["-vga", "none", "-device", ",".join(["kf3-gpu", "id=kf0", "fb-mb=%d" % q["fb_mb"], "bar1-size=134217728", "bar2-size=33554432",
                                                   "gpu-uuid=" + ids["gpu_uuid"], "guest-driver=" + (driver_version or q["guest_driver"])])]
    else:
        require(gpu == "none", "gpu must be none or kf3")
        a += ["-vga", "std"]
    a += ["-device", "qemu-xhci,id=xhci", "-device", "usb-tablet,bus=xhci.0", "-device", "usb-kbd,bus=xhci.0"]
    if daemonize:
        a += ["-daemonize", "-pidfile", "%s/qemu.pid" % wd]
    return a


# ---------------------------------------------------------------- cloud-init seed (setup only; the sealed image has it disabled)
def render_netplan(state):
    mac = state["ids"]["nics"][0]["mac"]
    return ("network:\n  version: 2\n  renderer: networkd\n  ethernets:\n    kfnic0:\n      match:\n        macaddress: \"%s\"\n"
            "      dhcp4: true\n      dhcp6: false\n      dhcp-identifier: mac\n      optional: true\n" % mac)


def render_cloud_init(state):
    """-> {user-data, meta-data, network-config}: deterministic, the same state gives the same bytes. The seed carries the
    throwaway account password and the PUBLIC key only."""
    g = state["guest"]
    pub = (g.get("ssh_public_key") or "").strip()
    require(re.fullmatch(W.PUBKEY_RE, pub), "state has no valid ssh public key")
    host = state["linux_ids"]["hostname"]
    sshd = "PasswordAuthentication no\nPermitRootLogin no\nKbdInteractiveAuthentication no\n"
    user = (
        "#cloud-config\n"
        "hostname: %s\npreserve_hostname: true\nmanage_etc_hosts: false\nssh_pwauth: false\ndisable_root: true\nresize_rootfs: true\n"
        "users:\n  - name: %s\n    gecos: kayfabe image\n    groups: [sudo, adm, video, render]\n    shell: /bin/bash\n"
        "    sudo: \"ALL=(ALL) NOPASSWD:ALL\"\n    lock_passwd: false\n    ssh_authorized_keys:\n      - %s\n"
        "chpasswd:\n  expire: false\n  users:\n    - {name: %s, password: \"%s\", type: text}\n"
        "write_files:\n  - path: /etc/ssh/sshd_config.d/60-kf.conf\n    permissions: \"0644\"\n    content: |\n%s"
        % (host, g["user"], pub, g["user"], g["password"], "".join("      " + l + "\n" for l in sshd.splitlines())))
    meta = "instance-id: kf-%s\nlocal-hostname: %s\n" % (state["linux_ids"]["machine_id"][:12], host)
    return {"user-data": user, "meta-data": meta, "network-config": render_netplan(state)}


def make_seed_iso(files, out, runner=subprocess.run):
    d = Path(str(out) + ".d")
    shutil.rmtree(d, ignore_errors=True)
    d.mkdir(parents=True)
    for name, text in files.items():
        (d / name).write_text(text)
    runner(["genisoimage", "-quiet", "-output", str(out), "-volid", "cidata", "-joliet", "-rock", str(d)], check=True)
    shutil.rmtree(d)
    return out


# ---------------------------------------------------------------- guest scripts (fixed, short, linear; run as root over ssh)
STAGE_A = r"""set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
cloud-init status --wait >/dev/null 2>&1 || true
# the host is shared: hand the page cache back instead of letting the guest grow into it
( while sleep 15; do sync; echo 3 > /proc/sys/vm/drop_caches; done ) >/dev/null 2>&1 &
printf 'Package: firefox thunderbird\nPin: release *\nPin-Priority: -1\n' > /etc/apt/preferences.d/kf-no-snap-browsers
apt-get update -qq
apt-get install -y -qq linux-image-generic linux-headers-generic build-essential dkms pkg-config libglvnd-dev unzip curl tmux python3 python3-venv nftables @@DESKTOP_PKGS@@
echo STAGE_A_OK
"""
DESKTOP_PKGS = "ubuntu-desktop-minimal"


def render_stage_a(state):
    """The desktop is OPTIONAL (state.desktop; a state written before the field existed is a desktop image). Terminal rows need none."""
    return STAGE_A.replace("@@DESKTOP_PKGS@@", DESKTOP_PKGS if state.get("desktop", True) else "")


AUTH_LOG = "/var/tmp/kf-auth-urls.log"
AUTH_SHIM = r"""#!/bin/sh
# kayfabe auth-URL capture shim: opens NOTHING. Appends the URL to the log (mode 0600) and prints it on the terminal.
LOG=${KF_AUTH_URL_LOG:-/var/tmp/kf-auth-urls.log}
umask 077
for u in "$@"; do
  case "$u" in -*) continue;; esac
  printf '%s\t%s\n' "$(date -u +%FT%TZ)" "$u" >> "$LOG"
  printf 'KF_AUTH_URL %s\n' "$u" >&2
done
chmod 600 "$LOG" 2>/dev/null || true
exit 0
"""

DROPCACHES_CMD = ("sudo bash -c 'nohup sh -c \"while sleep 20; do sync; echo 3 > /proc/sys/vm/drop_caches; done\" >/dev/null 2>&1 &'")

STAGE_B = r"""set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
MID=@@MACHINE_ID@@
HOST=@@HOSTNAME@@
KREL=$(uname -r)
case "$KREL" in *-generic) ;; *) echo "STAGE_B_FAIL running kernel $KREL is not a -generic kernel"; exit 1;; esac
test -d "/lib/modules/$KREL/build" || { echo "STAGE_B_FAIL no linux-headers for $KREL"; exit 1; }
apt-get autoremove --purge -y -qq >/dev/null 2>&1 || true   # keeps the running kernel
apt-mark hold linux-image-generic linux-headers-generic linux-generic "linux-image-$KREL" "linux-headers-$KREL" "linux-modules-$KREL" >/dev/null
# no unattended changes, no time sync (the network is off in a clone; a recorded clock must stay the only clock source)
systemctl mask apt-daily.timer apt-daily-upgrade.timer unattended-upgrades.service snapd.refresh.timer \
    systemd-timesyncd.service systemd-networkd-wait-online.service NetworkManager-wait-online.service >/dev/null 2>&1 || true
printf 'APT::Periodic::Update-Package-Lists "0";\nAPT::Periodic::Unattended-Upgrade "0";\n' > /etc/apt/apt.conf.d/20auto-upgrades
timedatectl set-local-rtc 0 >/dev/null 2>&1 || true
# a user session that does not stop must not hold a clone's shutdown for 90 s (ADDED after the proof run: not measured)
mkdir -p /etc/systemd/system.conf.d; printf '[Manager]\nDefaultTimeoutStopSec=20s\n' > /etc/systemd/system.conf.d/kf-timeouts.conf
# desktop: GDM autologin on X11, no blanking, no lock, no first-run wizard
@@DESKTOP_BEGIN@@
mkdir -p /etc/gdm3
printf '[daemon]\nAutomaticLoginEnable=true\nAutomaticLogin=@@USER@@\nWaylandEnable=false\n' > /etc/gdm3/custom.conf
mkdir -p /etc/dconf/profile /etc/dconf/db/local.d
printf 'user-db:user\nsystem-db:local\n' > /etc/dconf/profile/user
cat > /etc/dconf/db/local.d/00-kf <<'EOF'
[org/gnome/desktop/session]
idle-delay=uint32 0
[org/gnome/desktop/screensaver]
lock-enabled=false
idle-activation-enabled=false
[org/gnome/settings-daemon/plugins/power]
sleep-inactive-ac-type='nothing'
sleep-inactive-battery-type='nothing'
[org/gnome/desktop/notifications]
show-banners=false
EOF
dconf update
install -d -o @@USER@@ -g @@USER@@ /home/@@USER@@/.config
printf 'yes\n' > /home/@@USER@@/.config/gnome-initial-setup-done
chown @@USER@@:@@USER@@ /home/@@USER@@/.config/gnome-initial-setup-done
@@DESKTOP_END@@
# auth-URL capture shim (opens nothing; see auth-capture): xdg-open first in PATH, BROWSER for every session
cat > /usr/local/bin/kf-capture-url <<'KFEOF'
@@AUTH_SHIM@@
KFEOF
chmod 755 /usr/local/bin/kf-capture-url
ln -sf kf-capture-url /usr/local/bin/xdg-open
ln -sf kf-capture-url /usr/local/bin/x-www-browser
ln -sf kf-capture-url /usr/local/bin/sensible-browser
grep -q '^BROWSER=' /etc/environment || echo 'BROWSER=/usr/local/bin/kf-capture-url' >> /etc/environment
mkdir -p /etc/environment.d
printf 'BROWSER=/usr/local/bin/kf-capture-url\n' > /etc/environment.d/60-kf-browser.conf
printf 'export BROWSER=/usr/local/bin/kf-capture-url\n' > /etc/profile.d/kf-browser.sh
# identity: effective from the next boot
umount /etc/machine-id 2>/dev/null || true
printf '%s\n' "$MID" > /etc/machine-id
rm -f /var/lib/dbus/machine-id; ln -s /etc/machine-id /var/lib/dbus/machine-id
printf '%s\n' "$HOST" > /etc/hostname
printf '127.0.0.1 localhost\n127.0.1.1 %s\n' "$HOST" > /etc/hosts
rm -rf /var/log/journal/*
# network: the one pinned NIC, DHCP; cloud-init never touches it again
rm -f /etc/netplan/50-cloud-init.yaml
cat > /etc/netplan/60-kf.yaml <<'EOF'
@@NETPLAN@@
EOF
chmod 600 /etc/netplan/60-kf.yaml
printf 'network: {config: disabled}\n' > /etc/cloud/cloud.cfg.d/99-kf-network.cfg
touch /etc/cloud/cloud-init.disabled
cloud-init status --wait >/dev/null 2>&1 || true
timeout 300 snap wait system seed.loaded >/dev/null 2>&1 || true
date -u +%FT%TZ > /var/lib/kf-provisioned
echo STAGE_B_OK
"""

# the compiler the running kernel was built with (the NVIDIA module builds with exactly it; the clone is offline and cannot apt)
KERNEL_CC_CMD = (r'k=$(sed -n "s/^CONFIG_CC_VERSION_TEXT=\"\([^ ]*\) .*/\1/p" /lib/modules/$(uname -r)/build/.config 2>/dev/null); '
                 r'[ -n "$k" ] || k=$(sed -n "s/^Linux version [^ ]* ([^)]*) (\([^ ]*\) .*/\1/p" /proc/version); echo "$k"')

PROBE_PY = r"""
import json, glob, os, subprocess, datetime
def rd(p):
    try: return open(p).read().strip()
    except Exception: return None
def sh(c):
    try: return subprocess.run(c, shell=True, capture_output=True, text=True, timeout=20).stdout.strip()
    except Exception: return ""
o = {}
o["machine_id"] = rd("/etc/machine-id")
o["hostname"] = rd("/etc/hostname")
o["smbios_uuid"] = (rd("/sys/class/dmi/id/product_uuid") or "").lower()
o["smbios_serial"] = rd("/sys/class/dmi/id/product_serial")
o["baseboard_serial"] = rd("/sys/class/dmi/id/board_serial")
o["chassis_serial"] = rd("/sys/class/dmi/id/chassis_serial")
o["macs"] = sorted(rd(p) for p in glob.glob("/sys/class/net/*/address") if not p.startswith("/sys/class/net/lo/"))
o["disk_serials"] = sorted(s for s in (rd(p) for p in glob.glob("/sys/block/vd*/serial")) if s)
o["gpu_uuid"] = (sh("nvidia-smi --query-gpu=uuid --format=csv,noheader 2>/dev/null").splitlines() or [None])[0]
o["date_utc"] = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
o["kernel"] = os.uname().release
o["headers_present"] = os.path.isdir("/lib/modules/%s/build" % o["kernel"])
o["kernel_cc"] = sh(@@KCC@@)
o["kernel_cc_present"] = bool(o["kernel_cc"]) and bool(sh("command -v " + o["kernel_cc"]))
o["gcc_present"] = bool(sh("command -v gcc")) and bool(sh("command -v make"))
o["cloud_init_disabled"] = os.path.exists("/etc/cloud/cloud-init.disabled")
o["net_default_route"] = bool(sh("ip route show default"))
o["internet"] = sh("curl -s -m 4 -o /dev/null -w '%{http_code}' https://example.com/ || true") not in ("", "000")
o["ping"] = subprocess.run("ping -c1 -W2 1.1.1.1", shell=True, capture_output=True).returncode == 0
o["desktop_sessions"] = sh("loginctl list-sessions --no-legend 2>/dev/null | wc -l")
o["gdm_active"] = sh("systemctl is-active gdm3 2>/dev/null")
flags = set()
for l in open("/proc/cpuinfo"):
    if l.startswith("flags"):
        flags = set(l.split(":", 1)[1].split()); break
o["cpu_flags_forbidden"] = sorted(flags & {"avx512f", "rtm", "hle", "mpx", "sgx"})
o["cpu_vendor"] = sh("sed -n 's/^vendor_id[^:]*: *//p' /proc/cpuinfo | head -1")
print(json.dumps(o))
"""


def probe_script():
    return PROBE_PY.replace("@@KCC@@", repr(KERNEL_CC_CMD))


def render_stage_b(state):
    s = STAGE_B
    for k, v in (("@@AUTH_SHIM@@", AUTH_SHIM.rstrip("\n")), ("@@MACHINE_ID@@", state["linux_ids"]["machine_id"]), ("@@HOSTNAME@@", state["linux_ids"]["hostname"]),
                 ("@@USER@@", state["guest"]["user"]), ("@@NETPLAN@@", render_netplan(state).rstrip("\n"))):
        require(k in s, "stage B lacks " + k)
        s = s.replace(k, v)
    b, _, rest = s.partition("@@DESKTOP_BEGIN@@\n")
    body, _, tail = rest.partition("@@DESKTOP_END@@\n")
    s = b + (body if state.get("desktop", True) else "") + tail
    require("@@" not in s, "unfilled placeholder")
    return s


def compare(state, guest, elapsed_s=None, net="offline"):
    """-> [(item, MATCH|MISMATCH|WARN, detail)]. Pure; the ssh call is separate. Offline: no network reachable, and the clock
    is the recorded base plus the elapsed time (never earlier than the base)."""
    ids, lx = state["ids"], state["linux_ids"]
    out = []

    def add(name, want, got, blocking=True):
        ok = want == got and want is not None
        out.append((name, "MATCH" if ok else ("MISMATCH" if blocking else "WARN"), "want=%r got=%r" % (want, got)))

    sm = ids["smbios"]
    add("machine_id", lx["machine_id"], guest.get("machine_id"))
    add("hostname", lx["hostname"], guest.get("hostname"))
    add("smbios_uuid", sm["uuid"], guest.get("smbios_uuid"))
    add("smbios_serial", sm["serial"], guest.get("smbios_serial"))
    add("baseboard_serial", sm["baseboard_serial"], guest.get("baseboard_serial"))
    add("chassis_serial", sm["chassis_serial"], guest.get("chassis_serial"))
    add("mac", sorted(n["mac"] for n in ids["nics"]), guest.get("macs"))
    add("disk_serial", sorted(d["serial"] for d in ids["disks"]), guest.get("disk_serials"))
    if guest.get("gpu_uuid") is not None:
        add("gpu_uuid", ids["gpu_uuid"], guest.get("gpu_uuid"))
    out.append(("kernel_headers", "MATCH" if guest.get("headers_present") else "MISMATCH", "kernel=%s" % guest.get("kernel")))
    out.append(("driver_build_tools", "MATCH" if guest.get("kernel_cc_present") and guest.get("gcc_present") else "MISMATCH",
                "kernel_cc=%s" % guest.get("kernel_cc")))
    out.append(("cloud_init_disabled", "MATCH" if guest.get("cloud_init_disabled") else "MISMATCH", repr(guest.get("cloud_init_disabled"))))
    out.append(("cpu_no_forbidden_flags", "MATCH" if not guest.get("cpu_flags_forbidden") else "WARN", repr(guest.get("cpu_flags_forbidden"))))
    if net == "offline":
        out.append(("no_network", "MATCH" if not guest.get("internet") and not guest.get("ping") else "MISMATCH",
                    "internet=%r ping=%r" % (guest.get("internet"), guest.get("ping"))))
        last = state["clock"]["last_time_at_stop_utc"]
        if last and guest.get("date_utc"):
            base = recorded_base(state)
            g = W.utc(guest["date_utc"])
            hi = base + dt.timedelta(seconds=(elapsed_s if elapsed_s is not None else 1800) + 10)
            out.append(("clock", "MATCH" if base <= g <= hi else "MISMATCH",
                        "base=%s guest=%s window_s=%s" % (W.fmt(base), guest["date_utc"], int((hi - base).total_seconds()))))
        else:
            out.append(("clock", "MISMATCH", "state has no last_time_at_stop_utc"))
    return out


# ---------------------------------------------------------------- host guards
def mem_available_mb(meminfo="/proc/meminfo"):
    for line in Path(meminfo).read_text().splitlines():
        if line.startswith("MemAvailable:"):
            return int(line.split()[1]) // 1024
    raise PropError("no MemAvailable in " + meminfo)


def guard_memory(min_mb=MIN_AVAILABLE_MB_BOOT, meminfo="/proc/meminfo"):
    have = mem_available_mb(meminfo)
    require(have >= min_mb, "HOST_MEMORY_LOW: MemAvailable %d MB < %d MB; not booting" % (have, min_mb))
    return have


def dir_size_bytes(path):
    total = 0
    for p in Path(path).rglob("*"):
        if p.is_file() and not p.is_symlink():
            st = p.stat()
            total += st.st_blocks * 512
    return total


# ---------------------------------------------------------------- ssh / scp
def ssh_opts(state, key):
    return ["-i", str(key), "-p", str(state["ids"]["net"]["ssh_port"]), "-o", "IdentitiesOnly=yes", "-o", "StrictHostKeyChecking=no",
            "-o", "UserKnownHostsFile=/dev/null", "-o", "BatchMode=yes", "-o", "ConnectTimeout=8", "-o", "LogLevel=ERROR"]


def ssh_argv(state, key, remote, host="127.0.0.1"):
    return ["ssh"] + ssh_opts(state, key) + ["%s@%s" % (state["guest"]["user"], host), remote]


def scp_argv(state, key, src, dst, host="127.0.0.1"):
    o = ssh_opts(state, key)
    o[o.index("-p")] = "-P"
    return ["scp", "-q"] + o + [str(src), "%s@%s:%s" % (state["guest"]["user"], host, dst)]


def run_ssh(state, key, remote, stdin=None, timeout=600, runner=subprocess.run):
    return runner(ssh_argv(state, key, remote), input=stdin, text=True, capture_output=True, timeout=timeout)


def wait_ssh(state, key, timeout_s=420, runner=subprocess.run, sleep=time.sleep, clock=time.monotonic):
    t0 = clock()
    while clock() - t0 < timeout_s:
        try:
            if run_ssh(state, key, "true", timeout=20, runner=runner).returncode == 0:
                return round(clock() - t0, 1)
        except subprocess.TimeoutExpired:
            pass
        sleep(5)
    raise PropError("SSH_TIMEOUT: the guest did not answer ssh within %ds" % timeout_s)


def run_probe(state, key, runner=subprocess.run):
    p = run_ssh(state, key, "sudo python3 -", stdin=probe_script(), timeout=120, runner=runner)
    require(p.returncode == 0, "probe failed: " + (p.stderr or "")[-300:])
    line = [l for l in p.stdout.splitlines() if l.startswith("{")]
    require(line, "no JSON from the guest probe")
    return json.loads(line[-1])


# ---------------------------------------------------------------- credential grep (counts only)
KINDS = ("literal", "regex", "token")


def parse_patterns(literals=(), regexes=(), tokens=(), pattern_file=None):
    """-> [{label, kind, pat(bytes)}]. The label is kind + index + the first 8 hex of sha256(pattern): the report names a pattern
    by that label, never by its text (an e-mail address is itself a secret)."""
    items = [("literal", p) for p in literals] + [("regex", p) for p in regexes] + [("token", p) for p in tokens]
    if pattern_file:
        for line in Path(pattern_file).read_text().splitlines():
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            kind, _, pat = line.partition(":")
            require(kind in KINDS and pat, "pattern file line must be kind:pattern with kind in %s" % (KINDS,))
            items.append((kind, pat))
    out = []
    for i, (kind, pat) in enumerate(items):
        raw = pat.encode()
        if kind != "literal":
            try:
                re.compile(raw)
            except re.error as e:
                raise PropError("bad %s pattern #%d: %s" % (kind, i + 1, e.msg))
        out.append({"label": "%s%d-%s" % (kind, i + 1, hashlib.sha256(raw).hexdigest()[:8]), "kind": kind, "pat": raw})
    return out


def _compile(p):
    if p["kind"] == "literal":
        # a literal is searched as ASCII and as UTF-16LE (Windows-style stores; harmless on Linux)
        return re.compile(b"|".join([re.escape(p["pat"]), re.escape(p["pat"].decode().encode("utf-16-le"))]), re.I)
    return re.compile(p["pat"])


def count_in_bytes(patterns, data):
    return {p["label"]: len(_compile(p).findall(data)) for p in patterns}


def scan_raw_image(path, patterns, chunk=16 << 20, overlap=4096):
    """Counts per pattern label over a raw image. Sparse holes are skipped (SEEK_DATA); a match that straddles two chunks is
    counted once (a match counts in the chunk its START lies in). A single match longer than `overlap` is not supported."""
    comp = [(p["label"], _compile(p)) for p in patterns]
    counts = {p["label"]: 0 for p in patterns}
    size = os.path.getsize(path)
    scanned = 0
    with open(path, "rb") as f:
        fd = f.fileno()
        pos = 0
        while pos < size:
            try:
                data_at = os.lseek(fd, pos, os.SEEK_DATA)
            except OSError:
                break                      # no more data
            pos = max(pos, data_at - data_at % 4096)
            buf = os.pread(fd, chunk + overlap, pos)   # not f.read: the buffered file and os.lseek would disagree on the offset
            if not buf:
                break
            main = min(chunk, len(buf))
            for label, rx in comp:
                counts[label] += sum(1 for m in rx.finditer(buf) if m.start() < main)
            scanned += main
            pos += main
    return {"bytes_scanned": scanned, "image_bytes": size, "counts": counts}


GUEST_GREP_PY = r"""
import json, os, re, sys
spec = json.loads(sys.stdin.read())
pats = [(p["label"], re.compile(bytes.fromhex(p["hex"]), re.I if p["kind"] == "literal" else 0) if p["kind"] != "literal"
         else re.compile(re.escape(bytes.fromhex(p["hex"])), re.I)) for p in spec["patterns"]]
skip = ("/proc", "/sys", "/dev", "/run", "/tmp/kf-scan")
res, nfiles = {}, 0
for root, dirs, files in os.walk("/", followlinks=False):
    if root in skip or any(root.startswith(s + "/") for s in skip):
        dirs[:] = []; continue
    for fn in files:
        p = os.path.join(root, fn)
        try:
            if os.path.islink(p) or not os.path.isfile(p) or os.path.getsize(p) > spec["max_file_bytes"]: continue
            data = open(p, "rb").read()
        except Exception:
            continue
        nfiles += 1
        c = {l: len(rx.findall(data)) for l, rx in pats}
        c = {l: n for l, n in c.items() if n}
        if c: res[p] = c
print(json.dumps({"files_scanned": nfiles, "per_path": res}))
"""


def guest_grep(state, key, patterns, runner=subprocess.run, max_file_bytes=256 << 20):
    """Runs INSIDE the guest over ssh (before the stop): counts per file path and pattern label, never the matches."""
    spec = json.dumps({"patterns": [{"label": p["label"], "kind": p["kind"], "hex": p["pat"].hex()} for p in patterns],
                       "max_file_bytes": max_file_bytes})
    p = runner(ssh_argv(state, key, "sudo python3 -c %s" % shlex.quote(GUEST_GREP_PY)), input=spec, text=True, capture_output=True, timeout=7200)
    require(p.returncode == 0, "guest grep failed: " + (p.stderr or "")[-200:])
    return json.loads([l for l in p.stdout.splitlines() if l.startswith("{")][-1])


def format_grep_report(raw, guest, patterns):
    """Text report: labels and counts only."""
    lines = ["CREDENTIAL GREP REPORT (counts only; matches are never printed)", "patterns: " + (", ".join(p["label"] for p in patterns) or "none")]
    if raw is not None:
        lines.append("whole image (raw, incl. free space and deleted data): scanned %d of %d bytes" % (raw["bytes_scanned"], raw["image_bytes"]))
        for lab, n in sorted(raw["counts"].items()):
            lines.append("  image  %-22s %d" % (lab, n))
    if guest is not None:
        lines.append("guest files: %d scanned, %d path(s) with matches" % (guest["files_scanned"], len(guest["per_path"])))
        for path in sorted(guest["per_path"]):
            lines.append("  path   %s  %s" % (path, " ".join("%s=%d" % kv for kv in sorted(guest["per_path"][path].items()))))
    total = (sum(raw["counts"].values()) if raw else 0) + (sum(sum(c.values()) for c in guest["per_path"].values()) if guest else 0)
    lines.append("TOTAL %d" % total)
    return "\n".join(lines) + "\n", total


# ---------------------------------------------------------------- inventory (read only, no contents)
INVENTORY_PY = r"""
import json, os, sys
spec = json.loads(sys.stdin.read())
since = float(spec["since"])
roots = spec["roots"]; broad = spec["broad"]; noise = tuple(spec["noise"])
rows, seen = [], set()
def add(p, why):
    try:
        st = os.lstat(p)
    except OSError:
        return
    if p in seen: return
    seen.add(p)
    rows.append({"path": p, "size": st.st_size, "mtime": int(st.st_mtime), "mode": oct(st.st_mode & 0o7777), "uid": st.st_uid,
                 "type": "d" if os.path.isdir(p) and not os.path.islink(p) else ("l" if os.path.islink(p) else "f"), "why": why})
import glob
for pat in roots:
    for r in glob.glob(pat):
        if os.path.isdir(r) and not os.path.islink(r):
            add(r, "named-root")
            for d, ds, fs in os.walk(r):
                for n in ds + fs: add(os.path.join(d, n), "named-root")
        else:
            add(r, "named-root")
for top in broad:
    for d, ds, fs in os.walk(top):
        ds[:] = [x for x in ds if not os.path.join(d, x).startswith(noise)]
        for n in fs:
            p = os.path.join(d, n)
            if p.startswith(noise): continue
            try:
                if os.lstat(p).st_mtime >= since: add(p, "changed-since-provisioning")
            except OSError:
                pass
print(json.dumps(rows))
"""
INVENTORY_ROOTS = ["/home/*/.matlab", "/home/*/.MathWorks", "/home/*/.config/MathWorks", "/home/*/.local/share/MathWorks",
                   "/home/*/.cache/MathWorks", "/root/.matlab", "/root/.MathWorks", "/usr/local/MATLAB/*/licenses",
                   "/usr/local/MATLAB/*/etc", "/opt/MATLAB/*/licenses", "/opt/mathworks*", "/var/lib/MathWorks*",
                   "/etc/MathWorks*", "/usr/local/MathWorks*", "/var/MathWorks*", "/home/*/MathWorks*"]
INVENTORY_BROAD = ["/home", "/root", "/etc", "/opt", "/usr/local", "/var/lib", "/var/opt"]
INVENTORY_NOISE = ("/var/lib/apt", "/var/lib/dpkg", "/var/lib/systemd", "/var/lib/snapd", "/var/lib/cloud", "/var/lib/dhcpcd",
                   "/var/lib/NetworkManager", "/var/lib/gdm3", "/var/lib/AccountsService", "/var/lib/fwupd", "/var/lib/polkit-1",
                   "/var/lib/ucf", "/var/lib/PackageKit", "/var/lib/update-notifier", "/var/lib/command-not-found", "/home/kfimage/.cache/")


def inventory(state, key, runner=subprocess.run):
    """Files MATLAB/MathWorks created or changed after the sign-in: path, size, mtime, mode, owner uid. NO contents."""
    ref = run_ssh(state, key, "cat /var/lib/kf-provisioned", runner=runner)
    require(ref.returncode == 0 and ref.stdout.strip(), "the guest has no /var/lib/kf-provisioned marker (run setup first)")
    since = W.utc(ref.stdout.strip()).timestamp() + 30   # the provisioning marker is written at its end; its own files are not "after"
    spec = json.dumps({"since": since, "roots": INVENTORY_ROOTS, "broad": INVENTORY_BROAD, "noise": list(INVENTORY_NOISE)})
    p = run_ssh(state, key, "sudo python3 -c %s" % shlex.quote(INVENTORY_PY), stdin=spec, timeout=600, runner=runner)
    require(p.returncode == 0, "inventory failed: " + (p.stderr or "")[-200:])
    return json.loads([l for l in p.stdout.splitlines() if l.startswith("[")][-1])


def format_inventory(rows):
    out = ["# path\tsize\tmtime_utc\tmode\tuid\ttype\twhy (no contents are read or printed)"]
    for r in sorted(rows, key=lambda r: r["path"]):
        out.append("%s\t%d\t%s\t%s\t%d\t%s\t%s" % (r["path"], r["size"], dt.datetime.fromtimestamp(r["mtime"], dt.timezone.utc).strftime("%FT%TZ"),
                                                      r["mode"], r["uid"], r["type"], r["why"]))
    return "\n".join(out) + "\n"


# ---------------------------------------------------------------- offline check (no account or licence line ever enters evidence)
ACCOUNT_LINE_RE = r"licen[sc]e|account|@|e-?mail|user ?name|login|sign.?in|token|activation|host ?id|mathworks\.com|password"
DEFAULT_COMMANDS = ('matlab -batch "disp(1)"', 'matlab -batch "ver"')


def filter_output(text, extra_re=None):
    """Drop every account/licence-looking line. -> (kept_lines, dropped_count). Callers keep only counts and a hash."""
    rx = re.compile(ACCOUNT_LINE_RE + ("|" + extra_re if extra_re else ""), re.I)
    kept = [l for l in text.splitlines() if not rx.search(l)]
    return kept, len(text.splitlines()) - len(kept)


def check_row(cmd, rc, output, extra_re=None):
    """One evidence row: the command, its exit code, the filtered output's sha256 and line counts. NEVER the output."""
    kept, dropped = filter_output(output, extra_re)
    return {"cmd": cmd, "rc": rc, "pass": rc == 0, "kept_lines": len(kept), "dropped_lines": dropped,
            "output_sha256": hashlib.sha256("\n".join(kept).encode()).hexdigest()}


def run_checks(state, key, commands=DEFAULT_COMMANDS, matlab=None, runner=subprocess.run, timeout=900):
    rows = []
    for c in commands:
        cmd = c.replace("matlab ", shlex.quote(matlab) + " ", 1) if matlab and c.startswith("matlab ") else c
        p = run_ssh(state, key, "bash -lc %s" % shlex.quote(cmd), timeout=timeout, runner=runner)
        rows.append(check_row(c, p.returncode, (p.stdout or "") + "\n" + (p.stderr or "")))
    return rows


# ---------------------------------------------------------------- auth-capture (OAuth loopback sign-in through the owner's own browser)
# UNVERIFIED whether the application's sign-in is a system-browser + loopback redirect at all (vs an embedded web view or a custom
# URI scheme): this helper exists so the experiment answers it. It prints the authorization URL and the forward command; the code
# travels browser -> the owner's ssh tunnel -> the application, and is never read, printed or stored here.
SECRET_PARAMS = re.compile(r"^(?!user_code$)(code|.*token.*|.*secret.*|password|assertion|.*_code)$", re.I)
SESSION_ENV_SH = (r"p=$(pgrep -u $(id -u) -x gnome-shell | head -1); "
                  r"if [ -n \"$p\" ]; then tr '\0' '\n' < /proc/$p/environ | grep -E '^(DISPLAY|XAUTHORITY|DBUS_SESSION_BUS_ADDRESS|XDG_RUNTIME_DIR)=' "
                  r"| while IFS='=' read -r k v; do printf 'export %s=%q\n' \"$k\" \"$v\"; done; fi").replace('\\"', '"')


def redact_url(url):
    """The URL with the VALUE of every secret-looking query parameter (the authorization code, any token) replaced. An
    authorization request carries none; a logged callback would. Never print a URL that was not passed through here."""
    sp = urllib.parse.urlsplit(url)
    q = urllib.parse.parse_qsl(sp.query, keep_blank_values=True)
    red = [(k, "REDACTED" if SECRET_PARAMS.match(k) else v) for k, v in q]
    frag = urllib.parse.parse_qsl(sp.fragment, keep_blank_values=True)
    fred = urllib.parse.urlencode([(k, "REDACTED" if SECRET_PARAMS.match(k) else v) for k, v in frag]) if frag and "=" in sp.fragment else sp.fragment
    return urllib.parse.urlunsplit((sp.scheme, sp.netloc, sp.path, urllib.parse.urlencode(red, safe=":/@") if red else sp.query, fred))


def parse_auth_log(text):
    out = []
    for line in text.splitlines():
        ts, _, url = line.partition("\t")
        if url.startswith(("http://", "https://")):
            out.append({"ts": ts, "url": url})
    return out


def redirect_info(url):
    """The loopback redirect of an authorization URL: {kind: localhost|127.0.0.1|::1, host, port, path} or None."""
    q = dict(urllib.parse.parse_qsl(urllib.parse.urlsplit(url).query))
    ru = q.get("redirect_uri") or q.get("redirect_url")
    if not ru:
        return None
    sp = urllib.parse.urlsplit(ru)
    host = (sp.hostname or "").lower()
    kind = {"localhost": "localhost", "127.0.0.1": "127.0.0.1", "::1": "::1"}.get(host)
    if kind is None or sp.scheme not in ("http", "https"):
        return {"kind": None, "host": host, "port": sp.port, "path": sp.path, "scheme": sp.scheme}   # custom scheme / non-loopback
    return {"kind": kind, "host": host, "port": sp.port or (443 if sp.scheme == "https" else 80), "path": sp.path, "scheme": sp.scheme}


def parse_ss(text):
    """`ss -ltnpH` -> [{addr, port, pid, comm}]. Addr is 127.0.0.1, ::1, 0.0.0.0 or ::."""
    out = []
    for line in text.splitlines():
        f = line.split()
        if len(f) < 5 or f[0] != "LISTEN":
            continue
        local = f[3]
        host, _, port = local.rpartition(":")
        host = host.strip("[]")
        if host in ("*", ""):
            host = "0.0.0.0"
        m = re.search(r'\(\("([^"]*)",pid=(\d+)', line)
        if port.isdigit():
            out.append({"addr": host, "port": int(port), "pid": int(m.group(2)) if m else None, "comm": m.group(1) if m else None})
    return out


def parse_ps(text):
    """`ps -eo pid=,ppid=` -> {pid: ppid}."""
    out = {}
    for line in text.splitlines():
        f = line.split()
        if len(f) == 2 and all(x.isdigit() for x in f):
            out[int(f[0])] = int(f[1])
    return out


def descendants(root, ps):
    seen, todo = {root}, [root]
    while todo:
        cur = todo.pop()
        for pid, ppid in ps.items():
            if ppid == cur and pid not in seen:
                seen.add(pid)
                todo.append(pid)
    return seen


def pick_listener(listeners, baseline=(), tree=None, want_port=None):
    """The loopback listener of the application: in the process tree when it is known, else new since the baseline. A listener on
    the redirect_uri's port wins. Only loopback or wildcard addresses count (a service on another interface is not a redirect)."""
    base = {(b["addr"], b["port"]) for b in baseline}
    cand = [l for l in listeners if l["addr"] in ("127.0.0.1", "::1", "0.0.0.0", "::") and (l["addr"], l["port"]) not in base
            and l["port"] >= 1024 and l["port"] != 22]
    if tree is not None:
        cand = [l for l in cand if l["pid"] in tree]
    if want_port is not None:
        hit = [l for l in cand if l["port"] == want_port]
        cand = hit or cand
    return cand[0] if cand else None


def forward_plan(state, url, listener, coordinator="<user>@<coordinator>", key_hint="<dir>/ssh/id_ed25519"):
    """-> dict with the redirect facts and the exact `ssh -L` command for the owner's PC. P is the DETECTED listener port (the
    redirect_uri port when it agrees). localhost / 127.0.0.1 / ::1 are all handled and the one seen is named."""
    info = redirect_info(url)
    notes = []
    if info is None:
        notes.append("no redirect_uri in the URL: not a loopback redirect flow (see the fallback ladder)")
    elif info["kind"] is None:
        notes.append("redirect_uri %s://%s is not a loopback address: custom scheme or remote redirect (fallback ladder)" % (info["scheme"], info["host"]))
    port = (listener or {}).get("port") or (info or {}).get("port")
    if not port:
        return {"info": info, "listener": listener, "port": None, "command": None, "notes": notes + ["no listening port detected yet"]}
    if info and info["port"] and listener and info["port"] != listener["port"]:
        notes.append("redirect_uri port %d differs from the detected listener %d: forwarding the listener" % (info["port"], listener["port"]))
    target = "[::1]" if listener and listener["addr"] == "::1" else "127.0.0.1"
    kind = info["kind"] if info else None
    bind = {"127.0.0.1": "127.0.0.1:", "::1": "[::1]:", "localhost": "", None: ""}[kind]   # localhost: ssh binds both loopbacks
    cmd = ["ssh", "-N", "-J", coordinator, "-i", key_hint, "-o", "IdentitiesOnly=yes", "-o", "HostKeyAlias=" + state["linux_ids"]["hostname"],
           "-L", "%s%d:%s:%d" % (bind, port, target, port), "-p", str(state["ids"]["net"]["ssh_port"]),
           "%s@127.0.0.1" % state["guest"]["user"]]
    notes.append("redirect host seen: %s; the application listens on %s:%d" % (kind or "none", listener["addr"] if listener else "?", port))
    return {"info": info, "listener": listener, "port": port, "command": shlex.join(cmd), "argv": cmd, "notes": notes}


PTY_LOG = "/var/tmp/kf-auth-pty.log"
TMUX_SESSION = "kfauth"
ANSI_RE = re.compile(r"\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(\x07|\x1b\\)|\x1b[()][A-Za-z0-9]|\r")
URL_RE = re.compile(r"https?://[^\s\"'<>\x00-\x1f\x7f]+")
TOKEN_RE = re.compile(r"[A-Za-z0-9_\-\.~+/=]{20,}|\b\d{6,12}\b|[\w.+-]+@[\w-]+(\.[\w-]+)+")


def strip_ansi(text):
    return ANSI_RE.sub("", text)


def urls_from_pty(text):
    """URLs a terminal program printed (ANSI stripped), in order. The terminal's own wrapping is not applied to a pipe-pane stream."""
    return [m.group(0).rstrip(".,;)") for m in URL_RE.finditer(strip_ansi(text))]


def prompt_of(text):
    """The unfinished last line of the pty stream (a prompt waiting for input), or None."""
    t = strip_ansi(text)
    if not t or t.endswith("\n"):
        return None
    last = t.rsplit("\n", 1)[-1].strip()
    return last if last and (last[-1] in ":?>" or "code" in last.lower() or "enter" in last.lower() or "paste" in last.lower()) else None


def mask_tokens(text):
    """Prompt text for the owner log: URLs shortened to scheme://host, e-mail addresses and anything code-like masked."""
    t = URL_RE.sub(lambda m: urllib.parse.urlsplit(m.group(0)).scheme + "://" + (urllib.parse.urlsplit(m.group(0)).netloc or "?") + "/<url>", text)
    return TOKEN_RE.sub("<masked>", t)


def auth_capture(state, key, cmd=None, desktop_env=False, mode="auto", timeout_s=300, grace_s=30, coordinator="<user>@<coordinator>",
                 runner=subprocess.run, sleep=time.sleep, clock=time.monotonic, out=print, poll_s=2, owner_log=None):
    """Watch a sign-in. The command runs in a tmux pane (a pty: a terminal MATLAB prompts there) with its output piped to a
    0600 file, or, with desktop_env, in the desktop session's environment. URLs come from the xdg-open/BROWSER shim AND from the
    pty output. mode: loopback (detect the listener, print the forward command), paste (print the URL and the prompt; the owner
    uses auth-paste) or auto (a listener within grace_s -> loopback, else paste). NEVER prints or stores an authorization code.
    owner_log: a callable receiving one JSON-able dict of what was seen (mode, loopback port, MASKED prompt): the PRIVATE log."""
    require(mode in ("auto", "loopback", "paste"), "mode must be auto, loopback or paste")
    snap = ("sudo cat %s 2>/dev/null; echo @@PTY@@; sudo cat %s 2>/dev/null; echo @@SS@@; ss -ltnpH 2>/dev/null; echo @@PS@@; ps -eo pid=,ppid=; "
            "echo @@PANE@@; tmux display -p -t %s '#{pane_pid}' 2>/dev/null; true" % (AUTH_LOG, PTY_LOG, TMUX_SESSION))

    def look():
        p = run_ssh(state, key, "bash -c %s" % shlex.quote(snap), timeout=60, runner=runner)
        t = p.stdout or ""
        a, _, t = t.partition("@@PTY@@\n")
        pty, _, t = t.partition("@@SS@@\n")
        b, _, t = t.partition("@@PS@@\n")
        c, _, pane = t.partition("@@PANE@@\n")
        urls = [u["url"] for u in parse_auth_log(a)] + urls_from_pty(pty)
        return urls, pty, parse_ss(b), parse_ps(c), (int(pane.strip()) if pane.strip().isdigit() else None)

    _, _, baseline, _, _ = look()
    root = None
    if cmd and desktop_env:
        launch = 'eval "$(%s)"; PATH=/usr/local/bin:$PATH BROWSER=/usr/local/bin/kf-capture-url nohup bash -c %s >/var/tmp/kf-auth-app.log 2>&1 & echo $!' % (
            SESSION_ENV_SH, shlex.quote(cmd))
        r = run_ssh(state, key, "bash -c %s" % shlex.quote(launch), runner=runner)
        require(r.returncode == 0 and r.stdout.strip().isdigit(), "could not start the command in the guest: " + (r.stderr or "")[-200:])
        root = int(r.stdout.strip())
        out("started in the desktop session environment: pid %d" % root)
    elif cmd:
        script = ("umask 077; : > %s; tmux kill-session -t %s 2>/dev/null; tmux new-session -d -s %s -x 1000 -y 50 'bash -l'; "
                  "tmux pipe-pane -o -t %s 'cat >> %s'; sleep 1; tmux load-buffer -b kfcmd -; tmux paste-buffer -d -b kfcmd -t %s; "
                  "tmux send-keys -t %s Enter; echo started" % (PTY_LOG, TMUX_SESSION, TMUX_SESSION, TMUX_SESSION, PTY_LOG, TMUX_SESSION, TMUX_SESSION))
        full = "export PATH=/usr/local/bin:$PATH BROWSER=/usr/local/bin/kf-capture-url; " + cmd
        r = run_ssh(state, key, "bash -c %s" % shlex.quote(script), stdin=full, runner=runner)
        require(r.returncode == 0 and "started" in (r.stdout or ""), "could not start the tmux session: " + (r.stderr or "")[-200:])
        out("started in guest tmux session %s (a pty; terminal output is piped to a 0600 file)" % TMUX_SESSION)
    else:
        out("watching; trigger the sign-in now (terminal: start it inside `tmux new -s %s`; desktop: click Sign in WITHOUT entering credentials)" % TMUX_SESSION)
    t0 = clock()
    seen, seen_at, last_url = None, None, None
    while clock() - t0 < timeout_s:
        urls, pty, listeners, ps, pane_pid = look()
        if urls:
            if urls[-1] == last_url and seen is None:
                seen, seen_at = urls[-1], clock()    # stable across two polls: not a half-printed URL
            last_url = urls[-1]
        if seen:
            info = redirect_info(seen)
            tree = descendants(pane_pid or root, ps) if (pane_pid or root) else None
            lst = pick_listener(listeners, baseline, tree, (info or {}).get("port"))
            prompt = prompt_of(pty)
            waited = clock() - seen_at
            decided = lst if mode != "paste" else None
            if mode == "paste" and (prompt or waited >= 5) or lst and mode != "paste" or waited >= grace_s:
                got_mode = "loopback" if decided else "paste"
                if mode == "loopback" and not decided:
                    out("note: loopback mode requested but no listener appeared")
                plan = forward_plan(state, seen, decided, coordinator)
                plan["mode"], plan["prompt"] = got_mode, prompt
                out("MODE: %s" % got_mode)
                out("AUTHORIZATION URL (open it in YOUR OWN browser; SSO happens there):")
                out("  " + redact_url(seen))
                if got_mode == "loopback":
                    out("FORWARD (run on YOUR PC BEFORE you open the URL; keep it running until the browser says it is done):")
                    out("  " + (plan["command"] or "(no port detected: see notes)"))
                    for n in plan["notes"]:
                        out("note: " + n)
                else:
                    out("PASTE: log in in your browser, copy the code it shows, then run `auth-paste` here (hidden prompt; nothing is stored).")
                    if prompt:
                        out("prompt seen in the guest: " + mask_tokens(prompt))
                out("reminder: the URL holds a one-time state and a PKCE challenge and expires; open it within a few minutes. The code never")
                out("passes through this tool's output, files or arguments. Stop any tunnel afterwards.")
                if owner_log:
                    owner_log({"mode": got_mode, "redirect_kind": (info or {}).get("kind"), "redirect_port": (info or {}).get("port"),
                               "listener": decided, "prompt_masked": mask_tokens(prompt) if prompt else None, "url_host": urllib.parse.urlsplit(seen).netloc})
                return plan
        sleep(poll_s)
    raise PropError("AUTH_TIMEOUT: no authorization URL was captured within %ds (an embedded web view or a custom scheme? see the fallback ladder)" % timeout_s)


def auth_paste(state, key, code, session=TMUX_SESSION, runner=subprocess.run, sleep=time.sleep):
    """Type `code` + Enter into the guest pane without logging it: the code goes over the ssh STDIN into a tmux buffer (never an
    argument, a file or a history); pane logging is switched off around the paste, then back on."""
    require(isinstance(code, str) and code.strip() and "\n" not in code.strip(), "empty or multi-line code")
    require(re.fullmatch(r"[A-Za-z0-9_-]{1,32}", session), "bad session name")
    script = ("set -e; S=%s; tmux pipe-pane -t $S; tmux load-buffer -b kfcode -; tmux paste-buffer -d -b kfcode -t $S; "
              "tmux send-keys -t $S Enter; sleep 3; umask 077; tmux pipe-pane -o -t $S 'cat >> %s'; echo pasted" % (session, PTY_LOG))
    r = runner(ssh_argv(state, key, "bash -c %s" % shlex.quote(script)), input=code.strip(), text=True, capture_output=True, timeout=60)
    require(r.returncode == 0 and "pasted" in (r.stdout or ""), "paste failed: " + (r.stderr or "")[-160:])
    return True


# ---------------------------------------------------------------- NVIDIA driver, staged at run time on the offline clone
NVIDIA_RUN_RE = re.compile(r"NVIDIA-Linux-x86_64-(\d+\.\d+(?:\.\d+)?)\.run")


def driver_version_from_name(path):
    m = NVIDIA_RUN_RE.fullmatch(Path(path).name)
    require(m, "unexpected installer name %s (want NVIDIA-Linux-x86_64-<version>.run)" % Path(path).name)
    return m.group(1)


def driver_install_argv(version, cc=None, jobs=8, load_module=True, remote="/var/tmp"):
    """The installer command line. Model: scripts/drivermatrix/stage_fat_guest.sh and provision_bench_tree.sh (the repo's
    measured flag set): --silent --no-x-check --no-nouveau-check --no-questions -m=kernel-open -j<n>, built with the kernel's own
    compiler. nvidia-modprobe is KEPT (an unprivileged CUDA process needs it for the device nodes). --skip-module-load only
    where the clone has no GPU (the module cannot bind there)."""
    require(re.fullmatch(r"\d+\.\d+(\.\d+)?", version), "bad driver version")
    require(cc is None or re.fullmatch(r"[A-Za-z0-9_.+-]+", cc), "bad compiler name")
    a = ["sudo"] + (["env", "CC=" + cc] if cc else []) + ["sh", "%s/nv-%s.run" % (remote, version), "--silent", "--no-x-check",
                                                           "--no-nouveau-check", "--no-questions", "-m=kernel-open", "-j%d" % jobs]
    if not load_module:
        a.append("--skip-module-load")
    return a


def driver_clock(state, run_file, created_utc=None, offset_s=0):
    """The clock for a boot that loads this installer: the recorded clock, never earlier than the installer's created_utc
    (clockrule KF_CLOCK_BEFORE_INPUT, a ClockError). Without created_utc the file mtime is the fallback and the verdict a WARN."""
    spec = "%s=%s" % (run_file, created_utc) if created_utc else str(run_file)
    return clock_for_mode(state, "run", "offline", loaded_software=[spec], offset_s=offset_s)


def install_driver(state, key, run_file, sha256=None, created_utc=None, load_module=True, runner=subprocess.run, jobs=8, log=print):
    """Stage the .run on the running offline clone, build it with the kernel's compiler, verify content. Named failures:
    DRIVER_SHA_MISMATCH, KERNEL_CC_MISSING, GUEST_DRIVER_MISMATCH. Needs the clock rule to have passed at boot (driver_clock).
    The .run is the USER's file: `sha256` is an opt-in expectation (docs/design/V3_PINS_ARE_DEPLOYMENT_DATA.md); without it nothing is refused and
    the result carries the computed sha256 and size either way."""
    ver = driver_version_from_name(run_file)
    got = W.sha256_file(run_file)
    require(sha256 is None or got == sha256, "DRIVER_SHA_MISMATCH: %s is %s, the pin says %s" % (Path(run_file).name, got, sha256))
    q = run_ssh(state, key, KERNEL_CC_CMD, runner=runner)
    cc = q.stdout.strip() or None
    chk = run_ssh(state, key, "command -v %s" % shlex.quote(cc or "gcc"), runner=runner)
    require(chk.returncode == 0, "KERNEL_CC_MISSING: the clone is offline and the image lacks %s (install it before the seal)" % (cc or "gcc"))
    s = runner(scp_argv(state, key, run_file, "/var/tmp/nv-%s.run" % ver), capture_output=True, text=True, timeout=1800)
    require(s.returncode == 0, "scp of the installer failed: " + (s.stderr or "")[-200:])
    argv = driver_install_argv(ver, cc, jobs, load_module)
    p = run_ssh(state, key, shlex.join(argv) + "; echo NVRUN_RC=$?", timeout=3600, runner=runner)
    log("installer: " + (p.stdout or "").strip().splitlines()[-1] if (p.stdout or "").strip() else "installer: no output")
    mod = run_ssh(state, key, "modinfo -F version nvidia 2>/dev/null", runner=runner).stdout.strip()
    cuda = run_ssh(state, key, r"ls /usr/lib/x86_64-linux-gnu/libcuda.so.* 2>/dev/null | grep -oE '[0-9]+\.[0-9]+(\.[0-9]+)?$' | sort -V | tail -1",
                   runner=runner).stdout.strip()
    run_ssh(state, key, "sudo rm -f /var/tmp/nv-%s.run" % ver, runner=runner)
    require(mod == ver and cuda == ver, "GUEST_DRIVER_MISMATCH: modinfo=%r libcuda=%r want %s" % (mod, cuda, ver))
    return {"version": ver, "modinfo": mod, "libcuda": cuda, "cc": cc, "sha256": got, "bytes": Path(run_file).stat().st_size}


# ---------------------------------------------------------------- package (one disk, three members)
def sha256_file(path):
    return W.sha256_file(path)


def capture_files(state, workdir):
    wd = Path(workdir)
    files = {}
    for name in STATE_FILES:
        src = wd / name
        require(src.is_file() and not src.is_symlink(), "missing required file " + name)
        data = src.read_bytes()
        files[name] = {"bytes": len(data), "sha256": hashlib.sha256(data).hexdigest(), "data_base64": base64.b64encode(data).decode()}
    state["files"] = files
    return sorted(files)


def restore_files(state, workdir):
    for name, rec in state["files"].items():
        dst = Path(workdir) / name
        dst.parent.mkdir(parents=True, exist_ok=True)
        dst.write_bytes(base64.b64decode(rec["data_base64"], validate=True))
        dst.chmod(0o600)


def pack(state_path, image, out, extra=()):
    """image.qcow2 + state.json + manifest.json (+ extra named members: packages.tsv, seal_report.txt), members STORED."""
    state = load_state(state_path)
    members = {"image.qcow2": sha256_file(image), "state.json": hashlib.sha256(Path(state_path).read_bytes()).hexdigest()}
    for p in extra:
        members[Path(p).name] = sha256_file(p)
    want = (state.get("packages") or {}).get("manifest_sha256")
    if want:
        require(members.get(P.MANIFEST_NAME) == want, "PACKAGES_MANIFEST_MISSING_OR_CHANGED: the state records %s sha256 %s; pass that file (it sits next to the sealed image)" % (P.MANIFEST_NAME, want[:12]))
    manifest = {"manifest_version": 1, "kind": "propimage-linux", "members": members, "packages": state.get("packages")}
    with zipfile.ZipFile(out, "w", compression=zipfile.ZIP_STORED, allowZip64=True) as z:
        z.write(image, "image.qcow2")
        z.write(state_path, "state.json")
        for p in extra:
            z.write(p, Path(p).name)
        z.writestr("manifest.json", json.dumps(manifest, sort_keys=True, indent=1))
    return manifest


def unpack(zpath, dest):
    dest = Path(dest)
    with zipfile.ZipFile(zpath) as z:
        names = set(z.namelist())
        require({"image.qcow2", "state.json", "manifest.json"} <= names, "unexpected zip members")
        require(all(i.compress_type == zipfile.ZIP_STORED for i in z.infolist()), "members must be STORED")
        manifest = json.loads(z.read("manifest.json"))
        require(manifest.get("kind") == "propimage-linux", "not a Linux propimage package")
        require(set(manifest["members"]) == names - {"manifest.json"}, "manifest does not list exactly the members")
        for n in names:
            require(not n.startswith("/") and ".." not in Path(n).parts, "bad member name " + n)
        dest.mkdir(parents=True, exist_ok=True)
        for name, want in manifest["members"].items():
            z.extract(name, dest)
            require(sha256_file(dest / name) == want, "sha256 mismatch for " + name)
    st = load_state(dest / "state.json")
    pk_want = (st.get("packages") or {}).get("manifest_sha256")
    if pk_want:
        require(manifest["members"].get(P.MANIFEST_NAME) == pk_want, "PACKAGES_MANIFEST_MISMATCH: the zip's %s is not the one the state records" % P.MANIFEST_NAME)
    if P.MANIFEST_NAME in manifest["members"]:
        P.validate_manifest(json.loads((dest / P.MANIFEST_NAME).read_text()))
    return st


def qemu_img_convert_cmd(src, dst, compress=True):
    return ["qemu-img", "convert", "-f", "qcow2", "-O", "qcow2"] + (["-c"] if compress else []) + [str(src), str(dst)]


def clone_cmd(image, clone):
    return ["qemu-img", "create", "-f", "qcow2", "-b", str(image), "-F", "qcow2", str(clone)]


# ---------------------------------------------------------------- orchestration (touches the box; not unit-tested beyond argv)
def paths(d):
    d = Path(d).resolve()
    return {"dir": d, "state": d / "state.json", "key": d / "ssh" / "id_ed25519", "base": d / "base" / BASE["name"],
            "setup_dir": d / "setup", "disk": d / "setup" / "disk.qcow2", "seed": d / "setup" / "seed.iso",
            "sealed": d / "sealed", "private": d / "private", "run": d / "run"}


def qemu_pid(workdir):
    try:
        pid = int(Path(workdir, "qemu.pid").read_text())
        os.kill(pid, 0)
        return pid
    except (OSError, ValueError):
        return None


def wait_exit(pid, timeout_s, state_dir=None, min_mb=MIN_AVAILABLE_MB_RUNNING, log=print):
    """Wait for a qemu to exit, killing it if the host runs out of memory. -> seconds waited."""
    t0 = time.monotonic()
    while qemu_pid_alive(pid):
        if time.monotonic() - t0 > timeout_s:
            raise PropError("QEMU_TIMEOUT after %ds" % timeout_s)
        if mem_available_mb() < min_mb:
            os.kill(pid, signal.SIGKILL)
            raise PropError("HOST_MEMORY_LOW: killed the guest (MemAvailable below %d MB)" % min_mb)
        time.sleep(3)
    return round(time.monotonic() - t0, 1)


def qemu_pid_alive(pid):
    try:
        os.kill(pid, 0)
        return Path("/proc/%d" % pid).exists() and "zombie" not in Path("/proc/%d/status" % pid).read_text().lower()
    except OSError:
        return False


def start_qemu(argv, workdir, log=print):
    p = subprocess.run(argv, capture_output=True, text=True)
    require(p.returncode == 0, "qemu failed to start: " + (p.stderr or "")[-400:])
    pid = qemu_pid(workdir)
    require(pid, "qemu started but wrote no live pidfile")
    return pid


def qmp(workdir, cmd, args=None):
    import socket
    s = socket.socket(socket.AF_UNIX)
    s.settimeout(30)
    s.connect(str(Path(workdir) / "qmp.sock"))
    f = s.makefile("rw")
    f.readline()
    f.write(json.dumps({"execute": "qmp_capabilities"}) + "\n"); f.flush(); f.readline()
    f.write(json.dumps({"execute": cmd, "arguments": args or {}}) + "\n"); f.flush()
    while True:
        r = json.loads(f.readline())
        if "return" in r or "error" in r:
            return r


def tunnel_recipe(state, vnc_display, novnc_port=6080):
    return [
        "ssh tunnel (run on YOUR machine; the box's VNC listens on 127.0.0.1 only):",
        "  ssh -N -L %d:127.0.0.1:%d <user>@<this-box>" % (5900 + vnc_display, 5900 + vnc_display),
        "  then any VNC client: vnc://127.0.0.1:%d     (no password: the tunnel is the access control)" % (5900 + vnc_display),
        "noVNC in the browser (run `propimage_linux.py novnc --dir D` on the box first, then tunnel its port):",
        "  ssh -N -L %d:127.0.0.1:%d <user>@<this-box>   then open http://127.0.0.1:%d/vnc.html?autoconnect=1" % (novnc_port, novnc_port, novnc_port),
        "harness ssh: ssh -i <dir>/ssh/id_ed25519 -p %d %s@127.0.0.1 (127.0.0.1 on the box)" % (state["ids"]["net"]["ssh_port"], state["guest"]["user"]),
    ]


def new_keypair(tmpdir):
    d = Path(tmpdir)
    subprocess.run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-C", "kfimage-propimage", "-f", str(d / "k")], check=True)
    return (d / "k").read_text(), (d / "k.pub").read_text().strip()


def cmd_init(a):
    import tempfile
    d = Path(a.dir)
    require(not (d / "state.json").exists(), "state.json already exists in %s (an identity is never regenerated)" % d)
    with tempfile.TemporaryDirectory() as t:
        kp = (Path(a.ssh_key_dir, "id_ed25519").read_text(), Path(a.ssh_key_dir, "id_ed25519.pub").read_text().strip()) if a.ssh_key_dir else new_keypair(t)
    st = init_state(a.seed, a.ram_mb, a.smp, a.disk_gb, a.nic, kp, desktop=a.desktop)
    st["created_utc"] = W.fmt(dt.datetime.now(dt.timezone.utc))
    p = paths(d)
    (d / "ssh").mkdir(parents=True, exist_ok=True)
    p["key"].write_text(st["guest"]["ssh_private_key"])
    p["key"].chmod(0o600)
    save_state(st, p["state"])
    print("initialised %s (machine-id %s hostname %s mac %s)" % (p["state"], st["linux_ids"]["machine_id"], st["linux_ids"]["hostname"], st["ids"]["nics"][0]["mac"]))


def cmd_fetch_base(a):
    p = paths(a.dir)
    p["base"].parent.mkdir(parents=True, exist_ok=True)
    if not p["base"].exists() or sha256_file(p["base"]) != BASE["sha256"]:
        subprocess.run(["curl", "-sS", "-f", "-L", "-o", str(p["base"]), BASE["url"]], check=True)
    got = sha256_file(p["base"])
    require(got == BASE["sha256"], "BASE_SHA_MISMATCH: %s" % got)
    print("base ok %s sha256 %s" % (p["base"], got))


def cmd_setup(a):
    p = paths(a.dir)
    st = load_state(p["state"])
    guard_memory()
    require(not qemu_pid(p["setup_dir"]), "a setup VM is already running (pid %s)" % qemu_pid(p["setup_dir"]))
    require(p["base"].exists() and sha256_file(p["base"]) == BASE["sha256"], "no verified base image: run fetch-base")
    (p["setup_dir"] / "firmware").mkdir(parents=True, exist_ok=True)
    fresh = not p["disk"].exists()
    t0 = time.monotonic()
    if fresh:
        subprocess.run(["qemu-img", "convert", "-O", "qcow2", str(p["base"]), str(p["disk"])], check=True)
        subprocess.run(["qemu-img", "resize", str(p["disk"]), "%dG" % st["qemu"]["disk_gb"]], check=True)
        shutil.copyfile(OVMF_VARS_TEMPLATE, p["setup_dir"] / "firmware" / "OVMF_VARS.fd")
        make_seed_iso(render_cloud_init(st), p["seed"])
    argv = build_argv(st, "setup", p["disk"], p["setup_dir"], seed_iso=str(p["seed"]) if fresh else None, vnc_display=a.vnc_display)
    (p["setup_dir"] / "argv.txt").write_text(shlex.join(argv) + "\n")
    pid = start_qemu(argv, p["setup_dir"])
    print("setup VM pid %d; ram %d MB, smp %d; clock real, network ON" % (pid, st["qemu"]["ram_mb"], st["qemu"]["smp"]))
    print("\n".join(tunnel_recipe(st, a.vnc_display)))
    key = p["key"]
    print("ssh up after %ss" % wait_ssh(st, key))
    if fresh:
        steps = [("stage A (packages)", render_stage_a(st), "STAGE_A_OK", 5400), ("stage B (config, identity)", render_stage_b(st), "STAGE_B_OK", 1800)]
        for i, (name, script, ok, tmo) in enumerate(steps):
            t = time.monotonic()
            r = run_ssh(st, key, "sudo bash -s", stdin=script, timeout=tmo)
            (p["setup_dir"] / ("provision-%d.log" % i)).write_text((r.stdout or "") + (r.stderr or ""))
            require(r.returncode == 0 and ok in (r.stdout or ""), "%s failed (log: %s)" % (name, p["setup_dir"] / ("provision-%d.log" % i)))
            print("%s ok in %.0fs" % (name, time.monotonic() - t))
            subprocess.run(ssh_argv(st, key, "sudo systemctl reboot"), capture_output=True, timeout=60)
            time.sleep(25)
            print("rebooted; ssh up after %ss" % wait_ssh(st, key))
        pk = run_ssh(st, key, "dpkg-query -W -f='${Package}\\t${Version}\\n'", timeout=120).stdout
        (p["setup_dir"] / "packages.tsv").write_text(pk)
        kernel = run_ssh(st, key, "uname -r").stdout.strip()
        st["packages"] = {"kernel": kernel, "count": len(pk.splitlines()), "sha256": hashlib.sha256(pk.encode()).hexdigest()}
        save_state(st, p["state"])
        print("provisioned in %.0fs; kernel %s; %d packages" % (time.monotonic() - t0, kernel, st["packages"]["count"]))
    run_ssh(st, key, DROPCACHES_CMD)
    rows = compare(st, run_probe(st, key), net="online")
    for r in rows:
        print("%-9s %-24s %s" % (r[1], r[0], r[2]))
    print("the setup VM keeps running; install/sign in through the console, then `seal`")


def make_ssh(st, key, runner=subprocess.run):
    """The `ssh(remote, stdin=None, timeout=600)` callable the package module takes: this module's own transport."""
    def ssh(remote, stdin=None, timeout=600):
        return run_ssh(st, key, remote, stdin=stdin, timeout=timeout, runner=runner)
    return ssh


def load_recorded_apt(setup_dir):
    f = Path(setup_dir) / P.APT_RECORD_NAME
    return json.loads(f.read_text()) if f.exists() else None


def package_manifest(st, p, vendor, ssh=None, runner=subprocess.run):
    """The guest's complete package record (dpkg, apt sources, kernel, snaps, pip/npm/gem, vendor entries, inventory counts)."""
    ssh = ssh or make_ssh(st, p["key"], runner)
    try:
        inv = P.summarize_inventory(inventory(st, p["key"], runner=runner))
    except PropError as e:
        inv = {"unavailable": str(e)[:120]}
    return P.collect_manifest(ssh, vendor, inv, st["base"], dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"), load_recorded_apt(p["setup_dir"]))


def cmd_seal(a):
    p = paths(a.dir)
    st = load_state(p["state"])
    key = p["key"]
    pats = parse_patterns(a.grep_literal, a.grep_regex, a.grep_token, a.grep_file)
    vendor = [P.parse_vendor_entry(v) for v in a.vendor_entry]   # a bad entry stops here, before the long steps
    prev = P.load_manifest(a.compare) if a.compare else None
    pid = qemu_pid(p["setup_dir"])
    require(pid, "no running setup VM")
    t0 = time.monotonic()
    gg = guest_grep(st, key, pats) if pats and not a.no_guest_grep else None
    run_ssh(st, key, "sudo bash -c 'apt-get clean; rm -rf /var/lib/apt/lists/* /var/cache/apt/*.bin /root/.cache /home/%s/.cache /home/%s/.bash_history /root/.bash_history; "
                     "truncate -s0 /var/log/*.log /var/log/syslog /var/log/auth.log 2>/dev/null; rm -f /var/tmp/kf-auth-*; tmux kill-server 2>/dev/null; rm -rf /var/log/journal/* /var/lib/cloud/instances /var/lib/cloud/instance "
                     "/var/lib/cloud/data /var/lib/cloud/sem /var/lib/cloud/seed; touch /etc/cloud/cloud-init.disabled; sync; fstrim -av'"
            % (st["guest"]["user"], st["guest"]["user"]), timeout=900)
    pk = run_ssh(st, key, "dpkg-query -W -f='${Package}\\t${Version}\\n'", timeout=120).stdout
    (p["setup_dir"] / "packages.tsv").write_text(pk)
    st["packages"] = {"kernel": run_ssh(st, key, "uname -r").stdout.strip(), "count": len(pk.splitlines()), "sha256": hashlib.sha256(pk.encode()).hexdigest()}
    left = run_ssh(st, key, "ls /var/tmp/kf-auth-* 2>/dev/null | wc -l").stdout.strip()
    require(left == "0", "AUTH_FILES_LEFT: %s /var/tmp/kf-auth-* file(s) survived the seal cleanup" % left)
    man = package_manifest(st, p, vendor)   # while the guest still runs: a refusal below leaves the setup VM usable
    for w in P.seal_warnings(man):
        print(w)
    require(man["hardening"]["hardened"] or not a.require_hardened, "NOT_HARDENED: " + "; ".join(man["hardening"]["problems"]))
    guest_utc = run_ssh(st, key, "date -u +%Y-%m-%dT%H:%M:%SZ").stdout.strip()
    W.utc(guest_utc)
    st["clock"]["last_time_at_stop_utc"] = guest_utc
    run_ssh(st, key, "sudo systemctl poweroff", timeout=60)
    print("clean stop requested; guest UTC recorded %s; stopped after %ss" % (guest_utc, wait_exit(pid, 300)))
    p["sealed"].mkdir(parents=True, exist_ok=True)
    st["packages"]["manifest_sha256"] = P.write_manifest(man, p["sealed"] / P.MANIFEST_NAME)
    print("package manifest: %d dpkg, %d snap, %d pip, %d npm, %d gem, %d vendor entr%s; hardened=%s; sha256 %s" % (
        man["counts"]["dpkg"], man["counts"]["snaps"], man["counts"]["pip"], man["counts"]["npm"], man["counts"]["gem"], man["counts"]["vendor"],
        "y" if man["counts"]["vendor"] == 1 else "ies", man["hardening"]["hardened"], st["packages"]["manifest_sha256"]))
    if prev:
        print(P.format_diff(P.diff_manifests(prev, man)), end="")
    final = p["sealed"] / "image.qcow2"
    raw = p["sealed"] / "scan.raw"
    rawres = None
    if pats:
        subprocess.run(["qemu-img", "convert", "-f", "qcow2", "-O", "raw", str(p["disk"]), str(raw)], check=True)
        rawres = scan_raw_image(raw, pats)
        raw.unlink()
    subprocess.run(qemu_img_convert_cmd(p["disk"], final), check=True)
    st["seal"] = {"image_sha256": sha256_file(final), "image_bytes": final.stat().st_size, "virtual_bytes": st["qemu"]["disk_gb"] << 30}
    capture_files(st, p["setup_dir"])
    save_state(st, p["state"])
    report, total = format_grep_report(rawres, gg, pats)
    (p["sealed"] / "seal_report.txt").write_text(report)
    shutil.copyfile(p["setup_dir"] / "packages.tsv", p["sealed"] / "packages.tsv")
    print(report, end="")
    print("sealed image %s: %d bytes, sha256 %s; seal took %.0fs" % (final, st["seal"]["image_bytes"], st["seal"]["image_sha256"], time.monotonic() - t0))
    print("READ the report above yourself before any upload: a cached sign-in is a credential")


def boot_clone(st, p, name, net="offline", offset_s=0, driver=None, gpu="none", vnc=None, log=print):
    """A throwaway overlay of the sealed image, the pinned identity, the clock rule. -> (workdir, pid, start monotonic)."""
    guard_memory()
    wd = p["run"] / name
    shutil.rmtree(wd, ignore_errors=True)
    (wd / "firmware").mkdir(parents=True)
    restore_files(st, wd)
    subprocess.run(clone_cmd(p["sealed"] / "image.qcow2", wd / "clone.qcow2"), check=True, capture_output=True)
    clock = clock_for_mode(st, "run", net, loaded_software=[driver] if driver else (), offset_s=offset_s)
    for line in clock["lines"]:
        log(line)
    argv = build_argv(st, "run", wd / "clone.qcow2", wd, net=net, clock=clock, vnc_display=vnc, gpu=gpu)
    (wd / "argv.txt").write_text(shlex.join(argv) + "\n")
    t = time.monotonic()
    return wd, start_qemu(argv, wd), t


def stop_clone(st, p, wd, pid):
    try:
        run_ssh(st, p["key"], "sudo systemctl poweroff", timeout=30)
    except Exception:
        pass
    try:
        wait_exit(pid, 120)
    except PropError:
        os.kill(pid, signal.SIGKILL)


def cmd_run(a):
    p = paths(a.dir)
    st = load_state(p["state"])
    wd, pid, t = boot_clone(st, p, a.name, net=a.net, offset_s=a.offset_days * 86400, vnc=a.vnc_display)
    try:
        print("clone %s pid %d; ssh up after %ss" % (a.name, pid, wait_ssh(st, p["key"])))
        rows = compare(st, run_probe(st, p["key"]), elapsed_s=time.monotonic() - t, net=a.net)
        for r in rows:
            print("%-9s %-24s %s" % (r[1], r[0], r[2]))
        bad = [r for r in rows if r[1] == "MISMATCH"]
        print("RESULT %s" % ("MATCH" if not bad else "MISMATCH (%d)" % len(bad)))
        return 0 if not bad else 1
    finally:
        if not a.keep:
            stop_clone(st, p, wd, pid)


def cmd_offline_check(a):
    p = paths(a.dir)
    st = load_state(p["state"])
    wd, pid, t = boot_clone(st, p, a.name, offset_s=int(a.advance_days * 86400))
    result = {"image_sha256": (st.get("seal") or {}).get("image_sha256"), "advance_days": a.advance_days, "rounds": []}
    try:
        wait_ssh(st, p["key"])
        for rnd in range(a.reboots + 1):
            if rnd:
                subprocess.run(ssh_argv(st, p["key"], "sudo systemctl reboot"), capture_output=True, timeout=60)
                time.sleep(25)
                wait_ssh(st, p["key"])
            guest = run_probe(st, p["key"])
            result["rounds"].append({"round": rnd, "guest_utc": guest["date_utc"], "net": bool(guest["internet"] or guest["ping"]),
                                     "checks": run_checks(st, p["key"], a.cmd or DEFAULT_COMMANDS, a.matlab)})
        ok = all(c["pass"] for r in result["rounds"] for c in r["checks"]) and not any(r["net"] for r in result["rounds"])
        result["result"] = "PASS" if ok else "FAIL"
        print(json.dumps(result, indent=1))
        return 0 if ok else 1
    finally:
        stop_clone(st, p, wd, pid)


def cmd_inventory(a):
    p = paths(a.dir)
    st = load_state(p["state"])
    text = format_inventory(inventory(st, p["key"]))
    p["private"].mkdir(parents=True, exist_ok=True)
    out = p["private"] / ("inventory-%s.tsv" % dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ"))
    out.write_text(text)
    out.chmod(0o600)
    print(text, end="")
    print("saved (owner only, never commit): %s" % out)


def cmd_install_driver(a):
    p = paths(a.dir)
    st = load_state(p["state"])
    clock = driver_clock(st, a.run, a.created_utc)  # refuses with KF_CLOCK_BEFORE_INPUT before anything boots
    for line in clock["lines"]:
        print(line)
    print(json.dumps(install_driver(st, p["key"], a.run, a.sha256, a.created_utc, load_module=not a.skip_module_load)))


CLONE_NAME_RE = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,39}")
OFFLINE_ROWS_IGNORED = ("kernel_headers", "driver_build_tools", "cloud_init_disabled", "cpu_no_forbidden_flags")


def resolve_clone(p, name, runner=subprocess.run):
    """A RUNNING throwaway clone of this directory: <dir>/run/<name>/clone.qcow2, an overlay whose backing file is the sealed image.
    Refuses a name that leaves <dir>/run, one that resolves into the sealed directory, a file that is the sealed image itself or is not
    an overlay of it, and a clone that is not running. -> (workdir, pid)."""
    require(CLONE_NAME_RE.fullmatch(name or ""), "CLONE_NAME_REFUSED: %r" % (name,))
    sealed_dir, sealed_img = p["sealed"].resolve(), (p["sealed"] / "image.qcow2").resolve()
    wd = (p["run"] / name).resolve()
    require(wd.parent == p["run"].resolve() and wd != sealed_dir and sealed_dir not in wd.parents, "CLONE_REFUSED_SEALED: %s is not a clone directory" % name)
    img = wd / "clone.qcow2"
    require(img.is_file() and img.resolve() != sealed_img, "NOT_A_CLONE: no clone overlay in %s (the sealed image is never a target)" % wd)
    info = runner(["qemu-img", "info", "-U", "--output=json", str(img)], capture_output=True, text=True)
    require(info.returncode == 0, "NOT_A_CLONE: qemu-img info failed: " + (info.stderr or "")[-120:])
    j = json.loads(info.stdout)
    backing = j.get("full-backing-filename") or j.get("backing-filename")
    require(backing and Path(backing).resolve() == sealed_img, "NOT_AN_OVERLAY_OF_THE_SEALED_IMAGE: %s" % img)
    pid = qemu_pid(wd)
    require(pid, "CLONE_NOT_RUNNING: no live qemu in %s (start one with `run --keep`)" % wd)
    return wd, pid


def clone_preflight(st, p, name, runner=subprocess.run, now=time.time):
    """The same identity, network and clock checks `run` makes, against the running clone: refuses the setup VM (it has a network),
    a clone with another identity, and a clock outside the recorded window. -> (workdir, rows)."""
    wd, pid = resolve_clone(p, name, runner)
    elapsed = max(0.0, now() - (wd / "qemu.pid").stat().st_mtime)
    rows = compare(st, run_probe(st, p["key"], runner=runner), elapsed_s=elapsed, net="offline")
    bad = [r for r in rows if r[1] == "MISMATCH" and r[0] not in OFFLINE_ROWS_IGNORED]
    require(not bad, "CLONE_CHECK_FAILED: " + "; ".join("%s (%s)" % (r[0], r[2]) for r in bad))
    return wd, rows


def cmd_harden_offline(a):
    p = paths(a.dir)
    st = load_state(p["state"])
    require(qemu_pid(p["setup_dir"]), "no running setup VM (harden-offline is the last step before the seal, on the setup VM)")
    P.harden_offline(make_ssh(st, p["key"]), p["setup_dir"])
    print("HARDENED: the guest cannot reach any network. Do the online work (sign-in, installs) BEFORE this step; `seal` records the state.")


def cmd_unharden(a):
    p = paths(a.dir)
    st = load_state(p["state"])
    wd, pid = resolve_clone(p, a.clone)
    res = P.unharden(make_ssh(st, p["key"]))
    print("unhardened throwaway clone %s (runtime ruleset flushed: %d table(s) left; the sealed image is untouched)" % (a.clone, res["tables"]))


def cmd_inject(a):
    p = paths(a.dir)
    st = load_state(p["state"])
    wd, rows = clone_preflight(st, p, a.name)
    user = st["guest"]["user"]
    ssh = make_ssh(st, p["key"])
    argv_for = lambda remote: ssh_argv(st, p["key"], remote)
    if a.wheelhouse:
        require(a.lock and not a.src, "give --wheelhouse DIR with --lock FILE and no --src")
        res = P.inject_wheelhouse(ssh, argv_for, a.wheelhouse, a.lock, a.dest or P.INJECT_EXTRA_ROOT + "/pyenv", user, a.max_gb, a.replace)
    else:
        require(a.src and a.dest and not a.lock, "give --src and --dest (or --wheelhouse with --lock)")
        res = P.inject_path(argv_for, a.src, a.dest, user, a.max_gb, a.replace)
    print("tar stream sha256 %s (%d bytes) -> %s on clone %s" % (res["sha256"], res["bytes"], res["dest"], a.name))


def cmd_diff_packages(a):
    d = P.diff_manifests(P.load_manifest(a.a), P.load_manifest(a.b))
    print(P.format_diff(d), end="")
    return 1 if d else 0


def private_logger(p, name="auth-capture.log"):
    """Append JSON lines to the owner's PRIVATE log (mode 0600, under <dir>/private; never committed, never evidence)."""
    p["private"].mkdir(parents=True, exist_ok=True)
    path = p["private"] / name

    def log(rec):
        rec = dict(rec, ts=dt.datetime.now(dt.timezone.utc).strftime("%FT%TZ"))
        fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o600)
        with os.fdopen(fd, "a") as f:
            f.write(json.dumps(rec) + "\n")
    return log


def cmd_auth_capture(a):
    p = paths(a.dir)
    st = load_state(p["state"])
    auth_capture(st, p["key"], a.cmd, a.desktop_env, a.mode, a.timeout, coordinator=a.coordinator or "<user>@<coordinator>",
                 owner_log=private_logger(p))


def cmd_auth_paste(a):
    p = paths(a.dir)
    st = load_state(p["state"])
    code = getpass.getpass("authorization code (hidden; sent over ssh stdin, never stored): ")
    auth_paste(st, p["key"], code, a.session)
    print("sent")


def cmd_auth_selftest(a):
    """End to end on a REAL clone with the stub app (no MathWorks traffic, no credentials): loopback on localhost, 127.0.0.1 and
    ::1 through an `ssh -L` from this box (the stand-in for the owner's PC), and the paste-code flow. Prints PASS/FAIL rows."""
    import urllib.request
    p = paths(a.dir)
    st = load_state(p["state"])
    stub = HERE / "auth_stub_app.py"
    wd, pid, t = boot_clone(st, p, a.name)
    rows = []
    try:
        wait_ssh(st, p["key"])
        s = subprocess.run(scp_argv(st, p["key"], stub, "/var/tmp/kf-auth-stub.py"), capture_output=True, text=True)
        require(s.returncode == 0, "scp of the stub failed")
        for host, bind in (("localhost", "127.0.0.1"), ("127.0.0.1", "127.0.0.1"), ("[::1]", "::1")):
            run_ssh(st, p["key"], "rm -f /var/tmp/kf-auth-urls.log /var/tmp/kf-auth-stub.result")
            logged = []
            cmd = "python3 /var/tmp/kf-auth-stub.py --host %s --bind %s --result /var/tmp/kf-auth-stub.result" % (host, bind)
            plan = auth_capture(st, p["key"], cmd, mode="loopback", timeout_s=120, owner_log=logged.append, out=lambda *x: None)
            argv = plan["argv"]
            fwd = ["ssh", "-N", "-i", str(p["key"]), "-o", "StrictHostKeyChecking=no", "-o", "UserKnownHostsFile=/dev/null", "-o", "ExitOnForwardFailure=yes",
                   "-L", argv[argv.index("-L") + 1], "-p", str(st["ids"]["net"]["ssh_port"]), "%s@127.0.0.1" % st["guest"]["user"]]
            f = subprocess.Popen(fwd, stderr=subprocess.DEVNULL)
            try:
                time.sleep(2)
                url = "http://%s:%d/callback?code=FAKE-SELFTEST&state=stubstate123" % (host, plan["port"])
                urllib.request.urlopen(url, timeout=10).read()
                time.sleep(1)
                res = json.loads(run_ssh(st, p["key"], "cat /var/tmp/kf-auth-stub.result").stdout or "{}")
            finally:
                f.terminate()
            ok = plan["mode"] == "loopback" and res.get("callback") == 1 and res.get("state_ok") == 1
            rows.append(("loopback-" + host, "PASS" if ok else "FAIL", "port=%s redirect_kind=%s forward=-L %s code_len=%s" % (
                plan["port"], plan["info"]["kind"], argv[argv.index("-L") + 1], res.get("code_len"))))
        run_ssh(st, p["key"], "rm -f /var/tmp/kf-auth-pty.log /var/tmp/kf-auth-stub.result; tmux kill-session -t %s 2>/dev/null" % TMUX_SESSION)
        plan = auth_capture(st, p["key"], "python3 /var/tmp/kf-auth-stub.py --mode terminal --result /var/tmp/kf-auth-stub.result", mode="paste",
                            timeout_s=120, out=lambda *x: None)
        code = "TESTCODE-" + "x" * 11
        auth_paste(st, p["key"], code)
        time.sleep(2)
        res = json.loads(run_ssh(st, p["key"], "cat /var/tmp/kf-auth-stub.result").stdout or "{}")
        leak = run_ssh(st, p["key"], "sudo grep -c %s /var/tmp/kf-auth-pty.log; true" % shlex.quote(code)).stdout.strip()
        ok = plan["mode"] == "paste" and res.get("code_len") == len(code) and res.get("code_sha8") == hashlib.sha256(code.encode()).hexdigest()[:8]
        rows.append(("paste-code", "PASS" if ok else "FAIL", "code_len=%s pty_log_contains_code=%s" % (res.get("code_len"), leak)))
        rows.append(("paste-code-not-in-pty-log", "PASS" if leak == "0" else "FAIL", "count=%s" % leak))
    finally:
        stop_clone(st, p, wd, pid)
    for r in rows:
        print("%-5s %-26s %s" % (r[1], r[0], r[2]))
    return 0 if all(r[1] == "PASS" for r in rows) else 1


def cmd_scan_image(a):
    """Independent reader: the credential grep over a sealed qcow2 (or the image member of a packed zip), counts only."""
    import tempfile
    pats = parse_patterns(a.grep_literal, a.grep_regex, a.grep_token, a.grep_file)
    require(pats, "give at least one --grep-* pattern")
    src = Path(a.image)
    with tempfile.TemporaryDirectory(dir=a.tmpdir) as t:
        if src.suffix == ".zip":
            with zipfile.ZipFile(src) as z:
                z.extract("image.qcow2", t)
            src = Path(t) / "image.qcow2"
        raw = Path(t) / "scan.raw"
        subprocess.run(["qemu-img", "convert", "-f", "qcow2", "-O", "raw", str(src), str(raw)], check=True)
        res = scan_raw_image(raw, pats)
    report, total = format_grep_report(res, None, pats)
    print(report, end="")
    return 0 if total == 0 else 1


def novnc_requirements():
    """The pip requirements file, hash-pinned (pip --require-hashes): websockify's pure-python wheel from PyPI."""
    return "websockify==%s \\\n    --hash=sha256:%s\n" % (NOVNC["websockify"], NOVNC["websockify_wheel_sha256"])


def cmd_novnc(a):
    """websockify in a venv under the work dir, serving the pinned noVNC release; bound to 127.0.0.1 only. UNVERIFIED on a
    clean box until the owner runs it: this box's own run is recorded in docs/design/V3_PROPIMAGE_LINUX.md section 12."""
    import tarfile
    import urllib.request
    p = paths(a.dir)
    venv, web = p["dir"] / "novnc-venv", p["dir"] / "novnc-web"
    if not (venv / "bin" / "websockify").exists():
        subprocess.run([sys.executable, "-m", "venv", str(venv)], check=True)
        req = p["dir"] / "novnc-requirements.txt"
        req.write_text(novnc_requirements())
        subprocess.run([str(venv / "bin" / "pip"), "install", "--no-deps", "--require-hashes", "-r", str(req)], check=True)
    if not (web / "vnc.html").exists():
        tgz = p["dir"] / "novnc.tar.gz"
        urllib.request.urlretrieve(NOVNC["novnc_url"], tgz)
        require(sha256_file(tgz) == NOVNC["novnc_sha256"], "NOVNC_SHA_MISMATCH")
        with tarfile.open(tgz) as t:
            top = t.getnames()[0].split("/")[0]
            t.extractall(p["dir"] / "novnc-tmp", filter="data")
        shutil.move(str(p["dir"] / "novnc-tmp" / top), str(web))
        shutil.rmtree(p["dir"] / "novnc-tmp")
        tgz.unlink()
    cmd = [str(venv / "bin" / "websockify"), "--web", str(web), "127.0.0.1:%d" % a.port, "127.0.0.1:%d" % (5900 + a.vnc_display)]
    print("running (127.0.0.1 only): " + shlex.join(cmd))
    sys.stdout.flush()
    os.execv(cmd[0], cmd)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="verb", required=True)
    s = sub.add_parser("ids"); s.add_argument("seed")
    s = sub.add_parser("init"); s.add_argument("--seed", required=True); s.add_argument("--dir", required=True)
    s.add_argument("--ram-mb", type=int, default=8192); s.add_argument("--smp", type=int, default=4)
    s.add_argument("--disk-gb", type=int, default=60); s.add_argument("--nic", default="virtio-net-pci", choices=NIC_MODELS)
    s.add_argument("--ssh-key-dir")
    s.add_argument("--desktop", action="store_true", help="install the Ubuntu desktop + GDM autologin (GUI rows, noVNC); default: terminal-only")
    s = sub.add_parser("fetch-base"); s.add_argument("--dir", required=True)
    s = sub.add_parser("argv"); s.add_argument("state"); s.add_argument("mode", choices=MODES)
    s.add_argument("--disk", required=True); s.add_argument("--workdir", required=True)
    s.add_argument("--net", choices=("online", "offline")); s.add_argument("--vnc-display", type=int)
    s.add_argument("--offset-days", type=float, default=0); s.add_argument("--gpu", default="none"); s.add_argument("--driver-version")
    s = sub.add_parser("setup"); s.add_argument("--dir", required=True); s.add_argument("--vnc-display", type=int, default=7)
    s = sub.add_parser("novnc"); s.add_argument("--dir", required=True); s.add_argument("--port", type=int, default=6080)
    s.add_argument("--vnc-display", type=int, default=7)
    s = sub.add_parser("inventory"); s.add_argument("--dir", required=True)
    s = sub.add_parser("seal"); s.add_argument("--dir", required=True)
    s.add_argument("--grep-literal", action="append", default=[]); s.add_argument("--grep-regex", action="append", default=[], help="e-mail pattern list")
    s.add_argument("--grep-token", action="append", default=[], help="token-like pattern list"); s.add_argument("--grep-file")
    s.add_argument("--no-guest-grep", action="store_true")
    s.add_argument("--vendor-entry", action="append", default=[], metavar="'k=v ...'",
                   help="a vendor install, free-form key=value (name= required), repeatable: name=MATLAB release=R2026b products=MATLAB,Parallel_Computing_Toolbox "
                        "installer_sha256=<64 hex> installed_via=<installer|mpm>; keys that look like secrets are refused")
    s.add_argument("--compare", metavar="PREV", help="a previous packages.manifest.json (or packed zip): print what changed")
    s.add_argument("--require-hardened", action="store_true", help="refuse to seal unless harden-offline's state is live (default: WARN)")
    s = sub.add_parser("harden-offline", help="setup VM, before the seal: persistent nftables default drop (no apt, no mirrors), apt timers masked, sources removed"); s.add_argument("--dir", required=True)
    s = sub.add_parser("unharden", help="throwaway clone only: flush the ruleset (online logout step); refuses the sealed image"); s.add_argument("--dir", required=True)
    s.add_argument("--clone", default="clone1", help="name of the RUNNING clone under <dir>/run (a `run --keep` clone)")
    s = sub.add_parser("diff-packages", help="what changed between two seals"); s.add_argument("a", help="packages.manifest.json or packed zip"); s.add_argument("b")
    s = sub.add_parser("inject", help="copy a file or directory, or a wheelhouse, into a RUNNING offline clone over ssh (tar, no scp)")
    s.add_argument("--dir", required=True); s.add_argument("--name", default="clone1", help="the running clone under <dir>/run")
    s.add_argument("--src"); s.add_argument("--dest", help="guest path: relative = under /home/kfimage; absolute only under /home/kfimage or /opt/kf")
    s.add_argument("--wheelhouse", help="host dir of wheels (kfml build-wheels / pip download -r LOCK); installed inside the clone, --no-index --require-hashes")
    s.add_argument("--lock", help="the hashed requirements lock the wheelhouse was built from"); s.add_argument("--max-gb", type=float, default=P.DEFAULT_MAX_INJECT_GB)
    s.add_argument("--replace", action="store_true", help="remove an existing destination first")
    s = sub.add_parser("scan-image"); s.add_argument("image", help="a sealed qcow2 or a packed zip"); s.add_argument("--tmpdir")
    s.add_argument("--grep-literal", action="append", default=[]); s.add_argument("--grep-regex", action="append", default=[])
    s.add_argument("--grep-token", action="append", default=[]); s.add_argument("--grep-file")
    s = sub.add_parser("run"); s.add_argument("--dir", required=True); s.add_argument("--name", default="clone1")
    s.add_argument("--net", choices=("online", "offline"), default="offline"); s.add_argument("--offset-days", type=float, default=0)
    s.add_argument("--vnc-display", type=int); s.add_argument("--keep", action="store_true")
    s = sub.add_parser("offline-check"); s.add_argument("--dir", required=True); s.add_argument("--name", default="check1")
    s.add_argument("--reboots", type=int, default=0, help="E1: reboot the clone this many times and re-check")
    s.add_argument("--advance-days", type=float, default=0, help="E3: throwaway clone only; start the recorded clock this many days later")
    s.add_argument("--matlab", help="path of the matlab binary in the guest"); s.add_argument("--cmd", action="append")
    s = sub.add_parser("auth-capture"); s.add_argument("--dir", required=True); s.add_argument("--cmd", help="command to run in a guest tmux pane (omit: just watch)")
    s.add_argument("--mode", default="auto", choices=("auto", "loopback", "paste")); s.add_argument("--timeout", type=int, default=300)
    s.add_argument("--desktop-env", action="store_true", help="run --cmd in the desktop session's environment (needs a --desktop image)")
    s.add_argument("--coordinator", help="USER@HOST of this box as the owner's PC reaches it (ssh jump host)")
    s = sub.add_parser("auth-paste"); s.add_argument("--dir", required=True); s.add_argument("--session", default=TMUX_SESSION)
    s = sub.add_parser("auth-selftest"); s.add_argument("--dir", required=True); s.add_argument("--name", default="auth1")
    s = sub.add_parser("install-driver"); s.add_argument("--dir", required=True); s.add_argument("--run", required=True)
    s.add_argument("--sha256", help="optional: refuse unless the .run has this sha256"); s.add_argument("--created-utc"); s.add_argument("--skip-module-load", action="store_true")
    s = sub.add_parser("pack"); s.add_argument("state"); s.add_argument("image"); s.add_argument("--out", required=True)
    s.add_argument("--extra", action="append", default=[], help="more members, e.g. packages.tsv and seal_report.txt")
    s = sub.add_parser("unpack"); s.add_argument("zip"); s.add_argument("--dir", required=True); s.add_argument("--workdir")
    a = ap.parse_args(argv)
    try:
        if a.verb == "ids":
            print(json.dumps({"ids": W.derive_ids(a.seed), "linux_ids": derive_linux_ids(a.seed)}, indent=2))
        elif a.verb == "init":
            cmd_init(a)
        elif a.verb == "fetch-base":
            cmd_fetch_base(a)
        elif a.verb == "argv":
            st = load_state(a.state)
            net = a.net or ("online" if a.mode == "setup" else "offline")
            clock = clock_for_mode(st, a.mode, net, offset_s=int(a.offset_days * 86400))
            print(shlex.join(build_argv(st, a.mode, a.disk, a.workdir, net=net, clock=clock, vnc_display=a.vnc_display, gpu=a.gpu,
                                        driver_version=a.driver_version)))
        elif a.verb == "setup":
            cmd_setup(a)
        elif a.verb == "novnc":
            cmd_novnc(a)
        elif a.verb == "inventory":
            cmd_inventory(a)
        elif a.verb == "seal":
            cmd_seal(a)
        elif a.verb == "harden-offline":
            cmd_harden_offline(a)
        elif a.verb == "unharden":
            cmd_unharden(a)
        elif a.verb == "inject":
            cmd_inject(a)
        elif a.verb == "diff-packages":
            return cmd_diff_packages(a)
        elif a.verb == "scan-image":
            return cmd_scan_image(a)
        elif a.verb == "run":
            return cmd_run(a)
        elif a.verb == "offline-check":
            return cmd_offline_check(a)
        elif a.verb == "install-driver":
            cmd_install_driver(a)
        elif a.verb == "auth-capture":
            cmd_auth_capture(a)
        elif a.verb == "auth-paste":
            cmd_auth_paste(a)
        elif a.verb == "auth-selftest":
            return cmd_auth_selftest(a)
        elif a.verb == "pack":
            extra = list(a.extra)
            side = Path(a.image).parent / P.MANIFEST_NAME   # the seal's manifest sits next to the image and always travels with it
            if side.is_file() and P.MANIFEST_NAME not in [Path(x).name for x in extra]:
                extra.append(str(side))
            elif not side.is_file() and P.MANIFEST_NAME not in [Path(x).name for x in extra]:
                print("WARN: no %s next to the image: the zip carries no package record" % P.MANIFEST_NAME, file=sys.stderr)
            print(json.dumps(pack(a.state, a.image, a.out, extra)))
        else:
            st = unpack(a.zip, a.dir)
            if a.workdir:
                restore_files(st, a.workdir)
            print("unpacked and verified")
    except ClockError as e:
        print("KF_CLOCK_REFUSED reason=%s %s" % (e.reason, e.detail), file=sys.stderr)
        return 2
    except (PropError, P.PkgError, OSError, KeyError, json.JSONDecodeError, subprocess.SubprocessError) as e:
        print("propimage_linux: %s" % e, file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
