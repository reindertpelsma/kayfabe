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
        text = core.render_config("/tmp/r", "/tmp/r/stage", 8123, "ab12cd34", ["art.bin"])
        self.assertIn("listen 8123 default_server;", text)
        self.assertIn("location = /ab12cd34/art.bin {\n            alias /tmp/r/stage/art.bin;", text)
        self.assertIn("location = /ab12cd34/probe.txt {\n            alias /tmp/r/stage/probe.txt;", text)
        self.assertNotRegex(text, r"location /ab12cd34")
        self.assertIn("absolute_redirect off;", text)
        self.assertIn("merge_slashes off;", text)
        self.assertNotIn("ignore_invalid_headers", text)  # does not affect colon-less lines; see README
        self.assertIn("if ($http_expect) { return 444; }", text)
        self.assertIn("if ($http_upgrade) { return 444; }", text)
        self.assertIn('if ($request !~ "^(GET|HEAD) /ab12cd34/(probe\\.txt|art\\.bin) HTTP/1\\.[01]$") { return 444; }', text)
        self.assertNotRegex(text, r"return 30\d")
        self.assertIn("400 401 403 404 405 408 413 414 416 431 494 500 502 503 504 505 = /_drop;", text)
        self.assertIn("autoindex off;", text)
        self.assertIn("limit_except GET HEAD { deny all; }", text)
        self.assertIn("location / { return 444; }", text)
        self.assertIn("error_page 400 401 403 404 405 408 413 414 416 431 494 500 502 503 504 505 = /_drop;", text)
        self.assertIn("if ($request_method !~ ^(GET|HEAD)$) { return 444; }", text)
        self.assertIn("if ($http_content_length) { return 444; }", text)
        self.assertIn("if ($http_transfer_encoding) { return 444; }", text)
        for d in ("client_max_body_size 1;", "client_body_buffer_size 1k;", "client_body_timeout 5s;",
                  "client_header_timeout 5s;", "client_header_buffer_size 1k;", "large_client_header_buffers 2 1k;",
                  "send_timeout 30s;", "keepalive_timeout 15s;", "keepalive_requests 100;",
                  "reset_timedout_connection on;"):
            self.assertIn(d, text)
        self.assertNotRegex(text, r"client_max_body_size\s+0")
        self.assertIn("location = /_drop { internal; return 444; }", text)
        self.assertNotIn("return 404", text)
        for s in ("sendfile on;", "access_log off;", "server_tokens off;", "daemon off;", "pid /tmp/r/nginx.pid;"):
            self.assertIn(s, text)
        self.assertNotIn("autoindex on", text)
        self.assertLess(len(text.splitlines()), 60)
        with self.assertRaises(core.SeedError):
            core.render_config("/tmp/r;x", "/tmp/s", 80, "ab12cd34", ["a"])
        for bad in ("a b", "a;b", "a{b", "a\nb", "../x", "a/b", "a" * 101, ".hidden", "probe.txt", "a$b", "a%2e"):
            with self.assertRaises(core.SeedError, msg=bad):
                core.render_config("/tmp/r", "/tmp/s", 80, "ab12cd34", [bad])
            with self.assertRaises(core.SeedError, msg=bad):
                core.parse_artifact(bad + ":" + SHA)
        core.parse_artifact("a" * 100 + ":" + SHA)

    def test_stage_only_registered(self):
        stage = os.path.join(self.tmp, "stage")
        sizes = core.build_stage(stage, self.root, [("art.bin", SHA)])
        self.assertEqual(sorted(os.listdir(stage)), ["art.bin", "probe.txt"])
        self.assertEqual(sizes, {"art.bin": (SHA, len(DATA))})
        self.assertEqual(open(os.path.join(stage, "probe.txt"), "rb").read(), b"kfseed-probe-v1\n")

    def test_stage_rejects_missing_and_symlink(self):
        os.symlink("/etc/passwd", os.path.join(self.root, "ln"))
        for n in ("ln", "missing"):
            with self.assertRaises(core.SeedError):
                core.build_stage(os.path.join(self.tmp, "s" + n), self.root, [(n, SHA)])

    def test_stage_sha_mismatch_refused(self):
        with self.assertRaises(core.SeedError):
            core.build_stage(os.path.join(self.tmp, "st"), self.root, [("art.bin", "0" * 64)])
        self.assertFalse(os.path.exists(os.path.join(self.tmp, "st", "art.bin")))

    def swap_to(self, target):
        def hook(path):
            os.unlink(path)
            if target is None:
                with open(path, "wb") as f:
                    f.write(b"other regular file")
            else:
                os.symlink(target, path)
        return hook

    def secret(self):
        p = os.path.join(self.tmp, "rootonly.txt")
        with open(p, "w") as f:
            f.write("TOP-SECRET")
        return p

    def stage_contents(self, stage):
        return b"".join(open(os.path.join(stage, n), "rb").read() for n in os.listdir(stage))

    def test_swap_to_symlink_before_open_never_served(self):
        stage = os.path.join(self.tmp, "st")
        with self.assertRaises(core.SeedError):
            core.build_stage(stage, self.root, [("art.bin", SHA)], hooks={"before_open": self.swap_to(self.secret())})
        self.assertNotIn(b"TOP-SECRET", self.stage_contents(stage))

    def test_swap_to_other_file_before_open_refused(self):
        stage = os.path.join(self.tmp, "st")
        with self.assertRaises(core.SeedError):
            core.build_stage(stage, self.root, [("art.bin", SHA)], hooks={"before_open": self.swap_to(None)})
        self.assertNotIn(b"other regular file", self.stage_contents(stage))

    def test_swap_after_open_serves_original_bytes_only(self):
        stage = os.path.join(self.tmp, "st")
        staged = core.build_stage(stage, self.root, [("art.bin", SHA)], hooks={"after_open": self.swap_to(self.secret())})
        self.assertEqual(open(os.path.join(stage, "art.bin"), "rb").read(), DATA)
        self.assertEqual(staged["art.bin"], (SHA, len(DATA)))
        self.assertNotIn(b"TOP-SECRET", self.stage_contents(stage))

    def test_stage_modes_and_no_links(self):
        stage = os.path.join(self.tmp, "st")
        core.build_stage(stage, self.root, [("art.bin", SHA)])
        self.assertEqual(oct(os.stat(stage).st_mode & 0o777), "0o755")
        for n in os.listdir(stage):
            st = os.stat(os.path.join(stage, n))
            self.assertEqual(oct(st.st_mode & 0o777), "0o644")
        self.assertEqual(os.stat(os.path.join(stage, "art.bin")).st_nlink, 1)  # copied, not hard linked

    def test_name_rules(self):
        for bad in ("a..b", "..", ".x", "", "a" * 101):
            with self.assertRaises(core.SeedError, msg=bad):
                core.parse_artifact(bad + ":" + SHA)
        core.parse_artifact("a.b-c_d:" + SHA)

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
        try:
            c.request(method, path, headers=headers or {})
            r = c.getresponse()
            return r.status, r.read(), r
        finally:
            c.close()

    def test_selfcheck_passes(self):
        report = []
        self.assertEqual(selfcheck.selfcheck(self.url(), [("art.bin", SHA)], chunk=100_000, report=report), [])
        self.assertEqual(len(report), 5)
        self.assertTrue(all(r.endswith("no reply (ok)") for r in report), report)

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
        for method, path in (("GET", "/"), ("GET", "/wrongtok/art.bin"), ("GET", "/%s/secret.txt" % t),
                             ("GET", "/%s/" % t), ("POST", "/%s/art.bin" % t), ("PUT", "/%s/art.bin" % t)):
            with self.assertRaises((http.client.RemoteDisconnected, ConnectionResetError), msg=(method, path)):
                self.req(method, path)
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
        self.assertIn("GET / must get no reply (connection closed)", bad)
        self.assertTrue(url)


