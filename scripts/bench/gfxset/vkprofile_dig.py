#!/usr/bin/env python3
"""vkprofile_dig.py <VP_VULKANINFO_*.json> — digest a vulkaninfo capability profile section by section,
so the guest's Vulkan device can be compared with bare metal's field for field.
⊘ Removed before digesting (they identify the INSTANCE, not the capability, and legitimately differ):
UUIDs/LUIDs/node masks, PCI bus info, and the memory heaps (the guest's FB is a VM parameter) — the
heaps are printed as GSET_VAL instead. The normalized JSON is written beside the input for a diff."""
import hashlib, json, os, sys

DROP = {"deviceUUID", "driverUUID", "deviceLUID", "deviceLUIDValid", "deviceNodeMask", "pciDomain", "pciBus",
        "pciDevice", "pciFunction", "VkPhysicalDevicePCIBusInfoPropertiesEXT", "VkPhysicalDeviceMemoryProperties",
        "VkPhysicalDeviceIDProperties", "VkPhysicalDeviceIDPropertiesKHR"}

def scrub(x):
    if isinstance(x, dict):
        return {k: scrub(v) for k, v in x.items() if k not in DROP}
    if isinstance(x, list):
        return [scrub(v) for v in x]
    return x

def find(x, key):
    if isinstance(x, dict):
        if key in x:
            return x[key]
        for v in x.values():
            r = find(v, key)
            if r is not None:
                return r
    return None

p = sys.argv[1]
j = json.load(open(p))
dev = find(j, "device") or {}
side = os.environ.get("SIDE", "?"); item = os.environ.get("ITEM", "?")
norm = {}
for sec in ("extensions", "features", "properties", "formats", "queueFamiliesProperties"):
    v = scrub(dev.get(sec))
    norm[sec] = v
    d = hashlib.md5(json.dumps(v, sort_keys=True).encode()).hexdigest()[:16] if v is not None else "ABSENT"
    print(f"GSET_DIG side={side} item={item} key=vk_{sec} val={d}")
print(f"GSET_VAL side={side} item={item} key=vk_ext_count val={len(dev.get('extensions') or {})}")
mem = find(dev, "VkPhysicalDeviceMemoryProperties") or {}
heaps = [f"{h.get('size', 0) // (1 << 20)}MiB/{h.get('flags')}" for h in (mem.get("memoryHeaps") or [])]
print(f"GSET_VAL side={side} item={item} key=vk_heaps val={','.join(heaps) or '-'}")
json.dump(norm, open(os.path.join(os.path.dirname(p) or ".", "vkprofile_normalized.json"), "w"), sort_keys=True, indent=1)
