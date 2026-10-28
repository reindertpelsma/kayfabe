"""Deployment analytics boundaries; snippets here are synthetic, never environment values."""

import importlib.util
import io
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
SCRIPT = HERE / "deploy_analytics.py"
SNIPPET = '<script defer src="https://analytics.invalid/beacon.js"></script>'
PAGE = '<!doctype html><html><head><title>Fixture</title></head><body>Hi</body></html>'
TRUSTED = {
    "GITHUB_ACTIONS": "true",
    "GITHUB_REPOSITORY": "reindertpelsma/kayfabe",
    "GITHUB_JOB": "deploy",
    "GITHUB_EVENT_NAME": "push",
    "GITHUB_REF": "refs/heads/master",
    "SITE_DEPLOY_ENVIRONMENT": "github-pages",
}


def module():
    spec = importlib.util.spec_from_file_location("deploy_analytics", SCRIPT)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


def archive(path, pages=None):
    pages = pages or {"index.html": PAGE, "docs/master/FAQ.html": PAGE, "style.css": "body{}"}
    with tarfile.open(path, "w") as output:
        for name, text in pages.items():
            data = text.encode()
            item = tarfile.TarInfo(name)
            item.size = len(data)
            item.mode = 0o644
            output.addfile(item, io.BytesIO(data))


def contents(path):
    with tarfile.open(path) as source:
        return {m.name: source.extractfile(m).read().decode() for m in source if m.isfile()}


class Analytics(unittest.TestCase):
    # FALSIFIER: an ordinary build context gains a beacon, or a deploy page gets duplicates.
    def setUp(self):
        self.impl = module()
        self.temp = tempfile.TemporaryDirectory(prefix="site-analytics-")
        self.addCleanup(self.temp.cleanup)
        self.path = Path(self.temp.name) / "artifact.tar"
        archive(self.path)

    def test_all_deploy_pages_receive_one_snippet_inside_head(self):
        self.impl.transform(self.path, SNIPPET, TRUSTED)
        for name, text in contents(self.path).items():
            if name.endswith(".html"):
                self.assertEqual(text.count(SNIPPET), 1)
                self.assertLess(text.index("<head>"), text.index(SNIPPET))
                self.assertLess(text.index(SNIPPET), text.index("</head>"))

    def test_second_deploy_transform_is_byte_identical(self):
        self.impl.transform(self.path, SNIPPET, TRUSTED)
        before = self.path.read_bytes()
        self.impl.transform(self.path, SNIPPET, TRUSTED)
        self.assertEqual(self.path.read_bytes(), before)

    def test_existing_head_snippet_is_not_duplicated(self):
        text = PAGE.replace("</head>", SNIPPET + "</head>")
        self.assertEqual(self.impl.inject(text, SNIPPET), text)

    def test_comments_and_script_text_cannot_forge_head_end(self):
        text = PAGE.replace(
            "<title>", '<!-- </head> --><script>let s = "</head>";</script><title>'
        )
        result = self.impl.inject(text, SNIPPET)
        self.assertEqual(result.count(SNIPPET), 1)
        self.assertIn("</title>\n" + SNIPPET, result)

    def test_uppercase_head_and_quoted_attribute_are_valid(self):
        text = '<HTML><HEAD data-note="</head>"><TITLE>X</TITLE></HEAD><BODY></BODY></HTML>'
        result = self.impl.inject(text, SNIPPET)
        self.assertIn("</TITLE>\n" + SNIPPET, result)
        self.assertLess(result.index(SNIPPET), result.index("</HEAD>"))

    def test_invalid_heads_fail_before_archive_replacement(self):
        for text in ["<html><body>x</body></html>", "<head></head><head></head>",
                     "<head><title>x</head>", "<body><head></head></body>"]:
            with self.subTest(text=text):
                archive(self.path, {"ok.html": PAGE, "bad.html": text})
                before = self.path.read_bytes()
                with self.assertRaises(ValueError):
                    self.impl.transform(self.path, SNIPPET, TRUSTED)
                self.assertEqual(self.path.read_bytes(), before)

    def test_snippet_in_body_is_rejected(self):
        text = PAGE.replace("Hi", SNIPPET)
        with self.assertRaises(ValueError):
            self.impl.inject(text, SNIPPET)

    def test_duplicate_existing_snippet_is_rejected(self):
        text = PAGE.replace("</head>", SNIPPET * 2 + "</head>")
        with self.assertRaises(ValueError):
            self.impl.inject(text, SNIPPET)

    def test_empty_and_whitespace_leave_archive_bytes_untouched(self):
        before = self.path.read_bytes()
        for snippet in ["", " \n\t"]:
            self.impl.transform(self.path, snippet, TRUSTED)
            self.assertEqual(self.path.read_bytes(), before)

    def test_non_html_members_are_unchanged(self):
        self.impl.transform(self.path, SNIPPET, TRUSTED)
        self.assertEqual(contents(self.path)["style.css"], "body{}")

    def test_each_untrusted_context_is_noop(self):
        for key, value in [("GITHUB_ACTIONS", "false"), ("GITHUB_REPOSITORY", "fork/repo"),
                           ("GITHUB_JOB", "build"), ("GITHUB_EVENT_NAME", "pull_request"),
                           ("GITHUB_REF", "refs/heads/next"),
                           ("SITE_DEPLOY_ENVIRONMENT", "preview")]:
            with self.subTest(key=key):
                before = self.path.read_bytes()
                self.impl.transform(self.path, SNIPPET, {**TRUSTED, key: value})
                self.assertEqual(self.path.read_bytes(), before)

    def test_local_context_does_not_read_accidental_snippet(self):
        before = self.path.read_bytes()
        self.impl.transform(self.path, SNIPPET, {})
        self.assertEqual(self.path.read_bytes(), before)

    def test_publication_events_and_version_tags_are_supported(self):
        for event in ["push", "release", "workflow_dispatch", "schedule"]:
            self.assertTrue(self.impl.eligible({**TRUSTED, "GITHUB_EVENT_NAME": event}))
        self.assertTrue(self.impl.eligible({**TRUSTED, "GITHUB_REF": "refs/tags/v1.0"}))
        self.assertFalse(self.impl.eligible({**TRUSTED, "GITHUB_EVENT_NAME": "workflow_run"}))

    def cli(self, context, snippet):
        environment = {k: v for k, v in os.environ.items()
                       if not k.startswith(("GITHUB_", "SITE_", "WEB_ANALYTICS_"))}
        environment.update(context)
        if snippet is not None:
            environment["WEB_ANALYTICS_SNIPPET"] = snippet
        result = subprocess.run(["python3", str(SCRIPT), str(self.path)], env=environment,
                                capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn(SNIPPET, result.stdout + result.stderr)

    def test_cli_local_pr_and_fork_outputs_stay_beacon_free(self):
        for context in [{}, {**TRUSTED, "GITHUB_EVENT_NAME": "pull_request"},
                        {**TRUSTED, "GITHUB_REPOSITORY": "fork/repo"}]:
            self.cli(context, SNIPPET)
            self.assertNotIn(SNIPPET, self.path.read_bytes().decode(errors="replace"))

    def test_cli_unset_deploy_variable_is_noop(self):
        before = self.path.read_bytes()
        self.cli(TRUSTED, None)
        self.assertEqual(self.path.read_bytes(), before)

    def test_cli_deploy_injects_environment_html_without_logging_it(self):
        self.cli(TRUSTED, SNIPPET)
        self.assertEqual(contents(self.path)["index.html"].count(SNIPPET), 1)


if __name__ == "__main__":
    unittest.main()
