#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Thin adapter for the existing disposable-box provision and merge bar.
No provider/CI credentials or network clone machinery belongs here.
"""

import collections, json, os, re, select, shutil, signal, subprocess, sys, time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import vast_report as local_result_contract

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "bench"))
import native_xid_guard as xid_guard

STOP = False


class EvidenceError(Exception):
    def __init__(self, name, rows=()):
        self.name = name
        self.rows = list(rows)
        super().__init__(name)


def row(name, passed=0, failed=0, skipped=0):
    return {"name": name, "pass": passed, "fail": failed, "skip": skipped}


def only(pattern, text, name, rows=()):
    found = re.findall(pattern, text, re.M)
    if len(found) != 1:
        raise EvidenceError(name, rows)
    return found[0]


def provision_rows(text, rc):
    if (
        rc
        or red_line(text)
        or any(
            len(re.findall("^" + key + "_RC=0$", text, re.M)) != 1
            for key in ["BOX", "DRIVER", "TREE", "LADDER", "FG", "KF3"]
        )
        or re.search(r"^NOT_READY\b", text, re.M)
    ):
        raise EvidenceError("provision")
    only(r"^READY driver=(\d{3}\.\d{1,3}(?:\.\d{1,2})?)$", text, "provision")
    only(r"^EXIT [^\n]+$", text, "provision")
    return [row("provision", 1)]


def merge_rows(text, rc, sha):
    rows = []
    if only(r"^HEAD=([0-9a-f]{40}) CHECKOUT=[^\n]+$", text, "infrastructure") != sha:
        raise EvidenceError("infrastructure")
    # [2026-10-26] The bar no longer runs the GPU-free crate tests (CI's `stable` job does, on every
    # commit). The legacy totals line stays red; the opt-in with-KVM stage (kvm_tests.sh) is graded
    # here, and a request without exactly one passing result is a failure, never a silent skip.
    if re.search(r"^TESTS passed [0-9]+ failed [1-9]", text, re.M):
        raise EvidenceError("kf_tests")
    requested = len(re.findall(r"^KVM_TESTS_REQUESTED=1$", text, re.M))
    results = re.findall(
        r"^KVM_TESTS passed ([0-9]+) failed ([0-9]+) kvm_ran=([0-9]+) kvm_skipped=([0-9]+)$",
        text,
        re.M,
    )
    if re.search(r"^KVM_TESTS_RC=[1-9]", text, re.M):
        raise EvidenceError("kvm_tests", rows)
    if requested or results:
        if requested != 1 or len(results) != 1:
            raise EvidenceError("kvm_tests", rows)
        passed, failed, ran, skipped = map(int, results[0])
        if passed < 1 or failed or ran < 1 or skipped:
            raise EvidenceError("kvm_tests", rows)
        rows.append(row("kvm_tests", passed))
    only(r"^GATES_RC=0$", text, "gates", rows)
    only(r"^V3_GATES_SUMMARY ([^\n]+)$", text, "gates", rows)
    only(r"^V3_GATES_SUMMARY pass=9 fail=0 gate10=(PASS|FALLBACK)$", text, "gates", rows)
    rows.append(row("gates", 9))
    only(r"^KF3_RC=0$", text, "kf3_build", rows)
    rows.append(row("kf3_build", 1))
    only(
        r"^BARE_SUITE_PASS=30 BARE_SUITE_FAIL=0 BARE_SUITE_CRASH=0 ARMS=30$",
        text,
        "raw_suite",
        rows,
    )
    rows.append(row("raw_suite", 30))
    only(r"^FG_RC=0$", text, "thin_suite", rows)
    only(r"^SUITE_RC=0$", text, "thin_suite", rows)
    only(
        r"^FAST_SUITE_PASS=30 FAST_SUITE_FAIL=0 FAST_SUITE_CRASH=0 NOTRUN=0 ARMS=30$",
        text,
        "thin_suite",
        rows,
    )
    rows.append(row("thin_suite", 30))
    only(r"^BIRTH_CENSUS_OK arms=30(?: .*)?$", text, "birth_census", rows)
    rows.append(row("birth_census", 1))
    if rc or red_line(text) or len(re.findall(r"^EXIT rc=0 .+$", text, re.M)) != 1:
        raise EvidenceError("merge_bar", rows)
    return rows


def ladder_rows(text, rc, mode, sha):
    name = "cuda_host" if mode == "host" else "cuda_fat"
    if mode not in {"host", "guest"} or rc:
        raise EvidenceError(name)
    only(
        r"^CUDA_LADDER_STARTED=.+ mode="
        + mode
        + r" reps=1 rungs=cup2,cup3,cup8,cup8bench rev="
        + re.escape(sha[:8])
        + r"$",
        text,
        name,
    )
    only(r"^CUDA_LADDER_DONE=.+$", text, name)
    lines = [s for s in text.splitlines() if s.startswith("CL_ROW ")]
    seen = set()
    if len(lines) != 4:
        raise EvidenceError(name)
    for line in lines:
        match = re.match(
            r"^CL_ROW mode="
            + mode
            + r" rung=(cup2|cup3|cup8|cup8bench) rep=1 verdict=PASS "
            + ("rc=0" if mode == "host" else "boot_rc=0")
            + r"(?: |$)",
            line,
        )
        if not match or match[1] in seen:
            raise EvidenceError(name)
        seen.add(match[1])
    return [row(name, 4)]


def red_line(line):
    return bool(
        re.search(
            r"^(?:BOX|DRIVER|TREE|LADDER|FG|KF3|GATES|SUITE|CARGO|KVM_TESTS)_RC=[1-9][0-9]*\b",
            line,
            re.M,
        )
        or re.search(r"^(?:BARE|FAST)_CELL_ARM\b.* verdict=(?!PASS\b)[A-Z_]+\b", line, re.M)
        or re.search(r"^V3_GATES_SUMMARY\b.* fail=[1-9]", line, re.M)
        or re.search(r"^TESTS passed [0-9]+ failed [1-9]", line, re.M)
        or re.search(r"^KVM_TESTS passed [0-9]+ failed [1-9]", line, re.M)
        or re.search(r"^KVM_TESTS passed .* kvm_skipped=[1-9]", line, re.M)
        or re.search(r"^CL_ROW .* verdict=(?!PASS\b)[A-Z_]+\b", line, re.M)
    )


FAIL_LINE = re.compile(
    r"^(?:error(?:\[E\d+\])?:|NOT_READY\b|KF_[A-Z_]*(?:REFUSED|FAIL)\b|"
    r"[^\n]*(?:\bFATAL\b|\bpanicked at\b|BENCH_TREE_FAILED|\bFAILED\b|NEW HOST Xid))"
)
ANSI = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]|\x1b[ -/]*[@-~]")
SUMMARY_BEGIN = "KF_FINAL_SUMMARY_BEGIN"
SUMMARY_END = "KF_FINAL_SUMMARY_END"
SUMMARY_LIMIT = 4096
LINE_LIMIT = 240
# Records of the stages of the current run, in order; the FINAL SUMMARY is built from them.
STAGE_RECORDS = []
PROV_DIR = "/root/prov"
# provision_full.sh writes one log per step; its RC line names the step whose log has the cause.
PROVISION_STEP_LOGS = {
    "BOX": "box.log",
    "DRIVER": "driver.log",
    "TREE": "tree.log",
    "FG": "fg.log",
    "KF3": "kf3.log",
}


def printable(line, limit=LINE_LIMIT):
    line = ANSI.sub("", line)
    line = "".join(c if " " <= c <= "~" else " " if c == "\t" else "" for c in line)
    line = line.strip()
    return line if len(line) <= limit else line[: limit - 3] + "..."


def read_lines(path, limit=2 * 1024 * 1024):
    try:
        with open(path, "rb") as source:
            data = source.read(limit)
    except OSError:
        return []
    return [ANSI.sub("", s) for s in data.decode("utf-8", errors="replace").splitlines()]


def is_failing(line):
    return bool(red_line(line) or FAIL_LINE.search(line))


def first_failing_line(paths):
    """First red line of the stage's logs, in the order given; else the last line printed."""
    last = ""
    for path in paths:
        lines = read_lines(path)
        for line in lines:
            if is_failing(line):
                return printable(line)
        for line in reversed(lines):
            if line.strip() and not last:
                last = line
    return "no red line found; last line: " + printable(last) if last else "no output found"


