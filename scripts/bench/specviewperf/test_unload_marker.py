import unittest
import kayfabe_arm as arm


class UnloadMarker(unittest.TestCase):
    def test_unload_without_leftover_objects_is_counted(self):
        self.assertEqual(arm.unload_markers('kf3: GSP fn 47 UNLOADING_GUEST_DRIVER seq=0: newLevel=0\n'),1)

    def test_multiple_clean_unloads_are_counted(self):
        self.assertEqual(arm.unload_markers('kf3: GSP fn 47 UNLOADING_GUEST_DRIVER seq=0\n'*3),3)

    def test_optional_leftover_summary_is_not_a_second_cycle(self):
        self.assertEqual(arm.unload_markers('kf3: GSP fn 47 UNLOADING_GUEST_DRIVER seq=0\nkf3: GSP fn 47: the guest RM unloaded with 3 client(s)\n'),1)

    def test_summary_alone_is_not_a_new_unload(self):
        self.assertEqual(arm.unload_markers('kf3: GSP fn 47: the guest RM unloaded with 3 client(s)\n'),0)
