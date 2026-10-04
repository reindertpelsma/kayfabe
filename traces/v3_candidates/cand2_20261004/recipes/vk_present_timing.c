/* Observe the unmodified vkcube's successful vkQueuePresentKHR returns.
 * No wait, frame submission or return code is changed. The first 16 presents
 * are excluded so loader/shader startup and initial queue fill are not timed.
 * Test instrumentation only; build as a preload library in the guest. */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <vulkan/vulkan.h>

static pthread_once_t initialized = PTHREAD_ONCE_INIT;
static pthread_mutex_t lock = PTHREAD_MUTEX_INITIALIZER;
static PFN_vkQueuePresentKHR present_next;
static PFN_vkGetInstanceProcAddr instance_next;
static PFN_vkGetDeviceProcAddr device_next;
static unsigned long long count, errors;
static double first, last;

static void *lookup(void *handle, const char *name)
{
    /* This fixture runs on x86_64 glibc. Calling the versioned libc entry
     * avoids recursion through our dlsym observer below. */
    typedef void *(*Dlsym)(void *, const char *);
    Dlsym next = (Dlsym)dlvsym(RTLD_NEXT, "dlsym", "GLIBC_2.2.5");
    if (!next) {
        abort();
    }
    return next(handle, name);
}

void *dlsym(void *handle, const char *name)
{
    void *found = lookup(handle, name);
    Dl_info identity;
    /* Ubuntu 24.04's vkcube dlopens the loader and looks up its entry point
     * on that explicit handle. Ordinary ELF interposition alone sees none
     * of its calls. Intercept only entry points from the actual Vulkan loader,
     * never an ICD's entry point or a function from an unrelated library. */
    if (!found || !dladdr(found, &identity) || !identity.dli_fname ||
        !strstr(identity.dli_fname, "libvulkan.so")) {
        return found;
    }
    if (strcmp(name, "vkGetInstanceProcAddr") == 0) {
        instance_next = (PFN_vkGetInstanceProcAddr)found;
        fprintf(stderr, "VK_PRESENT_OBSERVER_BIND loader=%s\n", identity.dli_fname);
        return (void *)vkGetInstanceProcAddr;
    }
    if (strcmp(name, "vkGetDeviceProcAddr") == 0) {
        device_next = (PFN_vkGetDeviceProcAddr)found;
        return (void *)vkGetDeviceProcAddr;
    }
    if (strcmp(name, "vkQueuePresentKHR") == 0) {
        present_next = (PFN_vkQueuePresentKHR)found;
        return (void *)vkQueuePresentKHR;
    }
    return found;
}

static void resolve_present(void)
{
    if (!present_next) {
        present_next = (PFN_vkQueuePresentKHR)lookup(RTLD_NEXT, "vkQueuePresentKHR");
    }
    if (!present_next) {
        fprintf(stderr, "VK_PRESENT_OBSERVER_ERROR no loader export\n");
        abort();
    }
}

VKAPI_ATTR VkResult VKAPI_CALL vkQueuePresentKHR(VkQueue queue,
                                                const VkPresentInfoKHR *info)
{
    struct timespec now;
    pthread_once(&initialized, resolve_present);
    VkResult result = present_next(queue, info);
    clock_gettime(CLOCK_MONOTONIC, &now);
    double ended = (double)now.tv_sec + (double)now.tv_nsec / 1e9;
    pthread_mutex_lock(&lock);
    count++;
    if (result != VK_SUCCESS && result != VK_SUBOPTIMAL_KHR) {
        errors++;
    }
    if (count == 16) {
        first = ended;
    }
    last = ended;
    pthread_mutex_unlock(&lock);
    return result;
}

VKAPI_ATTR PFN_vkVoidFunction VKAPI_CALL vkGetDeviceProcAddr(VkDevice device,
                                                           const char *name)
{
    if (!device_next) {
        device_next = (PFN_vkGetDeviceProcAddr)lookup(RTLD_NEXT, "vkGetDeviceProcAddr");
    }
    PFN_vkVoidFunction found = device_next(device, name);
    if (found && strcmp(name, "vkQueuePresentKHR") == 0) {
        present_next = (PFN_vkQueuePresentKHR)found;
        return (PFN_vkVoidFunction)vkQueuePresentKHR;
    }
    return found;
}

VKAPI_ATTR PFN_vkVoidFunction VKAPI_CALL vkGetInstanceProcAddr(VkInstance instance,
                                                             const char *name)
{
    if (!instance_next) {
        instance_next = (PFN_vkGetInstanceProcAddr)lookup(RTLD_NEXT, "vkGetInstanceProcAddr");
    }
    PFN_vkVoidFunction found = instance_next(instance, name);
    if (found && strcmp(name, "vkQueuePresentKHR") == 0) {
        present_next = (PFN_vkQueuePresentKHR)found;
        return (PFN_vkVoidFunction)vkQueuePresentKHR;
    }
    if (found && strcmp(name, "vkGetDeviceProcAddr") == 0) {
        device_next = (PFN_vkGetDeviceProcAddr)found;
        return (PFN_vkVoidFunction)vkGetDeviceProcAddr;
    }
    if (found && strcmp(name, "vkGetInstanceProcAddr") == 0) {
        return (PFN_vkVoidFunction)vkGetInstanceProcAddr;
    }
    return found;
}

__attribute__((destructor)) static void report(void)
{
    double elapsed = count > 16 ? last - first : 0;
    double fps = elapsed > 0 ? (double)(count - 16) / elapsed : 0;
    fprintf(stderr, "VK_PRESENT_TIMING {\"count\":%llu,\"skip\":16,"
            "\"seconds\":%.9f,\"fps\":%.6f,\"errors\":%llu}\n",
            count, elapsed, fps, errors);
}
