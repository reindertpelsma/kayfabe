/* E3 (EXPERIMENT, claude/sparse-hw-20261026): H6 reproducer in one tile.
 *
 * Vulkan sparse bind -> unbind (vkQueueBindSparse) -> a COPY-ENGINE write into the unbound tile
 * (vkCmdFillBuffer / vkCmdCopyBuffer on the TRANSFER-ONLY queue family) -> read the tile back.
 *
 *   gcc -O2 -o sparse_h6 sparse_h6.c -lvulkan
 *   ./sparse_h6 [iterations] [mode]      mode: seq (fence between steps) | chain (semaphore: bind -> write)
 *
 * Expected on real hardware (H6 hardware half): no device loss, no Xid, a read of the unbound tile
 * returns zeros (residencyNonResidentStrict), and the write is absorbed (re-binding the original
 * memory shows its OLD content, not the write).
 *
 * Output: one `H6_*` line per fact. Exit 0 = every iteration behaved as expected, 3 = a read was
 * not zero / a write landed in memory, 4 = VK_ERROR_DEVICE_LOST or another failure after the unbind.
 */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <vulkan/vulkan.h>

#define TILE 0x10000u
#define BUFSZ (16u * TILE)
#define CK(x)                                                                                  \
    do {                                                                                       \
        VkResult r_ = (x);                                                                     \
        if (r_ != VK_SUCCESS) {                                                                \
            printf("H6_VKERR %s:%d %s = %d%s\n", __FILE__, __LINE__, #x, (int)r_,              \
                   r_ == VK_ERROR_DEVICE_LOST ? " (DEVICE_LOST)" : "");                        \
            fflush(stdout);                                                                    \
            exit(r_ == VK_ERROR_DEVICE_LOST ? 4 : 5);                                          \
        }                                                                                      \
    } while (0)

static VkDevice dev;
static VkPhysicalDevice pd;
static VkPhysicalDeviceMemoryProperties mp;

static uint32_t memtype(uint32_t bits, VkMemoryPropertyFlags want) {
    for (uint32_t i = 0; i < mp.memoryTypeCount; i++)
        if ((bits & (1u << i)) && (mp.memoryTypes[i].propertyFlags & want) == want) return i;
    printf("H6_VKERR no memory type bits=%x want=%x\n", bits, want);
    exit(5);
}

static VkBuffer mkbuf(VkDeviceSize sz, VkBufferUsageFlags use, VkBufferCreateFlags fl, uint32_t nq,
                      const uint32_t *qs) {
    VkBufferCreateInfo bi = {VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO};
    bi.flags = fl;
    bi.size = sz;
    bi.usage = use;
    bi.sharingMode = nq > 1 ? VK_SHARING_MODE_CONCURRENT : VK_SHARING_MODE_EXCLUSIVE;
    bi.queueFamilyIndexCount = nq > 1 ? nq : 0;
    bi.pQueueFamilyIndices = qs;
    VkBuffer b;
    CK(vkCreateBuffer(dev, &bi, NULL, &b));
    return b;
}

static VkDeviceMemory mkmem(VkDeviceSize sz, uint32_t bits, VkMemoryPropertyFlags f) {
    VkMemoryAllocateInfo ai = {VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO};
    ai.allocationSize = sz;
    ai.memoryTypeIndex = memtype(bits, f);
    VkDeviceMemory m;
    CK(vkAllocateMemory(dev, &ai, NULL, &m));
    return m;
}

