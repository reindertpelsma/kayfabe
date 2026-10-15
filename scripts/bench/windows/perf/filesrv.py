#!/usr/bin/env python3
"""filesrv.py SRC_DIR UP_DIR [PORT] - loopback file server for the Win11 VM (guest reaches the host as 10.0.2.2).
GET /<name>  serves SRC_DIR/<name>; PUT /<name> stores the body in UP_DIR/<name>. Basename only, no traversal."""
import http.server, os, sys
SRC, UP = os.path.abspath(sys.argv[1]), os.path.abspath(sys.argv[2])
PORT = int(sys.argv[3]) if len(sys.argv) > 3 else 8790
os.makedirs(UP, exist_ok=True)
class H(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def do_GET(self):
        p = os.path.join(SRC, os.path.basename(self.path))
        if not os.path.isfile(p):
            self.send_response(404); self.end_headers(); return
        b = open(p, "rb").read()
        self.send_response(200); self.send_header("Content-Length", str(len(b))); self.end_headers(); self.wfile.write(b)
    def do_PUT(self):
        n = int(self.headers.get("Content-Length") or 0)
        with open(os.path.join(UP, os.path.basename(self.path)), "wb") as f:
            f.write(self.rfile.read(n))
        self.send_response(200); self.send_header("Content-Length", "0"); self.end_headers()
http.server.ThreadingHTTPServer(("127.0.0.1", PORT), H).serve_forever()
