"""Semantic port of the already-tested scoped blob helper into the payload lineage."""

import json
import os
from pathlib import Path
import stat
import subprocess
import tempfile
import unittest

import test_vast_box as box

setUpModule = box.setUpModule
tearDownModule = box.tearDownModule
HERE = Path(__file__).resolve().parent


class SetupBlob(unittest.TestCase):
    def fake_curl(self, root):
        path = root / "curl"
        path.write_text(
            "#!/usr/bin/python3\nimport sys,json\n"
            "print(json.dumps({'argv':sys.argv[1:],'input':sys.stdin.read()}))\n"
        )
        path.chmod(0o700)

    def probe(self, argv, prefix="/prefix"):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            self.fake_curl(root)
            envfile = root / "env"
            envfile.write_text(
                'export KF_BROKER_URL="https://broker.example' + prefix + '"\n'
                'export KF_RUN_ID="4242-7"\nexport KF_RUN_TOKEN="test-scoped-token"\n'
            )
            return subprocess.run(
                ["sh", str(HERE / "kf-box-blob-url"), *argv],
                env={
                    **os.environ,
                    "PATH": str(root) + ":" + os.environ["PATH"],
                    "KF_CI_ENV_FILE": str(envfile),
                },
                capture_output=True,
                text=True,
                timeout=5,
            )

    def test_exact_scoped_path_and_prefix(self):
        p = self.probe(["setup-cache-test", "debs.tar.gz"])
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertIn(
            "https://broker.example/prefix/blob-url/4242-7/setup-cache-test/debs.tar.gz",
            json.loads(p.stdout)["argv"],
        )

    def test_scoped_token_is_stdin_only(self):
        p = self.probe(["setup-cache-test", "vendor.tar.gz"], prefix="")
        self.assertEqual(p.returncode, 0, p.stderr)
        data = json.loads(p.stdout)
        self.assertIn("Authorization: Bearer test-scoped-token", data["input"])
        self.assertNotIn("test-scoped-token", " ".join(data["argv"]))
        self.assertIn("=https", data["argv"])
        self.assertNotIn("--location", data["argv"])

    def test_traversal_refuses(self):
        self.assertNotEqual(self.probe(["setup-cache-test", "../bad"]).returncode, 0)

    def test_encoded_separator_refuses(self):
        self.assertNotEqual(self.probe(["setup-cache-test", "bad%2fname"]).returncode, 0)

    def installed(self, check):
        case = box.Onstart("test_pass_run_posts_result_and_exits_zero")
        case.setUp()
        try:
            broker = case.broker()
            root = case.tree(case.OK_PAYLOAD)
            p = case.onstart(root, broker)
            self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
            check(case)
        finally:
            case.tearDown()

    def test_real_onstart_installs_private_blob_helper(self):
        def check(case):
            path = case.bin / "kf-box-blob-url"
            self.assertTrue(path.is_file())
            self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o700)

        self.installed(check)

    def test_installed_helper_reads_the_validated_run_contract(self):
        def check(case):
            self.fake_curl(case.bin)
            p = subprocess.run(
                [str(case.bin / "kf-box-blob-url"), "setup-cache-test", "debs.tar.gz"],
                env={
                    **os.environ,
                    "PATH": str(case.bin) + ":" + os.environ["PATH"],
                    "KF_CI_ENV_FILE": str(case.run_dir / "env"),
                },
                capture_output=True,
                text=True,
                timeout=5,
            )
            self.assertEqual(p.returncode, 0, p.stderr)
            data = json.loads(p.stdout)
            self.assertIn("/blob-url/4242-7/setup-cache-test/debs.tar.gz", data["argv"][-1])
            self.assertIn(box.TOKEN, data["input"])

        self.installed(check)


if __name__ == "__main__":
    unittest.main()
