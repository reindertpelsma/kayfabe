# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Long-act inventory requirements; handles are never interpreted as PIDs."""
import unittest

import long_acts

TIMESLICE = "kf3: act timeslice: 0xc1d000a9:0xff0e0000 SET_TIMESLICE 400 us on 1 twin group(s) (875734 us, off the GSP lock)"
ENGINE = "kf3: act engine object: 0xc1d00183:0xff1fe032 class 0xc9c0 (Compute) on twin host 0x6b -> host object 0xcafe6137 (30104 us, off the GSP lock)"


class LongActs(unittest.TestCase):
    def test_timeslice_quantum_is_not_act_duration(self):
        row = long_acts.parse_lines([TIMESLICE])["acts"][0]
        self.assertEqual(row["duration_us"], 875734)
        self.assertEqual(row["api"], "SET_TIMESLICE")
        self.assertEqual(row["requested_timeslice_us"], 400)

    def test_engine_class_and_guest_handle_are_preserved(self):
        row = long_acts.parse_lines([ENGINE])["acts"][0]
        self.assertEqual(row["api"], "RM_ALLOC")
        self.assertEqual(row["object_class"], "0xc9c0")
        self.assertEqual(row["guest_client"], "0xc1d00183")
        self.assertEqual(row["guest_object"], "0xff1fe032")

    def test_timeslice_target_object_class_is_unknown_from_control_id(self):
        row = long_acts.parse_lines([TIMESLICE])["acts"][0]
        self.assertIsNone(row["object_class"])
        self.assertEqual(row["control_id"], "0xa06c0103")

    def test_unknown_guest_waiter_is_not_synthesized_from_handles(self):
        row = long_acts.parse_lines([ENGINE])["acts"][0]
        self.assertIsNone(row["guest_waiter_pid"])
        self.assertIsNone(row["guest_waiter_tid"])

    def test_unknown_group_mode_is_not_inferred_from_twin_token(self):
        self.assertIsNone(long_acts.parse_lines([ENGINE])["acts"][0]["group_mode"])

    def test_unknown_uptime_remains_unknown_without_event_clock(self):
        row = long_acts.parse_lines([TIMESLICE], vm_start_ns=0)["acts"][0]
        self.assertIsNone(row["vm_uptime_ns"])
        self.assertIsNone(row["ts_ns"])

    def test_explicit_event_end_clock_can_be_converted(self):
        line = "act_end_mono_ns=1000000001 " + ENGINE
        row = long_acts.parse_lines([line], vm_start_ns=1)["acts"][0]
        self.assertEqual(row["vm_uptime_ns"], 1_000_000_000)

    def test_tail_arrival_timestamp_is_not_an_act_end_clock(self):
        row = long_acts.parse_lines(["mono_ns=1000000001 " + ENGINE], vm_start_ns=1)["acts"][0]
        self.assertIsNone(row["vm_uptime_ns"])

    def test_threshold_is_strictly_greater_than_twenty_ms(self):
        row = ENGINE.replace("30104 us", "20000 us")
        self.assertEqual(long_acts.parse_lines([row])["acts"], [])

    def test_short_act_does_not_enter_long_table(self):
        self.assertEqual(long_acts.parse_lines([ENGINE.replace("30104 us", "19999 us")])["acts"], [])

    def test_unparsed_act_lines_are_explicitly_counted(self):
        r = long_acts.parse_lines(["unrelated", "kf3: act engine object: without a duration", ENGINE])
        self.assertEqual(r["unparsed_act_lines"], 1)
        self.assertEqual(r["timed_act_lines"], 1)
        self.assertEqual(r["acts"][0]["line"], 3)

    def test_report_marks_missing_process_and_group_unknown(self):
        table = long_acts.format_table(long_acts.parse_lines([ENGINE]))
        self.assertIn("UNKNOWN", table)
        self.assertIn("guest_waiter_pid", table)
        self.assertIn("group_mode", table)


if __name__ == "__main__":
    unittest.main()
