"""Linux apps release stage, ported from apps_gate.sh at 50640c605.

Verdict and row grammar stay compatible with vast_payload.apps_rows and the
run report. Batched failures are progress; an isolated rerun is authoritative.
The preexisting matrix and provisioning helpers remain separate legacy steps.
"""

from datetime import datetime
from dataclasses import replace
import hashlib
from pathlib import Path
import os
import re
import shutil
import tempfile
import time

from .proc import BASE_ENV
from .step import Cancelled, RetryClass, Step, StepResult, Verdict

# Explicit legacy dependency environment. In particular BASH_ENV carries the
# payload's existing Xid hooks; omitting it would silently weaken those checks.
APPS_ENV = BASE_ENV | frozenset(
    {
        "XID_OUTDIR",
        "BOOT_TIMEOUT",
        "DIE_RC",
        "KAYFABE_BENCH_HELPERS",
        "GQ_TIMEOUT",
        "PRE_POWEROFF_GUEST_CMD",
        "LLAMA_PIN",
        "KF_APPS_GEEKBENCH",
        "BASH_ENV",
        "KF_APPS_ROOT",
        "KF_APPS_LIST",
        "KF_APPS_MATRIX",
        "KF_APPS_BUDGET_S",
        "KF_APPS_PER_BOOT",
        "KF_APPS_POLL_S",
        "KF_APPS_APP_TMO",
        "KF_APPS_PROVISION",
        "APPS_ROOT",
        "APPS_PER_BOOT",
        "APPS_APP_TMO",
        "APPS_NO_ISOLATE",
        "APPS_GUEST_PM",
        "APPS_GUEST_DRM",
        "APPS_WEDGE_PROBE",
        "BENCH_DIR",
        "BENCH",
        "KF_GUEST_IMG",
        "KF_LOCK",
        "KF_DEVICE",
        "QEMU_BIN",
        "NVKVM_RAM_MB",
        "KF_SMP",
        "NVKVM_RAM_BACKEND",
        "KF3_FB_MB",
        "KF3_DEV_EXTRA",
        "KF3_BIN",
        "KF3_REPO",
        "KF3_REV",
        "KF_FIRMWARE",
        "KF_OVMF_CODE",
        "KF_OVMF_VARS",
        "KAYFABE_GUEST_BAR1_MB",
        "KF_APT_CACHE_ROOT",
        "KF_PAYLOAD_ROOT",
        "KF_PAYLOAD_XID_ALLOWED",
        "KF_PAYLOAD_XID_FAILED",
        "KF_PAYLOAD_LOG",
        "KF_PAYLOAD_DIAGNOSTIC_STATE",
        "KF_PAYLOAD_CONTROL_DIR",
        "KF_PAYLOAD_NATIVE_RECEIPT",
        "CARGO_TARGET_DIR",
        "CARGO_BUILD_JOBS",
        "CARGO_TERM_QUIET",
        "CARGO_TERM_COLOR",
        "CARGO_TERM_PROGRESS_WHEN",
        "RUSTFLAGS",
        "LD_LIBRARY_PATH",
        "CUDA_HOME",
        "CUDA_PATH",
        "KF_GPU_LOCK",
        "KF_GPU_IDLE_MIB",
    }
)


def legacy_allowlist(env):
    """Keep trusted product configuration; callers must keep these namespaces secret-free.

    KF3_*, KAYFABE_* and BASH_ENV are compatibility exceptions, not a secret detector.
    """
    return APPS_ENV | {key for key in env if key.startswith(("KF3_", "KAYFABE_"))}


def timestamp():
    return datetime.now().astimezone().isoformat(timespec="seconds")


def copy_image(source, destination):
    """Like cp to a new image: retain source permissions subject to the umask."""
    descriptor = os.open(
        destination, os.O_WRONLY | os.O_CREAT | os.O_EXCL, source.stat().st_mode & 0o777
    )
    with os.fdopen(descriptor, "wb") as target, source.open("rb") as original:
        shutil.copyfileobj(original, target)


