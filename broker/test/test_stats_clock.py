#!/usr/bin/env python3
"""The broker's diagnostic clock, extracted from the production source and instrumented."""
import os
from pathlib import Path
import re
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]

class StatsClock(unittest.TestCase):
    def sample(self, enabled):
        source = (ROOT / "nvkvm_broker.c").read_text()
        match = re.search(r"static uint64_t nb_stats_now_ns\(bool enabled\)\s*\{", source)
        self.assertIsNotNone(match, "stats-only poll timestamps need an explicit default-off clock gate")
        start = match.start()
        end = source.index("\n}", match.end()) + 2
        helper = source[start:end]
        code = r'''#include <stdbool.h>
#include <stdint.h>
#include <time.h>
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
static int calls;
static int fake_clock_gettime(clockid_t id, struct timespec *ts) {
    (void)id; calls++; errno = EIO; ts->tv_sec = 2; ts->tv_nsec = 5; return 0;
}
#define clock_gettime fake_clock_gettime
''' + helper + r'''
int main(int argc, char **argv) {
    (void)argc; errno = EBUSY;
    uint64_t value = nb_stats_now_ns(atoi(argv[1]) != 0);
    printf("%llu %d %d\n", (unsigned long long)value, calls, errno == EBUSY);
    return 0;
}
'''
        with tempfile.TemporaryDirectory(prefix="kf-stats-clock-") as tmp:
            src = Path(tmp) / "probe.c"
            exe = Path(tmp) / "probe"
            src.write_text(code)
            subprocess.run([os.environ.get("CC", "cc"), "-std=gnu11", "-O2", "-Wall", "-Wextra", "-Werror", str(src), "-o", str(exe)], check=True, capture_output=True)
            return tuple(map(int, subprocess.check_output([str(exe), str(int(enabled))], text=True).split()))

    def test_off_does_not_read_the_clock(self):
        self.assertEqual(self.sample(False)[:2], (0, 0))

    def test_off_leaves_the_system_error_state_untouched(self):
        self.assertEqual(self.sample(False)[2], 1)

    def test_on_keeps_the_legacy_nanosecond_value(self):
        self.assertEqual(self.sample(True)[0], 2_000_000_005)

    def test_on_reads_the_clock_exactly_once(self):
        self.assertEqual(self.sample(True)[1], 1)

    def test_poll_uses_the_existing_stats_flag_for_both_timestamps(self):
        source = (ROOT / "nvkvm_broker.c").read_text()
        body = source.split("/* ★ MEASUREMENT (--stats-line):", 1)[1].split("if (pfd[0].revents)", 1)[0]
        self.assertEqual(body.count("nb_stats_now_ns(cfg.stats_line)"), 2)
        self.assertNotIn("clock_gettime(", body)

if __name__ == "__main__":
    unittest.main()
