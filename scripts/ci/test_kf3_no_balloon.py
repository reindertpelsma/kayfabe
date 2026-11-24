#!/usr/bin/env python3
"""2026-11-19: no memory balloon beside kf3 (the AD104 in-QEMU reboot hang, reset probes 1-4).

kf3 hands the host driver one view of the guest memfd, built once per QEMU, so a balloon that
discards guest pages (free-page reporting, an inflate) leaves the GPU reading pages the guest no
longer uses. Three checks:

1. kf3.c's guard, compiled from the real source with stubs for the QEMU calls: no balloon passes,
   a balloon (or an ambiguous match) is refused with the fix in the text.
2. kf3.c wiring, read from the source: realize checks before it takes the discard requirement,
   the machine-init-done check is registered and removed on both exit paths.
3. Lint: no script under scripts/bench may start a balloon except propimage_linux.py, whose argv is
   pinned by test_propimage_linux.py (no balloon whenever kf3 is attached).

FALSIFIER: a kf3.c without the guard, or one that checks after building anything, or a bench script
that adds a balloon device to a kf3 VM.
"""
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "qemu/hw/misc/kf3/kf3.c"
ALLOWED = {"scripts/bench/linux/propimage_linux.py"}


def function(source, signature):
    start = source.index(signature)
    body = source.index("{", start)
    depth, end = 1, body + 1
    while depth:
        depth += (source[end] == "{") - (source[end] == "}")
        end += 1
    return source[start:end]


STUBS = r"""
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
typedef struct Object { int x; } Object;
typedef struct Notifier { void (*notify)(struct Notifier *, void *); } Notifier;
static Object the_balloon;
static int g_mode; /* 0 none, 1 one balloon, 2 ambiguous */
static const char *g_type;
static Object *object_resolve_path_type(const char *path, const char *type, bool *ambiguous)
{
    (void)path;
    g_type = type;
    if (g_mode == 2) { *ambiguous = true; return NULL; }
    return g_mode == 1 ? &the_balloon : NULL;
}
static void error_report(const char *fmt, const char *s) { (void)fmt; fputs(s, stdout); }
"""

MAIN = r"""
int main(int argc, char **argv)
{
    if (argc != 2) return 2;
    g_mode = atoi(argv[1]);
    const char *why = kf3_balloon_refusal();
    if (strcmp(g_type, "virtio-balloon-device")) return 3;
    if (g_mode == 0) return why ? 4 : 0;
    if (!why || !strstr(why, "memballoon model='none'")) return 5;
    Notifier n = {0};
    kf3_balloon_check(&n, NULL); /* exits 1 with the text */
    return 6;
}
"""


class Guard(unittest.TestCase):
    def setUp(self):
        self.src = SOURCE.read_text()

    def run_model(self, mode):
        cc = shutil.which("cc") or shutil.which("gcc")
        self.assertIsNotNone(cc, "a C compiler is required (the guard is checked by compiling it)")
        code = (STUBS + "#define KF3_BALLOON_TYPE \"virtio-balloon-device\"\n"
                + function(self.src, "static const char *kf3_balloon_refusal(void)") + "\n"
                + function(self.src, "static void kf3_balloon_check(Notifier *n, void *data)") + MAIN)
        with tempfile.TemporaryDirectory(prefix="kf3-balloon-") as d:
            c, b = Path(d) / "probe.c", Path(d) / "probe"
            c.write_text(code)
            subprocess.run([cc, "-std=c11", "-Wall", "-Wextra", "-Werror", str(c), "-o", str(b)],
                           check=True, capture_output=True)
            return subprocess.run([str(b), str(mode)], capture_output=True, text=True)

    def test_no_balloon_passes(self):
        self.assertEqual(self.run_model(0).returncode, 0)

    def test_a_balloon_is_refused_with_the_fix(self):
        for mode in (1, 2):
            r = self.run_model(mode)
            self.assertEqual(r.returncode, 1, f"mode {mode}: {r.stdout}")
            self.assertIn("memballoon model='none'", r.stdout)

    def test_type_name_is_virtio_balloons(self):
        self.assertIn('#define KF3_BALLOON_TYPE "virtio-balloon-device"', self.src)

    def test_realize_checks_before_it_builds_anything(self):
        realize = function(self.src, "static void kf3_dev_realize(PCIDevice *pci, Error **errp)")
        check = realize.index("kf3_balloon_refusal()")
        self.assertLess(check, realize.index("ram_block_discard_require(true)"))
        self.assertLess(check, realize.index("kf3_realize("))
        self.assertIn("qemu_add_machine_init_done_notifier(&s->balloon_check)", realize)
        fail = realize[realize.index("fail:"):]
        self.assertIn("qemu_remove_machine_init_done_notifier(&s->balloon_check)", fail)
        exit_fn = function(self.src, "static void kf3_dev_exit(PCIDevice *pci)")
        self.assertIn("qemu_remove_machine_init_done_notifier(&s->balloon_check)", exit_fn)


class BenchLint(unittest.TestCase):
    def test_no_bench_script_starts_a_balloon_beside_kf3(self):
        offenders = []
        for p in sorted((ROOT / "scripts/bench").rglob("*")):
            if not p.is_file() or p.suffix not in {".sh", ".py", ".ps1", ".xml", ".json", ""}:
                continue
            rel = p.relative_to(ROOT).as_posix()
            try:
                text = p.read_text(errors="replace")
            except OSError:
                continue
            if re.search(r"virtio-balloon|memballoon model=['\"](?!none)", text) and rel not in ALLOWED:
                offenders.append(rel)
        self.assertEqual(offenders, [], "a balloon device outside the pinned harness")


if __name__ == "__main__":
    unittest.main()
