/*
 * nv_rpctrace.c — a recorder for every GSP-RM message-queue element, request and
 * reply, with full bodies, captured inside CPU-RM on real hardware.
 *
 * SPDX-License-Identifier: MIT
 * Part of `kayfabe`. ⊘ Nothing here is copied from NVIDIA's tree. This file is
 * dropped into `kernel-open/nvidia/` of an `open-gpu-kernel-modules` 580.159.04
 * checkout by `scripts/rpctrace/build_instrumented.sh`.
 *
 * ────────────────────────────────────────────────────────────────────────────
 * WHY THE RECORDER LIVES HERE AND THE HOOKS LIVE IN src/nvidia
 *
 * The two functions that hold a complete message-queue element —
 * `GspMsgQueueSendCommand` and `GspMsgQueueReceiveStatus` — are in `src/nvidia`,
 * which is compiled OS-agnostically and links against the OS layer by undefined
 * symbol (see `src/nvidia/exports_link_command.txt`). It cannot call `vmalloc`
 * or `proc_create`. So the hooks are three lines each in `src/nvidia`, and
 * everything that touches the kernel is here, resolved at module link exactly
 * like the driver's own `os_*` calls.
 *
 * ★★★ WHY NOT printk. `cap1_coldboot_hermetic` is 359,062 records for a boot
 * that did not even finish. The kernel ring buffer drops silently and does not
 * tell the reader WHICH lines went, and a consumer's diff over this trace is
 * positional — one dropped element shifts every later index. A hole that
 * announces itself is a usable trace; a hole that does not is a wrong answer.
 * Hence: binary records in a vmalloc ring, drained through procfs, with the
 * drop counters carried IN the file so the decoder can refuse.
 * ────────────────────────────────────────────────────────────────────────────
 */

#define  __NO_VERSION__

#include "nv-linux.h"
#include "os-interface.h"
#include "nv-procfs.h"
#include "nv_rpctrace.h"

#if defined(CONFIG_PROC_FS)

/*
 * Ring size in KiB. 0 disables the recorder entirely (nothing is allocated and
 * every hook is a counter-free early return), which is how the instrumented
 * module can be left in place without paying for it.
 *
 * ★ Exposed as a parameter on purpose, and not only for convenience: forcing the
 * ring to overflow is the only way to prove the overflow guard fires, and a
 * guard nobody has watched fire is a guard nobody has tested. See
 * `scripts/rpctrace/capture.sh --break-overflow`.
 */
static unsigned int NVreg_RpcTraceKB = 65536;   /* 64 MiB */
module_param_named(NVreg_RpcTraceKB, NVreg_RpcTraceKB, uint, 0444);
MODULE_PARM_DESC(NVreg_RpcTraceKB,
    "Size in KiB of the GSP RPC trace ring (0 = disabled).");

/*
 * Hard ceiling on a single element. GSP_MSG_QUEUE_ELEMENT_SIZE_MAX is
 * RM_PAGE_SIZE * 16 == 64 KiB in 580.159.04. We do NOT include the driver's
 * header to learn that — this file must not depend on `src/nvidia` — so the
 * constant is repeated here and anything larger is REFUSED rather than
 * truncated. ⊘ A truncated body recorded under a full length is precisely the
 * `dlen=0` defect wearing a different hat.
 */
#define NV_RPCTRACE_MAX_ELEM (16u * 4096u)

static DEFINE_SPINLOCK(rpctrace_lock);

static char      *rpctrace_ring;        /* vmalloc'd, rpctrace_capacity bytes */
static u64        rpctrace_capacity;
static u64        rpctrace_used;
static u64        rpctrace_n_records;
static u64        rpctrace_n_payload;
static u64        rpctrace_n_dropped;
static u64        rpctrace_n_dropped_bytes;
static u64        rpctrace_n_refused_empty;
static u64        rpctrace_n_rx_failed;
static u64        rpctrace_t0_ns;
static u32        rpctrace_seq;
static u64        rpctrace_last_off;    /* offset of the most recent record  */
static u32        rpctrace_last_seq = NV_RPCTRACE_NO_REC;

