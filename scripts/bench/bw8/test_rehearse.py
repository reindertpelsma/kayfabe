"""Fixture reconstruction gate checks; never hardware or a production RED."""

import unittest

try:
    from rehearse import SUITES, assert_green
except ModuleNotFoundError:
    SUITES = ()

    def assert_green(rows):
        return False


def rows():
    return [{"suite": name, "ran": 30, "ok": True} for name in SUITES]


class CompleteRehearsal(unittest.TestCase):
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