def result_lines(path):
    """Bound any single evidence line before parsing it; never execute guest text."""
    if not path.is_file():
        return
    with path.open("rb") as stream:
        while True:
            line = stream.readline(65537)
            if not line:
                return
            if len(line) > 65536:
                prefix = line[:65536]
                while line and not line.endswith(b"\n"):
                    line = stream.readline(65537)
                # Keep the app prefix, but never grade a truncated PASS as evidence.
                yield prefix.decode("utf-8", errors="replace"), True
                continue
            yield line.decode("utf-8", errors="replace").removesuffix(
                "\n"
            ), b"\0" in line


def last_row(paths, app):
    prefix = "APPRES side=guest app=" + app + " "
    for path in paths:
        row = ""
        for line, invalid in result_lines(path):
            if line.startswith(prefix):
                row = (
                    (prefix + "verdict=INVALID rc=- secs=- note=invalid_result boot=-")
                    if invalid
                    else line
                )
        if row:
            return row
    return ""


def bounded_text(text, limit):
    """Render untrusted display text as one bounded UTF-8 line, without grading it."""
    text = "".join(
        " " if ord(char) < 32 or char in "\x7f\x85\u2028\u2029" else char
        for char in text
    )
    return text.encode("utf-8")[:limit].decode("utf-8", errors="ignore")


def field(pattern, row):
    match = re.match(pattern, row)
    return match[1] if match else ""


def grade_row(app, row):
    if not row:
        return (
            "NOT_RUN",
            "APP_ROW name=%s os=linux result=NOT_RUN secs=- guest_xid=- note=no result row (budget or boot)"
            % app,
        )
    verdict = field(r".* verdict=([A-Z_]*) .*", row)
    secs = field(r".* secs=([0-9-]*) .*", row) or "-"
    xid = field(r".* guest_xid=([0-9]*).*", row) or "-"
    note = bounded_text(field(r".* note=(.*) boot=.*", row), 120) or "-"
    if verdict == "PASS":
        result = "PASS"
    elif verdict in ("NOTRUN", "BOOT_FAIL"):
        result = "NOT_RUN"
        note = verdict + " " + note
    else:
        result = "FAIL"
        note = verdict + " " + note
    return result, "APP_ROW name=%s os=linux result=%s secs=%s guest_xid=%s note=%s" % (
        app,
        result,
        secs,
        xid,
        note,
    )


def decide(passed, failed, unrun, count, matrix_rc):
    if failed > 0:
        return StepResult(Verdict.FAIL)
    if unrun > 0:
        return StepResult.retry(
            RetryClass.TIME if matrix_rc in (124, 137) else RetryClass.BOX
        )
    if passed == count and matrix_rc == 0:
        return StepResult(Verdict.PASS)
    return StepResult.retry(RetryClass.OPERATOR)


