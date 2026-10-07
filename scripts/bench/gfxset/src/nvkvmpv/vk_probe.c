/*
 * nvkvm validation -- Vulkan probe.
 *
 * dlopen()s libvulkan.so.1 and hand-rolls the ~20 structs it needs, so it
 * builds with no Vulkan SDK and no vulkan.h. Argument 1 is a path to a
 * compute SPIR-V module.
 *
 * Checks:
 *   vk_loader           -- the loader is present and dispatchable
 *   vk_instance         -- vkCreateInstance succeeds
 *   vk_physical_device  -- at least one physical device, name reported
 *   vk_device_is_nvidia -- vendorID == 0x10DE and the name is NOT a software
 *                          rasteriser. llvmpipe/lavapipe/swrast is a FAIL:
 *                          silently falling back to software is exactly the
 *                          regression this suite exists to catch.
 *   vk_compute_dispatch -- runs data[i] = data[i]*3+7 over 4096 elements on
 *                          the GPU and verifies every element.
 */
#define _GNU_SOURCE
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <ctype.h>
#include <unistd.h>
#include <dlfcn.h>

/* ---- handles: dispatchable are pointers, non-dispatchable are uint64 ----- */
typedef void *VkInstance, *VkPhysicalDevice, *VkDevice, *VkQueue, *VkCommandBuffer;
typedef uint64_t VkBuffer, VkDeviceMemory, VkDescriptorSetLayout, VkPipelineLayout,
                 VkShaderModule, VkPipeline, VkPipelineCache, VkDescriptorPool,
                 VkDescriptorSet, VkCommandPool, VkFence;
typedef int VkResult;
typedef uint64_t VkDeviceSize;

#define VK_SUCCESS 0
#define VK_WHOLE_SIZE (~0ULL)

typedef struct { uint32_t sType; const void *pNext; const char *pApplicationName; uint32_t applicationVersion;
                 const char *pEngineName; uint32_t engineVersion; uint32_t apiVersion; } VkApplicationInfo;
typedef struct { uint32_t sType; const void *pNext; uint32_t flags; const VkApplicationInfo *pApplicationInfo;
                 uint32_t enabledLayerCount; const char *const *ppEnabledLayerNames;
                 uint32_t enabledExtensionCount; const char *const *ppEnabledExtensionNames; } VkInstanceCreateInfo;
typedef struct { uint32_t sType; const void *pNext; uint32_t flags; uint32_t queueFamilyIndex;
                 uint32_t queueCount; const float *pQueuePriorities; } VkDeviceQueueCreateInfo;
typedef struct { uint32_t sType; const void *pNext; uint32_t flags; uint32_t queueCreateInfoCount;
                 const VkDeviceQueueCreateInfo *pQueueCreateInfos; uint32_t enabledLayerCount;
                 const char *const *ppEnabledLayerNames; uint32_t enabledExtensionCount;
                 const char *const *ppEnabledExtensionNames; const void *pEnabledFeatures; } VkDeviceCreateInfo;
typedef struct { uint32_t queueFlags; uint32_t queueCount; uint32_t timestampValidBits;
                 uint32_t w, h, d; } VkQueueFamilyProperties;
typedef struct { uint32_t propertyFlags; uint32_t heapIndex; } VkMemoryType;
typedef struct { VkDeviceSize size; uint32_t flags; } VkMemoryHeap;
typedef struct { uint32_t memoryTypeCount; VkMemoryType memoryTypes[32];
                 uint32_t memoryHeapCount; VkMemoryHeap memoryHeaps[16]; } VkPhysicalDeviceMemoryProperties;
typedef struct { uint32_t sType; const void *pNext; uint32_t flags; VkDeviceSize size; uint32_t usage;
                 uint32_t sharingMode; uint32_t queueFamilyIndexCount; const uint32_t *pQueueFamilyIndices; } VkBufferCreateInfo;
typedef struct { VkDeviceSize size; VkDeviceSize alignment; uint32_t memoryTypeBits; } VkMemoryRequirements;
typedef struct { uint32_t sType; const void *pNext; VkDeviceSize allocationSize; uint32_t memoryTypeIndex; } VkMemoryAllocateInfo;
typedef struct { uint32_t sType; const void *pNext; uint32_t handleType; void *pHostPointer; } VkImportMemoryHostPointerInfoEXT;
typedef struct { uint32_t sType; void *pNext; uint32_t memoryTypeBits; } VkMemoryHostPointerPropertiesEXT;
typedef struct { char extensionName[256]; uint32_t specVersion; } VkExtensionProperties;
typedef struct { uint32_t binding; uint32_t descriptorType; uint32_t descriptorCount; uint32_t stageFlags;
                 const void *pImmutableSamplers; } VkDescriptorSetLayoutBinding;
typedef struct { uint32_t sType; const void *pNext; uint32_t flags; uint32_t bindingCount;
                 const VkDescriptorSetLayoutBinding *pBindings; } VkDescriptorSetLayoutCreateInfo;
typedef struct { uint32_t sType; const void *pNext; uint32_t flags; uint32_t setLayoutCount;
                 const VkDescriptorSetLayout *pSetLayouts; uint32_t pushConstantRangeCount;
                 const void *pPushConstantRanges; } VkPipelineLayoutCreateInfo;
