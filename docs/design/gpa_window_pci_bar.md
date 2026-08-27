# Design: GPA window as a 64-bit PCI BAR (#55)

## Problem

nvkvm maps host GPU memory into the guest by installing host buffers as KVM
memslots at **fixed guest physical addresses**:

| Window | GPA | Size | Install |
|---|---|---|---|
| shm (ioctl slots) | 0x10000000000 (1 TB) | 16 MiB | `memory_region` |
| mmap_win (legacy per-mmap) | 0x18000000000 (1.5 TB) | 16 GiB | per-mmap memslot |
| sparse window (bulk BAR/sysmem) | 0x20000000000 (2 TB) | 128 GiB | one `KVM_SET_USER_MEMORY_REGION` |

These GPAs are chosen to sit above any guest RAM, but nothing *reserves* them:
the guest firmware/kernel doesn't know they're occupied. For every real config
(guest RAM ≪ 1 TB) this is fine, and an interim guard in
`virtio_nvgpu_device_realize` now **fails loudly** if `ram_size >= 1 TB` so a
misconfiguration can't silently corrupt. But "squatting" remains fragile:
- A guest with ≥1 TB RAM, or firmware that places MMIO/PCI windows up high,
  could collide.
- The raw `kvm_add_memory_region` bypasses QEMU's `MemoryRegion`/PCI model, so
  the placement isn't visible to the guest's resource allocator.

## Proper fix

Expose the 128 GiB sparse window as a **64-bit, prefetchable PCI BAR** on the
virtio-nvgpu device, backed by the existing `MAP_NORESERVE` host mmap:

1. **QEMU**: `memory_region_init_ram_ptr(&nv->win_mr, ..., 128 GiB, va)` over the
   sparse mmap, then `pci_register_bar(pci_dev, bar, PCI_BASE_ADDRESS_MEM_TYPE_64
   | PCI_BASE_ADDRESS_MEM_PREFETCH, &nv->win_mr)`. QEMU's memory listener then
   installs the KVM memslot automatically when the guest programs the BAR — drop
   the manual `kvm_add_memory_region`. MAP_FIXED slice placement into the mmap is
   unchanged (the BAR is backed by the same host buffer).
2. **Guest firmware**: OVMF must assign a 128 GiB 64-bit BAR. This needs the
   machine's 64-bit PCI hole sized to fit it (`-machine ...,pci-hole64-size=256G`
   or a Q35 `pci-host` property). This is the main unknown — verify OVMF actually
   programs the BAR (SeaBIOS may not).
3. **Guest module**: read the BAR base with `pci_resource_start(pdev, bar)` and
   use it as `mmap_window_gpa_base` instead of the hardcoded constant. The guest
   already learns the GPA base dynamically (per-mmap `resp->gpa_base` and the
   virtio-config field), so this is mostly swapping the source.
4. **QEMU allocator**: `nvkvm_sparse_gpa_alloc` must hand out GPAs **inside the
   BAR's assigned range**, which is only known after the guest programs the BAR.
   Read it from the mapped `MemoryRegion`'s address (or a BAR-map callback) and
   set `sparse_gpa_base` then.

## Why deferred

This rewrites the exact memory path that carries the working CUDA/7B-inference
flow, and adds a guest-firmware dependency (OVMF 64-bit BAR assignment) that is
config-sensitive and easy to get subtly wrong. It is **high regression risk**
and warrants a dedicated branch with incremental, heavily-tested steps (BAR
visible → guest reads base → allocator uses it → drop the raw memslot), each
validated against matmul + `test_ioctl_fwd` + the 7B run. Until then the fixed
windows + the overlap guard are the safe, working state.

## Probe results (2026-05-30)

A probe (additive 64-bit prefetchable BAR 2, 128 GiB, MAP_NORESERVE, registered
in `virtio_nvgpu_pci_realize`, fixed-GPA path left intact) established:

