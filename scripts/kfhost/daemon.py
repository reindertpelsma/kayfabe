"""The daemon core: one outbound connection, offers, remote commands. Stdlib only."""

import json
import os
import random
import subprocess
import sys
import threading
import time

from . import VERSION, probes
from .config import ws_url
from .fetch import FetchError
from .jobs import JobContext
from .seeder import prepare_seeder
from .profile import Refused, allows_cmd, check_offer, derive, local_flags
from .transport import Closed

CMD_MAX_BYTES = 256 * 1024
CMD_CHUNK = 4096
CMD_DEFAULT_TIMEOUT, CMD_MAX_TIMEOUT = 300, 3600
EXIT_OK, EXIT_STOP = 0, 3


MAX_FRAME = 256 * 1024
MAX_NESTING = 32


def parse_frame(raw):
    """A JSON object from a frame, or None. Size and nesting are checked BEFORE json.loads."""
    if not isinstance(raw, str) or len(raw) > MAX_FRAME:
        return None
    depth = peak = 0
    in_str = esc = False
    for ch in raw:
        if in_str:
            if esc:
                esc = False
            elif ch == "\\":
                esc = True
            elif ch == '"':
                in_str = False
        elif ch == '"':
            in_str = True
        elif ch in "[{":
            depth += 1
            peak = max(peak, depth)
            if peak > MAX_NESTING:
                return None
        elif ch in "]}":
            depth -= 1
    try:
        msg = json.loads(raw)
    except (ValueError, RecursionError):
        return None
    return msg if isinstance(msg, dict) else None


