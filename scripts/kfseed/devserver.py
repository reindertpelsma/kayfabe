# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""DEV/TEST ONLY: minimal range-capable stdlib server with the nginx URL layout. Never used in production."""
import http.server
import os
import re
import threading

from .core import PROBE_CONTENT, PROBE_NAME


def make_handler(stage, token):
    prefix = "/%s/" % token

    class H(http.server.BaseHTTPRequestHandler):
        server_version = "kfseed-dev"
        sys_version = ""

        def log_message(self, *a):
            pass

        def _deny(self, code):
            # like nginx `return 444`: close without any reply
            self.close_connection = True

        def _serve(self, body):
            if not self.path.startswith(prefix):
                return self._deny(404)
            name = self.path[len(prefix):]
            if name == PROBE_NAME:
                path = None
            elif re.match(r"^[A-Za-z0-9][A-Za-z0-9._-]*$", name) and os.path.isfile(os.path.join(stage, name)):
                path = os.path.join(stage, name)
            else:
                return self._deny(404)
            size = len(PROBE_CONTENT) if path is None else os.path.getsize(path)
            lo, hi, code = 0, size - 1, 200
            m = re.match(r"^bytes=(\d*)-(\d*)$", self.headers.get("Range", ""))
            if m and (m.group(1) or m.group(2)):
                if m.group(1):
                    lo = int(m.group(1))
                    hi = min(int(m.group(2)), size - 1) if m.group(2) else size - 1
                else:
                    lo = max(0, size - int(m.group(2)))
                if lo >= size or lo > hi:
                    self.send_response(416)
                    self.send_header("Content-Range", "bytes */%d" % size)
                    self.send_header("Content-Length", "0")
                    self.end_headers()
                    return
                code = 206
            self.send_response(code)
            self.send_header("Accept-Ranges", "bytes")
            if code == 206:
                self.send_header("Content-Range", "bytes %d-%d/%d" % (lo, hi, size))
            self.send_header("Content-Length", str(hi - lo + 1))
            self.end_headers()
            if not body:
                return
            if path is None:
                self.wfile.write(PROBE_CONTENT[lo:hi + 1])
                return
            with open(path, "rb") as f:
                f.seek(lo)
                left = hi - lo + 1
                while left:
                    data = f.read(min(1 << 20, left))
                    if not data:
                        break
                    self.wfile.write(data)
                    left -= len(data)

        def do_GET(self):
            self._serve(True)

        def do_HEAD(self):
            self._serve(False)

        def do_POST(self):
            self._deny(405)

        do_PUT = do_DELETE = do_PATCH = do_POST

    return H


def make_server(stage, token, port=0, host="127.0.0.1"):
    return http.server.ThreadingHTTPServer((host, port), make_handler(stage, token))


def start_thread(stage, token, port=0):
    srv = make_server(stage, token, port)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv
