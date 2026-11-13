"""Builders for every frame the daemon sends. One place, so the contract test can build the real frames.

Shapes follow the Worker's golden fixture (tests/fixtures/hosts-wire.json). PENDING (W1 is adding them; shapes as
the coordinator specified): `ready`, hello.enforced (object of booleans, <= 8 keys), heartbeat.idle_since (epoch
milliseconds), seed_register.warnings (<= 5 strings of <= 200 chars).
"""

import re

SEED_NAME = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]{0,99}\Z")  # stricter than the Worker: no leading '-' or '.'
OFFER_ID = re.compile(r"[0-9a-f]{16}\Z")
CMD_ID = re.compile(r"[0-9a-f]{16}\Z")
MAX_REASON = 200


def _v(frame):
    frame["v"] = 1
    return frame


def hello(box_id, version, capabilities, enforced):
    enforced = {k: bool(v) for k, v in list(enforced.items())[:8]}
    return _v({"type": "hello", "box_id": box_id, "version": version, "capabilities": capabilities,
               "enforced": enforced})


def heartbeat(ts_s, state, run_id=None, idle_since_ms=None):
    f = {"type": "heartbeat", "ts": int(ts_s), "state": state}
    if run_id is not None:
        f["run_id"] = run_id
    elif idle_since_ms is not None:
        f["idle_since"] = int(idle_since_ms)
    return _v(f)


def ack(offer_id):
    return _v({"type": "ack", "offer_id": offer_id})


def ready(offer_id):
    return _v({"type": "ready", "offer_id": offer_id})


def nack(offer_id, reason):
    return _v({"type": "nack", "offer_id": offer_id, "reason": str(reason)[:MAX_REASON]})


def seed_register(offer_id, artifact, path, warnings=()):
    name, size = artifact.get("name"), artifact.get("size")
    if not (isinstance(name, str) and SEED_NAME.fullmatch(name) and ".." not in name
            and re.fullmatch(r"[0-9a-f]{64}", str(artifact.get("sha256", "")))
            and type(size) is int and size >= 1):
        raise ValueError("invalid artifact")
    if not (isinstance(path, str) and re.fullmatch(r"/[a-z0-9]{8}/" + re.escape(name), path)):
        raise ValueError("invalid seed path")  # /<8-char token>/<name>, as the Worker's SEED_PATH
    f = {"type": "seed_register", "offer_id": offer_id,
         "artifact": {"name": name, "sha256": artifact["sha256"], "size": size}, "path": path}
    w = [str(x)[:200] for x in list(warnings)[:5]]
    if w:
        f["warnings"] = w
    return _v(f)


def cmd_output(cmd_id, seq, stream, data, eof=False):
    f = {"type": "cmd_output", "cmd_id": cmd_id, "seq": seq, "stream": stream, "data": data}
    if eof:
        f["eof"] = True
    return _v(f)


def cmd_done(cmd_id, rc):
    return _v({"type": "cmd_done", "cmd_id": cmd_id, "rc": int(rc)})
