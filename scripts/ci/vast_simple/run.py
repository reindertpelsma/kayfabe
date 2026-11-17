#!/usr/bin/env python3
"""Simple Vast CI runner: rent one GPU VM, run the payload over SSH, always tear it down.

Usage: VAST_API_KEY=... python3 -I run.py --sha <40 hex> --profile check|sweep [--out DIR]
Exit code is the verdict: 0 PASS, 1 FAIL (the code under test), 2 UNAVAILABLE (no offer, boot
failed, transport broke: no verdict about the code), 3 LEAK (an instance could not be proven
absent; the reaper is the backstop). Stdlib only. Design: ci/vast-broker/docs/SIMPLE_ALTERNATIVE.md.
"""

import argparse
import ipaddress
import json
import os
import re
import secrets
import shlex
import signal
import subprocess
import sys
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import offers as offer_rules  # noqa: E402
import vast_api  # noqa: E402
from vast_api import PREFIX, ApiError  # noqa: E402

PASS, FAIL, UNAVAILABLE, LEAK = 0, 1, 2, 3
NAMES = {PASS: "PASS", FAIL: "FAIL", UNAVAILABLE: "UNAVAILABLE", LEAK: "LEAK"}
LOG_CAP = 4 << 20
LINE_CAP = 4000
MAX_BOOTS = 2
MAX_CREATE_TRIES = 6
VERIFY_TRIES, VERIFY_SECONDS = 12, 10
DEAD = {"exited", "stopped", "offline", "error"}
SRC = "/workspace/kf-ci-src"
RESULT = "/workspace/kf-result.json"
READY_CMD = "true"
RESULT_CMD = "head -c 65536 " + RESULT
TOOLS_CMD = (
    "command -v git >/dev/null && command -v python3 >/dev/null && command -v curl >/dev/null"
    " || { export DEBIAN_FRONTEND=noninteractive; apt-get update -qq &&"
    " apt-get install -y -qq git curl python3 ca-certificates; }"
)


def clone_command(url, sha, dest=SRC):
    """Same steps as the Worker's onstart (src/onstart.js), HEAD verified against the SHA."""
    if not re.fullmatch("[0-9a-f]{40}", sha) or not re.fullmatch("/[-A-Za-z0-9_./]+", dest):
        raise ValueError("invalid sha or destination")
    if not re.fullmatch(r"https://github\.com/[-\w.]+/[-\w.]+\.git|file:///[-\w./]+", url):
        raise ValueError("invalid repository url")
    return (
        "set -eu\n"
        "git clone --filter=blob:none --no-checkout %s %s\n" % (url, dest)
        + "cd %s\ngit fetch --depth=1 origin %s\ngit checkout --detach %s\n" % (dest, sha, sha)
        + "[ \"$(git rev-parse HEAD)\" = '%s' ]\n" % sha
    )


def payload_command(sha, nonce, soft_epoch, remaining):
    """The payload is run directly: vast_payload.sh needs the broker-served dependency cache."""
    return (
        "cd %s && rm -f %s && env KF_SOFT_DEADLINE_MS=%d timeout --signal=TERM --kill-after=30s"
        " %ds python3 -I scripts/ci/vast_payload.py %s %s"
        % (SRC, RESULT, soft_epoch * 1000, remaining, sha, nonce)
    )


def grade(rc, text):
    """PASS needs a clean exit, a PASS verdict, no failures or skips, passes, known hardware."""
    if rc != 0:
        return FAIL, "payload exit status %d" % rc, None
    try:
        result = json.loads(text)
        suites, hardware = result["suites"], result["hardware"]
        counts = [[s[k] for s in suites] for k in ("pass", "fail", "skip")]
        ints = all(isinstance(n, int) and n >= 0 for column in counts for n in column)
        cc = hardware["compute_cap"]
        known = isinstance(hardware["gpu_name"], str) and hardware["gpu_name"].lower() not in (
            "",
            "unknown",
        )
        if result["verdict"] != "PASS" or not ints or not suites:
            return FAIL, "result is not a clean PASS", None
        if sum(counts[0]) == 0 or sum(counts[1]) or sum(counts[2]):
            return FAIL, "result has no passes, or failures or skips", None
        if not known or not isinstance(cc, int) or isinstance(cc, bool) or cc < 750:
            return FAIL, "unknown hardware", None
    except (ValueError, KeyError, TypeError, AttributeError):
        return FAIL, "result missing or invalid", None
    return PASS, "pass", {k: hardware.get(k) for k in ("gpu_name", "compute_cap", "driver_version")}


