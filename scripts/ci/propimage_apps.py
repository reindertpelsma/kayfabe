#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The `propimage_apps` payload stage: the proprietary Windows image's seven apps, offline, on a GPU box (stdlib only).

STATUS: DESIGN-ONLY, 2026-11-15 (OWNER_RULINGS 2026-11-15: the apps of propimage_check.py must PASS in the release full run,
per GPU family). GPU-free tests only (test_propimage_apps.py); NOT run on a box, and it cannot PASS today: the image,
the signed GOP and the swtpm sources have no recorded sha256 (scripts/ci/propimage_apps_pins.json, all null), and the
harness never drives the Wukong benchmark (propimage_check.py grades it INCONCLUSIVE, `wukong_drive.py` of draft PR #417
is research tooling outside the harness). Both end the stage with a NAMED reason, never a pass.

    propimage_apps.py run --sha SHA --checkout ROOT --prefix DIR --out FILE --urls FILE [--bench DIR] [--deadline-epoch N]
    propimage_apps.py plan [--sha SHA] [--cpuinfo FILE]        preflight rows and the boot plan; touches nothing
    propimage_apps.py manifest --out FILE [--swtpm-build]      the download manifest (kayfabe.download-manifest/1); refuses unpinned items

PINS ARE DEPLOYMENT DATA (owner, 2026-11-17; docs/design/V3_PINS_ARE_DEPLOYMENT_DATA.md). `run` without an image flag is the DEPLOYMENT stage:
it fetches the image and the GOP named in the pins file (the repository's own, or YOUR file through `--pins FILE`) and refuses anything that is
not the pinned object. `run --image ...` / `--propimage ...` / `--images-file` is the generic runner: the sha256 in a source is an identity pin
that only a strict run (`--strict`, the Worker) enforces; a standalone run records the computed sha256 and size and `--expect-sha256 NAME=HASH`
opts in. The pins of the downloaded third-party tools (the swtpm and libtpms sources) are supply-chain integrity and hold in every mode.

What `run` does, in order (every step is a refusal point with a name, in the record):
  1. CPU vendor (NOT_RUN CPU_VENDOR on anything but AMD: the image runs the fixed CPU profile `kayfabe-amd-v1`, AuthenticAMD,
     propimage.py CPU_PROFILE; docs/design/V3_RELEASE_MATRIX.md), CPU features (CPU_PROFILE_UNSUPPORTED), GPU family
     (GPU_FAMILY), pins (PIN_UNSET:<role>), the kf3 QEMU of this revision, the URL file, free disk, swtpm.
  2. Prefetch through scripts/ci/box_downloader.py (the manifest comes from the pins, the presigned-URL file from the bootstrap
     layer; this stage never sees the run token). Image and GOP are `signed` sources only (private object); the swtpm sources are
     public mirrors, fetched only when the box's swtpm is older than 0.10.
  3. swtpm >= 0.10 (built under /opt/tpm when the box's is older; libtpms must match: the image's TPM state is not readable by
     0.6.3, traces/hwrun_20261113/README.md), the display broker (`make -C broker`), the verified unpack (propimage_check.load_image:
     both members' sha256 and the seed), then the archive is deleted.
  4. Boots of propimage_check.py, each on a fresh throwaway overlay, offline (slirp restrict=on, proved from the argv the harness
     wrote), grouped so that the sum of the apps' own timeouts stays under the Windows evaluation timer (about 55 minutes per
     boot); Wukong always last and alone, with the guest-visible framebuffer at 8192 MiB (4096 MiB TDR'd in 11 of 12 driven runs,
     traces/pcnight_20261114/README.md).
  5. The record: per app verdict, evidence kind, file names and sha256, score, guest TDR/LiveKernelEvent counts and the host Xid
     lines inside that app's window of the host kernel log. PASS needs all seven apps PASS with the expected evidence kind.
Cleanup removes the archive, the unpacked image, every overlay and the work directories, always (the image is never copied back
or redistributed); only the record and a small allow-list of text evidence stay.

The record is printed as one `KF_PROPIMAGE_APPS_RESULT {json}` line and written to --out; scripts/ci/vast_report.py validates it.
Extension point for another CPU vendor: ALLOWED_CPU_VENDORS and cpu_rows(). Allowing Intel also needs an Intel CPU profile in
propimage.py (its validate_state pins AuthenticAMD) and an image whose state names it: not a flag.
"""
import argparse
import dataclasses
import hashlib
import importlib.util
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import threading
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
SCHEMA = 1
PINS_SCHEMA = "kayfabe.propimage-pins/1"
PINS_PATH = HERE / "propimage_apps_pins.json"
WINDOWS_DIR = ROOT / "scripts/bench/windows"

# The seven apps of propimage_check.CHECKS in run order. Wukong is last: a GPU watchdog event there can wedge the guest.
APPS = ("superposition", "3dmark", "solidworks", "resolve", "edrawings", "edge", "wukong")
# the evidence kind a PASS must carry (propimage_check.verdict: FILE = a saved native file graded on the host, UIA = read through
# UI Automation); a PASS with another kind is not a PASS
EXPECTED_KIND = {"superposition": "FILE", "3dmark": "FILE", "solidworks": "FILE", "resolve": "FILE", "edrawings": "UIA",
                 "edge": "UIA", "wukong": "FILE"}
APP_VERDICTS = ("PASS", "FAIL", "INCONCLUSIVE", "LAUNCH_ONLY", "HARNESS", "NOT_RUN", "MISSING")
STAGE_VERDICTS = ("PASS", "FAIL", "RETRY", "REFUSED", "NOT_RUN")

# CPU rule (release matrix: proprietary stages only on AMD large boxes). EXTENSION POINT, see the module docstring.
ALLOWED_CPU_VENDORS = ("AuthenticAMD",)
# compute capability x100 (vast_payload.hardware()) -> die family. Datacenter parts (8.0, 9.0, 10.0) have no desktop driver
# path for the image's 580.88 desktop driver (INFERRED) and are refused. Only Ada is MEASURED with this image (RTX 4070, 4090).
FAMILY_BY_CC = {750: "turing", 860: "ampere", 890: "ada", 1200: "blackwell"}
MEASURED_FAMILIES = ("ada",)
IMAGE_ZIP_ESTIMATE = 47 * 1024 ** 3        # about 47 GiB packed (docs/howto/recreate-the-proprietary-image.md, MEASURED 2026-11-12)
DISK_HEADROOM = 40 * 1024 ** 3             # DESIGN: overlays, the swtpm build, evidence; never measured on a box
BOOT_BUDGET_S = 2700                       # DESIGN: ~55 min evaluation timer minus ~10 min of boot, ssh, power and gpu repair
BOOT_SLACK_S = 1800                        # DESIGN: wall limit of one harness process = its apps' timeouts + this
WUKONG_FB_MB = 8192                        # MEASURED 2026-11-14: 5 of 5 benchmark runs at 8192, 11 of 12 TDR at 4096
SWTPM_PREFIX = "/opt/tpm"
DEFAULT_PREFIX = "/workspace/kf-propimage"
DEFAULT_URLS = "/run/kf-ci/download-urls.json"   # DESIGN: the bootstrap layer (box_downloader_worker.py resolve) must write it here
MAX_EVIDENCE_FILES = 24
MAX_XID_LINES = 8
MAX_RECORD_BYTES = 24576
EVIDENCE_COPY = ("verdict.json", "result.json", "system_events.txt", "nvidia_smi_memory.txt")   # text only; never the image, no PNG
NAME_RE = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]{0,99}")
SHA_RE = re.compile(r"[0-9a-f]{40}")
HEX64 = re.compile(r"[0-9a-f]{64}")
XID_RE = re.compile(r"NVRM: Xid")
RUN_ID = re.compile(r"[0-9a-f]{32}")
SWTPM_DEPS = ("build-essential", "automake", "autoconf", "libtool", "pkg-config", "libssl-dev", "libgmp-dev", "libtasn1-6-dev",
              "libjson-glib-dev", "libgnutls28-dev", "libseccomp-dev", "libglib2.0-dev", "gawk", "socat",
              # 2026-11-16, MEASURED on a real Ubuntu 22.04 KVM box (image-B agent): swtpm's configure stops with "expect is required";
              # the other packages are what that build had installed when it completed (only `expect` was shown to be needed)
              "expect", "gnutls-bin", "libfuse-dev", "libtool-bin", "net-tools", "python3-cryptography", "tpm2-tools")


class Refused(Exception):
    """A named refusal: `reason` is an upper-case token (optionally `:detail`), never a value from the network or the guest."""

    def __init__(self, reason, detail=""):
        super().__init__(reason)
        self.reason, self.detail = reason, detail


def clean(text, limit=200):
    """Text that came from a guest, a log or a tool: printable ASCII only, bounded. Never forwarded raw."""
    return re.sub(r"[^\x20-\x7e]", "?", str(text))[:limit]


def load_module(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def harness(root=ROOT):
    """propimage_check as a module (CHECKS timeouts, load_image, require_swtpm, the image library PI)."""
    return load_module(Path(root) / "scripts/bench/windows/propimage_check.py", "kf_propimage_check_for_stage")


# ---------------------------------------------------------------- pins and the download manifest
def load_pins(path=PINS_PATH):
    """Strict: exact keys, unique roles and names, null or valid sha256 and size, safe relative dest."""
    doc = json.loads(Path(path).read_text(), object_pairs_hook=_no_duplicates)
    if type(doc) is not dict or doc.get("schema") != PINS_SCHEMA or set(doc) != {"schema", "status", "items"}:
        raise ValueError("pins: schema")
    items = doc["items"]
    if type(items) is not list or not 2 <= len(items) <= 16:
        raise ValueError("pins: items")
    roles, names, dests = set(), set(), set()
    for it in items:
        keys = {"role", "name", "dest", "sha256", "size", "needed_by", "recorded_in"}
        if type(it) is not dict or not keys <= set(it) <= keys | {"ref", "mirror"}:
            raise ValueError("pins: keys")
        if not re.fullmatch(r"[a-z][a-z0-9_]{0,15}", str(it["role"])) or not NAME_RE.fullmatch(str(it["name"])):
            raise ValueError("pins: names")
        dest = it["dest"]
        parts = dest.split("/") if type(dest) is str else [""]
        if dest.startswith("/") or any(p in ("", ".", "..") or not re.fullmatch(r"[A-Za-z0-9._@+-]{1,128}", p) for p in parts):
            raise ValueError("pins: dest")
        if (it["sha256"] is not None and not (type(it["sha256"]) is str and HEX64.fullmatch(it["sha256"]))) or (
                it["size"] is not None and not (type(it["size"]) is int and 0 < it["size"] < 1 << 40)):
            raise ValueError("pins: sha256 or size")
        if it["needed_by"] not in ("image", "tools") or ("ref" in it) == ("mirror" in it):
            raise ValueError("pins: sources")
        if "ref" in it and not NAME_RE.fullmatch(str(it["ref"])):
            raise ValueError("pins: ref")
        if "mirror" in it and not re.fullmatch(r"https://[A-Za-z0-9./_-]{1,200}", str(it["mirror"])):
            raise ValueError("pins: mirror")
        if it["role"] in roles or it["name"] in names or dest in dests:
            raise ValueError("pins: duplicate")
        roles.add(it["role"]), names.add(it["name"]), dests.add(dest)
    if not {"image", "gop"} <= roles:
        raise ValueError("pins: image and gop are required")
    return doc


def _no_duplicates(pairs):
    out = {}
    for k, v in pairs:
        if k in out:
            raise ValueError("duplicate key")
        out[k] = v
    return out


def unset_roles(pins, roles):
    """Roles among `roles` whose sha256 or size is not pinned. An unpinned object is never fetched."""
    return [i["role"] for i in pins["items"] if i["role"] in roles and (i["sha256"] is None or i["size"] is None)]


def needed_roles(build_swtpm):
    return ("image", "gop") + (("libtpms", "swtpm") if build_swtpm else ())


def build_manifest(pins, build_swtpm, roles=None):
    """kayfabe.download-manifest/1 for the needed roles. Refuses (PIN_UNSET:<first role>) when any needed item is unpinned."""
    roles = roles or needed_roles(build_swtpm)
    unset = unset_roles(pins, roles)
    if unset:
        raise Refused("PIN_UNSET:" + unset[0], ",".join(unset))
    items, stages = [], []
    for it in pins["items"]:
        if it["role"] not in roles:
            continue
        source = {"kind": "signed", "ref": it["ref"]} if "ref" in it else {"kind": "mirror", "url": it["mirror"]}
        items.append({"name": it["name"], "dest": it["dest"], "sha256": it["sha256"], "size": it["size"],
                      "needed_by": it["needed_by"], "sources": [source]})
    for it in items:
        if it["needed_by"] not in stages:
            stages.append(it["needed_by"])
    stages.sort(key=("tools", "image").index)
    return {"schema": "kayfabe.download-manifest/1", "stages": stages, "items": items}


# ---------------------------------------------------------------- preflight (rows are dicts: name, status, detail)
def row(name, status, detail):
    return {"name": name, "status": status, "detail": clean(detail, 160)}


def cpu_rows(cpuinfo_text, allowed=ALLOWED_CPU_VENDORS, pi=None):
    """cpu_vendor (NOT_RUN reason CPU_VENDOR when not allowed) and cpu_features (CPU_PROFILE_UNSUPPORTED)."""
    pi = pi or harness().PI
    vendor, _ = pi.parse_cpuinfo(cpuinfo_text)
    rows = [row("cpu_vendor", "MATCH" if vendor in allowed else "MISMATCH",
                "host %s, allowed %s" % (clean(vendor, 30), ",".join(allowed)))]
    for name, status, detail in pi.cpu_preflight({"cpu": pi.CPU_PROFILE}, cpuinfo_text)[1:]:
        rows.append(row(name, status, detail))
    return rows


def family_row(hardware):
    cc = hardware.get("compute_cap") if type(hardware) is dict else None
    family = FAMILY_BY_CC.get(cc) if type(cc) is int else None
    if family is None:
        return row("gpu_family", "MISMATCH", "compute capability %s is not a supported family" % clean(cc, 8)), None
    note = "MEASURED with this image" if family in MEASURED_FAMILIES else "not measured with this image yet"
    return row("gpu_family", "MATCH", "%s (%s)" % (family, note)), family


def disk_row(free_bytes, pins):
    size = next((i["size"] for i in pins["items"] if i["role"] == "image" and i["size"]), IMAGE_ZIP_ESTIMATE)
    need = 2 * size + DISK_HEADROOM       # the archive and its unpacked members exist together once
    return row("disk", "MATCH" if free_bytes >= need else "MISMATCH", "free %d GiB, need %d GiB" % (free_bytes >> 30, need >> 30)), need


def swtpm_version(swtpm, runner, env=None):
    """(major, minor, patch) of `swtpm --version`, or None."""
    try:
        done = runner([swtpm, "--version"], capture_output=True, text=True, timeout=30, env=env)
    except (OSError, subprocess.SubprocessError):
        return None
    m = re.search(r"\bversion\s+(\d+)\.(\d+)(?:\.(\d+))?", (done.stdout or "") + (done.stderr or ""))
    return tuple(int(x or 0) for x in m.groups()) if m else None


def swtpm_env(environ, prefix_bin=SWTPM_PREFIX + "/bin/swtpm"):
    """The environment `swtpm` needs: a binary built under /opt/tpm links libtpms from there (2026-11-16, MEASURED: without
    LD_LIBRARY_PATH an already built /opt/tpm/bin/swtpm does not start and counted as absent)."""
    if swtpm_binary(environ, prefix_bin) != prefix_bin:
        return environ
    lib = os.path.dirname(os.path.dirname(prefix_bin)) + "/lib"
    old = environ.get("LD_LIBRARY_PATH")
    if old and lib in old.split(":"):
        return dict(environ, SWTPM=prefix_bin)
    return dict(environ, SWTPM=prefix_bin, LD_LIBRARY_PATH=lib + (":" + old if old else ""))


def swtpm_binary(environ, prefix_bin=SWTPM_PREFIX + "/bin/swtpm"):
    if environ.get("SWTPM"):
        return environ["SWTPM"]
    return prefix_bin if os.access(prefix_bin, os.X_OK) else "swtpm"


# ---------------------------------------------------------------- boots
def plan_boots(timeouts, budget=BOOT_BUDGET_S):
    """Boots in app order: greedy groups whose timeouts sum to at most `budget`; Wukong is always last and alone."""
    boots, cur, total = [], [], 0
    for app in APPS[:-1]:
        t = timeouts[app]
        if cur and total + t > budget:
            boots.append(cur)
            cur, total = [], 0
        cur.append(app)
        total += t
    if cur:
        boots.append(cur)
    boots.append([APPS[-1]])
    return boots


def boot_command(check_py, a):
    """The exact propimage_check argv for one boot. `a` carries paths; nothing here comes from the guest."""
    cmd = [sys.executable, "-B", "-I", str(check_py), "--dir", a["dir"], "--work", a["work"], "--out", a["out"], "--qemu", a["qemu"],
           "--flags", a["flags"], "--broker", a["broker"], "--gop-efi", a["gop"], "--display", "broker", "--apps", ",".join(a["apps"])]
    if "superposition" in a["apps"]:
        cmd.append("--superposition-run")     # without it the harness only launches the launcher (LAUNCH_ONLY)
    if "wukong" in a["apps"]:
        cmd += ["--fb-mb", str(a.get("fb_mb", WUKONG_FB_MB))]
    return cmd


def offline_proof(work):
    """The harness writes the QEMU argv to <work>/run/last_cmd.txt: it must be restrict=on and boot the overlay, not the image."""
    try:
        text = (Path(work) / "run/last_cmd.txt").read_text()
    except OSError:
        return False
    return ("restrict=on" in text and "restrict=off" not in text and "file=" + str(work) + "/run/clone.qcow2" in text
            and "image.base.qcow2" not in text.replace("clone.qcow2", ""))


# ---------------------------------------------------------------- the host kernel log, in windows per app
class KernelLog:
    """The host kernel log from the start of the run, with a line offset per window. Same file formats as scripts/ci/kernel_log.py of
    draft PR #420 (kernel.log, stage-offsets.tsv); a local equivalent until that merges. An unreadable log is an error at start,
    never an empty capture. Limit: a line that reaches the follower after mark() settled lands in the next window (SETTLE_S)."""

    SETTLE_S = 1.0
    SOURCES = (["dmesg", "-w", "--time-format", "iso"], ["journalctl", "-k", "-f", "-o", "short-iso", "--no-pager"])

    def __init__(self, directory, source=None, sleep=time.sleep):
        self.dir, self.source, self.sleep, self.proc, self.file = Path(directory), source, sleep, None, None
        self.log, self.offsets = self.dir / "kernel.log", self.dir / "stage-offsets.tsv"

    def count(self):
        try:
            with open(self.log, "rb") as f:
                return sum(chunk.count(b"\n") for chunk in iter(lambda: f.read(1 << 20), b""))
        except FileNotFoundError:
            return 0

    def start(self):
        self.dir.mkdir(parents=True, exist_ok=True)
        self.file = open(self.log, "ab")
        sources = [self.source] if self.source else self.SOURCES
        for command in sources:
            if shutil.which(command[0]) is None:
                continue
            self.proc = subprocess.Popen(command, stdout=self.file, stderr=subprocess.DEVNULL, stdin=subprocess.DEVNULL,
                                         start_new_session=True)
            for _ in range(50):
                if self.proc.poll() is not None or self.count() > 0:
                    break
                self.sleep(0.1)
            if self.proc.poll() is None and self.count() > 0:
                return
            self.stop()
        self.stop()
        raise Refused("KERNEL_LOG")

    def stop(self):
        if self.proc is not None and self.proc.poll() is None:
            try:
                os.killpg(self.proc.pid, signal.SIGTERM)
                self.proc.wait(timeout=5)
            except (OSError, subprocess.SubprocessError):
                pass
        if self.file is not None:
            self.file.close()
            self.file = None

    def mark(self, label, edge, settle=None):
        assert edge in ("start", "end")
        wait, last = self.SETTLE_S if settle is None else settle, self.count()
        t_end = time.monotonic() + wait
        while time.monotonic() < t_end:
            self.sleep(min(0.2, wait))
            now = self.count()
            if now == last:
                break
            last = now
        with open(self.offsets, "a") as f:
            f.write("%s\t%s\t%d\t%d\n" % (label, edge, last, int(time.time())))
        return last

    def windows(self):
        opened, out = {}, {}
        try:
            rows = [r.split("\t") for r in self.offsets.read_text().splitlines()]
        except FileNotFoundError:
            rows = []
        for label, edge, n, _ in rows:
            if edge == "start":
                opened[label] = int(n)
            elif label in opened:
                out[label] = (opened.pop(label) + 1, int(n))
        for label, n in opened.items():
            out[label] = (n + 1, 1 << 62)
        return out

    def xids(self, first, last):
        """Kernel lines first..last (1-based, inclusive) that are Xid lines, as (line number, text)."""
        found = []
        try:
            with open(self.log, "rb") as f:
                for n, raw in enumerate(f, 1):
                    if n > last:
                        break
                    if n >= first:
                        text = raw.decode("utf-8", "replace")
                        if XID_RE.search(text):
                            found.append((n, clean(text.strip(), 160)))
        except FileNotFoundError:
            pass
        return found


def xid_by_window(klog, labels, boot_label):
    """{app: [(n, text)]} for each app window, plus the Xid lines of the boot that fall in no app window (unattributed)."""
    wins = klog.windows()
    per = {}
    for app in labels:
        lo_hi = wins.get(boot_label + ":" + app)
        per[app] = klog.xids(*lo_hi) if lo_hi else []
    boot = wins.get(boot_label)
    inside = {n for lines in per.values() for n, _ in lines}
    rest = [(n, t) for n, t in klog.xids(*boot)] if boot else []
    return per, [(n, t) for n, t in rest if n not in inside]


# ---------------------------------------------------------------- reading what the harness left
def sha256_file(path, limit=1 << 30):
    h, n = hashlib.sha256(), 0
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            n += len(chunk)
            if n > limit:
                raise ValueError("too large")
            h.update(chunk)
    return h.hexdigest()


def read_json(path, limit=1 << 20):
    with open(path, "rb") as f:
        raw = f.read(limit + 1)
    if len(raw) > limit:
        raise ValueError("too large")
    return json.loads(raw.decode("utf-8-sig"), object_pairs_hook=_no_duplicates)


def evidence_files(run_dir):
    """File names, sizes and sha256 of the regular files in one app run directory. Symlinks and odd names are skipped."""
    files = []
    for p in sorted(Path(run_dir).iterdir()):
        if len(files) >= MAX_EVIDENCE_FILES:
            break
        if p.is_symlink() or not p.is_file() or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{0,63}", p.name):
            continue
        try:
            files.append({"file": p.name, "bytes": p.stat().st_size, "sha256": sha256_file(p)})
        except (OSError, ValueError):
            continue
    return files


def run_dirs(out, app):
    base = Path(out) / app
    if base.is_symlink() or not base.is_dir():
        return []
    return [p for p in sorted(base.iterdir()) if RUN_ID.fullmatch(p.name) and p.is_dir() and not p.is_symlink()]


def guest_events(run_dir):
    """Guest GPU events of one run. Only the Wukong script collects them (propimage_check.WUKONG_RUN): `collected` says so,
    so a zero is never read as 'none happened' where nothing was looked at."""
    out = {"collected": False, "live_kernel_events": 0, "codes": [], "tdr_4101": 0, "nvlddmkm_153": 0}
    try:
        res = read_json(Path(run_dir) / "result.json")
    except (OSError, ValueError):
        res = {}
    if type(res) is dict and "kernel_events" in res:
        out["collected"] = True
        evs = res["kernel_events"] if type(res["kernel_events"]) is list else []
        codes = [clean(e.get("code"), 8) for e in evs if type(e) is dict and e.get("event") == "LiveKernelEvent"]
        out["live_kernel_events"], out["codes"] = len(codes), codes[:8]
    try:
        text = (Path(run_dir) / "system_events.txt").read_text(errors="replace")[:1 << 20]
        out["collected"] = True
        out["tdr_4101"] = len(re.findall(r"\bid=4101\b", text))
        out["nvlddmkm_153"] = len(re.findall(r"nvlddmkm id=153\b", text))
    except OSError:
        pass
    return out


def scores_of(app, signal_text):
    """Scores the harness itself wrote into its verdict signal from a graded file (3DMark: Result.xml)."""
    if app == "3dmark":
        m = re.search(r"3DMark score (\d+) \(graphics (\d+)\)", signal_text)
        return {"3dmark": int(m[1]), "graphics": int(m[2])} if m else {}
    m = re.search(r"\bscore (\d+)\b", signal_text) if app == "superposition" else None
    return {"score": int(m[1])} if m else {}


def collect_app(out, app, rows, xids, expected=None):
    """One app's record from the harness rows (result.json) and its verdict.json; both must agree for a PASS."""
    rec = {"app": app, "verdict": "MISSING", "kind": "NONE", "secs": 0, "detail": "", "evidence": [], "scores": {},
           "guest_events": {"collected": False, "live_kernel_events": 0, "codes": [], "tdr_4101": 0, "nvlddmkm_153": 0},
           "host_xid": {"count": len(xids), "lines": [t for _, t in xids[:MAX_XID_LINES]]}}
    step = next((r for r in rows if type(r) is dict and r.get("step") == "app:" + app), None)
    dirs = run_dirs(out, app)
    verdict_doc = None
    if len(dirs) == 1:
        try:
            verdict_doc = read_json(dirs[0] / "verdict.json")
            rec["evidence"] = evidence_files(dirs[0])
            rec["guest_events"] = guest_events(dirs[0])
        except (OSError, ValueError):
            verdict_doc = None
    if step is None:
        rec["detail"] = "no app row from the harness"
        return rec
    said = clean(step.get("verdict"), 20)
    rec["verdict"] = said if said in APP_VERDICTS else "FAIL"
    rec["detail"] = clean(step.get("detail"), 200)
    kind = clean(step.get("evidence"), 8)
    rec["kind"] = kind if kind in ("FILE", "UIA", "NONE") else "NONE"
    if type(verdict_doc) is dict:
        secs = verdict_doc.get("secs")
        rec["secs"] = secs if type(secs) is int and 0 <= secs < 1 << 20 else 0
        rec["scores"] = scores_of(app, clean(verdict_doc.get("signal"), 400))
    if rec["verdict"] == "PASS":
        proven = (type(verdict_doc) is dict and verdict_doc.get("app") == app and verdict_doc.get("ok") is True
                  and verdict_doc.get("evidence") == rec["kind"] == (expected or EXPECTED_KIND.get(app)) and len(dirs) == 1)
        if not proven:
            rec["verdict"], rec["detail"] = "FAIL", "UNPROVEN_PASS: " + rec["detail"][:150]
    return rec


def grade(apps):
    """(verdict, reason, failed names). PASS only when all seven are PASS; the first non-PASS app is named in the reason."""
    by = {a["app"]: a for a in apps}
    failed = [n for n in APPS if by.get(n, {}).get("verdict") != "PASS"]
    if not failed:
        return "PASS", "", []
    app_class = [n for n in failed if by.get(n, {}).get("verdict") in ("FAIL", "INCONCLUSIVE", "LAUNCH_ONLY")]
    if app_class:
        return "FAIL", "APP_FAIL:" + app_class[0], failed
    return "RETRY", "HARNESS:" + failed[0], failed


# ---------------------------------------------------------------- the stage
@dataclasses.dataclass
class Context:
    sha: str
    checkout: Path
    prefix: Path
    out: Path
    urls: Path
    bench: Path = Path("/workspace/bench")
    deadline_epoch: float = 0.0
    cpuinfo: str = "/proc/cpuinfo"
    environ: dict = dataclasses.field(default_factory=lambda: dict(os.environ))
    pins_path: Path = PINS_PATH
    check_py: Path = None
    hardware: dict = None
    free_bytes: object = None
    runner: object = subprocess.run
    boot_runner: object = None
    klog_source: object = None
    qemu: Path = None
    unpack: object = None
    plain_py: Path = None          # propimage_plain.py (the generic runner's no-state.json harness)

    def __post_init__(self):
        self.checkout, self.prefix, self.out, self.urls = (Path(x) for x in (self.checkout, self.prefix, self.out, self.urls))
        self.check_py = Path(self.check_py or self.checkout / "scripts/bench/windows/propimage_check.py")
        self.qemu = Path(self.qemu or Path(self.bench) / "kf3-bins" / self.sha[:8] / "qemu-system-x86_64")
        self.plain_py = Path(self.plain_py or self.checkout / "scripts/bench/windows/propimage_plain.py")


def new_record(ctx):
    # until the stage settles, a partial file on disk says so: RETRY, INTERRUPTED (a payload killed mid-run leaves this)
    return {"schema": SCHEMA, "verdict": "RETRY", "sha": ctx.sha, "reason": "INTERRUPTED", "family": None, "cpu_vendor": None,
            "preflight": [], "image": {"name": None, "sha256": None, "size": None}, "swtpm": {"version": None, "built": False},
            "boots": [], "apps": [], "failed": []}


def refusal_of(rows):
    """(verdict, reason) of the first refusing preflight row: the CPU vendor is NOT_RUN, the others REFUSED."""
    names = {"cpu_vendor": ("NOT_RUN", "CPU_VENDOR"), "cpu_features": ("REFUSED", "CPU_PROFILE_UNSUPPORTED"),
             "gpu_family": ("REFUSED", "GPU_FAMILY"), "pins": ("REFUSED", "PIN_UNSET"), "kf3_qemu": ("REFUSED", "NO_KF3_QEMU"),
             "urls": ("REFUSED", "URLS_MISSING"), "disk": ("REFUSED", "DISK_FREE"), "swtpm": ("REFUSED", "SWTPM_VERSION"),
             "broker_build": ("REFUSED", "NO_BROKER_BUILD"), "kernel_log": ("REFUSED", "KERNEL_LOG")}
    bad = [r for r in rows if r["status"] == "MISMATCH"]   # rows are in precedence order: cpu_vendor is first
    if bad:
        verdict, reason = names.get(bad[0]["name"], ("REFUSED", "PREFLIGHT"))
        return verdict, reason + (":" + bad[0]["detail"].split()[0] if reason == "PIN_UNSET" else "")
    return None


def preflight(ctx, swtpm_ok=None):
    """All cheap local checks, in the order of the module docstring. Returns (rows, family, need_swtpm_build)."""
    rows, pi = [], harness(ctx.checkout).PI
    try:
        cpuinfo = Path(ctx.cpuinfo).read_text()
    except OSError:
        cpuinfo = ""
    rows += cpu_rows(cpuinfo, pi=pi)
    vendor = {"AuthenticAMD": "amd", "GenuineIntel": "intel"}.get(pi.parse_cpuinfo(cpuinfo)[0], "other")
    frow, family = family_row(ctx.hardware or {})
    rows.append(frow)
    mem = (ctx.hardware or {}).get("memory_mib")
    if type(mem) is int and mem:
        rows.append(row("gpu_memory", "MATCH" if mem >= 2 * WUKONG_FB_MB else "WARN",
                        "%d MiB; the Wukong boot gives the guest %d MiB of framebuffer" % (mem, WUKONG_FB_MB)))
    version = swtpm_version(swtpm_binary(ctx.environ), ctx.runner, swtpm_env(ctx.environ))
    build = version is None or version < (0, 10, 0)
    pins = load_pins(ctx.pins_path)
    unset = unset_roles(pins, needed_roles(build))
    rows.append(row("pins", "MISMATCH" if unset else "MATCH",
                    (unset[0] + " unpinned (" + ",".join(unset) + ")") if unset else "image, gop%s pinned" % (", swtpm sources" if build else "")))
    if not build:
        rows.append(row("swtpm", "MATCH", "%d.%d.%d" % version))
    elif "swtpm" in unset or "libtpms" in unset:
        rows.append(row("swtpm", "MISMATCH", "older than 0.10 or missing, and the 0.10.1 sources are unpinned"))
    else:
        rows.append(row("swtpm", "WARN", "older than 0.10 or missing: built from the pinned sources"))
    rows.append(row("kf3_qemu", "MATCH" if os.access(ctx.qemu, os.X_OK) else "MISMATCH", "kf3-bins/%s/qemu-system-x86_64" % ctx.sha[:8]))
    rows.append(row("urls", "MATCH" if ctx.urls.is_file() and not ctx.urls.is_symlink() else "MISMATCH", "presigned-URL file from the bootstrap layer"))
    free = ctx.free_bytes if ctx.free_bytes is not None else shutil.disk_usage(_existing(ctx.prefix)).free
    drow, _ = disk_row(free, pins)
    rows.append(drow)
    rows.append(row("tesseract", "MATCH" if shutil.which("tesseract") else "WARN", "optional OCR extra of the Superposition result screen"))
    return rows, family, build, pins, vendor


def _existing(path):
    p = Path(path)
    while not p.exists() and p != p.parent:
        p = p.parent
    return p


def check_prefix(prefix):
    p = Path(prefix)
    if not p.is_absolute() or ".." in p.parts or str(p) in ("/", "/root", "/home", "/workspace") or len(p.parts) < 3:
        raise Refused("PREFIX")
    if p.exists() and (p.is_symlink() or not p.is_dir()):
        raise Refused("PREFIX")


def write_record(path, rec):
    data = json.dumps(rec, separators=(",", ":"), sort_keys=True)
    if len(data.encode()) > MAX_RECORD_BYTES:
        for a in rec["apps"]:
            a["host_xid"]["lines"] = a["host_xid"]["lines"][:2]
            a["evidence"] = a["evidence"][:8]
        data = json.dumps(rec, separators=(",", ":"), sort_keys=True)
    tmp = Path(str(path) + ".tmp")
    tmp.write_text(data + "\n")
    tmp.replace(path)
    return data


def run_command(ctx, argv, log, timeout, cwd=None, env=None):
    with open(log, "ab") as f:
        try:
            done = ctx.runner(argv, stdout=f, stderr=subprocess.STDOUT, timeout=timeout, cwd=cwd, env=env, stdin=subprocess.DEVNULL)
        except subprocess.TimeoutExpired:
            return 124
    return done.returncode


def build_swtpm(ctx, dl, work, logs):
    """libtpms then swtpm 0.10.x under /opt/tpm from the pinned tarballs. NOT TESTED on a box: the maintainers' recipe was not
    recorded (traces/hwrun_20261113/README.md only says 'built from source under /opt/tpm worked')."""
    log = logs / "swtpm_build.log"
    env = dict(ctx.environ, PKG_CONFIG_PATH=SWTPM_PREFIX + "/lib/pkgconfig", LD_LIBRARY_PATH=SWTPM_PREFIX + "/lib",
               DEBIAN_FRONTEND="noninteractive")
    steps = [(["apt-get", "install", "-y", "--no-install-recommends", *SWTPM_DEPS], None, 1200)]
    for name, args in (("libtpms-0.10.1.tar.gz", ["--with-openssl", "--with-tpm2"]), ("swtpm-0.10.1.tar.gz", ["--with-openssl"])):
        src = work / name.replace(".tar.gz", "")
        src.mkdir(parents=True, exist_ok=True)
        steps += [(["tar", "--strip-components=1", "-xzf", str(dl / "tools" / name), "-C", str(src)], None, 600),
                  (["./autogen.sh", "--prefix=" + SWTPM_PREFIX, *args], src, 1800),
                  (["make", "-j", str(min(8, os.cpu_count() or 2))], src, 3600), (["make", "install"], src, 600)]
    for argv, cwd, timeout in steps:
        if run_command(ctx, argv, log, timeout, cwd=cwd, env=env) != 0:
            raise Refused("SWTPM_BUILD")
    version = swtpm_version(SWTPM_PREFIX + "/bin/swtpm", ctx.runner, env)
    if version is None or version < (0, 10, 0):
        raise Refused("SWTPM_BUILD")
    return env


def fetch(ctx, manifest_path, dl, logs):
    """box_downloader.py: check, fetch (blocking, resumable), verify. The sha256 and size come from the pins file only."""
    dl_py = str(HERE / "box_downloader.py")
    log = logs / "downloader.log"
    for argv in (["check", str(manifest_path)],
                 ["fetch", str(manifest_path), "--dest", str(dl), "--urls", str(ctx.urls), "--reserve-gb", str(DISK_HEADROOM >> 30)],
                 ["verify", str(manifest_path), "--dest", str(dl)]):
        if run_command(ctx, [sys.executable, "-B", "-I", dl_py, *argv], log, 7200 if argv[0] == "fetch" else 3600) != 0:
            raise Refused("DOWNLOAD")


class Mark:
    """Reads the harness stdout while it runs and marks the kernel log at each app boundary."""

    LINE = re.compile(r"^(app:[a-z0-9_]+|verify|stop) ")

    def __init__(self, klog, boot_label, apps):
        self.klog, self.label, self.apps, self.seen = klog, boot_label, apps, []
        self.started = False

    def line(self, text):
        m = self.LINE.match(text)
        if not m:
            return
        step = m.group(1)
        if step == "verify" and not self.started:
            self.started = True
            self.klog.mark(self.label + ":" + self.apps[0], "start")
        elif step.startswith("app:") and step[4:] in self.apps:
            app = step[4:]
            self.klog.mark(self.label + ":" + app, "end")
            self.seen.append(app)
            if len(self.seen) < len(self.apps):
                self.klog.mark(self.label + ":" + self.apps[len(self.seen)], "start")


def run_boot(ctx, argv, log, timeout, mark):
    """Spawn the harness in its own session, feed its stdout to `mark`, kill the group on the wall limit. -> rc or 124."""
    with open(log, "a") as f:
        proc = subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL, start_new_session=True,
                                env=ctx.environ, text=True, errors="replace")

        def pump():
            for text in proc.stdout:
                f.write(text)
                f.flush()
                mark.line(text)

        t = threading.Thread(target=pump, daemon=True)
        t.start()
        try:
            rc = proc.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(proc.pid, signal.SIGKILL)
            except OSError:
                pass
            proc.wait()
            rc = 124
        t.join(10)
    return rc


