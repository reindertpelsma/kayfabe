"""Run the actual fixed deploy inline script against synthetic snippets and byte fixtures."""

import ast
import io
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import textwrap
import unittest

WORKFLOW = Path(__file__).resolve().parents[1] / ".github/workflows/site.yml"
SLOT = b"<!-- kf-slot-a -->"
SNIPPET = (
    "<!-- Cloudflare Web Analytics -->"
    "<script type='module' src='https://static.cloudflareinsights.com/beacon.min.js' "
    'data-cf-beacon=\'{"token":"fixture-token"}\'></script>'
    "<!-- End Cloudflare Web Analytics -->"
)
PAGE = b"<html><head>" + SLOT + b'</head><body><footer id="kf-footer"></footer></body></html>'
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
    return textwrap.dedent(
        text.split("          python3 -I - <<'PY'\n")[1].split("          PY\n")[0]
    )


def privacy_bytes(source):
    for node in ast.parse(source).body:
        if isinstance(node, ast.Assign) and any(
            isinstance(t, ast.Name) and t.id == "privacy" for t in node.targets
        ):
            return ast.literal_eval(node.value)
    raise AssertionError("fixed privacy script must live in the workflow")


class Deployment(unittest.TestCase):
    def setUp(self):
        self.source = inline_source()
        self.privacy = privacy_bytes(self.source)
        self.temp = tempfile.TemporaryDirectory(prefix="slot-deploy-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "pages-deploy").mkdir()
        self.archive = self.root / "pages-deploy/artifact.tar"
        self.folder = self.root / "pages-patched"
        self.pages = {"index.html": PAGE, "docs/master/FAQ.html": PAGE, "style.css": b"body{}"}
        self.pack()

    def pack(self):
        with tarfile.open(self.archive, "w") as target:
            for name, data in self.pages.items():
                item = tarfile.TarInfo(name)
                item.size = len(data)
                target.addfile(item, io.BytesIO(data))

    def run_inline(self, snippet=SNIPPET, context=None):
        env = {
            k: v
            for k, v in os.environ.items()
            if not k.startswith(("GITHUB_", "SITE_", "WEB_ANALYTICS_"))
        }
        env.update(TRUSTED if context is None else context)
        env.update(RUNNER_TEMP=str(self.root))
        if snippet is not None:
            env["WEB_ANALYTICS_SNIPPET"] = snippet
        result = subprocess.run(
            ["python3", "-I", "-"],
            input=self.source,
            env=env,
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertNotIn("fixture-token", result.stdout + result.stderr)
        return result

    def test_valid_snippet_replaces_exact_slot_once(self):
        self.assertEqual(self.run_inline().returncode, 0)
        expected = SNIPPET.encode() + self.privacy
        for name, raw in self.pages.items():
            self.assertEqual((self.folder / name).read_bytes(), raw.replace(SLOT, expected))

    def test_head_lookalikes_and_binary_bytes_outside_slot_are_untouched(self):
        raw = (
            b'<html><head><!-- </head> --><script>let s="</head>";</script>'
            b'<style>x{content:"</head>"}</style>'
            + SLOT
            + b"</HEAD ><body><pre>&lt;/head&gt;</pre>\r\n\xff</body></html>"
        )
        self.pages = {"index.html": raw}
        self.pack()
        self.assertEqual(self.run_inline().returncode, 0)
        self.assertEqual(
            (self.folder / "index.html").read_bytes(),
            raw.replace(SLOT, SNIPPET.encode() + self.privacy),
        )

    def test_empty_or_absent_variable_removes_slot_end_to_end(self):
        for snippet in ["", None]:
            self.assertEqual(self.run_inline(snippet).returncode, 0)
            for name, raw in self.pages.items():
                self.assertEqual((self.folder / name).read_bytes(), raw.replace(SLOT, b""))

    def test_invalid_variable_removes_slots_and_deploys_without_beacon(self):
        values = [
            SNIPPET + SNIPPET,
            SNIPPET + '<img src="x">',
            SNIPPET.replace("</script>", "alert(1)</script>"),
            SNIPPET.replace("static.cloudflareinsights.com", "wrong.invalid"),
            SNIPPET.replace("type='module'", "onclick='x()'"),
            SNIPPET.replace('"token":"fixture-token"', '"token":3'),
        ]
        for value in values:
            self.assertEqual(self.run_inline(value).returncode, 0)
            self.assertEqual((self.folder / "index.html").read_bytes(), PAGE.replace(SLOT, b""))

    def test_inline_body_is_never_injected(self):
        self.assertEqual(
            self.run_inline(SNIPPET.replace("</script>", "bad()</script>")).returncode, 0
        )
        self.assertNotIn(b"bad()", (self.folder / "index.html").read_bytes())

    def test_wrong_host_never_gets_a_script(self):
        self.assertEqual(
            self.run_inline(
                SNIPPET.replace("static.cloudflareinsights.com", "wrong.invalid")
            ).returncode,
            0,
        )
        self.assertNotIn(b"wrong.invalid", (self.folder / "index.html").read_bytes())

    def test_empty_value_has_no_privacy_script(self):
        self.assertEqual(self.run_inline("").returncode, 0)
        raw = (self.folder / "index.html").read_bytes()
        self.assertNotIn(self.privacy, raw)
        self.assertNotIn(b"Web Analytics", raw)

    def test_valid_value_without_any_slot_fails(self):
        self.pages = {"index.html": PAGE.replace(SLOT, b"")}
        self.pack()
        self.assertNotEqual(self.run_inline().returncode, 0)

    def test_valid_input_cannot_succeed_for_slotless_docs_only(self):
        self.pages = {"docs/latest/index.html": b"<html><head></head><body>docs</body></html>"}
        self.pack()
        self.assertNotEqual(self.run_inline().returncode, 0)

    def test_missing_slot_does_not_lose_document_bytes(self):
        self.pages = {"index.html": PAGE, "other.html": b"<html>plain</html>"}
        self.pack()
        self.assertEqual(self.run_inline().returncode, 0)
        self.assertEqual((self.folder / "other.html").read_bytes(), b"<html>plain</html>")

    def test_surrounding_snippet_whitespace_is_preserved(self):
        value = " \n" + SNIPPET + "\n\t"
        self.assertEqual(self.run_inline(value).returncode, 0)
        self.assertEqual(
            (self.folder / "index.html").read_bytes(),
            PAGE.replace(SLOT, value.encode() + self.privacy),
        )

    def test_local_pr_and_fork_contexts_cannot_patch(self):
        for context in [
            {},
            {**TRUSTED, "GITHUB_EVENT_NAME": "pull_request"},
            {**TRUSTED, "GITHUB_REPOSITORY": "fork/repo"},
        ]:
            self.assertEqual(self.run_inline(context=context).returncode, 0)
            self.assertFalse(self.folder.exists())

    def test_build_context_does_not_patch(self):
        self.assertEqual(self.run_inline(context={**TRUSTED, "GITHUB_JOB": "build"}).returncode, 0)
        self.assertFalse(self.folder.exists())


if __name__ == "__main__":
    unittest.main()
