#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Stub of a loopback-redirect (OAuth style) application for the auth-capture proof. No real service, no credentials.

Starts an HTTP listener on a random loopback port, calls `xdg-open` with an authorization URL whose redirect_uri is
http://<host>:<port>/callback, and waits for the callback. It records only THAT a callback arrived, that `state` matched, and
the LENGTH of any `code` parameter: never the code itself.
STATUS: LIVE as a test stub, 2026-11-16.
"""
import argparse
import http.server
import json
import subprocess
import sys
import threading
import urllib.parse


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("--bind", default="127.0.0.1", help="127.0.0.1 or ::1")
    ap.add_argument("--host", default="localhost", help="the host name written into redirect_uri: localhost, 127.0.0.1 or [::1]")
    ap.add_argument("--timeout", type=float, default=300)
    ap.add_argument("--result", help="write the JSON result here")
    ap.add_argument("--opener", default="xdg-open")
    ap.add_argument("--mode", default="loopback", choices=("loopback", "terminal"))
    a = ap.parse_args(argv)
    if a.mode == "terminal":
        # a terminal program (like `matlab -nodesktop`): prints the URL, then prompts for a code on stdin
        print("To sign in, open this URL in a browser on another machine:\nhttps://auth.invalid/device?user_code=STUBCODE&state=stubstate123", flush=True)
        sys.stdout.write("Enter the authorization code: ")
        sys.stdout.flush()
        line = sys.stdin.readline().rstrip("\n")
        import hashlib
        res = {"code_len": len(line), "code_sha8": hashlib.sha256(line.encode()).hexdigest()[:8]}   # the code itself is never stored
        if a.result:
            open(a.result, "w").write(json.dumps(res) + "\n")
        print()
        print(json.dumps(res))
        return 0
    state = "stubstate123"
    got = {}
    done = threading.Event()

    class H(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            q = urllib.parse.parse_qs(urllib.parse.urlparse(self.path).query)
            if self.path.startswith("/callback"):
                got.update(callback=1, state_ok=int(q.get("state") == [state]), code_len=len((q.get("code") or [""])[0]))
                done.set()
            self.send_response(200)
            self.end_headers()
            self.wfile.write(b"ok")

        def log_message(self, *args):
            pass

    class S(http.server.HTTPServer):
        address_family = 10 if ":" in a.bind else 2   # AF_INET6 / AF_INET

    srv = S((a.bind, 0), H)
    port = srv.server_address[1]
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    redirect = "http://%s:%d/callback" % (a.host, port)
    url = ("https://auth.invalid/authorize?response_type=code&client_id=stub&state=%s&code_challenge=stubchallenge"
           "&code_challenge_method=S256&redirect_uri=%s" % (state, urllib.parse.quote(redirect, safe="")))
    subprocess.run([a.opener, url], check=False)
    ok = done.wait(a.timeout)
    res = {"callback": int(ok), **got, "port": port}
    if a.result:
        open(a.result, "w").write(json.dumps(res) + "\n")
    print(json.dumps(res))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
