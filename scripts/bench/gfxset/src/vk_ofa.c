// vk_ofa — the optical-flow engine (OFA) through VK_NV_optical_flow, end to end, with a known answer.
//
// v3-gfxset: `[measured gs1]` a kf3 guest did not advertise VK_NV_optical_flow (the OFA engine was never
// stated to the guest RM). Advertising it is half the job; this program is the other half — it makes the
// OFA engine DO work and checks what it produced:
//   frame A: a deterministic textured pattern (R8, W x H); frame B: A shifted by (DX, DY) pixels.
//   One vkCmdOpticalFlowExecuteNV on the optical-flow queue family (upload, execute and readback all on
//   that family), 4x4 output grid, S10.5 flow vectors.
//   Self-check: the MEDIAN flow vector over the interior equals the known shift (to 1/4 px), and at
//   least 80 % of interior vectors are within 1 px of it.  Prints OFA_MEDIAN_X/Y, OFA_GOOD_FRAC,
//   OFA_HASH (FNV-1a 64 of the whole flow field) and OFA_OK / OFA_FAIL.
// ⊘ The hash is graded against the SAME program on bare metal (same GPU, same driver) by the suite; the
//   median/fraction check is the floor that holds without a host.
//   build: gcc -O2 vk_ofa.c -o vk_ofa -lvulkan -lm
#include <vulkan/vulkan.h>
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define W 512
#define H 256
#define DX 5
#define DY 3
#define GRID 4
#define CK(x) do { VkResult r_ = (x); if (r_ != VK_SUCCESS) { printf("OFA_FAIL %s -> %d (line %d)\n", #x, r_, __LINE__); exit(3); } } while (0)

static VkPhysicalDevice pd; static VkDevice dev; static VkPhysicalDeviceMemoryProperties mp;
static uint32_t memtype(uint32_t bits, VkMemoryPropertyFlags want) {
    for (uint32_t i = 0; i < mp.memoryTypeCount; i++)
        if ((bits & (1u << i)) && (mp.memoryTypes[i].propertyFlags & want) == want) return i;
    printf("OFA_FAIL no memory type bits=0x%x want=0x%x\n", bits, want); exit(3);
}
static uint64_t fnv(const void *p, size_t n) {
    const uint8_t *b = p; uint64_t h = 1469598103934665603ull;
    for (size_t i = 0; i < n; i++) { h ^= b[i]; h *= 1099511628211ull; }
    return h;
}
// a smooth-but-textured pattern: every 8x8 block distinct, so block matching has one answer
static uint8_t pattern(int x, int y) {
    uint32_t h = (uint32_t)(x / 3) * 2654435761u ^ (uint32_t)(y / 3) * 40503u;
    h ^= h >> 13; h *= 0x5bd1e995u; h ^= h >> 15;
    return (uint8_t)(64 + (h & 127) + (int)(40.0 * sin(x * 0.07) * cos(y * 0.05)));
}
static int cmp16(const void *a, const void *b) { return *(const int16_t *)a - *(const int16_t *)b; }

