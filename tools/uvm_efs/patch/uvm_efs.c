/*******************************************************************************
    Copyright (c) 2026 kayfabe contributors. Licensed under the same terms as the
    surrounding nvidia-uvm sources (MIT, see the header of uvm_ioctl.h).

    EXTERNAL FAULT SERVICE (EFS) — opt-in, default off.

    What it changes. In a VA space whose file opted in at UVM_INITIALIZE, a
    replayable fault that stock UVM would cancel because no UVM range services it
    (service_fault_batch_dispatch, status NV_ERR_INVALID_ADDRESS) is parked: its
    packet is copied into a bounded per-file table, the file's owner is woken, and
    the access stays pending in hardware. Nothing else changes: managed faults,
    fatal fault types, prefetch faults, non-replayable faults, every other VA
    space and every other process keep stock behaviour, through stock code.

    What the owner can do with a record: read it (UVM_EFS_WAIT), then resolve it
    (UVM_EFS_RESOLVE) with REPLAY (after mapping the page with the stock external
    mapping ioctls) or CANCEL (the access fails, as stock would have made it
    fail). Both act only through state this file saved: the saved packet and the
    VA space's own page directory. The ABI carries no instance pointer, no PDB
    and no host address.

    Ownership. The VA space belongs to the file (stock UVM). EFS records the
    thread group that initialized the file and refuses WAIT/RESOLVE from any
    other thread group: an EFS file passed by SCM_RIGHTS or inherited over fork
    does not delegate fault service. Registrations into the VA space are stock
    UVM's, with RM's duplication/verification checks (see the design doc).

    Lifetime (who keeps what alive):
    - The records live in uvm_efs_va_space_t, which lives exactly as long as the
      uvm_va_space_t (freed in uvm_va_space_destroy after the bottom halves were
      flushed), which lives as long as the file. An ioctl holds a file
      reference, so no WAIT/RESOLVE can run concurrently with the destroy.
    - A record holds no pointer into UVM or RM objects: it names its GPU by id
      and remembers the GPU VA space generation. remove_gpu_va_space bumps the
      generation under the VA space write lock, so a record from a removed GPU
      VA space can never reach hardware (its PDB is gone): it is dropped.
    - Every teardown path (mm shutdown at process exit, file release, deferred
      release) calls uvm_efs_va_space_shutdown first, before stock UVM stops a
      single channel: accepting stops, the timeout work is cancelled
      synchronously, and every parked fault is cancelled in hardware, which is
      what stock UVM would have done with it. Stock teardown then runs unchanged.
    - Channel unregistration needs no EFS action: a parked fault is hardware
      state stock UVM also leaves pending while it services a fault; a record
      whose channel went away is resolved harmlessly (replay: GPU-wide, no-op for
      a dead context; cancel: VA cancel against this VA space's own PDB) or
      expires.

    Bounds. uvm_efs_max_records parked records per VA space (the next fault is
    left to stock: cancelled). uvm_efs_timeout_ms after parking, the kernel
    cancels the record itself. Dedupe: a re-fault on a parked page merges into
    its record. A batch whose every fault was parked is not replayed (a replay
    would only re-raise the same parked accesses), while records are parked a
    periodic safety replay keeps any overflow-dropped fault of another tenant
    moving.

    Locking. efs->lock is a leaf spinlock. The bottom half calls
    uvm_efs_try_divert with the replayable service lock and the VA space lock
    (read) held. Every hardware action here takes the replayable ISR lock first
    and the VA space lock (read) second: the bottom half's own order.
*******************************************************************************/

#include "uvm_common.h"
#include "uvm_linux.h"
#include "uvm_api.h"
#include "uvm_efs.h"
#include "uvm_global.h"
#include "uvm_gpu.h"
#include "uvm_gpu_isr.h"
#include "uvm_kvmalloc.h"
#include "uvm_processors.h"
#include "uvm_va_space.h"

#include <linux/pid.h>
#include <linux/sched.h>
#include <linux/sched/signal.h>
#include <linux/spinlock.h>
#include <linux/timekeeping.h>
#include <linux/wait.h>
#include <linux/workqueue.h>

static int uvm_efs_enable = 0;
module_param(uvm_efs_enable, int, S_IRUGO);
MODULE_PARM_DESC(uvm_efs_enable,
                 "kayfabe b3: allow a UVM file to opt into EXTERNAL FAULT SERVICE at UVM_INITIALIZE (default 0: refused)");

