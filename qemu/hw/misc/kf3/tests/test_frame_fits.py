"""C-04/D-06 = S1-03: compile the real kf3_frame_fits (and kf3_frame_bpp) from kf3.c against the
real Kf3Frame from kf3.h, and drive it with exact fits and hostile frames.

FALSIFIER: a frame whose geometry needs more bytes than `len` passes, a stride compared in pixels
rather than bytes passes, or a valid exact-fit frame is refused.

Run: python3 -m unittest discover -s qemu/hw/misc/kf3/tests -p 'test_frame_fits.py'
(not yet in CI: kf3.c has no CI build, stage-1 S1-13).
"""
from pathlib import Path
import subprocess
import tempfile
import unittest

REPO = Path(__file__).resolve().parents[5]
KF3 = REPO / 'qemu/hw/misc/kf3'


def function(source, signature):
    start = source.index(signature)
    body = source.index('{', start)
    depth = 1
    end = body + 1
    while depth:
        depth += (source[end] == '{') - (source[end] == '}')
        end += 1
    return source[start:end]


MAIN = r'''
#include <stdio.h>
static int run(uint8_t *data, uint32_t w, uint32_t h, uint32_t stride, uint32_t fmt, uint64_t len)
{
    Kf3Frame f = { data, w, h, stride, fmt, 1, len };
    return kf3_frame_fits(&f) ? 1 : 0;
}
int main(void)
{
    static uint8_t px[16];
    uint8_t *d = px;
    int fails = 0;
#define EXPECT(want, ...) do { int got = run(__VA_ARGS__); if (got != (want)) { \
        printf("FAIL line %d: want %d got %d\n", __LINE__, want, got); fails++; } } while (0)
    /* exact fits */
    EXPECT(1, d, 1920, 1080, 7680, 1, 7680ull * 1080);
    EXPECT(1, d, 2, 2, 16, 1, 16 + 8);          /* last row needs width*bpp only */
    EXPECT(1, d, 2, 2, 4, 3, 8);                /* rgb565 */
    EXPECT(1, d, 16384, 1, 65536, 5, 65536);
    /* geometry larger than its bytes */
    EXPECT(0, d, 1920, 1080, 7680, 1, 7680ull * 1080 - 1);
    EXPECT(0, d, 2, 2, 16, 1, 16 + 7);
    EXPECT(0, d, 1920, 1080, 7680, 1, 0);       /* ABI <32 frame: no length */
    /* stride in BYTES (the old check compared it with width in pixels) */
    EXPECT(0, d, 1920, 1, 1920, 1, 1ull << 40);
    EXPECT(0, d, 1920, 1, 7676, 1, 1ull << 40);
    EXPECT(1, d, 1920, 1, 7680, 1, 1ull << 40);
    /* hostile */
    EXPECT(0, NULL, 1, 1, 4, 1, 4);
    EXPECT(0, d, 0, 1, 4, 1, 4);
    EXPECT(0, d, 1, 0, 4, 1, 4);
    EXPECT(0, d, 1, 1, 4, 9, 4);                /* unknown format */
    EXPECT(0, d, 4, 1, 18, 1, 1ull << 40);      /* stride & 3 */
    EXPECT(0, d, 16385, 1, 65540, 1, ~0ull);    /* width cap */
    EXPECT(0, d, 1, 16385, 4, 1, ~0ull);        /* height cap */
    EXPECT(0, d, 1, 1, 0x80000000u, 1, ~0ull);  /* stride the int cast makes negative */
    EXPECT(0, d, 0xffffffffu, 0xffffffffu, 0xfffffffcu, 1, ~0ull);
    return fails ? 1 : 0;
}
'''


class FrameFits(unittest.TestCase):
    def test_the_real_check_bounds_every_frame(self):
        source = (KF3 / 'kf3.c').read_text()
        program = '\n'.join([
            '#include <stdbool.h>',
            '#include <stdint.h>',
            '#include "kf3.h"',
            function(source, 'static uint64_t kf3_frame_bpp('),
            function(source, 'static bool kf3_frame_fits('),
            MAIN,
        ])
        with tempfile.TemporaryDirectory() as tmp:
            src = Path(tmp) / 'frame_fits.c'
            exe = Path(tmp) / 'frame_fits'
            src.write_text(program)
            subprocess.run(['cc', '-std=c11', '-Wall', '-Werror', '-I', str(KF3), '-o', str(exe),
                            str(src)], check=True)
            run = subprocess.run([str(exe)], capture_output=True, text=True)
            self.assertEqual(run.returncode, 0, run.stdout)


if __name__ == '__main__':
    unittest.main()
