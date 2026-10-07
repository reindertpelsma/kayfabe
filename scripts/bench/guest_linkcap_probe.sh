#!/usr/bin/env bash
# POST_CAPTURE_HOOK: read the ONE register `cuInit` now demonstrably dies on.
#
# ★★★ The chain, each link from ogkm-580 and each measured, not inferred:
#   `UVM_REGISTER_GPU.rmStatus = 0x40 NV_ERR_INVALID_STATE`   (boot us1445, measured)
#     <- `nvGpuOpsGetGpuInfo` `nv_gpu_ops.c:7220` returns the first failure
#     <- `getPCIELinkRateMBps` `nv_gpu_ops.c:2118` — and BOTH its prints are in that dmesg
#     <- `calculatePCIELinkRateMBps` `nv_gpu_ops.c:2078` default arm, "Unknown PCIe speed":
#        `pciLinkMaxSpeed` (bits 3:0) was not one of the six legal encodings
#        (`ctrl2080bus.h:357-363`, values 1..6)
#     <- `NV2080_CTRL_BUS_INFO_INDEX_PCIE_GPU_LINK_CAPS` (index **0x03**), which
#        ⊘ is **NOT RPC-forwarded** on a GSP client — `getBusInfos`'s `bSendRpc` switch
#        (`kern_bus_ctrl.c:296-330`) lists thirteen indices and 0x03 is not among them —
#        so it is answered by the guest's own kernel at `kernel_bif.c:1072`
#     <- `kbifGetGpuLinkCapabilities` `kernel_bif.c:879-903`
#     <- `GPU_BUS_CFG_RD32(pGpu, NV_XVE_LINK_CAPABILITIES)` = **BAR0 + 0x88084**
#        (`dev_nv_xve.h:104` = 0x84; `dev_nv_pcfg_xve_regmap.h:27` NV_PCFG window = 0x88000;
#        `kern_gpu_gm107.c:186` reads `DEVICE_BASE(NV_PCFG) + index` out of the REGISTER
#        aperture, ⊘ not out of PCI config space).
#
# ⇒ This hook reads that word two ways, because they are two different planes and only one of
# them is on RM's path:
#   (a) BAR0 + 0x88084 through `resource0` — ★ what RM actually reads;
#   (b) the real PCIe capability in config space through `setpci` — what QEMU presents.
# ⚠ They can disagree, and a fix applied to (b) alone would change nothing.
set -uo pipefail
REPO=/workspace/bench/kayfabe
G="$REPO/scripts/bench/gssh_nv"

$G 'cat > /tmp/linkcap.c' <<'EOF'
// Read BAR0 + 0x88084 (NV_PCFG + NV_XVE_LINK_CAPABILITIES) the way RM reads it.
#define _GNU_SOURCE
#include <stdio.h>
#include <stdint.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/mman.h>
int main(int argc, char **argv)
{
    const char *path = argc > 1 ? argv[1]
        : "/sys/bus/pci/devices/0000:00:03.0/resource0";
    int fd = open(path, O_RDONLY | O_SYNC);
    if (fd < 0) { perror("open resource0"); return 1; }
    // Map one page containing 0x88084. mmap offsets must be page aligned.
    off_t page = 0x88000;
    volatile unsigned char *m = mmap(NULL, 0x1000, PROT_READ, MAP_SHARED, fd, page);
    if (m == MAP_FAILED) { perror("mmap"); return 1; }
    struct { unsigned off; const char *name; } regs[] = {
        { 0x084, "NV_XVE_LINK_CAPABILITIES   (RM reads THIS for PCIE_GPU_LINK_CAPS)" },
        { 0x088, "NV_XVE_LINK_CONTROL_STATUS" },
        { 0x000, "NV_XVE_ID (vendor/device)" },
        { 0x078, "NV_XVE_DEVICE_CONTROL_STATUS" },
    };
    for (unsigned i = 0; i < sizeof regs / sizeof regs[0]; i++) {
        uint32_t v = *(volatile uint32_t *)(m + regs[i].off);
        printf("BAR0+0x%05x = 0x%08x   %s\n", (unsigned)page + regs[i].off, v, regs[i].name);
        if (regs[i].off == 0x084) {
            unsigned speed = v & 0xf, width = (v >> 4) & 0x3f;
            printf("    MAX_SPEED(3:0) = %u  %s\n", speed,
                   (speed >= 1 && speed <= 6)
                       ? "LEGAL — calculatePCIELinkRateMBps accepts this"
                       : "★ ILLEGAL — this is the 'Unknown PCIe speed' that returns 0x40");
            printf("    MAX_WIDTH(9:4) = %u lanes\n", width);
        }
    }
    return 0;
}
EOF

$G 'gcc -O0 -o /tmp/linkcap /tmp/linkcap.c 2>&1; echo GCC_RC=$?'

echo "=== ★★★ (a) BAR0 + 0x88084 — the word RM decodes ==="
$G 'sudo /tmp/linkcap 2>&1'

echo "=== (b) the PCIe capability QEMU presents in CONFIG space (a DIFFERENT plane) ==="
$G 'sudo lspci -vv -s 00:03.0 2>&1 | grep -A3 "LnkCap" | head -20; echo LSPCI_RC=$?'
$G 'sudo setpci -s 00:03.0 CAP_EXP+0c.l 2>&1; echo SETPCI_RC=$?'

echo "=== for the record: which device is which ==="
$G 'lspci -nn | grep -i nvidia; ls -la /sys/bus/pci/devices/0000:00:03.0/resource0'
