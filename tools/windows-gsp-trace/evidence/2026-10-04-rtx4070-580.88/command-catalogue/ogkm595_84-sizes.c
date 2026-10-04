/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later */
#include <stdio.h>
#include <ctrl/ctrl0073/ctrl0073specific.h>
int main(void) {
    printf("0x00730280 %zu\n", sizeof(NV0073_CTRL_SPECIFIC_GET_HDCP_STATE_PARAMS));
    printf("0x00730282 %zu\n", sizeof(NV0073_CTRL_SPECIFIC_HDCP_CTRL_PARAMS));
    return 0;
}
