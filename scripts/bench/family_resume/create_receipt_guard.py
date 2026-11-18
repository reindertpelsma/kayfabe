"""One durable create intent; discover only an exact, account-owned allocation."""

from dataclasses import dataclass
import datetime as dt
import fcntl
import hashlib
import json
import math
import os
from pathlib import Path
import re
import subprocess
import uuid


class Unresolved(ValueError):
    """Ownership or allocation is unproven; keep liability and forbid another create."""


@dataclass(frozen=True)
class IntentRef:
    path: Path
    sha256: str


def utc():
    return dt.datetime.now(dt.timezone.utc)


def process_start(pid):
    return int(Path("/proc", str(pid), "stat").read_text().rpartition(")")[2].split()[19])


def parent_alive(intent):
    try:
        return process_start(intent["parent_pid"]) == intent["parent_starttime"]
    except (FileNotFoundError, ProcessLookupError):
        return False


def fsync_directory(path):
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def durable(path, value, *, exclusive=False):
    data = (json.dumps(value, sort_keys=True, indent=2) + "\n").encode()
    target = path if exclusive else path.with_name(path.name + "." + uuid.uuid4().hex + ".tmp")
    fd = os.open(target, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    try:
        with os.fdopen(fd, "wb") as stream:
            stream.write(data)
            stream.flush()
            os.fchmod(stream.fileno(), 0o400 if exclusive else 0o600)
            os.fsync(stream.fileno())
        if not exclusive:
            os.replace(target, path)
        fsync_directory(path.parent)
    except BaseException:
        # A partial immutable intent/issue marker remains a refusal, never a retry token.
        raise
    return hashlib.sha256(data).hexdigest()


def positive_id(value):
    return type(value) is int and value > 1


def validate_snapshot(owner, snapshot):
    if (
        type(snapshot) is not dict
        or snapshot.get("owner_id") != owner
        or type(snapshot.get("owner_id")) is not int
        or snapshot.get("complete") is not True
    ):
        raise Unresolved("inventory owner/completeness unverified")
    rows = snapshot.get("rows")
    if type(rows) is not list or len(rows) > 10000:
        raise Unresolved("inventory shape unverified")
    ids = []
    for row in rows:
        if type(row) is not dict or not positive_id(row.get("id")):
            raise Unresolved("inventory id unverified")
        ids.append(row["id"])
    if len(set(ids)) != len(ids):
        raise Unresolved("duplicate inventory ids")
    return rows


def prepare(root, offer, plan, owner_id, snapshot, source):
    if not all(
        positive_id(x) for x in [owner_id, offer["id"], offer["machine_id"], offer["host_id"]]
    ):
        raise Unresolved("intent identity invalid")
    rows = validate_snapshot(owner_id, snapshot)
    armed = dt.datetime.fromisoformat(plan["armed_utc"])
    deadline = dt.datetime.fromisoformat(plan["deadline_utc"])
    if (
        armed.utcoffset() is None
        or deadline.utcoffset() is None
        or deadline - armed != dt.timedelta(seconds=plan["seconds"])
        or not 0 < plan["seconds"] <= 10800
        or not re.fullmatch("[0-9a-f]{40}", source)
    ):
        raise Unresolved("intent source/deadline invalid")
    data = dict(
        schema=1,
        label="codex-sw5-c-" + uuid.uuid4().hex,
        owner_id=owner_id,
        machine_id=offer["machine_id"],
        host_id=offer["host_id"],
        offer_id=offer["id"],
        source=source,
        plan=plan,
        excluded_ids=sorted(row["id"] for row in rows),
        parent_pid=os.getpid(),
        parent_starttime=process_start(os.getpid()),
    )
    path = root / "intent.json"
    digest = durable(path, data, exclusive=True)
    if not (root / "owned.json").exists():
        durable(root / "owned.json", [])
    return IntentRef(path, digest)


def load(ref):
    data = ref.path.read_bytes()
    if len(data) > 65536 or hashlib.sha256(data).hexdigest() != ref.sha256:
        raise Unresolved("immutable intent digest changed")
    intent = json.loads(data)
    if not re.fullmatch("codex-sw5-c-[0-9a-f]{32}", intent["label"]):
        raise Unresolved("intent label invalid")
    return intent


def match(intent, snapshot):
    rows = validate_snapshot(intent["owner_id"], snapshot)
    candidates = [row for row in rows if row.get("label") == intent["label"]]
    if len(candidates) != 1:
        raise Unresolved("exact intent match absent or multiple")
    row = candidates[0]
    if row["id"] in intent["excluded_ids"]:
        raise Unresolved("precreate inventory id excluded")
    for key in ("machine_id", "host_id"):
        if type(row.get(key)) is not int or row[key] != intent[key]:
            raise Unresolved("exact intent machine/host mismatch")
    if "client_id" in row and (
        type(row["client_id"]) is not int or row["client_id"] != intent["owner_id"]
    ):
        raise Unresolved("explicit provider owner mismatch")
    # Optional start_date is audit metadata, never an ownership predicate. An allocation
    # from this sole exact intent remains ours even when it appears after the deadline.
    return row["id"]


def issue(root, ref, ready, invoke, *, now=utc):
    intent = load(ref)
    if ready.get("state") != "ARMED" or ready.get("intent_sha256") != ref.sha256:
        raise Unresolved("matching watchdog not armed")
    if now() >= dt.datetime.fromisoformat(intent["plan"]["deadline_utc"]):
        raise Unresolved("create deadline expired")
    try:
        durable(
            root / "create_started.json",
            dict(intent_sha256=ref.sha256, utc=now().isoformat(), label=intent["label"]),
            exclusive=True,
        )
    except FileExistsError as error:
        raise Unresolved("create already issued or uncertain; zero retry") from error
    outcome = dict(kind="UNRESOLVED", intent_sha256=ref.sha256, retry_allowed=False)
    rawout = rawerr = b""
    candidate = None
    try:
        result = invoke(intent["label"])
        rawout, rawerr = result.stdout or b"", result.stderr or b""
        outcome["returncode"] = result.returncode
        if result.returncode != 0:
            raise Unresolved("nonzero create outcome")
        value = json.loads(rawout)
        if (
            type(value) is not dict
            or value.get("success") is not True
            or not positive_id(value.get("new_contract"))
            or value["new_contract"] in intent["excluded_ids"]
        ):
            raise Unresolved("create receipt lacks valid new contract")
        candidate = value["new_contract"]
        outcome.update(kind="ACCEPTED_RECEIPT", success=True, new_contract=candidate)
    except Exception as error:
        outcome["exception_type"] = type(error).__name__
        if isinstance(error, subprocess.TimeoutExpired):
            rawout, rawerr = error.output or b"", error.stderr or b""
    for name, raw in [("stdout", rawout), ("stderr", rawerr)]:
        if isinstance(raw, str):
            raw = raw.encode()
        outcome[name + "_bytes"] = len(raw)
        outcome[name + "_sha256"] = hashlib.sha256(raw).hexdigest()
    durable(root / "create_outcome.json", outcome)
    if candidate is None:
        raise Unresolved("create receipt unresolved; independent reconciliation required")
    return candidate


def bind(root, ref, intent, iid):
    with (root / "ledger.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        path = root / "owned.json"
        rows = json.loads(path.read_text())
        if rows:
            if (
                len(rows) != 1
                or rows[0].get("id") != iid
                or rows[0].get("intent_sha256") != ref.sha256
            ):
                raise Unresolved("ledger ownership conflict")
            return
        plan = intent["plan"]
        row = dict(
            id=iid,
            intent_sha256=ref.sha256,
            label=intent["label"],
            owner_id=intent["owner_id"],
            machine_id=intent["machine_id"],
            host_id=intent["host_id"],
            created_utc=plan["armed_utc"],
            quote_dph=plan["quoted_rate"],
            deadline_utc=plan["deadline_utc"],
            destroyed=False,
            ownership="exact immutable intent + authenticated inventory",
        )
        durable(path, [row])


def watch(
    root, ref, *, inventory, credit, now, pause, parent_alive, ready, publish, capture, destroy
):
    """Unresolved attempts keep their reservation and remain under read-only observation."""
    from lease_guard import retire

    intent = load(ref)
    plan = intent["plan"]
    deadline = dt.datetime.fromisoformat(plan["deadline_utc"])
    if now() >= deadline:
        raise Unresolved("cannot arm expired intent")
    ready(dict(state="ARMED", intent_sha256=ref.sha256, deadline_utc=plan["deadline_utc"]))
    owned = None

    def retire_known(reason):
        # Ownership was positively established earlier. Later API failures cannot revoke
        # our duty to stop that exact ID; they can prevent verified absence, never deletion.
        def status(iid):
            current = inventory()
            current_rows = validate_snapshot(intent["owner_id"], current)
            return dict(id=iid, absent=not any(r["id"] == iid for r in current_rows))

        result = retire(owned, capture=capture, destroy=destroy, status=status, pause=pause)
        return dict(
            result,
            state="CLOSED",
            reason=reason,
            retry_allowed=False,
            liability_usd=plan["reservation_usd"],
            intent_sha256=ref.sha256,
        )

    while True:
        state = dict(
            state="UNRESOLVED",
            id=owned,
            retry_allowed=False,
            liability_usd=plan["reservation_usd"],
            intent_sha256=ref.sha256,
        )
        # This check precedes inventory, credit, metadata and mutable-file reads. The fixed
        # deadline and the already verified exact ownership proof live in this process.
        due = (deadline - now()).total_seconds() <= plan.get("capture_lead_seconds", 420)
        dead = False
        if owned is not None:
            try:
                dead = not parent_alive(intent)
            except Exception:
                dead = True
        if owned is not None and (due or dead):
            try:
                reason = "creator exited or observer failed" if dead else "fixed capture cutoff"
                result = retire_known(reason + " using retained ownership proof")
            except Exception as error:
                state.update(exception_type=type(error).__name__)
                publish(state)
                pause(20)
                continue
            publish(result)
            return result
        try:
            load(ref)
            snapshot = inventory()
            rows = validate_snapshot(intent["owner_id"], snapshot)
            if owned is None:
                owned = match(intent, snapshot)
                bind(root, ref, intent, owned)
                state.update(state="BOUND", id=owned)
            elif not any(row["id"] == owned for row in rows):
                # An empty observation after binding does not replace our teardown duty.
                result = retire_known("bound ID missing from inventory")
                publish(result)
                return result
            elif match(intent, snapshot) != owned:
                raise Unresolved("bound inventory identity changed")
            remaining = (deadline - now()).total_seconds()
            outcome_path = root / "create_outcome.json"
            outcome = json.loads(outcome_path.read_text()) if outcome_path.exists() else {}
            balance = credit()
            rate = next(row["dph_total"] for row in rows if row["id"] == owned)
            if (
                type(balance) not in (float, int)
                or not math.isfinite(balance)
                or type(rate) not in (float, int)
                or not math.isfinite(rate)
                or rate <= 0
            ):
                raise Unresolved("invalid provider rate/credit")
            reason = None
            if not parent_alive(intent):
                reason = "creator exited"
            if outcome.get("kind") == "UNRESOLVED":
                reason = "lost create receipt"
            if remaining <= plan.get("capture_lead_seconds", 420):
                reason = "fixed capture cutoff"
            if rate > plan["rate_bound"]:
                reason = "rate beyond reservation"
            if balance < plan["reserve_usd"]:
                reason = "account reserve"
            if reason:

                result = retire_known(reason)
                state.update(result, state="CLOSED", reason=reason)
                publish(state)
                return state
        except Exception as error:
            state.update(state="UNRESOLVED", exception_type=type(error).__name__)
            if (
                not (root / "create_started.json").exists()
                and not parent_alive(intent)
                and now() >= deadline
            ):
                # A fresh complete inventory is still mandatory even when no issue marker exists.
                try:
                    rows = validate_snapshot(intent["owner_id"], inventory())
                    if not any(r.get("label") == intent["label"] for r in rows):
                        state.update(state="NO_CREATE_ISSUED", liability_usd=0)
                        publish(state)
                        return state
                except Exception:
                    pass
        publish(state)
        wait = 20
        if owned is not None:
            wait = min(
                wait,
                max(0, (deadline - now()).total_seconds() - plan.get("capture_lead_seconds", 420)),
            )
        pause(wait)
