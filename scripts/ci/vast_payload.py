#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Thin adapter for the existing disposable-box provision and merge bar.
No provider/CI credentials or network clone machinery belongs here.
"""

import collections, json, os, re, select, shutil, signal, subprocess, sys, time
from pathlib import Path

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
    if re.search(r"^TESTS passed [0-9]+ failed [1-9]", text, re.M):
        raise EvidenceError("kf_tests")
    count = only(r"^TESTS passed ([1-9][0-9]*) failed 0$", text, "kf_tests", rows)
    rows.append(row("kf_tests", int(count)))
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
            r"^(?:BOX|DRIVER|TREE|LADDER|FG|KF3|GATES|SUITE|CARGO)_RC=[1-9][0-9]*\b", line, re.M
        )
        or re.search(r"^(?:BARE|FAST)_CELL_ARM\b.* verdict=(?!PASS\b)[A-Z_]+\b", line, re.M)
        or re.search(r"^V3_GATES_SUMMARY\b.* fail=[1-9]", line, re.M)
        or re.search(r"^TESTS passed [0-9]+ failed [1-9]", line, re.M)
        or re.search(r"^CL_ROW .* verdict=(?!PASS\b)[A-Z_]+\b", line, re.M)
    )


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
    temporary.write_text(json.dumps(result, separators=(",", ":")) + "\n")
    temporary.replace(destination)


def execute_stages(stages, soft, result, destination, runner=run_bounded):
    """Persist the finished prefix even when a later suite times out or fails."""
    persist_result(destination, result)
    for stage in stages:
        name = stage["name"]
        print("KF_PAYLOAD_STAGE_START name=" + name, flush=True)
        start = time.monotonic()
        rc = runner(stage["command"], soft, stage["output"], stage["progress"])
        elapsed = int((time.monotonic() - start) * 1000)
        print(
            "KF_PAYLOAD_STAGE_END name=" + name + " rc=" + str(rc) + " duration_ms=" + str(elapsed),
            flush=True,
        )
        try:
            rows = stage["grade"](rc)
            if rc == 124:
                raise EvidenceError("timeout", rows)
            result["suites"].extend(rows)
            persist_result(destination, result)
        except EvidenceError as error:
            result["suites"].extend(error.rows)
            failure = "timeout" if rc == 124 else error.name
            result["suites"].append(row(failure, failed=1))
            append_arm_failure(result)
            result["verdict"] = "FAIL"
            persist_result(destination, result)
            print("KF_PAYLOAD_FAIL suite=" + failure, flush=True)
            return 1
    result["verdict"] = "PASS"
    persist_result(destination, result)
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
    state["events"].append({"arm": arm, "rc": rc, "admitted": bool(admitted), "mode": mode})
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
    result = {"verdict": "FAIL", "suites": [], "hardware": hardware()}

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

        def make_stage(name, command, output, progress, grade):
            watch_tag = tag + "_" + name
            watcher = bench / ("xid_" + watch_tag + ".log")
            return dict(
                name=name,
                command=watched(command, root, watch_tag),
                output=output,
                progress=progress,
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
        return 1
    except (KeyError, ValueError, OSError, subprocess.SubprocessError, UnicodeError):
        result["suites"].append(row("infrastructure", failed=1))
        persist_result(destination, result)
        print("KF_PAYLOAD_FAIL suite=infrastructure", flush=True)
        return 1
    finally:
        signal.signal(signal.SIGTERM, old_term)


if __name__ == "__main__":
    raise SystemExit(main())
