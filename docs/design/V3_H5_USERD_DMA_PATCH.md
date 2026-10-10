# V3 — H5: the USERD-in-sysmem DMA-address fix (host `nvidia.ko` patch)

**STATUS: LIVE (patch written, BUILD-ONLY verified at 595.91.07 for kernel 7.0.0-34; nothing loaded, installed or run on any GPU), 2026-10-10.**
Implements `DELEGATED_DECISIONS_20261010.md` D7. Plan and candidates: `V3_HOST_PATCH_LIST.md` §4.5 (which this
page corrects and refines). Stock fallback: the USERD relay (`V3_USERD_RELAY.md`), unchanged and still the default.
Labels as in the patch list: **[read]** source, **[measured]** a command run for this page or a committed trace,
**[reasoned]**, **[inferred]**, **[proposed]**.

## 0. Summary

- **What is broken.** On a Volta-to-Ada host GPU, host RM refuses a Passthrough twin whose USERD is guest system memory,
  because the I/O virtual address (IOVA) the IOMMU gave that memory does not fit the 40-bit USERD pointer of the runlist
  entry (`kchannelCreateUserdMemDesc_GV100: physical addr size ... is incorrect!`, run 61).
- **What the patch does.** It adds one `nvidia.ko` escape that runs `NV_ESC_RM_ALLOC_MEMORY` (nothing else) with the GPU's
  DMA mask lowered to N bits (N >= 40) **for that one call**, so the OS descriptor created by the call gets IOVAs below 2^N.
  Plus a query escape (the H0 probe). 4 files, 376 lines added, no RM-core change.
- **Option chosen: (a), per allocation, opted in by the caller.** Not USERD-only in the kernel (it cannot be, §2.3), but
  per object, and kayfabe decides which objects. Option (b) (cap the device mask) is not shipped.
- **The software size check is untouched** (relaxing it would truncate the pointer; D7 forbids it).
- **Nothing is mapped into any GPU VA space** by the patch. It maps nothing at all.
- **Distribution: the patch applies to the packaged DKMS tree** (`/usr/src/nvidia-595.91.07`, prebuilt RM core included)
  because it touches only the kernel-interface layer. This corrects the patch list's "full ogkm rebuild tier" for H5 (§4).

## 1. Scope by GPU family (new, [read])

The wall exists only where the USERD pointer is 40 bits wide. HAL choice: `src/nvidia/generated/g_kernel_channel_nvoc.c:1149-1169`
(595.84 tree).

| family (chip HAL) | check function | field | address bits | wall at a 47-bit IOVA |
|---|---|---|---|---|
| Turing TU102..TU117 | `kchannelIsUserdAddrSizeValid_GV100` | `NV_PPBDMA_USERD_ADDR` 31:9 + `_HI_ADDR` 7:0 (`dev_pbdma.h:26-27`) | 40 | yes |
| Ampere GA100..GA107, Ada AD102..AD107 | `_GA100` | `NV_RAMRL_ENTRY_CHAN_USERD_PTR_LO` 31:8 + `_HI_HW` 7:0 (`dev_ram.h:27-28`, ga100) | 40 | yes (the trusted host's RTX 4070, AD104) |
| Hopper, Blackwell (GH100 and the default branch) | `_GH100` | `_PTR_LO` 31:8 + `_PTR_HI_HW` 19:0 (`dev_ram.h:27-28`, gh100) | 52 | **no** (system physical address width is 52) |
| GB10B / GB20B / GB20C | `_GB10B` | as GH100 | 52 | no |

So H5 is needed on Turing/Ampere/Ada hosts only; on Hopper and newer the stock path would adopt the guest USERD without it.
The IOVA width itself is RM's `NV_CHIP_EXTENDED_SYSTEM_PHYSICAL_ADDRESS_BITS` (47 for TU102 and the Ampere/Ada parts it covers,
52 for Hopper; `hwproject.h:27`), set as the device DMA mask by `osDmaSetAddressSize` -> `nv_set_dma_address_size` at adapter
init (`kern_mem_sys_gm107.c:322`, `nv.c:3275`). That matches the measured IOVA `0x7ff9_f969_b000` (47 bits).
**The width to ask for (40) must be derived from the family's USERD field, never typed in** (`CLAUDE.md`, derive-never-capture):
the kayfabe follow-up reads `PTR_LO` + `PTR_HI_HW` from ogkm-generated register definitions.

