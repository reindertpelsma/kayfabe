#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""kfroot: the explicit root step of the CI harness, and the rootless assertion (stdlib only).

STATUS: LIVE as code, 2026-11-15; NOT RUN ON A BOX (GPU-free tests only; the first-box checklist is in
docs/design/V3_KFROOT_ROOTLESS.md). OWNER_RULINGS section AR item 6: kayfabe is proved to work as an unprivileged user with
no capabilities. The harness may still need root for a few box-setup operations; each one is a greppable `kfroot <cmd>` line.
This is accident prevention in an untrusted sandbox, not security inside the box.

The mechanism (owner design, 2026-11-11):
  /usr/local/bin/kfroot            #!/bin/bash; exec sudo -n -E /usr/local/sbin/kfroot-wrapper "$@"
  /usr/local/sbin/kfroot-wrapper   #!/bin/bash; exec "$@"   (root:root 0755)
  /etc/sudoers.d/kfroot            Defaults:ci !secure_path; ci ALL=(root) NOPASSWD:SETENV: /usr/local/sbin/kfroot-wrapper
Only the wrapper is whitelisted, so a third-party script that calls plain `sudo X` is refused. Two additions to the owner's
three lines, both for the audit and for stdio: `logfile=` (a file this tool reads by byte offset: the sudo entries of one run
window, independent of rsyslog) and `!use_pty` (no pseudo-terminal between the harness and the command).

Subcommands (all stdlib; `install` and `bootstrap` need root):
  install [--user ci] [--root /]        write the three files, sudoers checked with `visudo -c` BEFORE it is installed
  verify  [--user ci] [--root /]        the installed files are the expected ones (content, mode, owner)
  bootstrap [--user ci] [--dry-run]     the root-run box setup: user in group kvm, install, device nodes, lock, log, workspace
  drop-argv --user ci                   print the argv prefix that runs a command as that user with NO capabilities
  run --user ci -- CMD...               exec CMD as that user, every capability set empty, NoNewPrivs=1 (root, or via kfroot)
  audit-mark [--log F]                  print the byte offset of the sudo log (the start of a run window)
  audit-check --since N [--log F]       exit 1 if the window holds any sudo entry other than the kfroot wrapper
  assert-tree --pid N [--pid-file F]    read /proc: uid != 0, CapPrm/CapEff/CapBnd (and Inh/Amb) = 0, NoNewPrivs = 1
  steps                                 print the audited kfroot steps (STEPS) and the stage policy (STAGE_POLICY) as JSON
  lint-stage STAGE [--root DIR]         the static check the payload runs before it starts a stage dropped (section "stage policy")

