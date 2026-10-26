/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 * Collector file publication. Kept out of the freestanding kernel library.
 */
#ifndef KFWC_FILE_H
#define KFWC_FILE_H
#include <stdio.h>
#include "../lib/kfwc_record.h"

static __inline int kfwc_file_begin(FILE *out, const kfwc_cap_file *header)
{
    kfwc_u8 bytes[KFWC_CAP_FILE_HEADER_BYTES];
    if (kfwc_cap_file_encode(header, bytes) != KFWC_CAP_FILE_HEADER_BYTES)
        return 1;
    return fwrite(bytes, 1, sizeof(bytes), out) != sizeof(bytes) || fflush(out) != 0;
}
#endif
