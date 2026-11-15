#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The `llm_parity` payload stage: one LLM on the host and in the Linux guest, ratio recorded (stdlib only, Python 3.8+).

STATUS: LIVE as code, 2026-11-15; NOT YET RUN ON A GPU BOX, and every pin it needs is UNSET, so today it refuses (reason
`unpinned`). Design and first-run checklist: docs/design/V3_LLM_PARITY_STAGE.md. Owner ruling 2026-11-15 (OWNER_RULINGS section
AS item 8): running the LLM on the host and in the guest is fine; fetch the model and ollama ONCE through the box downloader and
share them read-only with the guest (virtio-fs, 9p only as a fallback); parity is at least 90 percent of native, preferably 95,
and it is REPORTED, NEVER A GATE.

    llm_parity.py plan  [--pins FILE] [--transport auto|virtiofs|9p]       print the plan and the pin state; run nothing
    llm_parity.py check [--pins FILE] [--transport ...]                    exit 0 when every item the transport needs is pinned
    llm_parity.py run --sha SHA --checkout DIR --work DIR --out FILE [--pins FILE] [--transport ...] [--pin-dir DIR]
                      [--urls FILE] [--s3-config FILE] [--vfsd-uid N] [--limit-bandwidth MBPS]
    llm_parity.py guest-hook TAG            (called by llm_parity_hook.sh inside boot_capture.sh with the guest up)