# Test names. libtest's terse mode prints `<name> --- FAILED`, its verbose mode `test <name> ... FAILED`,
# and both end with a `failures:` list of indented names. Measured 2026-10-26 (gate run 40263112941):
# the Worker/runner scrubber blanks any run of 32+ [A-Za-z0-9_-] characters, so a 60-character test
# name reached the log as `[redacted] --- FAILED`.
TEST_NAME = r"[A-Za-z_][A-Za-z0-9_]*(?:::[A-Za-z_][A-Za-z0-9_]*)*"
FAILED_TEST = re.compile(r"^(?:test )?(" + TEST_NAME + r")(?: \.\.\.| ---) FAILED$")
FAILURE_ITEM = re.compile(r"^    (" + TEST_NAME + r")$")
PANIC = re.compile(r"\bpanicked at\b")
FAILED_TEST_LIMIT = 20
# A snake_case test name: lowercase words, at least three, each at most 20 characters, at most 128
# characters in all and at most one digit in five. Hex, base64url, mixed-case, hyphenated and
# digit-heavy strings are not. ci/vast-broker (sanitize.js, runner.py scrub) uses the same grammar to
# decide which 32+ character runs a scrubber may keep; scrub-vectors.json pins both sides.
SNAKE_NAME = re.compile(r"[a-z][a-z0-9]{0,19}(?:_[a-z0-9]{1,20}){2,}")
LONG_RUN = re.compile(r"[A-Za-z0-9_-]{32,}")
FOLD = 30