class Daemon:
    def __init__(self, cfg, transport, runner_factory, prober=None, clock=time.monotonic, sleep=time.sleep,
                 rng=None, seed_timeout=15.0, seeder_run=None, euid=None, wall=time.time, log=None):
        self.cfg, self.transport, self.runner_factory = cfg, transport, runner_factory
        self.prober = prober or probes.Probes()
        self.clock, self.sleep, self.rng = clock, sleep, rng or random.Random()
        self.seed_timeout = seed_timeout
        self.local = local_flags(cfg)  # enforced locally; the Worker can add restrictions, never remove them
        self.profile = derive(self.local)  # no power before a welcome
        self.heartbeat_s = cfg["heartbeat_s"]
        self.lock = threading.Lock()
        self.conn = None
        self.active = None  # {"offer_id","run_id","runner","thread"}
        self.cmd_thread = None
        self.seeder_run = seeder_run
        self.wall = wall
        self.log = log or (lambda m: print(m, file=sys.stderr, flush=True))
        self.commands_until_ms = None
        self.delays = []  # every sleep the loop asked for (observable in tests)
        self.euid = os.geteuid() if euid is None else euid
        self.seeder = None
        self.seed_ok = True  # False after a refused/failed seed_register: seeding unavailable this session
        self._seed_event, self._seed_reply = threading.Event(), None
        self.threads = []
        self.stopping = False

    # ---- sending ----
    def send(self, msg):
        msg = dict(msg, v=1)
        with self.lock:
            conn = self.conn
            if conn is None:
                return False
            try:
                conn.send(json.dumps(msg, separators=(",", ":")))
            except Closed:
                return False
        return True

    def hello(self):
        caps = probes.measure(self.prober, self.cfg["work_dir"])
        return self.send({"type": "hello", "box_id": self.cfg["box_id"], "version": VERSION, "capabilities": caps,
                          "enforced": dict(self.local)}), caps

    def heartbeat(self):
        act = self.active
        msg = {"type": "heartbeat", "ts": int(self.wall()), "state": "busy" if act else "idle"}
        if act:
            msg["run_id"] = act["run_id"]
        self.send(msg)

    # ---- connection loop ----
    def headers(self):
        c = self.cfg
        return {
            "CF-Access-Client-Id": c["access_id"], "CF-Access-Client-Secret": c["access_secret"],
            "Authorization": "Bearer " + c["box_token"], "User-Agent": "kfhost/" + VERSION,
        }

    def backoff_delay(self, attempt, base=1.0, cap=60.0):
        return min(cap, base * 2 ** min(attempt, 10)) * (0.5 + 0.5 * self.rng.random())

    # Reconnect pacing per close code: (base, cap) seconds. 4409 = someone else holds the slot; 4403 = box
    # disabled or a transient Access 403. Neither stops the daemon; only 4401 (bad token) does.
    SLOW = {4409: (120.0, 600.0), 4403: (30.0, 300.0)}

    def run_forever(self, max_sessions=None):
        """Return an exit code: 3 on 4401 (bad token); 0 on a stop request or when max_sessions is reached."""
        attempt, sessions = 0, 0
        while not self.stopping:
            if max_sessions is not None and sessions >= max_sessions:
                return EXIT_OK
            sessions += 1
            code, welcomed = None, False
            try:
                conn = self.transport.connect(ws_url(self.cfg), self.headers())
            except Closed as error:
                code = error.code
            else:
                try:
                    welcomed = self.session(conn)
                except Closed as error:
                    code = error.code
                    welcomed = getattr(self, "_welcomed", False)
                finally:
                    with self.lock:
                        self.conn = None
                    conn.close()
            if code == 4401:
                self.log("kfhost: box token rejected (close 4401); fix the token in the config and restart")
                return EXIT_STOP
            if code in self.SLOW:
                self.log("kfhost: close %s (%s); retrying slowly" % (code, "another connection holds this box's slot"
                         if code == 4409 else "box disabled or access refused"))
                base, cap = self.SLOW[code]
                attempt += 1
                delay = self.backoff_delay(attempt - 1, base, cap)
            else:
                attempt = 0 if welcomed else attempt + 1
                delay = self.backoff_delay(attempt)
            self.delays.append(delay)
            self.sleep(delay)
        return EXIT_OK

    def session(self, conn):
        """Serve one connection until it drops (raises Closed). Running jobs are not touched."""
        self._welcomed = False
        with self.lock:
            self.conn = conn
        self.seed_ok = True
        self.seeder = None
        self.commands_until_ms = None
        ok, self.caps = self.hello()
        next_beat = self.clock() + self.heartbeat_s
        while not self.stopping:
            wait = max(0.05, min(1.0, next_beat - self.clock()))
            raw = conn.recv(wait)
            if self.clock() >= next_beat:
                self.heartbeat()
                next_beat = self.clock() + self.heartbeat_s
            if raw is None:
                continue
            msg = parse_frame(raw)
            if msg is None:
                self.log("kfhost: dropped an invalid or oversized frame")
                continue
            try:
                self.dispatch(msg)
            except Closed:
                raise
            except Exception as error:  # a hostile frame must never kill the daemon
                self.log("kfhost: frame handler failed: %s" % type(error).__name__)
        return self._welcomed

    # ---- dispatch ----
    def dispatch(self, msg):
        kind = msg.get("type")
        if kind == "welcome":
            self._welcomed = True
            self.profile = derive(self.local, msg)
            cu = msg.get("commands_until_ms")
            self.commands_until_ms = cu if type(cu) is int else None
            hb = msg.get("heartbeat_s")
            if isinstance(hb, (int, float)) and 1 <= hb <= 120:
                self.heartbeat_s = hb
        elif kind == "offer":
            self.on_offer(msg)
        elif kind == "cancel":
            act = self.active
            if act and act["offer_id"] == msg.get("offer_id"):
                act["runner"].cancel()
                act["cancelled"] = True
        elif kind == "seed_ack":
            self._seed_reply = msg
            self._seed_event.set()
        elif kind == "cmd":
            self.on_cmd(msg)
        # "ping" and unknown types are ignored

    def spawn(self, fn, *args):
        t = threading.Thread(target=fn, args=args, daemon=True)
        self.threads.append(t)
        t.start()
        return t

    def join(self, timeout=10):
        for t in list(self.threads):
            t.join(timeout)

    # ---- offers ----
    def on_offer(self, msg):
        oid = msg.get("offer_id") if isinstance(msg.get("offer_id"), str) else "?"
        try:
            offer = check_offer(self.profile, dict(msg), self.cfg, int(self.wall() * 1000))
            if self.profile == "borrowed" and self.euid == 0:
                raise Refused("borrowed refuses to run as root")
        except Refused as error:
            self.send({"type": "nack", "offer_id": oid, "reason": str(error)[:200]})
            return
        if self.active is not None or self.cmd_thread is not None and self.cmd_thread.is_alive():
            self.send({"type": "nack", "offer_id": oid, "reason": "busy"})
            return
        # Early ack: the offer passed the profile checks. A drop during the fetch loses nothing (the Worker
        # re-offers idempotently by offer_id); `ready` follows verification, `nack` follows a failed one.
        self.send({"type": "ack", "offer_id": oid})
        runner = self.runner_factory(self.cfg, self.caps)
        self.active = {"offer_id": oid, "run_id": offer["run_id"], "runner": runner, "cancelled": False}
        t = threading.Thread(target=self.job, args=(offer, runner, self.profile), daemon=True)
        self.active["thread"] = t
        self.threads.append(t)
        t.start()

    def job(self, offer, runner, profile):
        act = self.active
        timer = None
        try:
            try:
                checkout = runner.prepare(offer, profile)  # fetch + verify BEFORE anything runs
            except (FetchError, OSError) as error:
                self.send({"type": "nack", "offer_id": offer["offer_id"], "reason": ("verify: %s" % error)[:200]})
                return
            self.send({"type": "ready", "offer_id": offer["offer_id"]})
            # Time cap from the Worker's deadline (epoch milliseconds): cancel the run when it passes.
            left = max(0.0, offer["deadline"] / 1000.0 - self.wall())

            def expire():
                act["cancelled"] = True
                act["expired"] = True
                runner.cancel()

            timer = threading.Timer(left, expire)
            timer.daemon = True
            timer.start()
            ctx = JobContext(offer, profile, self.cfg, None, lambda: act["cancelled"])
            ctx.seed = lambda artifact, path: self.seed_register(offer["offer_id"], artifact, path, log=getattr(ctx, "log", None))
            runner.run(ctx, checkout)
        finally:
            if timer:
                timer.cancel()
            self.active = None

    # ---- seeding ----
    def seed_register(self, offer_id, artifact, path, log=None):
        """prepare_seeder once per session, send seed_register (with warnings), wait for seed_ack.
        Anything but ok:true, or an unavailable nginx, disables seeding for the session."""
        if not self.seed_ok:
            return False
        if self.seeder is None:
            kw = {"run": self.seeder_run} if self.seeder_run else {}
            self.seeder = prepare_seeder(self.profile, self.euid, **kw)
            for line in self.seeder["log"] + ["seeder warning: " + w for w in self.seeder["warnings"]]:
                (log or (lambda _t: None))(line)
            if not self.seeder["available"]:
                self.seed_ok = False
        if not self.seed_ok:
            return False
        self._seed_event.clear()
        self._seed_reply = None
        if not self.send({"type": "seed_register", "offer_id": offer_id, "artifact": artifact, "path": path,
                          "warnings": self.seeder["warnings"]}):
            self.seed_ok = False
            return False
        if not self._seed_event.wait(self.seed_timeout):
            self.seed_ok = False
            return False
        ok = bool(self._seed_reply and self._seed_reply.get("ok") is True)
        if not ok:
            self.seed_ok = False
        return ok

    # ---- remote commands (owned only) ----
    def on_cmd(self, msg):
        cid = msg.get("cmd_id")
        if not isinstance(cid, (str, int)):
            return
        refuse = None
        if not allows_cmd(self.profile, self.cfg, self.commands_until_ms, int(self.wall() * 1000)):
            refuse = "cmd refused: needs profile owned, local allow_cmd and an active commands_until"
        elif self.cmd_thread is not None and self.cmd_thread.is_alive():
            refuse = "cmd refused: another command is running"
        elif self.active is not None and msg.get("allow_during_run") is not True:
            refuse = "cmd refused: a run is active"
        else:
            argv, shell = msg.get("argv"), msg.get("shell")
            if (argv is None) == (shell is None):
                refuse = "cmd needs exactly one of argv or shell"
            elif argv is not None and not (isinstance(argv, list) and argv and all(isinstance(x, str) for x in argv)):
                refuse = "invalid argv"
            elif shell is not None and not isinstance(shell, str):
                refuse = "invalid shell"
        if refuse:
            self.send({"type": "cmd_output", "cmd_id": cid, "seq": 0, "stream": "stderr", "data": refuse, "eof": True})
            self.send({"type": "cmd_done", "cmd_id": cid, "rc": 126})
            return
        argv = msg["argv"] if msg.get("argv") is not None else ["/bin/sh", "-c", msg["shell"]]
        timeout = msg.get("timeout")
        timeout = min(timeout, CMD_MAX_TIMEOUT) if isinstance(timeout, (int, float)) and timeout > 0 else CMD_DEFAULT_TIMEOUT
        self.cmd_thread = self.spawn(self.run_cmd, cid, argv, timeout)

    def run_cmd(self, cid, argv, timeout):
        seq = [0]
        sent = [0]
        lock = threading.Lock()

        def emit(stream, data, eof=False):
            with lock:
                self.send({"type": "cmd_output", "cmd_id": cid, "seq": seq[0], "stream": stream, "data": data, "eof": eof})
                seq[0] += 1

        try:
            proc = subprocess.Popen(argv, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                    start_new_session=True)
        except OSError as error:
            emit("stderr", "spawn failed: %s" % error, True)
            self.send({"type": "cmd_done", "cmd_id": cid, "rc": 127})
            return
        truncated = [False]

        def pump(stream, name):
            for chunk in iter(lambda: stream.read1(CMD_CHUNK), b""):
                with lock:
                    room = CMD_MAX_BYTES - sent[0]
                    if room <= 0:
                        truncated[0] = True
                        continue  # keep draining so the child never blocks
                    chunk = chunk[:room]
                    sent[0] += len(chunk)
                emit(name, chunk.decode("utf-8", "replace"))

        readers = [threading.Thread(target=pump, args=(proc.stdout, "stdout"), daemon=True),
                   threading.Thread(target=pump, args=(proc.stderr, "stderr"), daemon=True)]
        for r in readers:
            r.start()
        timed_out = False
        try:
            rc = proc.wait(timeout)
        except subprocess.TimeoutExpired:
            timed_out = True
            try:
                os.killpg(proc.pid, 9)
            except OSError:
                proc.kill()
            rc = proc.wait()
        for r in readers:
            r.join(5)
        proc.stdout.close()
        proc.stderr.close()
        note = ("[output truncated at %d bytes]" % CMD_MAX_BYTES if truncated[0] else "") + ("[timeout]" if timed_out else "")
        emit("stderr", note, True)
        self.send({"type": "cmd_done", "cmd_id": cid, "rc": 124 if timed_out else rc})