int main(int argc, char **argv) {
    int iters = argc > 1 ? atoi(argv[1]) : 50;
    int chain = argc > 2 && !strcmp(argv[2], "chain");

    VkApplicationInfo app = {VK_STRUCTURE_TYPE_APPLICATION_INFO};
    app.apiVersion = VK_API_VERSION_1_1;
    VkInstanceCreateInfo ici = {VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO};
    ici.pApplicationInfo = &app;
    VkInstance inst;
    CK(vkCreateInstance(&ici, NULL, &inst));
    uint32_t n = 0;
    CK(vkEnumeratePhysicalDevices(inst, &n, NULL));
    VkPhysicalDevice pds[8];
    if (n > 8) n = 8;
    CK(vkEnumeratePhysicalDevices(inst, &n, pds));
    pd = VK_NULL_HANDLE;
    for (uint32_t i = 0; i < n; i++) {
        VkPhysicalDeviceProperties p;
        vkGetPhysicalDeviceProperties(pds[i], &p);
        if (p.vendorID == 0x10DE) {
            pd = pds[i];
            printf("H6_DEVICE %s driver=%u.%u.%u\n", p.deviceName, VK_VERSION_MAJOR(p.driverVersion),
                   VK_VERSION_MINOR(p.driverVersion), VK_VERSION_PATCH(p.driverVersion));
            break;
        }
    }
    if (!pd) { printf("H6_VKERR no NVIDIA device\n"); return 5; }
    vkGetPhysicalDeviceMemoryProperties(pd, &mp);
    VkPhysicalDeviceFeatures feats;
    vkGetPhysicalDeviceFeatures(pd, &feats);
    VkPhysicalDeviceProperties props;
    vkGetPhysicalDeviceProperties(pd, &props);
    printf("H6_FEATURES sparseBinding=%d sparseResidencyBuffer=%d residencyNonResidentStrict=%d\n",
           feats.sparseBinding, feats.sparseResidencyBuffer,
           props.sparseProperties.residencyNonResidentStrict);
    if (!feats.sparseBinding || !feats.sparseResidencyBuffer) { printf("H6_VKERR sparse unsupported\n"); return 5; }

    uint32_t nqf = 0;
    vkGetPhysicalDeviceQueueFamilyProperties(pd, &nqf, NULL);
    VkQueueFamilyProperties qf[16];
    if (nqf > 16) nqf = 16;
    vkGetPhysicalDeviceQueueFamilyProperties(pd, &nqf, qf);
    int gfx = -1, xfer = -1;
    for (uint32_t i = 0; i < nqf; i++) {
        VkQueueFlags f = qf[i].queueFlags;
        if (gfx < 0 && (f & VK_QUEUE_GRAPHICS_BIT) && (f & VK_QUEUE_SPARSE_BINDING_BIT)) gfx = (int)i;
        if (xfer < 0 && (f & VK_QUEUE_TRANSFER_BIT) && !(f & (VK_QUEUE_GRAPHICS_BIT | VK_QUEUE_COMPUTE_BIT)) &&
            (f & VK_QUEUE_SPARSE_BINDING_BIT)) xfer = (int)i;
    }
    printf("H6_QUEUES graphics_family=%d transfer_only_family=%d\n", gfx, xfer);
    if (gfx < 0 || xfer < 0) { printf("H6_VKERR no queue family\n"); return 5; }

    float prio = 1.0f;
    VkDeviceQueueCreateInfo qci[2] = {{VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO},
                                      {VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO}};
    qci[0].queueFamilyIndex = (uint32_t)gfx; qci[0].queueCount = 1; qci[0].pQueuePriorities = &prio;
    qci[1].queueFamilyIndex = (uint32_t)xfer; qci[1].queueCount = 1; qci[1].pQueuePriorities = &prio;
    VkPhysicalDeviceFeatures en = {0};
    en.sparseBinding = VK_TRUE;
    en.sparseResidencyBuffer = VK_TRUE;
    VkDeviceCreateInfo dci = {VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO};
    dci.queueCreateInfoCount = 2; dci.pQueueCreateInfos = qci; dci.pEnabledFeatures = &en;
    CK(vkCreateDevice(pd, &dci, NULL, &dev));
    VkQueue qg, qx;
    vkGetDeviceQueue(dev, (uint32_t)gfx, 0, &qg);
    vkGetDeviceQueue(dev, (uint32_t)xfer, 0, &qx);

    uint32_t qs[2] = {(uint32_t)gfx, (uint32_t)xfer};
    VkBuffer sb = mkbuf(BUFSZ, VK_BUFFER_USAGE_TRANSFER_SRC_BIT | VK_BUFFER_USAGE_TRANSFER_DST_BIT,
                        VK_BUFFER_CREATE_SPARSE_BINDING_BIT | VK_BUFFER_CREATE_SPARSE_RESIDENCY_BIT, 2, qs);
    VkMemoryRequirements sreq;
    vkGetBufferMemoryRequirements(dev, sb, &sreq);
    printf("H6_SPARSE_BUFFER size=%u align=%llu memtypebits=%x\n", BUFSZ, (unsigned long long)sreq.alignment,
           sreq.memoryTypeBits);
    if (sreq.alignment > TILE) { printf("H6_VKERR alignment > tile\n"); return 5; }
    VkDeviceMemory tilemem = mkmem(TILE, sreq.memoryTypeBits, VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT);

    /* host staging: in (pattern) and out (poisoned), both one tile */
    VkBuffer sin = mkbuf(TILE, VK_BUFFER_USAGE_TRANSFER_SRC_BIT | VK_BUFFER_USAGE_TRANSFER_DST_BIT, 0, 1, qs);
    VkBuffer sout = mkbuf(TILE, VK_BUFFER_USAGE_TRANSFER_SRC_BIT | VK_BUFFER_USAGE_TRANSFER_DST_BIT, 0, 1, qs);
    VkMemoryRequirements r1, r2;
    vkGetBufferMemoryRequirements(dev, sin, &r1);
    vkGetBufferMemoryRequirements(dev, sout, &r2);
    VkMemoryPropertyFlags hv = VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT;
    VkDeviceMemory min = mkmem(r1.size, r1.memoryTypeBits, hv), mout = mkmem(r2.size, r2.memoryTypeBits, hv);
    CK(vkBindBufferMemory(dev, sin, min, 0));
    CK(vkBindBufferMemory(dev, sout, mout, 0));
    uint32_t *pin, *pout;
    CK(vkMapMemory(dev, min, 0, TILE, 0, (void **)&pin));
    CK(vkMapMemory(dev, mout, 0, TILE, 0, (void **)&pout));

    VkCommandPoolCreateInfo cpi = {VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO};
    cpi.flags = VK_COMMAND_POOL_CREATE_RESET_COMMAND_BUFFER_BIT;
    cpi.queueFamilyIndex = (uint32_t)xfer;
    VkCommandPool pool;
    CK(vkCreateCommandPool(dev, &cpi, NULL, &pool));
    VkCommandBufferAllocateInfo cai = {VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO};
    cai.commandPool = pool; cai.level = VK_COMMAND_BUFFER_LEVEL_PRIMARY; cai.commandBufferCount = 1;
    VkCommandBuffer cb;
    CK(vkAllocateCommandBuffers(dev, &cai, &cb));
    VkFenceCreateInfo fci = {VK_STRUCTURE_TYPE_FENCE_CREATE_INFO};
    VkFence fence;
    CK(vkCreateFence(dev, &fci, NULL, &fence));
    VkSemaphoreCreateInfo sci = {VK_STRUCTURE_TYPE_SEMAPHORE_CREATE_INFO};
    VkSemaphore sem;
    CK(vkCreateSemaphore(dev, &sci, NULL, &sem));

    VkSparseMemoryBind sbind = {0};
    VkSparseBufferMemoryBindInfo sbi = {sb, 1, &sbind};
    VkBindSparseInfo bsi = {VK_STRUCTURE_TYPE_BIND_SPARSE_INFO};
    bsi.bufferBindCount = 1; bsi.pBufferBinds = &sbi;

    int bad_read = 0, landed = 0, ok_iter = 0, ctrl_bad = 0;
    for (int it = 0; it < iters; it++) {
        uint32_t tile = (uint32_t)(it % 15 + 1) * TILE; /* tiles 1..15 */
        uint32_t fillv = 0x11110000u | (uint32_t)it, wrv = 0xAAAA0000u | (uint32_t)it;
        /* 1. BIND tile -> tilemem and wait */
        sbind.resourceOffset = tile; sbind.size = TILE; sbind.memory = tilemem; sbind.memoryOffset = 0;
        bsi.signalSemaphoreCount = 0; bsi.pSignalSemaphores = NULL;
        CK(vkQueueBindSparse(qg, 1, &bsi, fence));
        CK(vkWaitForFences(dev, 1, &fence, VK_TRUE, 5000000000ull)); CK(vkResetFences(dev, 1, &fence));
        /* 2. positive control: fill via CE, copy to staging, check */
        VkCommandBufferBeginInfo bi = {VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO};
        VkSubmitInfo si = {VK_STRUCTURE_TYPE_SUBMIT_INFO};
        si.commandBufferCount = 1; si.pCommandBuffers = &cb;
        memset(pout, 0xEE, TILE);
        CK(vkResetCommandBuffer(cb, 0)); CK(vkBeginCommandBuffer(cb, &bi));
        vkCmdFillBuffer(cb, sb, tile, TILE, fillv);
        VkMemoryBarrier mb = {VK_STRUCTURE_TYPE_MEMORY_BARRIER, NULL, VK_ACCESS_TRANSFER_WRITE_BIT,
                              VK_ACCESS_TRANSFER_READ_BIT};
        vkCmdPipelineBarrier(cb, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_PIPELINE_STAGE_TRANSFER_BIT, 0, 1, &mb, 0, NULL, 0, NULL);
        VkBufferCopy cp = {tile, 0, TILE};
        vkCmdCopyBuffer(cb, sb, sout, 1, &cp);
        CK(vkEndCommandBuffer(cb));
        CK(vkQueueSubmit(qx, 1, &si, fence));
        CK(vkWaitForFences(dev, 1, &fence, VK_TRUE, 5000000000ull)); CK(vkResetFences(dev, 1, &fence));
        for (uint32_t i = 0; i < TILE / 4; i++) if (pout[i] != fillv) { ctrl_bad++; break; }
        /* 3. UNBIND (memory = NULL); chain: signal a semaphore the write submit waits on */
        sbind.memory = VK_NULL_HANDLE;
        VkPipelineStageFlags wst = VK_PIPELINE_STAGE_TRANSFER_BIT;
        if (chain) { bsi.signalSemaphoreCount = 1; bsi.pSignalSemaphores = &sem; }
        CK(vkQueueBindSparse(qg, 1, &bsi, chain ? VK_NULL_HANDLE : fence));
        if (!chain) { CK(vkWaitForFences(dev, 1, &fence, VK_TRUE, 5000000000ull)); CK(vkResetFences(dev, 1, &fence)); }
        /* 4. CE WRITE into the unbound tile: a copy from the pattern staging + a fill of its first 4 KiB */
        for (uint32_t i = 0; i < TILE / 4; i++) pin[i] = wrv;
        CK(vkResetCommandBuffer(cb, 0)); CK(vkBeginCommandBuffer(cb, &bi));
        VkBufferCopy wc = {0, tile, TILE};
        vkCmdCopyBuffer(cb, sin, sb, 1, &wc);
        vkCmdPipelineBarrier(cb, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_PIPELINE_STAGE_TRANSFER_BIT, 0, 1, &mb, 0, NULL, 0, NULL);
        vkCmdFillBuffer(cb, sb, tile, 0x1000, wrv ^ 0x0F0F0F0Fu);
        vkCmdPipelineBarrier(cb, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_PIPELINE_STAGE_TRANSFER_BIT, 0, 1, &mb, 0, NULL, 0, NULL);
        /* 5. read the unbound tile back, same submission */
        vkCmdCopyBuffer(cb, sb, sout, 1, &cp);
        CK(vkEndCommandBuffer(cb));
        memset(pout, 0xEE, TILE);
        si.waitSemaphoreCount = chain; si.pWaitSemaphores = chain ? &sem : NULL; si.pWaitDstStageMask = &wst;
        CK(vkQueueSubmit(qx, 1, &si, fence));
        CK(vkWaitForFences(dev, 1, &fence, VK_TRUE, 5000000000ull)); CK(vkResetFences(dev, 1, &fence));
        uint32_t zero = 0, poison = 0, other = 0, first = 0;
        for (uint32_t i = 0; i < TILE / 4; i++) {
            if (pout[i] == 0) zero++;
            else if (pout[i] == 0xEEEEEEEEu) poison++;
            else { if (!other) first = pout[i]; other++; }
        }
        if (other || poison) bad_read++;
        /* 6. re-bind the ORIGINAL memory and read it: did the sparse write land in real memory? */
        sbind.memory = tilemem;
        bsi.signalSemaphoreCount = 0; bsi.pSignalSemaphores = NULL;
        CK(vkQueueBindSparse(qg, 1, &bsi, fence));
        CK(vkWaitForFences(dev, 1, &fence, VK_TRUE, 5000000000ull)); CK(vkResetFences(dev, 1, &fence));
        memset(pout, 0xEE, TILE);
        CK(vkResetCommandBuffer(cb, 0)); CK(vkBeginCommandBuffer(cb, &bi));
        vkCmdCopyBuffer(cb, sb, sout, 1, &cp);
        CK(vkEndCommandBuffer(cb));
        si.waitSemaphoreCount = 0;
        CK(vkQueueSubmit(qx, 1, &si, fence));
        CK(vkWaitForFences(dev, 1, &fence, VK_TRUE, 5000000000ull)); CK(vkResetFences(dev, 1, &fence));
        uint32_t keep = 0, wr = 0, oth2 = 0;
        for (uint32_t i = 0; i < TILE / 4; i++) {
            if (pout[i] == fillv) keep++;
            else if (pout[i] == wrv || (i < 0x400 && pout[i] == (wrv ^ 0x0F0F0F0Fu))) wr++;
            else oth2++;
        }
        if (wr) landed++;
        if (!other && !poison && !wr && !oth2) ok_iter++;
        if (it < 3 || other || poison || wr || oth2 || it % 100 == 0)
            printf("H6_ITER %d tile=%#x read_unbound zero=%u poison=%u other=%u first_other=%#x | rebound old=%u write_landed=%u other=%u\n",
                   it, tile, zero, poison, other, first, keep, wr, oth2);
        /* unbind again for the next tile */
        sbind.memory = VK_NULL_HANDLE;
        CK(vkQueueBindSparse(qg, 1, &bsi, fence));
        CK(vkWaitForFences(dev, 1, &fence, VK_TRUE, 5000000000ull)); CK(vkResetFences(dev, 1, &fence));
    }
    printf("H6_SUMMARY mode=%s iters=%d ok=%d control_bad=%d unbound_read_not_zero=%d writes_landed_in_memory=%d\n",
           chain ? "chain" : "seq", iters, ok_iter, ctrl_bad, bad_read, landed);
    int rc = (ctrl_bad ? 5 : (bad_read || landed) ? 3 : 0);
    printf("H6_VERDICT %s\n", rc == 0 ? "ABSORBED_ZERO_NO_FAULT" : rc == 5 ? "INVALID_CONTROL" : "NONZERO_OR_LANDED");
    return rc;
}
