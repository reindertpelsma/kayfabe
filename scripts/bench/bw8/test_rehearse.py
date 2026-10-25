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

    def test_extra_first_suite_refused(self):
        data = rows()
        self.assertFalse(assert_green(data + data[:1]))

    def test_extra_last_suite_refused(self):
        data = rows()
        self.assertFalse(assert_green(data + data[-1:]))

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


try:
    from rehearse import complete_receipt
except ImportError:

    def complete_receipt(body):
        return False


def receipt():
    return {
        "suites": rows(),
        "static": [],
        "operator": {
            "budget_watch": {"absence_receipt": True, "destroy_called": True},
            "precreate_watchdog": {"state": "ARMED", "result": "no contract before deadline"},
        },
    }


class CompleteReceipt(unittest.TestCase):
    def test_complete_receipt_passes(self):
        self.assertTrue(complete_receipt(receipt()))

    def test_first_static_problem_refused(self):
        body = receipt()
        body["static"] = [["module", 1, "undefined"]]
        self.assertFalse(complete_receipt(body))

    def test_other_static_problem_refused(self):
        body = receipt()
        body["static"] = [["operator", 99, "unresolved attribute"]]
        self.assertFalse(complete_receipt(body))

    def test_budget_absence_missing_refused(self):
        body = receipt()
        body["operator"]["budget_watch"]["absence_receipt"] = False
        self.assertFalse(complete_receipt(body))

    def test_budget_destroy_missing_refused(self):
        body = receipt()
        body["operator"]["budget_watch"]["destroy_called"] = False
        self.assertFalse(complete_receipt(body))

    def test_precreate_not_armed_refused(self):
        body = receipt()
        body["operator"]["precreate_watchdog"]["state"] = "STOPPED"
        self.assertFalse(complete_receipt(body))

    def test_precreate_exit_missing_refused(self):
        body = receipt()
        body["operator"]["precreate_watchdog"]["result"] = "unknown"
        self.assertFalse(complete_receipt(body))

    def test_omitted_operator_refused(self):
        body = receipt()
        body.pop("operator")
        self.assertFalse(complete_receipt(body))

    def test_empty_operator_refused(self):
        body = receipt()
        body["operator"] = {}
        self.assertFalse(complete_receipt(body))


if __name__ == "__main__":
    unittest.main()
