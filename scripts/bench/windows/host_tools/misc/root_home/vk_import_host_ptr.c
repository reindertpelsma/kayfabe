/* SPDX-License-Identifier: GPL-2.0 OR Apache-2.0 */
/*
 * vk_import_host_ptr.c — isolate VK_EXT_external_memory_host import.
 *
 * Geekbench 7's Vulkan suite fails one workload (Path Tracer) in an nvkvm guest
 * with, from its own helper:
 *
 *   allocate_buffer_host(physical_device_, device_, host_ptr_, buffer_size_,
 *                        staging_usage, staging_properties, ...) returned -2
 *
 * MEASURED 2026-09-05: reproduces on RTX 4070 / 595.84 and RTX 3050 Laptop /
 * 580.173.02, on two trees, on an otherwise idle box.  The guest advertises
 * VK_EXT_external_memory_host, the same memory heaps and the same
 * maxMemoryAllocationSize as the host, so nothing in the advertised capability
 * explains it.  The `host_ptr` argument says the allocation imports APPLICATION
 * memory, which is the one operation a forwarding design has to do real work
 * for.  This isolates that call from the benchmark.
 *
 * Build:  cc -O2 -o vk_import_host_ptr vk_import_host_ptr.c -lvulkan
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <vulkan/vulkan.h>

static const char *rstr(VkResult r)
{
    switch (r) {
    case VK_SUCCESS:                        return "VK_SUCCESS";
    case VK_ERROR_OUT_OF_HOST_MEMORY:       return "VK_ERROR_OUT_OF_HOST_MEMORY (-1)";
    case VK_ERROR_OUT_OF_DEVICE_MEMORY:     return "VK_ERROR_OUT_OF_DEVICE_MEMORY (-2)";
    case VK_ERROR_INVALID_EXTERNAL_HANDLE:  return "VK_ERROR_INVALID_EXTERNAL_HANDLE";
    case VK_ERROR_INITIALIZATION_FAILED:    return "VK_ERROR_INITIALIZATION_FAILED";
    default: { static char b[32]; snprintf(b, sizeof b, "VkResult %d", (int)r); return b; }
    }
}

int main(int argc, char **argv)
{
    size_t mb = (argc > 1) ? (size_t)strtoul(argv[1], NULL, 10) : 256;
    size_t bytes = mb * 1024u * 1024u;

    VkApplicationInfo ai = { .sType = VK_STRUCTURE_TYPE_APPLICATION_INFO,
                             .apiVersion = VK_API_VERSION_1_1 };
    VkInstanceCreateInfo ici = { .sType = VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO,
                                 .pApplicationInfo = &ai };
    VkInstance inst;
    if (vkCreateInstance(&ici, NULL, &inst) != VK_SUCCESS) {
        fprintf(stderr, "vkCreateInstance failed\n"); return 2;
    }
    uint32_t n = 0; vkEnumeratePhysicalDevices(inst, &n, NULL);
    VkPhysicalDevice *pd = calloc(n, sizeof *pd);
    vkEnumeratePhysicalDevices(inst, &n, pd);

    VkPhysicalDevice chosen = VK_NULL_HANDLE; VkPhysicalDeviceProperties props;
    for (uint32_t i = 0; i < n; i++) {
        vkGetPhysicalDeviceProperties(pd[i], &props);
        if (props.deviceType == VK_PHYSICAL_DEVICE_TYPE_DISCRETE_GPU) { chosen = pd[i]; break; }
    }
    if (!chosen) { fprintf(stderr, "no discrete GPU\n"); return 2; }
    vkGetPhysicalDeviceProperties(chosen, &props);
    printf("device: %s\n", props.deviceName);

    VkPhysicalDeviceExternalMemoryHostPropertiesEXT hp = {
        .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_EXTERNAL_MEMORY_HOST_PROPERTIES_EXT };
    VkPhysicalDeviceProperties2 p2 = { .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PROPERTIES_2, .pNext = &hp };
    vkGetPhysicalDeviceProperties2(chosen, &p2);
    printf("minImportedHostPointerAlignment: 0x%llx\n", (unsigned long long)hp.minImportedHostPointerAlignment);

    float prio = 1.0f;
    VkDeviceQueueCreateInfo q = { .sType = VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO,
                                  .queueCount = 1, .pQueuePriorities = &prio };
    const char *devext[] = { VK_EXT_EXTERNAL_MEMORY_HOST_EXTENSION_NAME };
    VkDeviceCreateInfo dci = { .sType = VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO,
                               .queueCreateInfoCount = 1, .pQueueCreateInfos = &q,
                               .enabledExtensionCount = 1, .ppEnabledExtensionNames = devext };
    VkDevice dev;
    VkResult r = vkCreateDevice(chosen, &dci, NULL, &dev);
    if (r != VK_SUCCESS) { printf("vkCreateDevice: %s\n", rstr(r)); return 3; }

    PFN_vkGetMemoryHostPointerPropertiesEXT getProps =
        (PFN_vkGetMemoryHostPointerPropertiesEXT)vkGetDeviceProcAddr(dev, "vkGetMemoryHostPointerPropertiesEXT");
    if (!getProps) { printf("vkGetMemoryHostPointerPropertiesEXT: NOT RESOLVED\n"); return 3; }

    size_t align = hp.minImportedHostPointerAlignment ? hp.minImportedHostPointerAlignment : 4096;
    size_t sz = (bytes + align - 1) & ~(align - 1);
    void *host = NULL;
    if (posix_memalign(&host, align, sz) != 0) { fprintf(stderr, "posix_memalign failed\n"); return 2; }
    memset(host, 0, sz);
    printf("HOSTPTR %p  SIZE 0x%zx  LIMIT 0x%zx\n", host, sz, sz - 1);

    VkMemoryHostPointerPropertiesEXT mhp = { .sType = VK_STRUCTURE_TYPE_MEMORY_HOST_POINTER_PROPERTIES_EXT };
    r = getProps(dev, VK_EXTERNAL_MEMORY_HANDLE_TYPE_HOST_ALLOCATION_BIT_EXT, host, &mhp);
    printf("vkGetMemoryHostPointerPropertiesEXT: %s  memoryTypeBits=0x%x\n", rstr(r), mhp.memoryTypeBits);
    if (r != VK_SUCCESS) return 1;

    VkPhysicalDeviceMemoryProperties mp; vkGetPhysicalDeviceMemoryProperties(chosen, &mp);
    int type = -1;
    for (uint32_t i = 0; i < mp.memoryTypeCount; i++)
        if ((mhp.memoryTypeBits & (1u << i)) &&
            (mp.memoryTypes[i].propertyFlags & VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT)) { type = (int)i; break; }
    if (type < 0) { printf("no HOST_VISIBLE type accepts this pointer\n"); return 1; }
    printf("using memoryType %d (heap %u)\n", type, mp.memoryTypes[type].heapIndex);

    VkImportMemoryHostPointerInfoEXT imp = {
        .sType = VK_STRUCTURE_TYPE_IMPORT_MEMORY_HOST_POINTER_INFO_EXT,
        .handleType = VK_EXTERNAL_MEMORY_HANDLE_TYPE_HOST_ALLOCATION_BIT_EXT,
        .pHostPointer = host };
    VkMemoryAllocateInfo mai = { .sType = VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
                                 .pNext = &imp, .allocationSize = sz, .memoryTypeIndex = (uint32_t)type };
    VkDeviceMemory mem;
    r = vkAllocateMemory(dev, &mai, NULL, &mem);
    printf("IMPORT %zu MiB: %s\n", sz / (1024*1024), rstr(r));
    if (r == VK_SUCCESS) { vkFreeMemory(dev, mem, NULL); printf("RESULT: PASS\n"); return 0; }
    printf("RESULT: FAIL\n");
    return 1;
}