The audit of every root step is docs/design/V3_KFROOT_AUDIT.md (2026-11-20); STEPS and STAGE_POLICY below are that audit as data and
scripts/ci/test_kfroot_payload.py keeps the two in step. the payload's `--rootless require` uses them (docs/CI_V3.md; no script file name is written whole here: the stock-patch audit follows names).
"""

import argparse
import grp
import json
import os
from pathlib import Path
import pwd
import re
import shlex
import shutil
import stat
import subprocess
import sys
import tempfile

SCHEMA = 1
LAUNCHER = "/usr/local/bin/kfroot"
WRAPPER = "/usr/local/sbin/kfroot-wrapper"
SUDOERS = "/etc/sudoers.d/kfroot"
SUDO_LOG = "/var/log/kfroot-sudo.log"
AUTH_LOG = "/var/log/auth.log"
LIMITS = "/etc/security/limits.d/kfroot.conf"
GPU_LOCK = "/tmp/kayfabe-fastguest.lock"
DEFAULT_USER = "ci"
USER_RE = re.compile(r"^[a-z_][a-z0-9_-]{0,31}$")

LAUNCHER_TEXT = f'#!/bin/bash\nexec sudo -n -E {WRAPPER} "$@"\n'
WRAPPER_TEXT = '#!/bin/bash\nexec "$@"\n'


class KfrootError(Exception):
    """A refusal; the message is one short token or sentence."""


def check_user(user):
    if not isinstance(user, str) or not USER_RE.fullmatch(user) or user == "root":
        raise KfrootError(f"bad_user {user!r}")
    return user


def sudoers_text(user, logfile=True):
    """The sudoers file. `logfile=` is a sudo(ws) setting; sudo-rs (Ubuntu 25.10 and later) refuses it, and then the audit
    is the syslog file (AUTH_LOG) that both implementations write."""
    check_user(user)
    return (
        "# kayfabe scripts/ci/kfroot.py: the ONE sudoers entry of the CI user (accident prevention, not security).\n"
        f"Defaults:{user} !secure_path\n"
        f"Defaults:{user} !use_pty\n"
        + (f"Defaults:{user} logfile={SUDO_LOG}\nDefaults:{user} loglinelen=0\n" if logfile else "")
        + f"{user} ALL=(root) NOPASSWD:SETENV: {WRAPPER}\n"
    )


FILES = (  # (path, mode)
    (LAUNCHER, 0o755),
    (WRAPPER, 0o755),
    (SUDOERS, 0o440),
)


def file_texts(user, logfile=True):
    return {LAUNCHER: LAUNCHER_TEXT, WRAPPER: WRAPPER_TEXT, SUDOERS: sudoers_text(user, logfile)}


def default_log(root="/"):
    """The sudo audit file: the dedicated logfile when this install has one, else the syslog auth file."""
    return SUDO_LOG if under(root, SUDO_LOG).exists() else AUTH_LOG


def under(root, path):
    return Path(str(root).rstrip("/") + path) if str(root) != "/" else Path(path)


# ------------------------------------------------------------------------------------------------ install / verify
def visudo_check(path, visudo="visudo", run=subprocess.run):
    """`visudo -c -f PATH`: returns (ok, text). A missing visudo is a refusal, never a skipped check."""
    try:
        p = run([visudo, "-c", "-f", str(path)], capture_output=True, text=True, timeout=60)
    except FileNotFoundError:
        raise KfrootError("visudo_missing") from None
    return p.returncode == 0, (p.stdout + p.stderr).strip()


def write_file(target, text, mode, own_root):
    """Atomic write: a temporary file in the same directory, mode set before it is renamed in place."""
    target.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp = tempfile.mkstemp(prefix=".kfroot.", dir=target.parent)
    try:
        with os.fdopen(fd, "w") as f:
            f.write(text)
        os.chmod(tmp, mode)
        if own_root:
            os.chown(tmp, 0, 0)
        os.replace(tmp, target)
    except BaseException:
        if os.path.exists(tmp):
            os.unlink(tmp)
        raise


def install(user=DEFAULT_USER, root="/", visudo="visudo", run=subprocess.run):
    """Install the three files. The sudoers file is validated by visudo in a temporary file first: a file that visudo
    refuses is never put in /etc/sudoers.d (a broken sudoers file locks sudo out for everyone). A visudo that refuses
    only `logfile` (sudo-rs) gets the file without that line; any other refusal is final."""
    own_root = str(root) == "/" and os.geteuid() == 0
    if str(root) == "/" and os.geteuid() != 0:
        raise KfrootError("install_needs_root")
    logfile = True
    for attempt in (True, False):
        texts = file_texts(user, attempt)
        with tempfile.TemporaryDirectory(prefix="kfroot-visudo-") as d:
            probe = Path(d) / "kfroot"
            probe.write_text(texts[SUDOERS])
            os.chmod(probe, 0o440)
            ok, out = visudo_check(probe, visudo, run)
        if ok:
            logfile = attempt
            break
        if not (attempt and "logfile" in out):
            raise KfrootError("visudo_refused " + out[-200:])
    for path, mode in FILES:
        write_file(under(root, path), texts[path], mode, own_root)
    return {"installed": [p for p, _ in FILES], "user": user, "audit_log": SUDO_LOG if logfile else AUTH_LOG}


def sudo_flavor(sudo="sudo", run=subprocess.run):
    """`sudo --version`: "sudo-rs" (Ubuntu 25.10 and later: it IGNORES -E, so the environment is not forwarded: measured
    on sudo-rs 0.2.13), "sudo" (the original; Ubuntu 22.04's box image), or "missing"."""
    try:
        p = run([sudo, "--version"], capture_output=True, text=True, timeout=30)
    except (FileNotFoundError, subprocess.TimeoutExpired):
        return "missing"
    first = (p.stdout or p.stderr).strip().splitlines()[:1]
    return "sudo-rs" if first and "sudo-rs" in first[0].lower() else "sudo"


def verify(user=DEFAULT_USER, root="/", visudo="visudo", run=subprocess.run, sudo="sudo"):
    """Return a list of problems (empty = the installed state is the expected one)."""
    problems = []
    if str(root) == "/":
        flavor = sudo_flavor(sudo, run)
        if flavor != "sudo":
            problems.append(f"sudo_flavor {flavor}")
    for path, mode in FILES:
        target = under(root, path)
        try:
            st = target.stat()
        except FileNotFoundError:
            problems.append(f"missing {path}")
            continue
        wanted = [file_texts(user, lf)[path] for lf in (True, False)]
        if target.read_text() not in wanted:
            problems.append(f"content {path}")
        if (st.st_mode & 0o7777) != mode:
            problems.append(f"mode {path} {oct(st.st_mode & 0o7777)} want {oct(mode)}")
        if str(root) == "/" and st.st_uid != 0:
            problems.append(f"owner {path} uid {st.st_uid}")
    sudoers = under(root, SUDOERS)
    if sudoers.exists():
        ok, out = visudo_check(sudoers, visudo, run)
        if not ok:
            problems.append("visudo " + out[-120:])
    return problems


# ------------------------------------------------------------------------------------------------ the drop
def drop_argv(uid, gid, kvm_gid):
    """setpriv: that uid and gid, the supplementary group of /dev/kvm only, every capability set emptied, no_new_privs.
    (The install-flow stage module has its own copy of this argv; it should import this one. No literal module name here: the stock-patch audit follows names.)"""
    for n in (uid, gid, kvm_gid):
        if not isinstance(n, int) or isinstance(n, bool) or n <= 0:
            raise KfrootError("drop_identity")
    return [
        "setpriv", "--reuid", str(uid), "--regid", str(gid), "--groups", str(kvm_gid),
        "--inh-caps=-all", "--ambient-caps=-all", "--bounding-set=-all", "--no-new-privs",
    ]


def carried(environ):
    """sudo strips LD_* from the environment even with SETENV and -E (measured, sudo 1.9.17: LD_LIBRARY_PATH did not
    arrive). They are handed to the dropped command through `env` AFTER the drop, where they bind an unprivileged process."""
    return sorted(f"{k}={v}" for k, v in environ.items() if k.startswith("LD_") and "\0" not in v)


def run_prefix(uid, gid, kvm_gid, via_kfroot, memlock=True, environ=None):
    """The argv words before the command. The memlock limit is raised BEFORE the drop (a dropped process cannot raise a
    hard limit); `kfroot` is the front only when the caller is not root."""
    words = []
    if via_kfroot:
        words.append("kfroot")
    if memlock:
        words += ["prlimit", "--memlock=unlimited:unlimited"]
    words += drop_argv(uid, gid, kvm_gid) + ["--"]
    ld = carried(environ or {})
    return words + (["env", *ld] if ld else [])


def identity(user, kvm_group="kvm"):
    check_user(user)
    try:
        entry = pwd.getpwnam(user)
        kvm = grp.getgrnam(kvm_group).gr_gid
    except KeyError as e:
        raise KfrootError(f"no_such_account {e}") from None
    if entry.pw_uid == 0 or entry.pw_gid == 0 or kvm == 0:
        raise KfrootError("identity_is_root")
    return entry.pw_uid, entry.pw_gid, kvm


# ------------------------------------------------------------------------------------------------ the sudo audit
FAILURES = ("command not allowed", "incorrect password", "a password is required", "NOT in sudoers", "not allowed to")
LINE = re.compile(r"(?:\bsudo:\s+|\s:\s)(?P<user>\S+) : (?P<rest>.+)$")


def audit_mark(log=SUDO_LOG):
    try:
        return os.stat(log).st_size
    except FileNotFoundError:
        return 0


def audit_text(log, since):
    """The text of the log from byte `since`. A log that is now shorter than `since` was rotated or replaced: the whole
    file is the window. A log that is missing or unreadable is a refusal, never an empty window (a check that cannot read
    its evidence must not pass)."""
    try:
        size = os.stat(log).st_size
        with open(log, "rb") as f:
            f.seek(0 if size < since else since)
            return f.read().decode("utf-8", "replace")
    except FileNotFoundError:
        raise KfrootError(f"audit_log_missing {log}") from None
    except OSError as e:
        raise KfrootError(f"audit_log_unreadable {log} {e.strerror}") from None


def unwrap(text):
    """sudo wraps a logfile line at 80 columns unless loglinelen=0 (measured, sudo 1.9.17): the continuation lines start
    with whitespace. Join them to their entry so COMMAND= is on the line of its user."""
    out = []
    for line in text.splitlines():
        if out and line[:1] in (" ", "\t"):
            out[-1] += " " + line.strip()
        else:
            out.append(line)
    return out


def audit_entries(text, users=(DEFAULT_USER,)):
    """Parse sudo log lines into entries {user, command, failure}. A line that names a COMMAND= but cannot be read as a
    sudo entry is kept as an entry with failure `unparsed` (fail closed)."""
    entries = []
    for raw in unwrap(text):
        if "COMMAND=" not in raw:
            continue
        m = LINE.search(raw)
        if not m:
            entries.append({"user": "?", "command": raw.split("COMMAND=", 1)[1].strip(), "failure": "unparsed"})
            continue
        if users and m["user"] not in users:
            continue
        head, command = m["rest"].split("COMMAND=", 1)
        failure = next((f for f in FAILURES if f in head), "")
        entries.append({"user": m["user"], "command": command.strip(), "failure": failure})
    return entries


def is_wrapper(command):
    words = command.split()
    return bool(words) and words[0] == WRAPPER


def audit_check(text, users=(DEFAULT_USER,)):
    """Result: every entry of the window, and the violations (an entry that is not a clean call of the wrapper)."""
    entries = audit_entries(text, users)
    bad = [e for e in entries if e["failure"] or not is_wrapper(e["command"])]
    return {"entries": len(entries), "kfroot_calls": len(entries) - len(bad), "violations": bad,
            "commands": [e["command"][:200] for e in entries if not e["failure"] and is_wrapper(e["command"])]}


# ------------------------------------------------------------------------------------------------ /proc assertion
CAP_FIELDS = ("CapInh", "CapPrm", "CapEff", "CapBnd", "CapAmb")


def parse_status(text):
    fields = {}
    for line in text.splitlines():
        name, sep, value = line.partition(":")
        if sep and name not in fields:
            fields[name] = value.strip()
    return fields


def check_status(fields):
    """The violations of one process: root uid, a non-empty capability set, or NoNewPrivs not set. A missing or
    unreadable field is a violation, not a pass."""
    bad = []
    uids = fields.get("Uid", "").split()
    if len(uids) != 4 or not all(u.isdigit() for u in uids):
        bad.append("uid_unreadable")
    elif any(int(u) == 0 for u in uids):
        bad.append("uid_root " + "/".join(uids))
    for name in CAP_FIELDS:
        value = fields.get(name, "")
        try:
            held = int(value, 16)
        except ValueError:
            bad.append(f"{name.lower()}_unreadable")
            continue
        if held:
            bad.append(f"{name.lower()}_held {value}")
    if fields.get("NoNewPrivs") != "1":
        bad.append("no_new_privs_not_set " + fields.get("NoNewPrivs", "missing"))
    return bad


def read_text(path):
    try:
        return Path(path).read_text(errors="replace")
    except OSError:
        return None


def stat_ppid(text):
    """The parent pid from /proc/PID/stat (the comm field may hold spaces and parentheses)."""
    try:
        return int(text.rsplit(")", 1)[1].split()[1])
    except (IndexError, ValueError):
        return None


def tree_pids(roots, proc="/proc"):
    """The roots that still exist, then every descendant of each (breadth first)."""
    children = {}
    for entry in os.listdir(proc):
        if entry.isdigit():
            text = read_text(Path(proc) / entry / "stat")
            ppid = stat_ppid(text) if text else None
            if ppid is not None:
                children.setdefault(ppid, []).append(int(entry))
    seen, queue = [], [r for r in roots]
    while queue:
        pid = queue.pop(0)
        if pid in seen:
            continue
        seen.append(pid)
        queue.extend(sorted(children.get(pid, [])))
    return seen


def assert_tree(pids, proc="/proc", with_tree=True, gone=()):
    """The assertion over `pids` (and their descendants). `gone` names pids that were expected and are missing: a
    violation (a process that cannot be read proves nothing)."""
    procs, violations = [], [f"pid_gone {p}" for p in gone]
    for pid in (tree_pids(pids, proc) if with_tree else list(pids)):
        status = read_text(Path(proc) / str(pid) / "status")
        if status is None:
            if pid in pids:
                violations.append(f"pid_unreadable {pid}")
            continue  # a descendant that exited between the scan and the read
        fields = parse_status(status)
        comm = (read_text(Path(proc) / str(pid) / "comm") or "?").strip()
        bad = check_status(fields)
        procs.append({
            "pid": pid, "comm": comm, "uid": fields.get("Uid", "").split(), "cap": {n: fields.get(n, "") for n in CAP_FIELDS},
            "no_new_privs": fields.get("NoNewPrivs", ""), "violations": bad,
        })
        violations += [f"{pid}({comm}) {b}" for b in bad]
    if not procs and not violations:
        violations.append("no_process_read")
    return {"processes": procs, "violations": violations}


def summary_line(mode, ok, tree, sudo):
    procs = tree["processes"] if tree else []
    uids = {p["uid"][0] for p in procs if p["uid"]}
    caps = {n: {p["cap"][n] for p in procs} for n in ("CapEff", "CapPrm", "CapBnd")}
    nnp = {p["no_new_privs"] for p in procs}

    def one(values, fmt=lambda v: v):
        return fmt(next(iter(values))) if len(values) == 1 else ("none" if not values else "mixed")

    def capval(values):
        if not values:
            return "none"
        try:
            return "0" if not any(int(v, 16) for v in values) else "held"
        except ValueError:
            return "unreadable"

    parts = [f"mode={mode}", f"ok={int(ok)}", f"uid={one(uids)}", f"capeff={capval(caps['CapEff'])}",
             f"capprm={capval(caps['CapPrm'])}", f"capbnd={capval(caps['CapBnd'])}", f"nnp={one(nnp)}", f"procs={len(procs)}"]
    if sudo is not None:
        parts += [f"sudo_entries={sudo['entries']}", f"sudo_violations={len(sudo['violations'])}"]
    return "KFROOT_ASSERT " + " ".join(parts)


# ------------------------------------------------------------------------------------------------ bootstrap
class Step:
    def __init__(self, name, argv=None, required=True, text=None, path=None, mode=None):
        self.name, self.argv, self.required = name, argv, required
        self.text, self.path, self.mode = text, path, mode

    def describe(self):
        return {"name": self.name, "argv": self.argv, "path": self.path, "required": self.required}


def nvidia_gpu_count(proc="/proc"):
    d = Path(proc) / "driver/nvidia/gpus"
    return len(list(d.iterdir())) if d.is_dir() else 0


def _group_exists(name):
    try:
        grp.getgrnam(name)
        return True
    except KeyError:
        return False


def _kvm_node_ok(node):
    """/dev/kvm is group kvm and group read/write already (nothing to do)."""
    try:
        st = node.stat()
        return st.st_gid == grp.getgrnam("kvm").gr_gid and (st.st_mode & 0o060) == 0o060
    except (OSError, KeyError):
        return False


def _world_rw(path):
    try:
        return (path.stat().st_mode & 0o006) == 0o006
    except OSError:
        return False


def _owned_by(path, user):
    try:
        return path.stat().st_uid == pwd.getpwnam(user).pw_uid
    except (OSError, KeyError):
        return False


def plan(user=DEFAULT_USER, workspace="/workspace", dev="/dev", proc="/proc", chown=(), audit_log=SUDO_LOG,
         only_needed=False, memlock=True, lock=GPU_LOCK, dmesg_restrict="/proc/sys/kernel/dmesg_restrict"):
    """The root-run box setup as data: every step is an argv (or a file write) so a test reads the plan and a dry run
    prints it. Required steps stop the bootstrap; the others are recorded and the run continues (the harness refuses by
    name at its own preflight if it needed them).
    only_needed=True (2026-11-20, the audit: docs/design/V3_KFROOT_AUDIT.md) leaves a step out when its effect is already
    there (group kvm exists, /dev/kvm is group kvm and 0660, the device nodes exist and are world read/write, dmesg is
    readable, the workspace directories are the user's, the lock file is 0666): a step that is not run is not a kfroot step
    of the run. memlock=False leaves the limits.d file out (the rehearsal ran the rootless suite at the default limit)."""
    check_user(user)
    dev = Path(dev)
    need = (lambda missing: missing) if only_needed else (lambda missing: True)
    steps = []
    if need(not _group_exists("kvm")):
        steps.append(Step("group_kvm", ["sh", "-c", "getent group kvm >/dev/null || groupadd --system kvm"]))
    steps.append(Step("user", ["sh", "-c",
                               f"if id {shlex.quote(user)} >/dev/null 2>&1; then usermod -aG kvm {shlex.quote(user)}; "
                               f"else useradd --create-home --shell /bin/bash --groups kvm {shlex.quote(user)}; fi"]))
    if (dev / "kvm").exists():
        if need(not _kvm_node_ok(dev / "kvm")):
            steps += [Step("dev_kvm_group", ["chgrp", "kvm", str(dev / "kvm")]),
                      Step("dev_kvm_mode", ["chmod", "0660", str(dev / "kvm")])]
    else:
        steps.append(Step("dev_kvm_missing", ["false"], required=False))
    # nvidia-modprobe creates the nodes; NoNewPrivs blocks it once the run has dropped, so it runs here, before the drop
    nodes = [dev / "nvidiactl", dev / "nvidia-uvm", dev / "nvidia-uvm-tools", dev / "nvidia-modeset"]
    if shutil.which("nvidia-modprobe"):
        count = max(nvidia_gpu_count(proc), 1)
        for n in range(count):
            if need(not (dev / "nvidiactl").exists() or not (dev / f"nvidia{n}").exists()):
                steps.append(Step(f"nvidia_node_{n}", ["nvidia-modprobe", "-c", str(n)], required=False))
        if need(not (dev / "nvidia-uvm").exists()):
            steps.append(Step("nvidia_uvm", ["nvidia-modprobe", "-u", "-c", "0"], required=False))
        if need(not (dev / "nvidia-modeset").exists()):
            steps.append(Step("nvidia_modeset", ["nvidia-modprobe", "-m"], required=False))
    present = [p for p in [*nodes, *sorted(dev.glob("nvidia[0-9]*"))] if p.exists()]
    if need(any(not _world_rw(p) for p in present)):
        steps.append(Step("nvidia_node_perms", ["sh", "-c",
                                                f"for f in {dev}/nvidiactl {dev}/nvidia[0-9]* {dev}/nvidia-uvm {dev}/nvidia-uvm-tools "
                                                f"{dev}/nvidia-modeset; do [ -c \"$f\" ] && chmod a+rw \"$f\"; done; true"],
                          required=False))
    try:
        restricted = Path(dmesg_restrict).read_text().strip() != "0"
    except OSError:
        restricted = True
    if need(restricted):
        steps.append(Step("dmesg_readable", ["sysctl", "-w", "kernel.dmesg_restrict=0"], required=False))
    if memlock:
        steps.append(Step("memlock_limit", None, path=LIMITS, mode=0o644,
                          text=f"# kayfabe kfroot.py: no memlock ceiling for the CI user\n{user} - memlock unlimited\n"))
    ws = shlex.quote(workspace)
    dirs = [f"{ws}", f"{ws}/bench", f"{ws}/winvm", f"{ws}/winapps", f"{ws}/prov"]
    if need(any(not _owned_by(Path(d.strip("'")), user) for d in dirs)):
        steps.append(Step("workspace", ["sh", "-c", f"install -d -o {shlex.quote(user)} -g {shlex.quote(user)} -m 0755 " + " ".join(dirs)]))
    for p in chown:
        steps.append(Step("chown_tree", ["chown", "-R", f"{user}:{user}", str(p)]))
    if need(not _world_rw(Path(lock))):
        steps.append(Step("gpu_lock", ["sh", "-c", f"[ -e {lock} ] || : > {lock}; chmod 0666 {lock}"]))
    if audit_log == SUDO_LOG:
        steps.append(Step("sudo_log", ["sh", "-c",
                                       f"touch {SUDO_LOG} && chown root:{shlex.quote(user)} {SUDO_LOG} && chmod 0640 {SUDO_LOG}"]))
    else:  # sudo-rs: the syslog file; the dropped tree reads it only with an ACL (an unreadable audit fails closed)
        steps.append(Step("sudo_log_acl", ["setfacl", "-m", f"u:{user}:r", audit_log], required=False))
    return steps


def bootstrap(user=DEFAULT_USER, workspace="/workspace", chown=(), dry_run=False, run=subprocess.run, root="/",
              visudo="visudo", dev="/dev", proc="/proc", only_needed=False, memlock=True, install_files=True):
    """Install first (it decides which file is the audit), then the planned steps. install_files=False (the payload, which is root and
    needs no launcher: `kfroot` runs the command directly as root) leaves the sudoers file, the launcher and the wrapper out, so a box
    without sudo is not a failure; the audit file is then the dedicated log, which the plan still creates."""
    if not dry_run and os.geteuid() != 0:
        raise KfrootError("bootstrap_needs_root")
    results = []
    if dry_run:
        audit_log = SUDO_LOG
        results.append({"name": "kfroot_files", "argv": None, "path": SUDOERS, "required": True, "rc": None})
    elif install_files:
        audit_log = install(user, root, visudo, run)["audit_log"]
        results.append({"name": "kfroot_files", "argv": None, "path": SUDOERS, "required": True, "rc": 0})
    else:
        audit_log = SUDO_LOG
    for step in plan(user, workspace, dev, proc, chown, audit_log, only_needed, memlock):
        if dry_run:
            results.append({**step.describe(), "rc": None})
            continue
        if step.text is not None:
            write_file(under(root, step.path), step.text, step.mode, str(root) == "/")
            rc = 0
        else:
            rc = run(step.argv, capture_output=True, text=True).returncode
        results.append({**step.describe(), "rc": rc})
        if rc and step.required:
            raise KfrootError(f"bootstrap_step_failed {step.name} rc={rc}")
    return results


# ------------------------------------------------------------------------------------------------ the audit as data
# docs/design/V3_KFROOT_AUDIT.md (2026-11-20) is the prose; this is the same table as data, so the payload can refuse and record
# by name and a test can hold the two together. A step that is not in STEPS is not audited: it is refused, never run.
CLASSES = ("SETUP-ONCE", "VM-RUN", "CI-CONVENIENCE", "TEST-MECHANISM")
RECORD_SCHEMA = 1
NAME_RE = re.compile(r"[a-z][a-z0-9_.-]{0,47}")
SO, VR, CC, TM = CLASSES


def _s(cls, where, reason, equivalent):
    return {"class": cls, "where": where, "reason": reason, "equivalent": equivalent}


STEPS = {
    # --- the explicit `kfroot <cmd>` lines of the harness
    "apt_prereqs": _s(SO, "scripts/bench/box/windows_gate.sh:install_prereqs", "dpkg database and /usr",
                      "apt-get install ovmf swtpm swtpm-tools sbsigntool python3-virt-firmware xorriso netpbm p7zip-full gcc-mingw-w64-x86-64 g++-mingw-w64-x86-64, once"),
    "swtpm_apparmor_rule": _s(SO, "scripts/bench/windows/win_vm.sh:apparmor_swtpm", "/etc/apparmor.d/local is root-owned",
                              "one line in /etc/apparmor.d/local/usr.bin.swtpm allowing the swtpm state directory, once"),
    "swtpm_apparmor_load": _s(SO, "scripts/bench/windows/win_vm.sh:apparmor_swtpm", "loading a profile needs CAP_MAC_ADMIN",
                              "apparmor_parser -r /etc/apparmor.d/usr.bin.swtpm, once (or a reboot)"),
    "dmesg_via_kfroot": _s(CC, "scripts/lib/kfroot.sh:kf_dmesg", "kernel.dmesg_restrict=1 needs CAP_SYSLOG",
                           "none: reading the kernel log is the Xid instrument of the CI, not something an app needs"),
    "drop": _s(TM, "scripts/ci/kfroot.py:run_prefix", "setuid, setgroups and the bounding set need root",
               "none: a user is already unprivileged; the drop is how the test makes the VM run rootless"),
    # --- the bootstrap plan (plan())
    "kfroot_files": _s(TM, "scripts/ci/kfroot.py:install", "writes /usr/local and /etc/sudoers.d", "none: only so the test can name its root steps"),
    "group_kvm": _s(CC, "scripts/ci/kfroot.py:plan", "groupadd", "none on a distro: group kvm comes with it (only a minimal image lacks it)"),
    "user": _s(TM, "scripts/ci/kfroot.py:plan", "useradd", "none: the user is the account they log in as; membership of group kvm is dev_kvm_group"),
    "dev_kvm_group": _s(SO, "scripts/ci/kfroot.py:plan", "chgrp of a device node", "usermod -aG kvm $USER (log in again) and a udev rule giving /dev/kvm group kvm, once"),
    "dev_kvm_mode": _s(SO, "scripts/ci/kfroot.py:plan", "chmod of a device node", "the same udev rule: KERNEL==\"kvm\", GROUP=\"kvm\", MODE=\"0660\""),
    "dev_kvm_missing": _s(SO, "scripts/ci/kfroot.py:plan", "(a recorded absence: no /dev/kvm)", "enable virtualisation in the firmware or load kvm_intel or kvm_amd, once"),
    "nvidia_node": _s(CC, "scripts/ci/kfroot.py:plan", "nvidia-modprobe is setuid and NoNewPrivs defeats it",
                      "none: a user's first nvidia-smi or CUDA call creates the node through the setuid helper"),
    "nvidia_uvm": _s(CC, "scripts/ci/kfroot.py:plan", "as nvidia_node", "none: as nvidia_node"),
    "nvidia_modeset": _s(CC, "scripts/ci/kfroot.py:plan", "as nvidia_node", "none: as nvidia_node"),
    "nvidia_node_perms": _s(CC, "scripts/ci/kfroot.py:plan", "chmod of device nodes", "none: the nodes of the rehearsal image were 0666 (traces/release_manual_rehearsal_20261115/README.md:36)"),
    "dmesg_readable": _s(CC, "scripts/ci/kfroot.py:plan", "sysctl", "optional: kernel.dmesg_restrict=0 only to watch Xid; the apps do not need it"),
    "memlock_limit": _s(CC, "scripts/ci/kfroot.py:plan", "/etc/security/limits.d",
                        "none shown to be needed: limits.conf or LimitMEMLOCK only if a run fails on a locked allocation"),
    "workspace": _s(CC, "scripts/ci/kfroot.py:plan", "directories the CI user owns under /workspace", "none: a user works in their own home"),
    "chown_tree": _s(CC, "scripts/ci/kfroot.py:plan", "root-owned build outputs of the root provision stage", "none: a user builds as themselves"),
    "gpu_lock": _s(CC, "scripts/ci/kfroot.py:plan", "a /tmp lock file that root created is refused to the user (rehearsal README.md:39)", "none: one user, one lock owner"),
    "sudo_log": _s(TM, "scripts/ci/kfroot.py:plan", "a root-owned log readable by the CI user", "none: the audit of the test's own kfroot calls"),
    "sudo_log_acl": _s(TM, "scripts/ci/kfroot.py:plan", "setfacl on the syslog file", "none: as sudo_log"),
    # --- the payload (scripts/ci/vast_payload.py --rootless)
    "provision": _s(SO, "scripts/bench/box/provision_full.sh", "apt, the host driver, kernel modules and services; also nbd and mount for the fast guest",
                    "install the distro and the NVIDIA driver once; build kayfabe as the user"),
    "toolchain_for_user": _s(CC, "scripts/ci/kfroot.py:Rootless.common_setup", "the Rust toolchain is under /root, which the CI user cannot enter",
                             "none: a user installs rustup in their own home"),
    "root_stage": _s(VR, "the payload flag --rootless-root-stage", "a stage that was not converted runs as root by explicit approval",
                     "none yet: the stage is not proven rootless"),
}


def _p(as_, status, scripts=(), prepare=(), blocker="", evidence=""):
    return {"as": as_, "status": status, "scripts": tuple(scripts), "prepare": tuple(prepare), "blocker": blocker, "evidence": evidence}


XID = "scripts/bench/host_xid_watch.sh"
# The stock-patch audit (scripts/ci/no_patch_check.py) follows every literal script name in a non-comment line, so two entry-script names are
# joined here instead of written whole: a literal would make this module reach the apps bundle tools it never calls.
WIN_SCRIPTS = ("scripts/bench/box/windows_gate.sh", "scripts/bench/box/windows_" + "apps.sh", "scripts/bench/windows/win_vm.sh", XID)
REHEARSAL = "traces/release_manual_rehearsal_20261115/README.md"
# as: setup = root by nature (one recorded kfroot step), user = starts dropped, self_drop = a root launcher that drops its own VM,
# root_stage = not converted: refused unless approved by name. status: probed = a rootless hand run exists (evidence), not_yet_run.
STAGE_POLICY = {
    "provision": _p("setup", "not_applicable", evidence="docs/design/V3_KFROOT_AUDIT.md"),
    "cuda_host": _p("user", "probed", ("scripts/bench/cuda_ladder.sh", XID), evidence=REHEARSAL),
    "cuda_guest": _p("user", "probed", ("scripts/bench/cuda_ladder.sh", XID), evidence=REHEARSAL),
    "merge_bar": _p("root_stage", "not_yet_run", blocker="scripts/fastguest/build_fast_guest (.sh): modprobe nbd, qemu-nbd --connect, mount -o ro (host root operations)"),
    "apps": _p("root_stage", "not_yet_run", blocker="scripts/apps/apps_matrix (.sh): the host side writes /opt/apps (mkdir, cp) and its header says 'as root'"),
    "apps_discovery": _p("user", "not_yet_run", ("scripts/ci/apps_discovery" + "_stage.py", XID)),
    "kfml": _p("user", "not_yet_run", ("scripts/ci/kfml" "_stage.py", XID)),
    "install_flow": _p("self_drop", "not_yet_run", blocker="scripts/ci/install_flow (.py): a root launcher (not_root refusal) that drops its own VM tree"),
    "windows_gate": _p("user", "not_yet_run", WIN_SCRIPTS, ("bootstrap", "toolchain", "windows_prepare")),
    "windows_apps": _p("user", "not_yet_run", WIN_SCRIPTS, ("bootstrap", "toolchain", "windows_prepare")),
    "windows_gate_2": _p("user", "not_yet_run", WIN_SCRIPTS, ("bootstrap", "toolchain", "windows_prepare")),
    "windows_open_rows": _p("user", "not_yet_run", WIN_SCRIPTS, ("bootstrap", "toolchain", "windows_prepare")),
    "propimage_apps": _p("root_stage", "not_yet_run", blocker="scripts/ci/propimage_apps (.py) build_swtpm: apt-get install and make install under /opt/tpm"),
    "propimage_apps_b": _p("root_stage", "not_yet_run", blocker="scripts/ci/propimage_apps (.py) build_swtpm (reused): apt-get install and make install under /opt/tpm"),
    "matlab": _p("root_stage", "not_yet_run", blocker="scripts/ci/propimage_apps (.py) build_swtpm (reused): apt-get install and make install under /opt/tpm"),
    "specviewperf": _p("root_stage", "not_yet_run", blocker="scripts/bench/box/specviewperf_stage (.sh): a blanket 'not root' refusal and raw dmesg"),
}
USER_PREPARE = ("bootstrap", "toolchain")  # the common setup every user stage needs once, before its first start


# ------------------------------------------------------------------------------------------------ the static check
# An entry script of a stage that starts dropped may not contain a root operation that is not a `kfroot` line. This reads the entry
# scripts only (not what they call): it is the check that a stage cannot silently depend on root, not a proof that it does not.
_OPS = ("sudo|apt-get|apt|dpkg|modprobe|rmmod|insmod|mount|umount|losetup|qemu-nbd|sysctl|chown|chgrp|mknod|iptables|nft|systemctl|"
        "apparmor_parser|nvidia-modprobe|useradd|usermod|groupadd|setcap|swapon|dmesg")
SHELL_OP = re.compile(r"(?:^|[;&|({`]|\$\(|\bthen\b|\bdo\b|\belse\b)\s*(?:[A-Za-z_][A-Za-z0-9_]*=\S*\s+)*(" + _OPS + r")(?=\s|$|;|\)|\})")
PY_OP = re.compile(r"""\[\s*["'](""" + _OPS + r""")["']""")
ROOT_GUARD = re.compile(r"""id -u\)?"?\s*(?:=|-eq)\s*0|geteuid\(\)\s*(?:!=|==)\s*0|\$EUID""")


def unmarked_root_ops(text, python=False):
    """[(line number, operation)] for each root operation outside a `kfroot` or `kf_dmesg` line, and each blanket root guard."""
    found = []
    for n, line in enumerate(text.splitlines(), 1):
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        code_part = re.sub(r"\s#\s.*$", "", line)
        if "kfroot " in code_part or "kf_dmesg" in code_part:
            continue
        m = (PY_OP if python else SHELL_OP).search(code_part)
        if m:
            found.append((n, m.group(1)))
        elif ROOT_GUARD.search(code_part):
            found.append((n, "root_guard"))
    return found


def lint_stage(stage, root):
    """The refusal tokens for one stage ([] = nothing found). Only a stage that starts dropped is read."""
    policy = STAGE_POLICY[stage]
    if policy["as"] != "user":
        return []
    out = []
    for rel in policy["scripts"]:
        path = Path(root) / rel
        text = read_text(path)
        if text is None:
            out.append(f"missing_script:{stage}:{rel}")
            continue
        out += [f"unmarked_root_op:{stage}:{rel}:{n}:{op}" for n, op in unmarked_root_ops(text, rel.endswith(".py"))]
    return out


def stage_refusals(stages, root, approved=()):
    """Refuse by name, before any stage starts: a stage the audit does not know, a root stage nobody approved, an approval that
    names a stage that is not a root stage (approving a user stage as root would defeat the test), and an unmarked root operation."""
    tokens = []
    for name in approved:
        if name not in STAGE_POLICY or STAGE_POLICY[name]["as"] != "root_stage":
            tokens.append(f"approval_not_a_root_stage:{name}")
    for name in stages:
        policy = STAGE_POLICY.get(name)
        if policy is None:
            tokens.append(f"unclassified:{name}")
        elif policy["as"] == "root_stage" and name not in approved:
            tokens.append(f"root_stage_not_approved:{name}")
        else:
            tokens += lint_stage(name, root)
    return tokens


# ------------------------------------------------------------------------------------------------ the record of a run
USED_PREFIX = "KFROOT_USED"
REFUSED_PREFIX = "KFROOT_REFUSED"


def classify_command(text):
    """Which audited step a `kfroot <cmd>` call was: by the command, never by the caller's say-so. None = not audited."""
    words = text.split()
    base = [os.path.basename(w) for w in words]
    if "apt-get" in base or "apt" in base or ("vast_setup.py" in base and "apt" in base):
        return "apt_prereqs"
    if base[:1] == ["tee"] and any("/apparmor.d/" in w for w in words):
        return "swtpm_apparmor_rule"
    if base[:1] == ["apparmor_parser"]:
        return "swtpm_apparmor_load"
    if base[:1] == ["dmesg"]:
        return "dmesg_via_kfroot"
    return None


def printable(text, limit=120):
    return "".join(c if " " <= c <= "~" else "?" for c in str(text))[:limit]


class Record:
    """What a rootless payload run did as root: every audited kfroot step it ran (with the stage and the return code), every
    stage that was approved to run as root, and every `kfroot` call a dropped stage attempted (refused: a root need during the VM
    run). Bounded and printable: vast_report.validate_rootless reads it back."""

    LIMIT = 64

    def __init__(self, user, memlock):
        self.user, self.memlock = user, bool(memlock)
        self.steps, self.stages, self.refused, self.unaudited = [], {}, [], []

    def stage(self, name, how, status):
        self.stages[name] = {"as": how, "status": status}

    def step(self, name, stage, rc, source="payload", detail=""):
        if name not in STEPS:
            self.unaudited.append(printable(name, 48))
            name = "unaudited"
        if len(self.steps) < self.LIMIT:
            self.steps.append({"step": name, "stage": printable(stage, 32), "rc": int(rc), "source": source, "detail": printable(detail)})

    def used(self, text, stage):
        """The KFROOT_USED lines of a script run as root (scripts/lib/kfroot.sh writes them when KF_KFROOT_RECORD names a file)."""
        for line in text.splitlines():
            if not line.startswith(USED_PREFIX + "\t"):
                continue
            parts = line.split("\t", 2)
            if len(parts) != 3:
                self.unaudited.append("unparsed")
                continue
            how, command = parts[1], parts[2]
            name = classify_command(command)
            if name is None:
                self.unaudited.append(printable(command, 48))
            else:
                self.step(name, stage, 0 if how in ("root", "sudo") else 126, "script", command)

    def refusals(self, text, stage):
        """KFROOT_REFUSED lines in a dropped stage's output: the stage needed root inside the VM run."""
        for line in text.splitlines():
            if re.search(r"(?:^|\s)" + REFUSED_PREFIX + r"\b", line) and len(self.refused) < 16:
                self.refused.append({"stage": printable(stage, 32), "line": printable(line, 160)})

    def verdict(self):
        if self.unaudited or self.refused:
            return "VIOLATED"
        if any(s["as"] == "root_stage" for s in self.stages.values()):
            return "ROOT_STAGES_APPROVED"
        return "ROOTLESS"

    def to_json(self):
        return {"schema": RECORD_SCHEMA, "verdict": self.verdict(), "user": self.user, "memlock_raised": self.memlock,
                "stages": dict(sorted(self.stages.items())), "steps": list(self.steps), "refused": list(self.refused),
                "unaudited": sorted(set(self.unaudited))[:8]}


# ------------------------------------------------------------------------------------------------ the payload's orchestration
class RootlessRefused(Exception):
    """A root step the payload had to run failed, or was not allowed; the message is one short token."""


class Rootless:
    """What the payload's --rootless require does around the stages (the payload itself runs as root, as Vast starts it).
    provision runs as root and is one recorded SETUP-ONCE step. The first stage that starts dropped triggers the common setup
    (bootstrap with only the needed steps, the checkout and work trees owned by the user, the Rust toolchain copied to the
    user's home). A windows stage first runs `windows_gate.sh --prepare` as root (packages, the swtpm AppArmor rule). Then the
    stage's whole command (including host_xid_watch.sh) starts as the user with every capability set empty and NoNewPrivs.
    A root stage the owner approved runs as root and the trees go back to the user afterwards. Every effect goes through
    `run`, so the GPU-free tests read the argv and no root is needed."""

    Refused = RootlessRefused

    def __init__(self, user, root, run=subprocess.run, environ=None, workspace="/workspace", evidence=None, memlock=False,
                 geteuid=os.geteuid, home=None, kfroot_py=None, prov_dir=None, python=sys.executable, bootstrap_fn=None, toolchain_src="/root"):
        check_user(user)
        self.user, self.root, self.run, self.workspace = user, Path(root), run, workspace
        self._environ = None if environ is None else dict(environ)   # None: read os.environ when it is used (the payload sets BASH_ENV and PATH after this object exists)
        self.evidence, self.memlock, self.geteuid = evidence, bool(memlock), geteuid
        self.home = home or self._home()
        self.kfroot_py = str(kfroot_py or Path(__file__).resolve())
        self.prov_dir = prov_dir or f"{workspace}/prov"
        self.python, self.toolchain_src = python, Path(toolchain_src)
        self.bootstrap_fn = bootstrap_fn or bootstrap
        self.record = Record(user, memlock)
        self.common_done = self.prepared = False

    @property
    def environ(self):
        return dict(os.environ) if self._environ is None else self._environ

    def _home(self):
        try:
            return pwd.getpwnam(self.user).pw_dir
        except KeyError:
            return f"/home/{self.user}"

    def trees(self):
        """The trees the user must own: the checkout and the work directories that exist (a chown of a missing one would fail the bootstrap)."""
        out = [str(self.root), f"{self.workspace}/bench", f"{self.workspace}/prov", f"{self.workspace}/apps"]
        if self.evidence:
            out.append(str(self.evidence))
        return [t for t in out if Path(t).exists()]

    def preflight(self, stages, approved):
        """[] or the refusal tokens: the payload must be root (it starts the drop), and stage_refusals."""
        tokens = [] if self.geteuid() == 0 else ["payload_not_root"]
        tokens += stage_refusals(stages, self.root, approved)
        return tokens

    def _do(self, name, stage, argv, source="payload", required=True):
        p = self.run(argv, capture_output=True, text=True)
        self.record.step(name, stage, p.returncode, source, " ".join(argv))
        if p.returncode and required:
            raise RootlessRefused(f"step_failed:{name}")
        return p

    def common_setup(self, stage):
        """Once, at the first stage that starts dropped. Safe to call again: it only repeats when the trees must go back to the user."""
        if self.common_done:
            return
        self.common_done = True
        try:
            results = self.bootstrap_fn(self.user, self.workspace, chown=self.trees(), run=self.run, only_needed=True, memlock=self.memlock,
                                        install_files=False)
        except KfrootError as error:
            raise RootlessRefused("bootstrap:" + printable(error, 60).replace(" ", "_")) from None
        for r in results:
            name = re.sub(r"_[0-9]+$", "", r["name"])
            name = "nvidia_node" if name.startswith("nvidia_node") and name != "nvidia_node_perms" else name
            self.record.step(name, stage, r["rc"] or 0, "bootstrap", r["name"])
        src, dst = self.toolchain_src, Path(self.home)
        for sub in (".cargo", ".rustup"):
            if (src / sub).exists() and not (dst / sub).exists():
                self._do("toolchain_for_user", stage, ["cp", "-a", str(src / sub), str(dst / sub)])
                self._do("toolchain_for_user", stage, ["chown", "-R", f"{self.user}:{self.user}", str(dst / sub)])

    def give_back(self, stage):
        """After a stage that ran as root: the trees belong to the user again (a root-created file would stop the next dropped stage)."""
        if self.common_done:
            for tree in self.trees():
                if Path(tree).exists():
                    self._do("chown_tree", stage, ["chown", "-R", f"{self.user}:{self.user}", tree], required=False)

    def prepare(self, stage, tag, env=None):
        """The root steps a windows stage needs before it is dropped: windows_gate.sh --prepare (its own kfroot lines, recorded)."""
        policy = STAGE_POLICY[stage]
        if "windows_prepare" not in policy["prepare"] or self.prepared:
            return
        self.prepared = True
        mark = Path(self.prov_dir) / "kfroot_used.tsv"
        mark.parent.mkdir(parents=True, exist_ok=True)
        before = read_text(mark) or ""
        e = dict(self.environ if env is None else env, KF_KFROOT_RECORD=str(mark), KF_ROOTLESS="off")
        p = self.run(["bash", str(self.root / "scripts/bench/box/windows_gate.sh"), "--prepare", self.prov_dir],
                     capture_output=True, text=True, env=e)
        after = read_text(mark) or ""
        self.record.used(after[len(before):] if after.startswith(before) else after, stage)
        if p.returncode:
            raise RootlessRefused("step_failed:windows_prepare")

    def environment(self):
        """The assignments the dropped stage gets in place of root's: HOME, the user's toolchain on PATH, the provision directory."""
        path = [d for d in self.environ.get("PATH", "").split(":") if d and not d.startswith("/root/")]
        path.insert(0, f"{self.home}/.cargo/bin")
        return [f"HOME={self.home}", f"USER={self.user}", f"LOGNAME={self.user}", "PATH=" + ":".join(path),
                f"KF_PROV_DIR={self.prov_dir}"]

    def dropped(self, command):
        """The argv that starts `command` as the user: env (the assignments) then `kfroot.py run`, which raises no limit unless memlock."""
        run = [self.python, "-I", self.kfroot_py, "run", "--user", self.user] + ([] if self.memlock else ["--no-memlock"]) + ["--"]
        return ["env", "-u", "CARGO_HOME", "-u", "RUSTUP_HOME", *self.environment(), *run, *command]

    def as_root(self, command):
        """An approved root stage: root, with git told that the user-owned checkout is safe (git refuses a tree another uid owns)."""
        return ["env", "GIT_CONFIG_COUNT=1", "GIT_CONFIG_KEY_0=safe.directory", "GIT_CONFIG_VALUE_0=*", *command]

    def start(self, stage, command, approved=(), tag=""):
        """The argv to run for `stage` (`approved`: the root stages the owner named, checked by preflight). Raises RootlessRefused when a root step it needs failed."""
        policy = STAGE_POLICY[stage]
        how = policy["as"]
        self.record.stage(stage, how, policy["status"])
        if how == "setup":
            self.record.step("provision", stage, 0, "payload", "stage runs as root: box setup")
            return command
        if how == "root_stage":
            self.record.step("root_stage", stage, 0, "payload", "approved by name: " + policy["blocker"])
            return self.as_root(command)
        if how == "self_drop":
            return self.as_root(command)
        self.common_setup(stage)
        self.prepare(stage, tag)
        return self.dropped(command)

    def finish(self, stage, output_text=""):
        how = STAGE_POLICY[stage]["as"]
        if how == "user":
            self.record.refusals(output_text, stage)
        elif how in ("root_stage", "self_drop"):
            self.give_back(stage)


# ------------------------------------------------------------------------------------------------ command line
def cmd_install(a):
    print(json.dumps(install(a.user, a.root)))
    return 0


def cmd_verify(a):
    problems = verify(a.user, a.root)
    print("KFROOT_VERIFY " + ("ok" if not problems else "problems=" + "; ".join(problems)))
    return 1 if problems else 0


def cmd_bootstrap(a):
    results = bootstrap(a.user, a.workspace, a.chown, a.dry_run, only_needed=a.only_needed, memlock=not a.no_memlock)
    print("KFROOT_BOOTSTRAP " + json.dumps({"schema": SCHEMA, "user": a.user, "steps": results}, sort_keys=True))
    return 0


def cmd_drop_argv(a):
    uid, gid, kvm = identity(a.user, a.kvm_group)
    print(shlex.join(run_prefix(uid, gid, kvm, os.geteuid() != 0, not a.no_memlock)))
    return 0


def cmd_run(a):
    if not a.command:
        raise KfrootError("no_command")
    uid, gid, kvm = identity(a.user, a.kvm_group)
    env = dict(os.environ)
    env.setdefault("KF_ROOTLESS", "require")
    log = a.log or default_log()
    env["KF_SUDO_LOG_MARK"] = str(audit_mark(log))
    env["KF_SUDO_LOG"] = log
    env["KF_ROOTLESS_USER"] = a.user
    argv = run_prefix(uid, gid, kvm, os.geteuid() != 0, not a.no_memlock, os.environ) + a.command
    os.execvpe(argv[0], argv, env)


def cmd_steps(a):
    print(json.dumps({"schema": RECORD_SCHEMA, "classes": CLASSES, "steps": STEPS,
                      "stages": {k: {**v, "scripts": list(v["scripts"]), "prepare": list(v["prepare"])} for k, v in STAGE_POLICY.items()}},
                     indent=1, sort_keys=True))
    return 0


def cmd_lint_stage(a):
    if a.stage not in STAGE_POLICY:
        raise KfrootError(f"unclassified:{a.stage}")
    tokens = lint_stage(a.stage, a.root)
    for t in tokens:
        print("KFROOT_LINT " + t)
    print(f"KFROOT_LINT_RESULT stage={a.stage} as={STAGE_POLICY[a.stage]['as']} findings={len(tokens)}")
    return 1 if tokens else 0


def cmd_audit_mark(a):
    print(audit_mark(a.log or default_log()))
    return 0


def cmd_audit_check(a):
    text = audit_text(a.log or default_log(), a.since)
    result = audit_check(text, tuple(a.user))
    print("KFROOT_AUDIT " + json.dumps(result, sort_keys=True))
    return 1 if result["violations"] else 0


def cmd_assert(a):
    pids, gone = list(a.pid), []
    for f in a.pid_file:
        text = (read_text(f) or "").strip()
        if text.isdigit():
            pids.append(int(text))
        else:
            gone.append(f)
    proc = os.environ.get("KFROOT_PROC", "/proc")
    tree = assert_tree(pids, proc, gone=[f"pidfile {g}" for g in gone]) if (pids or gone) else None
    sudo = None
    if a.sudo_since is not None:
        text = audit_text(a.sudo_log or default_log(), a.sudo_since)
        sudo = audit_check(text, tuple(a.user))
    violations = (tree["violations"] if tree else []) + [f"sudo {v['command'][:80]}" for v in (sudo["violations"] if sudo else [])]
    if tree is None and sudo is None:
        raise KfrootError("nothing_to_assert")
    ok = not violations
    doc = {"schema": SCHEMA, "mode": a.mode, "ok": ok, "violations": violations, "tree": tree, "sudo": sudo}
    print(summary_line(a.mode, ok, tree, sudo))
    for v in violations[:6]:
        print("KFROOT_ASSERT_VIOLATION " + v)
    if a.out:
        Path(a.out).write_text(json.dumps(doc, indent=1, sort_keys=True) + "\n")
    return 0 if ok or a.mode != "require" else 1


def parser():
    p = argparse.ArgumentParser(prog="kfroot.py", description=__doc__.split("\n")[0])
    sub = p.add_subparsers(dest="cmd", required=True)

    def user_arg(s):
        s.add_argument("--user", default=DEFAULT_USER)

    s = sub.add_parser("install")
    user_arg(s)
    s.add_argument("--root", default="/")
    s.set_defaults(fn=cmd_install)
    s = sub.add_parser("verify")
    user_arg(s)
    s.add_argument("--root", default="/")
    s.set_defaults(fn=cmd_verify)
    s = sub.add_parser("bootstrap")
    user_arg(s)
    s.add_argument("--workspace", default="/workspace")
    s.add_argument("--chown", action="append", default=[], help="a tree the CI user must own (the checkout)")
    s.add_argument("--dry-run", action="store_true")
    s.add_argument("--only-needed", action="store_true", help="leave out every step whose effect is already there (the payload's mode)")
    s.add_argument("--no-memlock", action="store_true", help="do not write the limits.d memlock file")
    s.set_defaults(fn=cmd_bootstrap)
    for name, fn in (("drop-argv", cmd_drop_argv), ("run", cmd_run)):
        s = sub.add_parser(name)
        user_arg(s)
        s.add_argument("--kvm-group", default="kvm")
        s.add_argument("--no-memlock", action="store_true")
        if name == "run":
            s.add_argument("--log", default=None)
            s.add_argument("command", nargs=argparse.REMAINDER)
        s.set_defaults(fn=fn)
    s = sub.add_parser("steps")
    s.set_defaults(fn=cmd_steps)
    s = sub.add_parser("lint-stage")
    s.add_argument("stage")
    s.add_argument("--root", default=str(Path(__file__).resolve().parents[2]))
    s.set_defaults(fn=cmd_lint_stage)
    s = sub.add_parser("audit-mark")
    s.add_argument("--log", default=None)
    s.set_defaults(fn=cmd_audit_mark)
    s = sub.add_parser("audit-check")
    s.add_argument("--since", type=int, required=True)
    s.add_argument("--log", default=None)
    s.add_argument("--user", action="append", default=None)
    s.set_defaults(fn=cmd_audit_check)
    s = sub.add_parser("assert-tree")
    s.add_argument("--pid", type=int, action="append", default=[])
    s.add_argument("--pid-file", action="append", default=[])
    s.add_argument("--mode", choices=("require", "record"), default="record")
    s.add_argument("--out")
    s.add_argument("--sudo-since", type=int)
    s.add_argument("--sudo-log", default=None)
    s.add_argument("--user", action="append", default=None)
    s.set_defaults(fn=cmd_assert)
    return p


def main(argv=None):
    a = parser().parse_args(argv)
    if getattr(a, "user", None) is None:
        a.user = [DEFAULT_USER]
    if a.cmd == "run" and a.command[:1] == ["--"]:
        a.command = a.command[1:]
    try:
        return a.fn(a)
    except KfrootError as e:
        print(f"KFROOT_REFUSED {e}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
