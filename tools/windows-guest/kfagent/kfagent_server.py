#!/usr/bin/env python3
"""Lab-only command broker for the owner's Win11 VM (trusted host, loopback only).
Guest (SYSTEM scheduled task) polls GET /next, runs the PowerShell text, POSTs /result/<id>.
CLI: kfagent_server.py run 'powershell text' [timeout]  -> enqueues and prints the result."""
import http.server, json, os, sys, time, uuid, glob
D = "/data/kfagent"; Q = D + "/q"; R = D + "/r"
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
        self.send_response(200); self.send_header("Content-Length", str(len(body))); self.end_headers(); self.wfile.write(body)
    def do_POST(self):
        if not self.path.startswith("/result/"):
            self.send_response(404); self.end_headers(); return
        i = os.path.basename(self.path)
        n = int(self.headers.get("Content-Length") or 0)
        open(R + "/" + i + ".txt", "wb").write(self.rfile.read(n))
        self.send_response(200); self.send_header("Content-Length", "0"); self.end_headers()

def cli():
    cmd = sys.argv[2]; t = int(sys.argv[3]) if len(sys.argv) > 3 else 120
    i = "%d-%s" % (time.time() * 1000, uuid.uuid4().hex[:6])
    open(Q + "/" + i + ".json", "w").write(json.dumps({"id": i, "cmd": cmd}))
    t0 = time.time()
    while time.time() - t0 < t:
        if os.path.exists(R + "/" + i + ".txt"):
            sys.stdout.write(open(R + "/" + i + ".txt", "rb").read().decode("utf-8", "replace")); return 0
        time.sleep(1)
    print("TIMEOUT waiting for guest"); return 1

if len(sys.argv) > 1 and sys.argv[1] == "run":
    sys.exit(cli())
http.server.ThreadingHTTPServer(("127.0.0.1", 8765), H).serve_forever()
