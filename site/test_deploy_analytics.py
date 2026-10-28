"""Execute the workflow's actual isolated inline Python; all snippet tokens are synthetic."""

import io
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import textwrap
import unittest

WORKFLOW = Path(__file__).resolve().parents[1] / ".github/workflows/site.yml"
SNIPPET = (
    "<!-- Cloudflare Web Analytics -->"
    "<script type='module' src='https://static.cloudflareinsights.com/beacon.min.js' "
    "data-cf-beacon='{\"token\":\"fixture-token\"}'></script>"
    "<!-- End Cloudflare Web Analytics -->"
)
PAGE = '<!doctype html><html><head><title>Fixture</title></head><body>Hi</body></html>'
TRUSTED = {
    "GITHUB_ACTIONS": "true",
    "GITHUB_REPOSITORY": "reindertpelsma/kayfabe",
    "GITHUB_JOB": "deploy",
    "GITHUB_EVENT_NAME": "push",
    "GITHUB_REF": "refs/heads/master",
    "SITE_DEPLOY_ENVIRONMENT": "github-pages",
}


def inline_source():
    text = WORKFLOW.read_text()
    return textwrap.dedent(text.split("          python3 -I - <<'PY'\n")[1].split("          PY\n")[0])


class Analytics(unittest.TestCase):
    # FALSIFIER: inline deploy code corrupts a page, accepts active input, or enters a PR build.
    def setUp(self):
        self.source = inline_source()
        self.temp = tempfile.TemporaryDirectory(prefix="site-inline-analytics-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "pages-deploy").mkdir()
        self.archive = self.root / "pages-deploy/artifact.tar"
        self.output = self.root / "pages-patched"
        self.make_archive({"index.html": PAGE, "docs/master/FAQ.html": PAGE, "style.css": "body{}"})

    def make_archive(self, pages):
        with tarfile.open(self.archive, "w") as target:
            for name, text in pages.items():
                data = text if isinstance(text, bytes) else text.encode()
                member = tarfile.TarInfo(name)
                member.size = len(data)
                target.addfile(member, io.BytesIO(data))

    def run_inline(self, snippet=SNIPPET, context=None):
        env = {k: v for k, v in os.environ.items()
               if not k.startswith(("GITHUB_", "SITE_", "WEB_ANALYTICS_"))}
        env.update(TRUSTED if context is None else context)
        env.update(RUNNER_TEMP=str(self.root), GITHUB_OUTPUT=str(self.root / "outputs"))
        if snippet is not None:
            env["WEB_ANALYTICS_SNIPPET"] = snippet
        result = subprocess.run(["python3", "-I", "-"], input=self.source, env=env,
                                text=True, capture_output=True, check=False)
        self.assertNotIn("fixture-token", result.stdout + result.stderr)
        return result

    def test_comments_script_strings_and_escaped_code_do_not_forge_head(self):
        for extra in ['<!-- </head> -->', '<script>let x="</head>";</script>',
                      '<style>x::after{content:"</head>"}</style>', '<![CDATA[ </head> ]]>']:
            with self.subTest(extra=extra):
                page = PAGE.replace("<title>", extra + "<title>")
                page = page.replace("Hi", '<pre>&lt;/head&gt;</pre>')
                self.make_archive({"index.html": page})
                self.assertEqual(self.run_inline().returncode, 0)
                self.assertEqual((self.output / "index.html").read_bytes(),
                                 page.replace("</title></head>", "</title>" + SNIPPET + "</head>").encode())

    def test_uppercase_spaced_head_and_utf8_bytes_are_preserved(self):
        page = '<HTML><HEAD title="</head>"><TITLE>café</TITLE></HEAD ><BODY>λ</BODY></HTML>'
        self.make_archive({"index.html": page})
        self.assertEqual(self.run_inline().returncode, 0)
        self.assertEqual((self.output / "index.html").read_bytes(),
                         page.replace("</TITLE></HEAD >", "</TITLE>" + SNIPPET + "</HEAD >").encode())

    def test_every_html_gets_one_snippet_inside_head(self):
        self.assertEqual(self.run_inline().returncode, 0)
        for path in self.output.rglob("*.html"):
            text = path.read_text()
            self.assertEqual(text.count(SNIPPET), 1)
            self.assertLess(text.index(SNIPPET), text.index("</head>"))
        self.assertEqual((self.output / "style.css").read_bytes(), b"body{}")

    def test_repeated_transform_is_byte_identical(self):
        self.assertEqual(self.run_inline().returncode, 0)
        pages = {p.relative_to(self.output).as_posix(): p.read_bytes()
                 for p in self.output.rglob("*") if p.is_file()}
        self.make_archive(pages)
        self.assertEqual(self.run_inline().returncode, 0)
        self.assertEqual({p.relative_to(self.output).as_posix(): p.read_bytes()
                          for p in self.output.rglob("*") if p.is_file()}, pages)

    def test_already_instrumented_page_is_not_duplicated(self):
        page = PAGE.replace("</head>", SNIPPET + "</head>")
        self.make_archive({"index.html": page})
        self.assertEqual(self.run_inline().returncode, 0)
        self.assertEqual((self.output / "index.html").read_bytes(), page.encode())

    def test_empty_or_unset_does_not_extract_or_modify_archive(self):
        before = self.archive.read_bytes()
        for snippet in ["", None]:
            self.assertEqual(self.run_inline(snippet).returncode, 0)
            self.assertFalse(self.output.exists())
            self.assertEqual(self.archive.read_bytes(), before)

    def test_invalid_snippets_warn_and_keep_original_artifact(self):
        values = [SNIPPET.replace("</script>", "alert(1)</script>"), SNIPPET + SNIPPET,
                  SNIPPET + '<img src="https://example.invalid/x">',
                  SNIPPET.replace("static.cloudflareinsights.com", "example.invalid"),
                  SNIPPET.replace("type='module'", "onclick='x()'"), "text" + SNIPPET,
                  SNIPPET.replace('"token":"fixture-token"', '"other":"fixture-token"'),
                  SNIPPET.replace('"token":"fixture-token"', '"token":3')]
        for value in values:
            with self.subTest(value=value):
                self.assertEqual(self.run_inline(value).returncode, 0)
                self.assertFalse(self.output.exists())
                self.assertEqual((self.root / "outputs").read_text().splitlines()[-1],
                                 "artifact_name=github-pages")

    def test_inline_script_body_is_never_executed_or_accepted(self):
        self.assertEqual(self.run_inline(SNIPPET.replace("</script>", "bad()</script>")).returncode, 0)
        self.assertFalse(self.output.exists())

    def test_wrong_beacon_host_cannot_receive_injected_script(self):
        self.assertEqual(self.run_inline(SNIPPET.replace("static.cloudflareinsights.com",
                                                        "wrong.invalid")).returncode, 0)
        self.assertFalse(self.output.exists())

    def test_comment_containing_head_close_is_inert(self):
        value = "<!-- </head> -->" + SNIPPET
        self.assertEqual(self.run_inline(value).returncode, 0)
        self.assertEqual((self.output / "index.html").read_bytes(),
                         PAGE.replace("</head>", value + "</head>").encode())

    def test_bad_pages_are_skipped_without_blocking_good_pages(self):
        for bad in ["<html><body>no head</body></html>", "<head></head><head></head>",
                    "<head><title>unclosed</head>"]:
            with self.subTest(bad=bad):
                self.make_archive({"ok.html": PAGE, "bad.html": bad})
                run = self.run_inline()
                self.assertEqual(run.returncode, 0, run.stderr)
                self.assertIn("bad.html", run.stdout)
                self.assertEqual((self.output / "bad.html").read_bytes(), bad.encode())
                self.assertEqual((self.output / "ok.html").read_text().count(SNIPPET), 1)

    def test_zero_eligible_pages_fail_the_patch_step(self):
        self.make_archive({"no-head.html": "<html><body>nothing</body></html>"})
        self.assertNotEqual(self.run_inline().returncode, 0)

    def test_missing_heads_never_receive_a_snippet(self):
        self.make_archive({"bad.html": "<body>no head</body>", "ok.html": PAGE})
        self.assertEqual(self.run_inline().returncode, 0)
        self.assertEqual((self.output / "bad.html").read_bytes(), b"<body>no head</body>")

    def test_local_pr_and_fork_contexts_remain_beacon_free(self):
        for context in [{}, {**TRUSTED, "GITHUB_EVENT_NAME": "pull_request"},
                        {**TRUSTED, "GITHUB_REPOSITORY": "fork/repo"}]:
            self.assertEqual(self.run_inline(context=context).returncode, 0)
            self.assertFalse(self.output.exists())

    def test_build_job_context_does_not_extract(self):
        self.assertEqual(self.run_inline(context={**TRUSTED, "GITHUB_JOB": "build"}).returncode, 0)
        self.assertFalse(self.output.exists())

    def test_snippet_shell_syntax_is_only_attribute_data(self):
        value = SNIPPET.replace("fixture-token", "$(touch shell-executed)")
        self.assertEqual(self.run_inline(value).returncode, 0)
        self.assertFalse((self.root / "shell-executed").exists())
        self.assertIn(value.encode(), (self.output / "index.html").read_bytes())


if __name__ == "__main__":
    unittest.main()
