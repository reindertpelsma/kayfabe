"""Fixture reconstruction gate checks; never hardware or a production RED."""

import unittest

try:
    from rehearse import SUITES, assert_green
except ModuleNotFoundError:
    SUITES = ()

    def assert_green(rows):
        return False

try:
    from rehearse import isolated_environment
except ImportError:
    def isolated_environment(env):
        return dict(env)


def rows():
    return [{"suite": name, "ran": 30, "ok": True} for name in SUITES]


class CompleteRehearsal(unittest.TestCase):
    def test_git_directory_does_not_escape_into_fixture_repositories(self):
        env = isolated_environment({"GIT_DIR": "shared", "PATH": "tools"})
        self.assertNotIn("GIT_DIR", env)
        self.assertEqual(env["PATH"], "tools")

    def test_git_worktree_and_index_do_not_escape_into_fixture_repositories(self):
        env = isolated_environment({"GIT_WORK_TREE": "shared", "GIT_INDEX_FILE": "shared"})
        self.assertFalse(any(key.startswith("GIT_") for key in env))

    def test_exactly_twenty_seven_inherited_suites(self):
        self.assertEqual(len(SUITES), 27)

    def test_green_complete_bundle(self):
        self.assertTrue(assert_green(rows()))

    def test_missing_first_suite_refused(self):
        self.assertFalse(assert_green(rows()[1:]))

    def test_missing_last_suite_refused(self):
        self.assertFalse(assert_green(rows()[:-1]))

    def test_duplicate_suite_refused(self):
        data = rows()
        if data:
            data[-1] = data[0]
        self.assertFalse(assert_green(data))

    def test_unknown_suite_refused(self):
        data = rows()
        if data:
            data[-1]["suite"] = "replacement"
        self.assertFalse(assert_green(data))

    def test_failed_first_suite_refused(self):
        data = rows()
        if data:
            data[0]["ok"] = False
        self.assertFalse(assert_green(data))

    def test_failed_last_suite_refused(self):
        data = rows()
        if data:
            data[-1]["ok"] = False
        self.assertFalse(assert_green(data))

    def test_zero_tests_is_not_green(self):
        data = rows()
        if data:
            data[0]["ran"] = 0
        self.assertFalse(assert_green(data))

    def test_empty_loader_at_end_is_not_green(self):
        data = rows()
        if data:
            data[-1]["ran"] = 0
        self.assertFalse(assert_green(data))


if __name__ == "__main__":
    unittest.main()
