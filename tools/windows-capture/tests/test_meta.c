/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 * The cell sidecar: fixed key set, escaping, no board identity, and the record-file checker's
 * twin (the record writer is exercised in test_main.c / test_ring.c).
 */
#include "../lib/kfwc_meta.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int g_checks, g_fail;
#define CHECK(cond)                                                                    \
    do {                                                                               \
        g_checks++;                                                                    \
        if (!(cond)) {                                                                 \
            g_fail++;                                                                  \
            fprintf(stderr, "  FAIL %s:%d %s\n", __FILE__, __LINE__, #cond);          \
        }                                                                              \
    } while (0)

static int allowed(const char *k, size_t n)
{
    const char *const *a;
    for (a = kfwc_meta_allowed_keys; *a; a++)
        if (strlen(*a) == n && !memcmp(*a, k, n))
            return 1;
    return 0;
}

int main(void)
{
    static char out[16384], small[64];
    static kfwc_meta m;
    kfwc_event ev[2];
    size_t n, i, keys = 0, bad = 0;
    char esc[16];
    memset(&m, 0, sizeof(m));
    memset(ev, 0, sizeof(ev));
    ev[0].code = KFWC_EV_ATTACH;
    ev[0].generation = 1;
    m.windows_build = "26100";
    m.windows_ubr = "1742";
    m.windows_display_version = "24H2";
    /* a hostile version string: quotes, backslash, control characters, a newline */
    m.driver_version = "32.0.15.8088\"},\n\"uuid\": \"GPU-1234\x01\\";
    m.decode_tag = "580.82.07";
    m.info.module_version = KFWC_MODULE_VERSION;
    m.info.engine_abi = KFWC_ENGINE_ABI;
    m.info.pci_vendor_id = 0x10de;
    m.info.pci_device_id = 0x2786;
    m.info.pci_revision_id = 0xa1;
    m.info.qpc_frequency = 10000000;
    m.events = ev;
    m.n_events = 1;
    n = kfwc_meta_format(out, sizeof(out), &m);
    CHECK(n > 100 && n == strlen(out));
    CHECK(kfwc_meta_format(small, sizeof(small), &m) == 0);   /* never a silent truncation */
    /* every key in the output is on the allow-list: the injected "uuid" key could not appear */
    for (i = 0; i + 1 < n; i++) {
        if (out[i] == '"') {
            size_t j = i + 1;
            while (j < n && out[j] != '"')
                j += (out[j] == '\\') ? 2 : 1;   /* a backslash escapes the next byte */
            if (j + 1 < n && out[j + 1] == ':') {
                keys++;
                if (!allowed(out + i + 1, j - i - 1)) {
                    bad++;
                    fprintf(stderr, "  key not allowed: %.*s\n", (int)(j - i - 1), out + i + 1);
                }
            }
            i = j;
        }
    }
    if (keys <= 30) fprintf(stderr, "  keys=%zu\n", keys);
    CHECK(keys > 30 && bad == 0);
    CHECK(strstr(out, "GPU-") != NULL);                 /* escaped inside the string value, not a key */
    CHECK(strstr(out, "\\\"") != NULL);
    CHECK(strchr(out, '\x01') == NULL);
    /* nothing that identifies a board or a person */
    {
        static const char *const banned[] = {"subsys", "SUBSYS", "serial", "Serial", "VBIOS", "vbios", "mac_",
                                             "bus_", "hostname", "username", "ip_addr", 0};
        const char *const *b;
        for (b = banned; *b; b++)
            CHECK(strstr(out, *b) == NULL);
    }
    CHECK(strstr(out, "\"pci_device_id\": \"0x2786\"") != NULL && strstr(out, "\"decode_tag\": \"580.82.07\"") != NULL);
    CHECK(kfwc_json_escape(esc, sizeof(esc), "ab\"c") == 0 && !strcmp(esc, "ab\\\"c"));
    CHECK(kfwc_json_escape(esc, sizeof(esc), "a\nb") == 1 && !strcmp(esc, "ab"));
    CHECK(kfwc_json_escape(esc, 4, "abcdef") == 1 && strlen(esc) == 3);
    printf("test_meta: %d checks, %d failed\n", g_checks, g_fail);
    return g_fail ? 1 : 0;
}