## 2. What was located (595.91.07 source; line numbers from that tree unless a 595.84 path is given)

### 2.1 The refusal

| step | where | label |
|---|---|---|
| USERD offset shift = 9 (512-byte slot) | `kernel_fifo_gm107.c:1080-1091`, `dev_ram.h:49` (`NV_RAMUSERD_BASE_SHIFT 9`) | [read]; run 61's `userAddrLo=0x007cb4d8` == `0xf969b000 >> 9` [measured] |
| `userdAddr = memdescGetPhysAddr(pUserdMemDescForSubDev, AT_GPU, offset)` | `kernel_channel_gv100.c:204` | [read] |
| size check, `NV_ASSERT(0)`, `NV_ERR_INVALID_ADDRESS` | `kernel_channel_gv100.c:212-219` | [read]; same text in the host's dmesg [measured, run 61] |
| `AT_GPU` address of a sysmem memdesc = its IOMMU mapping when one exists in the GPU's IOVA space | `mem_desc.c` `memdescGetPteArrayForGpu` ~3127-3190 (`memdescGetIommuMap(... iovaspaceId)`) | [read] |

### 2.2 Where that IOVA is chosen (the call path of an OS descriptor), all [read] unless noted

1. `osdescConstruct_IMPL` (`mem_mgr/os_desc_mem.c`) -> `osCreateMemFromOsDescriptor` (`arch/nvalloc/unix/src/osmemdesc.c:58`)
   -> `osCreateOsDescriptorFromPageArray` (`:323`) -> `osCreateMemdescFromPages` (`:176`): pins the pages
   (`nv_register_user_pages`), then **`memdescMapIommu(pMemDesc, pGpu->busInfo.iovaspaceId)` (`:296`) inside the same call**.
2. `memdescMapIommu` (`mem_desc.c:4292`) -> `iovaspaceAcquireMapping` (`io_vaspace.c:482`) -> `_iovaspaceCreateMapping`
   (`:398`) -> `osIovaMap` (`os.c:3530`; `:3685`) -> `nv_dma_map_alloc(osGetDmaDeviceForMemDesc(...), ...)`.
3. `nv_dma_map_alloc` (`kernel-open/nvidia/nv-dma.c:594`) -> `nv_dma_map_pages` (`:466`) -> `nv_dma_map_contig` (`dma_map_page_attrs`, `:85`)
   or `nv_dma_map_scatterlist` (`dma_map_sg`, `:246`). The only check afterwards is against `addressable_range.limit`
   (`nv-dma.c:72-74, 94-105, 367-377`), which `nv_set_dma_address_size` sets to the same mask.
4. The kernel: `iommu_dma_map_page` and `iommu_dma_map_sg` call `iommu_dma_alloc_iova(domain, size, dma_get_mask(dev), dev)`
   (`drivers/iommu/dma-iommu.c` v7.0, lines 1206, 1484, 1798, 2177), which does `alloc_iova_fast(iovad, len, limit >> shift, true)`:
   **the highest free range at or below the device DMA mask** (`:780-796`; the "try 32 bits first" step is disabled once it
   fails once, `pci_32bit_workaround`). So the mask read **at map time** is the only knob. This replaces the patch list's
   "[inferred]" for the IOVA allocator.
5. **There is no per-mapping limit in the DMA API.** RM already works around that exactly once, the same way this patch does:
   `kern_mem_sys_gm107.c:300-322` lowers the DMA address size with `osDmaSetAddressSize` around the allocation of the sysmem
   flush buffer and restores it, with the comment "admittedly hacky and only safe during GPU initialization ... making it a
   part of the memdesc APIs would be cleaner". H5 is that technique, per call and serialised.

### 2.3 Consequences for "option (a)" (correction to the patch list)

- **The IOVA is assigned when the OS descriptor is created, not when a channel names it as USERD** (step 1, inside the
  `RM_ALLOC_MEMORY` call). RM learns "this memory is a USERD" later, at channel creation, when the address is already fixed.
  A kernel-side "USERD memory only" constraint would therefore have to **re-map** the memory at that moment.
