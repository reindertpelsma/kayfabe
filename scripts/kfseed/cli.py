# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
import argparse
import http.client
import socket
import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time

from . import core, devserver, nginxcheck, selfcheck

INSTALL_HELP = """nginx was not found on PATH. Install it (this tool changes nothing itself):
  Debian/Ubuntu: apt-get install -y --no-install-recommends nginx-light   (then: systemctl disable --now nginx)
  Fedora/RHEL:   dnf install -y nginx
  Alpine:        apk add nginx
kfseed starts its own nginx with `nginx -c <temp file> -p <temp dir>`; the system service is not needed.
For tests/development only: --fallback-python-server."""


def _artifacts(args):
    try:
        return [core.parse_artifact(a) for a in args.artifact]
    except core.SeedError as e:
        sys.exit("kfseed: %s" % e)


def port_in_use(port):
    """True if something already listens on, or holds, the port (so we never talk to someone else's server)."""
    try:
        socket.create_connection(("127.0.0.1", port), 1).close()
        return True
    except OSError:
        pass
    b = socket.socket()
    try:
        b.bind(("0.0.0.0", port))
        return False
    except OSError:
        return True
    finally:
        b.close()


def probe_ok(port, token):
    try:
        c = http.client.HTTPConnection("127.0.0.1", port, timeout=2)
        try:
            c.request("GET", "/%s/%s" % (token, core.PROBE_NAME))
            r = c.getresponse()
            return r.status == 200 and r.read() == core.PROBE_CONTENT
        finally:
            c.close()
    except (OSError, http.client.HTTPException):
        return False


def wait_ready(port, token, proc, run, timeout=5.0):
    """None when the server answers the probe with the exact content, else a reason string."""
    end = time.time() + timeout
    while time.time() < end:
        if proc and proc.poll() is not None:
            return "nginx exited early (rc %s)" % proc.returncode
        if proc and "[emerg]" in _tail(run, token):
            return "nginx reported an [emerg] error"
        if probe_ok(port, token):
            return None
        time.sleep(0.1)
    return "probe did not return the expected content within %.0f s" % timeout


def _tail(run, token, n=10):
    try:
        with open(os.path.join(run, "error.log"), errors="replace") as f:
            return "".join(f.readlines()[-n:]).replace(token, "<token>")
    except OSError:
        return ""


def serve(args):
    if args.check:
        return check_nginx(args)
    if not (args.root and args.port and args.artifact):
        sys.exit("kfseed: serve needs --root, --port and --artifact (or --check)")
    arts = _artifacts(args)
    nginx = shutil.which("nginx")
    if args.check:
        return check_nginx(args)
    if not nginx and not args.fallback_python_server:
        sys.stderr.write(INSTALL_HELP + "\n")
        return 2
    token = args.token or core.make_token()
    core.check_token(token)
    if port_in_use(args.port):
        sys.stderr.write("kfseed: port %d is already in use; refusing to start\n" % args.port)
        return 3
    run = tempfile.mkdtemp(prefix="kfseed-")
    os.chmod(run, 0o755)
    stage = os.path.join(run, "stage")
    try:
        staged = core.build_stage(stage, args.root, arts)
    except core.SeedError as e:
        shutil.rmtree(run, ignore_errors=True)
        sys.stderr.write("kfseed: %s\n" % e)
        return 3
    for sub in ("body", "proxy", "fastcgi", "uwsgi", "scgi"):
        os.makedirs(os.path.join(run, "tmp", sub))
    use_dev = args.fallback_python_server and not (nginx and not args.force_fallback)
    proc = srv = None
    stop = []
    signal.signal(signal.SIGTERM, lambda *a: stop.append(1))
    signal.signal(signal.SIGINT, lambda *a: stop.append(1))
    try:
        if use_dev:
            sys.stderr.write("kfseed: DEV-ONLY python server (not for production)\n")
            try:
                srv = devserver.start_thread(stage, token, args.port)
            except OSError as e:
                sys.stderr.write("kfseed: cannot bind port %d: %s\n" % (args.port, e))
                return 3
        else:
            conf = os.path.join(run, "nginx.conf")
            with open(conf, "w") as f:
                f.write(core.render_config(run, stage, args.port, token, [n for n, _ in arts]))
            t = subprocess.run([nginx, "-t", "-c", conf, "-p", run], capture_output=True, text=True)
            if t.returncode:
                sys.stderr.write(t.stderr)
                return 3
            proc = subprocess.Popen([nginx, "-c", conf, "-p", run])
        why = wait_ready(args.port, token, proc, run)
        if why:
            sys.stderr.write("kfseed: seeder not ready: %s\n%s" % (why, _tail(run, token)))
            return 3
        out = {"token": token, "port": args.port,
               "payloads": [core.seed_register_payload(n, *staged[n], token) for n, _ in arts]}
        print(json.dumps(out), flush=True)
        while not stop and (not args.serve_until or time.time() < args.serve_until):
            if proc and proc.poll() is not None:
                return 4
            time.sleep(0.2)
        return 0
    finally:
        if proc and proc.poll() is None:
            proc.terminate()
            try:
                proc.wait(10)
            except subprocess.TimeoutExpired:
                proc.kill()
        if srv:
            srv.shutdown()
        shutil.rmtree(run, ignore_errors=True)


def check_nginx(args):
    ok, lines = nginxcheck.check()
    print("\n".join(lines))
    if not ok:
        print(INSTALL_HELP)
    return 0 if ok else 2


def check(args):
    report = []
    bad = selfcheck.selfcheck(args.url, _artifacts(args), report=report)
    for r in report:
        print("PASS " + r)
    for b in bad:
        print("FAIL " + b)
    print("selfcheck: %s" % ("FAIL" if bad else "ok"))
    return 1 if bad else 0


def main(argv=None):
    p = argparse.ArgumentParser(prog="kfseed")
    sub = p.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("serve")
    s.add_argument("--root")
    s.add_argument("--port", type=int)
    s.add_argument("--token")
    s.add_argument("--artifact", action="append", default=[], metavar="NAME:SHA256")
    s.add_argument("--serve-until", type=float, help="unix timestamp")
    s.add_argument("--check", action="store_true", help="only report whether nginx is installed")
    s.add_argument("--fallback-python-server", action="store_true", help="DEV/TEST ONLY, used when nginx is missing")
    s.add_argument("--force-fallback", action="store_true", help="DEV/TEST ONLY: use the python server even if nginx exists")
    s.set_defaults(fn=serve)
    n = sub.add_parser("check", help="print installed and candidate nginx versions; changes nothing")
    n.set_defaults(fn=check_nginx)
    c = sub.add_parser("selfcheck")
    c.add_argument("--url", required=True)
    c.add_argument("--artifact", action="append", default=[], metavar="NAME:SHA256")
    c.set_defaults(fn=check)
    args = p.parse_args(argv)
    return args.fn(args)
