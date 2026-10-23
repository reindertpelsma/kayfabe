"""Owner-scoped diagnostic download rules; no account credentials in this helper."""

import io
import unittest
from unittest.mock import patch

import vast_diagnostic_logs as diag


class Response(io.BytesIO):
    status = 200


class Opener:
    def __init__(self, body=b"untrusted log"):
        self.body = body
        self.calls = []

    def open(self, request, timeout):
        self.calls.append((request, timeout))
        return Response(self.body)


class DiagnosticLogs(unittest.TestCase):
    url = "https://s3.amazonaws.com/bucket/own-log?signature=secret"

    def fetch(self, url=None, opener=None, owned=7, requested=7):
        return diag.fetch_instance_log(
            owned,
            requested,
            {"result_url": url or self.url, "url": "https://evil.example/ignored"},
            opener=opener or Opener(),
        )

    def test_exact_provider_url_is_used(self):
        opener = Opener()
        self.assertEqual(self.fetch(opener=opener), b"untrusted log")
        self.assertEqual(opener.calls[0][0].full_url, self.url)

    def test_regional_s3_is_allowed(self):
        url = "https://bucket.s3.us-west-2.amazonaws.com/key?signature=x"
        opener = Opener()
        self.fetch(url, opener)
        self.assertEqual(opener.calls[0][0].full_url, url)

    def test_other_instance_is_refused(self):
        with self.assertRaises(diag.LogRefused):
            self.fetch(requested=8)

    def test_missing_owned_id_is_refused(self):
        with self.assertRaises(diag.LogRefused):
            self.fetch(owned=None)

    def test_http_is_refused(self):
        with self.assertRaises(diag.LogRefused):
            self.fetch("http://s3.amazonaws.com/key")

    def test_non_s3_amazon_is_refused(self):
        with self.assertRaises(diag.LogRefused):
            self.fetch("https://ec2.amazonaws.com/key")

    def test_suffix_confusion_is_refused(self):
        with self.assertRaises(diag.LogRefused):
            self.fetch("https://s3.amazonaws.com.evil.example/key")

    def test_userinfo_is_refused(self):
        with self.assertRaises(diag.LogRefused):
            self.fetch("https://owner:secret@s3.amazonaws.com/key")

    def test_port_is_refused(self):
        with self.assertRaises(diag.LogRefused):
            self.fetch("https://s3.amazonaws.com:8443/key")

    def test_fragment_is_refused(self):
        with self.assertRaises(diag.LogRefused):
            self.fetch(self.url + "#secret")

    def test_no_authorization_headers(self):
        opener = Opener()
        self.fetch(opener=opener)
        self.assertNotIn("authorization", {k.lower() for k in opener.calls[0][0].headers})

    def test_no_cookie_headers(self):
        opener = Opener()
        self.fetch(opener=opener)
        self.assertNotIn("cookie", {k.lower() for k in opener.calls[0][0].headers})

    def test_timeout_is_thirty_seconds(self):
        opener = Opener()
        self.fetch(opener=opener)
        self.assertEqual(opener.calls[0][1], 30)

    def test_oversized_log_is_refused(self):
        with self.assertRaises(diag.LogRefused):
            self.fetch(opener=Opener(b"x" * (5 * 1024 * 1024 + 1)))

    def test_exact_size_limit_is_accepted(self):
        self.assertEqual(len(self.fetch(opener=Opener(b"x" * (5 * 1024 * 1024)))), 5242880)

    def test_read_is_bounded(self):
        class Bounded(Response):
            def read(self, amount=-1):
                self.amount = amount
                return super().read(amount)

        response = Bounded(b"ok")
        opener = Opener()
        opener.open = lambda *a, **kw: response
        self.fetch(opener=opener)
        self.assertEqual(response.amount, 5242881)

    def test_redirect_handler_refuses(self):
        handler = diag.NoRedirect()
        self.assertIsNone(handler.redirect_request(None, None, 302, "x", {}, self.url))

    def test_default_opener_has_redirect_guard(self):
        opener = Opener()
        with patch.object(diag.urllib.request, "build_opener", return_value=opener) as build:
            diag.fetch_instance_log(7, 7, {"result_url": self.url})
        self.assertTrue(any(isinstance(h, diag.NoRedirect) for h in build.call_args.args))

    def test_download_exception_hides_secret_url(self):
        opener = Opener()
        opener.open = lambda *a, **kw: (_ for _ in ()).throw(ValueError(self.url))
        with self.assertRaises(diag.LogRefused) as caught:
            self.fetch(opener=opener)
        self.assertNotIn("signature", str(caught.exception))
        self.assertIsNone(caught.exception.__cause__)

    def test_provider_response_cannot_override_empty_request_headers(self):
        opener = Opener()
        self.fetch(opener=opener)
        self.assertEqual(opener.calls[0][0].headers, {})

    def test_regional_download_also_has_fixed_timeout(self):
        opener = Opener()
        self.fetch("https://s3.eu-west-1.amazonaws.com/key", opener)
        self.assertEqual(opener.calls[0][1], 30)

    def test_foreign_redirect_is_also_refused(self):
        self.assertIsNone(
            diag.NoRedirect().redirect_request(None, None, 307, "x", {}, "https://evil.example/")
        )

    def test_default_opener_installs_an_instance_not_a_class(self):
        with patch.object(diag.urllib.request, "build_opener", return_value=Opener()) as build:
            diag.fetch_instance_log(9, 9, {"result_url": self.url})
        self.assertIsInstance(build.call_args.args[0], diag.NoRedirect)

    def test_error_text_contains_no_remote_body_or_credentials(self):
        opener = Opener()
        opener.open = lambda *a, **kw: (_ for _ in ()).throw(RuntimeError("REMOTE_SECRET"))
        with self.assertRaises(diag.LogRefused) as caught:
            self.fetch(opener=opener)
        self.assertEqual(str(caught.exception), "diagnostic download refused")

    def test_regional_s3_does_not_expand_normal_api(self):
        self.assertFalse(diag.is_vast_api_url("https://s3.eu-west-1.amazonaws.com/key"))

    def test_normal_api_allowlist_stays_vast_only(self):
        self.assertTrue(diag.is_vast_api_url("https://console.vast.ai/api/v0/instances/7/"))
        self.assertFalse(diag.is_vast_api_url(self.url))


if __name__ == "__main__":
    unittest.main()
