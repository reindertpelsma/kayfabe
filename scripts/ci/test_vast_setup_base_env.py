"""Fresh Vast onstart has KF_* data but can omit ordinary login environment values."""

import os
from pathlib import Path
import pwd
import unittest
from unittest.mock import patch

import test_vast_box as box

setUpModule = box.setUpModule
tearDownModule = box.tearDownModule


class BaseEnvironment(unittest.TestCase):
    def invoke(self, mode):
        case = box.Onstart("test_pass_run_posts_result_and_exits_zero")
        case.setUp()
        try:
            broker = case.broker()
            payload = 'set -u\nprintf "%s" "$HOME" > "$KF_TEST_DUMP.home"\n' + case.OK_PAYLOAD
            root = case.tree(payload)
            expected = pwd.getpwuid(os.geteuid()).pw_dir
            original = box.base_env
            if mode.startswith("explicit"):
                home = case.tmp / ("explicit home" if mode == "explicit_spaces" else "explicit")
                home.mkdir()
                expected = str(home)

            def environment(extra):
                env = original(extra)
                if mode == "missing":
                    env.pop("HOME", None)
                elif mode == "empty":
                    env["HOME"] = ""
                else:
                    env["HOME"] = expected
                return env

            with patch.object(box, "base_env", side_effect=environment):
                process = case.onstart(root, broker)
            self.assertEqual(process.returncode, 0, process.stdout + process.stderr)
            self.assertEqual((case.tmp / "dump.home").read_text(), expected)
            self.assertEqual(broker.results()[0]["verdict"], "PASS")
        finally:
            case.tearDown()

    def test_missing_home_uses_the_current_account_home(self):
        self.invoke("missing")

    def test_empty_home_uses_the_current_account_home(self):
        self.invoke("empty")

    def test_explicit_home_is_preserved(self):
        self.invoke("explicit")

    def test_explicit_home_with_spaces_is_preserved(self):
        self.invoke("explicit_spaces")