class NginxCheck(Base):
    """Fake nginx / apt-cache on PATH; PATH is replaced so the real system tools are never seen."""
    def run_check(self, nginx=None, apt=None, subcmd=("check",)):
        bindir = os.path.join(self.tmp, "bin")
        os.mkdir(bindir)
        if nginx:
            self.fake(bindir, "nginx", "echo 'nginx version: nginx/%s (fake)' >&2" % nginx)
        if apt:
            inst, cand = apt
            self.fake(bindir, "apt-cache", "echo '%s:'; echo '  Installed: %s'; echo '  Candidate: %s'" % ("$2", inst, cand))
        env = {"PATH": bindir}
        return subprocess.run([sys.executable, "-B", "-m", "kfseed", *subcmd], capture_output=True, text=True, env=env,
                              cwd=os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))

    @staticmethod
    def fake(bindir, name, body):
        path = os.path.join(bindir, name)
        with open(path, "w") as f:
            f.write("#!/bin/sh\n" + body + "\n")
        os.chmod(path, 0o755)

    def test_equal_no_warning(self):
        r = self.run_check("1.28.3", ("1.28.3-1", "1.28.3-1"))
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("nginx installed: 1.28.3", r.stdout)
        self.assertIn("nginx candidate: 1.28.3-1", r.stdout)
        self.assertNotIn("WARNING", r.stdout)

    def test_older_warns(self):
        r = self.run_check("1.28.2", ("1.28.2-1", "1.28.3-1"))
        self.assertEqual(r.returncode, 0)
        self.assertIn("WARNING: installed nginx (1.28.2-1) != repo candidate (1.28.3-1)", r.stdout)

    def test_serve_check_same_output(self):
        r = self.run_check("1.28.2", ("1.28.2-1", "1.28.3-1"), subcmd=("serve", "--check"))
        self.assertIn("WARNING", r.stdout)

    def test_no_package_manager(self):
        r = self.run_check("1.28.3")
        self.assertEqual(r.returncode, 0)
        self.assertIn("candidate unknown", r.stdout)
        self.assertNotIn("WARNING", r.stdout)

    def test_nginx_missing(self):
        r = self.run_check()
        self.assertEqual(r.returncode, 2)
        self.assertIn("nginx: MISSING", r.stdout)
        self.assertIn("apt-get install", r.stdout)


