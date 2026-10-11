/* Header-only shape, compiled against exact public ogkm host-driver tag.
 * gcc -I<exact-tag>/src/common/sdk/nvidia/inc layout.c -o layout
 */
#include <stddef.h>
#include <stdio.h>
#include <ctrl/ctrl2080/ctrl2080rc.h>
int main(void) {
 printf("%u %u %zu %zu %zu %zu %zu %zu\n",
        NV2080_CTRL_CMD_RC_GET_ERROR_COUNT, NV2080_CTRL_CMD_RC_GET_ERROR_V2,
        sizeof(NV2080_CTRL_RC_GET_ERROR_COUNT_PARAMS),
        sizeof(NV2080_CTRL_RC_GET_ERROR_V2_PARAMS),
        offsetof(NV2080_CTRL_RC_GET_ERROR_V2_PARAMS, whichBuffer),
        offsetof(NV2080_CTRL_RC_GET_ERROR_V2_PARAMS, outputRecordSize),
        offsetof(NV2080_CTRL_RC_GET_ERROR_V2_PARAMS, recordBuffer),
        sizeof(((NV2080_CTRL_RC_GET_ERROR_V2_PARAMS *)0)->recordBuffer));
}
