// nvkprobe — STEP-1 CONFIRMATION probe for the Mode-2 ESCHED-no-fetch blocker.
// Hypothesis: host RM's GR/CE ctx auto-promote (kgrctxMapCtxBuffers -> kgraphicsMapCtxBuffer
// -> dmaAllocMapping_GM107 -> gvaspaceAlloc) fails "can't alloc VA space for mapping" INTO
// our forwarded per-channel VAS (0xce2xxxxx), so the engine ctx is never resident.
// trace_kprobe refuses these (notrace module), so this is a raw kretprobe module:
//   - gvaspaceAlloc_IMPL: capture (pGVAS,size,align,rangeLo,rangeHi,mask)+stack at entry,
//     printk ONLY on failing return (the "can't alloc VA" inner call).
//   - dmaAllocMapping_GM107: capture (pVAS,pMemDesc,va,flags)+stack, printk on failure.
//   - kgraphicsMapCtxBuffer_IMPL: entry log (first N) — proves GR-ctx maps target which pVAS.
// Load: insmod nvkprobe.ko ; results in dmesg, prefix "NVKP".
#include <linux/module.h>
#include <linux/kprobes.h>
#include <linux/stacktrace.h>

#define NVKP_STACK_DEPTH 16
#define NVKP_MAX_FULL    48   /* full (with stack) reports per probe; then count only */

struct nvkp_data {
    u64 a[6];
    u64 pa;                       /* arg8: NvU64 *pAddr (gva) / pVaddr (dam, ==a[2]) */
    unsigned long stack[NVKP_STACK_DEPTH];
    unsigned int nstack;
};

static atomic_t gva_fail_full = ATOMIC_INIT(0);
static atomic_t gva_fail_total = ATOMIC_INIT(0);
static atomic_t gva_ok_full = ATOMIC_INIT(0);
static atomic_t dam_fail_full = ATOMIC_INIT(0);
static atomic_t dam_fail_total = ATOMIC_INIT(0);
static atomic_t dam_ok_full = ATOMIC_INIT(0);
static atomic_t kmap_seen = ATOMIC_INIT(0);

/* ---------- gvaspaceAlloc_IMPL(pGVAS,size,align,rangeLo,rangeHi,mask,flags,pAddr) ---------- */
static int gva_entry(struct kretprobe_instance *ri, struct pt_regs *regs)
{
    struct nvkp_data *d = (struct nvkp_data *)ri->data;
    u64 pa = 0;
    d->a[0] = regs->di; d->a[1] = regs->si; d->a[2] = regs->dx;
    d->a[3] = regs->cx; d->a[4] = regs->r8; d->a[5] = regs->r9;
    /* arg8 (NvU64 *pAddr) is on the stack: [sp+16] at function entry */
    copy_from_kernel_nofault(&pa, (void *)(regs->sp + 16), sizeof(pa));
    d->pa = pa;
    d->nstack = stack_trace_save(d->stack, NVKP_STACK_DEPTH, 1);
    return 0;
}
static int gva_ret(struct kretprobe_instance *ri, struct pt_regs *regs)
{
    struct nvkp_data *d = (struct nvkp_data *)ri->data;
    u32 st = (u32)regs_return_value(regs);
    if (st == 0) {
        /* SUCCESS logging for the TOP-OF-HEAP (RM-internal) region: who owns the VA the
         * GR FE later faults on (guest-pool-base + 0xe00000)? */
        u64 va = 0;
        if (d->pa)
            copy_from_kernel_nofault(&va, (void *)d->pa, sizeof(va));
        if (va >= 0x400000000000ULL && atomic_inc_return(&gva_ok_full) <= 200) {
            unsigned int i;
            pr_info("NVKP gvaspaceAlloc OK gvas=%px size=0x%llx lo=0x%llx hi=0x%llx "
                    "-> va=0x%llx\n", (void *)d->a[0], d->a[1], d->a[3], d->a[4], va);
            for (i = 0; i < d->nstack && i < 6; i++)
                pr_info("NVKP   gvaok-stk %pS\n", (void *)d->stack[i]);
        }
        return 0;
    }
    atomic_inc(&gva_fail_total);
    if (atomic_inc_return(&gva_fail_full) <= NVKP_MAX_FULL) {
        unsigned int i;
        pr_info("NVKP gvaspaceAlloc FAIL st=0x%x gvas=%px size=0x%llx align=0x%llx "
                "lo=0x%llx hi=0x%llx mask=0x%llx\n",
                st, (void *)d->a[0], d->a[1], d->a[2], d->a[3], d->a[4], d->a[5]);
        for (i = 0; i < d->nstack; i++)
            pr_info("NVKP   gva-stk %pS\n", (void *)d->stack[i]);
    }
    return 0;
}
static struct kretprobe rp_gva = {
    .kp.symbol_name = "gvaspaceAlloc_IMPL",
    .entry_handler = gva_entry,
    .handler = gva_ret,
    .data_size = sizeof(struct nvkp_data),
    .maxactive = 64,
};