- That is not available in the RM core: `memdescCreateSubMem`/`_iovaspaceCreateSubmapping` (`io_vaspace.c:258-340`) only
  take a *subset of the root descriptor's mapping* (`pSubMapping->iovaArray[j] = pRootIovaMapping->iovaArray[i]`), one mapping
  per IOVA space per root (`memdescAddIommuMap`, `mem_desc.c:4223`). A new independent root mapping for a USERD page would be
  an RM-core change (new memdesc over the same pinned pages, lifetime tied to the channel) that needs the full-source
  rebuild tier, and is not what an address-size fix should be. **[proposed: not done.]**
- So option (a) is implemented as: **a per-allocation window, requested by the creator of the OS descriptor.** The narrow
  use is for kayfabe to describe **the guest's USERD pages** (a small descriptor per declared USERD page range) through the
  window and everything else (the rest of guest RAM) without it. The patch itself is generic; the narrowness is kayfabe's.
- **Today kayfabe has one OS descriptor over the whole guest RAM** (`guest_ram_object`, `kf-qemu/src/mem.rs:2429`; 8 GiB took
  3.53 s to register, `display.rs:3730` [measured]). Two follow-up shapes (§7): F2, a dedicated small descriptor per USERD
  page (narrow, preferred); F1, build the one RAM descriptor through the window (simple; all guest RAM then has IOVA below
  2^40, so it needs the guest RAM to fit 1 TiB of IOVA, which it does for any guest this project runs, and is *not* USERD-only).

### 2.4 Probe path (patch list §3.1 corrected)

`nvidia_ioctl` first calls `nv_validate_ioctls(cmd)` (`nv.c:2404-2466` in 595.91.07); an escape that is in neither the RM nor
the frontend table returns `NV_ERR_INVALID_ARGUMENT` -> **`-EINVAL`, and writes `NVRM:unknown NVRM ioctl command: 0x..` to
the kernel log at error level** [read]. The patch list said an unknown escape "falls to `rm_ioctl` and returns `-EINVAL`
(`nv.c:2846-2856`)"; the result (-EINVAL) is right, the path is not, and **a stock host logs one line per probe**. Kayfabe
probes once per VMM start per GPU. (A zero-noise alternative, a sysfs parameter read, is possible; the escape is what D7 and
the patch list specify, so it is the one built.)

## 3. The patch

`tools/host_patches/h5_userd_dma/patch/nvidia_h5_userd_dma_595.91.07.patch`, `-p1` from the `kernel-open/` directory of an
`open-gpu-kernel-modules` checkout **or from the root of a packaged DKMS tree** (same layout). Four files:

| file | change |
|---|---|
| `common/inc/nv-kf-host-patch.h` (new) | the ABI: escape numbers `NV_IOCTL_BASE+40/+41`, the two structs, feature bit, verdicts |
| `common/inc/nv-kf-dma-window.h` (new) | `nv_kf_dma_window_plan()`: the decision, free of kernel types (unit-tested in userspace) |
| `nvidia/nv.c` | module parameter `kf_dma_window` (0644), a mutex, `nv_kf_query()`, `nv_kf_alloc_memory_dma_window()`, two `nv_validate_ioctls` table rows, two `switch` cases |
| `nvidia/nvidia.Kbuild` | `NV_CONFTEST_FUNCTION_COMPILE_TESTS += iommu_is_dma_domain` (so `nvidia.ko` alone can use it) |

### 3.1 `NV_ESC_KF_QUERY` (H0)

In: `flags` = 0. Out: `abi` = 1, `features` (bit 0 = H5, set only while `kf_dma_window` = 1), `feature_abi[bit]`. Open on the
control and GPU nodes, like `NV_ESC_CHECK_VERSION_STR`. Stock module: `-EINVAL`. Patched but `kf_dma_window=0`: answers with an empty
mask (the rule "an installed-but-disabled patch answers the probe as such", `V3_HOST_PATCH_LIST.md` §3.2).

### 3.2 `NV_ESC_KF_ALLOC_MEMORY_DMA_WINDOW` (H5)

