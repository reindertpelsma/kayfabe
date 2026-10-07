/*******************************************************************************
    Copyright (c) 2026 kayfabe contributors. Licensed under the same terms as the
    surrounding nvidia-uvm sources (MIT, see the header of uvm_ioctl.h).

    EXTERNAL FAULT SERVICE (EFS) — the user ABI of the opt-in b3 extension.

    A UVM file opts in at UVM_INITIALIZE with UVM_INIT_FLAGS_EXTERNAL_FAULT_SERVICE,
    before any registration. The module parameter uvm_efs_enable must be 1
    (default 0), otherwise that flag is refused with NV_ERR_NOT_SUPPORTED. A stock
    nvidia-uvm refuses the unknown flag with NV_ERR_INVALID_ARGUMENT, so the two
    are distinguishable from userspace.

    In an EFS VA space, a replayable GPU fault that stock UVM would cancel because
    no UVM range can service it (NV_ERR_INVALID_ADDRESS: no range, an external
    range, HMM off) is parked instead and queued to this file as a record. Only
    the thread group that initialized the file may read records or resolve them.
    Records never carry an instance pointer, a PDB or any host address: a record
    is named by an opaque id the kernel issued, and every hardware action the
    kernel takes on it uses state the kernel saved itself.
*******************************************************************************/

#ifndef _UVM_EFS_IOCTL_H
#define _UVM_EFS_IOCTL_H

#include "uvm_types.h"

#ifdef __cplusplus
extern "C" {
#endif

// Outside UVM_INIT_FLAGS_MASK on purpose (a high bit, far from NVIDIA's own).
#define UVM_INIT_FLAGS_EXTERNAL_FAULT_SERVICE            ((NvU64)0x4000000000000000ULL)

#define UVM_EFS_ABI_VERSION                              1

// Far from both the stock range (1..80) and the test range (200+).
#define UVM_EFS_QUERY                                    UVM_IOCTL_BASE(1800)
#define UVM_EFS_WAIT                                     UVM_IOCTL_BASE(1801)
#define UVM_EFS_RESOLVE                                  UVM_IOCTL_BASE(1802)

#define UVM_EFS_ACTION_REPLAY                            1
#define UVM_EFS_ACTION_CANCEL                            2

#define UVM_EFS_MAX_WAIT_RECORDS                         64
#define UVM_EFS_MAX_RESOLVE_RECORDS                      256
#define UVM_EFS_MAX_WAIT_TIMEOUT_US                      1000000

// Indices into UVM_EFS_QUERY_PARAMS.vaSpaceCounters.
#define UVM_EFS_CTR_DIVERTED                             0  // new records
#define UVM_EFS_CTR_DEDUPED                              1  // re-faults merged into a parked record
#define UVM_EFS_CTR_REFUSED_FULL                         2  // queue full: left to stock (cancel)
#define UVM_EFS_CTR_REFUSED_CLOSING                      3  // VA space tearing down: left to stock
#define UVM_EFS_CTR_DELIVERED                            4  // records handed to userspace
#define UVM_EFS_CTR_RESOLVED_REPLAY                      5
#define UVM_EFS_CTR_RESOLVED_CANCEL                      6
#define UVM_EFS_CTR_TIMED_OUT                            7  // cancelled by the kernel at the deadline
#define UVM_EFS_CTR_TEARDOWN_CANCELLED                   8  // cancelled by the kernel at shutdown
#define UVM_EFS_CTR_STALE                                9  // ids that named no live record
#define UVM_EFS_CTR_DROPPED_GPU_GONE                     10 // records dropped: GPU VA space removed
#define UVM_EFS_CTR_HW_REPLAYS                           11 // replays this VA space issued
#define UVM_EFS_CTR_COUNT                                12

// Indices into UVM_EFS_QUERY_PARAMS.globalCounters (module-wide).
#define UVM_EFS_GCTR_BATCH_REPLAYS_SKIPPED               0  // all-diverted batches not replayed
#define UVM_EFS_GCTR_SAFETY_REPLAYS                      1  // periodic replays while records parked
#define UVM_EFS_GCTR_EFS_VA_SPACES                       2  // EFS VA spaces created
#define UVM_EFS_GCTR_COUNT                               4

typedef struct
{
    NvU32           abiVersion;                                 // OUT
    NvU32           moduleEnabled;                              // OUT uvm_efs_enable
    NvU32           active;                                     // OUT this file is an EFS VA space
    NvU32           maxRecords;                                 // OUT
    NvU32           timeoutMs;                                  // OUT
    NvU32           skipDivertedReplays;                        // OUT
    NvU32           numParked;                                  // OUT
    NvU32           numUndelivered;                             // OUT
    NvU64           vaSpaceCounters[UVM_EFS_CTR_COUNT]  NV_ALIGN_BYTES(8); // OUT
    NvU64           globalCounters[UVM_EFS_GCTR_COUNT]  NV_ALIGN_BYTES(8); // OUT
    NV_STATUS       rmStatus;                                   // OUT
} UVM_EFS_QUERY_PARAMS;

typedef struct
{
    NvU64           recordId          NV_ALIGN_BYTES(8);  // opaque, kernel-issued, never reused
    NvU64           faultAddress      NV_ALIGN_BYTES(8);  // GPU VA, PAGE_SIZE aligned
    NvU64           gpuTimestampNs    NV_ALIGN_BYTES(8);  // from the fault packet (GPU timer)
    NvU64           divertTimeNs      NV_ALIGN_BYTES(8);  // ktime_get_real_ns() when parked
    NvProcessorUuid gpuUuid;                              // GPU (or GI) the fault came from
    NvU32           accessType;                           // highest access type seen
    NvU32           accessTypeMask;                       // all access types seen on this page
    NvU32           faultType;
    NvU32           clientType;                           // 0 = GPC, 1 = HUB
    NvU32           clientId;
    NvU32           gpcId;
    NvU32           utlbId;
    NvU32           veId;
    NvU32           mmuEngineId;
    NvU32           numInstances;                         // packets coalesced into this record
} UvmEfsFaultRecord;

typedef struct
{
    NvU64           records           NV_ALIGN_BYTES(8);  // IN  user pointer to UvmEfsFaultRecord[maxRecords]
    NvU32           maxRecords;                           // IN  1..UVM_EFS_MAX_WAIT_RECORDS
    NvU32           timeoutUs;                            // IN  0 = poll; capped at UVM_EFS_MAX_WAIT_TIMEOUT_US
    NvU32           numRecords;                           // OUT
    NV_STATUS       rmStatus;                             // OUT
} UVM_EFS_WAIT_PARAMS;

typedef struct
{
    NvU64           recordIds         NV_ALIGN_BYTES(8);  // IN  user pointer to NvU64[count]
    NvU32           count;                                // IN  1..UVM_EFS_MAX_RESOLVE_RECORDS
    NvU32           action;                               // IN  UVM_EFS_ACTION_*
    NvU32           numResolved;                          // OUT ids that named a live record
    NvU32           numStale;                             // OUT ids that did not (no action taken)
    NV_STATUS       rmStatus;                             // OUT
} UVM_EFS_RESOLVE_PARAMS;

#ifdef __cplusplus
}
#endif

#endif // _UVM_EFS_IOCTL_H