/* ------- dmaAllocMapping_GM107(pGpu,pDma,pVAS,pMemDesc,pVaddr,flags,flags2,...) ------- */
static int dam_entry(struct kretprobe_instance *ri, struct pt_regs *regs)
{
    struct nvkp_data *d = (struct nvkp_data *)ri->data;
    u64 vaddr = 0;
    d->a[0] = regs->dx;          /* pVAS */
    d->a[1] = regs->cx;          /* pMemDesc */
    d->a[2] = regs->r8;          /* pVaddr */
    d->a[3] = regs->r9;          /* flags */
    if (d->a[2])
        copy_from_kernel_nofault(&vaddr, (void *)d->a[2], sizeof(vaddr));
    d->a[4] = vaddr;
    d->nstack = stack_trace_save(d->stack, NVKP_STACK_DEPTH, 1);
    return 0;
}
static int dam_ret(struct kretprobe_instance *ri, struct pt_regs *regs)
{
    struct nvkp_data *d = (struct nvkp_data *)ri->data;
    u32 st = (u32)regs_return_value(regs);
    if (st == 0) {
        /* SUCCESS logging for allocator-chosen mappings (entry *pVaddr==0): these are the
         * host RM's internal maps (ctx buffers etc.) — log the RESULT VA + caller. */
        if (d->a[4] == 0 && d->a[2] && atomic_inc_return(&dam_ok_full) <= 200) {
            u64 va = 0; unsigned int i;
            copy_from_kernel_nofault(&va, (void *)d->a[2], sizeof(va));
            pr_info("NVKP dmaAllocMapping OK vas=%px memdesc=%px -> va=0x%llx flags=0x%llx\n",
                    (void *)d->a[0], (void *)d->a[1], va, d->a[3]);
            for (i = 0; i < d->nstack && i < 6; i++)
                pr_info("NVKP   damok-stk %pS\n", (void *)d->stack[i]);
        }
        return 0;
    }
    atomic_inc(&dam_fail_total);
    if (atomic_inc_return(&dam_fail_full) <= NVKP_MAX_FULL) {
        unsigned int i;
        pr_info("NVKP dmaAllocMapping FAIL st=0x%x vas=%px memdesc=%px va-in=0x%llx "
                "flags=0x%llx\n",
                st, (void *)d->a[0], (void *)d->a[1], d->a[4], d->a[3]);
        for (i = 0; i < d->nstack; i++)
            pr_info("NVKP   dam-stk %pS\n", (void *)d->stack[i]);
    }
    return 0;
}
static struct kretprobe rp_dam = {
    .kp.symbol_name = "dmaAllocMapping_GM107",
    .entry_handler = dam_entry,
    .handler = dam_ret,
    .data_size = sizeof(struct nvkp_data),
    .maxactive = 64,
};

/* ------- kgraphicsMapCtxBuffer_IMPL(pGpu,pKernelGraphics,pMemDesc,pVAS,...) ------- */
static int kmap_pre(struct kprobe *p, struct pt_regs *regs)
{
    int n = atomic_inc_return(&kmap_seen);
    if (n <= NVKP_MAX_FULL)
        pr_info("NVKP kgraphicsMapCtxBuffer #%d memdesc=%px vas=%px\n",
                n, (void *)regs->dx, (void *)regs->cx);
    return 0;
}
static struct kprobe kp_kmap = {
    .symbol_name = "kgraphicsMapCtxBuffer_IMPL",
    .pre_handler = kmap_pre,
};

static int __init nvkp_init(void)
{
    int rc;
    rc = register_kretprobe(&rp_gva);
    pr_info("NVKP gvaspaceAlloc_IMPL kretprobe: %d\n", rc);
    rc = register_kretprobe(&rp_dam);
    pr_info("NVKP dmaAllocMapping_GM107 kretprobe: %d\n", rc);
    rc = register_kprobe(&kp_kmap);
    pr_info("NVKP kgraphicsMapCtxBuffer_IMPL kprobe: %d\n", rc);
    return 0;
}
static void __exit nvkp_exit(void)
{
    unregister_kretprobe(&rp_gva);
    unregister_kretprobe(&rp_dam);
    unregister_kprobe(&kp_kmap);
    pr_info("NVKP unloaded: gva_fail=%d dam_fail=%d kmap=%d\n",
            atomic_read(&gva_fail_total), atomic_read(&dam_fail_total),
            atomic_read(&kmap_seen));
}
module_init(nvkp_init);
module_exit(nvkp_exit);
MODULE_LICENSE("GPL");
MODULE_DESCRIPTION("nvkvm Mode-2 STEP-1 confirmation kprobes");
