"""Host resource and privilege refusals must stay in place in this experiment."""

import unittest

from host_guard import Refused, check_memory, check_privilege, check_qemus


class HostGuard(unittest.TestCase):
    def test_owner_qemu_blocks_first_guest(self):
        with self.assertRaises(Refused):
            check_qemus({123: 456}, {})

    def test_foreign_qemu_blocks_pair(self):
        with self.assertRaises(Refused):
            check_qemus({123: 456, 124: 457}, {123: 456})

    def test_reused_pid_is_foreign(self):
        with self.assertRaises(Refused):
            check_qemus({123: 457}, {123: 456})

    def test_exact_owned_child_is_allowed(self):
        check_qemus({123: 456}, {123: 456})

    def test_low_memory_blocks_first_guest(self):
        with self.assertRaises(Refused):
            check_memory(3000, 2048)

    def test_low_memory_blocks_second_guest(self):
        with self.assertRaises(Refused):
            check_memory(4000, 2048)

    def test_headroom_is_required(self):
        check_memory(4096, 2048)

    def test_root_is_refused(self):
        with self.assertRaises(Refused):
            check_privilege(0, 0, 0, {108}, 108, 1002)

    def test_effective_capability_is_refused(self):
        with self.assertRaises(Refused):
            check_privilege(1002, 1, 0, {108}, 108, 1002)

    def test_permitted_capability_is_refused(self):
        with self.assertRaises(Refused):
            check_privilege(1002, 0, 1, {108}, 108, 1002)

    def test_sudo_group_is_refused(self):
        with self.assertRaises(Refused):
            check_privilege(1002, 0, 0, {108, 27}, 108, 1002)

    def test_video_group_is_refused(self):
        with self.assertRaises(Refused):
            check_privilege(1002, 0, 0, {108, 44}, 108, 1002)

    def test_no_kvm_group_is_refused(self):
        with self.assertRaises(Refused):
            check_privilege(1002, 0, 0, {1002}, 108, 1002)

    def test_plain_user_with_kvm_is_accepted(self):
        check_privilege(1002, 0, 0, {108, 1002}, 108, 1002)


if __name__ == "__main__":
    unittest.main()