static struct proc_dir_entry *rpctrace_entry;

static void rpctrace_fill_hdr(struct nv_rpctrace_file_hdr *h, u64 used)
{
    memset(h, 0, sizeof(*h));
    h->magic           = NV_RPCTRACE_FILE_MAGIC;
    h->version         = NV_RPCTRACE_VERSION;
    h->file_hdr_size   = (u32)sizeof(struct nv_rpctrace_file_hdr);
    h->rec_hdr_size    = (u32)sizeof(struct nv_rpctrace_rec_hdr);
    h->capacity        = rpctrace_capacity;
    h->used            = used;
    h->n_records       = rpctrace_n_records;
    h->n_payload_bytes = rpctrace_n_payload;
    h->n_dropped       = rpctrace_n_dropped;
    h->n_dropped_bytes = rpctrace_n_dropped_bytes;
    h->n_refused_empty = rpctrace_n_refused_empty;
    h->n_rx_failed     = rpctrace_n_rx_failed;
    h->t0_ns           = rpctrace_t0_ns;
    if (rpctrace_n_dropped != 0)
        h->flags |= NV_RPCTRACE_FF_OVERFLOWED;
    if (rpctrace_ring == NULL)
        h->flags |= NV_RPCTRACE_FF_DISABLED;
    strncpy(h->drv_version, NV_VERSION_STRING, sizeof(h->drv_version) - 1);
}

/*
 * ★★★ THE APPEND, AND WHY IT IS SHAPED LIKE THIS.
 *
 * Every early return below happens BEFORE the write cursor moves. There is no
 * ordering of these statements in which a header lands and its bytes do not:
 * the capacity test covers header + payload + padding as one quantity, and the
 * cursor advances only after both memcpy()s have run. That is the whole
 * argument for `cap_len` being trustworthy, and it is why it is worth reading
 * this function rather than the format comment.
 *
 * Called from the RPC transmit/receive paths, which can run in a bottom half —
 * hence irqsave. Up to 64 KiB of memcpy under a spinlock is a real latency cost;
 * this is a diagnostic build and is not left on the bench.
 */
unsigned int nv_rpctrace_record(unsigned int dir,
                                unsigned int rpc_fn,
                                unsigned int rpc_len,
                                unsigned int elem_seq,
                                unsigned int rpc_status,
                                unsigned int flags,
                                const void  *elem,
                                unsigned int cap_len)
{
    struct nv_rpctrace_rec_hdr hdr;
    unsigned long irqflags;
    u64 padded, need, off;
    u32 seq;

    if (rpctrace_ring == NULL)
        return NV_RPCTRACE_NO_REC;

    /*
     * ⊘ NOTHING TO RECORD IS NOT A RECORD. A caller with no bytes, a NULL
     * pointer, or an implausible length gets a counter bump and no row. The
     * counter is in the file header, so "we refused N things" is visible to the
     * decoder — as opposed to the C table's empty rows, which were
     * indistinguishable from a genuine zero-length answer.
     */
    if (elem == NULL || cap_len == 0 || cap_len > NV_RPCTRACE_MAX_ELEM) {
        spin_lock_irqsave(&rpctrace_lock, irqflags);
        rpctrace_n_refused_empty++;
        spin_unlock_irqrestore(&rpctrace_lock, irqflags);
        return NV_RPCTRACE_NO_REC;
    }

    padded = ((u64)cap_len + 7ull) & ~7ull;
    need   = sizeof(hdr) + padded;

    spin_lock_irqsave(&rpctrace_lock, irqflags);

    if (rpctrace_used + need > rpctrace_capacity) {
        /* Fill-and-stop: keep the ordered prefix, refuse the rest, and say so. */
        rpctrace_n_dropped++;
        rpctrace_n_dropped_bytes += cap_len;
        spin_unlock_irqrestore(&rpctrace_lock, irqflags);
        return NV_RPCTRACE_NO_REC;
    }

    seq = rpctrace_seq++;
    off = rpctrace_used;

    memset(&hdr, 0, sizeof(hdr));
    hdr.magic      = NV_RPCTRACE_REC_MAGIC;
    hdr.dir        = (u16)dir;
    hdr.flags      = (u16)flags;
    hdr.seq        = seq;
    hdr.elem_seq   = elem_seq;
    hdr.ts_ns      = ktime_get_ns();
    hdr.rpc_fn     = rpc_fn;
    hdr.rpc_len    = rpc_len;
    hdr.rpc_status = rpc_status;
    hdr.outcome    = 0;
    hdr.cap_len    = cap_len;

    memcpy(rpctrace_ring + off, &hdr, sizeof(hdr));
    memcpy(rpctrace_ring + off + sizeof(hdr), elem, cap_len);
    if (padded > cap_len)
        memset(rpctrace_ring + off + sizeof(hdr) + cap_len, 0,
               (size_t)(padded - cap_len));

    rpctrace_used     = off + need;   /* ← the cursor moves LAST, and only here */
    rpctrace_n_records++;
    rpctrace_n_payload += cap_len;
    rpctrace_last_off  = off;
    rpctrace_last_seq  = seq;

    spin_unlock_irqrestore(&rpctrace_lock, irqflags);
    return seq;
}