def copy_evidence(out, evidence_dir):
    """A small text allow-list per app run into the evidence directory; PNGs, STEP and result files stay (hashed in the record)."""
    for app in APPS:
        for d in run_dirs(out, app):
            for name in EVIDENCE_COPY:
                src = d / name
                if src.is_file() and not src.is_symlink() and src.stat().st_size <= 1 << 20:
                    dst = Path(evidence_dir) / app
                    dst.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(src, dst / name)


def run(ctx, evidence_dir=None):
    """The stage. Returns the record; never raises for a named refusal."""
    rec = new_record(ctx)
    klog, started, out_of_time = None, False, False
    try:
        check_prefix(ctx.prefix)
        rows, family, build, pins, vendor = preflight(ctx)
        rec["preflight"], rec["family"], rec["cpu_vendor"] = rows, family, vendor
        image = next(i for i in pins["items"] if i["role"] == "image")
        rec["image"] = {"name": image["name"], "sha256": image["sha256"], "size": image["size"]}
        refusal = refusal_of(rows)
        if refusal:
            rec["verdict"], rec["reason"] = refusal
            return rec
        pi_check = harness(ctx.checkout)
        started = True
        klog = KernelLog(ctx.prefix / "kernel", ctx.klog_source)
        klog.start()
        timeouts = {a: pi_check.CHECKS[a]["timeout"] for a in APPS}
        plan = plan_boots(timeouts)
        rec["boots"] = [{"index": i + 1, "apps": b, "rc": None, "secs": 0, "offline": False} for i, b in enumerate(plan)]
        work, dl, imgdir, logs = ctx.prefix / "work", ctx.prefix / "dl", ctx.prefix / "unpacked", ctx.prefix / "logs"
        for d in (work, dl, imgdir, logs):
            d.mkdir(parents=True, exist_ok=True)
        manifest = build_manifest(pins, build)
        manifest_path = ctx.prefix / "manifest.json"
        manifest_path.write_text(json.dumps(manifest))
        fetch(ctx, manifest_path, dl, logs)
        if build:
            env = build_swtpm(ctx, dl, work, logs)
            ctx.environ.update(SWTPM=SWTPM_PREFIX + "/bin/swtpm", LD_LIBRARY_PATH=env["LD_LIBRARY_PATH"])
            rec["swtpm"]["built"] = True
        ctx.environ.update(swtpm_env(ctx.environ))
        have_version = swtpm_version(swtpm_binary(ctx.environ), ctx.runner, ctx.environ)
        rec["swtpm"]["version"] = ".".join(str(x) for x in have_version) if have_version else None
        broker = ctx.checkout / "broker/nvkvm-display-broker"
        if not broker.is_file() and run_command(ctx, ["make", "-C", str(ctx.checkout / "broker")], logs / "broker_build.log", 900) != 0:
            raise Refused("NO_BROKER_BUILD")
        try:
            (ctx.unpack or pi_check.load_image)(str(dl / "image/image.zip"), str(imgdir))
        except Exception as exc:  # noqa: BLE001 propimage raises PropError, zipfile and json errors, ValueError
            rec["error"] = clean(exc, 120)
            raise Refused("UNPACK")
        (dl / "image/image.zip").unlink(missing_ok=True)        # the archive is not needed again; the GOP stays
        flags = ctx.checkout / "scripts/bench/windows/kf3_flags.sample.env"
        for boot in rec["boots"]:
            label = "boot%d" % boot["index"]
            need = sum(timeouts[a] for a in boot["apps"])
            if ctx.deadline_epoch and time.time() + need + 900 > ctx.deadline_epoch:
                boot["rc"] = 125
                out_of_time = True
                continue
            out, wk = ctx.prefix / "out" / label, work / label
            a = {"dir": str(imgdir), "work": str(wk), "out": str(out), "qemu": str(ctx.qemu), "flags": str(flags),
                 "broker": str(broker), "gop": str(dl / "image/kf-gop.signed.efi"), "apps": boot["apps"]}
            klog.mark(label, "start")
            t0 = time.monotonic()
            mark = Mark(klog, label, boot["apps"])
            (ctx.prefix / "out").mkdir(exist_ok=True)
            boot["rc"] = (ctx.boot_runner or run_boot)(ctx, boot_command(ctx.check_py, a), logs / (label + ".log"), need + BOOT_SLACK_S, mark)
            boot["secs"] = int(time.monotonic() - t0)
            klog.mark(label, "end")
            boot["offline"] = offline_proof(wk)
            try:
                rows_json = read_json(out / "result.json")
                rows_json = rows_json if type(rows_json) is list else []
            except (OSError, ValueError):
                rows_json = []
            boot["harness"] = [{"step": clean(r.get("step"), 24), "verdict": clean(r.get("verdict"), 12), "detail": clean(r.get("detail"), 120)}
                               for r in rows_json if type(r) is dict and r.get("class") == "harness"
                               and r.get("verdict") not in ("PASS", "OK")][:6]
            per, unattributed = xid_by_window(klog, boot["apps"], label)
            for app in boot["apps"]:
                item = collect_app(out, app, rows_json, per[app])
                if not boot["offline"] and item["verdict"] == "PASS":
                    item["verdict"], item["detail"] = "FAIL", "OFFLINE_NOT_PROVEN: " + item["detail"][:150]
                rec["apps"].append(item)
            if unattributed:
                boot["host_xid_unattributed"] = {"count": len(unattributed), "lines": [t for _, t in unattributed[:MAX_XID_LINES]]}
            copy_evidence(out, evidence_dir) if evidence_dir else None
            write_record(ctx.out, rec)
        have = {a["app"] for a in rec["apps"]}
        for app in APPS:
            if app not in have:
                rec["apps"].append(dict(collect_app(ctx.prefix / "none", app, [], []), verdict="NOT_RUN", detail="boot not started"))
        rec["apps"].sort(key=lambda a: APPS.index(a["app"]))
        rec["verdict"], rec["reason"], rec["failed"] = grade(rec["apps"])
        if rec["verdict"] == "PASS" and any(not b["offline"] for b in rec["boots"]):
            rec["verdict"], rec["reason"], rec["failed"] = "FAIL", "OFFLINE_NOT_PROVEN", []
        if out_of_time and rec["verdict"] != "PASS":
            rec["verdict"], rec["reason"] = "RETRY", "TIME"
    except Refused as r:
        rec["verdict"], rec["reason"] = "REFUSED", r.reason
    except (ValueError, OSError, KeyError, subprocess.SubprocessError) as exc:
        rec["verdict"], rec["reason"], rec["error"] = "REFUSED", "INTERNAL", clean(exc, 120)
    finally:
        if klog is not None:
            klog.stop()
        if started:
            cleanup(ctx)
    return rec