static unsigned uvm_efs_max_records = 1024;
module_param(uvm_efs_max_records, uint, S_IRUGO);
MODULE_PARM_DESC(uvm_efs_max_records, "EFS: parked fault records per VA space (1..65535)");

static unsigned uvm_efs_timeout_ms = 10000;
module_param(uvm_efs_timeout_ms, uint, S_IRUGO);
MODULE_PARM_DESC(uvm_efs_timeout_ms, "EFS: a record parked this long is cancelled by the kernel (ms, >= 10)");

static int uvm_efs_skip_replays = 1;
module_param_named(uvm_efs_skip_diverted_replays, uvm_efs_skip_replays, int, S_IRUGO);
MODULE_PARM_DESC(uvm_efs_skip_diverted_replays, "EFS: do not replay a fault batch whose every fault was parked (default 1)");

static int uvm_efs_teardown_cancel = 1;
module_param(uvm_efs_teardown_cancel, int, S_IRUGO);
MODULE_PARM_DESC(uvm_efs_teardown_cancel, "EFS: cancel parked faults in hardware when the VA space shuts down (default 1)");

#define EFS_SLOT_BITS        16
#define EFS_SLOT_MASK        ((1ULL << EFS_SLOT_BITS) - 1)
#define EFS_MAX_RECORDS_CAP  ((1U << EFS_SLOT_BITS) - 1)
#define EFS_MIN_TIMEOUT_MS   10

static atomic64_t g_efs_gctr[UVM_EFS_GCTR_COUNT];

typedef struct
{
    bool                        in_use;
    NvU64                       id;
    NvU64                       gen;
    uvm_gpu_id_t                gpu_id;
    NvProcessorUuid             gpu_uuid;
    NvU64                       divert_real_ns;
    NvU64                       park_mono_ns;
    NvU32                       num_instances;

    // A copy of the packet as stock UVM parsed it. Its va_space/gpu pointers and
    // list head are cleared: nothing in it is ever dereferenced.
    uvm_fault_buffer_entry_t    entry;

    // On efs->undelivered until handed to userspace by UVM_EFS_WAIT.
    struct list_head            undelivered_node;
    bool                        delivered;
} uvm_efs_record_t;

// What a hardware action needs, copied out of a record under the lock.
typedef struct
{
    uvm_gpu_id_t                gpu_id;
    NvU64                       gen;
    uvm_fault_buffer_entry_t    entry;
} uvm_efs_saved_t;

struct uvm_efs_va_space_struct
{
    spinlock_t                  lock;
    wait_queue_head_t           wait_queue;
    struct mutex                shutdown_mutex;
    struct pid                 *owner;
    uvm_va_space_t             *va_space;

    bool                        closing;
    bool                        shut_down;
    NvU32                       max_records;
    NvU32                       num_parked;
    NvU32                       num_undelivered;
    NvU32                       free_hint;
    NvU64                       next_seq;
    NvU64                       gpu_gen[UVM_ID_MAX_GPUS];
    NvU64                       ctr[UVM_EFS_CTR_COUNT];

    struct list_head            undelivered;
    struct delayed_work         timeout_work;

    uvm_efs_record_t           *records;
};

bool uvm_efs_enabled(void)
{
    return uvm_efs_enable != 0;
}

bool uvm_efs_skip_diverted_replays(void)
{
    return uvm_efs_skip_replays != 0;
}

void uvm_efs_count_batch_replay_skipped(void)
{
    atomic64_inc(&g_efs_gctr[UVM_EFS_GCTR_BATCH_REPLAYS_SKIPPED]);
}

static NvU32 efs_timeout_ms(void)
{
    return max(uvm_efs_timeout_ms, (unsigned)EFS_MIN_TIMEOUT_MS);
}

// The timeout work runs at a quarter of the timeout, at most once a second.
static unsigned long efs_tick_jiffies(void)
{
    NvU32 tick_ms = min(efs_timeout_ms() / 4, 1000U);

    return msecs_to_jiffies(max(tick_ms, 1U));
}

static bool efs_caller_is_owner(uvm_efs_va_space_t *efs)
{
    return task_tgid(current) == efs->owner;
}

