"""The daemon core: one outbound connection, offers, remote commands. Stdlib only."""

import json
import os
import random
import subprocess
import threading
import time

from . import VERSION, probes
from .config import ws_url
from .fetch import FetchError
from .jobs import JobContext
from .profile import Refused, allows_cmd, check_offer, effective
from .transport import Closed

CMD_MAX_BYTES = 256 * 1024
CMD_CHUNK = 4096
CMD_DEFAULT_TIMEOUT, CMD_MAX_TIMEOUT = 300, 3600
EXIT_OK, EXIT_STOP = 0, 3


class Daemon:
    def __init__(self, cfg, transport, runner_factory, prober=None, clock=time.monotonic, sleep=time.sleep,
                 rng=None, seed_timeout=15.0):
        self.cfg, self.transport, self.runner_factory = cfg, transport, runner_factory
        self.prober = prober or probes.Probes()
        self.clock, self.sleep, self.rng = clock, sleep, rng or random.Random()
        self.seed_timeout = seed_timeout
        self.profile = cfg["profile_ceiling"]  # until welcome; the strictest of both afterwards
        self.heartbeat_s = cfg["heartbeat_s"]
        self.lock = threading.Lock()
        self.conn = None
        self.active = None  # {"offer_id","run_id","runner","thread"}
        self.cmd_thread = None
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
        return self.send({"type": "hello", "box_id": self.cfg["box_id"], "version": VERSION, "capabilities": caps}), caps

    def heartbeat(self):
        act = self.active
        msg = {"type": "heartbeat", "ts": int(time.time()), "state": "busy" if act else "idle"}
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

    def run_forever(self, max_sessions=None):
        """Return an exit code: 0 on replace (4409) or stop request, 3 on 4401/4403."""
        attempt, sessions = 0, 0
        while not self.stopping:
            if max_sessions is not None and sessions >= max_sessions:
                return EXIT_OK
            sessions += 1
            try:
                conn = self.transport.connect(ws_url(self.cfg), self.headers())
            except Closed as error:
                code = self.closed_code(error)
                if code is not None:
                    return code
                self.sleep(self.backoff_delay(attempt))
                attempt += 1
                continue
            try:
                welcomed = self.session(conn)
            except Closed as error:
                code = self.closed_code(error)
                if code is not None:
                    return code
                welcomed = getattr(self, "_welcomed", False)
            finally:
                with self.lock:
                    self.conn = None
                conn.close()
            attempt = 0 if welcomed else attempt + 1
            self.sleep(self.backoff_delay(attempt))
        return EXIT_OK

    @staticmethod
    def closed_code(error):
        """None = retry. 4401/4403 stop with an error; 4409 means another connection replaced us: exit."""
        if error.code in (4401, 4403):
            return EXIT_STOP
        if error.code == 4409:
            return EXIT_OK
        return None

    def session(self, conn):
        """Serve one connection until it drops (raises Closed). Running jobs are not touched."""
        self._welcomed = False
        with self.lock:
            self.conn = conn
        self.seed_ok = True
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
            try:
                msg = json.loads(raw)
            except ValueError:
                continue
            if isinstance(msg, dict):
                self.dispatch(msg)
        return self._welcomed

    # ---- dispatch ----
    def dispatch(self, msg):
        kind = msg.get("type")
        if kind == "welcome":
            self._welcomed = True
            self.profile = effective(msg.get("profile"), self.cfg["profile_ceiling"])
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
            offer = check_offer(self.profile, {k: v for k, v in msg.items()}, self.cfg)
        except Refused as error:
            self.send({"type": "nack", "offer_id": oid, "reason": str(error)[:200]})
            return
        if self.active is not None or self.cmd_thread is not None and self.cmd_thread.is_alive():
            self.send({"type": "nack", "offer_id": oid, "reason": "busy"})
            return
        runner = self.runner_factory(self.cfg, self.caps)
        self.active = {"offer_id": oid, "run_id": offer["run_id"], "runner": runner, "cancelled": False}
        t = threading.Thread(target=self.job, args=(offer, runner, self.profile), daemon=True)
        self.active["thread"] = t
        self.threads.append(t)
        t.start()

    def job(self, offer, runner, profile):
        act = self.active
        try:
            try:
                checkout = runner.prepare(offer)  # fetch + verify the commit BEFORE acking
            except (FetchError, OSError) as error:
                self.send({"type": "nack", "offer_id": offer["offer_id"], "reason": ("fetch: %s" % error)[:200]})
                return
            self.send({"type": "ack", "offer_id": offer["offer_id"]})
            ctx = JobContext(offer, profile, self.cfg, self.seed_register_for(offer), lambda: act["cancelled"])
            runner.run(ctx, checkout)
        finally:
            self.active = None

    # ---- seeding ----
    def seed_register_for(self, offer):
        return lambda artifact, path: self.seed_register(offer["offer_id"], artifact, path)

    def seed_register(self, offer_id, artifact, path):
        """Send seed_register, wait for seed_ack. Anything but ok:true disables seeding for the session."""
        if not self.seed_ok:
            return False
        self._seed_event.clear()
        self._seed_reply = None
        if not self.send({"type": "seed_register", "offer_id": offer_id, "artifact": artifact, "path": path}):
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
        if not allows_cmd(self.profile):
            refuse = "cmd refused: profile %s" % self.profile
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