def is_test_identifier(run):
    return (
        len(run) <= 128
        and SNAKE_NAME.fullmatch(run) is not None
        and sum(c.isdigit() for c in run) * 5 <= len(run)
    )


def fold_identifiers(text):
    """Split test-name-shaped 32+ character runs into pieces of at most 30 joined by "~".

    A scrubber that has not been redeployed still blanks the long form; the pieces stay readable.
    Anything that is not test-name shaped is left whole, so it still reaches the scrubber whole.
    """
    return LONG_RUN.sub(
        lambda m: (
            "~".join(m[0][i : i + FOLD] for i in range(0, len(m[0]), FOLD))
            if is_test_identifier(m[0])
            else m[0]
        ),
        text,
    )


def failed_tests(paths, limit=FAILED_TEST_LIMIT):
    """(names, total): the failed test names in the logs, in order, de-duplicated, at most `limit`."""
    seen = []
    for path in paths:
        state = None  # "head" after `failures:`, "list" while its indented names continue
        for line in read_lines(path):
            if line == "failures:":
                state = "head"
                continue
            item = FAILURE_ITEM.match(line) if state else None
            state = "list" if item else None
            found = FAILED_TEST.match(line) or item
            if found and found[1] not in seen:
                seen.append(found[1])
    return seen[:limit], len(seen)


def first_panic(paths):
    """The first panic line of the logs and the message line after it, bounded; "" if none."""
    for path in paths:
        lines = read_lines(path)
        for i, line in enumerate(lines):
            if PANIC.search(line):
                following = lines[i + 1].strip() if i + 1 < len(lines) else ""
                return printable(line.strip() + (" | " + following if following else ""))
    return ""


def step_cause(root, first):
    """The last lines of the provisioning step log named by a failing RC line."""
    match = re.match(r"(BOX|DRIVER|TREE|FG|KF3)_RC=", first)
    if not match:
        return ""
    lines = [s for s in read_lines(Path(root) / PROVISION_STEP_LOGS[match[1]]) if s.strip()]
    return " | ".join(printable(s, 100) for s in lines[-3:])


def summary_lines(result, records):
    stage_rows, fail_rows, suite_rows = [], [], []
    for record in records:
        if "rc" not in record:
            stage_rows.append("stage name=" + record["name"] + " result=NOT_RUN")
            continue
        text = (
            "stage name="
            + record["name"]
            + " rc="
            + str(record["rc"])
            + " duration_ms="
            + str(record["duration_ms"])
            + " result="
            + record["result"]
        )
        if record.get("reason"):
            text += " reason=" + record["reason"]
        stage_rows.append(text)
        if record.get("first_fail"):
            fail_rows.append(
                "first_fail stage="
                + record["name"]
                + " line="
                + fold_identifiers(record["first_fail"])
            )
        if record.get("panic"):
            fail_rows.append(
                "panic stage=" + record["name"] + " line=" + fold_identifiers(record["panic"])
            )
        for name in record.get("failed_tests", []):
            fail_rows.append(
                "failed_test stage=" + record["name"] + " name=" + fold_identifiers(printable(name))
            )
        more = record.get("failed_total", 0) - len(record.get("failed_tests", []))
        if more > 0:
            fail_rows.append("failed_tests_more stage=%s count=%d" % (record["name"], more))
        if record.get("cause"):
            fail_rows.append("cause stage=" + record["name"] + " log_tail=" + record["cause"])
    for suite in result["suites"]:
        text = "suite name=%s pass=%d fail=%d skip=%d" % (
            suite["name"],
            suite["pass"],
            suite["fail"],
            suite["skip"],
        )
        suite_rows.append((suite["fail"] > 0, text))
    return stage_rows, suite_rows, fail_rows


def final_summary(result, records):
    """One bounded block; it is printed last so a tail-limited log always shows it."""
    begin = "%s verdict=%s stages=%d" % (SUMMARY_BEGIN, result["verdict"], len(records))
    stage_rows, suite_rows, fail_rows = summary_lines(result, records)
    budget = SUMMARY_LIMIT - len(begin) - len(SUMMARY_END) - 2 - 64
    keep = {}
    failed = {i for i, record in enumerate(records) if record.get("result") in ("FAIL", "TIMEOUT")}
    priority = (
        [("stage", i, s) for i, s in enumerate(stage_rows) if i in failed]
        + [("fail", i, s) for i, s in enumerate(fail_rows)]
        + [("suite", i, s) for i, (bad, s) in enumerate(suite_rows) if bad]
        + [("stage", i, s) for i, s in enumerate(stage_rows) if i not in failed]
        + [("suite", i, s) for i, (bad, s) in enumerate(suite_rows) if not bad]
    )
    dropped = 0
    for kind, index, text in priority:
        text = printable(text, LINE_LIMIT + 40)
        if len(text) + 1 > budget:
            dropped += 1
            continue
        budget -= len(text) + 1
        keep[(kind, index)] = text
    lines = [begin]
    for kind in ("stage", "suite", "fail"):
        lines += [keep[k] for k in sorted(keep) if k[0] == kind]
    if dropped:
        lines.append("truncated lines=%d" % dropped)
    lines.append(SUMMARY_END)
    return "\n".join(lines)


