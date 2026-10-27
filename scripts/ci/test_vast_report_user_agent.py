"""The box reporter must send an explicit User-Agent.

MEASURED 2026-10-25 on a real Vast box: Cloudflare answered HTTP 403 "error code: 1010" (browser
signature banned) to Python's default urllib User-Agent before Access or the Worker saw the
request, so the first protocol-7 rental never reported. The same request with a curl-style
User-Agent passed Access and reached the Worker.
FALSIFIER: the reporter's POST carries no User-Agent, or the urllib default one.
"""

import importlib.util
from pathlib import Path
import sys
import unittest

spec = importlib.util.spec_from_file_location(
    "v7_report_ua", Path(__file__).with_name("vast_report.py")
)
report = importlib.util.module_from_spec(spec)
sys.modules["v7_report_ua"] = report
spec.loader.exec_module(report)


class _Response:
    status = 200

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        return False

    def read(self, _n):
        return b"{}"


class _Opener:
    def __init__(self):
        self.requests = []

    def open(self, request, timeout=None):
        self.requests.append(request)
        return _Response()


class UserAgentTests(unittest.TestCase):
    def post(self, access=None):
        transport = report.HttpsTransport("t", access=access)
        transport.opener = _Opener()
        transport.post("https://example.invalid/x", b"{}")
        return transport.opener.requests[0]

    def test_post_has_explicit_user_agent(self):
        agent = self.post().get_header("User-agent")
        self.assertTrue(agent)
        self.assertNotIn("Python-urllib", agent)

    def test_user_agent_is_static_and_plain(self):
        agent = self.post({"KF_ACCESS_ID": "a.access", "KF_ACCESS_SECRET": "b"}).get_header(
            "User-agent"
        )
        self.assertRegex(agent, r"^kayfabe-vast-box/[0-9]+( \([^)]*\))?$")

    def test_access_headers_still_sent_with_user_agent(self):
        request = self.post({"KF_ACCESS_ID": "a.access", "KF_ACCESS_SECRET": "b"})
        self.assertEqual(request.get_header("Cf-access-client-id"), "a.access")
        self.assertTrue(request.get_header("User-agent"))


if __name__ == "__main__":
    unittest.main()