class Log:
    """Runner notes go out whole; box output is line-capped, byte-capped and redacted."""

    def __init__(self, stream, redactor, path=None, cap=LOG_CAP):
        self.stream, self.red, self.cap, self.used, self.dropped = stream, redactor, cap, 0, 0
        self.lock = threading.Lock()
        self.file = open(path, "w") if path else None

    def note(self, text):
        with self.lock:
            print("[runner] " + self.red.clean(text), file=self.stream, flush=True)

    def box(self, chunk):
        with self.lock:
            for line in self.red.clean(chunk).splitlines():
                line = line[:LINE_CAP]
                if self.used + len(line) + 1 > self.cap:
                    self.dropped += len(line) + 1
                    continue
                self.used += len(line) + 1
                print(line, file=self.stream)
                if self.file:
                    print(line, file=self.file)

    def close(self):
        if self.dropped:
            self.note("box output truncated, %d bytes dropped" % self.dropped)
        if self.file:
            self.file.close()


class SshTransport:
    """One ssh invocation per command: direct ip:port, no agent, no secrets in the argv."""

    def __init__(self, host, port, key=None, user="root"):
        self.host, self.port, self.key, self.user = host, port, key, user

    def argv(self, cmd):
        opts = ["BatchMode=yes", "StrictHostKeyChecking=no", "UserKnownHostsFile=/dev/null"]
        opts += ["LogLevel=ERROR", "ConnectTimeout=15", "ForwardAgent=no", "ServerAliveInterval=30"]
        argv = ["ssh", "-p", str(self.port), "-T"]
        for opt in opts:
            argv += ["-o", opt]
        if self.key:
            argv += ["-i", self.key, "-o", "IdentitiesOnly=yes"]
        return argv + ["%s@%s" % (self.user, self.host), "bash -c " + shlex.quote(cmd)]

    def exec(self, cmd, timeout, sink):
        proc = subprocess.Popen(
            self.argv(cmd),
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            errors="replace",
            start_new_session=True,
        )

        def pump():
            for line in proc.stdout:
                sink(line)

        reader = threading.Thread(target=pump, daemon=True)
        reader.start()
        try:
            rc = proc.wait(timeout)
        except subprocess.TimeoutExpired:
            os.killpg(proc.pid, signal.SIGKILL)
            proc.wait()
            rc = 124
        reader.join(10)
        return rc


@dataclass
class Deps:
    transport_factory: object = None
    clock: object = time.time
    sleep: object = time.sleep
    stdout: object = field(default_factory=lambda: sys.stdout)