- **Firmware assigns it.** SeaBIOS on the default `pc` machine + `-cpu host`
  (48 phys bits) placed the BAR at GPA `0x380000000000` (lspci:
  `Region 2: Memory at 380000000000 (64-bit, prefetchable) [size=128G]`). The
  VM boots normally. So the firmware-assignment concern is **resolved** — no
  OVMF/Q35/pci-hole64 tuning needed for a 128 GiB BAR here.
- **But the additive BAR regresses cuInit (→100).** With the BAR present *and*
  the fixed sparse window still installed, cuInit fails; reverting the BAR
  restores matmul. No QEMU/KVM error is logged and 56 TB fits 48 bits, so the
  likely cause is a **memslot conflict**: the sparse window is installed via a
  *raw* `kvm_add_memory_region` (manual slot id) while the BAR's RAM region is
  installed by QEMU's memory listener (auto slot id) — the two collide and the
  GPU-mapping window gets clobbered.

**Consequence for the migration:** the additive/incremental path is a dead end
(raw + BAR coexist → collision). The full migration must, in one step, **drop
the raw `kvm_add_memory_region`** and make the BAR's MemoryRegion the *sole*
backing for the window: point `nv->sparse_vmm_va` at the BAR buffer, set
`sparse_gpa_base` from the BAR's firmware-assigned address (read it from the
PCIDevice's `io_regions[2].addr` once the guest has programmed the BAR, or have
the guest read `pci_resource_start(pdev, 2)` and report it), and have the
allocator hand out `BAR_base + offset`. Then there is exactly one memslot (the
BAR's) and no collision. This is the high-risk single-shot change the rest of
this doc describes; it needs a dedicated, heavily-tested pass (matmul +
test_ioctl_fwd + 7B at each step). Probe code preserved for reference.

## Acceptance

- `lspci -vv` in the guest shows the 128 GiB 64-bit prefetchable BAR.
- The guest module derives the window base from the BAR (no hardcoded 2 TB).
- matmul, `test_ioctl_fwd` (48/48), and `run_llm_7b.sh` stay green.
- A guest with large RAM (e.g. 1.5 TB) boots and runs without collision.

## IMPLEMENTED (2026-05-30) — MMIO reservation BAR + raw KVM memslot

The chosen solution (per the project owner: "the only thing we need is that QEMU
doesn't pick our GPA; keep the raw KVM memory region"):

- **Reservation-only MMIO BAR** (`virtio_nvgpu_pci.c`): a 128 GiB 64-bit
  prefetchable BAR registered with `memory_region_init_io` (NOT `_ram_ptr`).
  Being MMIO, QEMU's listener creates **no** KVM memslot for it — so it does NOT
  collide with the window's own raw memslot (that collision was the probe's
  cuInit regression). Its sole job is to make the guest firmware ASSIGN + reserve
  a 128 GiB GPA range so QEMU/PCI never place anything else there.
- **Raw window installs at the BAR's GPA** (`nvkvm_mmap_host.c`): `sparse_init`
  now only mmaps the host buffer; `nvkvm_sparse_ensure()` lazily does the raw
  `KVM_SET_USER_MEMORY_REGION` at the firmware-assigned BAR base (read via a
  proxy callback `window_base_get`), or falls back to the fixed `NVKVM_SPARSE_GPA_BASE`
  if there's no BAR transport. The MMIO BAR is shadowed by this raw RAM memslot,
  so its accessors are never invoked.
- **No guest change**: `get_config` resolves the base and reports it in the
  existing `mmap_win_gpa`/`len` fields the guest already reads for GPA validation.
  The guest gets actual GPAs from QEMU responses as before.
- The legacy `mmap_win` (1.5 TB) is dead (both MMAP branches use the sparse
  window), so only the one window needed rebasing.

**Verified on RTX 3060:** lspci shows `Region 2: 128G @ 0x380000000000`;
`nvkvm_sparse_ensure: 128 GiB at GPA=0x380000000000 slot=64` (firmware base, not
the 2 TB fallback); matmul + full 7B inference (21 tok/s) both PASS. The fixed
GPA is now only a fallback for a BAR-less transport.
