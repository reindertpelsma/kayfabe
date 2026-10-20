#!/usr/bin/env python3
"""GPU-free tests for wd_parse.py (parser/classifier of the baton self-watchdog dump).

Run from this directory: python3 -I test_wd_parse.py
"""
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import wd_parse as wp  # noqa: E402

SAMPLE = """\
WD_BEGIN label=B pid=219 t=14.501 epoch=1792492660.250 idle_ms=2003.5 iters=1411 launched=1412 sync_active=1
HB label=A age_ms=4.2 iters=1930
STREAMQ rc=0 name=SUCCESS
TASK tid=219 comm=baton state=S wchan=do_sys_poll syscall=7 args=0x7f0000001000,0x1,0xffffffffffffffff
TASK tid=225 comm=cuda-EvtHandlr state=S wchan=do_sys_poll syscall=7 args=0x7f0000002000,0x2,0xffffffffffffffff
TASK tid=226 comm=baton state=S wchan=futex_wait_queue syscall=202 args=0x7f0000003000,0x80,0x0
STACK tid=219
[<0>] do_sys_poll+0x2b0/0x470
[<0>] __x64_sys_poll+0x7a/0x140
ENDSTACK
FD 3 -> /dev/nvidiactl
FD 4 -> /dev/nvidia0
FD 7 -> /dev/nvidia-uvm
FD 9 -> anon_inode:[eventfd]
PROC /proc/driver/nvidia-uvm/information
Driver version: 595.91.07
ENDPROC
WD_END label=B
WD_RECOVER label=B t=17.9 gap_ms=3400.1
"""

NOT_READY = SAMPLE.replace("STREAMQ rc=0 name=SUCCESS", "STREAMQ rc=600 name=NOT_READY")


class Parse(unittest.TestCase):
    def test_episode_header(self):
        eps = wp.parse(SAMPLE)
        self.assertEqual(len(eps), 1)
        e = eps[0]
        self.assertEqual(e["label"], "B")
        self.assertEqual(e["pid"], 219)
        self.assertAlmostEqual(e["t"], 14.501)
        self.assertAlmostEqual(e["idle_ms"], 2003.5)
        self.assertEqual(e["sync_active"], 1)

    def test_tasks_fds_hb(self):
        e = wp.parse(SAMPLE)[0]
        self.assertEqual([t["tid"] for t in e["tasks"]], [219, 225, 226])
        self.assertEqual(e["tasks"][0]["wchan"], "do_sys_poll")
        self.assertEqual(e["tasks"][2]["syscall"], 202)
        self.assertEqual(e["fds"][4], "/dev/nvidia0")
        self.assertEqual(e["hb"]["A"]["age_ms"], 4.2)
        self.assertEqual(e["streamq"], 0)
        self.assertIn(219, e["stacks"])
        self.assertIn("do_sys_poll", e["stacks"][219][0])
        self.assertIn("/proc/driver/nvidia-uvm/information", e["procs"])

    def test_recovery_attached(self):
        e = wp.parse(SAMPLE)[0]
        self.assertAlmostEqual(e["recover_gap_ms"], 3400.1)

    def test_multiple_and_garbage_lines(self):
        eps = wp.parse("noise\n" + SAMPLE + "BATON: x\n" + SAMPLE.replace("t=14.501", "t=30.0"))
        self.assertEqual([round(e["t"], 1) for e in eps], [14.5, 30.0])

    def test_unterminated_episode_kept(self):
        eps = wp.parse(SAMPLE.split("WD_END")[0])
        self.assertEqual(len(eps), 1)
        self.assertTrue(eps[0]["truncated"])


class Classify(unittest.TestCase):
    def test_stream_done_but_waiter_blocked_is_lost_wakeup(self):
        v = wp.classify(wp.parse(SAMPLE)[0])
        self.assertEqual(v["kind"], "LOST_WAKEUP")

    def test_stream_not_ready_is_gpu_side(self):
        v = wp.classify(wp.parse(NOT_READY)[0])
        self.assertEqual(v["kind"], "GPU_NOT_COMPLETE")

    def test_no_streamq_is_unknown(self):
        txt = SAMPLE.replace("STREAMQ rc=0 name=SUCCESS\n", "")
        self.assertEqual(wp.classify(wp.parse(txt)[0])["kind"], "UNKNOWN")

    def test_blocked_thread_summary_names_syscall_and_fd(self):
        v = wp.classify(wp.parse(SAMPLE)[0])
        self.assertIn("poll", v["main_wait"])
        self.assertIn("wchan=do_sys_poll", v["main_wait"])

    def test_syscall_names(self):
        self.assertEqual(wp.syscall_name(16), "ioctl")
        self.assertEqual(wp.syscall_name(202), "futex")
        self.assertEqual(wp.syscall_name(7), "poll")
        self.assertEqual(wp.syscall_name(99999), "sys_99999")

    def test_summary_text(self):
        txt = wp.summarize(wp.parse(SAMPLE))
        self.assertIn("LOST_WAKEUP", txt)
        self.assertIn("B", txt)


if __name__ == "__main__":
    unittest.main()
