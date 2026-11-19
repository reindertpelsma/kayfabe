#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""propimage_token_check.py SUBCOMMAND --sealed DIR|state.json --checklist FILE --run-root DIR ...  (prepare|start|report|end|status)

STATUS: LIVE, 2026-11-18 (GPU-free tooling; the VM, ssh and guest-PowerShell paths have NOT been run on hardware, only the planners,
the guards and the report scrubbing are tested: scripts/ci/test_propimage_token_check.py). Generic: it names no image; a sealed image is
a directory (or a state.json) given on the command line, and the per-image part is a checklist file.

The "untrusted party with a stolen image" check. Boots a THROWAWAY THIN OVERLAY of a sealed proprietary Windows image ONLINE (internet on,
the host's REAL clock, asserted by clocknet_assert.py), runs an in-guest INDICATOR REPORT, and lets a person try the apps from the
checklist, to see whether any cached login is still valid on a vendor's server. Local sign-out does not revoke a stored token; only a server
round-trip proves it (token_check/checklist_template.md).

  prepare   sealed files present, optional sha256 against manifest.json, free space, creates RUN_ROOT/runs/NAME with the overlay
            (qemu-img create -b SEALED -F qcow2: the sealed image is only a READ-ONLY backing file) and FRESH tpm/firmware copies
            restored from the sealed state. Refuses if the run directory exists.
  start     boots the overlay (own swtpm, broker window or gtk), waits for ssh, DETECTS the guest firewall and records
            'firewall in seal: yes/no' BEFORE any lift, optionally lifts the rules it can name (--lift-firewall), checks the guest clock,
            runs the indicator report, and detaches a holder that keeps the optional GPU lock until the guest is gone.
  report    re-runs the indicator report (saved under RUN_ROOT/evidence/NAME)
  end       clean Stop-Computer over ssh, then deletes the WHOLE run directory (overlay, tpm/firmware copies, key, logs)
  status    what is running

Nothing is hard-coded: paths and tools come from options or the environment (KF_TOKENCHECK_ROOT, _QEMU, _BROKER, _GOP_EFI, _GPU_LOCK,
_ACTIVE_LOG, _DESKTOP_USER). The sealed directory and its state are only read; the tool refuses a run root that is inside the sealed
directory. The report prints only booleans, counts, file names, sizes and modification times; every line is scrubbed again here (e-mail
addresses, long digit strings, long tokens). The tool never prints state.json, the ssh key or the guest password.
"""
import argparse
import base64
import fcntl
import hashlib
import json
import os
import pwd
import re
import shlex
import shutil
import signal
import subprocess
import sys
import time
import xml.etree.ElementTree as ET
from pathlib import Path

sys.dont_write_bytecode = True   # never leave a __pycache__ in a sealed or run directory
HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import clocknet_assert as cna  # noqa: E402  the real-clock-with-internet rule
import propimage as PI  # noqa: E402  state, argv, restore_files

TOOLS = HERE / "token_check"
DISTRO_QEMU = "/usr/bin/qemu-system-x86_64"
RESERVE_GB, GROWTH_GB = 20.0, 6.0
DESKTOP_GPU_PROCS = re.compile(r"gnome-shell|Xorg|Xwayland|gnome-|gsd-|nvkvm-display-broker|firefox|chrome|code|mutter|gjs|evolution|nautilus", re.I)
NAME_RX = re.compile(r"[A-Za-z0-9._-]{1,64}")
HOST_RX = re.compile(r"[A-Za-z0-9.-]{1,100}")
ENV_PREFIX = "KF_TOKENCHECK_"


class Refused(Exception):
    def __init__(self, msg, rc=2):
        super().__init__(msg)
        self.rc = rc


# ------------------------------------------------------------------ scrubbing
EMAIL_RX = re.compile(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}")
JWT_RX = re.compile(r"eyJ[A-Za-z0-9_.\-]{10,}")
GPU_RX = re.compile(r"GPU-[0-9a-f-]{20,}")
GUID_RX = re.compile(r"\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b")
LONG_RX = re.compile(r"[A-Za-z0-9_+/=\-]{40,}")
DIGITS_RX = re.compile(r"(bytes=\d+)|\d{9,}")     # ids, phone-like numbers, steam ids; byte counts are sizes and stay


def scrub(text):
    """Never let an e-mail address, a token-like string or a long number through (every line that is printed or saved)."""
    t = text or ""
    t = EMAIL_RX.sub("<account>", t)
    t = JWT_RX.sub("<jwt>", t)
    t = GPU_RX.sub("GPU-<uuid>", t)
    t = GUID_RX.sub("<guid>", t)
    t = LONG_RX.sub("<long>", t)
    return DIGITS_RX.sub(lambda m: m.group(1) or "<n>", t)


def say(msg=""):
    print(scrub(str(msg)), flush=True)


def sh(cmd, check=True, **kw):
    kw.setdefault("capture_output", True)
    kw.setdefault("text", True)
    p = subprocess.run(cmd, check=False, **kw)
    if check and p.returncode:
        raise Refused("command failed rc=%d: %s\n%s" % (p.returncode, shlex.join(map(str, cmd)), scrub((p.stderr or p.stdout)[-600:])))
    return p


def now():
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def stamp():
    return time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())


def free_gb(path):
    s = os.statvfs(path)
    return s.f_bavail * s.f_frsize / 1e9


# ------------------------------------------------------------------ checklist (the per-image part)
def checklist_problems(text):
    """A checklist must teach the measured lesson: SERVER ROUND-TRIP actions with PASS and FAIL texts ('shows signed in' is not proof)."""
    probs = []
    if not re.search(r"\bPASS\b", text):
        probs.append("no PASS text")
    if not re.search(r"\bFAIL\b", text):
        probs.append("no FAIL text")
    if not re.search(r"(?i)round[- ]trip", text):
        probs.append("no SERVER ROUND-TRIP section (a 'shows signed in' look is not proof; use an action that needs a valid server token)")
    return probs


def checklist_hosts(text):
    for m in re.finditer(r"(?im)^\s*Hosts:\s*(.+)$", text):      # the first line with a real (non-placeholder) host wins
        hosts = [h.strip() for h in m.group(1).split(",") if h.strip() and not h.strip().startswith("<")]
        if hosts:
            return hosts
    return []


def check_hosts(hosts):
    for h in hosts:
        if not HOST_RX.fullmatch(h):
            raise Refused("bad host name %r" % h)
    return hosts


def load_checklist(path):
    p = Path(path)
    if not p.is_file():
        raise Refused("checklist file missing: %s (copy token_check/checklist_template.md and fill it in)" % p)
    text = p.read_text()
    probs = checklist_problems(text)
    if probs:
        raise Refused("checklist %s refused: %s" % (p, "; ".join(probs)))
    return text


# ------------------------------------------------------------------ the sealed image (read only)
class Sealed:
    def __init__(self, image, state_p, manifest_p, member="image.qcow2"):
        self.image, self.state_p, self.manifest_p, self.member = Path(image), Path(state_p), (Path(manifest_p) if manifest_p else None), member

    def paths(self):
        return [p for p in (self.image, self.state_p, self.manifest_p) if p]


def resolve_sealed(sealed, image=None, manifest=None, member="image.qcow2"):
    """DIR with image.qcow2 + state.json (+ manifest.json), or a state.json file with its image next to it (or --image)."""
    p = Path(sealed)
    if p.is_dir():
        img, st, man = Path(image) if image else p / "image.qcow2", p / "state.json", (Path(manifest) if manifest else p / "manifest.json")
    elif p.is_file():
        img, st, man = Path(image) if image else p.parent / "image.qcow2", p, (Path(manifest) if manifest else p.parent / "manifest.json")
    else:
        raise Refused("sealed path does not exist: %s" % p)
    if not img.is_file():
        raise Refused("sealed image missing: %s" % img)
    if not st.is_file():
        raise Refused("sealed state.json missing: %s" % st)
    return Sealed(img, st, man if man.is_file() else None, member)


def load_state(sealed):
    st = json.loads(sealed.state_p.read_text())
    try:
        PI.validate_state(st, require_password=True)
        ok = PI.seed_matches(st)
    except (PI.PropError, KeyError, TypeError) as exc:
        raise Refused("state refused: %s" % exc)
    if not ok:
        raise Refused("state ids do not match the seed (%s)" % sealed.state_p)
    if not st["guest"].get("ssh_private_key"):
        raise Refused("state has no ssh private key (%s)" % sealed.state_p)
    return st


def is_inside(path, parent):
    path, parent = Path(path).resolve(), Path(parent).resolve()
    return path == parent or parent in path.parents


def run_dir_for(run_root, name):
    if not NAME_RX.fullmatch(name) or name in (".", ".."):
        raise Refused("bad run name %r" % name)
    return Path(run_root).resolve() / "runs" / name


def guard_outside_sealed(run, root, sealed):
    """The run root and the run directory (the only places this tool writes) may not be inside the directory of any sealed file, and the
    run directory may not contain a sealed file."""
    for sp in sealed.paths():
        sres = sp.resolve()
        if is_inside(run, sres.parent) or is_inside(root, sres.parent) or is_inside(sres, run):
            raise Refused("refusing to write into the sealed image's directory: run dir %s, root %s vs sealed %s" % (run, root, sres))


def default_name(sealed):
    base = sealed.image.resolve().parent.name or "sealed"
    return re.sub(r"[^A-Za-z0-9._-]", "_", base)[:64] or "sealed"


# ------------------------------------------------------------------ sha (gentle)
def sha_file(path, mbps=200.0):
    try:
        subprocess.run(["ionice", "-c3", "-p", str(os.getpid())], capture_output=True, check=False)
        os.nice(19)
    except (OSError, FileNotFoundError):
        pass
    h, done, t0 = hashlib.sha256(), 0, time.time()
    with open(path, "rb") as f:
        while True:
            b = f.read(4 << 20)
            if not b:
                break
            h.update(b)
            done += len(b)
            lag = done / (mbps * 1e6) - (time.time() - t0)
            if lag > 0:
                time.sleep(lag)
    return h.hexdigest()


def verify_sha(sealed, run_root, do_hash, hasher=sha_file):
    """-> (status, detail). Cache key = path + size + mtime, so a verified image is hashed once."""
    st = sealed.image.stat()
    key = "%s|%d|%d" % (sealed.image.resolve(), st.st_size, st.st_mtime_ns)
    cache_p = Path(run_root) / "sha_cache.json"
    cache = json.loads(cache_p.read_text()) if cache_p.exists() else {}
    want = None
    if sealed.manifest_p:
        want = json.loads(sealed.manifest_p.read_text()).get("members", {}).get(sealed.member)
    if key in cache:
        got = cache[key]
    elif do_hash:
        got = hasher(sealed.image)
        cache[key] = got
        cache_p.parent.mkdir(parents=True, exist_ok=True)
        cache_p.write_text(json.dumps(cache, indent=1))
    else:
        return "NOT-CHECKED", "sha not computed (--skip-sha or dry-run, no cache entry); manifest %s" % ("present" if want else "absent")
    if want is None:
        return "NO-MANIFEST", "sha256 %s... recorded in the cache only" % got[:12]
    if got != want:
        raise Refused("SHA MISMATCH: %s is %s..., manifest says %s..." % (sealed.image, got[:12], want[:12]))
    return "MATCH", "sha256 %s... equals the manifest" % got[:12]


# ------------------------------------------------------------------ apparmor (swtpm confinement is a host fact, not a pc fact)
def apparmor_covers(path, profiles="/sys/kernel/security/apparmor/profiles", enabled="/sys/module/apparmor/parameters/enabled",
                    local="/etc/apparmor.d/local/usr.bin.swtpm"):
    """-> (enforced, covered): does swtpm's AppArmor profile allow writing `path`?"""
    try:
        if Path(enabled).read_text().strip() != "Y" or "swtpm (enforce)" not in Path(profiles).read_text():
            return False, True
    except OSError:
        return False, True
    try:
        lines = Path(local).read_text().splitlines()
    except OSError:
        lines = []
    for line in lines:
        m = re.match(r"\s*(/\S+)\s+([a-z]+)\s*,", line)
        if m and "w" in m.group(2):
            rx = re.escape(m.group(1)).replace(r"\*\*", ".*").replace(r"\*", "[^/]*")
            if re.fullmatch(rx, str(path)):
                return True, True
    return True, False


# ------------------------------------------------------------------ argv (pure builders)
def overlay_cmd(image, overlay):
    """The only thing that creates a disk: a thin overlay whose backing file is the sealed image (opened read-only by QEMU)."""
    return ["qemu-img", "create", "-q", "-f", "qcow2", "-b", str(image), "-F", "qcow2", str(overlay)]


def build_guest_argv(state, sealed, run, name, qemu, no_gpu, sock=None, gop_efi=None, uid=1000, distro_qemu=DISTRO_QEMU):
    """The qemu argv of an ONLINE check: propimage mode `setup` (distro qemu + gtk, no GPU) or `install` (kf3 + broker): internet on, real clock.
    Refuses (Refused) unless the argv references exactly the overlay and carries the host's real clock with restrict=off."""
    overlay = run / "clone.qcow2"
    if no_gpu:
        argv = PI.build_argv(state, "setup", str(overlay), str(run), qemu=distro_qemu)
    else:
        if not qemu or not gop_efi or not sock:
            raise Refused("a kf3 start needs --qemu, --gop-efi and a broker socket (or use --no-gpu)")
        argv = PI.build_argv(state, "install", str(overlay), str(run), qemu=str(qemu), uid=uid, broker_sock=str(sock),
                             gop_efi=str(gop_efi), display="broker")
    i = argv.index("-name")
    argv[i + 1] = "token-check-" + name
    return argv, assert_online_real_clock(argv, sealed, run)


def assert_online_real_clock(argv, sealed, run, now_=None):
    try:
        clock, net = cna.assert_argv(argv, "propimage_token_check", now_)
    except cna.ClockNetViolation as exc:
        raise Refused(str(exc), 3)
    if net.off:
        raise Refused("the argv is OFFLINE (%s): a token check needs internet on" % net.why)
    if clock.faked or clock.vm_clock or clock.base is not None:
        raise Refused("the argv does not use the host's real clock")
    j = " ".join(str(a) for a in argv)
    if str(sealed.image) in j or str(sealed.image.resolve()) in j:
        raise Refused("the sealed image path is in the argv: only the overlay may be")
    if "restrict=off" not in j or str(run / "clone.qcow2") not in j:
        raise Refused("argv does not reference exactly the overlay with restrict=off")
    return "CLOCKNET_OK: internet on (restrict=off) + real clock (-rtc base=utc, clock=host)"


def swtpm_cmd(run, user_prefix=()):
    return list(user_prefix) + ["swtpm", "socket", "--tpm2", "--tpmstate", "dir=%s/tpm,mode=0600" % run, "--ctrl",
                                "type=unixio,path=%s/swtpm.sock,mode=0600" % run, "--log", "file=%s/run/swtpm.log,level=20" % run,
                                "--pid", "file=%s/run/swtpm.pid" % run, "--terminate", "--daemon"]


def ssh_base(st, key):
    return ["ssh", "-i", str(key), "-p", str(st["ids"]["net"]["ssh_port"]), "-o", "StrictHostKeyChecking=no", "-o", "UserKnownHostsFile=/dev/null",
            "-o", "LogLevel=ERROR", "-o", "ConnectTimeout=15", "-o", "BatchMode=yes", "-o", "IdentitiesOnly=yes", "%s@127.0.0.1" % st["guest"]["user"]]


def ps_wrap(script, prefix=""):
    """ONE stdin line: Invoke-Expression of the base64 script, so multi-line PowerShell survives `-Command -` (UNTESTED in a guest)."""
    b64 = base64.b64encode((prefix + "\n" + script).encode("utf-8")).decode()
    return "Invoke-Expression ([Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('%s')))\n" % b64


def hosts_prefix(hosts):
    return "$Hosts = @(%s)" % ",".join("'%s'" % h for h in check_hosts(list(hosts)))


# ------------------------------------------------------------------ firewall record (pure)
def parse_kv(out):
    return dict(l.split("=", 1) for l in out.splitlines() if "=" in l and l.split("=", 1)[0].startswith(("FW_", "GUEST_")))


def firewall_in_seal(out):
    """-> (line, present). `line` is 'firewall in seal: yes|no (...)'. Computed from the detect output BEFORE any lift; `present` says
    whether a block that this tool can name exists (kf-offline group, kayfabe-*/kf-* outbound Block rule, default outbound Block)."""
    vals = parse_kv(out)

    def num(k):
        return int(re.sub(r"\D", "", vals.get(k, "0")) or 0)
    evid = []
    if num("FW_KF_OFFLINE_GROUP_RULES"):
        evid.append("kf-offline group with %d rule(s)" % num("FW_KF_OFFLINE_GROUP_RULES"))
    if num("FW_KAYFABE_BLOCK_RULES_ENABLED"):
        evid.append("%d enabled kayfabe outbound Block rule(s)" % num("FW_KAYFABE_BLOCK_RULES_ENABLED"))
    if num("FW_OUTBOUND_DEFAULT_BLOCK_PROFILES"):
        evid.append("default outbound Block on %d profile(s)" % num("FW_OUTBOUND_DEFAULT_BLOCK_PROFILES"))
    failed = [k for k, v in vals.items() if k.startswith("FW_SEAL_TCP443_") and v.startswith("FAILED")]
    if failed:
        evid.append("outbound TCP 443 FAILED before any lift (%d probe(s))" % len(failed))
    line = "firewall in seal: %s%s" % ("yes" if evid else "no", (" (" + "; ".join(evid) + ")") if evid else " (no block found, outbound TCP 443 open)")
    return line, bool(num("FW_KF_OFFLINE_GROUP_RULES") or num("FW_KAYFABE_BLOCK_RULES_ENABLED") or num("FW_OUTBOUND_DEFAULT_BLOCK_PROFILES"))


# ------------------------------------------------------------------ processes (own pids only: the command line names the run dir)
def pids_for(run):
    out = []
    for p in Path("/proc").iterdir():
        if p.name.isdigit() and int(p.name) != os.getpid():
            try:
                cl = (p / "cmdline").read_bytes().replace(b"\0", b" ").decode("utf-8", "replace")
            except OSError:
                continue
            if str(run) + "/" in cl:
                out.append((int(p.name), cl))
    return out


def qemu_pid(run):
    for pid, cl in pids_for(run):
        if "qemu-system-x86_64" in cl.split(" -")[0] and "clone.qcow2" in cl:
            return pid
    return None


def kill_broker(sockname):
    for p in Path("/proc").iterdir():
        if p.name.isdigit() and int(p.name) != os.getpid():
            try:
                cl = (p / "cmdline").read_bytes().replace(b"\0", b" ").decode("utf-8", "replace")
            except OSError:
                continue
            if sockname in cl and "propimage_token_check" not in cl:
                try:
                    os.kill(int(p.name), signal.SIGTERM)
                except OSError:
                    pass


def own_kill(run, force_after=30):
    for sig in (signal.SIGTERM, signal.SIGKILL):
        for pid, _ in pids_for(run):
            try:
                os.kill(pid, sig)
            except OSError:
                pass
        t0 = time.time()
        while pids_for(run) and time.time() - t0 < force_after:
            time.sleep(1)
        if not pids_for(run):
            return


def gpu_busy_reasons(own_run=None):
    why = []
    p = sh(["nvidia-smi", "-q", "-x"], check=False, timeout=60)
    if p.returncode != 0:
        return ["nvidia-smi does not answer"]
    try:
        for pi_ in ET.fromstring(p.stdout).iter("process_info"):
            nm = pi_.findtext("process_name") or "?"
            if not DESKTOP_GPU_PROCS.search(nm):
                why.append("GPU process pid=%s %s" % (pi_.findtext("pid"), os.path.basename(nm)))
    except ET.ParseError:
        why.append("nvidia-smi xml unreadable")
    for line in sh(["pgrep", "-af", "qemu-system"], check=False).stdout.splitlines():
        if "kf3-gpu" in line and not (own_run and str(own_run) in line):
            why.append("kf3 qemu running: pid %s" % line.split()[0])
    return why


def take_lock(path, wait):
    fd = os.open(path, os.O_RDWR | os.O_CREAT)
    t0 = time.time()
    while True:
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            return fd
        except OSError:
            if time.time() - t0 >= wait:
                raise Refused("GPU LOCK HELD (%s): not waiting (use --wait N)" % path, 7)
            time.sleep(10)


def active(a, tag, msg):
    if a.active_log:
        with open(a.active_log, "a") as f:
            f.write("%s propimage-token-check %s %s\n" % (now(), tag, msg))


# ------------------------------------------------------------------ context
def env_default(name, default=None):
    return os.environ.get(ENV_PREFIX + name, default)


def context(a):
    """Resolve everything read-only; no write happens here."""
    if not a.run_root:
        raise Refused("--run-root DIR (or KF_TOKENCHECK_ROOT) is required: it holds runs/ and evidence/ and nothing else")
    sealed = resolve_sealed(a.sealed, a.image, a.manifest)
    name = a.name or default_name(sealed)
    run = run_dir_for(a.run_root, name)
    guard_outside_sealed(run, Path(a.run_root).resolve(), sealed)
    text = load_checklist(a.checklist)
    hosts = check_hosts([h.strip() for h in a.hosts.split(",") if h.strip()] if a.hosts else checklist_hosts(text))
    st = load_state(sealed)
    uid = pwd.getpwnam(a.desktop_user).pw_uid if a.desktop_user else os.getuid()
    xdg = a.xdg_runtime_dir or (os.environ.get("XDG_RUNTIME_DIR") if not a.desktop_user else None) or "/run/user/%d" % uid
    return dict(a=a, sealed=sealed, st=st, name=name, run=run, root=Path(a.run_root).resolve(), hosts=hosts, uid=uid, xdg=xdg,
                checklist=Path(a.checklist), user_prefix=(["runuser", "-u", a.desktop_user, "--"] if a.desktop_user else []),
                sock=Path(xdg) / "nvkvm" / ("display-token-%s.sock" % name))


def plan_text(c, argv, extra):
    run, st = c["run"], c["st"]
    lines = ["PLAN token check %s (%s)" % (c["name"], "DRY-RUN" if c["a"].dry_run else "real"),
             "  sealed image : %s (%.1f GB, read-only backing, never written)" % (c["sealed"].image, c["sealed"].image.stat().st_size / 1e9),
             "  state        : %s (not printed)" % c["sealed"].state_p,
             "  overlay      : %s  (thin qcow2, backing = sealed)" % (run / "clone.qcow2"),
             "  tpm/firmware : fresh copies from the sealed state under %s/{tpm,firmware}" % run,
             "  qemu         : %s" % argv[0],
             "  ssh          : 127.0.0.1:%d as %s (key written to the run dir at prepare, deleted with it at end)" % (st["ids"]["net"]["ssh_port"], st["guest"]["user"]),
             "  hosts probed : %s (TCP 443 connect only)" % (", ".join(c["hosts"]) or "none"),
             "  checklist    : %s" % c["checklist"]] + ["  " + e for e in extra]
    return "\n".join(lines)


def argv_for(c):
    a = c["a"]
    return build_guest_argv(c["st"], c["sealed"], c["run"], c["name"], a.qemu or "/NO/KF3/QEMU/GIVEN", a.no_gpu, sock=c["sock"],
                            gop_efi=c["run"] / "firmware/kf-gop.signed.efi", uid=c["uid"])


# ------------------------------------------------------------------ subcommands
def gpu_inputs(a):
    return (("kf3 qemu", a.qemu), ("boot GOP", a.gop_efi), ("display broker", a.broker))


def cmd_prepare(c):
    a, run = c["a"], c["run"]
    if run.exists():
        raise Refused("%s exists (an earlier run was not ended): run `end` first" % run)
    extra = []
    qi = sh(["qemu-img", "info", "-U", "--output=json", "--backing-chain", str(c["sealed"].image)], check=False)
    if qi.returncode:
        raise Refused("qemu-img info failed on the sealed image: " + qi.stderr[-300:])
    chain = json.loads(qi.stdout)
    chain = chain if isinstance(chain, list) else [chain]
    if chain[0].get("format") != "qcow2":
        raise Refused("sealed image is not qcow2")
    extra.append("sealed chain  : %d image(s), virtual size %.0f GiB%s" % (len(chain), chain[0]["virtual-size"] / 2**30,
                                                                         "" if len(chain) == 1 else " (has a backing file: fine, never written)"))
    status, detail = verify_sha(c["sealed"], c["root"], not (a.skip_sha or a.dry_run))
    extra.append("sha           : %s (%s)" % (status, detail))
    anc = c["root"]
    while not anc.exists():
        anc = anc.parent
    fgb = free_gb(anc)
    ok = fgb >= GROWTH_GB + RESERVE_GB
    extra.append("space         : free %.0f GB, need %.0f GB growth + %.0f GB reserve -> %s" % (fgb, GROWTH_GB, RESERVE_GB, "OK" if ok else "TOO LITTLE"))
    if not ok and not a.dry_run:
        raise Refused("not enough free space: %.0f GB" % fgb)
    enforced, covered = apparmor_covers(run / "tpm")
    if enforced and not covered:
        msg = ("AppArmor (swtpm, enforce) does not allow writes under %s: add `%s/** rwk,` to /etc/apparmor.d/local/usr.bin.swtpm and reload the profile "
               "(a system change: yours to decide), or pick a --run-root that is already covered." % (c["root"], c["root"]))
        if not a.dry_run:
            raise Refused(msg)
        extra.append("APPARMOR BLOCKER: " + msg)
    else:
        extra.append("apparmor     : swtpm may write the run dir (%s)" % ("rule found" if enforced else "not enforced"))
    argv, banner = argv_for(c)
    extra.append(banner)
    if not a.no_gpu:
        for what, p in gpu_inputs(a):
            if not p or not os.path.exists(p):
                extra.append("BLOCKER: %s missing (%s)" % (what, p))
    say(plan_text(c, argv, extra))
    if a.dry_run:
        say("DRY-RUN: nothing written. Would create %s/{clone.qcow2,tpm,firmware,run,id_kf,plan.json}" % run)
        return 0
    if not a.no_gpu and any(not p or not os.path.exists(p) for _, p in gpu_inputs(a)):
        raise Refused("kf3 qemu, boot GOP or display broker missing (see the plan; or use --no-gpu)")
    run.mkdir(parents=True)
    (run / "run").mkdir()
    PI.restore_files(c["st"], str(run))             # FRESH tpm + OVMF vars from the sealed state, never from an earlier run
    if a.gop_efi and os.path.isfile(a.gop_efi):
        (run / "firmware").mkdir(exist_ok=True)
        shutil.copyfile(a.gop_efi, run / "firmware/kf-gop.signed.efi")
    key = run / "id_kf"
    key.write_text(c["st"]["guest"]["ssh_private_key"].rstrip("\n") + "\n")
    key.chmod(0o600)
    sh(overlay_cmd(c["sealed"].image, run / "clone.qcow2"))
    info = json.loads(sh(["qemu-img", "info", "-U", "--output=json", str(run / "clone.qcow2")]).stdout)
    if Path(info.get("full-backing-filename", "")).resolve() != c["sealed"].image.resolve():
        raise Refused("overlay does not reference the sealed image")
    if a.desktop_user:
        sh(["chown", "-R", "%s:%s" % (a.desktop_user, a.desktop_user), str(run)], check=False)
    (run / "plan.json").write_text(json.dumps({"name": c["name"], "created": now(), "qemu": argv[0], "no_gpu": a.no_gpu}, indent=1) + "\n")
    say("prepared: overlay %s (%d bytes now). Next: start" % (run / "clone.qcow2", (run / "clone.qcow2").stat().st_size))
    return 0


def ssh_ps(st, key, script, prefix="", timeout=900):
    p = subprocess.run(ssh_base(st, key) + ["powershell", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", "-"],
                       input=ps_wrap(script, prefix), text=True, capture_output=True, timeout=timeout, errors="replace")
    return p.returncode, scrub(p.stdout.replace("\r", "")), scrub(p.stderr.replace("\r", "")[-600:])


def wait_ssh(st, key, timeout, alive):
    t0 = time.time()
    while time.time() - t0 < timeout:
        if not alive():
            raise Refused("qemu is gone while waiting for ssh (see run/qemu.log)", 4)
        try:
            p = sh(ssh_base(st, key) + ["echo", "up"], check=False, timeout=30)
            if p.returncode == 0 and "up" in p.stdout:
                return int(time.time() - t0)
        except subprocess.TimeoutExpired:
            pass
        time.sleep(8)
    raise Refused("ssh not up after %d s (guest left running; use `end --force` to stop it)" % timeout, 4)


def save_evidence(c, label, text):
    ev = c["root"] / "evidence" / c["name"]
    ev.mkdir(parents=True, exist_ok=True)
    fp = ev / ("%s_%s.txt" % (stamp(), label))
    fp.write_text(scrub(text))
    return fp


def run_report(c, label):
    a = c["a"]
    script = (TOOLS / "indicator_report.ps1").read_text()
    if a.dry_run:
        say("DRY-RUN report: would ssh 127.0.0.1:%d, send %d bytes of PowerShell as one base64 line, hosts %s"
            % (c["st"]["ids"]["net"]["ssh_port"], len(script), c["hosts"]))
        return 0
    rc, out, err = ssh_ps(c["st"], c["run"] / "id_kf", script, hosts_prefix(c["hosts"]), 900)
    fp = save_evidence(c, label + "_report", out)
    say(out)
    if "KF_TC_REPORT_END" not in out:
        say("WARNING: the report did not reach its end marker (rc=%d stderr: %s)" % (rc, err))
    say("report saved: %s" % fp)
    return 0


def firewall_step(c):
    """Detect and RECORD first, lift afterwards (and only with --lift-firewall)."""
    a, st, key = c["a"], c["st"], c["run"] / "id_kf"
    rc, out, err = ssh_ps(st, key, (TOOLS / "fw_detect.ps1").read_text(), "", 120)
    line, present = firewall_in_seal(out)
    say("--- firewall detect (rc=%d)\n%s" % (rc, out.strip()))
    say(line)
    fp = save_evidence(c, "firewall_in_seal", line + "\n" + out)
    say("recorded BEFORE any lift: %s" % fp)
    guest_utc = parse_kv(out).get("GUEST_UTC", "")
    if guest_utc:
        try:
            g = time.mktime(time.strptime(guest_utc, "%Y-%m-%dT%H:%M:%SZ")) - time.timezone
            say("guest clock: %s, host-now delta %+d s -> %s" % (guest_utc, g - time.time(), "REAL" if abs(g - time.time()) < 600 else "NOT REAL: stop and investigate (internet needs the real clock)"))
        except ValueError:
            pass
    if not (a.lift_firewall or a.lift_all):
        say("firewall: not lifted (no --lift-firewall). %s" % ("A block exists in the seal: the vendor hosts will probably be unreachable." if present else "Nothing to lift."))
        return
    if not present and not a.lift_all:
        say("firewall: no block that this tool can name was found: nothing lifted")
        return
    rc, out2, err = ssh_ps(st, key, (TOOLS / "fw_lift.ps1").read_text(), "$LiftAll = $%s" % ("true" if a.lift_all else "false"), 180)
    say("--- firewall lift (rc=%d, inbound ssh kept)\n%s" % (rc, out2.strip()))
    save_evidence(c, "firewall_lift", out2)


def cmd_start(c):
    a, run = c["a"], c["run"]
    argv, banner = argv_for(c)
    if a.dry_run:
        say(plan_text(c, argv, [banner, "gpu lock      : %s" % ("none (--no-gpu)" if a.no_gpu else (a.gpu_lock or "none given")),
                                "broker        : %s --socket %s" % (a.broker, c["sock"]), "qemu argv     : " + shlex.join(argv)[:1500]]))
        say("DRY-RUN steps: optional lock -> host checks -> swtpm -> %s -> qemu -> wait ssh (<=900 s) -> firewall detect+record -> optional lift -> clock check -> indicator report -> detach holder"
            % ("gtk window" if a.no_gpu else "broker"))
        run_report(c, "dry")
        return 0
    if not (run / "plan.json").is_file():
        raise Refused("not prepared: run `prepare` first")
    if not a.no_gpu and any(not p or not os.path.exists(p) for _, p in gpu_inputs(a)):
        raise Refused("kf3 qemu, boot GOP or display broker missing: pass --qemu, --gop-efi and --broker (or use --no-gpu)")
    if qemu_pid(run):
        raise Refused("already running (pid %d)" % qemu_pid(run))
    sj = run / "session.json"
    if sj.exists():
        raise Refused("session.json exists: run `end` first")
    port = c["st"]["ids"]["net"]["ssh_port"]
    if sh(["ss", "-ltn", "sport = :%d" % port], check=False).stdout.count("\n") > 1:
        raise Refused("ssh forward port %d is already in use (another VM of this image is running)" % port)
    if not (Path(c["xdg"]) / "wayland-0").exists() and not os.environ.get("WAYLAND_DISPLAY"):
        raise Refused("no wayland-0 socket in %s: no desktop session to show the window on" % c["xdg"])
    lockfd = None
    if not a.no_gpu:
        if a.gpu_lock:
            lockfd = take_lock(a.gpu_lock, a.wait)
        why = [w for w in gpu_busy_reasons(run) if "kf3 qemu" in w]
        if why:
            raise Refused("a kf3 qemu is already running: " + "; ".join(why))
        probe = "import ctypes,sys; r=ctypes.CDLL('libcuda.so.1').cuInit(0); sys.exit(0 if r==0 else 1)"
        if sh(["python3", "-c", probe], check=False, timeout=90).returncode:
            raise Refused("host cuInit not ready", 8)
    active(a, "%s START" % c["name"], "%sONLINE throwaway overlay, real clock" % ("(no GPU) " if a.no_gpu else ""))
    rc, procs = 1, {}
    try:
        key = run / "id_kf"
        sh(swtpm_cmd(run, c["user_prefix"]))
        for _ in range(50):
            if (run / "swtpm.sock").exists():
                break
            time.sleep(0.2)
        else:
            raise Refused("swtpm made no socket (AppArmor? see %s/run/swtpm.log)" % run, 3)
        gtk = ["env", "XDG_RUNTIME_DIR=" + c["xdg"], "WAYLAND_DISPLAY=" + (a.wayland_display or "wayland-0"), "GDK_BACKEND=wayland",
               "DBUS_SESSION_BUS_ADDRESS=unix:path=%s/bus" % c["xdg"]]
        title = "kayfabe TOKEN CHECK %s (ONLINE, throwaway overlay)" % c["name"]
        if not a.no_gpu:
            sh(["install", "-d", "-m", "0700"] + (["-o", a.desktop_user] if a.desktop_user else []) + [str(Path(c["xdg"]) / "nvkvm")])
            c["sock"].unlink(missing_ok=True)
            bp = subprocess.Popen(c["user_prefix"] + gtk + [a.broker, "--socket", str(c["sock"]), "--backend", "auto", "--persist", "--title", title],
                                  stdout=open(run / "run/broker.log", "w"), stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL, start_new_session=True)
            procs["broker"] = bp.pid
            for _ in range(75):
                if c["sock"].exists():
                    break
                if bp.poll() is not None:
                    raise Refused("broker exited rc=%s: %s" % (bp.returncode, (run / "run/broker.log").read_text()[-300:]), 3)
                time.sleep(0.2)
            else:
                raise Refused("broker made no socket", 3)
            cmd = ["nice", "-n", "10"] + argv
        else:
            cmd = c["user_prefix"] + gtk + ["nice", "-n", "10"] + argv
        (run / "run/last_cmd.txt").write_text(shlex.join(argv) + "\n")
        q = subprocess.Popen(cmd, env=dict(os.environ), stdout=open(run / "run/qemu.log", "ab"), stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL,
                             start_new_session=True)
        time.sleep(3)
        if q.poll() is not None:
            raise Refused("qemu exited rc=%s: %s" % (q.returncode, scrub((run / "run/qemu.log").read_text()[-500:])), 3)
        for _ in range(30):
            if qemu_pid(run):
                break
            time.sleep(0.5)
        procs["qemu"] = qemu_pid(run)
        sj.write_text(json.dumps({"started": now(), "pids": procs, "lock": a.gpu_lock if lockfd is not None else None, "name": c["name"]}) + "\n")
        say("qemu pid %s up; waiting for ssh ..." % procs["qemu"])
        up = wait_ssh(c["st"], key, 900, lambda: qemu_pid(run) is not None)
        say("GUEST UP (ssh after %d s). %s" % (up, banner))
        firewall_step(c)
        run_report(c, "start")
        say("\nUse the window titled '%s'. Checklist: %s. When finished: end" % (title, c["checklist"]))
        rc = 0
    except BaseException as e:
        rc = e.rc if isinstance(e, Refused) else (e.code if isinstance(e, SystemExit) and isinstance(e.code, int) else 1)
        say("start failed (rc=%s): cleaning up own processes" % rc)
        own_kill(run, 20)
        active(a, "%s END" % c["name"], "start failed rc=%s" % rc)
        if lockfd is not None:
            fcntl.flock(lockfd, fcntl.LOCK_UN)
        raise
    sys.stdout.flush()
    if os.fork() != 0:      # detach: the holder keeps the flock (inherited fd) and writes the END line when the guest is gone
        os._exit(0)
    os.setsid()
    dn = os.open(os.devnull, os.O_RDWR)
    for fd_ in (0, 1, 2):
        os.dup2(dn, fd_)
    (run / "run/holder.pid").write_text(str(os.getpid()))
    while qemu_pid(run):
        time.sleep(5)
    time.sleep(3)
    kill_broker("display-token-%s.sock" % c["name"])
    for pid, _ in pids_for(run):
        try:
            os.kill(pid, signal.SIGTERM)
        except OSError:
            pass
    try:
        sj.write_text(json.dumps(dict(json.loads(sj.read_text()), ended=now())) + "\n")
    except OSError:
        pass
    active(a, "%s END" % c["name"], "guest gone")
    if lockfd is not None:
        fcntl.flock(lockfd, fcntl.LOCK_UN)
    os._exit(0)


def cmd_report(c):
    if not c["a"].dry_run and not qemu_pid(c["run"]):
        raise Refused("no running guest for %s" % c["name"])
    return run_report(c, "report")


def remove_run_dir(run, root):
    """Delete the WHOLE run directory (overlay, tpm/firmware copies, key, logs) after the safety checks. Nothing else is touched."""
    run, root = Path(run), Path(root)
    if run.is_symlink() or run.resolve().parent != (root / "runs").resolve():
        raise Refused("refusing to delete %s (not directly under %s/runs)" % (run, root))
    for pth in (run / "clone.qcow2", run / "id_kf"):
        pth.unlink(missing_ok=True)
    shutil.rmtree(run)


def cmd_end(c):
    a, run = c["a"], c["run"]
    if a.dry_run:
        say("DRY-RUN end: would ssh Stop-Computer (127.0.0.1:%d), wait <=180 s for the guest and the holder, %s, then delete the whole run directory %s; "
            "the sealed image is never touched." % (c["st"]["ids"]["net"]["ssh_port"], "kill own pids if --force", run))
        return 0
    if not run.exists():
        say("nothing to end: %s does not exist" % run)
        return 0
    if run.is_symlink() or run.resolve().parent != (c["root"] / "runs").resolve():
        raise Refused("refusing to delete %s (not directly under %s/runs)" % (run, c["root"]))
    if qemu_pid(run):
        rc, out, err = ssh_ps(c["st"], run / "id_kf", "Stop-Computer -Force", "", 90)
        say("Stop-Computer sent (rc=%d)" % rc)
        for _ in range(45):
            if not qemu_pid(run):
                break
            time.sleep(4)
        if qemu_pid(run):
            if not a.force:
                raise Refused("guest still running after 180 s; not deleting. Use --force to kill own pids.", 5)
            say("--force: killing own pids")
            own_kill(run)
    hp, sjp = run / "run/holder.pid", run / "session.json"
    for _ in range(40):                    # the holder writes the END line and releases the lock when the guest is gone
        alive = hp.exists() and Path("/proc/%s" % hp.read_text().strip()).exists()
        if not sjp.exists() or "ended" in sjp.read_text() or not alive:
            break
        time.sleep(2)
    kill_broker("display-token-%s.sock" % c["name"])
    if sjp.exists() and "ended" not in sjp.read_text():
        active(a, "%s END" % c["name"], "holder gone without END line; written by `end`")
    own_kill(run, 10)                      # swtpm / broker leftovers (their command line names the run dir)
    remove_run_dir(run, c["root"])
    say("ended: %s deleted (overlay, tpm/firmware copies, key, logs). Scrubbed reports stay in %s/evidence/%s." % (run, c["root"], c["name"]))
    return 0


def cmd_status(c):
    run = c["run"]
    say("run dir: %s exists=%s; qemu pid=%s" % (run, run.exists(), qemu_pid(run) if run.exists() else None))
    if (run / "session.json").exists():
        say((run / "session.json").read_text().strip())
    return 0


def parser():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0], formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("cmd", choices=["prepare", "start", "report", "end", "status"])
    ap.add_argument("--sealed", required=True, help="sealed image DIRECTORY (image.qcow2, state.json, optional manifest.json) or a state.json file")
    ap.add_argument("--checklist", required=True, help="the per-image checklist (copy token_check/checklist_template.md); optional 'Hosts: a, b' line")
    ap.add_argument("--run-root", default=env_default("ROOT"), help="holds runs/NAME, evidence/NAME and sha_cache.json [KF_TOKENCHECK_ROOT]")
    ap.add_argument("--name", help="run name (default: the sealed directory's name)")
    ap.add_argument("--image", help="sealed image file when it is not image.qcow2 next to the state")
    ap.add_argument("--manifest", help="manifest.json when it is not next to the state")
    ap.add_argument("--hosts", help="identity-server host names to probe (TCP 443), comma separated; overrides the checklist's Hosts: line")
    ap.add_argument("--qemu", default=env_default("QEMU"), help="the kf3 qemu binary [KF_TOKENCHECK_QEMU]")
    ap.add_argument("--broker", default=env_default("BROKER"), help="the display broker binary [KF_TOKENCHECK_BROKER]")
    ap.add_argument("--gop-efi", default=env_default("GOP_EFI"), help="the boot GOP efi [KF_TOKENCHECK_GOP_EFI]")
    ap.add_argument("--no-gpu", action="store_true", help="distro qemu + gtk window: no kf3, no broker, no lock")
    ap.add_argument("--gpu-lock", default=env_default("GPU_LOCK"), help="a lock file to flock for the run (GPU runs) [KF_TOKENCHECK_GPU_LOCK]")
    ap.add_argument("--active-log", default=env_default("ACTIVE_LOG"), help="append START/END lines here [KF_TOKENCHECK_ACTIVE_LOG]")
    ap.add_argument("--desktop-user", default=env_default("DESKTOP_USER"), help="run swtpm, broker and gtk qemu as this user (needs root) [KF_TOKENCHECK_DESKTOP_USER]")
    ap.add_argument("--xdg-runtime-dir", help="the desktop session's XDG_RUNTIME_DIR (default: ours, or /run/user/UID)")
    ap.add_argument("--wayland-display", help="default wayland-0")
    ap.add_argument("--lift-firewall", action="store_true", help="after recording the seal's firewall state, lift the rules this tool can name (kf-offline group, kayfabe-*/kf-* outbound Block)")
    ap.add_argument("--lift-all", action="store_true", help="also lift every other outbound Block rule and the default-outbound Block (implies --lift-firewall)")
    ap.add_argument("--skip-sha", action="store_true")
    ap.add_argument("--dry-run", action="store_true", help="no VM, no lock, no writes (prepare writes nothing; the plan and the clock/net assert are printed)")
    ap.add_argument("--force", action="store_true", help="end: kill own pids if the clean stop did not work")
    ap.add_argument("--wait", type=int, default=0, help="lock wait in seconds (0 = fail fast)")
    return ap


def main(argv=None):
    a = parser().parse_args(argv)
    if a.no_gpu and a.gpu_lock:
        a.gpu_lock = None
    try:
        c = context(a)
        return {"prepare": cmd_prepare, "start": cmd_start, "report": cmd_report, "end": cmd_end, "status": cmd_status}[a.cmd](c)
    except Refused as exc:
        print("REFUSED: " + scrub(str(exc)), file=sys.stderr, flush=True)
        return exc.rc


if __name__ == "__main__":
    sys.exit(main())
