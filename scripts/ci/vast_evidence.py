#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Box-side evidence bundle for the Vast CI broker (stdlib only).

STATUS: LIVE, 2026-11-06 (GPU-free tests only: test_vast_evidence.py, evidence_mutants.py; no
real Worker, no real R2 and no box ran this).

Why: a rented box is destroyed after its grace period and the runner shows a bounded part of the
log, so the evidence of a failed gpu-vast run (the gate's /root/prov bundle, the Windows
screenshots, the host dmesg) was lost with the box. This helper, run by vast_onstart.sh after the
payload has ended (any exit status, any timeout) and before the result is reported, builds ONE
bounded, scrubbed bundle and uploads it, with the run capability, to POST <KF_BROKER_URL>/evidence/
<run>-<attempt> (ci/vast-broker/src/evidence.js). Never part of a verdict: every failure here is
one printed line, the exit status is always 0 and the whole step is bounded in time.

    vast_evidence.py run [--log LOG] [--out DIR] [--budget SECONDS] [--no-upload]

Bundle (a .tgz, at most BUNDLE_CAP bytes compressed and RAW_CAP bytes unpacked), in the order it is
kept when the cap binds (the LAST tier is dropped first; within a tier the oldest, then the
largest-index item):
  tier 0  MANIFEST.json, host state (free, df, ps of qemu/swtpm, nvidia-smi -q header, uname, date),
          the result file                                              never dropped
  tier 1  the payload log tail, the gate/provision `*.log` verdict logs   never dropped, but the
          log tail is halved (newest bytes kept) until the bundle fits
  tier 2  host dmesg -T tail, the stage `*.run` outputs, the newest `timeout_*` screendump
  tier 3  the gate/provision evidence tgz and directories (expanded, text), the Windows VM logs,
          the newest 3 screenshots
  tier 4  bench stage logs, collected guest evidence
  tier 5  older screenshots (up to 6 in all), gzip'd VM logs                    dropped first