// Must be called with efs->lock held.
static void efs_record_free_locked(uvm_efs_va_space_t *efs, uvm_efs_record_t *rec)
{
    UVM_ASSERT(rec->in_use);

    if (!rec->delivered) {
        list_del_init(&rec->undelivered_node);
        UVM_ASSERT(efs->num_undelivered > 0);
        --efs->num_undelivered;
    }

    rec->in_use = false;
    UVM_ASSERT(efs->num_parked > 0);
    --efs->num_parked;
    efs->free_hint = (NvU32)(rec - efs->records);
}

static void efs_timeout_work_fn(struct work_struct *work);

NV_STATUS uvm_efs_va_space_alloc(uvm_efs_va_space_t **efs_out)
{
    uvm_efs_va_space_t *efs;
    NvU32 max_records = uvm_efs_max_records;

    *efs_out = NULL;

    if (max_records == 0 || max_records > EFS_MAX_RECORDS_CAP)
        return NV_ERR_INVALID_ARGUMENT;

    efs = uvm_kvmalloc_zero(sizeof(*efs));
    if (!efs)
        return NV_ERR_NO_MEMORY;

    efs->records = uvm_kvmalloc_zero(sizeof(*efs->records) * max_records);
    if (!efs->records) {
        uvm_kvfree(efs);
        return NV_ERR_NO_MEMORY;
    }

    spin_lock_init(&efs->lock);
    init_waitqueue_head(&efs->wait_queue);
    mutex_init(&efs->shutdown_mutex);
    INIT_LIST_HEAD(&efs->undelivered);
    INIT_DELAYED_WORK(&efs->timeout_work, efs_timeout_work_fn);
    efs->max_records = max_records;
    efs->next_seq = 1;
    efs->owner = get_pid(task_tgid(current));

    atomic64_inc(&g_efs_gctr[UVM_EFS_GCTR_EFS_VA_SPACES]);

    *efs_out = efs;
    return NV_OK;
}

void uvm_efs_va_space_attach(uvm_va_space_t *va_space, uvm_efs_va_space_t *efs)
{
    UVM_ASSERT(!va_space->efs);

    efs->va_space = va_space;
    va_space->efs = efs;
}

void uvm_efs_va_space_free_unattached(uvm_efs_va_space_t *efs)
{
    if (!efs)
        return;

    put_pid(efs->owner);
    uvm_kvfree(efs->records);
    uvm_kvfree(efs);
}

bool uvm_efs_try_divert(uvm_va_space_t *va_space, uvm_gpu_t *gpu, const uvm_fault_buffer_entry_t *entry)
{
    uvm_efs_va_space_t *efs = va_space->efs;
    uvm_efs_record_t *rec = NULL;
    NvU32 gpu_index;
    NvU32 i;

    if (!efs)
        return false;

    // A prefetch fault needs no service: stock treats it as an invalid prefetch
    // and it disappears on the next replay.
    if (entry->fault_access_type == UVM_FAULT_ACCESS_TYPE_PREFETCH)
        return false;

    // A parked fault must be precisely cancellable later (Volta+).
    if (!gpu->parent->fault_cancel_va_supported)
        return false;

    gpu_index = uvm_id_gpu_index(gpu->id);

    spin_lock(&efs->lock);

    if (efs->closing) {
        ++efs->ctr[UVM_EFS_CTR_REFUSED_CLOSING];
        spin_unlock(&efs->lock);
        return false;
    }

    // A re-fault of a parked page (another warp, or a replay issued by anyone)
    // merges into its record.
    for (i = 0; i < efs->max_records; i++) {
        uvm_efs_record_t *cur = &efs->records[i];

        if (!cur->in_use ||
            !uvm_id_equal(cur->gpu_id, gpu->id) ||
            cur->gen != efs->gpu_gen[gpu_index] ||
            cur->entry.fault_address != entry->fault_address)
            continue;

        cur->entry.access_type_mask |= entry->access_type_mask;
        if (entry->fault_access_type > cur->entry.fault_access_type)
            cur->entry.fault_access_type = entry->fault_access_type;
        cur->num_instances += entry->num_instances;
        ++efs->ctr[UVM_EFS_CTR_DEDUPED];
        spin_unlock(&efs->lock);
        return true;
    }

    if (efs->num_parked >= efs->max_records) {
        ++efs->ctr[UVM_EFS_CTR_REFUSED_FULL];
        spin_unlock(&efs->lock);
        return false;
    }

    for (i = 0; i < efs->max_records; i++) {
        NvU32 slot = (efs->free_hint + i) % efs->max_records;

        if (!efs->records[slot].in_use) {
            rec = &efs->records[slot];
            break;
        }
    }
    UVM_ASSERT(rec);

    memset(rec, 0, sizeof(*rec));
    rec->in_use = true;
    rec->id = (efs->next_seq++ << EFS_SLOT_BITS) | (NvU64)(rec - efs->records);
    rec->gen = efs->gpu_gen[gpu_index];
    rec->gpu_id = gpu->id;
    uvm_uuid_copy(&rec->gpu_uuid, &gpu->uuid);
    rec->divert_real_ns = ktime_get_real_ns();
    rec->park_mono_ns = ktime_get_ns();
    rec->num_instances = entry->num_instances;
    rec->entry = *entry;
    rec->entry.va_space = NULL;
    rec->entry.gpu = NULL;
    INIT_LIST_HEAD(&rec->entry.merged_instances_list);
    rec->delivered = false;
    list_add_tail(&rec->undelivered_node, &efs->undelivered);

    ++efs->num_parked;
    ++efs->num_undelivered;
    ++efs->ctr[UVM_EFS_CTR_DIVERTED];

    // Queued under the lock: uvm_efs_va_space_shutdown sets closing under the
    // same lock before cancelling the work synchronously, so no work can be
    // queued after that cancel.
    queue_delayed_work(system_wq, &efs->timeout_work, efs_tick_jiffies());

    spin_unlock(&efs->lock);

    wake_up_interruptible(&efs->wait_queue);

    return true;
}

