// Throwaway: for (pid,va,len), rewrite leaf PTEs to WB (clear PCD/PWT/PAT bits
// -> PAT index 0 = WB), flush TLB. Tests the "force WB after remap" fix.
#include <linux/module.h>
#include <linux/sched.h>
#include <linux/sched/task.h>
#include <linux/mm.h>
#include <linux/pid.h>
#include <linux/pgtable.h>
#include <asm/tlbflush.h>
#include <asm/pgtable_types.h>
#include <linux/smp.h>
#include <asm/special_insns.h>
MODULE_LICENSE("GPL");
static void flush_local(void *info){ write_cr3(__read_cr3()); } /* flush non-global TLB on this CPU */
static int pid=0; static unsigned long va=0, len=0;
module_param(pid,int,0); module_param(va,ulong,0); module_param(len,ulong,0);

static int __init f_init(void)
{
    struct task_struct *t; struct mm_struct *mm; struct vm_area_struct *vma;
    unsigned long a, end = va + (len?len:0x800000), changed=0;
    pgd_t *pgd; p4d_t *p4d; pud_t *pud; pmd_t *pmd; pte_t *pte;
    unsigned long v, nv;
    rcu_read_lock(); t=pid_task(find_vpid(pid),PIDTYPE_PID); if(t) get_task_struct(t); rcu_read_unlock();
    if(!t){pr_info("fixwb: no pid %d\n",pid);return 0;}
    mm=get_task_mm(t); if(!mm){put_task_struct(t);return 0;}
    mmap_read_lock(mm);
    vma=find_vma(mm,va);
    for(a=va; a<end; a+=PAGE_SIZE){
        pgd=pgd_offset(mm,a);
        if(pgd_none(*pgd))continue; p4d=p4d_offset(pgd,a); if(p4d_none(*p4d))continue;
        pud=pud_offset(p4d,a); if(pud_none(*pud))continue; if(pud_large(*pud))continue;
        pmd=pmd_offset(pud,a); if(pmd_none(*pmd))continue; if(pmd_large(*pmd))continue;
        pte=pte_offset_kernel(pmd,a);
        v=pte_val(*pte);
        if(!(v&_PAGE_PRESENT))continue;
        nv = v & ~(_PAGE_PCD|_PAGE_PWT|_PAGE_PAT); /* -> PAT idx 0 = WB */
        if(nv!=v){ set_pte(pte, __pte(nv)); changed++; }
    }
    (void)vma;
    mmap_read_unlock(mm);
    on_each_cpu(flush_local, NULL, 1);
    pr_info("fixwb: pid=%d va=0x%lx len=0x%lx changed=%lu PTEs -> WB\n",pid,va,end-va,changed);
    mmput(mm); put_task_struct(t);
    return 0;
}
static void __exit f_exit(void){}
module_init(f_init); module_exit(f_exit);
