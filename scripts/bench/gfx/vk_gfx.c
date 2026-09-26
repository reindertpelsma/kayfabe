// vk_gfx — the two Vulkan workloads of the headless-graphics lane (V3_HEADLESS_GRAPHICS.md §6).
//
//   vk_gfx compute   1 Mi-element SSBO kernel, every element verified on the CPU against a closed
//                    form.  Prints `VKC_BAD=<n>` (0 = pass) — a value no stack can fabricate.
//   vk_gfx render    two render passes, all targets VK_IMAGE_TILING_OPTIMAL (block-linear):
//                      pass 1: three interpenetrating triangles, D32 depth test LESS → colour A
//                      pass 2: fullscreen triangle SAMPLING A (transposed, linear filter)  → B
//                    then A, B and the depth image are copied out and hashed (FNV-1a 64).
//                    Prints `VKR_HASH_A= VKR_HASH_B= VKR_HASH_Z=` plus self-checks at probe pixels.
//                    ⊘ The hash is graded against the SAME program on bare metal (same GPU, same
//                    580.159.04 userspace): rasterisation is deterministic there, so any difference
//                    is ours. The probe pixels are a floor that holds without a host to compare to.
//
// Built by build_gfx.sh (glslangValidator → *_spv.h, then gcc ... -lvulkan).
#include <vulkan/vulkan.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "scene_vert_spv.h"
#include "scene_frag_spv.h"
#include "blit_vert_spv.h"
#include "blit_frag_spv.h"
#include "compute_comp_spv.h"
#include "refcheck.h"

