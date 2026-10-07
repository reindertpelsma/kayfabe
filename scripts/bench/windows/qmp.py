#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""qmp.py -- the small QMP / guest-agent client win_vm.sh drives QEMU with.

usage:
  qmp.py SOCK cmd NAME [JSON-ARGS]     run one QMP command; print its "return" as JSON
                                       (exit 1 and print the error on an "error" reply)
  qmp.py SOCK events OUT.jsonl         append every QMP event to OUT.jsonl, one JSON object per
                                       line, flushed per line, until QEMU closes the socket
  qmp.py SOCK keys KEY SECONDS         send-key KEY (a QEMU qcode, e.g. ret) once a second for
                                       SECONDS -- Windows' "Press any key to boot from CD"
  qmp.py SOCK last-shutdown OUT.jsonl  print the reason of the LAST SHUTDOWN event in OUT.jsonl
                                       (or "none"); SOCK is ignored
  qga.py-style, on the guest-agent socket:
  qmp.py SOCK qga-ping                 guest-ping (exit 0 when the agent answers)
  qmp.py SOCK qga-exec PATH [ARG...]   guest-exec with captured output; prints stdout/stderr and
                                       exits with the program's exit code (124 on timeout)

Box output is data: nothing printed here is interpreted by the caller beyond exit codes and the
one-word reason of `last-shutdown`.
"""

import base64
import json
import socket
import sys
import time


def connect(path, timeout=10.0):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(timeout)
    s.connect(path)
    return s, s.makefile("rwb", buffering=0)


def send(f, obj):
    f.write(json.dumps(obj).encode() + b"\n")


def recv(f):
    line = f.readline()
    if not line:
        raise EOFError("socket closed")
    return json.loads(line)


def qmp_open(path, timeout=10.0):
    s, f = connect(path, timeout)
    greeting = recv(f)
    if "QMP" not in greeting:
        raise RuntimeError(f"not a QMP greeting: {greeting}")
    send(f, {"execute": "qmp_capabilities"})
    while True:
        r = recv(f)
        if "return" in r:
            return s, f
        if "error" in r:
            raise RuntimeError(f"qmp_capabilities: {r['error']}")


def qmp_cmd(f, name, args=None):
    msg = {"execute": name}
    if args:
        msg["arguments"] = args
    send(f, msg)
    while True:
        r = recv(f)
        if "event" in r:
            continue
        return r


def cmd(sock, name, args_json=None):
    s, f = qmp_open(sock)
    r = qmp_cmd(f, name, json.loads(args_json) if args_json else None)
    s.close()
    if "error" in r:
        print(json.dumps(r["error"]), file=sys.stderr)
        return 1
    print(json.dumps(r.get("return")))
    return 0


def events(sock, out):
    s, f = qmp_open(sock)
    s.settimeout(None)
    with open(out, "a", encoding="utf-8") as o:
        o.write(json.dumps({"event": "KF_LOGGER_ATTACHED", "timestamp": {"seconds": int(time.time())}}) + "\n")
        o.flush()
        while True:
            try:
                r = recv(f)
            except (EOFError, OSError, ValueError):
                break
            if "event" in r:
                o.write(json.dumps(r) + "\n")
                o.flush()
    return 0


def keys(sock, key, seconds):
    s, f = qmp_open(sock)
    end = time.time() + float(seconds)
    sent = 0
    while time.time() < end:
        r = qmp_cmd(f, "send-key", {"keys": [{"type": "qcode", "data": key}]})
        if "error" in r:
            print(json.dumps(r["error"]), file=sys.stderr)
            return 1
        sent += 1
        time.sleep(1.0)
    s.close()
    print(f"sent {key} x{sent}")
    return 0


def last_shutdown(out):
    reason = "none"
    try:
        with open(out, encoding="utf-8") as o:
            for line in o:
                try:
                    r = json.loads(line)
                except ValueError:
                    continue
                if r.get("event") == "SHUTDOWN":
                    reason = r.get("data", {}).get("reason", "unknown")
    except OSError:
        reason = "no-events-file"
    print(reason)
    return 0


def qga_open(path, timeout=10.0):
    s, f = connect(path, timeout)
    # guest-sync flushes any half-read state the agent may hold from an earlier client.
    token = int(time.time()) & 0x7FFFFFFF
    send(f, {"execute": "guest-sync", "arguments": {"id": token}})
    while True:
        r = recv(f)
        if r.get("return") == token:
            return s, f


def qga_ping(sock):
    try:
        s, f = qga_open(sock, timeout=5.0)
        send(f, {"execute": "guest-ping"})
        r = recv(f)
        s.close()
        return 0 if "return" in r else 1
    except (OSError, EOFError, ValueError):
        return 1


def qga_exec(sock, path, argv, timeout=300.0):
    s, f = qga_open(sock, timeout=30.0)
    send(f, {"execute": "guest-exec", "arguments": {"path": path, "arg": argv, "capture-output": True}})
    r = recv(f)
    if "error" in r:
        print(json.dumps(r["error"]), file=sys.stderr)
        return 1
    pid = r["return"]["pid"]
    end = time.time() + timeout
    while time.time() < end:
        send(f, {"execute": "guest-exec-status", "arguments": {"pid": pid}})
        st = recv(f).get("return", {})
        if st.get("exited"):
            for key, stream in (("out-data", sys.stdout), ("err-data", sys.stderr)):
                if key in st:
                    stream.write(base64.b64decode(st[key]).decode("utf-8", "replace"))
            return int(st.get("exitcode", 1))
        time.sleep(1.0)
    print("qga-exec: timeout", file=sys.stderr)
    return 124


def main(argv):
    if len(argv) < 3:
        print(__doc__, file=sys.stderr)
        return 2
    sock, verb, rest = argv[1], argv[2], argv[3:]
    if verb == "cmd" and rest:
        return cmd(sock, rest[0], rest[1] if len(rest) > 1 else None)
    if verb == "events" and len(rest) == 1:
        return events(sock, rest[0])
    if verb == "keys" and len(rest) == 2:
        return keys(sock, rest[0], rest[1])
    if verb == "last-shutdown" and len(rest) == 1:
        return last_shutdown(rest[0])
    if verb == "qga-ping":
        return qga_ping(sock)
    if verb == "qga-exec" and rest:
        return qga_exec(sock, rest[0], rest[1:])
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