typedef struct { VkImage img; VkDeviceMemory mem; VkImageView view; } Img;
static Img make_image(VkFormat f, uint32_t w, uint32_t h, VkImageUsageFlags usage, VkOpticalFlowUsageFlagsNV ofu, uint32_t qf) {
    Img r;
    VkOpticalFlowImageFormatInfoNV ofi = { VK_STRUCTURE_TYPE_OPTICAL_FLOW_IMAGE_FORMAT_INFO_NV, NULL, ofu };
    VkImageCreateInfo ici = { VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO, &ofi, 0, VK_IMAGE_TYPE_2D, f, { w, h, 1 }, 1, 1,
        VK_SAMPLE_COUNT_1_BIT, VK_IMAGE_TILING_OPTIMAL, usage, VK_SHARING_MODE_EXCLUSIVE, 1, &qf, VK_IMAGE_LAYOUT_UNDEFINED };
    CK(vkCreateImage(dev, &ici, NULL, &r.img));
    VkMemoryRequirements mr; vkGetImageMemoryRequirements(dev, r.img, &mr);
    VkMemoryAllocateInfo mai = { VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO, NULL, mr.size, memtype(mr.memoryTypeBits, VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT) };
    CK(vkAllocateMemory(dev, &mai, NULL, &r.mem)); CK(vkBindImageMemory(dev, r.img, r.mem, 0));
    VkImageViewCreateInfo vci = { VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO, NULL, 0, r.img, VK_IMAGE_VIEW_TYPE_2D, f,
        { 0, 0, 0, 0 }, { VK_IMAGE_ASPECT_COLOR_BIT, 0, 1, 0, 1 } };
    CK(vkCreateImageView(dev, &vci, NULL, &r.view));
    return r;
}
typedef struct { VkBuffer buf; VkDeviceMemory mem; void *map; } Buf;
static Buf make_buffer(VkDeviceSize n, VkBufferUsageFlags u) {
    Buf b;
    VkBufferCreateInfo bci = { VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO, NULL, 0, n, u, VK_SHARING_MODE_EXCLUSIVE, 0, NULL };
    CK(vkCreateBuffer(dev, &bci, NULL, &b.buf));
    VkMemoryRequirements mr; vkGetBufferMemoryRequirements(dev, b.buf, &mr);
    VkMemoryAllocateInfo mai = { VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO, NULL, mr.size,
        memtype(mr.memoryTypeBits, VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT) };
    CK(vkAllocateMemory(dev, &mai, NULL, &b.mem)); CK(vkBindBufferMemory(dev, b.buf, b.mem, 0));
    CK(vkMapMemory(dev, b.mem, 0, n, 0, &b.map));
    return b;
}
static void barrier(VkCommandBuffer cb, VkImage img, VkImageLayout from, VkImageLayout to) {
    VkImageMemoryBarrier b = { VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER, NULL, VK_ACCESS_MEMORY_WRITE_BIT,
        VK_ACCESS_MEMORY_READ_BIT | VK_ACCESS_MEMORY_WRITE_BIT, from, to, VK_QUEUE_FAMILY_IGNORED, VK_QUEUE_FAMILY_IGNORED, img,
        { VK_IMAGE_ASPECT_COLOR_BIT, 0, 1, 0, 1 } };
    vkCmdPipelineBarrier(cb, VK_PIPELINE_STAGE_ALL_COMMANDS_BIT, VK_PIPELINE_STAGE_ALL_COMMANDS_BIT, 0, 0, NULL, 0, NULL, 1, &b);
}

