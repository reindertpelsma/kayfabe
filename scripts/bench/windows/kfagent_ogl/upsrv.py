import http.server, os, sys
D = "/data/claude-oglw/up"; os.makedirs(D, exist_ok=True)
class H(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def do_PUT(self):
        n = int(self.headers.get("Content-Length") or 0); name = os.path.basename(self.path)
        with open(os.path.join(D, name), "wb") as f:
            left = n
            while left > 0:
                b = self.rfile.read(min(left, 1 << 20)); 
                if not b: break
                f.write(b); left -= len(b)
        self.send_response(200); self.send_header("Content-Length", "0"); self.end_headers()
http.server.ThreadingHTTPServer(("127.0.0.1", 8768), H).serve_forever()
