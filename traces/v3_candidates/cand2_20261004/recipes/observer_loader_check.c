/* Local observer positive control, not GPU evidence. Compile with FAKE_LOADER
 * as libvulkan.so, without it as the client. It checks explicit-handle lookup,
 * ELF-preempted function pointers, exact call count and unchanged return codes. */
#include <assert.h>
#include <stdint.h>
#include <string.h>
#include <vulkan/vulkan.h>
#ifdef FAKE_LOADER
VKAPI_ATTR VkResult VKAPI_CALL vkQueuePresentKHR(VkQueue q, const VkPresentInfoKHR *p)
{
    (void)p;
    return (uintptr_t)q == 7 ? VK_SUBOPTIMAL_KHR : VK_ERROR_DEVICE_LOST;
}
VKAPI_ATTR PFN_vkVoidFunction VKAPI_CALL vkGetDeviceProcAddr(VkDevice d, const char *n)
{
    (void)d;
    return strcmp(n, "vkQueuePresentKHR") == 0 ? (PFN_vkVoidFunction)vkQueuePresentKHR : NULL;
}
VKAPI_ATTR PFN_vkVoidFunction VKAPI_CALL vkGetInstanceProcAddr(VkInstance i, const char *n)
{
    (void)i;
    return strcmp(n, "vkGetDeviceProcAddr") == 0 ? (PFN_vkVoidFunction)vkGetDeviceProcAddr : NULL;
}
#else
#include <dlfcn.h>
#include <stdio.h>
int main(int argc, char **argv)
{
    assert(argc == 2);
    void *lib = dlopen(argv[1], RTLD_NOW | RTLD_LOCAL);
    assert(lib);
    PFN_vkGetInstanceProcAddr gi = (PFN_vkGetInstanceProcAddr)dlsym(lib, "vkGetInstanceProcAddr");
    assert(gi);
    PFN_vkGetDeviceProcAddr gd = (PFN_vkGetDeviceProcAddr)gi(NULL, "vkGetDeviceProcAddr");
    assert(gd);
    PFN_vkQueuePresentKHR present = (PFN_vkQueuePresentKHR)gd(NULL, "vkQueuePresentKHR");
    assert(present);
    for (unsigned i = 0; i < 489; ++i) {
        uintptr_t q = i < 480 ? 7 : 9;
        assert(present((VkQueue)q, NULL) == (q == 7 ? VK_SUBOPTIMAL_KHR : VK_ERROR_DEVICE_LOST));
    }
    puts("OBSERVER_LOADER_CONTROL PASS calls=489 errors=9 return_values_unchanged");
    return 0;
}
#endif