#define CK(x) do { VkResult r_ = (x); if (r_ != VK_SUCCESS) { \
    printf("VK_FAIL %s -> %d (line %d)\n", #x, r_, __LINE__); fflush(stdout); exit(3); } } while (0)

static VkInstance inst; static VkPhysicalDevice pd; static VkDevice dev; static VkQueue q;
static uint32_t qfam; static VkCommandPool pool; static VkPhysicalDeviceMemoryProperties mp;

static uint64_t fnv(const void *p, size_t n) {
    const uint8_t *b = p; uint64_t h = 1469598103934665603ull;
    for (size_t i = 0; i < n; i++) { h ^= b[i]; h *= 1099511628211ull; }
    return h;
}

static uint32_t memtype(uint32_t bits, VkMemoryPropertyFlags want) {
    for (uint32_t i = 0; i < mp.memoryTypeCount; i++)
        if ((bits & (1u << i)) && (mp.memoryTypes[i].propertyFlags & want) == want) return i;
    printf("VK_FAIL no memory type bits=0x%x want=0x%x\n", bits, want); exit(3);
}

static void init(void) {
    VkApplicationInfo ai = { .sType = VK_STRUCTURE_TYPE_APPLICATION_INFO, .pApplicationName = "vk_gfx",
                             .apiVersion = VK_API_VERSION_1_1 };
    VkInstanceCreateInfo ici = { .sType = VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO, .pApplicationInfo = &ai };
    CK(vkCreateInstance(&ici, NULL, &inst));
    uint32_t n = 0; CK(vkEnumeratePhysicalDevices(inst, &n, NULL));
    if (!n) { printf("VK_FAIL no physical devices\n"); exit(3); }
    VkPhysicalDevice pds[8]; if (n > 8) n = 8; CK(vkEnumeratePhysicalDevices(inst, &n, pds));
    pd = VK_NULL_HANDLE;
    for (uint32_t i = 0; i < n; i++) {
        VkPhysicalDeviceProperties pp; vkGetPhysicalDeviceProperties(pds[i], &pp);
        printf("VK_DEVICE[%u]=%s vendor=0x%x type=%d api=%u.%u.%u driver=0x%x\n", i, pp.deviceName, pp.vendorID,
               pp.deviceType, VK_VERSION_MAJOR(pp.apiVersion), VK_VERSION_MINOR(pp.apiVersion),
               VK_VERSION_PATCH(pp.apiVersion), pp.driverVersion);
        if (pp.vendorID == 0x10de && pd == VK_NULL_HANDLE) pd = pds[i];
    }
    if (pd == VK_NULL_HANDLE) { printf("VK_FAIL no NVIDIA physical device (llvmpipe only?)\n"); exit(3); }
    vkGetPhysicalDeviceMemoryProperties(pd, &mp);
    uint32_t nq = 0; vkGetPhysicalDeviceQueueFamilyProperties(pd, &nq, NULL);
    VkQueueFamilyProperties qp[16]; if (nq > 16) nq = 16;
    vkGetPhysicalDeviceQueueFamilyProperties(pd, &nq, qp);
    qfam = UINT32_MAX;
    for (uint32_t i = 0; i < nq; i++)
        if ((qp[i].queueFlags & (VK_QUEUE_GRAPHICS_BIT | VK_QUEUE_COMPUTE_BIT)) ==
            (VK_QUEUE_GRAPHICS_BIT | VK_QUEUE_COMPUTE_BIT)) { qfam = i; break; }
    if (qfam == UINT32_MAX) { printf("VK_FAIL no graphics+compute queue\n"); exit(3); }
    float pr = 1.0f;
    VkDeviceQueueCreateInfo qci = { .sType = VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO, .queueFamilyIndex = qfam,
                                    .queueCount = 1, .pQueuePriorities = &pr };
    VkDeviceCreateInfo dci = { .sType = VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO, .queueCreateInfoCount = 1,
                               .pQueueCreateInfos = &qci };
    CK(vkCreateDevice(pd, &dci, NULL, &dev));
    printf("VK_STAGE=device_created\n"); fflush(stdout);
    vkGetDeviceQueue(dev, qfam, 0, &q);
    VkCommandPoolCreateInfo pci = { .sType = VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO, .queueFamilyIndex = qfam };
    CK(vkCreateCommandPool(dev, &pci, NULL, &pool));
}

typedef struct { VkBuffer b; VkDeviceMemory m; void *p; } Buf;
static Buf mkbuf(VkDeviceSize sz, VkBufferUsageFlags use, VkMemoryPropertyFlags want) {
    Buf r = {0};
    VkBufferCreateInfo bi = { .sType = VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO, .size = sz, .usage = use };
    CK(vkCreateBuffer(dev, &bi, NULL, &r.b));
    VkMemoryRequirements mr; vkGetBufferMemoryRequirements(dev, r.b, &mr);
    VkMemoryAllocateInfo ma = { .sType = VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO, .allocationSize = mr.size,
                                .memoryTypeIndex = memtype(mr.memoryTypeBits, want) };
    CK(vkAllocateMemory(dev, &ma, NULL, &r.m)); CK(vkBindBufferMemory(dev, r.b, r.m, 0));
    if (want & VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT) CK(vkMapMemory(dev, r.m, 0, VK_WHOLE_SIZE, 0, &r.p));
    return r;
}

typedef struct { VkImage i; VkDeviceMemory m; VkImageView v; } Img;
static Img mkimg(VkFormat f, VkImageUsageFlags use, VkImageAspectFlags asp, uint32_t w, uint32_t h) {
    Img r = {0};
    VkImageCreateInfo ii = { .sType = VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO, .imageType = VK_IMAGE_TYPE_2D,
        .format = f, .extent = { w, h, 1 }, .mipLevels = 1, .arrayLayers = 1, .samples = VK_SAMPLE_COUNT_1_BIT,
        .tiling = VK_IMAGE_TILING_OPTIMAL, .usage = use, .initialLayout = VK_IMAGE_LAYOUT_UNDEFINED };
    CK(vkCreateImage(dev, &ii, NULL, &r.i));
    VkMemoryRequirements mr; vkGetImageMemoryRequirements(dev, r.i, &mr);
    VkMemoryAllocateInfo ma = { .sType = VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO, .allocationSize = mr.size,
        .memoryTypeIndex = memtype(mr.memoryTypeBits, VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT) };
    CK(vkAllocateMemory(dev, &ma, NULL, &r.m)); CK(vkBindImageMemory(dev, r.i, r.m, 0));
    VkImageViewCreateInfo vi = { .sType = VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO, .image = r.i,
        .viewType = VK_IMAGE_VIEW_TYPE_2D, .format = f, .subresourceRange = { asp, 0, 1, 0, 1 } };
    CK(vkCreateImageView(dev, &vi, NULL, &r.v));
    return r;
}

static VkShaderModule shader(const uint32_t *code, size_t bytes) {
    VkShaderModuleCreateInfo si = { .sType = VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO, .codeSize = bytes,
                                    .pCode = code };
    VkShaderModule m; CK(vkCreateShaderModule(dev, &si, NULL, &m)); return m;
}

static VkCommandBuffer begin(void) {
    VkCommandBufferAllocateInfo ai = { .sType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO, .commandPool = pool,
                                       .level = VK_COMMAND_BUFFER_LEVEL_PRIMARY, .commandBufferCount = 1 };
    VkCommandBuffer cb; CK(vkAllocateCommandBuffers(dev, &ai, &cb));
    VkCommandBufferBeginInfo bi = { .sType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO,
                                    .flags = VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT };
    CK(vkBeginCommandBuffer(cb, &bi)); return cb;
}
static void submit_wait(VkCommandBuffer cb) {
    CK(vkEndCommandBuffer(cb));
    VkFenceCreateInfo fi = { .sType = VK_STRUCTURE_TYPE_FENCE_CREATE_INFO };
    VkFence f; CK(vkCreateFence(dev, &fi, NULL, &f));
    VkSubmitInfo si = { .sType = VK_STRUCTURE_TYPE_SUBMIT_INFO, .commandBufferCount = 1, .pCommandBuffers = &cb };
    CK(vkQueueSubmit(q, 1, &si, f));
    // 20 s: a hang here is a lost completion, and it must surface as a named result, not a wedge.
    VkResult r = vkWaitForFences(dev, 1, &f, VK_TRUE, 20ull * 1000000000ull);
    if (r != VK_SUCCESS) { printf("VK_FAIL fence wait -> %d (completion never arrived)\n", r); fflush(stdout); exit(4); }
    vkDestroyFence(dev, f, NULL);
}
static void full_barrier(VkCommandBuffer cb) {
    VkMemoryBarrier mb = { .sType = VK_STRUCTURE_TYPE_MEMORY_BARRIER,
        .srcAccessMask = VK_ACCESS_MEMORY_WRITE_BIT, .dstAccessMask = VK_ACCESS_MEMORY_READ_BIT | VK_ACCESS_MEMORY_WRITE_BIT };
    vkCmdPipelineBarrier(cb, VK_PIPELINE_STAGE_ALL_COMMANDS_BIT, VK_PIPELINE_STAGE_ALL_COMMANDS_BIT, 0, 1, &mb, 0, NULL, 0, NULL);
}

static int run_compute(void) {
    const uint32_t N = 1u << 20; const VkDeviceSize SZ = (VkDeviceSize)N * 4;
    Buf in = mkbuf(SZ, VK_BUFFER_USAGE_STORAGE_BUFFER_BIT,
                   VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT);
    Buf out = mkbuf(SZ, VK_BUFFER_USAGE_STORAGE_BUFFER_BIT,
                    VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT);
    uint32_t *a = in.p, *b = out.p;
    for (uint32_t i = 0; i < N; i++) { a[i] = i * 2654435761u; b[i] = 0xdeadbeef; }
    VkDescriptorSetLayoutBinding lb[2] = {
        { 0, VK_DESCRIPTOR_TYPE_STORAGE_BUFFER, 1, VK_SHADER_STAGE_COMPUTE_BIT, NULL },
        { 1, VK_DESCRIPTOR_TYPE_STORAGE_BUFFER, 1, VK_SHADER_STAGE_COMPUTE_BIT, NULL } };
    VkDescriptorSetLayoutCreateInfo li = { .sType = VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO, .bindingCount = 2, .pBindings = lb };
    VkDescriptorSetLayout dsl; CK(vkCreateDescriptorSetLayout(dev, &li, NULL, &dsl));
    VkPipelineLayoutCreateInfo pli = { .sType = VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO, .setLayoutCount = 1, .pSetLayouts = &dsl };
    VkPipelineLayout pl; CK(vkCreatePipelineLayout(dev, &pli, NULL, &pl));
    VkComputePipelineCreateInfo cpi = { .sType = VK_STRUCTURE_TYPE_COMPUTE_PIPELINE_CREATE_INFO, .layout = pl,
        .stage = { .sType = VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO, .stage = VK_SHADER_STAGE_COMPUTE_BIT,
                   .module = shader(compute_comp_spv, sizeof compute_comp_spv), .pName = "main" } };
    VkPipeline p; CK(vkCreateComputePipelines(dev, VK_NULL_HANDLE, 1, &cpi, NULL, &p));
    VkDescriptorPoolSize ps = { VK_DESCRIPTOR_TYPE_STORAGE_BUFFER, 2 };
    VkDescriptorPoolCreateInfo dpi = { .sType = VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO, .maxSets = 1, .poolSizeCount = 1, .pPoolSizes = &ps };
    VkDescriptorPool dp; CK(vkCreateDescriptorPool(dev, &dpi, NULL, &dp));
    VkDescriptorSetAllocateInfo dai = { .sType = VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO, .descriptorPool = dp, .descriptorSetCount = 1, .pSetLayouts = &dsl };
    VkDescriptorSet ds; CK(vkAllocateDescriptorSets(dev, &dai, &ds));
    VkDescriptorBufferInfo bi[2] = { { in.b, 0, SZ }, { out.b, 0, SZ } };
    VkWriteDescriptorSet w[2] = {
        { .sType = VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET, .dstSet = ds, .dstBinding = 0, .descriptorCount = 1, .descriptorType = VK_DESCRIPTOR_TYPE_STORAGE_BUFFER, .pBufferInfo = &bi[0] },
        { .sType = VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET, .dstSet = ds, .dstBinding = 1, .descriptorCount = 1, .descriptorType = VK_DESCRIPTOR_TYPE_STORAGE_BUFFER, .pBufferInfo = &bi[1] } };
    vkUpdateDescriptorSets(dev, 2, w, 0, NULL);
    VkCommandBuffer cb = begin();
    vkCmdBindPipeline(cb, VK_PIPELINE_BIND_POINT_COMPUTE, p);
    vkCmdBindDescriptorSets(cb, VK_PIPELINE_BIND_POINT_COMPUTE, pl, 0, 1, &ds, 0, NULL);
    vkCmdDispatch(cb, N / 256, 1, 1);
    full_barrier(cb);
    submit_wait(cb);
    uint32_t bad = 0, first = UINT32_MAX;
    for (uint32_t i = 0; i < N; i++) {
        uint32_t want = a[i] * 3u + 1u + (i ^ (i >> 7));
        if (b[i] != want) { if (first == UINT32_MAX) first = i; bad++; }
    }
    if (bad) printf("VKC_FIRST_BAD i=%u got=0x%08x want=0x%08x\n", first, b[first], a[first] * 3u + 1u + (first ^ (first >> 7)));
    printf("VKC_N=%u\nVKC_HASH=%016llx\nVKC_BAD=%u\n", N, (unsigned long long)fnv(b, SZ), bad);
    return bad ? 1 : 0;
}

#define W 256
#define H 256
static VkRenderPass mkpass(int with_depth, VkImageLayout colfinal) {
    VkAttachmentDescription at[2] = {
        { 0, VK_FORMAT_R8G8B8A8_UNORM, VK_SAMPLE_COUNT_1_BIT, VK_ATTACHMENT_LOAD_OP_CLEAR, VK_ATTACHMENT_STORE_OP_STORE,
          VK_ATTACHMENT_LOAD_OP_DONT_CARE, VK_ATTACHMENT_STORE_OP_DONT_CARE, VK_IMAGE_LAYOUT_UNDEFINED, colfinal },
        { 0, VK_FORMAT_D32_SFLOAT, VK_SAMPLE_COUNT_1_BIT, VK_ATTACHMENT_LOAD_OP_CLEAR, VK_ATTACHMENT_STORE_OP_STORE,
          VK_ATTACHMENT_LOAD_OP_DONT_CARE, VK_ATTACHMENT_STORE_OP_DONT_CARE, VK_IMAGE_LAYOUT_UNDEFINED,
          VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL } };
    VkAttachmentReference cr = { 0, VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL };
    VkAttachmentReference dr = { 1, VK_IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL };
    VkSubpassDescription sp = { .pipelineBindPoint = VK_PIPELINE_BIND_POINT_GRAPHICS, .colorAttachmentCount = 1,
                                .pColorAttachments = &cr, .pDepthStencilAttachment = with_depth ? &dr : NULL };
    VkRenderPassCreateInfo ri = { .sType = VK_STRUCTURE_TYPE_RENDER_PASS_CREATE_INFO, .attachmentCount = with_depth ? 2 : 1,
                                  .pAttachments = at, .subpassCount = 1, .pSubpasses = &sp };
    VkRenderPass rp; CK(vkCreateRenderPass(dev, &ri, NULL, &rp)); return rp;
}
static VkPipeline mkgfx(VkRenderPass rp, VkPipelineLayout pl, VkShaderModule vs, VkShaderModule fs, int depth) {
    VkPipelineShaderStageCreateInfo st[2] = {
        { .sType = VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO, .stage = VK_SHADER_STAGE_VERTEX_BIT, .module = vs, .pName = "main" },
        { .sType = VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO, .stage = VK_SHADER_STAGE_FRAGMENT_BIT, .module = fs, .pName = "main" } };
    VkPipelineVertexInputStateCreateInfo vi = { .sType = VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO };
    VkPipelineInputAssemblyStateCreateInfo ia = { .sType = VK_STRUCTURE_TYPE_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO,
                                                  .topology = VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST };
    VkViewport vp = { 0, 0, W, H, 0, 1 }; VkRect2D sc = { { 0, 0 }, { W, H } };
    VkPipelineViewportStateCreateInfo vs_ = { .sType = VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_STATE_CREATE_INFO,
        .viewportCount = 1, .pViewports = &vp, .scissorCount = 1, .pScissors = &sc };
    VkPipelineRasterizationStateCreateInfo rs = { .sType = VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_STATE_CREATE_INFO,
        .polygonMode = VK_POLYGON_MODE_FILL, .cullMode = VK_CULL_MODE_NONE, .frontFace = VK_FRONT_FACE_COUNTER_CLOCKWISE, .lineWidth = 1 };
    VkPipelineMultisampleStateCreateInfo ms = { .sType = VK_STRUCTURE_TYPE_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO,
        .rasterizationSamples = VK_SAMPLE_COUNT_1_BIT };
    VkPipelineDepthStencilStateCreateInfo dss = { .sType = VK_STRUCTURE_TYPE_PIPELINE_DEPTH_STENCIL_STATE_CREATE_INFO,
        .depthTestEnable = depth, .depthWriteEnable = depth, .depthCompareOp = VK_COMPARE_OP_LESS };
    VkPipelineColorBlendAttachmentState cba = { .colorWriteMask = 0xf };
    VkPipelineColorBlendStateCreateInfo cb = { .sType = VK_STRUCTURE_TYPE_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO,
        .attachmentCount = 1, .pAttachments = &cba };
    VkGraphicsPipelineCreateInfo gi = { .sType = VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO, .stageCount = 2,
        .pStages = st, .pVertexInputState = &vi, .pInputAssemblyState = &ia, .pViewportState = &vs_,
        .pRasterizationState = &rs, .pMultisampleState = &ms, .pDepthStencilState = &dss, .pColorBlendState = &cb,
        .layout = pl, .renderPass = rp };
    VkPipeline p; CK(vkCreateGraphicsPipelines(dev, VK_NULL_HANDLE, 1, &gi, NULL, &p)); return p;
}
static void copyout(VkCommandBuffer cb, VkImage i, VkImageAspectFlags asp, VkBuffer b) {
    VkBufferImageCopy c = { .imageSubresource = { asp, 0, 0, 1 }, .imageExtent = { W, H, 1 } };
    vkCmdCopyImageToBuffer(cb, i, VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL, b, 1, &c);
}
static void write_ppm(const char *path, const uint8_t *rgba) {
    FILE *f = fopen(path, "wb"); if (!f) return;
    fprintf(f, "P6\n%d %d\n255\n", W, H);
    for (int i = 0; i < W * H; i++) fwrite(rgba + 4 * i, 1, 3, f);
    fclose(f);
}

static int run_render(const char *ppm_prefix) {
    Img A = mkimg(VK_FORMAT_R8G8B8A8_UNORM, VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT | VK_IMAGE_USAGE_SAMPLED_BIT |
                  VK_IMAGE_USAGE_TRANSFER_SRC_BIT, VK_IMAGE_ASPECT_COLOR_BIT, W, H);
    Img Z = mkimg(VK_FORMAT_D32_SFLOAT, VK_IMAGE_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT | VK_IMAGE_USAGE_TRANSFER_SRC_BIT,
                  VK_IMAGE_ASPECT_DEPTH_BIT, W, H);
    Img B = mkimg(VK_FORMAT_R8G8B8A8_UNORM, VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT | VK_IMAGE_USAGE_TRANSFER_SRC_BIT,
                  VK_IMAGE_ASPECT_COLOR_BIT, W, H);
    VkMemoryPropertyFlags hv = VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT;
    Buf ra = mkbuf(W * H * 4, VK_BUFFER_USAGE_TRANSFER_DST_BIT, hv);
    Buf rz = mkbuf(W * H * 4, VK_BUFFER_USAGE_TRANSFER_DST_BIT, hv);
    Buf rb = mkbuf(W * H * 4, VK_BUFFER_USAGE_TRANSFER_DST_BIT, hv);
    memset(ra.p, 0x5a, W * H * 4); memset(rz.p, 0x5a, W * H * 4); memset(rb.p, 0x5a, W * H * 4);

    VkRenderPass rp1 = mkpass(1, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL);
    VkRenderPass rp2 = mkpass(0, VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL);
    VkImageView v1[2] = { A.v, Z.v };
    VkFramebufferCreateInfo fi = { .sType = VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO, .renderPass = rp1,
                                   .attachmentCount = 2, .pAttachments = v1, .width = W, .height = H, .layers = 1 };
    VkFramebuffer fb1, fb2; CK(vkCreateFramebuffer(dev, &fi, NULL, &fb1));
    fi.renderPass = rp2; fi.attachmentCount = 1; fi.pAttachments = &B.v; CK(vkCreateFramebuffer(dev, &fi, NULL, &fb2));

    VkPipelineLayoutCreateInfo pli0 = { .sType = VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO };
    VkPipelineLayout pl1; CK(vkCreatePipelineLayout(dev, &pli0, NULL, &pl1));
    VkDescriptorSetLayoutBinding lb = { 0, VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER, 1, VK_SHADER_STAGE_FRAGMENT_BIT, NULL };
    VkDescriptorSetLayoutCreateInfo li = { .sType = VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO, .bindingCount = 1, .pBindings = &lb };
    VkDescriptorSetLayout dsl; CK(vkCreateDescriptorSetLayout(dev, &li, NULL, &dsl));
    VkPipelineLayoutCreateInfo pli = { .sType = VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO, .setLayoutCount = 1, .pSetLayouts = &dsl };
    VkPipelineLayout pl2; CK(vkCreatePipelineLayout(dev, &pli, NULL, &pl2));
    VkPipeline p1 = mkgfx(rp1, pl1, shader(scene_vert_spv, sizeof scene_vert_spv), shader(scene_frag_spv, sizeof scene_frag_spv), 1);
    VkPipeline p2 = mkgfx(rp2, pl2, shader(blit_vert_spv, sizeof blit_vert_spv), shader(blit_frag_spv, sizeof blit_frag_spv), 0);
    printf("VK_STAGE=pipelines_built\n"); fflush(stdout);

    VkSamplerCreateInfo smi = { .sType = VK_STRUCTURE_TYPE_SAMPLER_CREATE_INFO, .magFilter = VK_FILTER_LINEAR,
        .minFilter = VK_FILTER_LINEAR, .addressModeU = VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE,
        .addressModeV = VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE, .addressModeW = VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE, .maxLod = 0 };
    VkSampler smp; CK(vkCreateSampler(dev, &smi, NULL, &smp));
    VkDescriptorPoolSize ps = { VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER, 1 };
    VkDescriptorPoolCreateInfo dpi = { .sType = VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO, .maxSets = 1, .poolSizeCount = 1, .pPoolSizes = &ps };
    VkDescriptorPool dp; CK(vkCreateDescriptorPool(dev, &dpi, NULL, &dp));
    VkDescriptorSetAllocateInfo dai = { .sType = VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO, .descriptorPool = dp, .descriptorSetCount = 1, .pSetLayouts = &dsl };
    VkDescriptorSet ds; CK(vkAllocateDescriptorSets(dev, &dai, &ds));
    VkDescriptorImageInfo dii = { smp, A.v, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL };
    VkWriteDescriptorSet wd = { .sType = VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET, .dstSet = ds, .dstBinding = 0,
        .descriptorCount = 1, .descriptorType = VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER, .pImageInfo = &dii };
    vkUpdateDescriptorSets(dev, 1, &wd, 0, NULL);

    VkCommandBuffer cb = begin();
    VkClearValue cv[2] = { { .color = { { 0.1f, 0.2f, 0.3f, 1.0f } } }, { .depthStencil = { 1.0f, 0 } } };
    VkRenderPassBeginInfo rbi = { .sType = VK_STRUCTURE_TYPE_RENDER_PASS_BEGIN_INFO, .renderPass = rp1, .framebuffer = fb1,
                                  .renderArea = { { 0, 0 }, { W, H } }, .clearValueCount = 2, .pClearValues = cv };
    vkCmdBeginRenderPass(cb, &rbi, VK_SUBPASS_CONTENTS_INLINE);
    vkCmdBindPipeline(cb, VK_PIPELINE_BIND_POINT_GRAPHICS, p1);
    vkCmdDraw(cb, 9, 1, 0, 0);
    vkCmdEndRenderPass(cb);
    full_barrier(cb);
    VkClearValue cv2 = { .color = { { 0.0f, 0.0f, 0.0f, 1.0f } } };
    rbi.renderPass = rp2; rbi.framebuffer = fb2; rbi.clearValueCount = 1; rbi.pClearValues = &cv2;
    vkCmdBeginRenderPass(cb, &rbi, VK_SUBPASS_CONTENTS_INLINE);
    vkCmdBindPipeline(cb, VK_PIPELINE_BIND_POINT_GRAPHICS, p2);
    vkCmdBindDescriptorSets(cb, VK_PIPELINE_BIND_POINT_GRAPHICS, pl2, 0, 1, &ds, 0, NULL);
    vkCmdDraw(cb, 3, 1, 0, 0);
    vkCmdEndRenderPass(cb);
    VkImageMemoryBarrier ib = { .sType = VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER, .srcAccessMask = VK_ACCESS_SHADER_READ_BIT,
        .dstAccessMask = VK_ACCESS_TRANSFER_READ_BIT, .oldLayout = VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        .newLayout = VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL, .srcQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED,
        .dstQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED, .image = A.i, .subresourceRange = { VK_IMAGE_ASPECT_COLOR_BIT, 0, 1, 0, 1 } };
    vkCmdPipelineBarrier(cb, VK_PIPELINE_STAGE_ALL_COMMANDS_BIT, VK_PIPELINE_STAGE_TRANSFER_BIT, 0, 0, NULL, 0, NULL, 1, &ib);
    full_barrier(cb);
    copyout(cb, A.i, VK_IMAGE_ASPECT_COLOR_BIT, ra.b);
    copyout(cb, Z.i, VK_IMAGE_ASPECT_DEPTH_BIT, rz.b);
    copyout(cb, B.i, VK_IMAGE_ASPECT_COLOR_BIT, rb.b);
    VkMemoryBarrier hb = { .sType = VK_STRUCTURE_TYPE_MEMORY_BARRIER, .srcAccessMask = VK_ACCESS_TRANSFER_WRITE_BIT,
                           .dstAccessMask = VK_ACCESS_HOST_READ_BIT };
    vkCmdPipelineBarrier(cb, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_PIPELINE_STAGE_HOST_BIT, 0, 1, &hb, 0, NULL, 0, NULL);
    submit_wait(cb);

    const uint8_t *a = ra.p, *b = rb.p; const float *z = rz.p;
    // Vulkan framebuffer row r ↔ NDC y = (r+0.5)/H*2-1 — the convention refcheck.h assumes.
    int fails = ref_check("VKR", a, z, b, 0.1f);
    printf("VKR_HASH_A=%016llx\nVKR_HASH_Z=%016llx\nVKR_HASH_B=%016llx\n", (unsigned long long)fnv(a, W * H * 4),
           (unsigned long long)fnv(z, W * H * 4), (unsigned long long)fnv(b, W * H * 4));
    printf("VKR_REF_FAILS=%d\n", fails);
    if (ppm_prefix) { char p[512]; snprintf(p, sizeof p, "%s_A.ppm", ppm_prefix); write_ppm(p, a);
                      snprintf(p, sizeof p, "%s_B.ppm", ppm_prefix); write_ppm(p, b); }
    return fails ? 1 : 0;
}

int main(int argc, char **argv) {
    setvbuf(stdout, NULL, _IOLBF, 0);
    const char *mode = argc > 1 ? argv[1] : "render";
    printf("VK_START mode=%s\n", mode);
    init();
    int rc = !strcmp(mode, "compute") ? run_compute() : run_render(argc > 2 ? argv[2] : NULL);
    vkDeviceWaitIdle(dev);
    printf("VK_DONE rc=%d\n", rc);
    return rc;
}