int main(void) {
    printf("OFA_START %dx%d shift=(%d,%d) grid=%d\n", W, H, DX, DY, GRID);
    VkApplicationInfo ai = { VK_STRUCTURE_TYPE_APPLICATION_INFO, NULL, "vk_ofa", 1, NULL, 0, VK_API_VERSION_1_3 };
    VkInstanceCreateInfo ii = { VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO, NULL, 0, &ai, 0, NULL, 0, NULL };
    VkInstance inst; CK(vkCreateInstance(&ii, NULL, &inst));
    uint32_t n = 0; CK(vkEnumeratePhysicalDevices(inst, &n, NULL));
    VkPhysicalDevice pds[8]; if (n > 8) n = 8; CK(vkEnumeratePhysicalDevices(inst, &n, pds));
    pd = VK_NULL_HANDLE;
    for (uint32_t i = 0; i < n && !pd; i++) { VkPhysicalDeviceProperties p; vkGetPhysicalDeviceProperties(pds[i], &p); if (p.vendorID == 0x10de) { pd = pds[i]; printf("OFA_DEVICE %s\n", p.deviceName); } }
    if (!pd) { printf("OFA_FAIL no NVIDIA physical device\n"); return 3; }
    vkGetPhysicalDeviceMemoryProperties(pd, &mp);
    // the extension, the feature, and a queue family with VK_QUEUE_OPTICAL_FLOW_BIT_NV
    uint32_t ne = 0; vkEnumerateDeviceExtensionProperties(pd, NULL, &ne, NULL);
    VkExtensionProperties *ep = calloc(ne, sizeof *ep); vkEnumerateDeviceExtensionProperties(pd, NULL, &ne, ep);
    int has = 0; for (uint32_t i = 0; i < ne; i++) has |= !strcmp(ep[i].extensionName, VK_NV_OPTICAL_FLOW_EXTENSION_NAME);
    if (!has) { printf("OFA_FAIL VK_NV_optical_flow not advertised\n"); return 2; }
    VkPhysicalDeviceOpticalFlowFeaturesNV off = { VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_OPTICAL_FLOW_FEATURES_NV };
    VkPhysicalDeviceFeatures2 f2 = { VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FEATURES_2, &off };
    vkGetPhysicalDeviceFeatures2(pd, &f2);
    if (!off.opticalFlow) { printf("OFA_FAIL opticalFlow feature is FALSE\n"); return 2; }
    uint32_t nq = 0; vkGetPhysicalDeviceQueueFamilyProperties(pd, &nq, NULL);
    VkQueueFamilyProperties qp[16]; if (nq > 16) nq = 16; vkGetPhysicalDeviceQueueFamilyProperties(pd, &nq, qp);
    uint32_t qf = UINT32_MAX; for (uint32_t i = 0; i < nq; i++) if (qp[i].queueFlags & VK_QUEUE_OPTICAL_FLOW_BIT_NV) { qf = i; break; }
    if (qf == UINT32_MAX) { printf("OFA_FAIL no optical-flow queue family\n"); return 2; }
    printf("OFA_QUEUE_FAMILY %u flags=0x%x\n", qf, qp[qf].queueFlags);
    float prio = 1.0f;
    VkDeviceQueueCreateInfo qci = { VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO, NULL, 0, qf, 1, &prio };
    VkPhysicalDeviceVulkan13Features v13 = { VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_3_FEATURES, NULL };
    v13.synchronization2 = VK_TRUE;
    VkPhysicalDeviceOpticalFlowFeaturesNV offe = { VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_OPTICAL_FLOW_FEATURES_NV, &v13, VK_TRUE };
    const char *exts[] = { VK_NV_OPTICAL_FLOW_EXTENSION_NAME };
    VkDeviceCreateInfo dci = { VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO, &offe, 0, 1, &qci, 0, NULL, 1, exts, NULL };
    CK(vkCreateDevice(pd, &dci, NULL, &dev));
    VkQueue q; vkGetDeviceQueue(dev, qf, 0, &q);
    PFN_vkCreateOpticalFlowSessionNV pCreate = (void *)vkGetDeviceProcAddr(dev, "vkCreateOpticalFlowSessionNV");
    PFN_vkBindOpticalFlowSessionImageNV pBind = (void *)vkGetDeviceProcAddr(dev, "vkBindOpticalFlowSessionImageNV");
    PFN_vkCmdOpticalFlowExecuteNV pExec = (void *)vkGetDeviceProcAddr(dev, "vkCmdOpticalFlowExecuteNV");
    PFN_vkDestroyOpticalFlowSessionNV pDestroy = (void *)vkGetDeviceProcAddr(dev, "vkDestroyOpticalFlowSessionNV");
    PFN_vkGetPhysicalDeviceOpticalFlowImageFormatsNV pFmts = (void *)vkGetInstanceProcAddr(inst, "vkGetPhysicalDeviceOpticalFlowImageFormatsNV");
    if (!pCreate || !pBind || !pExec || !pDestroy || !pFmts) { printf("OFA_FAIL entry points missing\n"); return 3; }
    // the formats the engine takes: R8_UNORM input must be listed; the flow format is R16G16_S10_5
    VkOpticalFlowImageFormatInfoNV fi = { VK_STRUCTURE_TYPE_OPTICAL_FLOW_IMAGE_FORMAT_INFO_NV, NULL, VK_OPTICAL_FLOW_USAGE_INPUT_BIT_NV };
    uint32_t nf = 0; CK(pFmts(pd, &fi, &nf, NULL));
    VkOpticalFlowImageFormatPropertiesNV fp[16]; if (nf > 16) nf = 16;
    for (uint32_t i = 0; i < nf; i++) fp[i] = (VkOpticalFlowImageFormatPropertiesNV){ VK_STRUCTURE_TYPE_OPTICAL_FLOW_IMAGE_FORMAT_PROPERTIES_NV };
    CK(pFmts(pd, &fi, &nf, fp));
    int r8 = 0; for (uint32_t i = 0; i < nf; i++) r8 |= fp[i].format == VK_FORMAT_R8_UNORM;
    printf("OFA_INPUT_FORMATS %u r8=%d\n", nf, r8);
    if (!r8) { printf("OFA_FAIL R8_UNORM is not an optical-flow input format\n"); return 3; }
    const uint32_t FW = W / GRID, FH = H / GRID;
    Img a = make_image(VK_FORMAT_R8_UNORM, W, H, VK_IMAGE_USAGE_TRANSFER_DST_BIT, VK_OPTICAL_FLOW_USAGE_INPUT_BIT_NV, qf);
    Img b = make_image(VK_FORMAT_R8_UNORM, W, H, VK_IMAGE_USAGE_TRANSFER_DST_BIT, VK_OPTICAL_FLOW_USAGE_INPUT_BIT_NV, qf);
    Img fl = make_image(VK_FORMAT_R16G16_S10_5_NV, FW, FH, VK_IMAGE_USAGE_TRANSFER_SRC_BIT, VK_OPTICAL_FLOW_USAGE_OUTPUT_BIT_NV, qf);
    Buf up = make_buffer(2 * W * H, VK_BUFFER_USAGE_TRANSFER_SRC_BIT);
    Buf down = make_buffer((VkDeviceSize)FW * FH * 4, VK_BUFFER_USAGE_TRANSFER_DST_BIT);
    uint8_t *pa = up.map, *pb = pa + W * H;
    for (int y = 0; y < H; y++) for (int x = 0; x < W; x++) { pa[y * W + x] = pattern(x, y); pb[y * W + x] = pattern(x - DX, y - DY); }
    VkOpticalFlowSessionCreateInfoNV sci = { VK_STRUCTURE_TYPE_OPTICAL_FLOW_SESSION_CREATE_INFO_NV, NULL, W, H,
        VK_FORMAT_R8_UNORM, VK_FORMAT_R16G16_S10_5_NV, VK_FORMAT_UNDEFINED, VK_OPTICAL_FLOW_GRID_SIZE_4X4_BIT_NV,
        0, VK_OPTICAL_FLOW_PERFORMANCE_LEVEL_SLOW_NV, 0 };
    VkOpticalFlowSessionNV s; CK(pCreate(dev, &sci, NULL, &s));
    CK(pBind(dev, s, VK_OPTICAL_FLOW_SESSION_BINDING_POINT_INPUT_NV, b.view, VK_IMAGE_LAYOUT_GENERAL));
    CK(pBind(dev, s, VK_OPTICAL_FLOW_SESSION_BINDING_POINT_REFERENCE_NV, a.view, VK_IMAGE_LAYOUT_GENERAL));
    CK(pBind(dev, s, VK_OPTICAL_FLOW_SESSION_BINDING_POINT_FLOW_VECTOR_NV, fl.view, VK_IMAGE_LAYOUT_GENERAL));
    VkCommandPoolCreateInfo pci = { VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO, NULL, 0, qf };
    VkCommandPool pool; CK(vkCreateCommandPool(dev, &pci, NULL, &pool));
    VkCommandBufferAllocateInfo cai = { VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO, NULL, pool, VK_COMMAND_BUFFER_LEVEL_PRIMARY, 1 };
    VkCommandBuffer cb; CK(vkAllocateCommandBuffers(dev, &cai, &cb));
    VkCommandBufferBeginInfo bi = { VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO, NULL, VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT, NULL };
    CK(vkBeginCommandBuffer(cb, &bi));
    barrier(cb, a.img, VK_IMAGE_LAYOUT_UNDEFINED, VK_IMAGE_LAYOUT_GENERAL);
    barrier(cb, b.img, VK_IMAGE_LAYOUT_UNDEFINED, VK_IMAGE_LAYOUT_GENERAL);
    barrier(cb, fl.img, VK_IMAGE_LAYOUT_UNDEFINED, VK_IMAGE_LAYOUT_GENERAL);
    VkBufferImageCopy c = { 0, 0, 0, { VK_IMAGE_ASPECT_COLOR_BIT, 0, 0, 1 }, { 0, 0, 0 }, { W, H, 1 } };
    vkCmdCopyBufferToImage(cb, up.buf, a.img, VK_IMAGE_LAYOUT_GENERAL, 1, &c);
    c.bufferOffset = W * H; vkCmdCopyBufferToImage(cb, up.buf, b.img, VK_IMAGE_LAYOUT_GENERAL, 1, &c);
    barrier(cb, a.img, VK_IMAGE_LAYOUT_GENERAL, VK_IMAGE_LAYOUT_GENERAL);
    barrier(cb, b.img, VK_IMAGE_LAYOUT_GENERAL, VK_IMAGE_LAYOUT_GENERAL);
    VkOpticalFlowExecuteInfoNV xi = { VK_STRUCTURE_TYPE_OPTICAL_FLOW_EXECUTE_INFO_NV, NULL, 0, 0, NULL };
    pExec(cb, s, &xi);
    barrier(cb, fl.img, VK_IMAGE_LAYOUT_GENERAL, VK_IMAGE_LAYOUT_GENERAL);
    VkBufferImageCopy d = { 0, 0, 0, { VK_IMAGE_ASPECT_COLOR_BIT, 0, 0, 1 }, { 0, 0, 0 }, { FW, FH, 1 } };
    vkCmdCopyImageToBuffer(cb, fl.img, VK_IMAGE_LAYOUT_GENERAL, down.buf, 1, &d);
    CK(vkEndCommandBuffer(cb));
    VkFenceCreateInfo fci = { VK_STRUCTURE_TYPE_FENCE_CREATE_INFO, NULL, 0 };
    VkFence fence; CK(vkCreateFence(dev, &fci, NULL, &fence));
    VkSubmitInfo si = { VK_STRUCTURE_TYPE_SUBMIT_INFO, NULL, 0, NULL, NULL, 1, &cb, 0, NULL };
    CK(vkQueueSubmit(q, 1, &si, fence));
    CK(vkWaitForFences(dev, 1, &fence, VK_TRUE, 20ull * 1000 * 1000 * 1000));
    // the flow field: R16G16_S10_5 (signed, 5 fraction bits = 1/32 px), one vector per 4x4 block
    const int16_t *v = down.map; int ni = 0, good = 0;
    int16_t *xs = malloc(sizeof(int16_t) * FW * FH), *ys = malloc(sizeof(int16_t) * FW * FH);
    for (uint32_t y = 4; y < FH - 4; y++) for (uint32_t x = 4; x < FW - 4; x++) { xs[ni] = v[2 * (y * FW + x)]; ys[ni] = v[2 * (y * FW + x) + 1]; ni++; }
    qsort(xs, ni, sizeof *xs, cmp16); qsort(ys, ni, sizeof *ys, cmp16);
    double mx = xs[ni / 2] / 32.0, my = ys[ni / 2] / 32.0;
    // the motion of B relative to A is (+DX,+DY); a flow reported the other way round is (-DX,-DY)
    double sx = fabs(mx - DX) <= fabs(mx + DX) ? DX : -DX, sy = sx > 0 ? DY : -DY;
    for (uint32_t y = 4; y < FH - 4; y++) for (uint32_t x = 4; x < FW - 4; x++) {
        double fx = v[2 * (y * FW + x)] / 32.0, fy = v[2 * (y * FW + x) + 1] / 32.0;
        good += fabs(fx - sx) <= 1.0 && fabs(fy - sy) <= 1.0;
    }
    double frac = (double)good / ni;
    printf("OFA_MEDIAN_X %.3f\nOFA_MEDIAN_Y %.3f\nOFA_GOOD_FRAC %.4f\nOFA_HASH %016llx\n", mx, my, frac, (unsigned long long)fnv(down.map, (size_t)FW * FH * 4));
    int ok = fabs(fabs(mx) - DX) <= 0.25 && fabs(fabs(my) - DY) <= 0.25 && (mx > 0) == (my > 0) && frac >= 0.8;
    printf(ok ? "OFA_OK\n" : "OFA_FAIL the flow field does not recover the known shift\n");
    pDestroy(dev, s, NULL);
    vkDestroyDevice(dev, NULL); vkDestroyInstance(inst, NULL);
    return ok ? 0 : 1;
}
