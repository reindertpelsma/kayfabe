/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 * The root measurement tool may read exactly the 4-KiB NV_PDISP_FE_SW page,
 * with aligned 32-bit loads, as well as its existing three GSP registers.
 */
#include "../lib/kfwc_platform.h"
#include <assert.h>
#include <stdio.h>
#include <string.h>

typedef struct mock_regs {
    kfwc_u32 calls, fail_at, fill, allones_at;
} mock_regs;

static int read_reg(void *ctx, kfwc_u32 off, kfwc_u32 *out)
{
    mock_regs *m = (mock_regs *)ctx;
    assert(off == KFWC_DISPLAY_CAPS_BASE + m->calls * 4u);
    assert(m->calls < KFWC_DISPLAY_CAPS_WORDS);
    if (m->calls++ == m->fail_at)
        return 1;
    *out = m->calls - 1u == m->allones_at ? 0xffffffffu : m->fill;
    return 0;
}

int main(void)
{
    kfwc_u32 i;
    struct {
        kfwc_u32 before, words[KFWC_DISPLAY_CAPS_WORDS], after;
    } out;
    mock_regs mock = {0, 0xffffffffu, 0x12345678u, 0xffffffffu};
    kfwc_platform p;
    memset(&p, 0, sizeof(p));
    p.ctx = &mock;
    p.read_reg32 = read_reg;
    for (i = 0; i < 1024u; i++) {
        assert(kfwc_reg_allowed(0x00640000u + i * 4u));
        assert(!kfwc_reg_allowed(0x00640001u + i * 4u));
    }
    assert(!kfwc_reg_allowed(0x0063fffcu));
    assert(!kfwc_reg_allowed(0x00641000u));
    assert(!kfwc_reg_allowed(0xfffffffcu));
    assert(kfwc_reg_allowed(KFWC_REG_GSP_MAILBOX0));
    assert(kfwc_reg_allowed(KFWC_REG_GSP_MAILBOX1));
    out.before = out.after = 0x13572468u;
    assert(!kfwc_display_caps_read(&p, out.words, KFWC_DISPLAY_CAPS_WORDS));
    assert(mock.calls == KFWC_DISPLAY_CAPS_WORDS);
    for (i = 0; i < KFWC_DISPLAY_CAPS_WORDS; i++)
        assert(out.words[i] == mock.fill);
    assert(out.before == 0x13572468u && out.after == 0x13572468u);

    mock.calls = 0;
    assert(kfwc_display_caps_read(&p, out.words, KFWC_DISPLAY_CAPS_WORDS - 1u));
    assert(kfwc_display_caps_read(&p, NULL, KFWC_DISPLAY_CAPS_WORDS));
    assert(kfwc_display_caps_read(NULL, out.words, KFWC_DISPLAY_CAPS_WORDS));
    assert(mock.calls == 0);
    mock.fill = 0;
    assert(kfwc_display_caps_read(&p, out.words, KFWC_DISPLAY_CAPS_WORDS));
    assert(mock.calls == KFWC_DISPLAY_CAPS_WORDS);
    mock.calls = 0;
    mock.fill = 0xffffffffu;
    assert(kfwc_display_caps_read(&p, out.words, KFWC_DISPLAY_CAPS_WORDS));
    assert(mock.calls == KFWC_DISPLAY_CAPS_WORDS);
    mock.calls = 0;
    mock.fill = 0x12345678u;
    mock.allones_at = 3;
    assert(!kfwc_display_caps_read(&p, out.words, KFWC_DISPLAY_CAPS_WORDS));
    assert(out.words[3] == 0xffffffffu && mock.calls == KFWC_DISPLAY_CAPS_WORDS);
    mock.calls = 0;
    mock.fill = 1;
    mock.fail_at = 4;
    assert(kfwc_display_caps_read(&p, out.words, KFWC_DISPLAY_CAPS_WORDS));
    assert(mock.calls == 5);
    puts("DISPLAY_CAPS_BOUNDS_PASS");
    return 0;
}