typedef struct { uint32_t sType; const void *pNext; uint32_t flags; size_t codeSize; const uint32_t *pCode; } VkShaderModuleCreateInfo;
typedef struct { uint32_t sType; const void *pNext; uint32_t flags; uint32_t stage; VkShaderModule module;
                 const char *pName; const void *pSpecializationInfo; } VkPipelineShaderStageCreateInfo;
typedef struct { uint32_t sType; const void *pNext; uint32_t flags; VkPipelineShaderStageCreateInfo stage;
                 VkPipelineLayout layout; VkPipeline basePipelineHandle; int32_t basePipelineIndex; } VkComputePipelineCreateInfo;
typedef struct { uint32_t type; uint32_t descriptorCount; } VkDescriptorPoolSize;
typedef struct { uint32_t sType; const void *pNext; uint32_t flags; uint32_t maxSets;
                 uint32_t poolSizeCount; const VkDescriptorPoolSize *pPoolSizes; } VkDescriptorPoolCreateInfo;
typedef struct { uint32_t sType; const void *pNext; VkDescriptorPool descriptorPool;
                 uint32_t descriptorSetCount; const VkDescriptorSetLayout *pSetLayouts; } VkDescriptorSetAllocateInfo;
typedef struct { VkBuffer buffer; VkDeviceSize offset; VkDeviceSize range; } VkDescriptorBufferInfo;
typedef struct { uint32_t sType; const void *pNext; VkDescriptorSet dstSet; uint32_t dstBinding;
                 uint32_t dstArrayElement; uint32_t descriptorCount; uint32_t descriptorType;
                 const void *pImageInfo; const VkDescriptorBufferInfo *pBufferInfo;
                 const void *pTexelBufferView; } VkWriteDescriptorSet;
typedef struct { uint32_t sType; const void *pNext; uint32_t flags; uint32_t queueFamilyIndex; } VkCommandPoolCreateInfo;
typedef struct { uint32_t sType; const void *pNext; VkCommandPool commandPool; uint32_t level;
                 uint32_t commandBufferCount; } VkCommandBufferAllocateInfo;
typedef struct { uint32_t sType; const void *pNext; uint32_t flags; const void *pInheritanceInfo; } VkCommandBufferBeginInfo;
typedef struct { uint32_t sType; const void *pNext; uint32_t waitSemaphoreCount; const void *pWaitSemaphores;
                 const uint32_t *pWaitDstStageMask; uint32_t commandBufferCount;
                 const VkCommandBuffer *pCommandBuffers; uint32_t signalSemaphoreCount;
                 const void *pSignalSemaphores; } VkSubmitInfo;
typedef struct { uint32_t sType; const void *pNext; uint32_t flags; } VkFenceCreateInfo;

#define ST_APP_INFO        0
#define ST_INSTANCE_CI     1
#define ST_DEVQUEUE_CI     2
#define ST_DEVICE_CI       3
#define ST_SUBMIT_INFO     4
#define ST_MEMALLOC_INFO   5
#define ST_FENCE_CI        8
#define ST_BUFFER_CI      12
#define ST_SHADERMOD_CI   16
#define ST_PSSCI          18
#define ST_COMPUTE_PIPE_CI 29
#define ST_PIPELAYOUT_CI  30
/* VK_EXT_external_memory_host.  Values read out of vulkan_core.h, not
 * remembered: extension 179 -> structure types 1000178000/1. */
#define ST_IMPORT_HOSTPTR_INFO 1000178000
#define ST_MEM_HOSTPTR_PROPS   1000178001
#define EXT_MEM_HANDLE_HOST_ALLOC 0x00000080u
#define ST_DSL_CI         32
#define ST_DPOOL_CI       33
#define ST_DSET_ALLOC     34
#define ST_WRITE_DSET     35
#define ST_CMDPOOL_CI     39
#define ST_CMDBUF_ALLOC   40
#define ST_CMDBUF_BEGIN   42

#define VK_BUFFER_USAGE_STORAGE_BUFFER_BIT 0x20
#define VK_DESCRIPTOR_TYPE_STORAGE_BUFFER  7
#define VK_SHADER_STAGE_COMPUTE_BIT        0x20
#define VK_QUEUE_COMPUTE_BIT               0x02
#define VK_PIPELINE_BIND_POINT_COMPUTE     1
#define VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT  0x02
#define VK_MEMORY_PROPERTY_HOST_COHERENT_BIT 0x04

static const char *ALL_CHECKS[] = { "vk_loader", "vk_instance", "vk_physical_device",
                                    "vk_device_is_nvidia", "vk_compute_dispatch",
                                    "vk_import_host_ptr", NULL };
static int reported[16];
static void emit(const char *name, const char *status, const char *fmt, ...) {
    char buf[1024]; va_list ap; va_start(ap, fmt); vsnprintf(buf, sizeof buf, fmt, ap); va_end(ap);
    int i; for (i = 0; ALL_CHECKS[i]; i++) if (!strcmp(ALL_CHECKS[i], name)) reported[i] = 1;
    printf("CHECK|%s|%s|%s\n", name, status, buf); fflush(stdout);
}
static void finish(const char *reason) {
    int i; for (i = 0; ALL_CHECKS[i]; i++) if (!reported[i]) printf("CHECK|%s|SKIP|%s\n", ALL_CHECKS[i], reason);
    fflush(stdout);
}

