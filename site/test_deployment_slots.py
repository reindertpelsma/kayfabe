"""Unprivileged byte splicing and candidate/local isolation."""

import importlib.util
from pathlib import Path
import tempfile
import unittest


def implementation():
    path = Path(__file__).with_name("deployment_slots.py")
    spec = importlib.util.spec_from_file_location("deployment_slots", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class Slots(unittest.TestCase):
    def setUp(self):
        self.impl = implementation()
        self.temp = tempfile.TemporaryDirectory(prefix="slot-build-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def test_candidate_inserts_neutral_slot_at_real_head_end(self):
        raw = (
            b'<html><head><!-- </head> --><script>let x="</head>";</script>'
            b"<title>caf\xc3\xa9</title></HEAD ><body>z</body></html>"
        )
        result = self.impl.prepare_page(raw, True)
        self.assertIn(b"</title><!-- kf-slot-a --></HEAD >", result)
        self.assertEqual(
            result.replace(b"<!-- kf-slot-a -->", b"").replace(
                b'<footer id="kf-footer"></footer>', b""
            ),
            raw,
        )

    def test_candidate_slot_is_inside_plain_document_head(self):
        raw = b'<html><head><meta charset="utf-8"/></head><body>x</body></html>'
        result = self.impl.prepare_page(raw, True)
        self.assertIn(b'<meta charset="utf-8"/><!-- kf-slot-a --></head>', result)

    def test_plain_page_has_footer_but_no_slot_or_privacy_copy(self):
        result = self.impl.prepare_page(b"<html><head></head><body>x</body></html>", False)
        self.assertIn(b'id="kf-footer"', result)
        for value in [b"kf-slot-", b"cloudflareinsights", b"Web Analytics"]:
            self.assertNotIn(value, result)

    def test_candidate_preparation_is_idempotent(self):
        raw = b'<html><head></head><body><footer id="kf-footer"></footer></body></html>'
        once = self.impl.prepare_page(raw, True)
        self.assertEqual(self.impl.prepare_page(once, True), once)
        self.assertEqual(once.count(b"<!-- kf-slot-a -->"), 1)

    def test_ambiguous_head_is_refused_before_mutation(self):
        with self.assertRaises(ValueError):
            self.impl.prepare_page(b"<head></head><head></head><body></body>", True)

    def test_cli_candidate_flag_is_ignored_outside_upstream_publication(self):
        import os
        import sys
        from unittest.mock import patch

        path = Path(__file__).with_name("build.py")
        spec = importlib.util.spec_from_file_location("slot_build", path)
        builder = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(builder)
        with patch.object(builder, "build") as run, patch.object(
            builder, "tools", return_value={}
        ), patch.dict(os.environ, {}, clear=True), patch.object(
            sys, "argv", ["build.py", "--tools-dir", str(self.root), "--deploy-candidate"]
        ):
            builder.main()
            self.assertFalse(run.call_args.kwargs["deploy_candidate"])

    def test_pr_local_and_fork_cannot_request_candidate(self):
        base = {
            "GITHUB_ACTIONS": "true",
            "GITHUB_REPOSITORY": "reindertpelsma/kayfabe",
            "GITHUB_EVENT_NAME": "push",
            "GITHUB_REF": "refs/heads/master",
        }
        self.assertTrue(self.impl.candidate_context(base))
        self.assertFalse(self.impl.candidate_context({}))
        self.assertFalse(self.impl.candidate_context({**base, "GITHUB_EVENT_NAME": "pull_request"}))
        self.assertFalse(self.impl.candidate_context({**base, "GITHUB_REPOSITORY": "fork/repo"}))


if __name__ == "__main__":
    unittest.main()
