"""Per-arm workload admission seam over the guardian's accepted journal."""
def may_run_apps(native, start, current):
    return tuple(current)==tuple(native)
