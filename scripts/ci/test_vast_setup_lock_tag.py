"""STATUS: LIVE 2026-11-11. The setup-cache lock tag must be mintable by the Worker's blob route.

Falsifier: the lock names a tag the Worker refuses (outside BLOB_ALLOWED_TAG_PREFIXES, or absent from
BLOB_REPO), and the box fails with an unnamed setup error. The Worker side is
ci/vast-broker/test/setup-cache-blob.test.js; this file checks the lock's prefix and that a refused
locator surfaces as the named KF_SETUP_CACHE_REFUSED phase=locator_<kind> and never as a pass.
"""

import contextlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(HERE))
import vast_setup  # noqa: E402

PREFIX = "setup-cache-"


def lock():
    return json.loads((ROOT / "scripts/ci/setup-cache.lock.json").read_text())


class LockTag(unittest.TestCase):
    def test_tag_has_the_prefix_the_worker_var_must_list(self):
        self.assertTrue(lock()["tag"].startswith(PREFIX), lock()["tag"])
        design = (ROOT / "docs/design/VAST_SETUP_CACHE_20261023.md").read_text()
        self.assertIn("`" + PREFIX + "`", design)

    def test_tag_is_a_strict_blob_route_segment(self):
        tag = lock()["tag"]
        self.assertRegex(tag, r"^[A-Za-z0-9._-]{1,64}$")
        self.assertNotIn(tag, (".", ".."))

    def test_a_refused_locator_prints_the_named_reason_and_fails(self):
        for kind in [a["kind"] for a in lock()["assets"]]:
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as d:
                out = io.StringIO()
                calls = []

                def refused(argv, **kw):
                    calls.append(argv)
                    raise subprocess.CalledProcessError(1, argv)

                only = {**lock(), "assets": [a for a in lock()["assets"] if a["kind"] == kind]}
                with contextlib.ExitStack() as stack:
                    stack.enter_context(patch.dict(os.environ, {"KF_CI_WORKSPACE": d}))
                    stack.enter_context(patch.object(sys, "argv", ["vast_setup.py", "restore", d]))
                    # restore() binds CACHE_ROOT as a default argument, so patch the default.
                    stack.enter_context(
                        patch.object(vast_setup.restore, "__defaults__", (Path(d, "cache"),))
                    )
                    stack.enter_context(patch.object(vast_setup, "validate_lock"))
                    stack.enter_context(patch.object(vast_setup, "wait_for_ready"))
                    stack.enter_context(patch.object(vast_setup, "policy_digest", return_value=""))
                    stack.enter_context(patch.object(subprocess, "check_output", refused))
                    stack.enter_context(patch.object(Path, "read_text", _lock_reader(only)))
                    stack.enter_context(contextlib.redirect_stdout(out))
                    rc = vast_setup.main()
                self.assertEqual(rc, 2)
                self.assertEqual(calls[0][1:], [lock()["tag"], only["assets"][0]["name"]])
                self.assertIn("KF_SETUP_CACHE_REFUSED phase=locator_" + kind + "\n", out.getvalue())
                result = json.loads(Path(d, "kf-result.json").read_text())
                self.assertEqual(result["verdict"], "FAIL")


def _lock_reader(replacement):
    real = Path.read_text

    def read_text(self, *a, **k):
        if self.name == "setup-cache.lock.json":
            return json.dumps(replacement)
        return real(self, *a, **k)

    return read_text


if __name__ == "__main__":
    unittest.main()