def cleanup(ctx):
    """The image never stays on the box: archive, unpacked members, overlays, work and logs of the swtpm/qemu run."""
    for sub in ("dl", "unpacked", "work"):
        shutil.rmtree(ctx.prefix / sub, ignore_errors=True)


# ---------------------------------------------------------------- command line
def hardware_now(runner=subprocess.run):
    """gpu_name, compute_cap (x100, as vast_payload.hardware) and memory_mib of the one GPU; zeros when it cannot be read."""
    try:
        done = runner(["nvidia-smi", "--query-gpu=name,compute_cap,memory.total", "--format=csv,noheader,nounits"],
                      capture_output=True, text=True, timeout=30)
        lines = done.stdout.strip().splitlines()
        name, cc, mem = [x.strip() for x in lines[0].split(",")]
        if done.returncode or len(lines) != 1:
            raise ValueError
        return {"gpu_name": clean(name, 64), "compute_cap": int(round(float(cc) * 100)), "memory_mib": int(float(mem))}
    except (OSError, subprocess.SubprocessError, ValueError, IndexError):
        return {"gpu_name": "unknown", "compute_cap": 0, "memory_mib": 0}


def finish(rec, out):
    data = write_record(out, rec) if out else json.dumps(rec, separators=(",", ":"), sort_keys=True)
    print("KF_PROPIMAGE_APPS_RESULT " + data, flush=True)
    return 0 if rec["verdict"] in ("PASS", "NOT_RUN") else (3 if rec["verdict"] == "RETRY" else 1)


