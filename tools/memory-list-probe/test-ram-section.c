/* Compile the ACTUAL production predicate with a minimal QEMU type fixture. */
#include <assert.h>
#include <stdbool.h>
#include <stddef.h>
struct MemoryRegion { bool ram, readonly, ram_device; void *ram_block; };
typedef struct { struct MemoryRegion *mr; bool readonly; } MemoryRegionSection;
static bool memory_region_is_ram(struct MemoryRegion *mr) { return mr->ram; }
static bool memory_region_is_ram_device(struct MemoryRegion *mr) { return mr->ram_device; }
#include "../../qemu/hw/misc/kf3/kf3_guest_ram.h"
int main(void)
{
    struct MemoryRegion mr = { .ram = true, .ram_block = &mr };
    MemoryRegionSection old = { .mr = &mr, .readonly = false };
    MemoryRegionSection ro_alias = { .mr = &mr, .readonly = true };
    assert(kf3_is_guest_ram(&old));
    assert(!kf3_is_guest_ram(&ro_alias)); /* inherited read-only alias */
    mr.readonly = true; /* changed live MR before old section's region_del */
    assert(kf3_is_guest_ram(&old)); /* MUST delete old writable registration */
    assert(!kf3_is_guest_ram(&ro_alias)); /* MUST NOT register new read-only view */
    mr.ram_device = true; assert(!kf3_is_guest_ram(&old));
    mr.ram_device = false; mr.ram_block = NULL; assert(!kf3_is_guest_ram(&old));
    mr.ram_block = &mr; mr.ram = false; assert(!kf3_is_guest_ram(&old));
    return 0;
}
