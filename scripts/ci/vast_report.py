"""Repo-side bounded reporter. Per-run capabilities only; no account authority."""

import json, math, os, re, select, subprocess, time

url = os.environ["KF_BROKER_URL"] + "/report/" + os.environ["KF_RUN_ID"]
token = os.environ["KF_RUN_TOKEN"]
nonce = os.environ["KF_NONCE"]
runner_nonce = os.environ.get("KF_RUNNER_NONCE", "")
sha = os.environ["KF_SHA"]
grace = int(os.environ["KF_GRACE_FAIL"]) // 60
seq = 0
sent = 0
attempted = 0
omitted = 0
backup = "/workspace/kf-ci-output.log"


def post(kind, text=None, result=None):
    global seq, sent, attempted, omitted
    payload = {"seq": seq + 1, "kind": kind}
    if text is not None:
        payload["text"] = text
    if result is not None:
        payload["result"] = result
    body = json.dumps(payload, separators=(",", ":"), ensure_ascii=True).encode()
    heartbeat_post = kind == "log" and text == ""
    if kind != "result" and not heartbeat_post and (attempted >= 600 or sent + len(body) > 900000):
        omitted += len((text or "").encode())
        return
    seq += 1
    sent += len(body)
    attempted += 1
    # curl retries reuse this exact payload/sequence; no redirect or arbitrary URL.
    response = subprocess.run(
        [
            "curl",
            "--silent",
            "--show-error",
            "--fail",
            "--max-time",
            "10",
            "--retry",
            "2",
            "--retry-delay",
            "1",
            "--retry-all-errors",
            "--proto",
            "=https",
            "--request",
            "POST",
            "--header",
            "Content-Type: application/json",
            "--header",
            "Authorization: Bearer " + token,
            "--data-binary",
            "@-",
            url,
        ],
        input=body,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    if kind == "result" and response.returncode == 0:
        try:
            return json.loads(response.stdout)
        except (ValueError, TypeError):
            return None


work = r"""exec timeout --signal=TERM --kill-after=30s __KF_SOFT_SECONDS__s bash -s <<'KF_TEST'
set -euo pipefail
find /workspace -maxdepth 1 -name kf-ci-result.json -delete
cd /workspace/kf-ci-src
export KF_SOFT_DEADLINE_MS=__KF_SOFT_DEADLINE_MS__
bash scripts/ci/vast_payload.sh "$KF_SHA" "$KF_NONCE"
KF_TEST
"""
soft_deadline = int(os.environ["KF_SOFT_DEADLINE"]) * 1000
remaining = math.floor(soft_deadline / 1000 - time.time()) if soft_deadline is not None else 0
work = (
    work.replace("__KF_SOFT_SECONDS__", str(max(1, remaining))) if remaining > 0 else "exit 124\n"
)
work = work.replace(
    "__KF_SOFT_DEADLINE_MS__",
    str(
        soft_deadline
        if soft_deadline is not None
        else math.floor(time.time() * 1000 + remaining * 1000)
    ),
)
post("progress", "Suite starting")
child = subprocess.Popen(
    ["bash", "-s"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT
)
child.stdin.write(work.encode())
child.stdin.close()
pending = b""
last = time.monotonic()
heartbeat = last
with open(backup, "wb") as local:
    while True:
        ready, _, _ = select.select([child.stdout], [], [], 1)
        if ready:
            data = os.read(child.stdout.fileno(), 8192)
            if not data:
                break
            local.write(data)
            local.flush()
            pending += data
        current = time.monotonic()
        if pending and (len(pending) >= 8192 or current - last >= 1):
            post("log", pending.decode("utf-8", errors="replace"))
            pending = b""
            last = current
        if current - heartbeat >= int(os.environ["KF_HEARTBEAT_SECONDS"]):
            post("log", "")
            heartbeat = current
    if pending:
        post("log", pending.decode("utf-8", errors="replace"))
rc = child.wait()
with open(backup, "rb") as local:
    local.seek(0, os.SEEK_END)
    local.seek(max(0, local.tell() - 8192))
    tail = local.read(8192).decode("utf-8", errors="replace")
# A process exit or log text cannot supply a GPU verdict. Consume only the
# bounded structured file written by the approved payload, binding SHA + nonce.
unknown = {"gpu_name": "unknown", "compute_cap": 0, "driver_version": "unknown"}
result = {
    "nonce": nonce,
    "sha": sha,
    "verdict": "FAIL",
    "suites": [{"name": "infrastructure", "pass": 0, "fail": 1, "skip": 0}],
    "hardware": unknown,
}


def unique(items):
    out = {}
    for k, v in items:
        if k in out:
            raise ValueError("duplicate field")
        out[k] = v
    return out


try:
    with open("/workspace/kf-ci-result.json", "rb") as source:
        raw = source.read(65537)
    if len(raw) > 65536:
        raise ValueError("size")
    candidate = json.loads(raw.decode("utf-8", errors="strict"), object_pairs_hook=unique)
    if (
        type(candidate) is not dict
        or set(candidate) != {"nonce", "sha", "verdict", "suites", "hardware"}
        or candidate["nonce"] != nonce
        or candidate["sha"] != sha
        or candidate["verdict"] not in {"PASS", "FAIL", "UNAVAILABLE"}
    ):
        raise ValueError("binding")
    rows = candidate["suites"]
    hardware = candidate["hardware"]
    if (
        type(rows) is not list
        or not 1 <= len(rows) <= 19
        or any(
            type(r) is not dict
            or set(r) != {"name", "pass", "fail", "skip"}
            or not isinstance(r["name"], str)
            or not re.fullmatch(r"[a-z][a-z0-9_]{0,47}", r["name"])
            or any(
                type(r[k]) is not int or not 0 <= r[k] <= 10000000 for k in ["pass", "fail", "skip"]
            )
            for r in rows
        )
        or len({r["name"] for r in rows}) != len(rows)
    ):
        raise ValueError("suites")
    if (
        type(hardware) is not dict
        or set(hardware) != {"gpu_name", "compute_cap", "driver_version"}
        or not isinstance(hardware["gpu_name"], str)
        or not re.fullmatch(r"[A-Za-z0-9_. -]{1,100}", hardware["gpu_name"])
        or type(hardware["compute_cap"]) is not int
        or not 0 <= hardware["compute_cap"] <= 2000
        or not isinstance(hardware["driver_version"], str)
        or not re.fullmatch(
            r"unknown|[0-9]{3}\.[0-9]{1,3}(?:\.[0-9]{1,2})?", hardware["driver_version"]
        )
    ):
        raise ValueError("hardware")
    complete = all(r["fail"] == 0 and r["skip"] == 0 and r["pass"] > 0 for r in rows)
    if (
        rc != 0
        or candidate["verdict"] != "PASS"
        or not complete
        or hardware["gpu_name"] == "unknown"
        or hardware["driver_version"] == "unknown"
        or hardware["compute_cap"] < 750
    ):
        candidate["verdict"] = "FAIL"
        if not any(r["fail"] for r in rows):
            rows.append(
                {"name": "timeout" if rc == 124 else "app_exit", "pass": 0, "fail": 1, "skip": 0}
            )
    result = candidate
except (ValueError, TypeError, KeyError, OSError, UnicodeError):
    pass
if rc == 124 and not any(r["name"] == "timeout" for r in result["suites"]):
    result["suites"].append({"name": "timeout", "pass": 0, "fail": 1, "skip": 0})
if runner_nonce:
    result["runner_nonce"] = runner_nonce
ack = post(
    "result",
    "Suite complete; sender omitted "
    + str(omitted)
    + " raw bytes beyond its bounded transport allowance.",
    result,
)
seconds = ack.get("grace_remaining_seconds") if isinstance(ack, dict) else None
remaining = (
    str(math.ceil(seconds / 60))
    if type(seconds) is int and 0 <= seconds <= grace * 60
    else "unknown"
)
print(
    "Use kf-box-kill [reason] to end this box early; configured maximum "
    + str(grace)
    + " minutes; Worker reports "
    + remaining
    + " minutes remaining (hard TTL included)."
)
raise SystemExit(rc if rc else (0 if result["verdict"] == "PASS" else 1))
