#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""One arm of the llm_parity stage: the SAME file runs on the host and inside the Linux guest (stdlib only, Python 3.8+).

STATUS: LIVE as code, 2026-11-15; NOT YET RUN ON A GPU BOX, and no real ollama has ever run through it (the GPU-free tests use a
fake `ollama` that speaks the same four HTTP calls). Design: docs/design/V3_LLM_PARITY_STAGE.md.

It exists as ONE file because a parity ratio compares two measurements and two copies of a runner drift in prompt, token count and
what the timer encloses (scripts/bench/provision_host_llm.sh learned that the hard way). The orchestrator (scripts/ci/llm_parity.py)
runs it directly for the host arm and sends it over ssh to `python3 -I -` for the guest arm.

    llm_parity_arm.py --label host|guest --tarball FILE --model FILE --work DIR [--port N] [--repeats N] [--timeout SECS]

What it does, in order (any failure ends it with `KF_LLM_ARM_FAIL reason=<name>`, exit 1):
  1. reads the model and the ollama tarball once, timed, hashing both (`KF_LLM_SHARE_READ ...` lines: this is the virtio-fs
     throughput check, and the sha256 values let the orchestrator compare what this side saw with the pinned values);
  2. unpacks the tarball into DIR with a member filter (no absolute name, no `..`, no device, no link that leaves DIR);
  3. refuses if something already answers on 127.0.0.1:PORT (it would measure somebody else's server);
  4. starts `bin/ollama serve` (own HOME and model directory, 127.0.0.1 only), imports the model with `ollama create`
     from a Modelfile (`FROM <model>`; temperature 0, top_k 1, fixed seed), then one warm-up generate and REPEATS measured ones
     (greedy, fixed prompt, NUM_PREDICT tokens), then asks /api/ps how much of the model sits in GPU memory;
  5. stops the server (process group) and prints ONE line `KF_LLM_ARM_RESULT {json}` with raw counts and nanoseconds (the
     orchestrator computes every rate itself; nothing here is a verdict).
"""
import argparse
import hashlib
import json
import math
import os
import re
import shutil
import signal
import socket
import subprocess
import tarfile
import time
import urllib.error
import urllib.request

SCHEMA = 1
PROMPT = "Explain, in plain words, what a GPU does when it runs one kernel. Answer:"
NUM_PREDICT = 128
SEED = 1
NUM_CTX = 2048
REPEATS = 3
MODEL_TAG = "kf-parity"
MAX_UNPACK_BYTES = 12 << 30
MAX_MEMBERS = 20000
MAX_HTTP_BYTES = 1 << 20
CHUNK = 4 << 20
RESULT_PREFIX = "KF_LLM_ARM_RESULT "
LABEL_RE = re.compile(r"(host|guest)")
PATH_RE = re.compile(r"/[A-Za-z0-9._@+/-]{1,2000}")  # no space, newline or quote: a path is written into the Modelfile


class ArmFail(Exception):
    def __init__(self, reason, detail=""):
        self.reason = reason
        self.detail = detail
        super().__init__(reason)


def prompt_sha256():
    return hashlib.sha256(PROMPT.encode()).hexdigest()


def finite(value):
    return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value)


# ------------------------------------------------------------------ reading and unpacking

def read_checked(path, clock=time.perf_counter):
    """(bytes, seconds spent in read(), sha256) of one file read start to end. Hashing time is not counted."""
    h = hashlib.sha256()
    total, spent = 0, 0.0
    with open(path, "rb", buffering=0) as f:
        while True:
            t0 = clock()
            block = f.read(CHUNK)
            spent += clock() - t0
            if not block:
                break
            total += len(block)
            h.update(block)
    return total, spent, h.hexdigest()


def member_ok(member, dest):
    """A tarball member is acceptable when it is a regular file, a directory, or a symlink that stays inside DEST; its
    name is relative and has no `..`. Hard links, devices, fifos and anything else are refused."""
    name = member.name
    if not name or name.startswith("/") or "\0" in name or any(part == ".." for part in name.split("/")):
        return False
    target = os.path.normpath(os.path.join(dest, name))
    if target != dest and not target.startswith(dest + os.sep):
        return False
    if member.isdir() or member.isreg():
        return True
    if member.issym():
        link = member.linkname
        if not link or "\0" in link:
            return False
        resolved = os.path.normpath(os.path.join(os.path.dirname(target), link))
        return not os.path.isabs(link) and (resolved == dest or resolved.startswith(dest + os.sep))
    return False


def unpack(tarball, dest):
    """Extract with the member filter above and the size and count caps; returns the path of bin/ollama."""
    try:
        os.makedirs(dest, mode=0o700)
    except FileExistsError:
        raise ArmFail("work_exists") from None
    except OSError:
        raise ArmFail("work_unwritable") from None
    dest = os.path.realpath(dest)
    total = 0
    try:
        with tarfile.open(tarball, "r:*") as tar:
            members = []
            for member in tar:
                members.append(member)
                if len(members) > MAX_MEMBERS:
                    raise ArmFail("tar_unsafe", "members")
                if not member_ok(member, dest):
                    raise ArmFail("tar_unsafe", "member")
                total += max(member.size, 0)
                if total > MAX_UNPACK_BYTES:
                    raise ArmFail("tar_unsafe", "size")
            for member in members:
                target = os.path.join(dest, member.name)
                if member.isdir():
                    os.makedirs(target, mode=0o755, exist_ok=True)
                elif member.issym():
                    os.makedirs(os.path.dirname(target), mode=0o755, exist_ok=True)
                    os.symlink(member.linkname, target)
                else:
                    os.makedirs(os.path.dirname(target), mode=0o755, exist_ok=True)
                    src = tar.extractfile(member)
                    with open(target, "wb") as out:
                        shutil.copyfileobj(src, out, CHUNK)
                    os.chmod(target, member.mode & 0o755 | 0o400)
    except (tarfile.TarError, OSError, EOFError) as error:
        raise ArmFail("tar_unreadable", type(error).__name__) from None
    binary = os.path.join(dest, "bin", "ollama")
    if not os.path.isfile(binary) or not os.access(binary, os.X_OK):
        raise ArmFail("tar_no_binary")
    return binary


# ------------------------------------------------------------------ the server

def answers(port):
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=1):
            return True
    except OSError:
        return False


def http_json(port, path, body=None, timeout=300):
    data = None if body is None else json.dumps(body).encode()
    request = urllib.request.Request(f"http://127.0.0.1:{port}{path}", data=data,
                                     headers={"Content-Type": "application/json"} if data else {})
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            raw = response.read(MAX_HTTP_BYTES + 1)
    except (urllib.error.URLError, OSError, ValueError) as error:
        raise ArmFail("http_failed", type(error).__name__) from None
    if len(raw) > MAX_HTTP_BYTES:
        raise ArmFail("http_too_large")
    try:
        doc = json.loads(raw.decode("utf-8"), parse_constant=lambda _c: (_ for _ in ()).throw(ValueError()))
    except (ValueError, UnicodeError):
        raise ArmFail("http_not_json") from None
    if not isinstance(doc, dict):
        raise ArmFail("http_not_json")
    return doc


def server_env(work, port):
    keep = {k: os.environ[k] for k in ("PATH", "LANG", "LD_LIBRARY_PATH", "CUDA_VISIBLE_DEVICES") if k in os.environ}
    keep.update(
        HOME=os.path.join(work, "home"), OLLAMA_HOST=f"127.0.0.1:{port}", OLLAMA_MODELS=os.path.join(work, "models"),
        OLLAMA_NUM_PARALLEL="1", OLLAMA_MAX_LOADED_MODELS="1", OLLAMA_KEEP_ALIVE="30m", OLLAMA_NOPRUNE="1",
        OLLAMA_NOHISTORY="1", OLLAMA_DEBUG="0",
    )
    return keep


class Server:
    def __init__(self, binary, work, port):
        self.binary, self.work, self.port = binary, work, port
        self.proc = None
        self.log = os.path.join(work, "serve.log")

    def start(self, timeout):
        os.makedirs(os.path.join(self.work, "home"), mode=0o700, exist_ok=True)
        os.makedirs(os.path.join(self.work, "models"), mode=0o700, exist_ok=True)
        if answers(self.port):
            raise ArmFail("port_busy")
        with open(self.log, "wb") as log:
            self.proc = subprocess.Popen([self.binary, "serve"], env=server_env(self.work, self.port), stdout=log,
                                         stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL, start_new_session=True)
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if self.proc.poll() is not None:
                raise ArmFail("serve_exited", self.tail())
            try:
                http_json(self.port, "/api/version", timeout=2)
                return
            except ArmFail:
                time.sleep(0.5)
        raise ArmFail("serve_timeout", self.tail())

    def tail(self, lines=12):
        try:
            with open(self.log, "rb") as f:
                text = f.read()[-4000:].decode("utf-8", "replace")
        except OSError:
            return ""
        return " | ".join(text.splitlines()[-lines:])[:1500]

    def stop(self):
        if self.proc is None:
            return
        try:
            os.killpg(self.proc.pid, signal.SIGTERM)
        except OSError:
            pass
        try:
            self.proc.wait(timeout=20)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(self.proc.pid, signal.SIGKILL)
            except OSError:
                pass
            self.proc.wait()
        self.proc = None


def create_model(binary, work, port, model, timeout):
    modelfile = os.path.join(work, "Modelfile")
    with open(modelfile, "w") as f:
        f.write(f"FROM {model}\nPARAMETER temperature 0\nPARAMETER top_k 1\nPARAMETER seed {SEED}\nPARAMETER num_ctx {NUM_CTX}\n")
    t0 = time.perf_counter()
    try:
        done = subprocess.run([binary, "create", MODEL_TAG, "-f", modelfile], env=server_env(work, port),
                              stdin=subprocess.DEVNULL, capture_output=True, timeout=timeout)
    except (subprocess.TimeoutExpired, OSError):
        raise ArmFail("create_failed", "timeout") from None
    if done.returncode != 0:
        raise ArmFail("create_failed", (done.stderr or done.stdout).decode("utf-8", "replace")[-300:])
    return time.perf_counter() - t0


def generate(port, timeout):
    doc = http_json(port, "/api/generate", {
        "model": MODEL_TAG, "prompt": PROMPT, "stream": False, "keep_alive": "30m",
        "options": {"temperature": 0, "top_k": 1, "seed": SEED, "num_predict": NUM_PREDICT, "num_ctx": NUM_CTX},
    }, timeout=timeout)
    text = doc.get("response")
    if not isinstance(text, str):
        raise ArmFail("generate_bad_response")
    ints = {}
    for key in ("eval_count", "eval_duration", "prompt_eval_count", "total_duration", "load_duration"):
        v = doc.get(key)
        if type(v) is not int or v < 0:
            raise ArmFail("generate_bad_response", key)
        ints[key] = v
    context = doc.get("context")
    ids = None
    if isinstance(context, list) and context and all(type(x) is int for x in context):
        ids = hashlib.sha256(json.dumps(context, separators=(",", ":")).encode()).hexdigest()
    return text, ints, ids


def gpu_name():
    try:
        out = subprocess.run(["nvidia-smi", "--query-gpu=name,driver_version", "--format=csv,noheader"],
                             capture_output=True, timeout=20, stdin=subprocess.DEVNULL)
    except (OSError, subprocess.TimeoutExpired):
        return ""
    return out.stdout.decode("utf-8", "replace").splitlines()[0][:80] if out.returncode == 0 and out.stdout else ""


# ------------------------------------------------------------------ the arm

def run_arm(label, tarball, model, work, port, repeats, timeout, say=print):
    inputs = {}
    for key, path in (("model", model), ("ollama", tarball)):
        try:
            size, secs, digest = read_checked(path)
        except OSError:
            raise ArmFail("input_unreadable", key) from None
        inputs[key] = {"bytes": size, "read_secs": round(secs, 6), "sha256": digest}
        mbps = size / secs / 1e6 if secs > 0 else 0.0
        say(f"KF_LLM_SHARE_READ label={label} file={key} bytes={size} secs={secs:.3f} MBps={mbps:.1f}")
    os.makedirs(work, mode=0o700, exist_ok=True)
    binary = unpack(tarball, os.path.join(work, "ollama"))
    server = Server(binary, work, port)
    try:
        server.start(timeout=min(120, timeout))
        version = http_json(port, "/api/version", timeout=10).get("version")
        if not isinstance(version, str) or not re.fullmatch(r"[A-Za-z0-9._+-]{1,40}", version):
            raise ArmFail("version_bad")
        create_secs = create_model(binary, work, port, model, timeout)
        warm_text, warm, _ = generate(port, timeout)
        runs = []
        for _ in range(repeats):
            text, ints, ids = generate(port, timeout)
            runs.append({
                "eval_count": ints["eval_count"], "eval_duration_ns": ints["eval_duration"],
                "prompt_eval_count": ints["prompt_eval_count"], "total_duration_ns": ints["total_duration"],
                "text_sha256": hashlib.sha256(text.encode()).hexdigest(), "token_ids_sha256": ids,
            })
            if len(runs) == 1:
                first = text
        ps = http_json(port, "/api/ps", timeout=10)
        models = ps.get("models")
        entry = models[0] if isinstance(models, list) and len(models) == 1 and isinstance(models[0], dict) else {}
        size, vram = entry.get("size"), entry.get("size_vram")
        if type(size) is not int or type(vram) is not int or size < 0 or vram < 0:
            raise ArmFail("ps_bad")
    finally:
        server.stop()
    return {
        "schema": SCHEMA, "label": label, "ollama_version": version, "inputs": inputs,
        "create_secs": round(create_secs, 6), "load_ms": round(warm["load_duration"] / 1e6, 3),
        "params": {"num_predict": NUM_PREDICT, "seed": SEED, "repeats": repeats, "prompt_sha256": prompt_sha256()},
        "warmup_text_sha256": hashlib.sha256(warm_text.encode()).hexdigest(),
        "runs": runs, "text": first[:160], "ps": {"size": size, "size_vram": vram}, "gpu": gpu_name(),
    }


def build_parser():
    p = argparse.ArgumentParser(prog="llm_parity_arm.py", description=__doc__.split("\n\n")[0])
    p.add_argument("--label", required=True)
    p.add_argument("--tarball", required=True)
    p.add_argument("--model", required=True)
    p.add_argument("--work", required=True)
    p.add_argument("--port", type=int, default=11555)
    p.add_argument("--repeats", type=int, default=REPEATS)
    p.add_argument("--timeout", type=int, default=900)
    return p


def main(argv=None):
    args = build_parser().parse_args(argv)
    if not all(PATH_RE.fullmatch(x) for x in (args.tarball, args.model, args.work)):
        print("KF_LLM_ARM_FAIL reason=usage", flush=True)
        return 2
    if not LABEL_RE.fullmatch(args.label) or not 1024 <= args.port <= 65535 or not 1 <= args.repeats <= 8 or not 30 <= args.timeout <= 7200:
        print("KF_LLM_ARM_FAIL reason=usage", flush=True)
        return 2
    try:
        doc = run_arm(args.label, args.tarball, args.model, args.work, args.port, args.repeats, args.timeout)
    except ArmFail as error:
        print("KF_LLM_ARM_FAIL reason=" + error.reason, flush=True)
        if error.detail:
            print("KF_LLM_ARM_TAIL " + re.sub(r"[^\x20-\x7e]", "?", error.detail)[:1500], flush=True)
        return 1
    print(RESULT_PREFIX + json.dumps(doc, separators=(",", ":"), sort_keys=True), flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
