/*******************************************************************************
    Copyright (c) 2026 kayfabe contributors. Licensed under the same terms as the
    surrounding nvidia-uvm sources (MIT, see the header of uvm_ioctl.h).

    EXTERNAL FAULT SERVICE (EFS): kernel-internal interface. See uvm_efs_ioctl.h
    for the user ABI and uvm_efs.c for the design and the lifetime rules.
*******************************************************************************/

#ifndef __UVM_EFS_H__
#define __UVM_EFS_H__

#include "uvm_forward_decl.h"
#include "uvm_linux.h"
#include "uvm_efs_ioctl.h"
#include "uvm_hal_types.h"

typedef struct uvm_efs_va_space_struct uvm_efs_va_space_t;

// Module parameter uvm_efs_enable, read at UVM_INITIALIZE time.
bool uvm_efs_enabled(void);

// Module parameter uvm_efs_skip_diverted_replays.
bool uvm_efs_skip_diverted_replays(void);

// Global counter bump for the replayable-fault service loop.
void uvm_efs_count_batch_replay_skipped(void);

// Allocated before the VA space is created and attached before the file is
// published, so no fault can ever be attributed to an EFS VA space that has no
// EFS state. Freed by uvm_efs_va_space_free().
NV_STATUS uvm_efs_va_space_alloc(uvm_efs_va_space_t **efs_out);
void uvm_efs_va_space_attach(uvm_va_space_t *va_space, uvm_efs_va_space_t *efs);
void uvm_efs_va_space_free_unattached(uvm_efs_va_space_t *efs);

// First step of every teardown path (mm shutdown and VA space destroy), before
// stock UVM stops any channel. Idempotent. Stops accepting records, cancels the
// timeout work and (uvm_efs_teardown_cancel=1, the default) cancels every parked
// fault in hardware, so the stock teardown finds exactly the hardware state it
// would have found without EFS: no fault left pending on its behalf.
//
// LOCKING: no UVM locks held. Takes the replayable ISR lock, then the VA space
//          lock in read mode (the bottom half's own order).
void uvm_efs_va_space_shutdown(uvm_va_space_t *va_space);

// Last step of uvm_va_space_destroy, after the bottom halves were flushed.
void uvm_efs_va_space_free(uvm_va_space_t *va_space);

// Called from remove_gpu_va_space() with the VA space lock held in write mode.
// Records against that GPU VA space become stale: they are dropped without any
// hardware action (its page directory is going away).
void uvm_efs_gpu_va_space_removed(uvm_va_space_t *va_space, uvm_gpu_t *gpu);

// Called by the replayable fault servicing path, with the replayable service
// lock held and the VA space lock held in read mode, for a fault stock UVM is
// about to mark fatal with NV_ERR_INVALID_ADDRESS. Returns true if the fault was
// parked (or merged into a parked record): the caller must then neither cancel
// nor count it as fatal. Never sleeps and never allocates.
bool uvm_efs_try_divert(uvm_va_space_t *va_space, uvm_gpu_t *gpu, const uvm_fault_buffer_entry_t *entry);

// Hardware helpers implemented in uvm_gpu_replayable_faults.c (they wrap static
// functions there). The caller holds uvm_parent_gpu_replayable_faults_isr_lock.
NV_STATUS uvm_efs_hw_cancel_va_locked(uvm_fault_buffer_entry_t *entry);
NV_STATUS uvm_efs_hw_flush_and_replay_locked(uvm_gpu_t *gpu);
NV_STATUS uvm_efs_hw_replay_locked(uvm_gpu_t *gpu);

// ioctls
NV_STATUS uvm_api_efs_query(UVM_EFS_QUERY_PARAMS *params, struct file *filp);
NV_STATUS uvm_api_efs_wait(UVM_EFS_WAIT_PARAMS *params, struct file *filp);
NV_STATUS uvm_api_efs_resolve(UVM_EFS_RESOLVE_PARAMS *params, struct file *filp);

#endif // __UVM_EFS_H__