void nv_rpctrace_set_outcome(unsigned int seq, unsigned int status)
{
    unsigned long irqflags;

    if (rpctrace_ring == NULL || seq == NV_RPCTRACE_NO_REC)
        return;

    spin_lock_irqsave(&rpctrace_lock, irqflags);
    /*
     * Only the most recent record can be stamped, and only if it is still the
     * one the caller means. A stale seq is dropped rather than applied to
     * whatever now occupies that offset.
     */
    if (rpctrace_last_seq == seq) {
        struct nv_rpctrace_rec_hdr *h =
            (struct nv_rpctrace_rec_hdr *)(rpctrace_ring + rpctrace_last_off);
        h->outcome = status;
        if (status != 0)
            h->flags |= NV_RPCTRACE_F_NOT_SENT;
    }
    spin_unlock_irqrestore(&rpctrace_lock, irqflags);
}

void nv_rpctrace_note_rx_error(unsigned int status)
{
    unsigned long irqflags;

    (void)status;
    if (rpctrace_ring == NULL)
        return;

    spin_lock_irqsave(&rpctrace_lock, irqflags);
    rpctrace_n_rx_failed++;
    spin_unlock_irqrestore(&rpctrace_lock, irqflags);
}

/* ---------------------------------------------------------------- procfs -- */

/*
 * Snapshot taken at open(2). The file a reader sees is `hdr` followed by exactly
 * `hdr.used` bytes of ring — self-consistent even if the driver keeps recording
 * during the drain, because `used` only ever grows and the prefix never moves.
 */
struct rpctrace_snapshot {
    struct nv_rpctrace_file_hdr hdr;
};

static int rpctrace_open(struct inode *inode, struct file *file)
{
    struct rpctrace_snapshot *snap;
    unsigned long irqflags;

    snap = kzalloc(sizeof(*snap), GFP_KERNEL);
    if (snap == NULL)
        return -ENOMEM;

    spin_lock_irqsave(&rpctrace_lock, irqflags);
    rpctrace_fill_hdr(&snap->hdr, rpctrace_used);
    spin_unlock_irqrestore(&rpctrace_lock, irqflags);

    file->private_data = snap;
    return 0;
}

static int rpctrace_release(struct inode *inode, struct file *file)
{
    kfree(file->private_data);
    file->private_data = NULL;
    return 0;
}

