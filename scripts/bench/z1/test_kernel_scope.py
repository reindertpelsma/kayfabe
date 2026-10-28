"""A scoped journal must retain its exact starting cursor and boot identity."""

import unittest

from kernel_scope import ScopeRefused, decode

SCOPE = {"cursor": "s=abc;i=1", "boot": "abc"}


def row(cursor="s=abc;i=1", boot="abc", message="NVRM: fixture"):
    return {
        "__CURSOR": cursor,
        "_BOOT_ID": boot,
        "_TRANSPORT": "kernel",
        "__REALTIME_TIMESTAMP": "1793142000000000",
        "MESSAGE": message,
    }


class KernelScope(unittest.TestCase):
    def test_missing_start_cursor_refuses(self):
        with self.assertRaises(ScopeRefused):
            decode([row(cursor="s=abc;i=2")], SCOPE)

    def test_vacuumed_prefix_refuses(self):
        with self.assertRaises(ScopeRefused):
            decode([row(cursor="s=abc;i=9"), row(cursor="s=abc;i=10")], SCOPE)

    def test_reboot_refuses(self):
        with self.assertRaises(ScopeRefused):
            decode([row(boot="def")], SCOPE)

    def test_mixed_boot_refuses(self):
        with self.assertRaises(ScopeRefused):
            decode([row(), row(cursor="s=abc;i=2", boot="def")], SCOPE)

    def test_non_kernel_transport_refuses(self):
        entry = row()
        entry["_TRANSPORT"] = "syslog"
        with self.assertRaises(ScopeRefused):
            decode([entry], SCOPE)

    def test_missing_transport_refuses(self):
        entry = row()
        del entry["_TRANSPORT"]
        with self.assertRaises(ScopeRefused):
            decode([entry], SCOPE)

    def test_empty_read_refuses(self):
        with self.assertRaises(ScopeRefused):
            decode([], SCOPE)

    def test_cursor_and_new_fault_are_retained(self):
        text = decode([row(), row(cursor="s=abc;i=2", message="NVRM: Xid new")], SCOPE)
        self.assertIn("kernel: NVRM: fixture", text)
        self.assertIn("kernel: NVRM: Xid new", text)
        self.assertEqual(len(text.splitlines()), 2)


if __name__ == "__main__":
    unittest.main()
