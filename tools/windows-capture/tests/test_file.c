/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later */
#define _POSIX_C_SOURCE 200809L
#include "../user/kfwc_file.h"
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

static void header_init(kfwc_cap_file *header)
{
    memset(header, 0, sizeof(*header));
    header->os_id = KFWC_CAP_OS_WINDOWS;
    header->module_version = 1;
    header->pci_vendor_id = 0x10de;
}

static int header_visible_size(void)
{
    kfwc_cap_file header;
    struct stat state;
    FILE *stream = tmpfile();
    int pass;
    if (!stream) return 0;
    header_init(&header);
    pass = kfwc_file_begin(stream, &header) == 0;
    pass = pass && fstat(fileno(stream), &state) == 0 && state.st_size == 128;
    fclose(stream);
    return pass;
}

static int header_visible_reader(void)
{
    kfwc_cap_file header, decoded;
    kfwc_u8 bytes[128];
    FILE *stream = tmpfile();
    int pass;
    if (!stream) return 0;
    header_init(&header);
    pass = kfwc_file_begin(stream, &header) == 0;
    /* A separate descriptor read must see readiness while the producer stays open. */
    pass = pass && pread(fileno(stream), bytes, sizeof(bytes), 0) == sizeof(bytes);
    pass = pass && kfwc_cap_file_parse(bytes, sizeof(bytes), &decoded) == KFWC_PARSE_OK;
    fclose(stream);
    return pass;
}

int main(void)
{
    int size = header_visible_size(), reader = header_visible_reader();
    printf("%s header_visible_size\n", size ? "ok" : "FAIL");
    printf("%s header_visible_reader\n", reader ? "ok" : "FAIL");
    return size && reader ? 0 : 1;
}