In: `abi` = 1, `dma_bits` in [40, 63], `flags` = 0, `inner_size` (<= 512), `inner_ptr` (the user pointer of an ordinary
`NV_ESC_RM_ALLOC_MEMORY` argument). Out: `applied`, `verdict`, `mask_bits`; the inner argument is written back
(RM's `status` is inside it). GPU nodes only. Behaviour, in order:

1. Validate the arguments and the inner size exactly as an ordinary `RM_ALLOC_MEMORY` (`nv_validate_ioctls`). The inner command
   number is **fixed in the kernel** (`0x27`); the caller cannot choose another RM escape.
2. Take `nv_kf_dma_window_lock` (interruptible). Read `dma_get_mask(dev)` and whether the device is behind a translating
   IOMMU domain (`iommu_get_domain_for_dev` + `iommu_is_dma_domain`, the same calls `nvidia-uvm` uses).
3. `nv_kf_dma_window_plan`: refuse bad bits (`-EINVAL`) or a disabled feature (`-EOPNOTSUPP`), run the call **unchanged** when
   there is no translating domain (`NOT_TRANSLATED`: an identity or absent IOMMU makes the DMA address the physical address,
   and a lower mask could make the DMA layer bounce) or the mask is already narrow enough (`ALREADY_NARROW`), else lower the mask
   with `dma_set_mask(dev, (1<<bits)-1)`.
4. Run `rm_ioctl(... NV_ESC_RM_ALLOC_MEMORY ...)` on the copied argument, restore the saved mask, unlock, copy the argument
   back. `dma_set_mask` failing leaves the call unchanged (`MASK_REFUSED`).

It never raises a mask and never touches `addressable_range` (so RM's own post-map check still compares against the full
width). The device mask is a device-wide setting: **DMA mappings that other threads or `nvidia-uvm` make while the window is
open also get IOVAs below 2^bits.** That is harmless for correctness (every consumer accepts a narrower address) and bounded in
time by the one `RM_ALLOC_MEMORY` call (§2.3: ~3.5 s for an 8 GiB guest-RAM descriptor, which is the F1 worst case; F2's
descriptors are microseconds). `nv_set_dma_address_size` runs at adapter init only and an open file keeps the adapter
initialised, so it cannot interleave **[read: `nv.c:3275`, `kern_mem_sys_gm107.c:300-322`; inferred for GPU reset paths, which
re-run init and are the first thing the hardware test must try]**.

## 4. Distribution, corrected: no rebuild tier for H5 [measured]

The patch list's flavour constraint (§9.1: "RM-core patches cannot be applied to the packaged tree") stays true for H6/H7. For
H5 it does not apply, because H5 changes only `kernel-open/nvidia/`:

- The trusted host's `/usr/src/nvidia-595.91.07` (package `nvidia-dkms-595-open`) was compared, read-only, with the GitHub tag's
  `kernel-open/`: **identical except `nvidia/nv-kernel.o_binary`, `nvidia-modeset/nv-modeset-kernel.o_binary`, `patches/` and `dkms.conf`**
  (`diff -rq`, 2026-10-10). The prebuilt RM core is linked unchanged.
- `patch -p1 --dry-run` is clean on that tree, on the tag's `kernel-open/`, and on 595.84's `kernel-open/` (offsets only).
- The build links against the packaged `nv-kernel.o_binary`; result in §8. Export CRCs of the patched `nvidia.ko` (`__kcrctab`,
  `__kcrctab_gpl`) are **byte-identical** to the host's stock module, so a stock `nvidia-uvm`/`-modeset`/`-drm` would load against
  it; the one import CRC added is `iommu_get_domain_for_dev` (GPL, already imported by `nvidia-uvm`, present in the running kernel).

Packaging proposal [proposed, not built]: a second DKMS source package `kayfabe-nvidia-patches/<tag>` whose `PRE_BUILD` copies
`/usr/src/nvidia-<tag>`, runs `patch --dry-run` and then the patch (a failed dry run skips the patched build and leaves the stock
modules, `V3_HOST_PATCH_LIST.md` §9.2), and builds `nvidia`, `nvidia-uvm`, `nvidia-modeset`, `nvidia-drm` together with the packaged
`dkms.conf` make line. `box/build_h5.sh` is that build without the install. The module **version string is unchanged**, so
userspace driver libraries keep matching; the tier is told apart only by the probe (`V3_HOST_PATCH_LIST.md` §3.1). Because both packages
would produce `nvidia.ko` for the same version, the admin installs one or the other (`dkms remove` of the stock registration first).

## 5. Hostile-guest review

**New privilege.** An unprivileged process that can already open the GPU node and allocate an OS descriptor over its own memory
(it needs nothing else: `RM_ALLOC_MEMORY` of `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` is unprivileged and was used by every Passthrough
birth) gains one thing: **to choose the IOVA ceiling (at least 2^40) of the mapping RM makes for that allocation.** It cannot choose
an address, only a ceiling at or above 1 TiB, and the choice cannot make anything reachable that was not reachable before: an IOVA
is a name inside the GPU's own IOMMU domain for pages the caller already pinned and described.

Bounds, each [read] in the patch:

- `dma_bits` in [40, 63]; the floor is the USERD field, and 2^40 of IOVA (1 TiB) cannot be exhausted by one client the way a
  4 GiB window could. A request not narrower than the current mask is a no-op.
- The inner command is the constant `0x27`; `inner_size` <= 512 and passes the ordinary size validation; no pointer from the
  caller is used except through `copy_from_user`/`copy_to_user` of that one argument.
- The mask is only ever lowered, restored on every exit path of the same function, under a mutex that only this escape takes
  (lock order: window lock, then RM locks; nothing takes the window lock while holding an RM lock).
- It adds no RM verb, maps nothing into any GPU VA space, reads no guest bytes, and no value from a guest reaches it: kayfabe
  authors `dma_bits` from the family row (§1).
- `kf_dma_window=0` turns the escape off at runtime and removes the feature bit, restoring stock behaviour exactly.

**Residual risks to review.** (1) *Other tenants of the GPU:* mappings made while a window is open land below 2^bits; an
attacker that holds 1 TiB of pinned memory could fail a concurrent victim mapping, but with 1 TiB of pinned memory it could
exhaust the host anyway. (2) *Window duration:* a huge descriptor holds the mutex (only other windowed callers wait,
interruptibly). (3) *`dma_set_mask` while the device is live* is the technique RM itself uses once at init; the kernel path
re-reads the mask per mapping [read, v7.0], but a GPU reset or suspend/resume interleaved with a window is untested [inferred
benign: those paths run under `nv_system_pm_lock`, which the ioctl holds for read]. (4) *No IOMMU* is excluded by the
`NOT_TRANSLATED` verdict because a lowered mask could make the DMA layer bounce. (5) *A bug in this code is a host kernel bug*:
the blast radius is the ~230 lines added to `nv.c`, all on a path that exists only when the escape is called.

## 6. Hardware test plan (for the owner/coordinator; desktop down or a second GPU; not run)

**Rules.** Nothing below was run for this page. It reloads the NVIDIA stack: it stops the display manager and every CUDA/VM user
of the GPU. Do it with no VM running, no other agent using the GPU, and the previous modules saved first.

### 6.0 Facts about the trusted host that change the test [measured, read-only ssh, 2026-10-10 ~13:27]

- GPU `0000:01:00.0` is an **RTX 4070 (AD104) = Ada = 40-bit USERD pointer**, driver 595.91.07 (open flavour, DKMS), kernel 7.0.0-34.
  `nvidia-smi` reports `display_active = Disabled`; an AMD Raphael iGPU (`card0`) also exists; `gdm` is running and `nvidia_drm`
  has one user (`modeset=1`). Confirm which card drives the owner's desktop before assuming either way.
- **The GPU's IOMMU group (11) reads `identity` right now** (set by an earlier Windows run per run 89; `DMA-FQ` is the stock state
  and the state that shows the wall). With `identity` the patch answers `NOT_TRANSLATED` and changes nothing, **and the wall is
  not present either**. Put the group back to `DMA-FQ` for this test (the run-89 procedure in reverse:
  `traces/windows_code43_walls_20261007/run89-iommu-restore.log`), and put it back to what it was afterwards.