static ssize_t rpctrace_read(struct file *file, char __user *ubuf,
                             size_t count, loff_t *ppos)
{
    struct rpctrace_snapshot *snap = file->private_data;
    u64 hdr_size, total;
    loff_t pos = *ppos;
    size_t done = 0;

    if (snap == NULL)
        return -EINVAL;

    hdr_size = sizeof(snap->hdr);
    total    = hdr_size + snap->hdr.used;

    if (pos < 0 || (u64)pos >= total)
        return 0;

    if (count > (size_t)(total - (u64)pos))
        count = (size_t)(total - (u64)pos);

    if ((u64)pos < hdr_size) {
        size_t n = (size_t)min_t(u64, (u64)count, hdr_size - (u64)pos);

        if (copy_to_user(ubuf, ((const char *)&snap->hdr) + pos, n))
            return -EFAULT;
        done  += n;
        pos   += n;
        ubuf  += n;
        count -= n;
    }

    if (count > 0 && rpctrace_ring != NULL) {
        u64 off = (u64)pos - hdr_size;

        if (copy_to_user(ubuf, rpctrace_ring + off, count))
            return -EFAULT;
        done += count;
        pos  += count;
    }

    *ppos = pos;
    return (ssize_t)done;
}

static const nv_proc_ops_t rpctrace_fops = {
    NV_PROC_OPS_SET_OWNER()
    .NV_PROC_OPS_OPEN    = rpctrace_open,
    .NV_PROC_OPS_READ    = rpctrace_read,
    .NV_PROC_OPS_LSEEK   = default_llseek,
    .NV_PROC_OPS_RELEASE = rpctrace_release,
};

int nv_rpctrace_procfs_init(struct proc_dir_entry *parent)
{
    u64 bytes = (u64)NVreg_RpcTraceKB * 1024ull;

    rpctrace_t0_ns = ktime_get_ns();

    if (bytes == 0) {
        printk(KERN_NOTICE "NVRM: rpctrace: disabled (NVreg_RpcTraceKB=0)\n");
    } else {
        rpctrace_ring = vmalloc(bytes);
        if (rpctrace_ring == NULL) {
            /*
             * ⊘ Fail LOUD and leave the recorder disarmed rather than quietly
             * running with a smaller ring. A capture that silently shrank is a
             * capture that silently dropped.
             */
            printk(KERN_ERR "NVRM: rpctrace: vmalloc(%llu) FAILED — recorder disarmed\n",
                   bytes);
            return -ENOMEM;
        }
        rpctrace_capacity = bytes;
        printk(KERN_NOTICE "NVRM: rpctrace: armed, ring %llu bytes, rec_hdr %u bytes\n",
               bytes, (unsigned)sizeof(struct nv_rpctrace_rec_hdr));
    }

    rpctrace_entry = proc_create_data("rpctrace", S_IFREG | S_IRUSR, parent,
                                      &rpctrace_fops, NULL);
    if (rpctrace_entry == NULL) {
        vfree(rpctrace_ring);
        rpctrace_ring = NULL;
        rpctrace_capacity = 0;
        return -ENOMEM;
    }
    return 0;
}

void nv_rpctrace_shutdown(void)
{
    /* The proc entry is a child of proc_nvidia and is removed with it. */
    rpctrace_entry = NULL;
    vfree(rpctrace_ring);
    rpctrace_ring = NULL;
    rpctrace_capacity = 0;
    rpctrace_used = 0;
}

#else  /* !CONFIG_PROC_FS */

unsigned int nv_rpctrace_record(unsigned int dir, unsigned int rpc_fn,
                                unsigned int rpc_len, unsigned int elem_seq,
                                unsigned int rpc_status, unsigned int flags,
                                const void *elem, unsigned int cap_len)
{
    return NV_RPCTRACE_NO_REC;
}
void nv_rpctrace_set_outcome(unsigned int seq, unsigned int status) { }
void nv_rpctrace_note_rx_error(unsigned int status) { }
void nv_rpctrace_shutdown(void) { }

#endif /* CONFIG_PROC_FS */