class Runner:
    def __init__(self, args, api, red, log, deps, cfg):
        self.args, self.api, self.red, self.log, self.deps, self.cfg = (
            args,
            api,
            red,
            log,
            deps,
            cfg,
        )
        self.start = deps.clock()
        self.pending = {}  # label -> instance id (None until known)
        self.summary = {"sha": args.sha, "profile": args.profile, "attempts": []}

    def now(self):
        return self.deps.clock()

    def purge(self, label, iid=None):
        """Destroy every instance with this exact label and prove none is left (instances-v1)."""
        try:
            rows = self.api.list_instances(label)
            if any(r.get("label") != label for r in rows):
                self.log.note("listing returned a foreign label, deleting nothing")
                return False
            targets = {r["id"] for r in rows} | ({iid} if iid else set())
            for target in sorted(targets):
                self.api.destroy(target, label)
        except ApiError as error:
            self.log.note("teardown error: %s" % error)
            return False
        for _ in range(VERIFY_TRIES):
            try:
                gone = not self.api.list_instances(label) and (
                    iid is None or self.api.inspect(iid) is None
                )
            except ApiError:
                gone = False
            if gone:
                self.pending.pop(label, None)
                return True
            self.deps.sleep(VERIFY_SECONDS)
        return False

    def address(self, inst):
        try:
            host = str(ipaddress.ip_address(inst["public_ipaddr"]))
            port = int(inst["ports"]["22/tcp"][0]["HostPort"])
            return (host, port) if 0 < port < 65536 else None
        except (KeyError, IndexError, TypeError, ValueError):
            return None

    def wait_boot(self, iid, label, deadline):
        while True:
            inst = self.api.inspect(iid)
            if inst is None or inst.get("label") != label:
                return None
            status = inst.get("actual_status")
            addr = self.address(inst) if status == "running" else None
            if addr and self.deps.transport_factory(*addr).exec(READY_CMD, 20, lambda s: None) == 0:
                return addr
            if status in DEAD or self.now() >= deadline:
                return None
            self.deps.sleep(self.args.poll_seconds)

    def on_box(self, conn):
        """Clone, run, fetch the result. Returns (code, detail, hardware)."""
        a = self.args
        hard_end = self.start + a.hard_minutes * 60
        soft_end = int(hard_end - self.cfg["softMarginMinutes"] * 60)
        url = a.repo_url or "https://github.com/%s.git" % a.repo
        wait = int(min(900, soft_end - self.now()))
        if wait <= 0 or conn.exec(TOOLS_CMD + "\n" + clone_command(url, a.sha), wait, self.log.box):
            return FAIL, "checkout failed or deadline passed", None
        remaining = int(soft_end - self.now())
        if remaining <= 0:
            return FAIL, "deadline passed before the payload", None
        nonce = secrets.token_hex(16)
        cmd = payload_command(a.sha, nonce, soft_end, remaining)
        rc = conn.exec(cmd, remaining + 120, self.log.box)
        chunks = []
        got = conn.exec(RESULT_CMD, 60, chunks.append)
        return grade(rc, "".join(chunks) if got == 0 else "")

    def attempt(self, offer):
        a = self.args
        label = "%s%d-%s" % (PREFIX, int(self.now()), secrets.token_hex(4))
        self.pending[label] = None
        record = {"offer": offer["id"], "machine": offer["machine_id"], "die": offer["die"]}
        self.summary["attempts"].append(record)
        try:
            iid = self.api.create(offer["id"], label)
        except ApiError as error:
            self.log.note("create failed (%s)" % error)
            record["outcome"] = "create_failed"
            return None, self.purge(label)
        self.pending[label] = record["instance"] = iid
        self.log.note("rented instance %d (%s, offer %d)" % (iid, offer["die"], offer["id"]))
        deadline = min(self.now() + a.boot_minutes * 60, self.start + a.hard_minutes * 60)
        addr = self.wait_boot(iid, label, deadline)
        if addr is None:
            record["outcome"] = "boot_failed"
            self.log.note("instance %d did not boot in time" % iid)
            try:
                self.log.box(self.api.boot_log(iid))
            except (ApiError, OSError, ValueError):
                pass
            return None, self.purge(label, iid)
        record["outcome"] = "booted"
        try:
            code, detail, hardware = self.on_box(self.deps.transport_factory(*addr))
        except Exception as error:  # a broken transport says nothing about the code under test
            self.log.note("transport error %s" % type(error).__name__)
            code, detail, hardware = UNAVAILABLE, "transport error", None
        self.summary.update(detail=detail, hardware=hardware)
        return code, True

    def execute(self):
        a, cfg = self.args, self.cfg
        verdict, leaked = UNAVAILABLE, False
        try:
            found = self.api.offers(cfg)
            rotate = int(self.now() // 86400)
            queue = offer_rules.candidates(found, cfg, a.profile, a.dies, rotate)
            self.log.note("%d offers, %d allowed" % (len(found), len(queue)))
            boots, tries, machines = 0, 0, set()
            for offer in queue:
                if boots >= MAX_BOOTS or tries >= MAX_CREATE_TRIES:
                    break
                if offer["machine_id"] in machines:
                    continue
                machines.add(offer["machine_id"])
                tries += 1
                code, clean = self.attempt(offer)
                boots += "instance" in self.summary["attempts"][-1]
                if not clean:
                    leaked = True
                    break
                if code is not None:
                    verdict = code
                    break
        except ApiError as error:
            self.log.note("provider error: %s" % error)
        finally:
            for label, iid in list(self.pending.items()):
                leaked = not self.purge(label, iid) or leaked
        self.summary.update(verdict=NAMES[LEAK if leaked else verdict])
        if leaked:
            self.log.note("LEAK: an instance could not be proven absent; run the reaper")
        return LEAK if leaked else verdict


def read_bad_machines(path):
    ids = set()
    for line in Path(path).read_text().splitlines():
        line = line.split("#")[0].strip()
        if line:
            if not re.fullmatch("[1-9][0-9]{0,18}", line):
                raise ValueError("bad machine file")
            ids.add(int(line))
    return ids


def parse(argv):
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    p.add_argument("--sha", required=True)
    p.add_argument("--profile", choices=["check", "sweep"], required=True)
    p.add_argument("--out", default="vast-simple-out")
    p.add_argument("--defaults", default=str(offer_rules.DEFAULTS_PATH))
    p.add_argument("--api-base", default=vast_api.CONSOLE)
    p.add_argument("--bad-machines")
    p.add_argument("--repo", default="reindertpelsma/kayfabe")
    p.add_argument("--repo-url")
    p.add_argument("--ssh-key")
    p.add_argument("--ssh-user", default="root")
    p.add_argument("--dies", type=lambda s: s.split(","))
    p.add_argument("--boot-minutes", type=int, default=15)
    p.add_argument("--hard-minutes", type=int)
    p.add_argument("--poll-seconds", type=int, default=15)
    return p.parse_args(argv)


def main(argv=None, deps=None, env=None):
    env = os.environ if env is None else env
    deps = deps or Deps()
    try:
        args = parse(argv)
    except SystemExit as exit_:
        return int(exit_.code or 2)
    if not re.fullmatch("[0-9a-f]{40}", args.sha):
        print("[runner] --sha must be 40 lowercase hex", file=deps.stdout)
        return UNAVAILABLE
    red = vast_api.Redactor(env.get("VAST_API_KEY"))
    try:
        defaults = offer_rules.load_defaults(args.defaults)
        bad = read_bad_machines(args.bad_machines) if args.bad_machines else set()
        cfg = offer_rules.make_config(defaults, bad)
        args.hard_minutes = args.hard_minutes or defaults["hardMinutes"]
        api = vast_api.VastApi(env.get("VAST_API_KEY"), args.api_base, redactor=red)
    except (ApiError, OSError, ValueError, KeyError) as error:
        print("[runner] configuration refused: %s" % red.clean(error), file=deps.stdout)
        return UNAVAILABLE
    if deps.transport_factory is None:
        deps.transport_factory = lambda h, p: SshTransport(h, p, args.ssh_key, args.ssh_user)
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    log = Log(deps.stdout, red, out / "box.log")
    runner = Runner(args, api, red, log, deps, cfg)
    try:
        return runner.execute()
    finally:
        log.close()
        (out / "summary.json").write_text(json.dumps(runner.summary, indent=1) + "\n")


def cli():
    def term(*_):
        raise KeyboardInterrupt

    signal.signal(signal.SIGTERM, term)
    try:
        return main(sys.argv[1:])
    except KeyboardInterrupt:
        print("[runner] interrupted; instances were torn down first", file=sys.stderr)
        return 130


if __name__ == "__main__":
    sys.exit(cli())