class AppsGate(Step):
    id = "apps_gate"
    needs = ("linux_apps_ready",)
    produces = ("linux_apps_checked",)
    estimated_minutes = 50
    timeout_seconds = (
        10200  # optional provisioning (2 h) plus the default matrix budget
    )

    def __init__(self, profile=None):
        self.profile = profile
        if profile is not None:
            if profile not in ("full", "middle"):
                raise ValueError("unknown apps profile")
            self.id = "linux_apps_" + profile
            self.needs = ("kf3_bins", "open_image")
            self.produces = ()

    def run(self, ctx):
        if self.profile is not None:
            parameters = dict(ctx.parameters)
            parameters.setdefault("which", self.profile)
            identity = (
                (ctx.assignment_id or "local") + ":" + (ctx.instance_id or self.id)
            )
            parameters.setdefault(
                "tag", "kfci_" + hashlib.sha256(identity.encode()).hexdigest()[:24]
            )
            parameters.setdefault("prov", ctx.env.get("KF_PROV_DIR") or "/root/prov")
            ctx = replace(ctx, parameters=parameters)
        try:
            result = self.execute(ctx)
        except TimeoutError:
            result = StepResult.retry(RetryClass.TIME)
        except (OSError, ValueError, Cancelled):
            result = StepResult.retry(RetryClass.OPERATOR)
        suffix = " class=" + result.retry_class.value if result.retry_class else ""
        ctx.emit("APPS_STAGE_" + result.verdict.value + suffix)
        ctx.emit("APPS_STAGE_EXIT rc=%d" % result.rc)
        return result

    def execute(self, ctx):
        args, env = ctx.parameters, ctx.env
        tag, prov, which = (
            args.get("tag", ""),
            args.get("prov", ""),
            args.get("which", "full"),
        )
        if (
            set(args) - {"tag", "prov", "which"}
            or not isinstance(tag, str)
            or not re.fullmatch(r"[A-Za-z0-9_]+", tag)
            or not isinstance(prov, str)
            or not prov
            or which not in ("full", "middle")
            or (self.profile is not None and which != self.profile)
        ):
            return StepResult.retry(RetryClass.USAGE)
        apps = ctx.root / "scripts/apps"
        listing = Path(
            env.get("KF_APPS_LIST")
            or apps / ("gate_list_middle.txt" if which == "middle" else "gate_list.txt")
        )
        budget = env.get("KF_APPS_BUDGET_S") or "3000"
        per_boot = env.get("KF_APPS_PER_BOOT") or "100"
        poll = float(env.get("KF_APPS_POLL_S") or "5")
        if poll <= 0:
            raise ValueError("poll interval must be positive")
        root = Path(env.get("KF_APPS_ROOT") or "/workspace/apps")
        bench = Path(env.get("BENCH_DIR") or "/workspace/bench")
        env["KF_GUEST_IMG"] = env.get("KF_GUEST_IMG") or str(bench / "guest_apps.qcow2")
        matrix = env.get("KF_APPS_MATRIX") or str(apps / "apps_matrix.sh")
        results = root / "results" / tag
        marker = root / "PROVISIONED"
        prov = Path(prov)
        prov.mkdir(parents=True, exist_ok=True)
        if not listing.is_file() or listing.stat().st_size == 0:
            return StepResult.retry(RetryClass.NO_LIST)
        with listing.open(newline="\n") as stream:
            gate = [
                s.removesuffix("\n")
                for s in stream
                if not re.match(r"^[ \t\r\v\f]*(#|$)", s)
            ]
        if not gate:
            return StepResult.retry(RetryClass.NO_LIST)
        with tempfile.TemporaryDirectory() as scratch:
            # run_apps.sh creates its output directory even when asked only for names.
            prior = env.get("APPS_ROOT")
            env["APPS_ROOT"] = scratch
            try:
                known = ctx.process(
                    ["bash", str(apps / "run_apps.sh"), "x", "list"],
                    timeout=30,
                    allow_env=legacy_allowlist(env),
                )
            finally:
                if prior is None:
                    env.pop("APPS_ROOT", None)
                else:
                    env["APPS_ROOT"] = prior
        if known.timed_out or known.truncated:
            return StepResult.retry(RetryClass.OPERATOR)
        for app in gate:
            if (
                not re.fullmatch(r"[A-Za-z0-9_]+", app)
                or " " + app + " " not in " " + known.output.rstrip("\n") + " "
            ):
                return StepResult.retry(RetryClass.BAD_LIST)
        ctx.emit(
            "APPS_STAGE_LIST n=%d budget_s=%s per_boot=%s"
            % (len(gate), budget, per_boot)
        )
        bundle = root / "bundle.tgz"
        if not bundle.is_file() or not bundle.stat().st_size or not marker.is_file():
            if env.get("KF_APPS_PROVISION") == "1":
                ctx.emit("APPS_PROVISION_START " + timestamp())
                try:
                    rc = self.provision(
                        ctx, apps, bench, marker, prov / (tag + "_apps_provision.log")
                    )
                except (OSError, ValueError):
                    rc = 1
                ctx.emit("APPS_PROVISION_END rc=%d %s" % (rc, timestamp()))
            if not marker.is_file():
                return StepResult.retry(RetryClass.NOT_BUILT)
        if results.exists():
            shutil.rmtree(results)
        results.mkdir(parents=True)
        start = int(time.time())
        ctx.emit("APPS_STAGE_START %s tag=%s" % (timestamp(), tag))
        seen, next_poll = 0, 0.0

        def emit_tries(force=False):
            nonlocal seen, next_poll
            if not force and time.monotonic() < next_poll:
                return
            next_poll = time.monotonic() + poll
            total = 0
            for line, _invalid in result_lines(results / "guest.res"):
                if not line.startswith("APPRES "):
                    continue
                total += 1
                if total > seen:
                    rest = line.removeprefix("APPRES side=guest ")
                    ctx.emit(bounded_text("APP_TRY " + rest, 300))
            seen = total

        env["APPS_PER_BOOT"] = per_boot
        env["APPS_APP_TMO"] = env.get("KF_APPS_APP_TMO") or "1000"
        # GNU timeout's duration grammar is retained by leaving the budget wrapper
        # in argv. The independent outer bound is the declared context deadline;
        # do not replace GNU's duration parser or change its rc=125 classification.
        with (prov / (tag + "_apps_matrix.log")).open("wb") as log:
            ran = ctx.process(
                ["timeout", "-k", "30", budget, "bash", matrix, "guest", tag, *gate],
                timeout=ctx.remaining(),
                allow_env=legacy_allowlist(env),
                log=log,
                on_tick=emit_tries,
            )
        matrix_rc = ran.returncode if ran.returncode >= 0 else 128 - ran.returncode
        emit_tries(True)
        elapsed = int(time.time()) - start
        ctx.emit("APPS_STAGE_MATRIX_END rc=%d secs=%d" % (matrix_rc, elapsed))
        passed = failed = unrun = 0
        for app in gate:
            row = last_row([results / "guest_isolated.res", results / "guest.res"], app)
            result, line = grade_row(app, row)
            ctx.emit(line)
            if result == "PASS":
                passed += 1
            elif result == "NOT_RUN":
                unrun += 1
            else:
                failed += 1
                ctx.emit(app + " --- FAILED")
        ctx.emit(
            "APPS_STAGE_COUNTS pass=%d fail=%d not_run=%d of=%d secs=%d"
            % (passed, failed, unrun, len(gate), elapsed)
        )
        return decide(passed, failed, unrun, len(gate), matrix_rc)

    def provision(self, ctx, apps, bench, marker, logfile):
        with logfile.open("wb") as log:
            built = ctx.process(
                ["bash", str(apps / "build_bundle.sh")],
                timeout=7200,
                allow_env=legacy_allowlist(ctx.env),
                log=log,
            )
            if built.returncode:
                return (
                    built.returncode
                    if built.returncode >= 0
                    else 128 - built.returncode
                )
            image = Path(ctx.env["KF_GUEST_IMG"])
            if not image.is_file():
                copy_image(bench / "guest.qcow2", image)
            # The old sed changes every image= line and preserves all other bytes.
            staged = (bench / "guest.qcow2.STAGED").read_bytes()
            updated = re.sub(
                rb"(?m)^image=.*", lambda _: b"image=" + os.fsencode(image), staged
            )
            Path(str(image) + ".STAGED").write_bytes(updated)
            ran = ctx.process(
                ["bash", str(apps / "provision_guest_apps.sh")],
                timeout=7200,
                allow_env=legacy_allowlist(ctx.env),
                log=log,
            )
            if ran.returncode == 0:
                marker.write_text(timestamp() + "\n")
            return ran.returncode if ran.returncode >= 0 else 128 - ran.returncode