def print_final_summary(result):
    print(final_summary(result, STAGE_RECORDS), flush=True)


def watch_delta(text, rc):
    """Validate the existing watcher's instrument, process status and exact delta."""
    if int(only(r"^\[xid-watch\] host_dmesg_lines=([0-9]+) .*$", text, "host_xid")) < 2:
        raise EvidenceError("host_xid")
    before = int(only(r"^\[xid-watch\] xid_before=([0-9]+)$", text, "host_xid"))
    after = int(only(r"^\[xid-watch\] xid_after=([0-9]+)$", text, "host_xid"))
    only(r"^\[xid-watch\] --- command output begins ---$", text, "host_xid")
    command_rc = int(
        only(r"^\[xid-watch\] --- command output ends \(rc=([0-9]+)\) ---$", text, "host_xid")
    )
    if command_rc:
        raise EvidenceError("host_xid")
    if rc == 0 and before == after and "HOST Xid CLEAN" in text:
        return []
    if rc != 6:
        raise EvidenceError("host_xid")
    suffix = text.split("[xid-watch] --- command output ends ", 1)[1]
    if suffix.count("NEW HOST Xid ACROSS THE COMMAND") != 1:
        raise EvidenceError("host_xid")
    delta = [
        s[4:]
        for s in suffix.split("NEW HOST Xid ACROSS THE COMMAND", 1)[1].splitlines()
        if s.startswith("    ")
    ]
    if not delta or after - before != len(delta):
        raise EvidenceError("host_xid")
    return delta


def xid_ok(text, rc, arm, arm_output=""):
    """Admission delegates to the single S4 exact-source implementation."""
    try:
        delta = watch_delta(text, rc)
        if arm not in ("--defer-liveness", "--missing-page-fault"):
            return not delta
        journal = "\n".join(delta)
        if len(xid_guard.xid_lines(journal)) != len(delta):
            return False
        xid_guard.approve_native_arm(journal, arm, arm_output)
        return True
    except (EvidenceError, xid_guard.GuardRefused, ValueError, TypeError):
        return False


