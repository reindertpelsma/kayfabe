"""Argument-list subprocesses with bounded capture and ownership-scoped cleanup.

No implicit inherited environment. A caller can explicitly extend the key allow-list
for a legacy tool. The pipe is drained in fixed chunks, including unterminated lines.
Every child gets its own session; only that process group is ever signalled.
"""

from dataclasses import dataclass
import math
import os
import selectors
import signal
import subprocess
import time

BASE_ENV = frozenset(
    {"PATH", "HOME", "USER", "LOGNAME", "LANG", "LC_ALL", "LC_CTYPE", "TMPDIR", "TZ"}
)


@dataclass(frozen=True)
class ProcessResult:
    returncode: int
    output: str
    timed_out: bool = False
    truncated: bool = False


def _kill_group(pid, sig):
    try:
        os.killpg(pid, sig)
    except ProcessLookupError:
        pass


def run(
    argv,
    *,
    timeout,
    env=None,
    allow_env=BASE_ENV,
    cwd=None,
    capture_limit=65536,
    log=None,
    on_tick=None,
    poll_interval=0.05,
    grace_seconds=30
):
    if (
        not isinstance(argv, list)
        or not argv
        or not all(isinstance(a, str) and "\0" not in a for a in argv)
    ):
        raise ValueError("command must be a nonempty list of strings")
    for val, positive in (
        (timeout, True),
        (poll_interval, True),
        (grace_seconds, False),
    ):
        if (
            type(val) not in (int, float)
            or not math.isfinite(val)
            or (val <= 0 if positive else val < 0)
        ):
            raise ValueError("invalid process time limit")
    if type(capture_limit) is not int or capture_limit < 0:
        raise ValueError("invalid capture limit")
    environment = {k: v for k, v in (env or {}).items() if k in allow_env}
    child = subprocess.Popen(
        argv,
        cwd=cwd,
        env=environment,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        start_new_session=True,
    )
    selector = selectors.DefaultSelector()
    captured = bytearray()
    total = 0
    timed_out = False
    forced = False
    deadline = time.monotonic() + timeout
    term_at = None
    parent_done_at = None
    try:
        os.set_blocking(child.stdout.fileno(), False)
        selector.register(child.stdout, selectors.EVENT_READ)
        while True:
            now = time.monotonic()
            rc = child.poll()
            if rc is not None and parent_done_at is None:
                parent_done_at = now
                # A wrapper may exit leaving children with the pipe open. They belong
                # to this invocation too; do not wait for a grandchild's 30-minute sleep.
                _kill_group(child.pid, signal.SIGTERM)
            if rc is None and not timed_out and now >= deadline:
                timed_out = True
                term_at = now
                _kill_group(child.pid, signal.SIGTERM)
            cleanup_at = term_at if term_at is not None else parent_done_at
            if cleanup_at is not None and now >= cleanup_at + grace_seconds:
                _kill_group(child.pid, signal.SIGKILL)
                forced = rc is None
            if on_tick is not None:
                on_tick()
            for key, _ in selector.select(
                min(poll_interval, max(0, deadline - now))
                if not timed_out and rc is None
                else poll_interval
            ):
                block = os.read(key.fd, 65536)
                if not block:
                    selector.unregister(key.fileobj)
                    continue
                total += len(block)
                if log is not None:
                    log.write(block)
                    log.flush()
                captured.extend(block)
                if len(captured) > capture_limit:
                    del captured[: len(captured) - capture_limit]
            if child.poll() is not None and not selector.get_map():
                break
        rc = child.wait()
        if timed_out:
            rc = 137 if forced else 124
        return ProcessResult(
            rc,
            captured.decode("utf-8", errors="replace"),
            timed_out,
            total > capture_limit,
        )
    finally:
        # Also runs when a callback, signal handler, or log write raises.
        _kill_group(child.pid, signal.SIGKILL)
        child.wait()
        selector.close()
        child.stdout.close()
