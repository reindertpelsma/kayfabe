#!/usr/bin/env python3
# Test double of the Worker /bench-data route (as traces/spec_stage_proof_20261109): answers GET /bench-data/<run>/<name> with a 302 to a
# presigned URL read from a 0600 file. Local to this box only.
import http.server, ssl, json, sys
urls = json.load(open("/root/.hwrun/urls.json"))
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        p = self.path.split("/")
        if len(p) == 4 and p[1] == "bench-data" and p[3] == "specviewperf-15.0.0.tar.zst":
            self.send_response(302); self.send_header("Location", urls["spec"]); self.end_headers()
            print("MINT", p[2], p[3], flush=True)
        else:
            self.send_response(404); self.end_headers()
    def log_message(self, *a): pass
s = http.server.ThreadingHTTPServer(("127.0.0.1", 8443), H)
c = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER); c.load_cert_chain("/root/.hwrun/cert.pem", "/root/.hwrun/key.pem")
s.socket = c.wrap_socket(s.socket, server_side=True)
s.serve_forever()
