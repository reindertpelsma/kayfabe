# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""kf_gldual.c / kf_gldual_logic.h with the host C compiler: the pure logic is run, the Windows part is only type-checked against declaration-only
stubs (tests/fixtures/winstub). This is NOT an MSVC build (INFERRED until the first Windows run)."""
import os
import shutil
import subprocess
import tempfile
import unittest

import helpers as H

CC = shutil.which("gcc") or shutil.which("cc") or shutil.which("clang")

TEST_C = r'''
#include "kf_gldual_logic.h"
int main(int argc, char **argv) {
    kf_cfg c;
    kf_stats a, b;
    char line[256];
    uint32_t n, s, sum, i;
    int mode = atoi(argv[1]);
    memset(&a, 0, sizeof a);
    memset(&b, 0, sizeof b);
    if (mode == 1) {
        kf_parse_args(argc - 1, argv + 1, &c);
        printf("%d %d %d %d\n", c.seconds, c.share, c.any_renderer, c.bad_args);
    } else if (mode == 2) {
        n = (uint32_t)strtoul(argv[2], 0, 10);
        s = (uint32_t)strtoul(argv[3], 0, 10);
        sum = 0;
        for (i = 0; i < n; i++) sum += i * 2654435761u + s;
        printf("%u %u\n", sum, kf_expected_sum(n, s));
    } else if (mode == 3) {
        a.iters = 5;
        b.iters = 5;
        if (argv[2][0] == 'a') kf_note_bad(&a, "compute checksum", 7);
        if (argv[2][0] == 'b') kf_note_bad(&b, "pixel", 9);
        if (argv[2][0] == 'z') b.iters = 0;
        if (argv[2][0] == 'y') a.iters = 0;
        printf("%d|", kf_result(&a, &b, argv[2][0] == 'c', line, sizeof line));
        puts(line);
    } else if (mode == 4) {
        printf("%u %u %u\n", kf_channel(0, 0, 0), kf_channel(1, 1, 2), kf_channel(100, 0, 1));
    }
    return 0;
}
'''


@unittest.skipIf(CC is None, "no C compiler")
class Logic(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.dir = tempfile.mkdtemp(prefix="kfopen-c-")
        src = os.path.join(cls.dir, "t.c")
        open(src, "w").write(TEST_C)
        cls.exe = os.path.join(cls.dir, "t")
        r = subprocess.run([CC, "-std=gnu99", "-Wall", "-Wextra", "-Werror", "-I", H.OR, src, "-o", cls.exe], capture_output=True, text=True)
        assert r.returncode == 0, r.stderr

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.dir, True)

    def run_t(self, *args):
        return subprocess.run([self.exe] + [str(a) for a in args], capture_output=True, text=True, check=True).stdout.strip()

    def test_expected_sum_matches_bruteforce(self):
        for n, seed in ((64, 0), (65536, 977), (65536, 4000000000)):
            a, b = self.run_t(2, n, seed).split()
            self.assertEqual(a, b, (n, seed))

    def test_expected_sum_wraps_like_uint32(self):
        a, b = self.run_t(2, 262144, 4294967295).split()
        self.assertEqual(a, b)

    def test_arguments(self):
        self.assertEqual("600 0 0 0", self.run_t(1))
        self.assertEqual("30 1 1 0", self.run_t(1, "--seconds", 30, "--share", "--any-renderer"))
        self.assertEqual("600 0 0 1", self.run_t(1, "--bogus"))
        self.assertEqual("0 0 0 1", self.run_t(1, "--seconds", 0))
        self.assertEqual("90000 0 0 1", self.run_t(1, "--seconds", 90000))

    def test_result_ok(self):
        self.assertEqual("0|KFGL RESULT OK iters=5/5", self.run_t(3, "ok"))

    def test_result_fails_when_a_bad(self):
        out = self.run_t(3, "a")
        self.assertTrue(out.startswith("1|KFGL RESULT FAIL bad=1/0 first=compute checksum at iteration 7"), out)

    def test_result_fails_when_b_bad(self):
        out = self.run_t(3, "b")
        self.assertTrue(out.startswith("1|KFGL RESULT FAIL bad=0/1 first=pixel at iteration 9"), out)

    def test_result_fails_on_context_failure(self):
        self.assertEqual("1|KFGL RESULT FAIL context-setup-failed", self.run_t(3, "c"))

    def test_result_fails_when_a_thread_did_no_iterations(self):
        self.assertTrue(self.run_t(3, "z").startswith("1|KFGL RESULT FAIL a-thread-did-no-iterations"))

    def test_result_fails_when_the_first_thread_did_no_iterations(self):
        self.assertTrue(self.run_t(3, "y").startswith("1|KFGL RESULT FAIL a-thread-did-no-iterations a=0"))

    def test_channels_are_deterministic_and_distinct_per_thread(self):
        self.assertEqual(self.run_t(4), self.run_t(4))
        a, b, c = [int(x) for x in self.run_t(4).split()]
        self.assertTrue(0 <= a < 256 and 0 <= b < 256 and 0 <= c < 256)


@unittest.skipIf(CC is None, "no C compiler")
class WindowsPartTypeChecks(unittest.TestCase):
    def test_kf_gldual_c_type_checks_against_the_stub_headers(self):
        stub = os.path.join(H.FIX, "winstub")
        r = subprocess.run([CC, "-std=gnu99", "-fsyntax-only", "-Wall", "-Wextra", "-Wno-unused-parameter", "-Wno-unused-function", "-I", stub, "-I", H.OR,
                            os.path.join(H.OR, "kf_gldual.c")], capture_output=True, text=True)
        self.assertEqual(0, r.returncode, r.stderr)

    def test_the_c_file_has_the_contexts_threads_and_paths_the_row_promises(self):
        src = open(os.path.join(H.OR, "kf_gldual.c")).read()
        for needle in ("wglCreateContextAttribsARB", "CreateThread", "glDispatchCompute", "KF_PATCHES", "glBlitFramebuffer", "glFenceSync", "SwapBuffers", "g_cfg.share"):
            self.assertTrue(needle in src, needle)
        self.assertEqual(2, src.count("for (i = 0; i < 2; i++)"), "one loop makes two windows with their contexts, one starts two threads")


if __name__ == "__main__":
    unittest.main()