// Looks up and retains the GPU a record names, if its GPU VA space is still
// registered in this VA space. Returns NULL otherwise.
static uvm_gpu_t *efs_retain_gpu(uvm_va_space_t *va_space, uvm_gpu_id_t gpu_id)
{
    uvm_gpu_t *gpu = NULL;

    uvm_va_space_down_read(va_space);

    if (uvm_processor_mask_test(&va_space->registered_gpu_va_spaces, gpu_id)) {
        gpu = uvm_gpu_get(gpu_id);
        if (gpu)
            uvm_gpu_retain(gpu);
    }

    uvm_va_space_up_read(va_space);

    return gpu;
}

// Cancels saved[0..n) in hardware on one GPU, then flushes the fault buffer and
// replays, as stock UVM does after its own precise cancels. Records whose GPU VA
// space was removed or replaced since they were parked are skipped. Returns the
// number of records that reached hardware.
static NvU32 efs_hw_cancel(uvm_va_space_t *va_space, uvm_efs_saved_t *saved, NvU32 n)
{
    uvm_efs_va_space_t *efs = va_space->efs;
    uvm_gpu_t *gpu;
    NvU32 i, cancelled = 0;
    NV_STATUS status;

    if (n == 0)
        return 0;

    gpu = efs_retain_gpu(va_space, saved[0].gpu_id);
    if (!gpu)
        return 0;

    uvm_parent_gpu_replayable_faults_isr_lock(gpu->parent);
    uvm_va_space_down_read(va_space);

    if (uvm_gpu_va_space_get(va_space, gpu)) {
        for (i = 0; i < n; i++) {
            uvm_fault_buffer_entry_t entry = saved[i].entry;

            UVM_ASSERT(uvm_id_equal(saved[i].gpu_id, gpu->id));

            // gpu_gen only changes under the VA space write lock.
            if (saved[i].gen != efs->gpu_gen[uvm_id_gpu_index(gpu->id)])
                continue;

            entry.va_space = va_space;
            entry.gpu = gpu;
            entry.is_fatal = true;
            entry.filtered = false;
            entry.fatal_reason = UvmEventFatalReasonInvalidAddress;
            entry.replayable.cancel_va_mode = UVM_FAULT_CANCEL_VA_MODE_ALL;
            INIT_LIST_HEAD(&entry.merged_instances_list);

            status = uvm_efs_hw_cancel_va_locked(&entry);
            if (status != NV_OK)
                UVM_ERR_PRINT("EFS cancel of 0x%llx failed: %s, GPU %s\n",
                              entry.fault_address, nvstatusToString(status), uvm_gpu_name(gpu));
            else
                ++cancelled;
        }
    }

    uvm_va_space_up_read(va_space);

    status = uvm_efs_hw_flush_and_replay_locked(gpu);
    if (status != NV_OK)
        UVM_ERR_PRINT("EFS flush after cancel failed: %s, GPU %s\n", nvstatusToString(status), uvm_gpu_name(gpu));

    uvm_parent_gpu_replayable_faults_isr_unlock(gpu->parent);
    uvm_gpu_release(gpu);

    return cancelled;
}