def main(argv=None):
    """`run` with image flags (--image, --propimage, --images-file) is the generic runner (propimage_images.py); without them the
    legacy stage below. The image flags are taken out of the words first: they are per image and order matters."""
    sys.path.insert(0, str(WINDOWS_DIR))
    import propimage_runspec as spec_mod
    try:
        words, spec = spec_mod.split_flags(list(sys.argv[1:] if argv is None else argv))
        if "--images-file" in words:
            i = words.index("--images-file")
            path, words = words[i + 1], words[:i] + words[i + 2:]
            if spec:
                raise spec_mod.SpecError("IMAGES_FILE_AND_FLAGS")
            spec = spec_mod.parse_json(Path(path).read_text())
    except (spec_mod.SpecError, OSError, IndexError) as exc:
        print("KF_PROPIMAGE_APPS_USAGE SPEC:" + clean(exc, 80), flush=True)
        return 2
    return _main(words, spec)


def _main(argv, spec):
    p = argparse.ArgumentParser(prog="propimage_apps.py", description=__doc__.split("\n\n")[0])
    sub = p.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run")
    r.add_argument("--sha", required=True)
    r.add_argument("--checkout", default=str(ROOT))
    r.add_argument("--prefix", default=DEFAULT_PREFIX)
    r.add_argument("--out", required=True)
    r.add_argument("--urls", default=DEFAULT_URLS)
    r.add_argument("--bench", default="/workspace/bench")
    r.add_argument("--deadline-epoch", type=float, default=0.0)
    r.add_argument("--evidence-dir")
    r.add_argument("--qemu", help="generic runner: the kf3 QEMU binary (default: the bench dir's kf3-bins/<sha>/)")
    r.add_argument("--gop-efi", help="generic runner: the signed boot GOP driver for state.json images")
    r.add_argument("--pins", help="YOUR deployment's pins file (kayfabe.propimage-pins/1): the objects `pin:NAME` and the legacy stage may fetch "
                                  "(default: the repository's own propimage_apps_pins.json)")
    q = sub.add_parser("plan")
    q.add_argument("--sha", default="0" * 40)
    q.add_argument("--cpuinfo", default="/proc/cpuinfo")
    m = sub.add_parser("manifest")
    m.add_argument("--out", required=True)
    m.add_argument("--swtpm-build", action="store_true")
    a = p.parse_args(argv)
    if spec and a.cmd == "manifest":
        return 2
    try:
        if a.cmd == "manifest":
            Path(a.out).write_text(json.dumps(build_manifest(load_pins(), a.swtpm_build), indent=1) + "\n")
            return 0
        if a.cmd == "plan" and spec:           # the per-image plan; touches nothing
            import propimage_registry
            import propimage_runspec
            reg, doc = propimage_registry.load_registry(), propimage_runspec.validate(spec)
            for img, line in zip(doc["images"], propimage_runspec.describe(doc)):
                rows, _, decided = propimage_runspec.plan_apps(img, reg)
                print(line, "decided=" + decided, "run=" + ",".join(r["app"] for r in rows if r["action"] == "run"),
                      "not_run=" + ",".join("%s(%s)" % (r["app"], r["reason"]) for r in rows if r["action"] == "not_run"), flush=True)
            return 0
        if a.cmd == "plan":
            ctx = Context(a.sha, ROOT, DEFAULT_PREFIX, "/dev/null", DEFAULT_URLS, cpuinfo=a.cpuinfo, hardware=hardware_now())
            rows, family, build, _, _ = preflight(ctx)
            timeouts = {x: harness().CHECKS[x]["timeout"] for x in APPS}
            print(json.dumps({"preflight": rows, "family": family, "swtpm_build": build, "boots": plan_boots(timeouts)}, indent=1))
            return 0
        if not SHA_RE.fullmatch(a.sha):
            return 2
        if spec:
            sys.path.insert(0, str(HERE))
            import propimage_images
            ctx = Context(a.sha, a.checkout, a.prefix, a.out, a.urls, bench=Path(a.bench), deadline_epoch=a.deadline_epoch, hardware=hardware_now(),
                          qemu=a.qemu, pins_path=Path(a.pins or PINS_PATH))
            return propimage_images.finish(propimage_images.run_images(ctx, spec, a.evidence_dir, gop=Path(a.gop_efi) if a.gop_efi else None), a.out)
        ctx = Context(a.sha, a.checkout, a.prefix, a.out, a.urls, bench=Path(a.bench), deadline_epoch=a.deadline_epoch, hardware=hardware_now(),
                      pins_path=Path(a.pins or PINS_PATH))
        return finish(run(ctx, a.evidence_dir), a.out)
    except Refused as exc:
        print("KF_PROPIMAGE_APPS_REFUSED " + clean(exc.reason, 80), flush=True)
        return 2
    except (ValueError, OSError) as exc:
        print("KF_PROPIMAGE_APPS_USAGE " + clean(exc, 100), flush=True)
        return 2


if __name__ == "__main__":
    sys.exit(main())
