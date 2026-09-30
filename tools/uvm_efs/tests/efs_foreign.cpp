// efs_foreign.cpp — E-S1, the Bug 1624521 negative control
// (docs/design/V3_UVM_GUEST_FAULT_PLANE.md §5, V3_UVM_B3_IMPLEMENTATION.md §0.3).
//
// In EFS mode kayfabe's twin VA spaces are externally owned — registrable through the same stock
// UVM path every CUDA process's VA space is. This asks what a FOREIGN host process (not the VMM: its
// own RM client, its own /dev/nvidiactl fd, its own UVM file) gets when it names the VMM's client
// and twin VA-space handles in UVM_REGISTER_GPU_VASPACE:
//   NEG  the VMM's handles           — expected: refused (RM's dup applies the client's share policy)
//   POS  a fault-capable VA space of its OWN, same call — must succeed, or NEG proves nothing
// usage: efs_foreign <vmm hClient> <vmm hVaSpace>
// Prints one tagged line per check; exit 0 iff NEG was refused AND POS succeeded.
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include "rm_mini.h"

int main(int argc, char **argv)
{
    if (argc < 3) {
        fprintf(stderr, "usage: efs_foreign <vmm hClient> <vmm hVaSpace>\n");
        return 2;
    }
    NvHandle vmm_client = (NvHandle)strtoul(argv[1], NULL, 0);
    NvHandle vmm_vas = (NvHandle)strtoul(argv[2], NULL, 0);

    rmm_t r;
    int s = rmm_open(&r);
    if (s) {
        printf("E-S1 SETUP_FAIL rmm_open=%d\n", s);
        return 2;
    }
    printf("E-S1 FOREIGN pid=%d own_client=%#x vmm_client=%#x vmm_vas=%#x\n", getpid(), r.client, vmm_client,
           vmm_vas);

    // NEG — the VMM's handles, from this process's own ctl fd and UVM file.
    int fd = -1;
    NV_STATUS st = 0;
    if (uvm_open_init(UVM_INIT_FLAGS_MULTI_PROCESS_SHARING_MODE, &fd, &st) < 0 || st != NV_OK) {
        printf("E-S1 SETUP_FAIL uvm_init status=0x%x\n", st);
        return 2;
    }
    NV_STATUS g = uvm_register_gpu(fd, &r.uuid);
    NV_STATUS neg = uvm_register_gpu_vas(fd, &r.uuid, r.ctl, vmm_client, vmm_vas);
    printf("E-S1 NEG REGISTER_GPU=0x%x REGISTER_GPU_VASPACE(vmm client, vmm vas)=0x%x -> %s\n", g, neg,
           neg == NV_OK ? "ACCEPTED (the foreign process now co-owns the VMM's twin space)" : "refused");

    // POS — the same verb on a fault-capable VA space this process owns.
    NvHandle own = 0;
    int a = rmm_alloc_faulting_vas(&r, &own);
    int fd2 = -1;
    NV_STATUS st2 = 0;
    NV_STATUS pos = 0xffffffffu;
    if (a == 0 && uvm_open_init(UVM_INIT_FLAGS_MULTI_PROCESS_SHARING_MODE, &fd2, &st2) >= 0 && st2 == NV_OK) {
        NV_STATUS g2 = uvm_register_gpu(fd2, &r.uuid);
        pos = uvm_register_gpu_vas(fd2, &r.uuid, r.ctl, r.client, own);
        printf("E-S1 POS REGISTER_GPU=0x%x REGISTER_GPU_VASPACE(own client, own vas %#x)=0x%x -> %s\n", g2, own, pos,
               pos == NV_OK ? "accepted (the path works)" : "REFUSED (the control failed: NEG is inconclusive)");
    } else {
        printf("E-S1 POS SETUP_FAIL alloc=%d uvm_init=0x%x\n", a, st2);
    }
    int ok = neg != NV_OK && pos == NV_OK;
    printf("E-S1 RESULT %s\n", ok ? "PASS (foreign registration refused; control accepted)"
                                  : (neg == NV_OK ? "FAIL (foreign registration ACCEPTED)" : "INCONCLUSIVE (control failed)"));
    return ok ? 0 : 1;
}