static bool efs_hw_replay(uvm_va_space_t *va_space, uvm_gpu_id_t gpu_id)
{
    uvm_gpu_t *gpu = efs_retain_gpu(va_space, gpu_id);
    NV_STATUS status;

    if (!gpu)
        return false;

    uvm_parent_gpu_replayable_faults_isr_lock(gpu->parent);
    status = uvm_efs_hw_replay_locked(gpu);
    uvm_parent_gpu_replayable_faults_isr_unlock(gpu->parent);

    if (status != NV_OK)
        UVM_ERR_PRINT("EFS replay failed: %s, GPU %s\n", nvstatusToString(status), uvm_gpu_name(gpu));

    uvm_gpu_release(gpu);

    return status == NV_OK;
}

// Cancels the given saved records GPU by GPU (usually one GPU).
static NvU32 efs_hw_cancel_all(uvm_va_space_t *va_space, uvm_efs_saved_t *saved, NvU32 n)
{
    NvU32 done = 0, cancelled = 0;

    // Group by GPU in place: stable enough for the handful of GPUs involved.
    while (done < n) {
        NvU32 i, end = done + 1;

        for (i = done + 1; i < n; i++) {
            if (uvm_id_equal(saved[i].gpu_id, saved[done].gpu_id)) {
                uvm_efs_saved_t tmp = saved[end];
                saved[end] = saved[i];
                saved[i] = tmp;
                ++end;
            }
        }

        cancelled += efs_hw_cancel(va_space, &saved[done], end - done);
        done = end;
    }

    return cancelled;
}

static void efs_timeout_work_fn(struct work_struct *work)
{
    uvm_efs_va_space_t *efs = container_of(to_delayed_work(work), uvm_efs_va_space_t, timeout_work);
    uvm_va_space_t *va_space = efs->va_space;
    uvm_efs_saved_t *saved;
    NvU64 now = ktime_get_ns();
    NvU64 timeout_ns = (NvU64)efs_timeout_ms() * 1000 * 1000;
    uvm_gpu_id_t replay_gpus[4];
    NvU32 num_replay_gpus = 0;
    NvU32 i, n = 0;
    bool requeue;

    saved = uvm_kvmalloc(sizeof(*saved) * efs->max_records);

    spin_lock(&efs->lock);

    for (i = 0; saved && i < efs->max_records; i++) {
        uvm_efs_record_t *rec = &efs->records[i];

        if (!rec->in_use)
            continue;

        if (now - rec->park_mono_ns >= timeout_ns) {
            saved[n].gpu_id = rec->gpu_id;
            saved[n].gen = rec->gen;
            saved[n].entry = rec->entry;
            ++n;
            efs_record_free_locked(efs, rec);
            ++efs->ctr[UVM_EFS_CTR_TIMED_OUT];
        }
        else if (num_replay_gpus < ARRAY_SIZE(replay_gpus)) {
            NvU32 j;
            bool seen = false;

            for (j = 0; j < num_replay_gpus; j++)
                seen = seen || uvm_id_equal(replay_gpus[j], rec->gpu_id);
            if (!seen)
                replay_gpus[num_replay_gpus++] = rec->gpu_id;
        }
    }

    spin_unlock(&efs->lock);

    if (n > 0)
        efs_hw_cancel_all(va_space, saved, n);

    // Safety replay: parked records suppress batch replays, so make sure a
    // fault some other tenant lost to a buffer overflow still gets replayed.
    for (i = 0; i < num_replay_gpus; i++) {
        if (efs_hw_replay(va_space, replay_gpus[i]))
            atomic64_inc(&g_efs_gctr[UVM_EFS_GCTR_SAFETY_REPLAYS]);
    }

    uvm_kvfree(saved);

    spin_lock(&efs->lock);
    requeue = !efs->closing && efs->num_parked > 0;
    if (requeue)
        queue_delayed_work(system_wq, &efs->timeout_work, efs_tick_jiffies());
    spin_unlock(&efs->lock);
}