- The IOMMU is AMD-Vi (`/sys/class/iommu/ivhd0`); there is no `/sys/kernel/debug/iommu`, so IOVAs are read from the `dma:dma_map_sg` /
  `dma:dma_map_phys` trace events (present in `/sys/kernel/tracing/events/dma/`).

### 6.1 Build and verify (no reload yet)

```
cd <worktree>/tools/host_patches/h5_userd_dma
bash tests/run_tests.sh                                   # GPU-free: include/ == patch, model test
bash box/build_h5.sh /usr/src/nvidia-595.91.07 patch/nvidia_h5_userd_dma_595.91.07.patch /root/kf-h5-build
#  expect PATCH_RC=0 BUILD_RC=0, vermagic == uname -r, "parm: kf_dma_window"
gcc -O1 -Wall -I include -I tests/stub tests/hw/kf_h5_hwtest.c -o /root/kf-h5-build/kf_h5_hwtest
```
`/root/kf-h5-build` is outside `/usr/src`, `/lib/modules` and `/var/lib/kf-windows-*`. Nothing is installed.

### 6.2 Save the stock state (rollback material)

```
K=$(uname -r); S=/root/kf-h5-stock-$K; mkdir -p $S
cp -a /lib/modules/$K/updates/dkms/nvidia*.ko.zst $S/ ; dkms status > $S/dkms-status.txt
lsmod | grep -E '^nvidia' > $S/lsmod.txt ; cat /sys/module/nvidia_drm/parameters/modeset > $S/drm-modeset.txt
cat /sys/kernel/iommu_groups/11/type > $S/iommu-group-type.txt
nvidia-smi > $S/nvidia-smi.txt
```