Scrubbed (text, in place, counted in the manifest): GPU/board UUIDs (a distinct synthetic UUID per
value, as scripts/bench/box/trio_stage.sh does), serial numbers, MAC addresses (QEMU's own
52:54:00:* kept), public IPv4/IPv6 addresses. Screenshots are not scrubbed (pixels); the bundle is
private (R2, coordinator only). Scanned, on the RAW bytes of every member, before anything is packed
(a hit REFUSES the whole upload and prints the class and member, never the value): the run token,
the broker URL and its secret path, the Access pair, the runner nonce, the guest password and the
harness ssh key (read from $WINVM_ROOT/*/secrets, the same files windows_gate.sh scans for), a Vast
API key, JWT-shaped strings, `Bearer <token>`, presigned-URL parameters, private-key blocks, GitHub
tokens and URLs with credentials. Never collected: secrets/, tpm/, firmware/, disk/, ssh keys.

Work order and limits (review of PR #274): the files are collected first, the host commands last,
each in its own process group with a hard timeout (a hung GPU costs only its command); no symlink is
followed (lstat, O_NOFOLLOW); every member is scanned at full length, with the bytes before any cut,
before it is cut, and so are the file names and MANIFEST.json (a secret-shaped NAME is printed as a
hash); a 16-character prefix or suffix of a long secret is a hit; the scan is linear in the input.
Details: scripts/ci/VAST_PAYLOAD.md, "Evidence bundle".
"""

import argparse
import gzip
import hashlib
import io
import ipaddress
import json
import os
import posixpath
import re
import signal
import ssl
import stat
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.error
import urllib.request
import zlib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import vast_report as contract  # noqa: E402  (the one definition of the box environment contract)

BUNDLE_CAP = 8 * 1024 * 1024  # compressed; the Worker refuses more (src/evidence.js)
RAW_CAP = 48 * 1024 * 1024  # unpacked
FIT = 0.97  # the estimate may use this share of BUNDLE_CAP before the real archive is built
LOG_CAP = 3 * 1024 * 1024
FILE_CAP = 1024 * 1024
DMESG_CAP = 256 * 1024
VM_LOG_CAP = 512 * 1024
SHOT_COUNT, SHOT_MAX = 6, 2 * 1024 * 1024
MEMBER_READ_CAP = 16 * 1024 * 1024
ARCHIVE_UNPACK_CAP = 128 * 1024 * 1024
ARCHIVE_INPUT_CAP = 256 * 1024 * 1024  # compressed bytes read from one evidence tgz
MAX_FILES = 600
LEAD = 8192  # bytes read before a cut, scanned too (a secret that crosses a cut)
COMMAND_TIMEOUT = 6.0  # seconds per host command, then its whole process group is killed
COMMAND_PHASE = 25.0  # seconds for all host commands together
FRAGMENT = 16  # a prefix or suffix of a secret this long is a hit too
META_MAX = 4096  # bytes of the X-KF-Evidence-Meta header (the Worker's cap)
USER_AGENT = contract.USER_AGENT
FORBIDDEN_PARTS = {"secrets", "tpm", "firmware", "disk", ".ssh", "unattend.d"}
FORBIDDEN_NAME = re.compile(
    r"(?:^id_[A-Za-z0-9_]+$|\.(?:pem|key|qcow2|img|iso|vhd|vhdx|raw)$|^win_password$|"
    r"^unattend|^swtpm|\.vast_api_key$|^vast_api_key$)"
)
TEXT_SUFFIX = (".log", ".txt", ".out", ".err", ".run", ".json", ".jsonl", ".md", ".env", ".tsv")


class ScanHit(Exception):
    def __init__(self, label, member):
        super().__init__(label)
        self.label, self.member = label, member


def lstat(path):
    try:
        return os.lstat(path)
    except OSError:
        return None


def is_plain_file(path):
    """A regular file that is not a symlink (lstat: nothing is followed)."""
    st = lstat(path)
    return st is not None and stat.S_ISREG(st.st_mode)


def is_plain_dir(path):
    st = lstat(path)
    return st is not None and stat.S_ISDIR(st.st_mode)


def open_plain(path):
    """Open a regular file without following a symlink at the last component; never block."""
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC)
    try:
        if not stat.S_ISREG(os.fstat(fd).st_mode):
            raise OSError("not a regular file")
    except OSError:
        os.close(fd)
        raise
    return os.fdopen(fd, "rb")


def safe_name(name, limit=120):
    """A member name that cannot carry anything but a path: [A-Za-z0-9._/-], no `..`, no leading /."""
    parts = []
    for part in str(name).replace("\\", "/").split("/"):
        if part in ("", ".", ".."):
            continue
        parts.append(re.sub(r"[^A-Za-z0-9._-]", "_", part)[:60])
    return "/".join(parts)[-limit:] or "unnamed"


def allowed_path(path):
    """Defense in depth beyond the scan: a path that may hold a secret is never read."""
    parts = Path(path).parts
    if any(p in FORBIDDEN_PARTS for p in parts):
        return False
    return not FORBIDDEN_NAME.search(parts[-1] if parts else "")


# --- scrubbing -------------------------------------------------------------------------------

_UUID = re.compile(
    rb"\b[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}\b"
)
SYNTHETIC_UUID = b"fa4efa4e-fa4e-4fa4-8fa4-fa4efa4e"
_SERIAL_LINE = re.compile(
    rb"(?im)^([ \t]*(?:[A-Za-z]+ )*(?:Serial(?: Number| No\.?)?|SerialNumber|ID_SERIAL(?:_SHORT)?)"
    rb"[ \t]*:[ \t]*)([^\r\n]*[^\s])"
)
_SERIAL_INLINE = re.compile(rb"(?i)\b((?:serial(?:_?number)?|ID_SERIAL(?:_SHORT)?)=)([^\s,;\"']+)")
_SERIAL_EMPTY = {b"", b"n/a", b"none", b"0", b"unknown", b"not specified", b"<serial-scrubbed>"}
_MAC = re.compile(rb"(?<![0-9A-Fa-f:-])(?:[0-9A-Fa-f]{2}[:-]){5}[0-9A-Fa-f]{2}(?![0-9A-Fa-f:-])")
_IPV4 = re.compile(
    rb"(?<![\w.-])((?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)(?:\.(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)){3})"
    rb"(?![\w-]|\.\d)"
)
_IPV6 = re.compile(rb"(?<![0-9A-Za-z:.])(?:[0-9A-Fa-f]{1,4}:|:)[0-9A-Fa-f:]{1,44}(?![0-9A-Za-z:])")


class Scrubber:
    """Counts what it replaced; one instance per bundle so equal values stay equal."""

    def __init__(self):
        self.uuids, self.macs, self.v4, self.v6 = {}, {}, {}, {}
        self.counts = {"uuid": 0, "serial": 0, "mac": 0, "ipv4": 0, "ipv6": 0}

    def _uuid(self, match):
        value = match.group(0).lower()
        if value.startswith(SYNTHETIC_UUID) or set(value.replace(b"-", b"")) <= {ord("0")}:
            return match.group(0)
        n = self.uuids.setdefault(value, len(self.uuids) + 1)
        self.counts["uuid"] += 1
        return SYNTHETIC_UUID + b"%04x" % (n & 0xFFFF)

    def _serial(self, match):
        if match.group(2).strip().lower() in _SERIAL_EMPTY:
            return match.group(0)
        self.counts["serial"] += 1
        return match.group(1) + b"<serial-scrubbed>"

    def _mac(self, match):
        value = match.group(0).lower().replace(b"-", b":")
        if value.startswith(b"52:54:00:") or value in (b"00:00:00:00:00:00", b"ff:ff:ff:ff:ff:ff"):
            return match.group(0)
        n = self.macs.setdefault(value, len(self.macs) + 1)
        self.counts["mac"] += 1
        return b"02:fa:4e:00:%02x:%02x" % ((n >> 8) & 0xFF, n & 0xFF)

    def _v4(self, match):
        try:
            if not ipaddress.IPv4Address(match.group(1).decode()).is_global:
                return match.group(0)
        except ValueError:
            return match.group(0)
        n = self.v4.setdefault(match.group(1), len(self.v4) + 1)
        self.counts["ipv4"] += 1
        return b"<ip4-%d>" % n

    def _v6(self, match):
        text = match.group(0)
        if b"::" not in text and text.count(b":") != 7:
            return text
        try:
            if not ipaddress.IPv6Address(text.decode()).is_global:
                return text
        except ValueError:
            return text
        n = self.v6.setdefault(text.lower(), len(self.v6) + 1)
        self.counts["ipv6"] += 1
        return b"<ip6-%d>" % n

    def text(self, data):
        data = _UUID.sub(self._uuid, data)
        data = _SERIAL_LINE.sub(self._serial, data)
        data = _SERIAL_INLINE.sub(self._serial, data)
        data = _MAC.sub(self._mac, data)
        data = _IPV4.sub(self._v4, data)
        return _IPV6.sub(self._v6, data)


# --- scanning --------------------------------------------------------------------------------

_JWT_RUN = re.compile(rb"eyJ[A-Za-z0-9_.-]*")


def _jwt(data):
    """A JWT-shaped string: eyJ + 6 characters, a dot, 6 characters, a dot, 20 characters.

    Linear: one regex match per run of token characters (a match consumes its whole run, so
    `eyJ-eyJ-eyJ-...` is one match), then a split on dots; never a backtracking quantifier.
    """
    for run in _JWT_RUN.finditer(data):
        parts = run.group().split(b".")
        for i in range(len(parts) - 2):
            start = parts[i].find(b"eyJ")
            if (
                start >= 0
                and len(parts[i]) - start >= 9
                and len(parts[i + 1]) >= 6
                and len(parts[i + 2]) >= 20
            ):
                return True
    return False


# (label, compiled regex or a function of the bytes). Every regex is anchored on a literal and has
# a bounded quantifier, so the time is linear in the input (test_scanning_is_linear_on_hostile_input).
PATTERNS = (
    ("jwt", _jwt),
    ("bearer", re.compile(rb"(?i)\bBearer[ \t]+[A-Za-z0-9._~+/=-]{20,256}")),
    (
        "presigned_url",
        re.compile(
            rb"(?i)X-Amz-(?:Signature|Credential|Security-Token)=|[?&]Signature=[0-9a-f]{32,256}|AWSAccessKeyId="
        ),
    ),
    ("private_key", re.compile(rb"-----BEGIN [A-Z0-9 ]{0,30}PRIVATE KEY-----")),
    (
        "access_header",
        re.compile(rb"(?i)CF-Access-Client-(?:Id|Secret)[ \t]*[:=][ \t]*[A-Za-z0-9._-]{8,256}"),
    ),
    (
        "vast_key",
        re.compile(
            rb"(?i)(?:instance_api_key|vast[_-]?api[_-]?key)[\"']?[ \t]*[:=][ \t]*[\"']?[A-Za-z0-9]{24,256}"
        ),
    ),
    (
        "github_token",
        re.compile(rb"\b(?:gh[pousr]_[A-Za-z0-9]{20,255}|github_pat_[A-Za-z0-9_]{20,255})"),
    ),
    ("url_credentials", re.compile(rb"(?i)\bhttps?://[^\s/:@]{1,64}:[^\s/@]{1,128}@")),
)
EXACT_MIN = 6


# Labels whose value is long and secret by itself: a 16-character prefix or suffix of it is a hit
# too (a cut through a secret leaves such a fragment). Not the broker URL or the Access id, whose
# first characters are ordinary text.
FRAGMENT_LABELS = {
    "run_token",
    "access_secret",
    "runner_nonce",
    "guest_password",
    "ssh_key",
    "vast_key",
    "broker_path",
}


class Scanner:
    """`exact` is (label, value) pairs: the secret VALUES that must never appear in a bundle."""

    def __init__(self, exact):
        self.exact = []
        for label, value in exact:
            value = value.encode() if isinstance(value, str) else value
            if len(value) >= EXACT_MIN:
                self.exact.append((label, value))
            if label in FRAGMENT_LABELS and len(value) >= 2 * FRAGMENT:
                self.exact.append((label + "_fragment", value[:FRAGMENT]))
                self.exact.append((label + "_fragment", value[-FRAGMENT:]))

    def scan_exact(self, member, data):
        for label, value in self.exact:
            if value in data:
                raise ScanHit(label, member)

    def scan(self, member, data):
        self.scan_exact(member, data)
        for label, pattern in PATTERNS:
            hit = pattern(data) if callable(pattern) else pattern.search(data) is not None
            if hit:
                raise ScanHit(label, member)


def secret_values(env, winvm_root, home):
    """Everything this box knows that must not leave it: names the label, never prints a value."""
    found = []
    for name, label in (
        ("KF_RUN_TOKEN", "run_token"),
        ("KF_BROKER_URL", "broker_url"),
        ("KF_ACCESS_ID", "access_id"),
        ("KF_ACCESS_SECRET", "access_secret"),
        ("KF_RUNNER_NONCE", "runner_nonce"),
        ("VAST_API_KEY", "vast_key"),
        ("CONTAINER_API_KEY", "vast_key"),
    ):
        if env.get(name):
            found.append((label, env[name]))
    url = env.get("KF_BROKER_URL", "")
    path = url.split("/", 3)[3] if url.count("/") >= 3 else ""
    for part in {path, *path.split("/")} if path else ():
        if len(part) >= 16:
            found.append(("broker_path", part))

    def private_lines(blob):
        return [x.strip() for x in blob.splitlines() if len(x.strip()) >= 40 and b"-----" not in x]

    files = []
    root = Path(winvm_root)
    if root.is_dir():
        files += [p for p in root.glob("*/secrets/*") if p.is_file() and p.suffix != ".pub"]
    ssh = Path(home) / ".ssh"
    if ssh.is_dir():
        files += [p for p in ssh.glob("id_*") if p.is_file() and p.suffix != ".pub"]
    files += [Path(home) / ".vast_api_key", Path(home) / ".config/vastai/vast_api_key"]
    for path in files:
        try:
            if not path.is_file() or path.stat().st_size > 16384:
                continue
            blob = path.read_bytes()
        except OSError:
            continue
        if b"PRIVATE KEY" in blob:
            found.extend(("ssh_key", line) for line in private_lines(blob))
        elif b"\0" not in blob:
            lines = [x.strip() for x in blob.splitlines() if x.strip()]
            label = "vast_key" if "vast" in path.name else "guest_password"
            if len(lines) == 1 and len(lines[0]) <= 256:
                found.append((label, lines[0]))
    return found


# --- collection ------------------------------------------------------------------------------


class Item:
    def __init__(self, name, data, tier, section, order=0.0, shrink=False, lead=b""):
        self.name, self.data, self.tier, self.section = name, data, tier, section
        self.order, self.shrink = order, shrink
        self.lead = lead  # up to LEAD bytes that came just before `data` (scanned, never packed)
        self.cut = 0  # bytes removed from the start
        self.estimate = None

    def key(self):
        return (self.tier, self.order, self.name)


def read_tail(path, cap):
    """(the last `cap` bytes of a plain file, the bytes before them, up to LEAD of those)."""
    with open_plain(path) as handle:
        size = os.fstat(handle.fileno()).st_size
        start = max(0, size - cap)
        first = max(0, start - LEAD)
        handle.seek(first)
        lead = handle.read(start - first)
        return handle.read(cap), start, lead


def is_text(data):
    return b"\0" not in data[:8192]


class Roots:
    def __init__(self, workspace="/workspace", prov="/root/prov", winvm=None, home=None):
        self.workspace = Path(workspace)
        self.prov = Path(prov)
        self.winvm = Path(winvm or os.path.join(workspace, "winvm"))
        self.home = Path(home or os.path.expanduser("~"))


def run_command(argv, limit=2 * 1024 * 1024, timeout=COMMAND_TIMEOUT):
    """Run a host command; at the timeout kill its whole process group and wait at most 2 s more.

    Output goes to a temporary file, not a pipe, so a grandchild that holds the pipe open cannot
    block the reader; a process stuck in the kernel (a hung GPU) is abandoned after the kill.
    """
    note = b""
    try:
        with tempfile.TemporaryFile() as out:
            proc = subprocess.Popen(
                argv,
                stdin=subprocess.DEVNULL,
                stdout=out,
                stderr=subprocess.STDOUT,
                start_new_session=True,
            )
            try:
                proc.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                note = b"[kf-evidence: command timed out, killed]\n"
                try:
                    os.killpg(proc.pid, signal.SIGKILL)
                except OSError:
                    pass
                try:
                    proc.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    note = b"[kf-evidence: command timed out, not reaped]\n"
            size = out.seek(0, os.SEEK_END)
            out.seek(max(0, size - limit))
            return out.read(limit) + note
    except (OSError, subprocess.SubprocessError) as error:
        return ("[kf-evidence: command failed: %s]\n" % type(error).__name__).encode()


def host_items(runner, expired, skipped):
    """The host state. Run LAST (after every file is collected), within COMMAND_PHASE seconds."""
    items = []
    until = time.monotonic() + COMMAND_PHASE

    def output(argv):
        if expired() or time.monotonic() > until:
            skipped.append("host/" + argv[0] + " (time)")
            return b"[kf-evidence: skipped, out of time]\n"
        return runner(argv)

    def add(name, argv, tier, cap=64 * 1024, head=None):
        data = output(argv)
        if head is not None:
            data = b"".join(data.splitlines(keepends=True)[:head])
        item = Item("host/" + name, data[-cap:], tier, "host")
        item.cut = max(0, len(data) - cap)
        items.append(item)

    add("free.txt", ["free", "-m"], 0)
    add("df.txt", ["df", "-h"], 0)
    add("uname.txt", ["uname", "-a"], 0)
    add("date.txt", ["date", "-u", "+%Y-%m-%dT%H:%M:%SZ"], 0)
    add("uptime.txt", ["uptime"], 0)
    ps = output(["ps", "-eo", "pid,ppid,etime,stat,args", "--width", "2000"])
    lines = ps.splitlines(keepends=True)
    keep = [x for x in lines[:1] if x] + [
        x[:2000] + (b"\n" if len(x) > 2000 else b"")
        for x in lines[1:]
        if re.search(rb"qemu|swtpm", x)
    ]
    items.append(Item("host/ps_qemu_swtpm.txt", b"".join(keep)[-64 * 1024 :], 0, "host"))
    add("nvidia-smi_q_head.txt", ["nvidia-smi", "-q"], 0, head=60)
    add("dmesg_tail.txt", ["dmesg", "-T"], 2, cap=DMESG_CAP)
    return items


def walk_files(root, depth=3):
    """Plain files under root; no symlink is followed, never into a forbidden directory."""
    root = Path(root)
    if not is_plain_dir(root):
        return
    stack = [(root, 0)]
    while stack:
        directory, level = stack.pop()
        try:
            entries = sorted(directory.iterdir())
        except OSError:
            continue
        for entry in entries:
            if not allowed_path(entry.relative_to(root)):
                continue
            if is_plain_dir(entry):
                if level < depth:
                    stack.append((entry, level + 1))
            elif is_plain_file(entry):
                yield entry


def gz_tail(path, cap, read_cap=64 * 1024 * 1024):
    """(last `cap` bytes of a gzip file's content, the bytes before them, up to LEAD of those)."""
    with open_plain(path) as raw:
        with gzip.GzipFile(fileobj=raw) as handle:
            data = handle.read(read_cap)
    start = max(0, len(data) - cap)
    return data[start:], start, data[max(0, start - LEAD) : start]


def tgz_members(path, budget):
    """Yield (name, bytes, cut, lead) for regular members of a tar.gz; bounded, never extracts."""
    unpacked = 0
    with open_plain(path) as raw:
        with tarfile.open(fileobj=gzip.GzipFile(fileobj=raw), mode="r|") as archive:
            for member in archive:
                if raw.tell() > ARCHIVE_INPUT_CAP:
                    break
                if not member.isreg() or unpacked >= ARCHIVE_UNPACK_CAP or budget():
                    continue
                name = posixpath.normpath(member.name.replace("\\", "/"))
                if name.startswith(("/", "..")) or not allowed_path(name):
                    continue
                handle = archive.extractfile(member)
                data = handle.read(min(member.size, MEMBER_READ_CAP)) if handle else b""
                unpacked += len(data)
                start = max(0, len(data) - FILE_CAP * 2)
                yield name, data[start:], start, data[max(0, start - LEAD) : start]


def collect(roots, log, runner, expired, now):
    """All candidate items, unscrubbed: the files first, the host commands last."""
    items, skipped = [], []

    def mtime(path):
        st = lstat(path)
        return st.st_mtime if st else 0.0

    def add_file(prefix, path, tier, section, cap=FILE_CAP, shrink=False, base=None):
        try:
            if path.suffix == ".gz":
                data, cut, lead = gz_tail(path, cap)
                name = path.name[:-3]
            else:
                data, cut, lead = read_tail(path, cap)
                name = path.name if base is None else str(path.relative_to(base))
            age = now - mtime(path)
        except (OSError, EOFError, zlib.error):
            skipped.append(safe_name(prefix + "/" + path.name))
            return
        if not is_text(data) and path.suffix != ".png":
            skipped.append(safe_name(prefix + "/" + name))
            return
        item = Item(
            prefix + "/" + safe_name(name), data, tier, section, order=age, shrink=shrink, lead=lead
        )
        item.cut = cut
        items.append(item)

    # the payload log and the result
    try:
        data, cut, lead = read_tail(log, LOG_CAP)
        item = Item("log/output.log", data, 1, "log", shrink=True, lead=lead)
        item.cut = cut
        items.append(item)
    except OSError:
        pass
    result = roots.workspace / "kf-result.json"
    if is_plain_file(result):
        add_file("log", result, 0, "log", cap=64 * 1024)
    # the payload's own evidence directory
    for path in list(walk_files(roots.workspace / "kf-ci-evidence"))[:MAX_FILES]:
        if not expired():
            add_file("evidence", path, 2, "evidence", base=roots.workspace / "kf-ci-evidence")
    # the gate / provision evidence: /root/prov
    if is_plain_dir(roots.prov) and not expired():
        top = sorted(p for p in roots.prov.iterdir() if is_plain_file(p))
        packed = {p.name[: -len(".tgz")] for p in top if p.name.endswith(".tgz")}
        for path in top:
            if path.name.endswith((".tgz", ".tar.gz")):
                try:
                    for name, data, cut, lead in tgz_members(path, expired):
                        if is_text(data) or name.endswith(".png"):
                            item = Item(
                                "prov/"
                                + safe_name(path.name.rsplit(".", 1)[0])
                                + "/"
                                + safe_name(name),
                                data,
                                3,
                                "prov",
                                order=now - mtime(path),
                                lead=lead,
                            )
                            item.cut = cut
                            items.append(item)
                except (OSError, tarfile.TarError, EOFError, zlib.error):
                    skipped.append(safe_name("prov/" + path.name))
            elif path.suffix in (".log", ".txt", ".out", ".err", ".run", ".gz") and allowed_path(
                path.name
            ):
                verdict_log = path.name.endswith("_windows.log") or path.name in ("prov.log",)
                add_file("prov", path, 1 if verdict_log else 2, "prov", shrink=verdict_log)
        for path in sorted(p for p in roots.prov.iterdir() if is_plain_dir(p)):
            if path.name in packed or not allowed_path(path.name) or expired():
                continue
            for entry in list(walk_files(path))[:MAX_FILES]:
                add_file("prov/" + safe_name(path.name), entry, 3, "prov", base=path)
    # bench stage logs
    bench = roots.workspace / "bench"
    if is_plain_dir(bench) and not expired():
        logs = [
            p
            for p in bench.iterdir()
            if is_plain_file(p) and (p.suffix == ".out" or p.name.startswith("xid_"))
        ]
        for path in sorted(logs, key=mtime, reverse=True)[:30]:
            add_file("bench", path, 4, "bench", cap=256 * 1024)
    # the Windows VM: logs and screenshots only (never secrets/, tpm/, firmware/, disk/)
    vms = sorted(roots.winvm.iterdir()) if is_plain_dir(roots.winvm) else []
    for vm in vms:
        logs = vm / "logs"
        if expired() or not is_plain_dir(vm) or not is_plain_dir(logs):
            continue
        names = [p for p in logs.iterdir() if is_plain_file(p) and allowed_path(p.relative_to(vm))]
        text = [p for p in names if p.suffix in TEXT_SUFFIX]
        for path in sorted(text, key=mtime, reverse=True)[:40]:
            add_file("vm/" + safe_name(vm.name), path, 3, "vm", cap=VM_LOG_CAP)
        zipped = [p for p in names if p.suffix == ".gz" and lstat(p).st_size <= 8 * 1024 * 1024]
        for path in sorted(zipped, key=mtime, reverse=True)[:10]:
            add_file("vm/" + safe_name(vm.name), path, 5, "vm", cap=1024 * 1024)
        shots = logs / "shots"
        pngs = []
        if is_plain_dir(shots):
            pngs = sorted(
                (
                    p
                    for p in shots.glob("*.png")
                    if is_plain_file(p) and lstat(p).st_size <= SHOT_MAX
                ),
                key=mtime,
                reverse=True,
            )
        timeouts = [p for p in pngs if p.name.startswith("timeout_")][:1]
        chosen = timeouts + [p for p in pngs if p not in timeouts]
        for index, path in enumerate(chosen[:SHOT_COUNT]):
            tier = 2 if path in timeouts else (3 if index < 3 else 5)
            try:
                with open_plain(path) as handle:
                    data = handle.read(SHOT_MAX + 1)
            except OSError:
                continue
            items.append(
                Item(
                    "shots/" + safe_name(vm.name) + "/" + safe_name(path.name),
                    data,
                    tier,
                    "shots",
                    order=now - mtime(path),
                )
            )
    # the host commands, last: a hung GPU or a stale mount costs only these
    items += host_items(runner, expired, skipped)
    return items, skipped


# --- packing ---------------------------------------------------------------------------------


def estimate(item):
    if item.estimate is None:
        data = item.data
        item.estimate = len(zlib.compress(data, 6)) if is_text(data) else len(data)
    return item.estimate


def fits(items, cap):
    raw = sum(len(i.data) for i in items)
    return raw <= RAW_CAP and sum(estimate(i) for i in items) + 8192 <= cap * FIT


def select(items, cap=BUNDLE_CAP, shrink=True):
    """Keep the highest tiers; drop the lowest first, then halve the shrinkable tier-1 logs.

    With shrink=False it only drops (tiers 2 to 5) and stops there, whatever is still too big:
    build() scans the tier-0/1 data at full length before any of it is cut.
    """
    kept = sorted(items, key=Item.key)
    dropped = []
    while not fits(kept, cap):
        low = max(kept, key=Item.key)
        if low.tier >= 2:
            kept.remove(low)
            dropped.append(low)
            continue
        if not shrink:
            break
        shrinkable = [i for i in kept if i.shrink and len(i.data) > 4096]
        if not shrinkable:
            return None, dropped
        big = max(shrinkable, key=lambda i: len(i.data))
        half = len(big.data) // 2
        big.cut += len(big.data) - half
        big.data, big.estimate = big.data[-half:], None
    return kept, dropped


def pack(items, dropped, scrub_counts, meta, path, now, cap=BUNDLE_CAP, scanner=None):
    """Write the .tgz; returns (size, sha256) or None when it still exceeds the cap."""
    sections = {}
    for item in items:
        row = sections.setdefault(
            item.section, {"name": item.section, "files": 0, "raw_bytes": 0, "dropped": 0}
        )
        row["files"] += 1
        row["raw_bytes"] += len(item.data)
    for item in dropped:
        sections.setdefault(
            item.section, {"name": item.section, "files": 0, "raw_bytes": 0, "dropped": 0}
        )["dropped"] += 1
    rows = sorted(sections.values(), key=lambda r: r["name"])
    manifest = {
        "format": 1,
        "run": meta.get("run"),
        "built_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(now)),
        "caps": {"bundle_bytes": cap, "raw_bytes": RAW_CAP},
        "sections": rows,
        "scrub": dict(scrub_counts),
        "scan": "clean",
        "dropped": [i.name for i in dropped][:200],
        "cut": (
            {i.name: i.cut for i in items if i.cut}
            if len([i for i in items if i.cut]) <= 200
            else "many"
        ),
        "skipped": meta.get("skipped", [])[:100],
        "tiers": "0 never dropped; 1 log tails shrink; 2-5 dropped from 5 down (see vast_evidence.py)",
    }
    body = json.dumps(manifest, indent=1, sort_keys=True).encode()
    if scanner is not None:
        scanner.scan("MANIFEST.json", body)  # dropped and skipped names are in it
    with tarfile.open(path, "w:gz", compresslevel=9) as archive:
        for name, data in [("MANIFEST.json", body)] + [(i.name, i.data) for i in items]:
            info = tarfile.TarInfo(name)
            info.size, info.mtime, info.mode = len(data), int(now), 0o644
            archive.addfile(info, io.BytesIO(data))
    size = os.path.getsize(path)
    if size > cap:
        return None
    with open(path, "rb") as handle:
        digest = hashlib.sha256(handle.read()).hexdigest()
    short = [{k: r[k] for k in ("name", "files", "raw_bytes", "dropped")} for r in rows]
    header = {
        "v": 1,
        "sections": short,
        "dropped": len(dropped),
        "raw_bytes": sum(len(i.data) for i in items),
        "scrub": dict(scrub_counts),
        "scan": "clean",
    }
    return size, digest, header


class Refused(Exception):
    pass


def build(roots, log, env, out, runner, expired, now, cap=BUNDLE_CAP):
    """Collect, scan (RAW), scrub, select, pack. Returns a dict, or raises Refused/ScanHit."""
    scanner = Scanner(secret_values(env, roots.winvm, roots.home))
    scrubber = Scrubber()
    candidates, skipped = collect(roots, log, runner, expired, now)
    # 1. Drop what the cap cannot hold (on the raw sizes, so that dropped items are never scanned
    #    or scrubbed), but cut nothing yet.
    chosen, dropped = select(candidates, cap, shrink=False)
    # 2. Scan every kept item at FULL length, with the bytes before its cut (a secret that crosses
    #    a cut must be seen whole), then scrub it.
    for item in chosen:
        if expired():
            raise Refused("time")
        try:
            scanner.scan(item.name, item.name.encode())
        except ScanHit as hit:  # never print a name that holds a secret: a hash and nothing else
            digest = hashlib.sha256(item.name.encode()).hexdigest()[:10]
            raise ScanHit(hit.label, "name-" + digest) from None
        scanner.scan(item.name, item.lead + item.data)
        item.lead = b""
        if is_text(item.data) and not item.name.endswith(".png"):
            item.data = scrubber.text(item.data)
        item.estimate = None
    # 3. Now the size cut (logs are halved, newest bytes kept), and a last exact check of the
    #    final bytes (a secret or a fragment of one must not survive any cut).
    chosen, more = select(chosen, cap)
    if chosen is None:
        raise Refused("too_large")
    dropped += more
    for item in chosen:
        scanner.scan_exact(item.name, item.data)
    out.mkdir(parents=True, exist_ok=True)
    path = out / "evidence.tgz"
    meta = {"run": env.get("KF_RUN_ID"), "skipped": skipped}
    for _ in range(12):
        packed = pack(chosen, dropped, scrubber.counts, meta, path, now, cap, scanner)
        if packed is not None:
            size, digest, header = packed
            break
        low = max(chosen, key=Item.key)
        if low.tier >= 2:
            chosen.remove(low)
            dropped.append(low)
        else:
            raise Refused("too_large")
    else:
        raise Refused("too_large")
    encoded = json.dumps(header, separators=(",", ":"))
    while len(encoded) > META_MAX and header["sections"]:
        header["sections"].pop()
        encoded = json.dumps(header, separators=(",", ":"))
    return {
        "path": path,
        "size": size,
        "sha256": digest,
        "meta": encoded,
        "files": len(chosen),
        "dropped": len(dropped),
        "scrub": dict(scrubber.counts),
    }


# --- upload ----------------------------------------------------------------------------------


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


class Transport:
    """HTTPS POST of the bundle to one fixed URL; no proxy, no redirect; token only in a header."""

    def __init__(self, token, access=None, timeout=60.0):
        self.token, self.timeout = token, timeout
        ident, secret = (access or {}).get("KF_ACCESS_ID"), (access or {}).get("KF_ACCESS_SECRET")
        self.access = (
            {"CF-Access-Client-Id": ident, "CF-Access-Client-Secret": secret}
            if ident and secret
            else {}
        )
        self.opener = urllib.request.build_opener(
            urllib.request.ProxyHandler({}),
            _NoRedirect(),
            urllib.request.HTTPSHandler(context=ssl.create_default_context()),
        )

    def post(self, url, body, extra, timeout=None):
        headers = {
            "Content-Type": "application/gzip",
            "Authorization": "Bearer " + self.token,
            "User-Agent": USER_AGENT,
            **extra,
            **self.access,
        }
        request = urllib.request.Request(url, data=body, headers=headers, method="POST")
        try:
            with self.opener.open(request, timeout=timeout or self.timeout) as response:
                status, raw = response.status, response.read(65536)
        except urllib.error.HTTPError as error:
            status, raw = error.code, error.read(65536)
            error.close()
        except (urllib.error.URLError, OSError, ValueError):
            return None, None
        try:
            return status, json.loads(raw)
        except ValueError:
            return status, None


def upload(env, built, transport, deadline, sleep=time.sleep, clock=time.monotonic):
    """POST the bundle. Returns (ok, reason).

    Only the Worker's own `EVIDENCE_EXISTS` counts as stored (an earlier try of ours arrived). Any
    other 409 (the ledger queue's BUSY) and 429, 5xx and network errors are retried, at most 3 tries.
    """
    url = "%s/evidence/%s" % (env["KF_BROKER_URL"], env["KF_RUN_ID"])
    with open(built["path"], "rb") as handle:
        body = handle.read()
    extra = {"X-KF-Evidence-SHA256": built["sha256"], "X-KF-Evidence-Meta": built["meta"]}
    reason = "upload_failed"
    for attempt in range(3):
        left = deadline - clock()
        if left < 3:
            return False, "time"
        status, doc = transport.post(url, body, extra, timeout=min(60.0, left))
        state = doc.get("state") if isinstance(doc, dict) else None
        if status == 200:
            return True, "stored"
        if status == 409 and state == "EVIDENCE_EXISTS":
            return True, "already_stored"
        reason = "upload_http_%s" % (status if status is not None else "none")
        if status not in (None, 409, 429, 500, 502, 503, 504):
            return False, reason
        sleep(min(2.0 * (attempt + 1), max(0.0, deadline - clock() - 3)))
    return False, reason


def say(text):
    print(text, flush=True)


def run(args, env, runner=run_command, transport=None, clock=time.monotonic, now=time.time):
    """The whole step. Never raises, never changes a verdict; prints the outcome lines."""
    started = clock()
    budget = args.budget
    if args.until:
        budget = min(budget, args.until - now())
    if budget < 10:
        say("KF_EVIDENCE_SKIPPED reason=time")
        return 0
    deadline = started + budget

    def expired():
        return clock() > deadline - 8

    try:
        problems = contract.validate_env(env)
        if problems and not args.no_upload:
            say("KF_EVIDENCE_SKIPPED reason=environment_contract")
            return 0
        roots = Roots(args.workspace, args.prov, args.winvm)
        built = build(roots, Path(args.log), env, Path(args.out), runner, expired, now())
    except ScanHit as hit:
        say(
            "KF_EVIDENCE_REFUSED reason=scan_hit class=%s member=%s"
            % (hit.label, hit.member if hit.member.startswith("name-") else safe_name(hit.member))
        )
        return 0
    except Refused as error:
        say("KF_EVIDENCE_REFUSED reason=%s" % error)
        return 0
    except Exception as error:  # noqa: BLE001  (a helper bug must not touch the run)
        say("KF_EVIDENCE_SKIPPED reason=build_error:%s" % type(error).__name__)
        return 0
    say(
        "KF_EVIDENCE_BUNDLE files=%d dropped=%d packed=%d scrub=%s"
        % (
            built["files"],
            built["dropped"],
            built["size"],
            ",".join("%s:%d" % kv for kv in sorted(built["scrub"].items())),
        )
    )
    if args.no_upload:
        return 0
    try:
        transport = transport or Transport(env["KF_RUN_TOKEN"], access=env)
        ok, reason = upload(env, built, transport, deadline, clock=clock)
    except Exception as error:  # noqa: BLE001
        ok, reason = False, "upload_error:%s" % type(error).__name__
    if ok:
        run_id, _, attempt = env["KF_RUN_ID"].partition("-")
        say(
            "evidence uploaded: %s/%s %d sha256 %s%s"
            % (
                run_id,
                attempt,
                built["size"],
                built["sha256"][:12],
                "" if reason == "stored" else " (%s)" % reason,
            )
        )
    else:
        say("KF_EVIDENCE_SKIPPED reason=%s" % reason)
    return 0


def parser():
    top = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = top.add_subparsers(dest="command", required=True)
    p = sub.add_parser("run")
    p.add_argument("--log", default="/workspace/kf-ci-output.log")
    p.add_argument("--out", default="/workspace/kf-evidence")
    p.add_argument("--budget", type=float, default=100.0)
    p.add_argument("--until", type=float, default=0.0, help="absolute epoch seconds to be done by")
    p.add_argument("--no-upload", action="store_true")
    p.add_argument("--workspace", default="/workspace")
    p.add_argument("--prov", default=os.environ.get("KF_PROV_DIR", "/root/prov"))
    p.add_argument("--winvm", default=None)
    return top


def main(argv=None, env=None):
    args = parser().parse_args(argv)
    try:
        return run(args, dict(os.environ if env is None else env))
    except Exception as error:  # noqa: BLE001
        say("KF_EVIDENCE_SKIPPED reason=error:%s" % type(error).__name__)
        return 0


if __name__ == "__main__":
    sys.exit(main())
