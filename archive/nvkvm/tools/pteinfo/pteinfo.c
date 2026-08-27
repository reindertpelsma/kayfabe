// Throwaway diagnostic: walk (pid,va) page table, decode the leaf PTE's
// x86 PAT index (PAT/PCD/PWT bits) -> effective guest memory type.
// Usage: insmod pteinfo.ko pid=<P> va=0x<hex>
#include <linux/module.h>
#include <linux/kernel.h>
#include <linux/sched.h>
#include <linux/sched/task.h>
#include <linux/mm.h>
#include <linux/pid.h>
#include <linux/pgtable.h>
#include <asm/pgtable_types.h>

static int pid = 0;
static unsigned long va = 0;
module_param(pid, int, 0);
module_param(va, ulong, 0);
MODULE_LICENSE("GPL");

static const char *pat_name(unsigned idx)
{
    // Linux default PAT MSR: 0:WB 1:WC 2:UC- 3:UC 4:WB 5:WC 6:UC- 7:UC(WP)
    static const char *n[8] = {"WB","WC","UC-","UC","WB","WC","UC-","UC/WP"};
    return n[idx & 7];
}

static int __init pteinfo_init(void)
{
    struct task_struct *t;
    struct mm_struct *mm;
    pgd_t *pgd; p4d_t *p4d; pud_t *pud; pmd_t *pmd; pte_t *pte;
    unsigned long pteval, idx, pfn;
    int level = 4;

    rcu_read_lock();
    t = pid_task(find_vpid(pid), PIDTYPE_PID);
    if (t) get_task_struct(t);
    rcu_read_unlock();
    if (!t) { pr_info("pteinfo: no task pid=%d\n", pid); return 0; }
    mm = get_task_mm(t);
    if (!mm) { pr_info("pteinfo: no mm\n"); put_task_struct(t); return 0; }

    mmap_read_lock(mm);
    pgd = pgd_offset(mm, va);
    if (pgd_none(*pgd) || pgd_bad(*pgd)) { pr_info("pteinfo: pgd none\n"); goto out; }
    p4d = p4d_offset(pgd, va);
    if (p4d_none(*p4d) || p4d_bad(*p4d)) { pr_info("pteinfo: p4d none\n"); goto out; }
    pud = pud_offset(p4d, va);
    if (pud_none(*pud)) { pr_info("pteinfo: pud none\n"); goto out; }
    if (pud_large(*pud)) {
        pteval = pud_val(*pud);
        idx = ((pteval>>12)&1)<<2 | ((pteval&_PAGE_PCD)?2:0) | ((pteval&_PAGE_PWT)?1:0);
        pfn = (pteval & PTE_PFN_MASK) >> PAGE_SHIFT;
        level = 1; goto decode;
    }
    pmd = pmd_offset(pud, va);
    if (pmd_none(*pmd)) { pr_info("pteinfo: pmd none\n"); goto out; }
    if (pmd_large(*pmd)) {
        pteval = pmd_val(*pmd);
        idx = ((pteval>>12)&1)<<2 | ((pteval&_PAGE_PCD)?2:0) | ((pteval&_PAGE_PWT)?1:0);
        pfn = (pteval & PTE_PFN_MASK) >> PAGE_SHIFT;
        level = 2; goto decode;
    }
    pte = pte_offset_kernel(pmd, va);
    pteval = pte_val(*pte);
    // leaf PTE PAT bit is bit 7 (_PAGE_PAT)
    idx = ((pteval&_PAGE_PAT)?4:0) | ((pteval&_PAGE_PCD)?2:0) | ((pteval&_PAGE_PWT)?1:0);
    pfn = (pteval & PTE_PFN_MASK) >> PAGE_SHIFT;
decode:
    pr_info("pteinfo: pid=%d va=0x%lx level=%d pteval=0x%lx present=%ld PAT_idx=%lu type=%s pfn(GPA)=0x%lx\n",
            pid, va, level, pteval, pteval & _PAGE_PRESENT ? 1L : 0L,
            idx, pat_name(idx), pfn);
out:
    mmap_read_unlock(mm);
    mmput(mm);
    put_task_struct(t);
    return 0;
}
static void __exit pteinfo_exit(void) {}
module_init(pteinfo_init);
module_exit(pteinfo_exit);