### 6.3 Replace by `insmod` from the build directory (nothing persisted: a reboot is a full rollback)

```
systemctl stop gdm ; systemctl stop nvidia-persistenced        # whichever are active; also stop kf brokers/VMs
fuser -v /dev/nvidia* /dev/dri/card1 ; # must list nothing
rmmod nvidia_drm nvidia_modeset nvidia_uvm nvidia             # (+ nvidia_peermem if loaded)
#  the HDA audio function of the GPU (0000:01:00.1) stays bound to snd_hda_intel; not touched
B=/root/kf-h5-build
insmod $B/nvidia.ko NVreg_PreserveVideoMemoryAllocations=1 NVreg_TemporaryFilePath=/var   # the host's /etc/modprobe.d options
insmod $B/nvidia-uvm.ko
insmod $B/nvidia-modeset.ko
insmod $B/nvidia-drm.ko modeset=1
systemctl start nvidia-persistenced ; systemctl start gdm
nvidia-smi ; cat /sys/module/nvidia/parameters/kf_dma_window     # expect 1
```
Use the options from `/etc/modprobe.d/*nvidia*` as they are at the time. To make it persistent later: copy the four `.ko` into
`/lib/modules/$K/updates/dkms/` (zstd-compressed like the originals), `depmod -a`, and `modprobe` normally; or the DKMS package
of §4.

### 6.4 Tests (in this order; stop at the first failure and roll back)

