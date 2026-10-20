/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 * Compile against pinned OGKM580.65.06 commit307159f2623d3bf45feb9177bd2da52ffbc5ddf9.
 * No GPU access: public C layouts must agree with the tracer's wire constants. */
#include <stddef.h>
#include "nvtypes.h"
#include "gspifpub.h"
#include "../lib/kfwc_wire.h"
_Static_assert(KFWC_FMC_TARGET == offsetof(GSP_FMC_BOOT_PARAMS, gspRmParams.target), "FMC target offset");
_Static_assert(KFWC_FMC_LIBOS == offsetof(GSP_FMC_BOOT_PARAMS, gspRmParams.bootArgsOffset), "FMC LibOS offset");
_Static_assert(KFWC_FMC_COHERENT_SYSTEM == GSP_DMA_TARGET_COHERENT_SYSTEM, "coherent aperture");
_Static_assert(KFWC_FMC_NONCOHERENT_SYSTEM == GSP_DMA_TARGET_NONCOHERENT_SYSTEM, "noncoherent aperture");
int main(void) { return 0; }
