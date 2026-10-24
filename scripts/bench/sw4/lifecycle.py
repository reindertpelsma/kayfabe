"""One owned lease: precreate arming, immediate reporting, capture and exact teardown."""

import hashlib
from pathlib import Path


def source_lock(files):
    for filename, expected in files.items():
        actual = hashlib.sha256(Path(filename).read_bytes()).hexdigest()
        if actual != expected:
            raise ValueError("source lock mismatch: " + Path(filename).name)


def execute(backend, plan):
    watch = backend.arm(plan)
    if watch.get("state") != "ARMED" or watch.get("deadline_utc") != plan["deadline_utc"]:
        raise ValueError("watchdog did not attest the fixed precreate deadline")
    row = {"watch": watch, "deadline_utc": plan["deadline_utc"], "id": None}
    backend.report(row)
    iid = backend.create()
    if type(iid) is not int or iid <= 1:
        raise ValueError("create did not return an exact contract id")
    row["id"] = iid
    backend.report(row)
    failure = None
    try:
        backend.ready(iid)
        backend.run(iid)
    except Exception as error:
        failure = error
    try:
        capture, expected = backend.capture(iid)
        if hashlib.sha256(capture).hexdigest() != expected:
            raise ValueError("private capture hash mismatch")
        row.update(capture_verified=True, capture_sha256=expected)
        backend.report(row)
    except Exception as error:
        failure = error
    # Safety teardown never waits for capture integrity or publication.
    backend.destroy(iid)
    if not backend.absent(iid):
        raise RuntimeError("API absence unknown; watchdog remains armed")
    row["api_absent"] = True
    if row.get("capture_verified"):
        backend.cancel(watch)
        row["watch_cancelled_after_absence"] = True
    backend.report(row)
    if failure is not None:
        raise failure
    return row