| id | what | pass |
|---|---|---|
| T0 | module loads, `dmesg` clean (no `kf H5`/`NVRM: kf` errors), `nvidia-smi` healthy, desktop back | as stated |
| T1 | `kf_h5_hwtest probe` | `PROBE present abi=1 features=0x1 h5_abi=1 h5_usable=1`; and with `echo 0 > /sys/module/nvidia/parameters/kf_dma_window`: `features=0x0`; restore to 1 |
| T2 | group at `DMA-FQ`; trace on: `echo 1 > /sys/kernel/tracing/events/dma/dma_map_sg/enable` (and `dma_map_phys`), `kf_h5_hwtest window 0 40` | `STOCK osdesc: ... status=0x0`, `WINDOW osdesc: ioctl=0 inner_status=0x0 applied=1 verdict=0 mask_bits=40`; the trace lines for 0000:01:00.0 show IOVAs **>= 2^46 for the STOCK object and < 2^40 for the WINDOW object**; mask afterwards: run `window` again and the STOCK lines are high again (restored) |
| T3 | negative controls: `window 0 39` -> `-EINVAL` verdict 1; `window 0 47` -> `ALREADY_NARROW` (or applied=0), stock result; group `identity` -> `NOT_TRANSLATED`, stock result; `kf_dma_window=0` -> `-EOPNOTSUPP`; SIGKILL the tool during `window` (mask restored; repeat T2) | as stated |
| T4 | a stock CUDA program (`nvidia-smi`, a small CUDA sample) with a windowed allocation running concurrently: no errors in either | no Xid, no `NVRM` errors |
| T5 | kayfabe unchanged binary at the same revision: bring-up prints `kf-host: host tier: patched [h5]`; Linux fast suite 30/30 and `v3_gates.sh` 9/9 on the patched module (the stock bar, `V3_HOST_PATCH_LIST.md` §8), then again with `kf_dma_window=0` (`host tier: patched []`) | all green, identical to stock |
| T6 | the real acceptance, **after the kayfabe follow-up of §7 exists**: Windows run, relay off (`KF3_USERD_RELAY_OFF=1`), group `DMA-FQ`, patched host: the user-work twins are born with `userd=Ram`, **zero `physical addr size ... incorrect` lines in dmesg** (run 61's refusal), zero `USERD relay` lines; A/B against the same binary with the relay on, stock module | twin births succeed; no new Xid; the TDR behaviour equals the relay's (run 89 showed the relay is not the TDR cause) |

### 6.5 A/B against the relay

Same kayfabe binary (`kf3-bins/<rev>/`), same box, same flags: (A) stock `nvidia.ko`, relay on; (B) patched `nvidia.ko`, relay off with
the follow-up on. Record the tier line, doorbells to the engine, `GP_GET` agreement at release, and the relay's removed worker
hop. There is no relay latency figure to compare against (`V3_USERD_RELAY.md`; patch list §4.5 "none measured"); measure both.

### 6.6 Failure modes and recovery

| failure | what you see | recovery |
|---|---|---|
| build fails | `BUILD_RC != 0` | nothing changed on the host; fix, rebuild |
| `rmmod` refuses (`in use`) | a process still holds `/dev/nvidia*` | `fuser -v`, stop it; never `rmmod -f` |
| `insmod nvidia.ko` fails (`Unknown symbol`, `Invalid module format`, `Key rejected`) | `dmesg`; stack is unloaded and the screen/gdm cannot start | `modprobe nvidia nvidia_modeset nvidia_uvm nvidia_drm` loads the **stock** modules still installed in `/lib/modules` (nothing was installed over them); then `systemctl start gdm`. Secure Boot: the build dir's modules are unsigned, sign them with the machine's key or test on a box without Secure Boot |
| GPU does not initialise after `insmod` (`NVRM: ... probe failed`) | `dmesg`, `nvidia-smi` fails | `rmmod` the four, `modprobe` stock; if the GPU is wedged, a reboot (nothing persisted) |
| desktop does not come back | black screen, `gdm` failed | ssh in (a VM or a second machine): `systemctl restart gdm`; if it is on the NVIDIA card and the stock modules also fail: reboot |
| patched module misbehaves at run time | Xid, mapping errors, `NVRM: kf H5: could not restore the DMA mask` | `echo 0 > /sys/module/nvidia/parameters/kf_dma_window` (escape off immediately, no reload), then roll back by stopping gdm, `rmmod` the four, `modprobe` stock |
| IOMMU group left at `DMA-FQ`/`identity` | next Windows run behaves differently | write back the type recorded in `$S/iommu-group-type.txt` (needs the nvidia modules unloaded and the HDA function unbound, as in run 89) |

`rollback = reboot` is always sufficient for the `insmod` method, because nothing is written under `/lib/modules`.

## 7. The kayfabe follow-up (NOT enabled; the exact condition)

Built now: the probe (`crates/kf-host/src/tier.rs`), called once per `HostRm::open` (after the version gate, R2b), logged as
`kf-host: host tier: stock` / `patched [h5]`, stored in `HostRm::host_tier()`. **No behaviour reads it** (a unit test checks that
the string `h5_dma_window` appears nowhere in `lib.rs`/`channel.rs`).

The behaviour change, to be built **after T0-T5 pass on hardware and the owner confirms**, adopts the guest's USERD slot (drop the relay for
that twin) when **all** hold, evaluated per twin birth:

1. `HostRm::host_tier().h5_dma_window()` is true (Present, bit set, feature abi 1);
2. the host GPU's family row says its USERD pointer is `bits` wide and the host IOMMU width (`NV_CHIP_EXTENDED_SYSTEM_PHYSICAL_ADDRESS_BITS`)
   is larger, i.e. the wall exists (40 vs 47 on Turing/Ampere/Ada); for the 52-bit families there is no wall and the guest USERD is
   adopted with no window (both widths derived from ogkm-generated `dev_ram.h`/`hwproject.h`, never typed);