void uvm_efs_va_space_shutdown(uvm_va_space_t *va_space)
{
    uvm_efs_va_space_t *efs = va_space->efs;
    uvm_efs_saved_t *saved;
    NvU32 i, n = 0, cancelled = 0;

    if (!efs)
        return;

    mutex_lock(&efs->shutdown_mutex);

    if (efs->shut_down) {
        mutex_unlock(&efs->shutdown_mutex);
        return;
    }

    spin_lock(&efs->lock);
    efs->closing = true;
    spin_unlock(&efs->lock);

    // No work can be queued after this: try_divert queues under the lock only
    // while !closing, and the work requeues itself only while !closing.
    cancel_delayed_work_sync(&efs->timeout_work);

    saved = uvm_kvmalloc(sizeof(*saved) * efs->max_records);

    spin_lock(&efs->lock);
    for (i = 0; i < efs->max_records; i++) {
        uvm_efs_record_t *rec = &efs->records[i];

        if (!rec->in_use)
            continue;

        if (saved) {
            saved[n].gpu_id = rec->gpu_id;
            saved[n].gen = rec->gen;
            saved[n].entry = rec->entry;
            ++n;
        }
        efs_record_free_locked(efs, rec);
    }
    spin_unlock(&efs->lock);

    if (uvm_efs_teardown_cancel && n > 0)
        cancelled = efs_hw_cancel_all(va_space, saved, n);

    uvm_kvfree(saved);

    spin_lock(&efs->lock);
    efs->ctr[UVM_EFS_CTR_TEARDOWN_CANCELLED] += cancelled;
    spin_unlock(&efs->lock);

    wake_up_all(&efs->wait_queue);

    UVM_INFO_PRINT("EFS VA space of tgid %d shut down: parked_at_shutdown=%u hw_cancelled=%u diverted=%llu "
                   "deduped=%llu delivered=%llu replayed=%llu cancelled=%llu timed_out=%llu refused_full=%llu "
                   "refused_closing=%llu stale=%llu gpu_gone=%llu hw_replays=%llu\n",
                   pid_nr(efs->owner), n, cancelled,
                   efs->ctr[UVM_EFS_CTR_DIVERTED], efs->ctr[UVM_EFS_CTR_DEDUPED],
                   efs->ctr[UVM_EFS_CTR_DELIVERED], efs->ctr[UVM_EFS_CTR_RESOLVED_REPLAY],
                   efs->ctr[UVM_EFS_CTR_RESOLVED_CANCEL], efs->ctr[UVM_EFS_CTR_TIMED_OUT],
                   efs->ctr[UVM_EFS_CTR_REFUSED_FULL], efs->ctr[UVM_EFS_CTR_REFUSED_CLOSING],
                   efs->ctr[UVM_EFS_CTR_STALE], efs->ctr[UVM_EFS_CTR_DROPPED_GPU_GONE],
                   efs->ctr[UVM_EFS_CTR_HW_REPLAYS]);

    efs->shut_down = true;
    mutex_unlock(&efs->shutdown_mutex);
}

void uvm_efs_va_space_free(uvm_va_space_t *va_space)
{
    uvm_efs_va_space_t *efs = va_space->efs;

    if (!efs)
        return;

    // Every destroy path already shut it down; this is the backstop.
    uvm_efs_va_space_shutdown(va_space);

    UVM_ASSERT(efs->num_parked == 0);
    UVM_ASSERT(!delayed_work_pending(&efs->timeout_work));

    va_space->efs = NULL;
    uvm_efs_va_space_free_unattached(efs);
}

void uvm_efs_gpu_va_space_removed(uvm_va_space_t *va_space, uvm_gpu_t *gpu)
{
    uvm_efs_va_space_t *efs = va_space->efs;
    NvU32 i, gpu_index;

    if (!efs)
        return;

    uvm_assert_rwsem_locked_write(&va_space->lock);

    gpu_index = uvm_id_gpu_index(gpu->id);

    spin_lock(&efs->lock);

    ++efs->gpu_gen[gpu_index];

    for (i = 0; i < efs->max_records; i++) {
        uvm_efs_record_t *rec = &efs->records[i];

        if (rec->in_use && uvm_id_equal(rec->gpu_id, gpu->id)) {
            efs_record_free_locked(efs, rec);
            ++efs->ctr[UVM_EFS_CTR_DROPPED_GPU_GONE];
        }
    }

    spin_unlock(&efs->lock);
}