PASS means exactly this (and nothing about the ratio's value): both arms ran, every measured run of both arms produced the same
text, both arms held the whole model in GPU memory, the timings are usable and the ratio is recorded. The payload adds "no Xid beyond
the allow-list" (every stage runs under its Xid watcher). The ratio and the 0.90/0.95 targets are written as facts
(`meets_min`, `meets_preferred`) and `ratio.gate` is always false: nothing in this file or in the payload reads them to decide.

Trust: the host arm is trusted. The guest arm's output is UNTRUSTED data from the machine under test: it is size-bounded, strictly
parsed (`parse_arm_output`) and only counted, never executed or forwarded. A guest that lies can forge a ratio, which nobody gates on.
"""
import argparse
import json
import math
import os
from pathlib import Path
import re
import shlex
import shutil
import statistics
import subprocess
import sys
import time

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import box_downloader  # noqa: E402

sys.path.pop(0)

SCHEMA = 1
PINS_SCHEMA = "kayfabe.llm-parity-pins/1"
DEFAULT_PINS = HERE / "llm_parity_pins.json"
UNSET = "UNSET"
ITEM_NAMES = ("model", "ollama", "virtiofsd")
TRANSPORT_ITEMS = {"virtiofs": ("model", "ollama", "virtiofsd"), "9p": ("model", "ollama")}
TRANSPORTS = ("auto", "virtiofs", "9p")
STAGE = "llm_parity"
RATIO_MIN = 0.90          # owner ruling: at least 90 percent of native ...
RATIO_PREFERRED = 0.95    # ... preferably 95. Reported, never a gate.
MAX_ARM_OUTPUT = 64 * 1024
MAX_RECORD_BYTES = 16 * 1024
TAG = "kfllm"
SOCKET_NAME = "kfllm.sock"
VFSD_SOCKET_WAIT_S = 30
GUEST_MOUNT = "/mnt/kf-llm"
GUEST_WORK = "/var/tmp/kf-llm-arm"
SHARE_FILES = {"model": "model.gguf", "ollama": "ollama.tgz"}
DEFAULT_VFSD_UID = 65534
HOST_PORT = 11555
GUEST_PORT = 11555
MIN_FREE_BYTES = 12 << 30
TAG_RE = re.compile(r"[A-Za-z0-9_]{1,31}")
PATH_RE = re.compile(r"/[A-Za-z0-9._@+/-]{1,4000}")  # no comma (QEMU option separator), space, quote, `=`, glob or control character
SOCKET_PATH_MAX = 100  # sun_path is 108 bytes including the NUL
REASON_RE = re.compile(r"[a-z0-9_]{1,48}")
HEX64 = re.compile(r"[0-9a-f]{64}")
VERSION_RE = re.compile(r"[A-Za-z0-9._+-]{1,40}")

# Every reason this module can name (a test checks that nothing else is ever emitted).
REASONS = frozenset((
    # refusals, before any work
    "unpinned", "pins_invalid", "usage", "not_root", "no_kf3_binary", "no_share_transport", "transport_unavailable",
    "share_path_unsafe", "share_not_traversable", "disk_space", "download_failed", "share_build_failed",
    "virtiofsd_failed", "boot_failed", "guest_no_result",
    # after the arms ran
    "host_arm_failed", "guest_arm_failed", "inputs_mismatch", "version_mismatch", "params_differ", "not_on_gpu",
    "timing_invalid", "text_disagree", "host_server_left_running", "guest_hook_failed",
    # a bug in this module: the record still reaches the log and the result file
    "internal_error",
))


class StageError(Exception):
    """A refusal or a failure with a NAMED reason (always one of REASONS)."""

    def __init__(self, reason, detail=""):
        assert reason in REASONS, reason
        self.reason = reason
        self.detail = detail
        super().__init__(reason)


# ------------------------------------------------------------------ pins and the downloader manifest

def load_pins(path=DEFAULT_PINS):
    """The pin file, strictly. An item is pinned only when sha256 and size are BOTH real values; UNSET is allowed for both."""
    try:
        raw = Path(path).read_bytes()
        if len(raw) > 64 * 1024:
            raise ValueError("size")
        doc = json.loads(raw.decode("utf-8"), parse_constant=lambda _c: (_ for _ in ()).throw(ValueError("constant")),
                         object_pairs_hook=_no_duplicates)
        check_pins(doc)
    except (OSError, ValueError, UnicodeError, RecursionError):
        raise StageError("pins_invalid") from None
    return doc


def _no_duplicates(pairs):
    out = {}
    for k, v in pairs:
        if k in out:
            raise ValueError("duplicate key")
        out[k] = v
    return out


def check_pins(doc):
    if type(doc) is not dict or set(doc) != {"schema", "status", "ollama_version", "model_id", "items"} or doc["schema"] != PINS_SCHEMA:
        raise ValueError("shape")
    version = doc["ollama_version"]
    if not (version == UNSET or (type(version) is str and VERSION_RE.fullmatch(version))):
        raise ValueError("ollama_version")
    if type(doc["status"]) is not str or type(doc["model_id"]) is not str:
        raise ValueError("text")
    items = doc["items"]
    if type(items) is not dict or tuple(sorted(items)) != tuple(sorted(ITEM_NAMES)):
        raise ValueError("items")
    for name, item in items.items():
        if type(item) is not dict or set(item) != {"dest", "sha256", "size", "sources", "note"}:
            raise ValueError("item " + name)
        sha, size = item["sha256"], item["size"]
        if sha != UNSET and not (type(sha) is str and HEX64.fullmatch(sha)):
            raise ValueError("sha256")
        if size != UNSET and not (type(size) is int and 1 <= size <= box_downloader.MAX_SIZE):
            raise ValueError("size")
        if (sha == UNSET) != (size == UNSET):
            raise ValueError("half pinned")
        if type(item["sources"]) is not list or type(item["note"]) is not str or type(item["dest"]) is not str:
            raise ValueError("item fields")


def unpinned(pins, names):
    """The names among NAMES that are not pinned, plus `ollama_version` when it is unset and ollama is wanted."""
    out = [n for n in names if pins["items"][n]["sha256"] == UNSET]
    if "ollama" in names and pins["ollama_version"] == UNSET:
        out.append("ollama_version")
    return sorted(out)


def build_manifest(pins, names):
    """The downloader manifest (kayfabe.download-manifest/1) for NAMES, validated by the downloader itself. Refuses unpinned."""
    missing = unpinned(pins, names)
    if missing:
        raise StageError("unpinned", ",".join(missing))
    doc = {
        "schema": box_downloader.SCHEMA, "stages": [STAGE],
        "items": [
            {"name": n, "dest": pins["items"][n]["dest"], "sha256": pins["items"][n]["sha256"], "size": pins["items"][n]["size"],
             "needed_by": STAGE, "sources": pins["items"][n]["sources"]}
            for n in names
        ],
    }
    try:
        box_downloader.from_document(doc)
    except box_downloader.ManifestError as error:
        raise StageError("pins_invalid", str(error)[:80]) from None
    return doc


# ------------------------------------------------------------------ the share (arguments, with hostile-input handling)

def safe_path(value, *, directory=False, check_fs=True, writable_ok=False):
    """A path that may go into a QEMU option or a daemon argument: absolute, a conservative alphabet (so no comma, space, quote,
    newline, `=` or glob), no `.`/`..`/empty segment, no trailing slash, and (check_fs) already canonical: no symlink on the way,
    and a directory that nobody but its owner can write when it is a share. Refuses by raising StageError."""
    if type(value) is not str or not PATH_RE.fullmatch(value) or "//" in value or (len(value) > 1 and value.endswith("/")):
        raise StageError("share_path_unsafe")
    if any(seg in (".", "..") for seg in value.split("/")[1:]):
        raise StageError("share_path_unsafe")
    if check_fs:
        if os.path.realpath(value) != value:
            raise StageError("share_path_unsafe")
        if directory and not os.path.isdir(value):
            raise StageError("share_path_unsafe")
        if directory and not writable_ok and os.stat(value).st_mode & 0o022:
            raise StageError("share_path_unsafe")
    return value


def traversable(path):
    """Every ancestor of PATH (and PATH itself) can be searched by an unrelated uid: the daemon runs as one."""
    parts = Path(path).parts
    for n in range(1, len(parts) + 1):
        if not os.stat(os.path.join(*parts[:n])).st_mode & 0o001:
            return False
    return True


def safe_tag(tag):
    if type(tag) is not str or not TAG_RE.fullmatch(tag):
        raise StageError("share_path_unsafe")
    return tag


def safe_uid(uid, gid=None):
    for n in (uid, gid if gid is not None else uid):
        if type(n) is not int or isinstance(n, bool) or not 1 <= n <= 2**31 - 2:
            raise StageError("usage")
    return uid


def virtiofsd_argv(binary, socket_path, shared_dir, uid, gid=None):
    """The daemon, as an unprivileged uid with every capability and no_new_privs, exporting ONE read-only directory.
    `--sandbox none`: measured 2026-11-15 on Ubuntu 26.04 as uid 65534 without capabilities, it is the one mode that starts
    (`namespace` was refused with EACCES on the same machine, `chroot` only runs as root). No DAX flag exists in this list and
    the QEMU side never gets `cache-size`, so no host file is mapped into guest-visible memory."""
    safe_path(binary, check_fs=False)
    safe_path(socket_path, check_fs=False)
    if len(socket_path) > SOCKET_PATH_MAX:
        raise StageError("share_path_unsafe")
    safe_path(shared_dir, directory=True)
    uid = safe_uid(uid, gid)
    gid = uid if gid is None else gid
    return [
        "setpriv", "--reuid", str(uid), "--regid", str(gid), "--clear-groups", "--inh-caps=-all", "--ambient-caps=-all",
        "--bounding-set=-all", "--no-new-privs",
        binary, f"--socket-path={socket_path}", f"--shared-dir={shared_dir}", "--readonly", "--sandbox", "none",
        "--cache", "always", "--rlimit-nofile", "0", "--log-level", "warn",
    ]


def qemu_share_args(transport, shared_dir=None, socket_path=None, tag=TAG):
    """The extra QEMU arguments (boot_capture.sh takes them after `--`). Shared guest RAM is already required by kf3
    (boot_nvkvm.sh sets memory-backend-memfd,share=on); vhost-user-fs needs exactly that."""
    tag = safe_tag(tag)
    if transport == "virtiofs":
        safe_path(socket_path, check_fs=False)
        if len(socket_path) > SOCKET_PATH_MAX:
            raise StageError("share_path_unsafe")
        return ["-chardev", f"socket,id=kfllmc,path={socket_path}", "-device", f"vhost-user-fs-pci,chardev=kfllmc,tag={tag}"]
    if transport == "9p":
        safe_path(shared_dir, directory=True)
        return ["-fsdev", f"local,id=kfllm9p,path={shared_dir},security_model=none,readonly=on",
                "-device", f"virtio-9p-pci,fsdev=kfllm9p,mount_tag={tag}"]
    raise StageError("usage")


def guest_mount_command(transport, tag=TAG):
    """The shell line the hook sends to the guest: a fixed mount point, the validated tag, read-only."""
    tag = safe_tag(tag)
    opts = {"virtiofs": "-t virtiofs -o ro", "9p": "-t 9p -o trans=virtio,version=9p2000.L,ro,msize=1048576"}.get(transport)
    if opts is None:
        raise StageError("usage")
    mp = shlex.quote(GUEST_MOUNT)
    return f"sudo mkdir -p {mp} && sudo modprobe {'virtiofs' if transport == 'virtiofs' else '9pnet_virtio'} 2>/dev/null; " \
           f"sudo mount {opts} {shlex.quote(tag)} {mp}"


def choose_transport(requested, devices):
    """`auto` prefers virtio-fs, falls back to 9p only when the QEMU build has no vhost-user-fs device, and refuses when it has
    neither; a named transport is never silently replaced by the other."""
    if requested not in TRANSPORTS:
        raise StageError("usage")
    has = {"virtiofs": "vhost-user-fs-pci" in devices, "9p": "virtio-9p-pci" in devices}
    if requested == "auto":
        for t in ("virtiofs", "9p"):
            if has[t]:
                return t
        raise StageError("no_share_transport")
    if not has[requested]:
        raise StageError("transport_unavailable", requested)
    return requested


def parse_device_list(text):
    return frozenset(re.findall(r'^name "([A-Za-z0-9_.,-]{1,60})"', text, re.M))


# ------------------------------------------------------------------ arm output, maths, verdict

def tok_s(count, duration_ns):
    """Tokens per second from raw counts. None for anything that is not a usable measurement (zero, negative, NaN, a bool)."""
    for v in (count, duration_ns):
        if type(v) is not int or v <= 0:
            return None
    return count * 1e9 / duration_ns


def median_rate(rates):
    if not rates or any(r is None or not math.isfinite(r) or r <= 0 for r in rates):
        return None
    return statistics.median(rates)


def ratio_block(host_rates, guest_rates):
    """guest / host from the medians of the measured runs; INVALID (value null) when either median is not a usable positive
    number. The two targets are facts about the value, never read by the verdict."""
    h, g = median_rate(host_rates), median_rate(guest_rates)
    value = None if h is None or g is None else g / h
    if value is not None and not (math.isfinite(value) and value > 0):
        value = None
    return {
        "state": "OK" if value is not None else "INVALID", "value": None if value is None else round(value, 4),
        "host_tok_s": None if h is None else round(h, 4), "guest_tok_s": None if g is None else round(g, 4),
        "min": RATIO_MIN, "preferred": RATIO_PREFERRED,
        "meets_min": None if value is None else value >= RATIO_MIN,
        "meets_preferred": None if value is None else value >= RATIO_PREFERRED,
        "gate": False,
    }


def _bounded_int(v, lo=0, hi=2**62):
    return type(v) is int and lo <= v <= hi


def parse_arm_output(text, label):
    """The ONE `KF_LLM_ARM_RESULT {json}` line of an arm, validated strictly (the guest's is hostile data). Returns the dict or
    raises StageError(`<label>_arm_failed`)."""
    reason = label + "_arm_failed"
    if len(text) > MAX_ARM_OUTPUT * 64:
        text = text[-MAX_ARM_OUTPUT * 64:]
    found = re.findall(r"^KF_LLM_ARM_RESULT (\{.*\})\s*$", text, re.M)
    if len(found) != 1 or len(found[0]) > MAX_ARM_OUTPUT:
        raise StageError(reason, arm_fail_line(text))
    try:
        doc = json.loads(found[0], parse_constant=lambda _c: (_ for _ in ()).throw(ValueError("constant")), object_pairs_hook=_no_duplicates)
        check_arm(doc, label)
    except (ValueError, TypeError, KeyError, RecursionError):
        raise StageError(reason, "malformed") from None
    return doc


def arm_fail_line(text):
    m = re.search(r"^KF_LLM_ARM_FAIL reason=([a-z0-9_]{1,40})$", text, re.M)
    return m.group(1) if m else "no_result_line"


def check_arm(doc, label):
    keys = {"schema", "label", "ollama_version", "inputs", "create_secs", "load_ms", "params", "warmup_text_sha256", "runs", "text", "ps", "gpu"}
    if type(doc) is not dict or set(doc) != keys or doc["schema"] != 1 or doc["label"] != label:
        raise ValueError("arm shape")
    if type(doc["ollama_version"]) is not str or not VERSION_RE.fullmatch(doc["ollama_version"]):
        raise ValueError("version")
    inputs = doc["inputs"]
    if type(inputs) is not dict or set(inputs) != {"model", "ollama"}:
        raise ValueError("inputs")
    for v in inputs.values():
        if type(v) is not dict or set(v) != {"bytes", "read_secs", "sha256"} or not _bounded_int(v["bytes"], 0, 1 << 42):
            raise ValueError("input")
        if not _finite_number(v["read_secs"]) or type(v["sha256"]) is not str or not HEX64.fullmatch(v["sha256"]):
            raise ValueError("input")
    for key in ("create_secs", "load_ms"):
        if not _finite_number(doc[key]):
            raise ValueError(key)
    p = doc["params"]
    if type(p) is not dict or set(p) != {"num_predict", "seed", "repeats", "prompt_sha256"} or not all(
            _bounded_int(p[k], 0, 10**6) for k in ("num_predict", "seed", "repeats")) or not HEX64.fullmatch(str(p["prompt_sha256"])):
        raise ValueError("params")
    runs = doc["runs"]
    if type(runs) is not list or not 1 <= len(runs) <= 8 or len(runs) != p["repeats"]:
        raise ValueError("runs")
    for r in runs:
        if type(r) is not dict or set(r) != {"eval_count", "eval_duration_ns", "prompt_eval_count", "total_duration_ns", "text_sha256", "token_ids_sha256"}:
            raise ValueError("run")
        if not all(_bounded_int(r[k]) for k in ("eval_count", "eval_duration_ns", "prompt_eval_count", "total_duration_ns")):
            raise ValueError("run ints")
        if not HEX64.fullmatch(str(r["text_sha256"])) or not (r["token_ids_sha256"] is None or HEX64.fullmatch(str(r["token_ids_sha256"]))):
            raise ValueError("run hashes")
    if type(doc["warmup_text_sha256"]) is not str or not HEX64.fullmatch(doc["warmup_text_sha256"]):
        raise ValueError("warmup")
    if type(doc["text"]) is not str or len(doc["text"]) > 200 or type(doc["gpu"]) is not str or len(doc["gpu"]) > 80:
        raise ValueError("text")
    ps = doc["ps"]
    if type(ps) is not dict or set(ps) != {"size", "size_vram"} or not _bounded_int(ps["size"]) or not _bounded_int(ps["size_vram"]):
        raise ValueError("ps")


def _finite_number(v):
    return type(v) in (int, float) and math.isfinite(v) and v >= 0


def arm_record(doc):
    """What the result record keeps of one arm: raw counts, the rates computed HERE, the share read and the GPU residency."""
    rates = [tok_s(r["eval_count"], r["eval_duration_ns"]) for r in doc["runs"]]
    return {
        "ran": True,
        "tok_s": [None if r is None else round(r, 4) for r in rates],
        "eval_count": [r["eval_count"] for r in doc["runs"]],
        "eval_duration_ns": [r["eval_duration_ns"] for r in doc["runs"]],
        "text_sha256": doc["runs"][0]["text_sha256"],
        "load_ms": doc["load_ms"], "create_secs": doc["create_secs"],
        "size": doc["ps"]["size"], "size_vram": doc["ps"]["size_vram"], "gpu": doc["gpu"],
        "share_read": {k: {"bytes": v["bytes"], "read_secs": v["read_secs"],
                           "MBps": round(v["bytes"] / v["read_secs"] / 1e6, 2) if v["read_secs"] > 0 else 0.0}
                       for k, v in doc["inputs"].items()},
    }


def agreement(host, guest):
    """Exact: every measured run of both arms has the same text sha256, the same eval_count, and (when ollama returned token ids
    in all of them) the same token-id hash. There is no looser mode: if the first box run shows drift, a tolerance is defined from
    the measured drift by a reviewed change, not guessed here."""
    runs = host["runs"] + guest["runs"]
    texts = {r["text_sha256"] for r in runs}
    counts = {r["eval_count"] for r in runs}
    ids = [r["token_ids_sha256"] for r in runs]
    compared = all(i is not None for i in ids)
    distinct_ids = len(set(ids)) if compared else 0
    equal = len(texts) == 1 and len(counts) == 1 and (not compared or distinct_ids == 1)
    return {"mode": "exact", "equal": equal, "distinct_texts": len(texts), "token_ids_compared": compared}


def decide(host, guest, pins, transport):
    """(verdict, reason, partial record) for two parsed arms. The ratio is computed and recorded and NEVER read by the verdict."""
    names = TRANSPORT_ITEMS[transport][:2]
    ratio = ratio_block(
        [tok_s(r["eval_count"], r["eval_duration_ns"]) for r in host["runs"]],
        [tok_s(r["eval_count"], r["eval_duration_ns"]) for r in guest["runs"]],
    )
    reason = None
    for arm in (host, guest):
        for name in names:
            if arm["inputs"][name]["sha256"] != pins["items"][name]["sha256"] or arm["inputs"][name]["bytes"] != pins["items"][name]["size"]:
                reason = reason or "inputs_mismatch"
    if reason is None and not (host["ollama_version"] == guest["ollama_version"] == pins["ollama_version"]):
        reason = "version_mismatch"
    if reason is None and host["params"] != guest["params"]:
        reason = "params_differ"
    if reason is None and any(a["ps"]["size"] <= 0 or a["ps"]["size_vram"] != a["ps"]["size"] for a in (host, guest)):
        reason = "not_on_gpu"
    if reason is None and ratio["state"] != "OK":
        reason = "timing_invalid"
    agree = agreement(host, guest)
    if reason is None and not agree["equal"]:
        reason = "text_disagree"
    return ("PASS" if reason is None else "FAIL"), reason, ratio, agree


def build_record(sha, transport, pins, *, host=None, guest=None, reason=None, verdict=None, detail=""):
    """The result record (one JSON line, <= 16 KiB). With both arms it carries the verdict `decide` reached; a refusal or a failed
    arm carries what exists and the reason."""
    names = TRANSPORT_ITEMS.get(transport, ()) if pins else ()
    rec = {
        "schema": SCHEMA, "sha": sha, "verdict": "FAIL", "transport": transport or "none",
        "ollama_version": pins["ollama_version"] if pins and VERSION_RE.fullmatch(str(pins["ollama_version"])) else "unset",
        "items": {n: {"sha256": pins["items"][n]["sha256"], "size": pins["items"][n]["size"]} for n in names
                  if pins["items"][n]["sha256"] != UNSET},
        "arms": {},
    }
    if host is not None:
        rec["arms"]["host"] = arm_record(host)
    if guest is not None:
        rec["arms"]["guest"] = arm_record(guest)
    if host is not None and guest is not None:
        v, r, ratio, agree = decide(host, guest, pins, transport)
        rec["ratio"], rec["agreement"] = ratio, agree
        verdict, reason = (v, r) if reason is None else ("FAIL", reason)
    if reason is not None:
        rec["verdict"] = "FAIL"
        rec["reason"] = reason
    elif verdict == "PASS":
        rec["verdict"] = "PASS"
    if detail:
        rec["detail"] = re.sub(r"[^\x20-\x7e]", "?", detail)[:200]
    validate_record(rec)
    return rec


def validate_record(rec):
    """The schema, shared in spirit with vast_report.validate_llm_parity (a test feeds both the same corpus). Raises ValueError."""
    keys = {"schema", "sha", "verdict", "transport", "ollama_version", "items", "arms", "reason", "ratio", "agreement", "detail"}
    ok = type(rec) is dict and {"schema", "sha", "verdict", "transport", "ollama_version", "items", "arms"} <= set(rec) <= keys
    ok = ok and rec["schema"] == 1 and rec["verdict"] in ("PASS", "FAIL") and type(rec["sha"]) is str and re.fullmatch("[0-9a-f]{40}", rec["sha"]) is not None
    ok = ok and rec["transport"] in ("virtiofs", "9p", "none") and type(rec["ollama_version"]) is str and (
        VERSION_RE.fullmatch(rec["ollama_version"]) is not None)
    ok = ok and type(rec["items"]) is dict and set(rec["items"]) <= set(ITEM_NAMES) and all(
        type(v) is dict and set(v) == {"sha256", "size"} and type(v["sha256"]) is str and HEX64.fullmatch(v["sha256"]) is not None
        and _bounded_int(v["size"], 1, 1 << 42) for v in rec["items"].values())
    ok = ok and type(rec["arms"]) is dict and set(rec["arms"]) <= {"host", "guest"}
    if ok:
        for arm in rec["arms"].values():
            ok = ok and _check_arm_record(arm)
    if ok and "reason" in rec:
        ok = type(rec["reason"]) is str and REASON_RE.fullmatch(rec["reason"]) is not None and rec["verdict"] == "FAIL"
    if ok and rec["verdict"] == "FAIL":
        ok = "reason" in rec
    if ok and "detail" in rec:
        ok = type(rec["detail"]) is str and len(rec["detail"]) <= 200
    if ok and "ratio" in rec:
        r = rec["ratio"]
        ok = type(r) is dict and set(r) == {"state", "value", "host_tok_s", "guest_tok_s", "min", "preferred", "meets_min", "meets_preferred", "gate"}
        ok = ok and r["state"] in ("OK", "INVALID") and r["gate"] is False and r["min"] == RATIO_MIN and r["preferred"] == RATIO_PREFERRED
        ok = ok and all(r[k] is None or _positive(r[k]) for k in ("value", "host_tok_s", "guest_tok_s"))
        ok = ok and (r["state"] == "OK") == (r["value"] is not None)
        ok = ok and all(r[k] is None or type(r[k]) is bool for k in ("meets_min", "meets_preferred"))
    if ok and "agreement" in rec:
        a = rec["agreement"]
        ok = type(a) is dict and set(a) == {"mode", "equal", "distinct_texts", "token_ids_compared"} and a["mode"] == "exact"
        ok = ok and type(a["equal"]) is bool and _bounded_int(a["distinct_texts"], 0, 64) and type(a["token_ids_compared"]) is bool
    if ok and rec["verdict"] == "PASS":
        ok = set(rec["arms"]) == {"host", "guest"} and "reason" not in rec and rec.get("ratio", {}).get("state") == "OK" and (
            rec.get("agreement", {}).get("equal") is True)
    if ok and len(json.dumps(rec, separators=(",", ":")).encode()) > MAX_RECORD_BYTES:
        ok = False
    if not ok:
        raise ValueError("llm_parity record")


def _positive(v):
    return type(v) in (int, float) and math.isfinite(v) and v > 0


def _check_arm_record(a):
    keys = {"ran", "tok_s", "eval_count", "eval_duration_ns", "text_sha256", "load_ms", "create_secs", "size", "size_vram", "gpu", "share_read"}
    if type(a) is not dict or set(a) != keys or a["ran"] is not True:
        return False
    for k in ("tok_s", "eval_count", "eval_duration_ns"):
        if type(a[k]) is not list or not 1 <= len(a[k]) <= 8:
            return False
    n = len(a["tok_s"])
    ok = len(a["eval_count"]) == len(a["eval_duration_ns"]) == n
    ok = ok and all(r is None or _positive(r) for r in a["tok_s"])
    ok = ok and all(_bounded_int(v) for v in a["eval_count"] + a["eval_duration_ns"])
    ok = ok and type(a["text_sha256"]) is str and HEX64.fullmatch(a["text_sha256"]) is not None
    ok = ok and _finite_number(a["load_ms"]) and _finite_number(a["create_secs"]) and _bounded_int(a["size"]) and _bounded_int(a["size_vram"])
    ok = ok and type(a["gpu"]) is str and len(a["gpu"]) <= 80
    sr = a["share_read"]
    ok = ok and type(sr) is dict and set(sr) == {"model", "ollama"} and all(
        type(v) is dict and set(v) == {"bytes", "read_secs", "MBps"} and _bounded_int(v["bytes"], 0, 1 << 42)
        and _finite_number(v["read_secs"]) and _finite_number(v["MBps"]) for v in sr.values())
    return ok


# ------------------------------------------------------------------ preflight and plumbing

def kf3_binary(checkout, bench="/workspace/bench"):
    """The per-revision kf3 binary build_kf3.sh installs (boot_nvkvm.sh selects the same one)."""
    try:
        out = subprocess.run(["git", "-C", str(checkout), "rev-parse", "--short=8", "HEAD"], capture_output=True, text=True, timeout=30)
    except (OSError, subprocess.SubprocessError):
        raise StageError("no_kf3_binary", "git") from None
    rev = out.stdout.strip()
    if out.returncode != 0 or not re.fullmatch("[0-9a-f]{8}", rev):
        raise StageError("no_kf3_binary")
    path = Path(bench) / "kf3-bins" / rev / "qemu-system-x86_64"
    if not os.access(path, os.X_OK):
        raise StageError("no_kf3_binary", str(path))
    return path


def device_list(qemu):
    try:
        out = subprocess.run([str(qemu), "-device", "help"], capture_output=True, text=True, timeout=60)
    except (OSError, subprocess.SubprocessError):
        raise StageError("no_kf3_binary", "device help") from None
    return parse_device_list(out.stdout + out.stderr)


def build_share(dl_root, share_dir, pins):
    """A directory holding ONLY the model and the ollama tarball, as regular read-only files (hard links when the filesystem
    allows, copies otherwise); the downloader's own state directory and the virtiofsd binary never enter it."""
    share = Path(share_dir)
    try:
        share.mkdir(mode=0o755, parents=False)
        for name, target in SHARE_FILES.items():
            src = Path(dl_root) / pins["items"][name]["dest"]
            if not src.is_file() or src.is_symlink():
                raise OSError("source")
            dst = share / target
            try:
                os.link(src, dst)
            except OSError:
                shutil.copyfile(src, dst)
            os.chmod(dst, 0o444)
        os.chmod(share, 0o555)
    except OSError:
        raise StageError("share_build_failed") from None
    return share


def fetch_items(manifest_doc, dest, args, say):
    """Run the box downloader on the manifest (a file under DEST's parent) and wait for it; the downloader prints its own KF_STEP
    lines. Nothing here reads a token or a URL."""
    Path(dest).parent.mkdir(parents=True, exist_ok=True)
    mpath = Path(dest).parent / "llm_parity_manifest.json"
    mpath.write_text(json.dumps(manifest_doc, indent=1) + "\n")
    cmd = [sys.executable, "-I", str(HERE / "box_downloader.py"), "fetch", str(mpath), "--dest", str(dest)]
    for d in args.pin_dir:
        cmd += ["--pin-dir", d]
    if args.urls:
        cmd += ["--urls", args.urls]
    if args.s3_config:
        cmd += ["--s3-config", args.s3_config]
    if args.limit_bandwidth is not None:
        cmd += ["--limit-bandwidth", str(args.limit_bandwidth)]
    say("KF_LLM_PARITY_FETCH items=" + ",".join(i["name"] for i in manifest_doc["items"]))
    try:
        done = subprocess.run(cmd, timeout=3 * 3600)
    except subprocess.TimeoutExpired:
        raise StageError("download_failed", "timeout") from None
    if done.returncode != 0:
        raise StageError("download_failed", "rc=%d" % done.returncode)


def run_host_arm(share_dir, work, say, timeout=3600):
    cmd = [sys.executable, "-I", str(HERE / "llm_parity_arm.py"), "--label", "host", "--tarball", f"{share_dir}/{SHARE_FILES['ollama']}",
           "--model", f"{share_dir}/{SHARE_FILES['model']}", "--work", f"{work}/arm_host", "--port", str(HOST_PORT)]
    try:
        done = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        raise StageError("host_arm_failed", "timeout") from None
    for line in done.stdout.splitlines():
        if line.startswith(("KF_LLM_SHARE_READ ", "KF_LLM_ARM_FAIL ", "KF_LLM_ARM_TAIL ")):
            say(line)
    if subprocess.run(["pgrep", "-x", "ollama"], capture_output=True).returncode == 0:
        raise StageError("host_server_left_running")
    return done.stdout


def start_virtiofsd(binary, sock_dir, shared_dir, uid, log_path):
    sock_dir = Path(sock_dir)
    sock_dir.mkdir(mode=0o700)
    if os.geteuid() == 0:
        os.chown(sock_dir, uid, uid)
    sock = f"{sock_dir}/{SOCKET_NAME}"
    argv = virtiofsd_argv(binary, sock, shared_dir, uid)
    with open(log_path, "wb") as log:
        proc = subprocess.Popen(argv, stdout=log, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL, start_new_session=True)
    deadline = time.monotonic() + VFSD_SOCKET_WAIT_S
    while time.monotonic() < deadline:
        if proc.poll() is not None:
            raise StageError("virtiofsd_failed", "exited rc=%s" % proc.returncode)
        if os.path.exists(sock):
            return proc, sock
        time.sleep(0.2)
    proc.kill()
    raise StageError("virtiofsd_failed", "no socket")


def stop_process(proc):
    if proc is None or proc.poll() is not None:
        return
    proc.terminate()
    try:
        proc.wait(timeout=15)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait()


def run_guest_arm(checkout, work, share_dir, transport, vfsd_binary, uid, tag, say, boot_timeout=3600):
    """Start the share daemon (virtio-fs), boot the Linux guest through boot_capture.sh with the share arguments and this
    module's hook, and return the guest arm's raw output from the file the hook wrote."""
    work = Path(work)
    cfg_path, out_path = work / "guest_cfg.json", work / "guest_arm.out"
    cfg = {"transport": transport, "tag": TAG, "port": GUEST_PORT, "out": str(out_path), "guest_work": GUEST_WORK,
           "tarball": f"{GUEST_MOUNT}/{SHARE_FILES['ollama']}", "model": f"{GUEST_MOUNT}/{SHARE_FILES['model']}",
           "arm": str(HERE / "llm_parity_arm.py"), "gssh": str(Path(checkout) / "scripts/bench/gssh_nv")}
    cfg_path.write_text(json.dumps(cfg))
    proc, sock = None, None
    try:
        if transport == "virtiofs":
            proc, sock = start_virtiofsd(vfsd_binary, work / "vfs", share_dir, uid, work / "virtiofsd.log")
        args = qemu_share_args(transport, shared_dir=str(share_dir), socket_path=sock)
        env = dict(os.environ, KF_DEVICE="kf3", NVKVM_RAM_MB="8192", KF_SMP="4", GQ_TIMEOUT="900",
                   POST_CAPTURE_HOOK=str(HERE / "llm_parity_hook.sh"), LLM_PARITY_HOOK_CFG=str(cfg_path))
        say("KF_LLM_PARITY_BOOT tag=%s transport=%s" % (tag, transport))
        with open(work / "guest_boot.log", "wb") as log:
            try:
                done = subprocess.run(["bash", str(Path(checkout) / "scripts/bench/boot_capture.sh"), tag, "--", *args], env=env,
                                      stdout=log, stderr=subprocess.STDOUT, timeout=boot_timeout)
            except subprocess.TimeoutExpired:
                raise StageError("boot_failed", "timeout") from None
        if done.returncode != 0 and not out_path.exists():
            raise StageError("boot_failed", "rc=%d" % done.returncode)
    finally:
        stop_process(proc)
    if not out_path.exists():
        raise StageError("guest_no_result")
    return out_path.read_bytes()[: MAX_ARM_OUTPUT * 64].decode("utf-8", "replace")


# ------------------------------------------------------------------ the guest hook (runs on the host, talks to the guest by ssh)

def guest_ssh(gssh, remote, stdin=None, timeout=300):
    return subprocess.run([gssh, remote], input=stdin, capture_output=True, timeout=timeout)


def guest_hook(cfg_path):
    """Mount the share in the guest read-only, prove it is read-only and virtiofs/9p, check the disk, send the arm over ssh and
    store its stdout in cfg['out']. Exit 0 whenever it wrote the file (the orchestrator judges the content)."""
    cfg = json.loads(Path(cfg_path).read_text())
    gssh, out = cfg["gssh"], Path(cfg["out"])
    transport = cfg["transport"]
    notes = []
    if guest_ssh(gssh, "true", timeout=60).returncode != 0:
        out.write_text("KF_LLM_ARM_FAIL reason=guest_unreachable\n")
        return 0
    mount = guest_ssh(gssh, guest_mount_command(transport, cfg["tag"]), timeout=120)
    check = guest_ssh(gssh, f"findmnt -no FSTYPE,OPTIONS {shlex.quote(GUEST_MOUNT)}; "
                            f"touch {shlex.quote(GUEST_MOUNT)}/.kf-write-probe 2>&1 && echo WRITABLE || echo READONLY", timeout=60)
    text = check.stdout.decode("utf-8", "replace")
    want = {"virtiofs": "virtiofs", "9p": "9p"}[transport]
    first = text.splitlines()[0].split() if text.splitlines() else []
    notes.append("KF_LLM_SHARE_MOUNT transport=%s fstype=%s ro=%d writable_probe=%s mount_rc=%d" % (
        transport, (first[0] if first else "none")[:12], int(len(first) > 1 and "ro" in first[1].split(",")),
        "yes" if "WRITABLE" in text else "no", mount.returncode))
    if not first or first[0] != want or "ro" not in (first[1].split(",") if len(first) > 1 else []) or "WRITABLE" in text:
        out.write_text("\n".join(notes) + "\nKF_LLM_ARM_FAIL reason=guest_mount\n")
        return 0
    disk = guest_ssh(gssh, f"mkdir -p {shlex.quote(cfg['guest_work'])} && df -B1 --output=avail {shlex.quote(cfg['guest_work'])} | tail -1", timeout=60)
    try:
        free = int(disk.stdout.decode().split()[0])
    except (ValueError, IndexError):
        free = 0
    notes.append("KF_LLM_GUEST_DISK avail_bytes=%d" % free)
    if free < MIN_FREE_BYTES:
        out.write_text("\n".join(notes) + "\nKF_LLM_ARM_FAIL reason=guest_disk\n")
        return 0
    remote = "python3 -I - --label guest --tarball %s --model %s --work %s --port %d" % (
        shlex.quote(cfg["tarball"]), shlex.quote(cfg["model"]), shlex.quote(cfg["guest_work"] + "/run"), cfg["port"])
    # a stale run directory would make the arm's makedirs refuse; remove it first (it is the arm's own scratch)
    guest_ssh(gssh, f"sudo rm -rf {shlex.quote(cfg['guest_work'] + '/run')}", timeout=120)
    arm = guest_ssh(gssh, remote, stdin=Path(cfg["arm"]).read_bytes(), timeout=3600)
    out.write_text("\n".join(notes) + "\n" + arm.stdout.decode("utf-8", "replace")[-MAX_ARM_OUTPUT * 8:])
    return 0


# ------------------------------------------------------------------ the CLI

def say(line):
    print(line, flush=True)


def plan_lines(pins, transport):
    names = TRANSPORT_ITEMS["virtiofs" if transport == "auto" else transport]
    lines = ["KF_LLM_PARITY_PLAN transport=%s items=%s unpinned=%s targets=min:%.2f,preferred:%.2f gate=none" % (
        transport, ",".join(names), ",".join(unpinned(pins, names)) or "none", RATIO_MIN, RATIO_PREFERRED)]
    for n in names:
        i = pins["items"][n]
        lines.append("KF_LLM_PARITY_ITEM name=%s sha256=%s size=%s sources=%d" % (n, i["sha256"], i["size"], len(i["sources"])))
    return lines


def emit_record(rec, out, sha):
    line = json.dumps(rec, separators=(",", ":"), sort_keys=True)
    try:
        Path(out).write_text(line + "\n")
    except OSError:
        pass
    say("KF_LLM_PARITY_RESULT " + line)
    say("KF_LLM_PARITY_VERDICT %s reason=%s ratio=%s" % (
        rec["verdict"], rec.get("reason", "none"), (rec.get("ratio") or {}).get("value")))
    return 0 if rec["verdict"] == "PASS" else 1


def clean_scratch(work):
    """Remove the previous attempt's own scratch (never the downloader's directory, which resumes) before a rerun."""
    for name in ("share", "vfs", "arm_host"):
        path = Path(work) / name
        if path.is_symlink() or path.is_file():
            path.unlink()
        elif path.is_dir():
            for root, dirs, files in os.walk(path):
                for n in dirs + files:
                    try:
                        os.chmod(os.path.join(root, n), 0o700, follow_symlinks=False)
                    except (OSError, NotImplementedError):
                        pass
            os.chmod(path, 0o700)
            shutil.rmtree(path)


def cmd_run(args, steps=None):
    """`steps` replaces the box-bound steps in tests: preflight, fetch, build_share, host_arm, guest_arm. Always ends by printing
    one result record, also for a refusal (the payload keeps it)."""
    steps = steps or {}
    sha = args.sha if re.fullmatch("[0-9a-f]{40}", args.sha or "") else "0" * 40
    pins = transport = host = guest = None
    try:
        pins = load_pins(args.pins)
        if sha != args.sha:
            raise StageError("usage")
        for p in (args.checkout, args.work):
            safe_path(p, check_fs=False)
        early = unpinned(pins, ("model", "ollama"))  # needed by both transports: refuse before anything else is looked at
        if early:
            raise StageError("unpinned", ",".join(early))
        transport = (steps.get("preflight") or preflight)(args, pins)
        names = TRANSPORT_ITEMS[transport]
        manifest = build_manifest(pins, names)
        work = Path(args.work)
        work.mkdir(mode=0o755, parents=True, exist_ok=True)
        clean_scratch(work)
        if not traversable(work):
            raise StageError("share_not_traversable")
        if shutil.disk_usage(work).free < MIN_FREE_BYTES + sum(pins["items"][n]["size"] for n in names):
            raise StageError("disk_space")
        dl = work / "dl"
        (steps.get("fetch") or fetch_items)(manifest, dl, args, say)
        share = (steps.get("build_share") or build_share)(dl, work / "share", pins)
        host = parse_arm_output((steps.get("host_arm") or run_host_arm)(share, work, say), "host")
        guest_text = (steps.get("guest_arm") or run_guest_arm_default)(args, transport, work, share, pins)
        for line in guest_text.splitlines():
            if line.startswith(("KF_LLM_SHARE_READ ", "KF_LLM_SHARE_MOUNT ", "KF_LLM_GUEST_DISK ", "KF_LLM_ARM_FAIL ")):
                say(line[:300])
        guest = parse_arm_output(guest_text, "guest")
        rec = build_record(sha, transport, pins, host=host, guest=guest)
    except StageError as error:
        say("KF_LLM_PARITY_REFUSED reason=" + error.reason)
        rec = build_record(sha, transport, pins, host=host, guest=guest, reason=error.reason, detail=error.detail)
    except Exception as error:  # a bug here must still leave one record in the log and the result file
        say("KF_LLM_PARITY_REFUSED reason=internal_error")
        rec = build_record(sha, transport, pins, host=host, guest=guest, reason="internal_error", detail=type(error).__name__)
    return emit_record(rec, args.out, sha)


def run_guest_arm_default(args, transport, work, share, pins):
    tag = "lp" + os.urandom(6).hex()
    vfsd = Path(work) / "dl" / pins["items"]["virtiofsd"]["dest"] if transport == "virtiofs" else None
    if vfsd is not None:
        if not vfsd.is_file() or vfsd.is_symlink() or not traversable(vfsd.parent):
            raise StageError("share_not_traversable")  # the daemon runs as another uid and has to reach its own binary
        os.chmod(vfsd, 0o755)
    return run_guest_arm(args.checkout, work, share, transport, str(vfsd) if vfsd else None, args.vfsd_uid, tag, say)


def preflight(args, pins):
    """Everything that can be refused before a byte is downloaded: root, pins for the chosen transport, the kf3 binary and the
    devices it was built with."""
    if os.geteuid() != 0:
        raise StageError("not_root")
    safe_uid(args.vfsd_uid)
    qemu = kf3_binary(args.checkout)
    transport = choose_transport(args.transport, device_list(qemu))
    missing = unpinned(pins, TRANSPORT_ITEMS[transport])
    if missing:
        raise StageError("unpinned", ",".join(missing))
    return transport


def build_parser():
    p = argparse.ArgumentParser(prog="llm_parity.py", description=__doc__.split("\n\n")[0])
    sub = p.add_subparsers(dest="cmd", required=True)
    for name in ("plan", "check", "run"):
        s = sub.add_parser(name)
        s.add_argument("--pins", default=str(DEFAULT_PINS))
        s.add_argument("--transport", default="auto", choices=TRANSPORTS)
    r = sub.choices["run"]
    r.add_argument("--sha", required=True)
    r.add_argument("--checkout", required=True)
    r.add_argument("--work", required=True)
    r.add_argument("--out", required=True)
    r.add_argument("--pin-dir", action="append", default=[])
    r.add_argument("--urls")
    r.add_argument("--s3-config")
    r.add_argument("--vfsd-uid", type=int, default=DEFAULT_VFSD_UID)
    r.add_argument("--limit-bandwidth", type=float)
    sub.add_parser("guest-hook").add_argument("tag")
    return p


def main(argv=None):
    args = build_parser().parse_args(argv)
    try:
        if args.cmd == "guest-hook":
            return guest_hook(os.environ["LLM_PARITY_HOOK_CFG"])
        if args.cmd == "run":
            return cmd_run(args)
        pins = load_pins(args.pins)
        for line in plan_lines(pins, args.transport):
            say(line)
        if args.cmd == "check":
            missing = unpinned(pins, TRANSPORT_ITEMS["virtiofs" if args.transport == "auto" else args.transport])
            if missing:
                say("KF_LLM_PARITY_REFUSED reason=unpinned items=" + ",".join(missing))
                return 3
        return 0
    except StageError as error:
        say("KF_LLM_PARITY_REFUSED reason=" + error.reason)
        return 2
    except KeyError:
        say("KF_LLM_PARITY_REFUSED reason=usage")
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
