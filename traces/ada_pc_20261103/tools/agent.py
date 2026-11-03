#!/usr/bin/env python3
"""Lab-only loopback command broker for the Win11 test VM (copy of /data/hp1-20261025/agent.py with the
queue directory as argv[1]). The guest's SYSTEM scheduled task polls GET http://10.0.2.2:8765/next, runs the
PowerShell text and POSTs /result/<id>.
server: agent.py DIR            CLI: agent.py DIR run 'powershell text' [timeout]"""
import http.server, json, os, sys, time, uuid, glob, tempfile
D = sys.argv[1]; Q = D + "/q"; R = D + "/r"
for p in (Q, R): os.makedirs(p, exist_ok=True)

class H(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def do_GET(self):
        if self.path != "/next":
            self.send_response(404); self.end_headers(); return
        fs = sorted(glob.glob(Q + "/*.json"))
        if not fs:
            self.send_response(204); self.end_headers(); return
        f = fs[0]; body = open(f, "rb").read(); os.rename(f, f + ".sent")
        self.send_response(200); self.send_header("Content-Type", "application/json; charset=utf-8"); self.send_header("Content-Length", str(len(body))); self.end_headers(); self.wfile.write(body)
    def do_POST(self):
        if not self.path.startswith("/result/"):
            self.send_response(404); self.end_headers(); return
        i = os.path.basename(self.path)
        n = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(n)
        with tempfile.NamedTemporaryFile(dir=R, prefix=i + ".", suffix=".pending", mode="wb", delete=False) as t:
            t.write(body); pending = t.name
        os.replace(pending, R + "/" + i + ".txt")
        self.send_response(200); self.send_header("Content-Length", "0"); self.end_headers()

def cli():
    cmd = "& {\n" + sys.argv[3] + "\n}"; t = int(sys.argv[4]) if len(sys.argv) > 4 else 120
    i = "%d-%s" % (time.time() * 1000, uuid.uuid4().hex[:6])
    open(Q + "/" + i + ".json", "w").write(json.dumps({"id": i, "cmd": cmd}))
    t0 = time.time()
    while time.time() - t0 < t:
        if os.path.exists(R + "/" + i + ".txt"):
            sys.stdout.write(open(R + "/" + i + ".txt", "rb").read().decode("utf-8", "replace")); return 0
        time.sleep(1)
    # withdraw an unsent command so a late guest does not run it
    try: os.remove(Q + "/" + i + ".json")
    except OSError: pass
    print("TIMEOUT waiting for guest"); return 1

if len(sys.argv) > 2 and sys.argv[2] == "run":
    sys.exit(cli())
http.server.ThreadingHTTPServer(("127.0.0.1", 8765), H).serve_forever()