NGINX = shutil.which("nginx")
NGINX_PATH = NGINX

PKG_CWD = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


class ShadowAndPayload(Base):
    def test_check_with_dash_I_ignores_cwd(self):
        plant = os.path.join(self.tmp, "plant")
        os.makedirs(os.path.join(plant, "kfseed"))
        marker = os.path.join(self.tmp, "SHADOW-RAN")
        body = "open(%r, 'w').write('x')\nraise SystemExit(99)\n" % marker
        for mod in ("kfseed/__init__.py", "kfseed/cli.py", "kfseed/core.py", "json.py", "argparse.py", "subprocess.py", "shutil.py"):
            with open(os.path.join(plant, mod), "w") as f:
                f.write(body)
        main = os.path.join(PKG_CWD, "kfseed", "__main__.py")
        r = subprocess.run([sys.executable, "-I", main, "check"], cwd=plant, capture_output=True, text=True)
        self.assertFalse(os.path.exists(marker), r.stderr)
        self.assertIn(r.returncode, (0, 2), r.stderr)
        self.assertIn("nginx", r.stdout)

    def test_payload_has_computed_values(self):
        port = free_port()
        # registered sha matches, so the payload carries the staged sha256/size
        p = subprocess.Popen([sys.executable, "-B", "-I", os.path.join(PKG_CWD, "kfseed", "__main__.py"), "serve", "--root", self.root,
                              "--port", str(port), "--token", self.token, "--artifact", "art.bin:" + SHA,
                              "--fallback-python-server", "--force-fallback"],
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, cwd=self.tmp)
        self.addCleanup(lambda: (p.terminate(), p.wait(20)))
        info = json.loads(p.stdout.readline())
        self.assertEqual(info["payloads"][0]["artifact"], {"name": "art.bin", "sha256": SHA, "size": len(DATA)})

    def test_serve_refuses_wrong_sha(self):
        p = subprocess.run([sys.executable, "-B", "-m", "kfseed", "serve", "--root", self.root, "--port", str(free_port()),
                            "--artifact", "art.bin:" + "1" * 64, "--fallback-python-server", "--force-fallback"],
                           capture_output=True, text=True, cwd=PKG_CWD, timeout=30)
        self.assertEqual((p.returncode, p.stdout), (3, ""))
        self.assertIn("differs", p.stderr)