NV_STATUS uvm_api_efs_query(UVM_EFS_QUERY_PARAMS *params, struct file *filp)
{
    uvm_va_space_t *va_space = uvm_fd_va_space(filp);
    uvm_efs_va_space_t *efs = va_space ? va_space->efs : NULL;
    NvU32 i;

    params->abiVersion = UVM_EFS_ABI_VERSION;
    params->moduleEnabled = uvm_efs_enabled();
    params->maxRecords = uvm_efs_max_records;
    params->timeoutMs = efs_timeout_ms();
    params->skipDivertedReplays = uvm_efs_skip_diverted_replays();
    params->active = efs != NULL;
    params->numParked = 0;
    params->numUndelivered = 0;
    memset(params->vaSpaceCounters, 0, sizeof(params->vaSpaceCounters));

    for (i = 0; i < UVM_EFS_GCTR_COUNT; i++)
        params->globalCounters[i] = atomic64_read(&g_efs_gctr[i]);

    if (efs) {
        spin_lock(&efs->lock);
        params->maxRecords = efs->max_records;
        params->numParked = efs->num_parked;
        params->numUndelivered = efs->num_undelivered;
        memcpy(params->vaSpaceCounters, efs->ctr, sizeof(params->vaSpaceCounters));
        spin_unlock(&efs->lock);
    }

    return NV_OK;
}

static void efs_fill_user_record(const uvm_efs_record_t *rec, UvmEfsFaultRecord *out)
{
    memset(out, 0, sizeof(*out));
    out->recordId = rec->id;
    out->faultAddress = rec->entry.fault_address;
    out->gpuTimestampNs = rec->entry.timestamp;
    out->divertTimeNs = rec->divert_real_ns;
    out->gpuUuid = rec->gpu_uuid;
    out->accessType = rec->entry.fault_access_type;
    out->accessTypeMask = rec->entry.access_type_mask;
    out->faultType = rec->entry.fault_type;
    out->clientType = rec->entry.fault_source.client_type;
    out->clientId = rec->entry.fault_source.client_id;
    out->gpcId = rec->entry.fault_source.gpc_id;
    out->utlbId = rec->entry.fault_source.utlb_id;
    out->veId = rec->entry.fault_source.ve_id;
    out->mmuEngineId = rec->entry.fault_source.mmu_engine_id;
    out->numInstances = rec->num_instances;
}

// Moves up to max undelivered records into out[]. Returns how many.
static NvU32 efs_take_undelivered(uvm_efs_va_space_t *efs, UvmEfsFaultRecord *out, NvU32 max, bool *closing)
{
    NvU32 n = 0;

    spin_lock(&efs->lock);

    while (n < max && !list_empty(&efs->undelivered)) {
        uvm_efs_record_t *rec = list_first_entry(&efs->undelivered, uvm_efs_record_t, undelivered_node);

        list_del_init(&rec->undelivered_node);
        rec->delivered = true;
        --efs->num_undelivered;
        efs_fill_user_record(rec, &out[n++]);
    }

    efs->ctr[UVM_EFS_CTR_DELIVERED] += n;
    *closing = efs->closing;

    spin_unlock(&efs->lock);

    return n;
}

NV_STATUS uvm_api_efs_wait(UVM_EFS_WAIT_PARAMS *params, struct file *filp)
{
    uvm_va_space_t *va_space = uvm_va_space_get(filp);
    uvm_efs_va_space_t *efs = va_space->efs;
    UvmEfsFaultRecord *out;
    NvU32 max = params->maxRecords;
    NvU32 timeout_us = min(params->timeoutUs, (NvU32)UVM_EFS_MAX_WAIT_TIMEOUT_US);
    NvU32 n;
    bool closing = false;
    NV_STATUS status = NV_OK;

    params->numRecords = 0;

    if (!efs)
        return NV_ERR_NOT_SUPPORTED;

    if (!efs_caller_is_owner(efs))
        return NV_ERR_INSUFFICIENT_PERMISSIONS;

    if (max == 0 || max > UVM_EFS_MAX_WAIT_RECORDS)
        return NV_ERR_INVALID_ARGUMENT;

    out = uvm_kvmalloc(sizeof(*out) * max);
    if (!out)
        return NV_ERR_NO_MEMORY;

    n = efs_take_undelivered(efs, out, max, &closing);
    if (n == 0 && timeout_us > 0 && !closing) {
        long ret = wait_event_interruptible_timeout(efs->wait_queue,
                                                    READ_ONCE(efs->num_undelivered) > 0 || READ_ONCE(efs->closing),
                                                    usecs_to_jiffies(timeout_us));
        // A signal or a timeout both return with whatever is there (maybe nothing).
        (void)ret;
        n = efs_take_undelivered(efs, out, max, &closing);
    }

    // ⚠ Records are marked delivered before the copy: a faulting user buffer
    // loses them to userspace and they expire through the timeout (cancelled).
    if (n > 0 && copy_to_user((void __user *)(uintptr_t)params->records, out, sizeof(*out) * n))
        status = NV_ERR_INVALID_ADDRESS;
    else
        params->numRecords = n;

    uvm_kvfree(out);

    return status;
}

