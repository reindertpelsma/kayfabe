"""HM1 host-fault policy seam; initial implementation preserves the original guard."""


def audit(lines, *, arm, pid, allowed_vas, prime_limit, serial):
    """Return rejected kernel records. Initially every Xid/OOM is rejected."""
    return [line for line in lines if any(word in line for word in (
        "Xid", "Out of memory:", "oom-kill:", "Killed process "
    ))]


def exit_code(reason, process_returncode):
    """Preserve the initial controller's stop status pending the expected-red test."""
    return 124 if reason else process_returncode
