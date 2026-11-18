// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
// gap_vkprio.c: which Vulkan queue global priorities does this machine offer, and does vkCreateDevice accept each one (docs/design/V3_GPUPV_GAP_MATRIX.md).
// STATUS: RESEARCH, 2026-11-18. gcc -o gap_vkprio gap_vkprio.c -ldl; no Vulkan header, libvulkan is opened with dlopen and the few structures used are declared here
// (structure type values from the Vulkan registry). Prints one line per observation:
//   VKPRIO device=<name> queue_family=<i> flags=<hex> count=<n> reported=<LOW,MEDIUM,HIGH,REALTIME>
//   VKPRIO create family=<i> priority=<NAME> result=<VkResult number>
// The exit status is 0 when the program ran (a refusal is an observation, not an error).
#include <dlfcn.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef int32_t VkResult;
typedef void *VkInstance, *VkPhysicalDevice, *VkDevice;
typedef struct { int32_t sType; const void *pNext; const char *pApplicationName; uint32_t applicationVersion; const char *pEngineName; uint32_t engineVersion; uint32_t apiVersion; } VkApplicationInfo;
typedef struct { int32_t sType; const void *pNext; uint32_t flags; const VkApplicationInfo *pApplicationInfo; uint32_t enabledLayerCount; const char *const *ppEnabledLayerNames; uint32_t enabledExtensionCount; const char *const *ppEnabledExtensionNames; } VkInstanceCreateInfo;
typedef struct { int32_t sType; void *pNext; uint32_t priorityCount; int32_t priorities[16]; } VkQueueFamilyGlobalPriorityProperties;
typedef struct { uint32_t queueFlags, queueCount, timestampValidBits; uint32_t minImageTransferGranularity[3]; } VkQueueFamilyProperties;
typedef struct { int32_t sType; void *pNext; VkQueueFamilyProperties queueFamilyProperties; } VkQueueFamilyProperties2;
typedef struct { int32_t sType; const void *pNext; int32_t globalPriority; } VkDeviceQueueGlobalPriorityCreateInfo;
typedef struct { int32_t sType; const void *pNext; uint32_t flags, queueFamilyIndex, queueCount; const float *pQueuePriorities; } VkDeviceQueueCreateInfo;
typedef struct { int32_t sType; const void *pNext; uint32_t flags, queueCreateInfoCount; const VkDeviceQueueCreateInfo *pQueueCreateInfos; uint32_t enabledLayerCount; const char *const *ppEnabledLayerNames;
                 uint32_t enabledExtensionCount; const char *const *ppEnabledExtensionNames; const void *pEnabledFeatures; } VkDeviceCreateInfo;
typedef struct { uint32_t apiVersion, driverVersion, vendorID, deviceID, deviceType; char deviceName[256]; uint8_t rest[1024]; } VkPhysicalDevicePropertiesHead;
static const int32_t PRIO[] = {128, 256, 512, 1024};
static const char *PN[] = {"LOW", "MEDIUM", "HIGH", "REALTIME"};
static const char *name_of(int32_t p) { for (int i = 0; i < 4; i++) if (PRIO[i] == p) return PN[i]; return "OTHER"; }

int main(void) {
  void *h = dlopen("libvulkan.so.1", RTLD_NOW);
  if (!h) { puts("VKPRIO error=no_libvulkan"); return 0; }
  VkResult (*createInstance)(const VkInstanceCreateInfo *, const void *, VkInstance *) = dlsym(h, "vkCreateInstance");
  void *(*getProc)(VkInstance, const char *) = dlsym(h, "vkGetInstanceProcAddr");
  if (!createInstance || !getProc) { puts("VKPRIO error=no_symbols"); return 0; }
  VkApplicationInfo app = {0, NULL, "gap_vkprio", 1, "gap", 1, (1u << 22) | (3u << 12)};
  VkInstanceCreateInfo ici = {1, NULL, 0, &app, 0, NULL, 0, NULL};
  VkInstance inst; VkResult r = createInstance(&ici, NULL, &inst);
  if (r) { printf("VKPRIO error=vkCreateInstance result=%d\n", r); return 0; }
  VkResult (*enumPhys)(VkInstance, uint32_t *, VkPhysicalDevice *) = getProc(inst, "vkEnumeratePhysicalDevices");
  void (*getProps)(VkPhysicalDevice, VkPhysicalDevicePropertiesHead *) = getProc(inst, "vkGetPhysicalDeviceProperties");
  void (*getQ2)(VkPhysicalDevice, uint32_t *, VkQueueFamilyProperties2 *) = getProc(inst, "vkGetPhysicalDeviceQueueFamilyProperties2");
  VkResult (*createDevice)(VkPhysicalDevice, const VkDeviceCreateInfo *, const void *, VkDevice *) = getProc(inst, "vkCreateDevice");
  void (*destroyDevice)(VkDevice, const void *) = getProc(inst, "vkDestroyDevice");
  uint32_t n = 0; enumPhys(inst, &n, NULL);
  VkPhysicalDevice *pd = calloc(n ? n : 1, sizeof *pd); enumPhys(inst, &n, pd);
  for (uint32_t d = 0; d < n; d++) {
    VkPhysicalDevicePropertiesHead pr; memset(&pr, 0, sizeof pr); getProps(pd[d], &pr);
    uint32_t nq = 0; getQ2(pd[d], &nq, NULL);
    VkQueueFamilyGlobalPriorityProperties gp[16]; VkQueueFamilyProperties2 qp[16]; if (nq > 16) nq = 16;
    for (uint32_t i = 0; i < nq; i++) { memset(&gp[i], 0, sizeof gp[i]); gp[i].sType = 1000388001; memset(&qp[i], 0, sizeof qp[i]); qp[i].sType = 1000059005; qp[i].pNext = &gp[i]; }
    getQ2(pd[d], &nq, qp);
    for (uint32_t i = 0; i < nq; i++) {
      char rep[96] = ""; for (uint32_t k = 0; k < gp[i].priorityCount && k < 16; k++) { strcat(rep, k ? "," : ""); strcat(rep, name_of(gp[i].priorities[k])); }
      printf("VKPRIO device=%s queue_family=%u flags=0x%x count=%u reported=%s\n", pr.deviceName, i, qp[i].queueFamilyProperties.queueFlags, qp[i].queueFamilyProperties.queueCount, rep[0] ? rep : "-");
    }
    const char *ext[] = {"VK_KHR_global_priority"};
    for (uint32_t i = 0; i < nq; i++) for (int p = 0; p < 4; p++) {
      float one = 1.0f; VkDeviceQueueGlobalPriorityCreateInfo gi = {1000174000, NULL, PRIO[p]};
      VkDeviceQueueCreateInfo qi = {2, &gi, 0, i, 1, &one};
      VkDeviceCreateInfo di = {3, NULL, 0, 1, &qi, 0, NULL, 1, ext, NULL};
      VkDevice dev = NULL; VkResult cr = createDevice(pd[d], &di, NULL, &dev);
      printf("VKPRIO create family=%u priority=%s result=%d\n", i, PN[p], cr);
      if (!cr && dev) destroyDevice(dev, NULL);
    }
  }
  return 0;
}
