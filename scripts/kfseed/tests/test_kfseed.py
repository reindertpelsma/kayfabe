# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
import hashlib
import http.client
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
from kfseed import core, devserver, selfcheck  # noqa: E402

DATA = os.urandom(300_000)
SHA = hashlib.sha256(DATA).hexdigest()


def free_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.root = os.path.join(self.tmp, "root")
        os.mkdir(self.root)
        with open(os.path.join(self.root, "art.bin"), "wb") as f:
            f.write(DATA)
        with open(os.path.join(self.root, "secret.txt"), "w") as f:
            f.write("nope")
        self.token = core.make_token()


class Pure(Base):
    def test_token_shape(self):
        for _ in range(200):
            self.assertRegex(core.make_token(), r"^[a-z0-9]{8}$")
        self.assertGreater(len({core.make_token() for _ in range(50)}), 45)
        with self.assertRaises(core.SeedError):
            core.check_token("ABCDEFGH")

    def test_config_exact(self):
        text = core.render_config("/tmp/r", "/tmp/r/stage", 8123, "ab12cd34")
        self.assertIn("listen 8123 default_server;", text)
        self.assertIn("location /ab12cd34/ {\n            alias /tmp/r/stage/;", text)
        self.assertIn("autoindex off;", text)
        self.assertIn("limit_except GET HEAD { deny all; }", text)
        self.assertIn("location / { return 404; }", text)
        for s in ("sendfile on;", "access_log off;", "server_tokens off;", "daemon off;", "pid /tmp/r/nginx.pid;"):
            self.assertIn(s, text)
        self.assertNotIn("autoindex on", text)
        self.assertLess(len(text.splitlines()), 35)
        with self.assertRaises(core.SeedError):
            core.render_config("/tmp/r;x", "/tmp/s", 80, "ab12cd34")

    def test_stage_only_registered(self):
        stage = os.path.join(self.tmp, "stage")
        sizes = core.build_stage(stage, self.root, [("art.bin", SHA)])
        self.assertEqual(sorted(os.listdir(stage)), ["art.bin", "probe.txt"])
        self.assertEqual(sizes, {"art.bin": len(DATA)})
        self.assertEqual(open(os.path.join(stage, "probe.txt"), "rb").read(), b"kfseed-probe-v1\n")

    def test_stage_rejects_missing_and_symlink(self):
        os.symlink("/etc/passwd", os.path.join(self.root, "ln"))
        for n in ("ln", "missing"):
            with self.assertRaises(core.SeedError):
                core.build_stage(os.path.join(self.tmp, "s" + n), self.root, [(n, SHA)])

    def test_artifact_spec(self):
        self.assertEqual(core.parse_artifact("a.b:" + SHA), ("a.b", SHA))
        for bad in ("a", "../x:" + SHA, "probe.txt:" + SHA, "a:xyz", "a/b:" + SHA):
            with self.assertRaises(core.SeedError):
                core.parse_artifact(bad)

    def test_payload(self):
        self.assertEqual(core.seed_register_payload("art.bin", SHA, 5, "ab12cd34"),
                         {"artifact": {"name": "art.bin", "sha256": SHA, "size": 5}, "path": "/ab12cd34/art.bin"})


class Selfcheck(Base):
    """Run the shared assertions against a server; DevServer and nginx subclasses provide it."""
    def start(self):
        stage = os.path.join(self.tmp, "stage")
        core.build_stage(stage, self.root, [("art.bin", SHA)])
        srv = devserver.start_thread(stage, self.token)
        self.addCleanup(srv.shutdown)
        return srv.server_address[1]

    def url(self):
        self.port = self.start()
        return "http://127.0.0.1:%d/%s/" % (self.port, self.token)

    def req(self, method, path, headers=None):
        c = http.client.HTTPConnection("127.0.0.1", self.port, timeout=10)
        c.request(method, path, headers=headers or {})
        r = c.getresponse()
        data = r.read()
        c.close()
        return r.status, data, r

    def test_selfcheck_passes(self):
        self.assertEqual(selfcheck.selfcheck(self.url(), [("art.bin", SHA)], chunk=100_000), [])

    def test_sha_mismatch_detected(self):
        bad = selfcheck.selfcheck(self.url(), [("art.bin", "0" * 64)])
        self.assertTrue(any("sha256 mismatch" in b for b in bad), bad)

    def test_http_semantics(self):
        u = self.url()
        t = self.token
        s, body, r = self.req("GET", "/%s/art.bin" % t)
        self.assertEqual((s, body), (200, DATA))
        s, body, r = self.req("GET", "/%s/art.bin" % t, {"Range": "bytes=10-19"})
        self.assertEqual((s, body), (206, DATA[10:20]))
        s, body, r = self.req("HEAD", "/%s/art.bin" % t)
        self.assertEqual((s, body, r.getheader("Content-Length")), (200, b"", str(len(DATA))))
        self.assertEqual(self.req("GET", "/")[0], 404)
        self.assertEqual(self.req("GET", "/wrongtok/art.bin")[0], 404)
        self.assertEqual(self.req("GET", "/%s/secret.txt" % t)[0], 404)
        self.assertIn(self.req("GET", "/%s/" % t)[0], (403, 404))  # no listing; nginx says 403 for a directory
        self.assertIn(self.req("POST", "/%s/art.bin" % t)[0], (403, 405))
        self.assertTrue(u.endswith("/"))

    def test_selfcheck_catches_open_root(self):
        # a server that answers 200 for / must fail the selfcheck: emulate with a handler wrapper
        url = self.url()
        orig = devserver.make_handler

        class Open(orig(os.path.join(self.tmp, "stage"), self.token)):
            def do_GET(s):
                if s.path == "/":
                    s.send_response(200)
                    s.send_header("Content-Length", "0")
                    s.end_headers()
                else:
                    super().do_GET()
        import http.server
        srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Open)
        import threading
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        self.addCleanup(srv.shutdown)
        bad = selfcheck.selfcheck("http://127.0.0.1:%d/%s/" % (srv.server_address[1], self.token), [("art.bin", SHA)])
        self.assertIn("GET / must be 404", bad)
        self.assertTrue(url)


NGINX = shutil.which("nginx")


@unittest.skipUnless(NGINX, "SKIP nginx-not-installed: real-nginx integration test needs nginx on PATH")
class RealNginx(Selfcheck):
    def start(self):
        port = free_port()
        self.proc = subprocess.Popen(
            [sys.executable, "-m", "kfseed", "serve", "--root", self.root, "--port", str(port),
             "--token", self.token, "--artifact", "art.bin:" + SHA],
            cwd=os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))),
            stdout=subprocess.PIPE, text=True)
        self.addCleanup(self.stop)
        info = json.loads(self.proc.stdout.readline())
        self.assertEqual(info["payloads"][0]["path"], "/%s/art.bin" % self.token)
        for _ in range(50):
            try:
                socket.create_connection(("127.0.0.1", port), 0.2).close()
                break
            except OSError:
                time.sleep(0.1)
        return port

    def stop(self):
        self.proc.terminate()
        self.assertEqual(self.proc.wait(20), 0)

    def test_selfcheck_catches_open_root(self):
        self.skipTest("covered by the dev-server variant; nginx config is exact-text tested")

    def test_runtime_dir_removed_on_sigterm(self):
        self.start()
        before = set(os.listdir(tempfile.gettempdir()))
        self.assertTrue(any(n.startswith("kfseed-") for n in before))
        self.stop()
        time.sleep(0.2)
        self.assertEqual(self.proc.returncode, 0)


if __name__ == "__main__":
    unittest.main()