NV_STATUS uvm_api_efs_resolve(UVM_EFS_RESOLVE_PARAMS *params, struct file *filp)
{
    uvm_va_space_t *va_space = uvm_va_space_get(filp);
    uvm_efs_va_space_t *efs = va_space->efs;
    NvU64 *ids = NULL;
    uvm_efs_saved_t *saved = NULL;
    NvU32 count = params->count;
    NvU32 i, n = 0, stale = 0;
    NV_STATUS status = NV_OK;

    params->numResolved = 0;
    params->numStale = 0;

    if (!efs)
        return NV_ERR_NOT_SUPPORTED;

    if (!efs_caller_is_owner(efs))
        return NV_ERR_INSUFFICIENT_PERMISSIONS;

    if (count == 0 || count > UVM_EFS_MAX_RESOLVE_RECORDS)
        return NV_ERR_INVALID_ARGUMENT;

    if (params->action != UVM_EFS_ACTION_REPLAY && params->action != UVM_EFS_ACTION_CANCEL)
        return NV_ERR_INVALID_ARGUMENT;

    ids = uvm_kvmalloc(sizeof(*ids) * count);
    saved = uvm_kvmalloc(sizeof(*saved) * count);
    if (!ids || !saved) {
        status = NV_ERR_NO_MEMORY;
        goto out;
    }

    if (copy_from_user(ids, (const void __user *)(uintptr_t)params->recordIds, sizeof(*ids) * count)) {
        status = NV_ERR_INVALID_ADDRESS;
        goto out;
    }

    spin_lock(&efs->lock);

    for (i = 0; i < count; i++) {
        NvU64 slot = ids[i] & EFS_SLOT_MASK;
        uvm_efs_record_t *rec;

        if (slot >= efs->max_records) {
            ++stale;
            continue;
        }

        rec = &efs->records[slot];
        if (!rec->in_use || rec->id != ids[i]) {
            ++stale;
            continue;
        }

        if (rec->gen != efs->gpu_gen[uvm_id_gpu_index(rec->gpu_id)]) {
            // Its GPU VA space went away: nothing in hardware to act on.
            efs_record_free_locked(efs, rec);
            ++stale;
            continue;
        }

        saved[n].gpu_id = rec->gpu_id;
        saved[n].gen = rec->gen;
        saved[n].entry = rec->entry;
        ++n;
        efs_record_free_locked(efs, rec);
    }

    efs->ctr[UVM_EFS_CTR_STALE] += stale;
    if (params->action == UVM_EFS_ACTION_REPLAY)
        efs->ctr[UVM_EFS_CTR_RESOLVED_REPLAY] += n;
    else
        efs->ctr[UVM_EFS_CTR_RESOLVED_CANCEL] += n;

    spin_unlock(&efs->lock);

    params->numResolved = n;
    params->numStale = stale;

    if (n == 0)
        goto out;

    if (params->action == UVM_EFS_ACTION_CANCEL) {
        efs_hw_cancel_all(va_space, saved, n);
    }
    else {
        // One replay per GPU involved. The mapping ioctls the owner issued before
        // this call have completed their PTE writes and TLB invalidates (they wait
        // on their trackers), so the replayed accesses see the new mappings.
        uvm_gpu_id_t done[4];
        NvU32 num_done = 0, j;

        for (i = 0; i < n; i++) {
            bool seen = false;

            for (j = 0; j < num_done; j++)
                seen = seen || uvm_id_equal(done[j], saved[i].gpu_id);
            if (seen)
                continue;

            if (efs_hw_replay(va_space, saved[i].gpu_id)) {
                spin_lock(&efs->lock);
                ++efs->ctr[UVM_EFS_CTR_HW_REPLAYS];
                spin_unlock(&efs->lock);
            }

            if (num_done < ARRAY_SIZE(done))
                done[num_done++] = saved[i].gpu_id;
        }
    }

out:
    uvm_kvfree(saved);
    uvm_kvfree(ids);
    return status;
}