static void *L;
#define VKSYM(v, n) do { *(void **)(&v) = dlsym(L, n); } while (0)

/* see the note in the CUDA probe: body + _exit(), never a normal return, so a
 * teardown crash inside the ICD cannot mask an otherwise complete result set */
static int probe_main(int argc, char **argv) {
    const char *spv_path = (argc > 1) ? argv[1] : "comp.spv";

    L = dlopen("libvulkan.so.1", RTLD_NOW);
    if (!L) L = dlopen("libvulkan.so", RTLD_NOW);
    if (!L) {
        emit("vk_loader", "SKIP", "dlopen(libvulkan.so.1): %s -- install libvulkan1 in the guest", dlerror());
        finish("no Vulkan loader");
        return 0;
    }

    VkResult (*vkCreateInstance)(const VkInstanceCreateInfo *, const void *, VkInstance *);
    VkResult (*vkEnumeratePhysicalDevices)(VkInstance, uint32_t *, VkPhysicalDevice *);
    void (*vkGetPhysicalDeviceProperties)(VkPhysicalDevice, void *);
    void (*vkGetPhysicalDeviceQueueFamilyProperties)(VkPhysicalDevice, uint32_t *, VkQueueFamilyProperties *);
    void (*vkGetPhysicalDeviceMemoryProperties)(VkPhysicalDevice, VkPhysicalDeviceMemoryProperties *);
    VkResult (*vkCreateDevice)(VkPhysicalDevice, const VkDeviceCreateInfo *, const void *, VkDevice *);
    void (*vkGetDeviceQueue)(VkDevice, uint32_t, uint32_t, VkQueue *);
    VkResult (*vkCreateBuffer)(VkDevice, const VkBufferCreateInfo *, const void *, VkBuffer *);
    void (*vkGetBufferMemoryRequirements)(VkDevice, VkBuffer, VkMemoryRequirements *);
    VkResult (*vkAllocateMemory)(VkDevice, const VkMemoryAllocateInfo *, const void *, VkDeviceMemory *);
    VkResult (*vkBindBufferMemory)(VkDevice, VkBuffer, VkDeviceMemory, VkDeviceSize);
    VkResult (*vkMapMemory)(VkDevice, VkDeviceMemory, VkDeviceSize, VkDeviceSize, uint32_t, void **);
    void (*vkUnmapMemory)(VkDevice, VkDeviceMemory);
    VkResult (*vkCreateDescriptorSetLayout)(VkDevice, const VkDescriptorSetLayoutCreateInfo *, const void *, VkDescriptorSetLayout *);
    VkResult (*vkCreatePipelineLayout)(VkDevice, const VkPipelineLayoutCreateInfo *, const void *, VkPipelineLayout *);
    VkResult (*vkCreateShaderModule)(VkDevice, const VkShaderModuleCreateInfo *, const void *, VkShaderModule *);
    VkResult (*vkCreateComputePipelines)(VkDevice, VkPipelineCache, uint32_t, const VkComputePipelineCreateInfo *, const void *, VkPipeline *);
    VkResult (*vkCreateDescriptorPool)(VkDevice, const VkDescriptorPoolCreateInfo *, const void *, VkDescriptorPool *);
    VkResult (*vkAllocateDescriptorSets)(VkDevice, const VkDescriptorSetAllocateInfo *, VkDescriptorSet *);
    void (*vkUpdateDescriptorSets)(VkDevice, uint32_t, const VkWriteDescriptorSet *, uint32_t, const void *);
    VkResult (*vkCreateCommandPool)(VkDevice, const VkCommandPoolCreateInfo *, const void *, VkCommandPool *);
    VkResult (*vkAllocateCommandBuffers)(VkDevice, const VkCommandBufferAllocateInfo *, VkCommandBuffer *);
    VkResult (*vkBeginCommandBuffer)(VkCommandBuffer, const VkCommandBufferBeginInfo *);
    void (*vkCmdBindPipeline)(VkCommandBuffer, uint32_t, VkPipeline);
    void (*vkCmdBindDescriptorSets)(VkCommandBuffer, uint32_t, VkPipelineLayout, uint32_t, uint32_t, const VkDescriptorSet *, uint32_t, const uint32_t *);
    void (*vkCmdDispatch)(VkCommandBuffer, uint32_t, uint32_t, uint32_t);
    VkResult (*vkEndCommandBuffer)(VkCommandBuffer);
    VkResult (*vkCreateFence)(VkDevice, const VkFenceCreateInfo *, const void *, VkFence *);
    VkResult (*vkQueueSubmit)(VkQueue, uint32_t, const VkSubmitInfo *, VkFence);
    VkResult (*vkWaitForFences)(VkDevice, uint32_t, const VkFence *, uint32_t, uint64_t);
    VkResult (*vkEnumerateDeviceExtensionProperties)(VkPhysicalDevice, const char *, uint32_t *, VkExtensionProperties *);
    void (*vkFreeMemory)(VkDevice, VkDeviceMemory, const void *);
    void *(*vkGetDeviceProcAddr)(VkDevice, const char *);

    VKSYM(vkCreateInstance, "vkCreateInstance");
    VKSYM(vkEnumerateDeviceExtensionProperties, "vkEnumerateDeviceExtensionProperties");
    VKSYM(vkFreeMemory, "vkFreeMemory");
    VKSYM(vkGetDeviceProcAddr, "vkGetDeviceProcAddr");
    VKSYM(vkEnumeratePhysicalDevices, "vkEnumeratePhysicalDevices");
    VKSYM(vkGetPhysicalDeviceProperties, "vkGetPhysicalDeviceProperties");
    VKSYM(vkGetPhysicalDeviceQueueFamilyProperties, "vkGetPhysicalDeviceQueueFamilyProperties");
    VKSYM(vkGetPhysicalDeviceMemoryProperties, "vkGetPhysicalDeviceMemoryProperties");
    VKSYM(vkCreateDevice, "vkCreateDevice");
    VKSYM(vkGetDeviceQueue, "vkGetDeviceQueue");
    VKSYM(vkCreateBuffer, "vkCreateBuffer");
    VKSYM(vkGetBufferMemoryRequirements, "vkGetBufferMemoryRequirements");
    VKSYM(vkAllocateMemory, "vkAllocateMemory");
    VKSYM(vkBindBufferMemory, "vkBindBufferMemory");
    VKSYM(vkMapMemory, "vkMapMemory");
    VKSYM(vkUnmapMemory, "vkUnmapMemory");
    VKSYM(vkCreateDescriptorSetLayout, "vkCreateDescriptorSetLayout");
    VKSYM(vkCreatePipelineLayout, "vkCreatePipelineLayout");
    VKSYM(vkCreateShaderModule, "vkCreateShaderModule");
    VKSYM(vkCreateComputePipelines, "vkCreateComputePipelines");
    VKSYM(vkCreateDescriptorPool, "vkCreateDescriptorPool");
    VKSYM(vkAllocateDescriptorSets, "vkAllocateDescriptorSets");
    VKSYM(vkUpdateDescriptorSets, "vkUpdateDescriptorSets");
    VKSYM(vkCreateCommandPool, "vkCreateCommandPool");
    VKSYM(vkAllocateCommandBuffers, "vkAllocateCommandBuffers");
    VKSYM(vkBeginCommandBuffer, "vkBeginCommandBuffer");
    VKSYM(vkCmdBindPipeline, "vkCmdBindPipeline");
    VKSYM(vkCmdBindDescriptorSets, "vkCmdBindDescriptorSets");
    VKSYM(vkCmdDispatch, "vkCmdDispatch");
    VKSYM(vkEndCommandBuffer, "vkEndCommandBuffer");
    VKSYM(vkCreateFence, "vkCreateFence");
    VKSYM(vkQueueSubmit, "vkQueueSubmit");
    VKSYM(vkWaitForFences, "vkWaitForFences");

    if (!vkCreateInstance || !vkEnumeratePhysicalDevices || !vkGetPhysicalDeviceProperties) {
        emit("vk_loader", "FAIL", "libvulkan.so.1 loaded but core symbols missing");
        finish("loader missing symbols");
        return 1;
    }
    {
        Dl_info di;
        if (dladdr((void *)vkCreateInstance, &di) && di.dli_fname)
            emit("vk_loader", "PASS", "%s", di.dli_fname);
        else
            emit("vk_loader", "PASS", "libvulkan.so.1 loaded");
    }

    VkApplicationInfo ai; memset(&ai, 0, sizeof ai);
    ai.sType = ST_APP_INFO; ai.pApplicationName = "nvkvm-validate";
    ai.pEngineName = "nvkvm-validate"; ai.apiVersion = (1u << 22) | (0u << 12) | 0u;

    VkInstanceCreateInfo ici; memset(&ici, 0, sizeof ici);
    ici.sType = ST_INSTANCE_CI; ici.pApplicationInfo = &ai;

    VkInstance inst = NULL;
    VkResult r = vkCreateInstance(&ici, NULL, &inst);
    if (r != VK_SUCCESS) {
        emit("vk_instance", "FAIL", "vkCreateInstance rc=%d (no usable ICD? check /usr/share/vulkan/icd.d/)", r);
        finish("no Vulkan instance");
        return 1;
    }
    emit("vk_instance", "PASS", "vkCreateInstance rc=0");

    uint32_t ndev = 0;
    r = vkEnumeratePhysicalDevices(inst, &ndev, NULL);
    if (r != VK_SUCCESS || ndev == 0) {
        emit("vk_physical_device", "FAIL", "vkEnumeratePhysicalDevices rc=%d count=%u", r, ndev);
        finish("no Vulkan physical device");
        return 1;
    }
    VkPhysicalDevice *pds = calloc(ndev, sizeof *pds);
    vkEnumeratePhysicalDevices(inst, &ndev, pds);

    /* VkPhysicalDeviceProperties: apiVersion(0) driverVersion(4) vendorID(8)
       deviceID(12) deviceType(16) deviceName[256](20). Over-allocate so we
       never depend on the size of the trailing limits/sparse members. */
    unsigned char props[4096];
    int chosen = -1; char chosen_name[256] = {0}; uint32_t chosen_vendor = 0;
    char all_names[768] = {0};

    uint32_t i;
    for (i = 0; i < ndev; i++) {
        memset(props, 0, sizeof props);
        vkGetPhysicalDeviceProperties(pds[i], props);
        uint32_t vendor; memcpy(&vendor, props + 8, 4);
        const char *nm = (const char *)(props + 20);
        if (all_names[0]) strncat(all_names, ", ", sizeof all_names - strlen(all_names) - 1);
        strncat(all_names, nm, sizeof all_names - strlen(all_names) - 1);
        if (chosen < 0 || (vendor == 0x10DE && chosen_vendor != 0x10DE)) {
            chosen = (int)i; chosen_vendor = vendor;
            snprintf(chosen_name, sizeof chosen_name, "%s", nm);
        }
    }
    emit("vk_physical_device", "PASS", "%u device(s): %s", ndev, all_names);

    /* software rasteriser check -- a fallback must FAIL, never pass */
    char lower[256]; size_t k;
    for (k = 0; k < sizeof lower - 1 && chosen_name[k]; k++) lower[k] = (char)tolower((unsigned char)chosen_name[k]);
    lower[k] = 0;
    int is_sw = strstr(lower, "llvmpipe") || strstr(lower, "lavapipe") ||
                strstr(lower, "swiftshader") || strstr(lower, "software") || strstr(lower, "swrast");
    if (is_sw) {
        emit("vk_device_is_nvidia", "FAIL",
             "SOFTWARE RASTERISER: '%s' (vendorID 0x%04X) -- NVIDIA ICD not reachable",
             chosen_name, chosen_vendor);
        finish("no hardware Vulkan device");
        return 1;
    }
    if (chosen_vendor != 0x10DE) {
        emit("vk_device_is_nvidia", "FAIL", "vendorID 0x%04X != 0x10DE (NVIDIA); device '%s'",
             chosen_vendor, chosen_name);
        finish("chosen Vulkan device is not NVIDIA");
        return 1;
    }
    emit("vk_device_is_nvidia", "PASS", "%s (vendorID 0x%04X)", chosen_name, chosen_vendor);

    VkPhysicalDevice pd = pds[chosen];

    /* ---- compute dispatch ------------------------------------------------ */
    if (!vkCreateDevice || !vkCreateComputePipelines || !vkQueueSubmit) {
        emit("vk_compute_dispatch", "SKIP", "loader is missing compute entry points");
        finish("incomplete loader"); return 1;
    }

    FILE *f = fopen(spv_path, "rb");
    if (!f) { emit("vk_compute_dispatch", "SKIP", "cannot open SPIR-V '%s'", spv_path); finish("no spirv"); return 1; }
    fseek(f, 0, SEEK_END); long spvlen = ftell(f); fseek(f, 0, SEEK_SET);
    uint32_t *spv = malloc((size_t)spvlen);
    if (fread(spv, 1, (size_t)spvlen, f) != (size_t)spvlen) {
        emit("vk_compute_dispatch", "SKIP", "short read on SPIR-V"); fclose(f); finish("bad spirv"); return 1;
    }
    fclose(f);
    if (spvlen < 4 || spv[0] != 0x07230203u) {
        emit("vk_compute_dispatch", "FAIL", "SPIR-V magic is 0x%08X, expected 0x07230203", spvlen >= 4 ? spv[0] : 0);
        finish("bad spirv"); return 1;
    }

    uint32_t nqf = 0;
    vkGetPhysicalDeviceQueueFamilyProperties(pd, &nqf, NULL);
    VkQueueFamilyProperties *qf = calloc(nqf, sizeof *qf);
    vkGetPhysicalDeviceQueueFamilyProperties(pd, &nqf, qf);
    int qfi = -1;
    for (i = 0; i < nqf; i++) if (qf[i].queueFlags & VK_QUEUE_COMPUTE_BIT) { qfi = (int)i; break; }
    if (qfi < 0) { emit("vk_compute_dispatch", "FAIL", "no compute-capable queue family among %u", nqf); finish("no compute queue"); return 1; }

    float prio = 1.0f;
    VkDeviceQueueCreateInfo qci; memset(&qci, 0, sizeof qci);
    qci.sType = ST_DEVQUEUE_CI; qci.queueFamilyIndex = (uint32_t)qfi; qci.queueCount = 1; qci.pQueuePriorities = &prio;
    VkDeviceCreateInfo dci; memset(&dci, 0, sizeof dci);
    dci.sType = ST_DEVICE_CI; dci.queueCreateInfoCount = 1; dci.pQueueCreateInfos = &qci;

    /* VK_EXT_external_memory_host, enabled ONLY if the driver advertises it.
     * Asking for an absent extension makes vkCreateDevice fail, which would
     * turn a missing feature into a spurious vk_compute_dispatch failure. */
    static const char *EMH = "VK_EXT_external_memory_host";
    int have_emh = 0;
    if (vkEnumerateDeviceExtensionProperties) {
        uint32_t ne = 0;
        if (vkEnumerateDeviceExtensionProperties(pd, NULL, &ne, NULL) == VK_SUCCESS && ne) {
            VkExtensionProperties *ep = calloc(ne, sizeof *ep);
            if (ep && vkEnumerateDeviceExtensionProperties(pd, NULL, &ne, ep) == VK_SUCCESS)
                for (i = 0; i < ne; i++)
                    if (!strcmp(ep[i].extensionName, EMH)) { have_emh = 1; break; }
            free(ep);
        }
    }
    if (have_emh) { dci.enabledExtensionCount = 1; dci.ppEnabledExtensionNames = &EMH; }

    VkDevice dev = NULL;
    r = vkCreateDevice(pd, &dci, NULL, &dev);
    if (r != VK_SUCCESS) { emit("vk_compute_dispatch", "FAIL", "vkCreateDevice rc=%d", r); finish("no device"); return 1; }
    VkQueue q = NULL; vkGetDeviceQueue(dev, (uint32_t)qfi, 0, &q);

    /*
     * vk_import_host_ptr -- import application memory (VK_EXT_external_memory_host).
     *
     * This is a ONE-CALL check for a bug that otherwise only shows up as a
     * benchmark scoring badly.  nvkvm denied the ioctl this import rides on
     * (NV_ESC_RM_VID_HEAP_CONTROL, NVOS32 function 27 ALLOC_OS_DESCRIPTOR), so
     * vkAllocateMemory returned VK_ERROR_OUT_OF_DEVICE_MEMORY at EVERY size --
     * 1 MiB through 512 MiB, on RTX 4070/595.84 and RTX 3050 Laptop/580.173.02.
     * Geekbench 7's Vulkan suite scored 0 on one workload of eleven and ~30% of
     * native overall; every other workload, and all of OpenCL, was unaffected.
     * A composite score is a terrible regression detector, so: check the call.
     *
     * 2 MiB, 2 MiB-aligned.  allocationSize must be a multiple of
     * minImportedHostPointerAlignment and the pointer must be aligned to it;
     * over-aligning is always legal, so this holds for any realistic value
     * (measured 0x1000 on both sides) without a second properties query.
     */
    if (!have_emh) {
        emit("vk_import_host_ptr", "SKIP", "%s not advertised by this driver", EMH);
    } else {
        VkResult (*getHostPtrProps)(VkDevice, uint32_t, const void *, VkMemoryHostPointerPropertiesEXT *) = NULL;
        if (vkGetDeviceProcAddr)
            *(void **)(&getHostPtrProps) = vkGetDeviceProcAddr(dev, "vkGetMemoryHostPointerPropertiesEXT");
        if (!getHostPtrProps) {
            emit("vk_import_host_ptr", "FAIL", "%s advertised but vkGetMemoryHostPointerPropertiesEXT did not resolve", EMH);
        } else {
            size_t align = 2u * 1024 * 1024, sz = align;
            void *hp = NULL;
            if (posix_memalign(&hp, align, sz) != 0 || !hp) {
                emit("vk_import_host_ptr", "SKIP", "posix_memalign(%zu) failed on the guest", sz);
            } else {
                VkMemoryHostPointerPropertiesEXT hpp; memset(&hpp, 0, sizeof hpp);
                hpp.sType = ST_MEM_HOSTPTR_PROPS;
                memset(hp, 0, sz);
                r = getHostPtrProps(dev, EXT_MEM_HANDLE_HOST_ALLOC, hp, &hpp);
                if (r != VK_SUCCESS) {
                    emit("vk_import_host_ptr", "FAIL", "vkGetMemoryHostPointerPropertiesEXT rc=%d", r);
                } else {
                    VkPhysicalDeviceMemoryProperties hmp; memset(&hmp, 0, sizeof hmp);
                    vkGetPhysicalDeviceMemoryProperties(pd, &hmp);
                    int hti = -1;
                    for (i = 0; i < hmp.memoryTypeCount; i++)
                        if ((hpp.memoryTypeBits & (1u << i)) &&
                            (hmp.memoryTypes[i].propertyFlags & VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT)) { hti = (int)i; break; }
                    if (hti < 0) {
                        emit("vk_import_host_ptr", "FAIL", "no HOST_VISIBLE type accepts an imported pointer (typeBits=0x%X)", hpp.memoryTypeBits);
                    } else {
                        VkImportMemoryHostPointerInfoEXT imp; memset(&imp, 0, sizeof imp);
                        imp.sType = ST_IMPORT_HOSTPTR_INFO;
                        imp.handleType = EXT_MEM_HANDLE_HOST_ALLOC;
                        imp.pHostPointer = hp;
                        VkMemoryAllocateInfo hmai; memset(&hmai, 0, sizeof hmai);
                        hmai.sType = ST_MEMALLOC_INFO; hmai.pNext = &imp;
                        hmai.allocationSize = sz; hmai.memoryTypeIndex = (uint32_t)hti;
                        VkDeviceMemory hmem = 0;
                        r = vkAllocateMemory(dev, &hmai, NULL, &hmem);
                        if (r == VK_SUCCESS) {
                            emit("vk_import_host_ptr", "PASS", "imported %zu MiB of host memory (type %d)", sz / (1024 * 1024), hti);
                            if (vkFreeMemory) vkFreeMemory(dev, hmem, NULL);
                        } else {
                            emit("vk_import_host_ptr", "FAIL", "vkAllocateMemory(import %zu MiB) rc=%d%s",
                                 sz / (1024 * 1024), r,
                                 r == -2 ? " (VK_ERROR_OUT_OF_DEVICE_MEMORY -- NVOS32 fn27 denied?)" : "");
                        }
                    }
                }
                free(hp);
            }
        }
    }

    const uint32_t N = 4096;
    VkDeviceSize bytes = (VkDeviceSize)N * 4;
    VkBufferCreateInfo bci; memset(&bci, 0, sizeof bci);
    bci.sType = ST_BUFFER_CI; bci.size = bytes; bci.usage = VK_BUFFER_USAGE_STORAGE_BUFFER_BIT;
    VkBuffer buf = 0;
    r = vkCreateBuffer(dev, &bci, NULL, &buf);
    if (r != VK_SUCCESS) { emit("vk_compute_dispatch", "FAIL", "vkCreateBuffer rc=%d", r); finish("x"); return 1; }

    VkMemoryRequirements mr; memset(&mr, 0, sizeof mr);
    vkGetBufferMemoryRequirements(dev, buf, &mr);
    VkPhysicalDeviceMemoryProperties mp; memset(&mp, 0, sizeof mp);
    vkGetPhysicalDeviceMemoryProperties(pd, &mp);
    int mti = -1;
    for (i = 0; i < mp.memoryTypeCount; i++) {
        uint32_t want = VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT;
        if ((mr.memoryTypeBits & (1u << i)) && (mp.memoryTypes[i].propertyFlags & want) == want) { mti = (int)i; break; }
    }
    if (mti < 0) { emit("vk_compute_dispatch", "FAIL", "no HOST_VISIBLE|HOST_COHERENT memory type (typeBits=0x%X, %u types)", mr.memoryTypeBits, mp.memoryTypeCount); finish("x"); return 1; }

    VkMemoryAllocateInfo mai; memset(&mai, 0, sizeof mai);
    mai.sType = ST_MEMALLOC_INFO; mai.allocationSize = mr.size; mai.memoryTypeIndex = (uint32_t)mti;
    VkDeviceMemory mem = 0;
    r = vkAllocateMemory(dev, &mai, NULL, &mem);
    if (r != VK_SUCCESS) { emit("vk_compute_dispatch", "FAIL", "vkAllocateMemory rc=%d", r); finish("x"); return 1; }
    r = vkBindBufferMemory(dev, buf, mem, 0);
    if (r != VK_SUCCESS) { emit("vk_compute_dispatch", "FAIL", "vkBindBufferMemory rc=%d", r); finish("x"); return 1; }

    uint32_t *mapped = NULL;
    r = vkMapMemory(dev, mem, 0, VK_WHOLE_SIZE, 0, (void **)&mapped);
    if (r != VK_SUCCESS) { emit("vk_compute_dispatch", "FAIL", "vkMapMemory rc=%d", r); finish("x"); return 1; }
    for (i = 0; i < N; i++) mapped[i] = i;

    VkDescriptorSetLayoutBinding b; memset(&b, 0, sizeof b);
    b.binding = 0; b.descriptorType = VK_DESCRIPTOR_TYPE_STORAGE_BUFFER; b.descriptorCount = 1; b.stageFlags = VK_SHADER_STAGE_COMPUTE_BIT;
    VkDescriptorSetLayoutCreateInfo dslci; memset(&dslci, 0, sizeof dslci);
    dslci.sType = ST_DSL_CI; dslci.bindingCount = 1; dslci.pBindings = &b;
    VkDescriptorSetLayout dsl = 0;
    r = vkCreateDescriptorSetLayout(dev, &dslci, NULL, &dsl);
    if (r != VK_SUCCESS) { emit("vk_compute_dispatch", "FAIL", "vkCreateDescriptorSetLayout rc=%d", r); finish("x"); return 1; }

    VkPipelineLayoutCreateInfo plci; memset(&plci, 0, sizeof plci);
    plci.sType = ST_PIPELAYOUT_CI; plci.setLayoutCount = 1; plci.pSetLayouts = &dsl;
    VkPipelineLayout pl = 0;
    r = vkCreatePipelineLayout(dev, &plci, NULL, &pl);
    if (r != VK_SUCCESS) { emit("vk_compute_dispatch", "FAIL", "vkCreatePipelineLayout rc=%d", r); finish("x"); return 1; }

    VkShaderModuleCreateInfo smci; memset(&smci, 0, sizeof smci);
    smci.sType = ST_SHADERMOD_CI; smci.codeSize = (size_t)spvlen; smci.pCode = spv;
    VkShaderModule sm = 0;
    r = vkCreateShaderModule(dev, &smci, NULL, &sm);
    if (r != VK_SUCCESS) { emit("vk_compute_dispatch", "FAIL", "vkCreateShaderModule rc=%d", r); finish("x"); return 1; }

    VkComputePipelineCreateInfo cpci; memset(&cpci, 0, sizeof cpci);
    cpci.sType = ST_COMPUTE_PIPE_CI;
    cpci.stage.sType = ST_PSSCI; cpci.stage.stage = VK_SHADER_STAGE_COMPUTE_BIT;
    cpci.stage.module = sm; cpci.stage.pName = "main";
    cpci.layout = pl;
    VkPipeline pipe = 0;
    r = vkCreateComputePipelines(dev, 0, 1, &cpci, NULL, &pipe);
    if (r != VK_SUCCESS) { emit("vk_compute_dispatch", "FAIL", "vkCreateComputePipelines rc=%d", r); finish("x"); return 1; }

    VkDescriptorPoolSize ps; ps.type = VK_DESCRIPTOR_TYPE_STORAGE_BUFFER; ps.descriptorCount = 1;
    VkDescriptorPoolCreateInfo dpci; memset(&dpci, 0, sizeof dpci);
    dpci.sType = ST_DPOOL_CI; dpci.maxSets = 1; dpci.poolSizeCount = 1; dpci.pPoolSizes = &ps;
    VkDescriptorPool dp = 0;
    r = vkCreateDescriptorPool(dev, &dpci, NULL, &dp);
    if (r != VK_SUCCESS) { emit("vk_compute_dispatch", "FAIL", "vkCreateDescriptorPool rc=%d", r); finish("x"); return 1; }

    VkDescriptorSetAllocateInfo dsai; memset(&dsai, 0, sizeof dsai);
    dsai.sType = ST_DSET_ALLOC; dsai.descriptorPool = dp; dsai.descriptorSetCount = 1; dsai.pSetLayouts = &dsl;
    VkDescriptorSet ds = 0;
    r = vkAllocateDescriptorSets(dev, &dsai, &ds);
    if (r != VK_SUCCESS) { emit("vk_compute_dispatch", "FAIL", "vkAllocateDescriptorSets rc=%d", r); finish("x"); return 1; }

    VkDescriptorBufferInfo dbi; dbi.buffer = buf; dbi.offset = 0; dbi.range = VK_WHOLE_SIZE;
    VkWriteDescriptorSet w; memset(&w, 0, sizeof w);
    w.sType = ST_WRITE_DSET; w.dstSet = ds; w.dstBinding = 0; w.descriptorCount = 1;
    w.descriptorType = VK_DESCRIPTOR_TYPE_STORAGE_BUFFER; w.pBufferInfo = &dbi;
    vkUpdateDescriptorSets(dev, 1, &w, 0, NULL);

    VkCommandPoolCreateInfo cpci2; memset(&cpci2, 0, sizeof cpci2);
    cpci2.sType = ST_CMDPOOL_CI; cpci2.queueFamilyIndex = (uint32_t)qfi;
    VkCommandPool cp = 0;
    r = vkCreateCommandPool(dev, &cpci2, NULL, &cp);
    if (r != VK_SUCCESS) { emit("vk_compute_dispatch", "FAIL", "vkCreateCommandPool rc=%d", r); finish("x"); return 1; }

    VkCommandBufferAllocateInfo cbai; memset(&cbai, 0, sizeof cbai);
    cbai.sType = ST_CMDBUF_ALLOC; cbai.commandPool = cp; cbai.level = 0; cbai.commandBufferCount = 1;
    VkCommandBuffer cb = NULL;
    r = vkAllocateCommandBuffers(dev, &cbai, &cb);
    if (r != VK_SUCCESS) { emit("vk_compute_dispatch", "FAIL", "vkAllocateCommandBuffers rc=%d", r); finish("x"); return 1; }

    VkCommandBufferBeginInfo cbbi; memset(&cbbi, 0, sizeof cbbi);
    cbbi.sType = ST_CMDBUF_BEGIN; cbbi.flags = 1;
    vkBeginCommandBuffer(cb, &cbbi);
    vkCmdBindPipeline(cb, VK_PIPELINE_BIND_POINT_COMPUTE, pipe);
    vkCmdBindDescriptorSets(cb, VK_PIPELINE_BIND_POINT_COMPUTE, pl, 0, 1, &ds, 0, NULL);
    vkCmdDispatch(cb, N / 64, 1, 1);
    vkEndCommandBuffer(cb);

    VkFenceCreateInfo fci; memset(&fci, 0, sizeof fci); fci.sType = ST_FENCE_CI;
    VkFence fence = 0;
    r = vkCreateFence(dev, &fci, NULL, &fence);
    if (r != VK_SUCCESS) { emit("vk_compute_dispatch", "FAIL", "vkCreateFence rc=%d", r); finish("x"); return 1; }

    VkSubmitInfo si; memset(&si, 0, sizeof si);
    si.sType = ST_SUBMIT_INFO; si.commandBufferCount = 1; si.pCommandBuffers = &cb;
    r = vkQueueSubmit(q, 1, &si, fence);
    if (r != VK_SUCCESS) { emit("vk_compute_dispatch", "FAIL", "vkQueueSubmit rc=%d", r); finish("x"); return 1; }
    r = vkWaitForFences(dev, 1, &fence, 1, 5000000000ULL);
    if (r != VK_SUCCESS) { emit("vk_compute_dispatch", "FAIL", "vkWaitForFences rc=%d (timeout/lost device)", r); finish("x"); return 1; }

    /* verify EVERY element: data[i] must be i*3+7 */
    uint32_t bad = 0, firsti = 0, firstgot = 0;
    for (i = 0; i < N; i++) {
        uint32_t want = i * 3u + 7u;
        if (mapped[i] != want) { if (!bad) { firsti = i; firstgot = mapped[i]; } bad++; }
    }
    if (bad) emit("vk_compute_dispatch", "FAIL",
                  "%u/%u elements wrong; first i=%u expected %u got %u",
                  bad, N, firsti, firsti * 3u + 7u, firstgot);
    else     emit("vk_compute_dispatch", "PASS",
                  "%u elements, data[i]=i*3+7 verified on %s (e.g. data[%u]=%u)",
                  N, chosen_name, N - 1, mapped[N - 1]);

    vkUnmapMemory(dev, mem);
    finish("not reached");
    return 0;
}

int main(int argc, char **argv) { int rc = probe_main(argc, argv); fflush(NULL); _exit(rc); }
