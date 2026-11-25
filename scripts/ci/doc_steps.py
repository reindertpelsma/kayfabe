#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Run the commands the manual tells users to run (stdlib only; works under python3 -I).

STATUS: LIVE, 2026-11-15 (OWNER_RULINGS section AS: the exact documented commands are tested on the CI GPU boxes).
There is ONE copy of every command: the fenced block in docs/howto/*.md. This module reads those blocks, in
document order, and runs them. Nothing here restates a command.

MARKER CONVENTION. A fenced block is runnable only when the line directly above its opening fence (column 0, no
blank line between) is exactly one HTML comment of one of these two shapes:

    <!-- kf-run: STEP [user=root|unpriv] [timeout=SECONDS] -->    a shell block, run by bash with `set -euo pipefail`
    <!-- kf-args: STEP -->                                         a QEMU argument block (words, not a script)

STEP is [a-z][a-z0-9_]{0,39} and is unique within its file. `user` defaults to `unpriv`; `root` is an explicit
root step (packages, device permissions) and the run records it. `timeout` is whole seconds, 1 to 14400 (default 3600).
Anything else on a line that starts (after spaces) with `<!-- kf-` outside a fenced block (an unknown kind, an unknown key, a duplicate, a marker
whose next line is not a fence, a marker above an indented or `~~~` fence) is REFUSED, so a typo cannot silently turn a
tested command into an untested one. A marker that is shown as an example inside a fenced block is text, not a marker.
Fences with the `[untested]` meaning are simply left unmarked.

BLOCK SEMANTICS. Each block runs in a fresh bash. Only two things carry from one block to the next: the working
directory (it must stay inside the run prefix) and PATH (so `. "$HOME/.cargo/env"` in one block serves the next).
The environment is rebuilt from nothing for every block (HOME, TMPDIR and the work directory are under the prefix);
the caller's variables are not inherited. A block that does not reach its own last line (an `exec`, an unterminated
quote that swallows the epilogue) fails: the runner checks a done-marker, never just the exit status.

DOCUMENTED SUBSTITUTION. Where the manual names a thing the test cannot use as written (the public clone URL, a
placeholder like `<a fixed uuid for this VM>`), the caller passes `Substitution(step, literal, replacement)`. The literal
must occur in the block EXACTLY ONCE, or the run is refused; the replacement must be one line with no NUL; a
replacement that names a path must name one inside the prefix. The substitutions in force are recorded in the result
and are described in the docs' STATUS notes (docs/howto/building.md).
"""

import dataclasses
import hashlib
import os
from pathlib import Path
import re
import subprocess
import time

MAX_DOC_BYTES = 1 << 20
DEFAULT_TIMEOUT = 3600
MAX_TIMEOUT = 14400
STEP_RE = re.compile(r"[a-z][a-z0-9_]{0,39}")
MARKER_RE = re.compile(r"^<!-- (kf-run|kf-args): ([a-z][a-z0-9_]{0,39})((?: [a-z]+=[A-Za-z0-9]+)*) -->$")
FENCE_RE = re.compile(r"^( {0,3})(`{3,}|~{3,})(.*)$")
USERS = ("root", "unpriv")
KEYS = {"kf-run": ("user", "timeout"), "kf-args": ()}
DEFAULT_PATH = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"


class DocStepError(Exception):
    """A refusal. The message names the file, the line and the rule, never block content."""


@dataclasses.dataclass(frozen=True)
class Step:
    doc: str
    name: str
    kind: str  # "run" or "args"
    user: str
    timeout: int
    text: str
    line: int


@dataclasses.dataclass(frozen=True)
class Substitution:
    step: str
    literal: str
    replacement: str


@dataclasses.dataclass(frozen=True)
class Parsed:
    doc: str
    sha256: str
    steps: tuple
    fences: int  # fenced blocks in the file, marked or not


def _refuse(doc, line, why):
    raise DocStepError(f"{doc}:{line}: {why}")


def parse(raw, doc):
    """Bytes of one Markdown file to a `Parsed`. CRLF is read as LF; a bare CR or a NUL is refused."""
    if len(raw) > MAX_DOC_BYTES:
        raise DocStepError(f"{doc}: larger than {MAX_DOC_BYTES} bytes")
    try:
        text = raw.decode("utf-8")
    except UnicodeError:
        raise DocStepError(f"{doc}: not UTF-8") from None
    if "\0" in text:
        raise DocStepError(f"{doc}: NUL byte")
    text = text.replace("\r\n", "\n")
    if "\r" in text:
        raise DocStepError(f"{doc}: bare CR")
    lines = text.split("\n")
    steps, seen, fences = [], set(), 0
    fence = None  # (char, length) while inside a fenced block
    marker = None  # (line number, kind, name, options) waiting for its fence
    body, body_start = [], 0
    for number, line in enumerate(lines, 1):
        if fence is not None:
            closing = FENCE_RE.match(line)
            if closing and closing.group(2)[0] == fence[0] and len(closing.group(2)) >= fence[1] and not closing.group(3).strip():
                if marker is not None:
                    steps.append(_step(doc, marker, "\n".join(body) + "\n", seen))
                    marker = None
                fence = None
            elif marker is not None:
                body.append(line)
            continue
        opening = FENCE_RE.match(line)
        if marker is not None:
            # the marker's very next line must be a column-0 backtick fence
            if not opening or opening.group(1) or opening.group(2)[0] != "`" or "`" in opening.group(3):
                _refuse(doc, marker[0], "marker is not directly above a column-0 ``` fence")
            fence, body, body_start = (opening.group(2)[0], len(opening.group(2))), [], number
            fences += 1
            continue
        if opening and "`" not in opening.group(3):
            fence = (opening.group(2)[0], len(opening.group(2)))
            fences += 1
            continue
        if re.match(r"\s*<!--\s*kf-", line):
            found = MARKER_RE.match(line)
            if not found:
                _refuse(doc, number, "malformed kf- marker")
            marker = (number, found.group(1), found.group(2), found.group(3).split())
    if marker is not None:
        _refuse(doc, marker[0], "marker has no fenced block")
    if fence is not None:
        _refuse(doc, body_start or len(lines), "unterminated fence")
    return Parsed(doc, hashlib.sha256(raw).hexdigest(), tuple(steps), fences)


def _step(doc, marker, text, seen):
    number, kind, name, options = marker
    if name in seen:
        _refuse(doc, number, "duplicate step name")
    seen.add(name)
    values = {}
    for option in options:
        key, _, value = option.partition("=")
        if key not in KEYS[kind] or key in values:
            _refuse(doc, number, "unknown or repeated marker key")
        values[key] = value
    user = values.get("user", "unpriv")
    if user not in USERS:
        _refuse(doc, number, "user must be root or unpriv")
    timeout = values.get("timeout", str(DEFAULT_TIMEOUT))
    if not re.fullmatch(r"[1-9][0-9]{0,4}", timeout) or int(timeout) > MAX_TIMEOUT:
        _refuse(doc, number, "timeout out of range")
    if not text.strip():
        _refuse(doc, number, "empty block")
    if kind == "kf-run" and user == "unpriv" and re.search(r"(^|[\s;&|(])sudo(\s|$)", text, re.M):
        _refuse(doc, number, "an unprivileged step may not call sudo; mark it user=root")
    return Step(doc, name, kind[3:], user, int(timeout), text, number)


def load(path, name=None):
    """Read once, bounded; the hash is of the exact bytes parsed."""
    try:
        with open(path, "rb") as source:
            raw = source.read(MAX_DOC_BYTES + 1)
    except OSError:
        raise DocStepError(f"{name or path}: unreadable") from None
    return parse(raw, name or str(path))


def inside(prefix, path):
    """True only if `path` resolves (symlinks followed) to the prefix or below it."""
    base = os.path.realpath(prefix)
    real = os.path.realpath(path)
    return real == base or real.startswith(base.rstrip("/") + "/")


def substitute(step, subs, prefix):
    """Apply the substitutions that name this step; each literal must occur exactly once."""
    text = step.text
    used = []
    for sub in subs:
        if sub.step != step.name:
            continue
        if not sub.literal or text.count(sub.literal) != 1:
            raise DocStepError(f"{step.doc}:{step.line}: substitution literal must occur exactly once in step {step.name}")
        if "\n" in sub.replacement or "\0" in sub.replacement or not sub.replacement:
            raise DocStepError(f"{step.doc}:{step.line}: substitution value is not one clean line")
        for path in re.findall(r"(?:file://)?(/[^\s'\"]*)", sub.replacement):
            if not inside(prefix, path):
                raise DocStepError(f"{step.doc}:{step.line}: substitution names a path outside the prefix")
        text = text.replace(sub.literal, sub.replacement)
        used.append(sub.literal)
    return dataclasses.replace(step, text=text), used


def args_words(step):
    """A kf-args block as a list of words (shell-style quoting, `#` comments)."""
    import shlex

    return shlex.split(step.text, comments=True)


# ---------------------------------------------------------------- running

def wrapper(step, state):
    """The script bash runs: the block between a prologue and an epilogue. `state` is the per-run state directory."""
    prologue = [
        "set -euo pipefail",
        f'cd "$(cat "{state}/cwd")"',
        f'PATH="$(cat "{state}/path")"; export PATH',
    ]
    if step.user == "root":
        # the runner is root: `sudo` in a documented command is a no-op here, and the step is recorded as root
        prologue.append('sudo() { "$@"; }')
    epilogue = [
        f"pwd -P > {state}/cwd.new",
        f'printf %s "$PATH" > {state}/path.new',
        f"id -u > {state}/uid.new",
        f"grep -m1 '^CapEff:' /proc/self/status > {state}/cap.new || true",
        f"echo done > {state}/done",
    ]
    return "\n".join(prologue) + "\n" + step.text.rstrip("\n") + "\n" + "\n".join(epilogue) + "\n"


class Runner:
    """Runs steps in order under one prefix. `drop` is the argv prefix that becomes the unprivileged user
    (for example setpriv ...); None means run as the current uid (tests)."""

    def __init__(self, prefix, drop=None, base_env=None, log=print, require_unpriv=False, clock=time.monotonic):
        if not re.fullmatch(r"/[A-Za-z0-9_./-]+", str(Path(prefix).resolve())):
            raise DocStepError("prefix: unsafe characters")
        self.prefix = Path(prefix).resolve()
        self.drop = list(drop) if drop else None
        self.require_unpriv = require_unpriv
        self.log = log
        self.clock = clock
        self.records = []
        self.home = self.prefix / "home"
        self.work = self.prefix / "work"
        self.state = self.prefix / "state"
        self.logs = self.prefix / "logs"
        self.tmp = self.prefix / "tmp"
        for d in (self.home, self.work, self.state, self.logs, self.tmp):
            d.mkdir(parents=True, exist_ok=True)
        (self.state / "cwd").write_text(str(self.work))
        (self.state / "path").write_text((base_env or {}).get("PATH", DEFAULT_PATH))
        self.base_env = {k: v for k, v in (base_env or {}).items() if k in ("LANG", "TERM")}
        self.counter = 0

    def environment(self):
        env = {
            "HOME": str(self.home),
            "TMPDIR": str(self.tmp),
            "USER": "kfinstall",
            "LOGNAME": "kfinstall",
            "LANG": "C.UTF-8",
            "TERM": "dumb",
            "PATH": DEFAULT_PATH,
            "DEBIAN_FRONTEND": "noninteractive",
        }
        env.update(self.base_env)
        return env

    def run(self, step):
        """Run one shell step. Returns the record; raises DocStepError when the step must stop the run."""
        if step.kind != "run":
            raise DocStepError(f"{step.doc}:{step.line}: {step.name} is not a kf-run block")
        self.counter += 1
        tag = f"{self.counter:02d}-{step.name}"
        for name in ("done", "cwd.new", "path.new", "uid.new", "cap.new"):
            (self.state / name).unlink(missing_ok=True)
        script = self.logs / (tag + ".sh")
        script.write_text(wrapper(step, self.state))
        logfile = self.logs / (tag + ".log")
        argv = ["/bin/bash", "--noprofile", "--norc", str(script)]
        if step.user == "unpriv" and self.drop:
            argv = [*self.drop, *argv]
        self.log(f"KF_DOCSTEP_START doc={step.doc} step={step.name} user={step.user} timeout={step.timeout}")
        start = self.clock()
        try:
            with open(logfile, "wb") as out:
                rc = subprocess.run(
                    argv, env=self.environment(), stdin=subprocess.DEVNULL, stdout=out, stderr=subprocess.STDOUT,
                    timeout=step.timeout, check=False,
                ).returncode
        except subprocess.TimeoutExpired:
            rc = 124
        ms = int((self.clock() - start) * 1000)
        record = {"doc": step.doc, "step": step.name, "user": step.user, "rc": rc, "ms": ms}
        reached = (self.state / "done").exists()
        if rc == 0 and not reached:
            record.update(rc=125, reason="block did not reach its end")
        if record["rc"] == 0:
            self._carry(step, record)
        self.records.append(record)
        self.log(f"KF_DOCSTEP_END doc={step.doc} step={step.name} rc={record['rc']} ms={ms}" + (
            f" reason={record['reason']}" if "reason" in record else ""))
        if record["rc"]:
            raise DocStepError(f"{step.doc}: step {step.name} failed rc={record['rc']}")
        return record

    def _carry(self, step, record):
        cwd = (self.state / "cwd.new").read_text().strip()
        uid = (self.state / "uid.new").read_text().strip()
        cap = (self.state / "cap.new").read_text().split() if (self.state / "cap.new").exists() else []
        record["uid"] = int(uid) if uid.isdigit() else -1
        record["cap_eff"] = cap[-1] if cap else "unknown"
        if not inside(self.prefix, cwd):
            record.update(rc=126, reason="working directory left the prefix")
            return
        if step.user == "unpriv" and self.require_unpriv and (record["uid"] in (0, -1) or set(record["cap_eff"]) - {"0"}):
            record.update(rc=127, reason="unprivileged step held root or capabilities")
            return
        (self.state / "cwd").write_text(cwd)
        (self.state / "path").write_text((self.state / "path.new").read_text())

    def root_steps(self):
        return [r["step"] for r in self.records if r["user"] == "root"]