3. the USERD pages of the guest were described to host RM through `NV_ESC_KF_ALLOC_MEMORY_DMA_WINDOW` with `dma_bits = bits`
   and the call reported `applied = 1` or `verdict = ALREADY_NARROW` (never `NOT_TRANSLATED` for a 47-bit family: that means the IOMMU
   is identity, where the old workaround of run 89 already works, or absent, and the fallback is the relay);
4. `KF3_USERD_RELAY_OFF` semantics: the relay stays the default until the owner flips it; any refusal at birth (the host still
   says `physical addr size`) falls back to the relay for that twin, named and counted, never a failed birth.

Shape: F2 (a dedicated OS descriptor per USERD page range of the guest, through the window; the whole-RAM descriptor unchanged) is the
narrow reading of D7; F1 (the whole-RAM descriptor through the window) is smaller and wider. Decide with the owner; F2 keeps the
rule "the Passthrough space holds only guest-PT-leaf mappings" trivially, because neither shape maps anything.

## 8. Verification done and not done

**Done (all on the controller, GPU-free, nothing loaded):**

- `patch -p1 --dry-run`: clean on (a) the host's packaged DKMS tree copied read-only to a scratch directory, (b) the 595.91.07 tag's
  `kernel-open/`, (c) 595.84's `kernel-open/` (hunks at offset -4). `tests/run_tests.sh` also checks `include/` against the patch.
- **Build** with `box/build_h5.sh` against the host's DKMS tree and the host's kernel headers (7.0.0-34-generic, gcc 15.2.0, the packaged
  `dkms.conf` make line): `BUILD_RC=0`; `nv.c` compiles without a warning; `NV_IOMMU_IS_DMA_DOMAIN_PRESENT` is defined so the
  translated-domain check is compiled in; `modinfo` shows version 595.91.07, vermagic equal to the running kernel, `parm: kf_dma_window`.
  Final patch sha256 prefix `d1766b944b0c92d2`; `nvidia.ko` sha256 prefix `2cb483f5fcd6dbdf` (36 551 720 bytes, unstripped), `nvidia-uvm.ko`
  `69e9fe72473cbb1f`, `nvidia-modeset.ko` `d316bd845597279c`, `nvidia-drm.ko` `0355ad4b8d0dd7a8`. Build wall time about 1.5 min on 4 cores. The
  only objtool warnings are in the prebuilt RM core (`nvswitch_*`, not ours).
- Export CRCs identical to the installed stock module (§4).
- Userspace model test of the decision and the arithmetic (`tests/test_dma_window.c`): reproduces run 61's `Hi=0x7ff9, Lo=0x007cb4d8`
  as refused by the 40-bit predicate and accepted by the Hopper one; a 64 GiB descriptor has 134 624 refused sampled USERD slots at a
  47-bit mask and zero at 40; the plan never widens a mask (all 63 x 71 x 4 combinations).
- Rust: `cargo test -p kf-host --lib tier` (11 tests incl. the header-constant cross-check).

**Not verified (needs hardware or a loaded module):** that the module loads and `NV_ESC_KF_QUERY` answers; that `dma_set_mask` at run
time changes the next `dma_map_sg`'s IOVA on this kernel with AMD-Vi (read in the v7.0 source, not run); that a USERD in a windowed
descriptor passes `kchannelIsUserdAddrSizeValid_GA100` on the real GPU (the arithmetic is tested, the channel is not); GPU reset /
suspend interleaved with a window; behaviour on Turing and on non-AMD IOMMUs; that nothing in `nvidia-uvm` caches the DMA mask; the
full-source (GitHub tree) build with the RM core compiled from source (the patch does not touch it, but it was not built); the DKMS
packaging.

## 9. Files

`tools/host_patches/h5_userd_dma/`: `patch/nvidia_h5_userd_dma_595.91.07.patch`, `include/` (the two headers, checked against the
patch), `box/build_h5.sh` (build only), `box/regen_patch.sh`, `tests/run_tests.sh`, `tests/test_dma_window.c`, `tests/stub/`,
`tests/hw/kf_h5_hwtest.c` (compiled only, never run), `README.md`. Kayfabe side: `crates/kf-host/src/tier.rs`.