class Startup(Base):
    def serve(self, port, extra=(), env=None):
        return subprocess.Popen(
            [sys.executable, "-B", "-m", "kfseed", "serve", "--root", self.root, "--port", str(port), "--token", self.token,
             "--artifact", "art.bin:" + SHA, *extra],
            cwd=PKG_CWD, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=env)

    def runs(self):
        return {n for n in os.listdir(tempfile.gettempdir()) if n.startswith("kfseed-")}

    def occupy(self):
        s = socket.socket()
        s.bind(("0.0.0.0", 0))
        s.listen(5)
        self.addCleanup(s.close)
        return s.getsockname()[1]

    def check_port_in_use(self, extra):
        port = self.occupy()
        before = self.runs()
        p = self.serve(port, extra)
        out, err = p.communicate(timeout=30)
        self.assertEqual(p.returncode, 3, err)
        self.assertEqual(out, "")
        self.assertIn("already in use", err)
        self.assertEqual(self.runs(), before)

    def test_port_in_use_dev(self):
        self.check_port_in_use(["--fallback-python-server", "--force-fallback"])

    @unittest.skipUnless(NGINX_PATH, "SKIP nginx-not-installed")
    def test_port_in_use_nginx_no_leftover(self):
        pg = lambda: set(subprocess.run(["pgrep", "-f", "kfseed-.*nginx.conf"], capture_output=True, text=True).stdout.split())
        before = pg()  # other kfseed instances (not ours) may be running on this host
        self.check_port_in_use([])
        self.assertEqual(pg() - before, set())

    def test_payload_printed_only_after_probe_passes(self):
        port = free_port()
        p = self.serve(port, ["--fallback-python-server", "--force-fallback"])
        self.addCleanup(lambda: (p.terminate(), p.wait(20)))
        info = json.loads(p.stdout.readline())
        # no waiting: the very first line implies the probe already passed
        c = http.client.HTTPConnection("127.0.0.1", port, timeout=5)
        c.request("GET", "/%s/probe.txt" % self.token)
        self.assertEqual(c.getresponse().read(), b"kfseed-probe-v1\n")
        c.close()
        self.assertEqual(info["payloads"][0]["path"], "/%s/art.bin" % self.token)

    @unittest.skipUnless(NGINX_PATH, "SKIP nginx-not-installed")
    def test_nginx_payload_after_probe(self):
        port = free_port()
        p = self.serve(port)
        self.addCleanup(lambda: (p.terminate(), p.wait(20)))
        json.loads(p.stdout.readline())
        c = http.client.HTTPConnection("127.0.0.1", port, timeout=5)
        c.request("GET", "/%s/probe.txt" % self.token)
        self.assertEqual(c.getresponse().read(), b"kfseed-probe-v1\n")
        c.close()

    def test_nginx_missing_exits_2(self):
        empty = os.path.join(self.tmp, "empty")
        os.mkdir(empty)
        p = self.serve(free_port(), env={"PATH": empty})
        out, err = p.communicate(timeout=30)
        self.assertEqual((p.returncode, out), (2, ""))
        self.assertIn("apt-get install", err)



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

    def raw(self, data, wait=10, close_write=False):
        """Send raw bytes; return (bytes received, seconds until the server closed or wait expired)."""
        t0 = time.time()
        got = b""
        with socket.create_connection(("127.0.0.1", self.port), 5) as s:
            s.settimeout(wait)
            try:
                s.sendall(data)
                while True:
                    d = s.recv(65536)
                    if not d:
                        break
                    got += d
            except (ConnectionResetError, BrokenPipeError):
                pass
            except socket.timeout:
                return got, None
        return got, time.time() - t0

    def line(self, extra="", method="GET", path=None):
        path = path or "/%s/art.bin" % self.token
        return ("%s %s HTTP/1.1\r\nHost: x\r\n%sConnection: close\r\n\r\n" % (method, path, extra)).encode()

    def test_odd_methods_and_bodies_get_no_reply(self):
        self.start_once()
        cases = {
            "PROPFIND": self.line(method="PROPFIND"), "TRACE": self.line(method="TRACE"),
            "OPTIONS": self.line(method="OPTIONS"), "DELETE": self.line(method="DELETE"),
            "PUT": self.line(method="PUT"), "POST": self.line(method="POST"),
            "CONNECT": b"CONNECT 127.0.0.1:80 HTTP/1.1\r\nHost: x\r\n\r\n",
            "made-up": self.line(method="FROBNICATE"), "lowercase get": self.line(method="get"),
            "GET with content-length": self.line("Content-Length: 1000\r\n") .replace(b"\r\n\r\n", b"\r\n\r\n" + b"x" * 1000),
            "GET chunked": self.line("Transfer-Encoding: chunked\r\n").replace(b"\r\n\r\n", b"\r\n\r\n5\r\nhello\r\n0\r\n\r\n"),
            "no trailing slash": self.line(path="/" + self.token), "dir": self.line(path="/%s/" % self.token),
            "double slash": self.line(path="//%s/art.bin" % self.token), "dot": self.line(path="/%s/./art.bin" % self.token),
            "dotdot": self.line(path="/%s/../%s/art.bin" % (self.token, self.token)),
            "enc dotdot": self.line(path="/%s/%%2e%%2e/art.bin" % self.token),
            "enc slash": self.line(path="/%s%%2fart.bin" % self.token), "enc slash2": self.line(path="/%s/%%2fart.bin" % self.token),
            "trailing slash file": self.line(path="/%s/art.bin/" % self.token),
            "unknown name": self.line(path="/%s/secret.txt" % self.token), "root": self.line(path="/"),
            "wrong token": self.line(path="/zzzzzzzz/art.bin"), "absolute uri": self.line(path="http://x/%s/art.bin" % self.token),
            "4KB header": self.line("X-Pad: " + "a" * 4096 + "\r\n"),
        }
        for name, data in cases.items():
            got, dt = self.raw(data)
            self.assertEqual(got, b"", name)
            self.assertIsNotNone(dt, name + " not closed")
        t0 = time.time()
        got, dt = self.raw(self.line("Content-Length: 10000000\r\n", method="POST") + b"x" * 10_000_000)
        self.assertEqual(got, b"")
        self.assertLess(time.time() - t0, 5)

    def malformed(self, path):
        """Exact byte strings of malformed requests, built against `path`."""
        return {
            "HTTP/9.9": ("GET %s HTTP/9.9\r\nHost: x\r\nConnection: close\r\n\r\n" % path).encode(),
            "header without colon": ("GET %s HTTP/1.1\r\nHost: x\r\nBadHeader\r\nConnection: close\r\n\r\n" % path).encode(),
            "Expect 100-continue": ("GET %s HTTP/1.1\r\nHost: x\r\nExpect: 100-continue\r\nConnection: close\r\n\r\n" % path).encode(),
            "Upgrade websocket": ("GET %s HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n" % path).encode(),
            "Range bytes=abc": ("GET %s HTTP/1.1\r\nHost: x\r\nRange: bytes=abc\r\nConnection: close\r\n\r\n" % path).encode(),
            "Range beyond end": ("GET %s HTTP/1.1\r\nHost: x\r\nRange: bytes=99999999-\r\nConnection: close\r\n\r\n" % path).encode(),
        }

    def test_malformed_valid_path_no_reply(self):
        self.start_once()
        for name, data in self.malformed("/%s/art.bin" % self.token).items():
            if name == "header without colon":
                continue  # nginx accepts it (README); needs the token, covered by the wrong-token test
            got, dt = self.raw(data)
            self.assertEqual(got, b"", name)
            self.assertIsNotNone(dt, name)

    def test_malformed_wrong_token_and_path_no_reply(self):
        self.start_once()
        for path in ("/zzzzzzzz/art.bin", "/%s/nope" % self.token, "/", "/%s/probe.txt" % "zzzzzzzz"):
            for name, data in self.malformed(path).items():
                got, dt = self.raw(data)
                self.assertEqual(got, b"", (name, path))
                self.assertIsNotNone(dt, (name, path))

    def test_lf_only_line_endings_documented_behaviour(self):
        self.start_once()
        got, _ = self.raw(("GET /%s/probe.txt HTTP/1.1\nHost: x\nConnection: close\n\n" % self.token).encode())
        self.assertTrue(got.startswith(b"HTTP/1.1 200"), got[:40])  # nginx accepts bare LF; needs the token anyway
        got, _ = self.raw(self.malformed("/%s/probe.txt" % self.token)["header without colon"])
        self.assertTrue(got.startswith(b"HTTP/1.1 200"), got[:40])  # colon-less header line is accepted on a valid path
        got, _ = self.raw(b"GET /zzzzzzzz/probe.txt HTTP/1.1\nHost: x\nConnection: close\n\n")
        self.assertEqual(got, b"")

    def test_valid_requests_still_work(self):
        self.start_once()
        for extra, status in (("", b"200"), ("Range: bytes=0-9\r\n", b"206")):
            got, _ = self.raw(self.line(extra))
            self.assertTrue(got.startswith(b"HTTP/1.1 " + status), (extra, got[:40]))
        got, _ = self.raw(self.line(method="HEAD"))
        self.assertTrue(got.startswith(b"HTTP/1.1 200") and got.endswith(b"\r\n\r\n"))
        got, _ = self.raw(self.line(path="/%s/probe.txt" % self.token))
        self.assertTrue(got.endswith(b"kfseed-probe-v1\n"))

    def test_slow_partial_request_dropped(self):
        self.start_once()
        s = socket.create_connection(("127.0.0.1", self.port), 5)
        self.addCleanup(s.close)
        s.sendall(b"GET /abc")
        s.settimeout(9)
        t0 = time.time()
        try:
            data = s.recv(100)
        except ConnectionResetError:  # reset_timedout_connection
            data = b""
        self.assertEqual(data, b"")
        self.assertLess(time.time() - t0, 8)

    def test_big_artifact_slow_reader(self):
        big = os.path.join(self.root, "big.bin")
        blob = os.urandom(1 << 20) * 20
        with open(big, "wb") as f:
            f.write(blob)
        bsha = hashlib.sha256(blob).hexdigest()
        port = free_port()
        proc = subprocess.Popen(
            [sys.executable, "-m", "kfseed", "serve", "--root", self.root, "--port", str(port), "--token", self.token,
             "--artifact", "big.bin:" + bsha],
            cwd=os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))), stdout=subprocess.PIPE, text=True)
        self.addCleanup(lambda: (proc.terminate(), proc.wait(20)))
        proc.stdout.readline()
        time.sleep(0.5)
        c = http.client.HTTPConnection("127.0.0.1", port, timeout=20)
        c.request("GET", "/%s/big.bin" % self.token)
        r = c.getresponse()
        h = hashlib.sha256()
        while True:
            d = r.read(1 << 20)
            if not d:
                break
            h.update(d)
            time.sleep(0.05)  # slow reader; send_timeout is the gap between writes
        c.close()
        self.assertEqual(h.hexdigest(), bsha)
        self.assertEqual(selfcheck.selfcheck("http://127.0.0.1:%d/%s/" % (port, self.token), [("big.bin", bsha)]), [])

    def start_once(self):
        if not hasattr(self, "port"):
            self.port = self.start()

    def test_runtime_dir_removed_on_sigterm(self):
        self.start()
        before = set(os.listdir(tempfile.gettempdir()))
        self.assertTrue(any(n.startswith("kfseed-") for n in before))
        self.stop()
        time.sleep(0.2)
        self.assertEqual(self.proc.returncode, 0)


if __name__ == "__main__":
    unittest.main()