def _stop(child):
    # Every child here was started in its own session by this function's caller.
    if child.poll() is not None:
        return
    try:
        os.killpg(child.pid, signal.SIGTERM)
    except ProcessLookupError:
        return
    try:
        child.wait(timeout=5)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(child.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        child.wait()


def run_bounded(command, soft_deadline_ms, output, progress_files):
    if STOP or int(time.time() * 1000) >= soft_deadline_ms:
        return 124
    output = Path(output)
    output.parent.mkdir(parents=True, exist_ok=True)
    positions = {}
    pending = {}
    failure = False
    child = None

    def forward(path):
        nonlocal failure
        try:
            with open(path, "rb") as log:
                log.seek(positions.get(str(path), 0))
                data = log.read(65536)
                positions[str(path)] = log.tell()
        except FileNotFoundError:
            return
        if not data:
            return
        decoded = data.decode("utf-8", errors="replace")
        print(decoded, end="", flush=True)
        value = pending.get(str(path), "") + decoded
        lines = value.split("\n")
        pending[str(path)] = lines.pop()[-65536:]
        if any(red_line(line) for line in lines):
            failure = True

    try:
        with output.open("wb") as log:
            child = subprocess.Popen(
                command, stdout=log, stderr=subprocess.STDOUT, start_new_session=True
            )
            while True:
                for path in [output, *progress_files]:
                    forward(path)
                if STOP or int(time.time() * 1000) >= soft_deadline_ms:
                    _stop(child)
                    return 124
                if failure or (
                    os.environ.get("KF_PAYLOAD_XID_FAILED")
                    and Path(os.environ["KF_PAYLOAD_XID_FAILED"]).exists()
                ):
                    _stop(child)
                    return 1
                rc = child.poll()
                if rc is not None:
                    for path in [output, *progress_files]:
                        forward(path)
                    return rc if rc else int(failure)
                time.sleep(0.05)
    finally:
        if child is not None:
            _stop(child)


def bounded_text(path, limit=4 * 1024 * 1024):
    with open(path, "rb") as source:
        data = source.read(limit + 1)
    if len(data) > limit:
        raise EvidenceError("infrastructure")
    return data.decode("utf-8", errors="strict")


def prepare_checkout(root, sha):
    def git(*args):
        return subprocess.check_output(
            ["git", "-C", str(root), *args], text=True, stderr=subprocess.DEVNULL
        ).strip()

    if git("rev-parse", "HEAD") != sha:
        raise EvidenceError("infrastructure")
    # Existing helpers fetch a branch. Point those reads at this already approved
    # local checkout, so their unmodified flows cannot select different remote code.
    branch = "kf-ci-approved"
    git("remote", "set-url", "origin", str(root))
    git("update-ref", "refs/heads/" + branch, sha)
    git("update-ref", "refs/remotes/origin/" + branch, sha)
    return branch


def hardware():
    try:
        lines = (
            subprocess.check_output(
                [
                    "nvidia-smi",
                    "--query-gpu=name,compute_cap,driver_version",
                    "--format=csv,noheader",
                ],
                text=True,
            )
            .strip()
            .splitlines()
        )
        if len(lines) != 1:
            raise ValueError()
        name, cc, driver = [x.strip() for x in lines[0].split(",")]
        if not re.fullmatch(r"[A-Za-z0-9_. -]{1,100}", name) or not re.fullmatch(
            r"\d{3}\.\d{1,3}(?:\.\d{1,2})?", driver
        ):
            raise ValueError()
        cap = int(round(float(cc) * 100))
        return {"gpu_name": name, "compute_cap": cap, "driver_version": driver}
    except (subprocess.SubprocessError, ValueError, OSError):
        return {"gpu_name": "unknown", "compute_cap": 0, "driver_version": "unknown"}


def persist_result(destination, result):
    destination = Path(destination)
    temporary = destination.with_suffix(".tmp")
    snapshot = dict(result)
    state_path = os.environ.get("KF_PAYLOAD_DIAGNOSTIC_STATE")
    if state_path and Path(state_path).is_file():
        events = json.loads(bounded_text(state_path, 65536))["events"]
        local_result_contract.validate_diagnostics(events)
        if events:
            snapshot["diagnostics"] = events
    temporary.write_text(json.dumps(snapshot, separators=(",", ":")) + "\n")
    temporary.replace(destination)


def execute_stages(stages, soft, result, destination, runner=run_bounded):
    """Persist the finished prefix even when a later suite times out or fails."""
    records = STAGE_RECORDS
    result["host_reference"] = "MISSING"
    records[:] = [dict(name=stage["name"]) for stage in stages]
    persist_result(destination, result)
    for stage, record in zip(stages, records):
        name = stage["name"]
        if name == "cuda_host":
            result["host_reference"] = "NOT_GREEN"
            persist_result(destination, result)
        print("KF_PAYLOAD_STAGE_START name=" + name, flush=True)
        start = time.monotonic()
        rc = runner(stage["command"], soft, stage["output"], stage["progress"])
        elapsed = int((time.monotonic() - start) * 1000)
        record.update(rc=rc, duration_ms=elapsed, result="PASS")
        print(
            "KF_PAYLOAD_STAGE_END name=" + name + " rc=" + str(rc) + " duration_ms=" + str(elapsed),
            flush=True,
        )
        try:
            rows = stage["grade"](rc)
            if rc == 124:
                raise EvidenceError("timeout", rows)
            if name == "cuda_host":
                result["host_reference"] = "GREEN"
            result["suites"].extend(rows)
            persist_result(destination, result)
        except EvidenceError as error:
            result["suites"].extend(error.rows)
            failure = "timeout" if rc == 124 else error.name
            result["suites"].append(row(failure, failed=1))
            append_arm_failure(result)
            result["verdict"] = "FAIL"
            persist_result(destination, result)
            record.update(result="TIMEOUT" if rc == 124 else "FAIL", reason=failure)
            files = [*stage["progress"], stage["output"], *stage.get("detail", [])]
            record["first_fail"] = first_failing_line(files)
            record["failed_tests"], record["failed_total"] = failed_tests(files)
            record["panic"] = first_panic(files)
            if name == "provision":
                record["cause"] = step_cause(PROV_DIR, record["first_fail"])
            print("KF_PAYLOAD_FAIL suite=" + failure, flush=True)
            print_final_summary(result)
            return 1
    result["verdict"] = "PASS"
    persist_result(destination, result)
    print_final_summary(result)
    return 0


def arm_diagnostic(arm, text, rc, admitted, mode="native"):
    """Send a bounded head/tail through the existing reporter, even on refusal."""
    destination = os.environ.get("KF_PAYLOAD_LOG")
    state_path = os.environ.get("KF_PAYLOAD_DIAGNOSTIC_STATE")
    if not destination or not state_path:
        return
    if not re.fullmatch(r"--[a-z0-9-]{1,40}|guest", arm) or mode not in ("native", "guest"):
        raise EvidenceError("diagnostics")
    path = Path(state_path)
    state = json.loads(bounded_text(path, 65536)) if path.exists() else {"bytes": 0, "events": []}
    if (
        set(state) != {"bytes", "events"}
        or type(state["bytes"]) is not int
        or not 0 <= state["bytes"] <= 1048576
        or not isinstance(state["events"], list)
        or len(state["events"]) >= 128
    ):
        raise EvidenceError("diagnostics")
    raw = text.encode("utf-8")
    if len(raw) > 32768:
        raw = raw[:16384] + b"\n[arm diagnostic truncated; head and tail retained]\n" + raw[-16384:]
    content = raw.decode("utf-8", errors="replace")
    frame = (
        "KF_ARM_DIAG_BEGIN arm="
        + arm
        + " wrapper_rc="
        + str(rc)
        + " admitted="
        + str(int(admitted))
        + "\n"
        + content
        + "\nKF_ARM_DIAG_END arm="
        + arm
        + "\n"
    ).encode("utf-8")
    limit = 1048576 - (65536 if admitted else 0)
    remaining = max(0, limit - state["bytes"])
    frame = frame[:remaining].decode("utf-8", errors="ignore").encode("utf-8")
    descriptor = os.open(destination, os.O_WRONLY | os.O_APPEND | os.O_CREAT | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, "ab") as output:
        output.write(frame)
    state["bytes"] += len(frame)
    item = local_result_contract.diagnostic_record(arm, mode, rc, bool(admitted), text)
    state["events"] = local_result_contract.compact_diagnostics([*state["events"], item])
    temporary = path.with_suffix(".next")
    temporary.write_text(json.dumps(state) + "\n")
    temporary.replace(path)


def append_arm_failure(result):
    path = os.environ.get("KF_PAYLOAD_DIAGNOSTIC_STATE")
    if not path or not Path(path).is_file():
        return
    events = json.loads(bounded_text(path, 65536))["events"]
    if not isinstance(events, list) or len(events) > 128:
        raise EvidenceError("diagnostics")
    for event in reversed(events):
        if event.get("admitted") is False:
            arm = event.get("arm", "")
            if not re.fullmatch(r"--[a-z0-9-]{1,40}|guest", arm):
                raise EvidenceError("diagnostics")
            prefix = "guest_" if event.get("mode") == "guest" else "arm_"
            name = prefix + arm.removeprefix("--").replace("-", "_")
            if name not in {r["name"] for r in result["suites"]}:
                result["suites"].append(row(name, failed=1))
            break


def xid_record_key(line):
    if line.count("NVRM: Xid") != 1:
        raise EvidenceError("host_xid")
    return line[line.index("NVRM: Xid") :]


def retain_native_control(arm, text):
    directory = os.environ.get("KF_PAYLOAD_CONTROL_DIR")
    receipt = os.environ.get("KF_PAYLOAD_NATIVE_RECEIPT")
    if not directory or not receipt or arm not in ("--defer-liveness", "--missing-page-fault"):
        return
    directory = Path(directory)
    directory.mkdir(parents=True, exist_ok=True)
    (directory / (arm.removeprefix("--") + ".log")).write_text(text)
    defer = directory / "defer-liveness.log"
    missing = directory / "missing-page-fault.log"
    if defer.is_file() and missing.is_file():
        lines = xid_guard.approve_native(
            xid_guard.kernel_snapshot(), bounded_text(defer), bounded_text(missing)
        )
        persist_result(receipt, {"approved_native_xid_lines": list(lines)})


def guard_command(arm, log_path, rc):
    text = bounded_text(log_path)
    ok = xid_ok(text, rc, arm, text)
    if ok:
        try:
            retain_native_control(arm, text)
        except (xid_guard.GuardRefused, EvidenceError, OSError, ValueError) as error:
            text += "\nKF_NATIVE_RECEIPT_REFUSED reason=" + str(error) + "\n"
            ok = False
    arm_diagnostic(arm, text, rc, ok)
    if ok:
        delta = watch_delta(text, rc)
        if delta:
            with open(os.environ["KF_PAYLOAD_XID_ALLOWED"], "a") as approved:
                for line in delta:
                    approved.write(json.dumps(xid_record_key(line)) + "\n")
    else:
        Path(os.environ["KF_PAYLOAD_XID_FAILED"]).touch()
        print("KF_XID_REFUSED arm=" + arm, flush=True)
    return 0 if ok else 6


def guest_command(qlog, arguments):
    """Keep the caller's timeout; S4 owns and validates the actual QEMU child."""
    qlog = Path(qlog)
    arm = "guest"
    admitted = False
    serial = None
    error = ""
    rc = 6
    try:
        binary = Path(arguments[0])
        argv = arguments[1:]
        arm = xid_guard.guest_launch_arm(argv)
        serials = [
            argv[i + 1].split("path=", 1)[1]
            for i, arg in enumerate(argv[:-1])
            if arg == "-chardev" and argv[i + 1].startswith("file,id=kfcon0,path=")
        ]
        if len(serials) != 1 or not qlog.is_file():
            raise EvidenceError("host_xid")
        serial = Path(serials[0])
        root = Path(os.environ["KF_PAYLOAD_ROOT"])
        revision = subprocess.check_output(
            ["git", "-C", str(root), "rev-parse", "HEAD"], text=True
        ).strip()
        receipt = Path(os.environ["KF_PAYLOAD_NATIVE_RECEIPT"])
        before = json.loads(xid_guard.bounded(receipt))
        baseline = before.get("accepted_xid_lines", before["approved_native_xid_lines"])
        result = xid_guard.supervise_guest(binary, argv, receipt, revision, root, serial, qlog)
        if result != 0:
            raise EvidenceError("host_xid")
        state = json.loads(xid_guard.bounded(receipt))
        accepted = state.get("accepted_xid_lines", state["approved_native_xid_lines"])
        if accepted[: len(baseline)] != baseline:
            raise EvidenceError("host_xid")
        with open(os.environ["KF_PAYLOAD_XID_ALLOWED"], "a") as approved:
            for line in accepted[len(baseline) :]:
                approved.write(json.dumps(xid_record_key(line)) + "\n")
        rc = 0
        admitted = True
    except (
        EvidenceError,
        xid_guard.GuardRefused,
        OSError,
        ValueError,
        KeyError,
        subprocess.SubprocessError,
    ) as failure:
        error = "KF_GUEST_XID_REFUSED reason=" + str(failure) + "\n"
        Path(os.environ["KF_PAYLOAD_XID_FAILED"]).touch()
    finally:
        parts = [error]
        for label, path in (("SERIAL", serial), ("QEMU", qlog)):
            if path is not None and path.is_file():
                with path.open("rb") as source:
                    size = path.stat().st_size
                    head = source.read(16384)
                    source.seek(max(0, size - 16384))
                    tail = source.read(16384) if size > 16384 else b""
                parts.append(label + "\n" + (head + tail).decode("utf-8", errors="replace"))
        arm_diagnostic(arm, "\n".join(parts), rc, admitted, mode="guest")
    return rc


def watched(command, root, name):
    return ["bash", str(root / "scripts/bench/host_xid_watch.sh"), name, "--", *command]


def stage_grade(root, tag, log, grade, rc):
    if rc not in (0, 6):
        return grade(rc)
    text = bounded_text(log)
    try:
        delta = [xid_record_key(line) for line in watch_delta(text, rc)]
        allowed_path = Path(os.environ["KF_PAYLOAD_XID_ALLOWED"])
        allowed = [xid_record_key(json.loads(s)) for s in allowed_path.read_text().splitlines()]
        if collections.Counter(delta) != collections.Counter(allowed):
            raise EvidenceError("host_xid")
        allowed_path.write_text("")
    except (EvidenceError, ValueError, OSError):
        # Parse completed rows before appending the failed Xid gate.
        try:
            rows = grade(1)
        except EvidenceError as error:
            rows = error.rows
        raise EvidenceError("host_xid", rows) from None
    return grade(0)


def main():
    global STOP
    if len(sys.argv) == 5 and sys.argv[1] == "--grade-xid":
        return guard_command(sys.argv[2], Path(sys.argv[3]), int(sys.argv[4]))
    if len(sys.argv) > 3 and sys.argv[1] == "--guest-exec":
        return guest_command(sys.argv[2], sys.argv[3:])
    sha = sys.argv[1] if len(sys.argv) > 1 else ""
    nonce = sys.argv[2] if len(sys.argv) > 2 else ""
    if not re.fullmatch("[0-9a-f]{40}", sha) or not re.fullmatch("[0-9a-f]{32}", nonce):
        return 2
    root = Path(__file__).resolve().parents[2]
    bench = Path("/workspace/bench")
    evidence = Path("/workspace/kf-ci-evidence")
    evidence.mkdir(parents=True, exist_ok=True)
    destination = Path("/workspace/kf-result.json")
    result = {"verdict": "FAIL", "suites": [], "hardware": hardware(), "host_reference": "MISSING"}
    STAGE_RECORDS.clear()

    def terminate(*_):
        global STOP
        STOP = True

    old_term = signal.signal(signal.SIGTERM, terminate)
    try:
        soft = int(os.environ["KF_SOFT_DEADLINE_MS"])
        subprocess.run(["df", "-h", "/"], check=True)
        free = shutil.disk_usage("/").free
        print("KF_PREFLIGHT_FREE_BYTES=" + str(free), flush=True)
        if free < 60000000000 or not Path("/dev/kvm").exists():
            raise EvidenceError("preflight")
        if int(time.time() * 1000) >= soft:
            raise EvidenceError("timeout")
        branch = prepare_checkout(root, sha)
        link = Path("/root/kayfabe")
        if link.exists() and link.resolve() != root:
            raise EvidenceError("preflight")
        if not link.exists():
            link.symlink_to(root, target_is_directory=True)
        shutil.copyfile(root / "scripts/bench/provision_box.sh", "/root/provision_box.sh")
        os.environ.update(
            KF_PROVISION_PROFILE="ci",
            KAYFABE_BRANCH=branch,
            KAYFABE_REPO=str(root),
            BENCH_DIR=str(bench),
            KF_DEVICE="kf3",
            CARGO_TARGET_DIR=str(root / "target"),
            CARGO_BUILD_JOBS=str(min(8, os.cpu_count() or 2)),
            # Quiet only: cargo's own progress and colour. Lint level and RUSTFLAGS are not touched.
            CARGO_TERM_QUIET="true",
            CARGO_TERM_COLOR="never",
            CARGO_TERM_PROGRESS_WHEN="never",
            BASH_ENV=str(root / "scripts/ci/vast_payload_hooks.sh"),
            KF_PAYLOAD_ROOT=str(root),
            KF_PAYLOAD_XID_ALLOWED=str(evidence / "xid-allowed.jsonl"),
            KF_PAYLOAD_XID_FAILED=str(evidence / "xid-failed"),
            KF_PAYLOAD_LOG="/workspace/kf-ci-output.log",
            KF_PAYLOAD_DIAGNOSTIC_STATE=str(evidence / "arm-diagnostics.json"),
            KF_PAYLOAD_CONTROL_DIR=str(evidence / "native-controls"),
            KF_PAYLOAD_NATIVE_RECEIPT=str(evidence / "native-receipt.json"),
        )
        Path(os.environ["KF_PAYLOAD_DIAGNOSTIC_STATE"]).write_text(
            json.dumps({"bytes": 0, "events": []}) + "\n"
        )
        Path(os.environ["KF_PAYLOAD_XID_ALLOWED"]).write_text("")
        Path(os.environ["KF_PAYLOAD_XID_FAILED"]).unlink(missing_ok=True)
        os.environ["PATH"] = str(Path.home() / ".cargo/bin") + ":" + os.environ["PATH"]
        tag = "ci" + nonce[:16]

        def make_stage(name, command, output, progress, grade, detail=()):
            watch_tag = tag + "_" + name
            watcher = bench / ("xid_" + watch_tag + ".log")
            return dict(
                name=name,
                command=watched(command, root, watch_tag),
                output=output,
                progress=progress,
                # Logs read only for the FINAL SUMMARY's first failing line.
                detail=[*detail, watcher],
                grade=lambda rc: stage_grade(root, watch_tag, watcher, grade, rc),
            )

        def provision_grade(rc):
            result["hardware"] = hardware()
            rows = provision_rows(bounded_text("/root/prov/prov.log"), rc)
            tags = {
                s
                for s in (root / "tools/drivermatrix/tags.txt").read_text().splitlines()
                if s and not s.startswith("#")
            }
            if (
                result["hardware"]["driver_version"] not in tags
                or result["hardware"]["compute_cap"] < 750
            ):
                raise EvidenceError("preflight", rows)
            return rows

        stages = [
            make_stage(
                "provision",
                ["bash", str(root / "scripts/bench/box/provision_full.sh")],
                evidence / "provision.run",
                [Path("/root/prov/prov.log")],
                provision_grade,
                [Path("/root/prov") / x for x in PROVISION_STEP_LOGS.values()],
            )
        ]
        for mode in ["host"]:
            path = bench / ("cl_" + tag + "_" + mode + ".out")
            stages.append(
                make_stage(
                    "cuda_" + mode,
                    [
                        "bash",
                        str(root / "scripts/bench/cuda_ladder.sh"),
                        mode,
                        tag,
                        "1",
                        "cup2,cup3,cup8,cup8bench",
                    ],
                    evidence / ("cuda_" + mode + ".run"),
                    [path],
                    lambda rc, mode=mode, path=path: ladder_rows(bounded_text(path), rc, mode, sha),
                )
            )
        merge = Path("/root/prov") / (tag + ".log")
        stages.append(
            make_stage(
                "merge_bar",
                ["bash", str(root / "scripts/bench/box/merge_check.sh"), sha, tag],
                evidence / "merge.run",
                [merge, Path("/root/prov") / (tag + "_host.log"), bench / (tag + "_suite.out")],
                lambda rc: merge_rows(bounded_text(merge), rc, sha),
                [
                    Path("/root/prov") / (tag + "_" + x)
                    for x in [
                        "tests.log",
                        "gates.log",
                        "kf3.log",
                        "client.log",
                        "fg.log",
                        "suite.run",
                        "census_selftest.log",
                        "births.log",
                    ]
                ],
            )
        )
        path = bench / ("cl_" + tag + "_guest.out")
        stages.append(
            make_stage(
                "cuda_guest",
                [
                    "bash",
                    str(root / "scripts/bench/cuda_ladder.sh"),
                    "guest",
                    tag,
                    "1",
                    "cup2,cup3,cup8,cup8bench",
                ],
                evidence / "cuda_guest.run",
                [path],
                lambda rc: ladder_rows(bounded_text(path), rc, "guest", sha),
            )
        )
        return execute_stages(stages, soft, result, destination)
    except EvidenceError as error:
        result["suites"].extend(error.rows)
        result["suites"].append(row(error.name, failed=1))
        persist_result(destination, result)
        print("KF_PAYLOAD_FAIL suite=" + error.name, flush=True)
        print_final_summary(result)
        return 1
    except (KeyError, ValueError, OSError, subprocess.SubprocessError, UnicodeError):
        result["suites"].append(row("infrastructure", failed=1))
        persist_result(destination, result)
        print("KF_PAYLOAD_FAIL suite=infrastructure", flush=True)
        print_final_summary(result)
        return 1
    finally:
        signal.signal(signal.SIGTERM, old_term)


if __name__ == "__main__":
    raise SystemExit(main())
