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
    (Same argv as install_flow.py's drop_argv in PR #423: that file should import this one.)"""
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


def plan(user=DEFAULT_USER, workspace="/workspace", dev="/dev", proc="/proc", chown=(), audit_log=SUDO_LOG):
    """The root-run box setup as data: every step is an argv (or a file write) so a test reads the plan and a dry run
    prints it. Required steps stop the bootstrap; the others are recorded and the run continues (the harness refuses by
    name at its own preflight if it needed them)."""
    check_user(user)
    dev = Path(dev)
    steps = [
        Step("group_kvm", ["sh", "-c", "getent group kvm >/dev/null || groupadd --system kvm"]),
        Step("user", ["sh", "-c",
                      f"if id {shlex.quote(user)} >/dev/null 2>&1; then usermod -aG kvm {shlex.quote(user)}; "
                      f"else useradd --create-home --shell /bin/bash --groups kvm {shlex.quote(user)}; fi"]),
    ]
    if (dev / "kvm").exists():
        steps += [Step("dev_kvm_group", ["chgrp", "kvm", str(dev / "kvm")]),
                  Step("dev_kvm_mode", ["chmod", "0660", str(dev / "kvm")])]
    else:
        steps.append(Step("dev_kvm_missing", ["false"], required=False))
    # nvidia-modprobe creates the nodes; NoNewPrivs blocks it once the run has dropped, so it runs here, before the drop
    if shutil.which("nvidia-modprobe"):
        count = max(nvidia_gpu_count(proc), 1)
        for n in range(count):
            steps.append(Step(f"nvidia_node_{n}", ["nvidia-modprobe", "-c", str(n)], required=False))
        steps += [Step("nvidia_uvm", ["nvidia-modprobe", "-u", "-c", "0"], required=False),
                  Step("nvidia_modeset", ["nvidia-modprobe", "-m"], required=False)]
    steps.append(Step("nvidia_node_perms", ["sh", "-c",
                                            f"for f in {dev}/nvidiactl {dev}/nvidia[0-9]* {dev}/nvidia-uvm {dev}/nvidia-uvm-tools "
                                            f"{dev}/nvidia-modeset; do [ -c \"$f\" ] && chmod a+rw \"$f\"; done; true"],
                      required=False))
    steps.append(Step("dmesg_readable", ["sysctl", "-w", "kernel.dmesg_restrict=0"], required=False))
    steps.append(Step("memlock_limit", None, path=LIMITS, mode=0o644,
                      text=f"# kayfabe kfroot.py: no memlock ceiling for the CI user\n{user} - memlock unlimited\n"))
    ws = shlex.quote(workspace)
    dirs = [f"{ws}", f"{ws}/bench", f"{ws}/winvm", f"{ws}/winapps", f"{ws}/prov"]
    steps.append(Step("workspace", ["sh", "-c", f"install -d -o {shlex.quote(user)} -g {shlex.quote(user)} -m 0755 " + " ".join(dirs)]))
    for p in chown:
        steps.append(Step("chown_tree", ["chown", "-R", f"{user}:{user}", str(p)]))
    steps.append(Step("gpu_lock", ["sh", "-c", f"[ -e {GPU_LOCK} ] || : > {GPU_LOCK}; chmod 0666 {GPU_LOCK}"]))
    if audit_log == SUDO_LOG:
        steps.append(Step("sudo_log", ["sh", "-c",
                                       f"touch {SUDO_LOG} && chown root:{shlex.quote(user)} {SUDO_LOG} && chmod 0640 {SUDO_LOG}"]))
    else:  # sudo-rs: the syslog file; the dropped tree reads it only with an ACL (an unreadable audit fails closed)
        steps.append(Step("sudo_log_acl", ["setfacl", "-m", f"u:{user}:r", audit_log], required=False))
    return steps


def bootstrap(user=DEFAULT_USER, workspace="/workspace", chown=(), dry_run=False, run=subprocess.run, root="/",
              visudo="visudo", dev="/dev", proc="/proc"):
    """Install first (it decides which file is the audit), then the planned steps."""
    if not dry_run and os.geteuid() != 0:
        raise KfrootError("bootstrap_needs_root")
    results = []
    if dry_run:
        audit_log = SUDO_LOG
        results.append({"name": "kfroot_files", "argv": None, "path": SUDOERS, "required": True, "rc": None})
    else:
        audit_log = install(user, root, visudo, run)["audit_log"]
        results.append({"name": "kfroot_files", "argv": None, "path": SUDOERS, "required": True, "rc": 0})
    for step in plan(user, workspace, dev, proc, chown, audit_log):
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


# ------------------------------------------------------------------------------------------------ command line
def cmd_install(a):
    print(json.dumps(install(a.user, a.root)))
    return 0


def cmd_verify(a):
    problems = verify(a.user, a.root)
    print("KFROOT_VERIFY " + ("ok" if not problems else "problems=" + "; ".join(problems)))
    return 1 if problems else 0


def cmd_bootstrap(a):
    results = bootstrap(a.user, a.workspace, a.chown, a.dry_run)
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
