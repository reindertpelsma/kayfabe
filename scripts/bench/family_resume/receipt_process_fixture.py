"""Filesystem-only fake provider. Never imports or invokes the Vast CLI."""

import datetime as dt
import json
import os
from pathlib import Path
import subprocess
import sys
import time

import create_receipt_guard as G


def event(root, kind):
    with (root / "events.jsonl").open("a") as out:
        out.write(json.dumps(dict(kind=kind)) + "\n")
        out.flush()
        os.fsync(out.fileno())


def watcher(root):
    ref = G.IntentRef(root / "intent.json", (root / "intent.sha256").read_text())

    def ready(value):
        G.load(ref)
        event(root, "ARMED")
        G.durable(root / "ready.json", value)

    def destroy(iid):
        assert type(iid) is int and iid == 42
        event(root, "destroy42")
        G.durable(root / "fake_inventory.json", dict(owner_id=71, complete=True, rows=[]))

    def capture(iid):
        assert iid == 42
        event(root, "capture42")
        return dict(id=iid, capture_verified=True)

    result = G.watch(
        root,
        ref,
        inventory=lambda: json.loads((root / "fake_inventory.json").read_text()),
        credit=lambda: 40,
        now=G.utc,
        pause=lambda _: time.sleep(0.01),
        parent_alive=G.parent_alive,
        ready=ready,
        publish=lambda v: G.durable(root / "watch_state.json", v),
        capture=capture,
        destroy=destroy,
    )
    G.durable(root / "watch_done.json", result)


def creator(root, mode):
    root.mkdir()
    G.durable(root / "fake_inventory.json", dict(owner_id=71, complete=True, rows=[]))
    now = G.utc()
    plan = dict(
        armed_utc=now.isoformat(),
        deadline_utc=(now + dt.timedelta(seconds=8)).isoformat(),
        seconds=8,
        rate_bound=0.34,
        quoted_rate=0.24,
        reserve_usd=8,
        reservation_usd=1.23,
        stop_usd=1.5,
        capture_lead_seconds=0,
    )
    ref = G.prepare(
        root,
        dict(id=900, machine_id=21018, host_id=120071),
        plan,
        71,
        dict(owner_id=71, complete=True, rows=[]),
        "7dcedefea093de078188efd5f712561bde746362",
    )
    (root / "intent.sha256").write_text(ref.sha256)
    event(root, "intent_durable")
    log = (root / "watch.log").open("wb")
    child = subprocess.Popen(
        [sys.executable, __file__, "watch", str(root)],
        stdout=log,
        stderr=log,
        stdin=subprocess.DEVNULL,
        start_new_session=True,
    )
    G.durable(
        root / "watch_process.json", dict(pid=child.pid, starttime=G.process_start(child.pid))
    )
    until = time.monotonic() + 4
    while not (root / "ready.json").exists():
        if time.monotonic() >= until:
            raise RuntimeError("fake watcher did not arm")
        time.sleep(0.01)
    ready = json.loads((root / "ready.json").read_text())

    def invoke(label):
        assert (root / "create_started.json").exists()
        event(root, "create")
        G.durable(
            root / "fake_inventory.json",
            dict(
                owner_id=71,
                complete=True,
                rows=[
                    dict(
                        id=42,
                        label=label,
                        machine_id=21018,
                        host_id=120071,
                        dph_total=0.24,
                        start_date=G.utc().timestamp(),
                    )
                ],
            ),
        )
        if mode == "crash":
            os._exit(23)
        if mode in ("timeout", "adapter-timeout"):
            raise subprocess.TimeoutExpired(
                "fake-create", 45, output=b"FAKE_CREATE_SECRET_MUST_NEVER_PERSIST"
            )
        return subprocess.CompletedProcess([], 0, b"", b"FAKE_CREATE_SECRET_MUST_NEVER_PERSIST")

    try:
        if mode.startswith("adapter-"):
            import operator_backend as O

            O.approval_gate.require_go = lambda *args: {"fixture_only": True}
            O.P.create_once = lambda offer, label: invoke(label)
            O.P.snapshot = lambda owner, **kwargs: dict(owner_id=owner, complete=True, rows=[])

            class LiveFakeWatch:
                def poll(self):
                    return None

            backend = O.Backend.__new__(O.Backend)
            backend.here = Path(O.__file__).resolve().parent
            backend.offer = dict(id=900, machine_id=21018, host_id=120071)
            backend.intent_ref, backend.evidence, backend.watch = ref, root, LiveFakeWatch()
            backend.label = G.load(ref)["label"]
            G.durable(root / "watch_ready.json", ready)
            backend.create()
        else:
            G.issue(root, ref, ready, invoke)

    except G.Unresolved:
        os._exit(17)
    raise AssertionError("fake lost response unexpectedly succeeded")


if __name__ == "__main__":
    action, root = sys.argv[1], Path(sys.argv[2])
    if action == "watch":
        watcher(root)
    elif action == "creator":
        creator(root, sys.argv[3])
    else:
        raise ValueError("fixture action invalid")
