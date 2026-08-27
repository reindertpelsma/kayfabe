# #101 NVENC throughput — FINAL corrected analysis (2026-06-02)

This file went through three wrong root-cause theories. The measurements below
(clean apples-to-apples, pre-generated raw input, host vs guest, same ffmpeg
command) settle it. **#101 is NOT a 7× throughput bug, NOT an unsignaled eventfd,
NOT a mapped-fence coherence bug.**

## The numbers (RTX 3060, driver 580.159.04, h264_nvenc preset p4, -f null)

| input path                 | host fps | guest fps | ratio |
|----------------------------|---------:|----------:|------:|
| 720p,  CPU raw input       |      932 |       895 | 0.96× (PARITY) |
| 1080p, CPU raw input       |      428 |        63 | 6.8×  |
| 1080p, CUDA `hwupload`     |      302 |       121 | 2.5×  |

- Latency-bound (single surface, no pipelining), 720p: guest 4.7–6.2 ms/frame vs
  host 2.0 — a minor (+3 ms) VM cross-thread/event wakeup tax, well inside any
  real-time budget. Not the issue.
- The historical "55 vs 373 / 28 fps / 7×" were short-clip cumulative-average
  artifacts + CPU-bound `testsrc` input generation. Same lesson as
  decode_14x_slow_root_causes / #95.

## What's actually slow (proven by gdb + smaps, not guessed)
At 1080p with CPU raw input, the guest **main ffmpeg thread is 91% of one core,
8/8 gdb samples in `__memcpy_avx` ← `av_image_copy` ← `avcodec_send_frame`** —
i.e. copying each input frame INTO NVENC's CPU input surface (the pointer from
`NvEncLockInputBuffer`). The GPU and the other 3 vCPUs sit idle. The host main
thread at 1080p is only 60% CPU and overlaps that memcpy with `poll()` → 428 fps.

Properties of that NVENC input buffer (3240 kB = one 1080p yuv420p frame):
- HOST smaps: `/dev/zero (deleted)` shared, VmFlags `rd wr sh mr mw me ms sd` —
  pinned sysmem, WB-cached, page-backed. Fast host memcpy.
- GUEST smaps: same buffer, VmFlags add `pf io` — it has been remapped by nvkvm's
  CPU-page **migrate-range** path (the #94 path) onto the GPA window; the guest
  writes through the window every frame.

## Ruled out (with evidence)
- **Cacheability / WC:** an experiment mapping ALL guest isolate mmaps WB gave
  ZERO change (still 63 fps). The buffer was already WB via the migrate-range
  path (#94, `578662f` line in nvkvm_mmap.c). So it is NOT a WC/uncached bug.
  (Bonus: matmul still PASSED under WB-all → on x86 KVM, true device-BAR pfns get
  EPT=UC forced by `kvm_is_mmio_pfn()` regardless of guest PAT, so a WB doorbell
  does not hang. The c5d5d8a "would hang" fear was untested/over-cautious.)
- **Per-frame forwarding / migration:** steady-state ioctls ≈ 0 (120 frames→1667
  ioctls, 360→1669; ~1600 are one-time init), writes ≈ init-only, ~16 futex/frame.
  No per-frame round-trips.
- **Eventfd / OS-event delivery (the original theory):** completion is a mapped
  fence + worker write()→eventfd, syscall-free per frame, same on host and guest.
  The VQ_EVT chain built for it gave zero improvement and was reverted. Do NOT
  re-attempt it for #101.

## Real root cause
Per-frame **large CPU writes into a GPA-window-backed buffer are much slower in
the guest than a native host-RAM memcpy.** CUDA/compute dodges this: per #94's
note, the guest fills HtoD buffers as cached anon RAM *before* migration, so it
never writes *through* the window. NVENC's CPU input surface is migrated once and
then written *through the window every frame* → the cost shows up.
Confirmed by the `hwupload` test: routing input via `cuMemcpyHtoD` (the at-parity
HtoD path) instead of the NVENC input surface ~2×'s guest throughput (63→121 fps).
The residual 2.5× at 1080p is the remaining per-frame 3 MB CPU→GPU upload cost
through the window. Likely virtualized-memory overhead (EPT TLB pressure / 2-level
page walk) on per-frame multi-MB CPU writes; not NVENC-specific.

## Product impact (why this is NOT a pre-public blocker)
- NVENC encode is correct and usable (#99). 720p at host parity.
- 1080p guest is 63–121 fps depending on input path — still real-time for
  1080p30/1080p60 streaming, which is the stated use case.
- The user's real pipeline (capture a GPU-resident framebuffer → NVENC, frame
  already on the GPU) never touches the slow NVENC CPU input surface — it behaves
  like (or better than) the `hwupload` path. The 6.8× is largely an artifact of
  feeding NVENC from CPU raw frames in the benchmark.

## Fix direction (deferred performance work, same family as #94)
1. **Huge-page-back the GPA window memslot** (THP / MADV_HUGEPAGE or hugetlbfs in
   QEMU). A 3 MB per-frame CPU write touches 768 4 KB EPT entries; 2 MB pages cut
   that to ~2 → far less EPT TLB pressure. Contained QEMU-side change; would speed
   ALL large guest accesses through the window (HtoD, DtoH, NVENC), not just this.
2. For write-heavy *reused* buffers, consider keeping them as cached anon guest
   RAM (stub-side sync) rather than remapping onto the window — but that
   reintroduces per-frame sync; measure before committing.
3. Document the GPU-resident-input guidance for downstream encode pipelines.

Validate any fix with the table above (CPU-raw 1080p is the sensitive case).

---

## UPDATE 2026-06-02 — root cause CONFIRMED: GPA window is UC in the EPT

A clean microbench (`tests/integration/pinned_write_bench.c`: CPU memcpy in/out of
`cuMemAllocHost` vs `malloc`) pins it, isolated from NVENC:

```
            guest WRITE   guest READ      (host: no penalty, ~14 GB/s both)
malloc      13–15 GB/s    13 GB/s
pinned       0.22 GB/s     0.12 GB/s     (60–112× slower)
```

So **guest CPU access to any forwarded buffer through the GPA window is uncached.**
Mechanism: the window is exposed as a prefetchable **MMIO** PCI BAR
(`memory_region_init_io`, `virtio_nvgpu_pci.c:108`, GPA `0x380000000000`), so KVM
maps that guest-physical range **UC in the EPT**.

Proven the guest cannot fix it (all no-ops, identical 0.12/0.22):
- guest PTE WB (`pgprot=0x…0027`, PWT=0 PCD=0)
- guest PTE WC (`pgprot=0x…002f`, PWT=1)
- a WB `/proc/mtrr` entry over the window range

→ the UC is forced **below** the guest PTE/MTRR, in the host EPT. The GPU's own DMA
does not use the CPU EPT memory-type, so compute/decode and `cuMemcpy` HtoD/DtoH
stay at parity; only **CPU-direct** access suffers (NVENC `av_image_copy` into the
input surface, `cuMemAllocHost` CPU fills, any CPU memcpy into pinned/mapped GPU
buffers). This silently regressed #94's WB win once #55 moved the window into BAR
space. (The earlier huge-page / TLB hypothesis is **wrong** — reads are UC too.)

### Fix (QEMU-side; guest cacheability flags are irrelevant)
Make KVM map the window memslot WB: expose the window as a RAM region
(`memory_region_init_ram_ptr` over `sparse_vmm_va`) instead of an MMIO BAR, drop the
raw `KVM_SET_USER_MEMORY_REGION`, and resolve the #55 memslot collision
(ivshmem-style RAM BARs are WB). Core memory-path change → regression risk.
Validate: `pinned_write_bench` pinned≈malloc (~10 GB/s) + matmul/parity no-regress
+ NVENC 1080p. Tracked as task #111. (Replacing the guest `dev_id` WB/WC heuristic
does **not** help here — the guest PTE is proven irrelevant.)

### Why VFIO passthrough, vGPU and gVisor nvproxy do NOT have this
The UC penalty is specific to nvkvm's "forward ioctls into a VM + expose
host-resident buffers through a guest PCI-BAR window" design. The others avoid the
cross-address-space window entirely:

- **VFIO GPU passthrough:** the *real* nvidia driver runs **in the guest**, so
  DMA-target/staging buffers (`cudaHostAlloc` etc.) are **native guest RAM** —
  ordinary WB pages — and the GPU reaches them via the **IOMMU** translating to
  guest physical addresses. The CPU touches them at full WB speed; only the GPU's
  own BAR aperture is WC/UC (exactly as on bare metal). No host-memory window
  exists, so there is nothing for KVM to mark UC.
- **vGPU (GRID / SR-IOV):** same shape — the guest owns its system memory (WB),
  the mediated/virtual function DMAs into guest RAM via the host IOMMU. Pinned
  buffers are guest WB RAM, not a view onto host memory.
- **gVisor nvproxy:** the closest cousin — it *also* forwards the ioctl ABI, and
  on its **KVM platform it runs a real VM with a GPA→HPA (EPT) map**, exactly like
  us (the ptrace platform has no VM, but KVM mode does). Yet it has no UC penalty.
  The difference is purely *how the forwarded host memory is installed in the
  guest*: gVisor maps it as **ordinary RAM memslots** (its sentry doesn't emulate a
  PCI bus to the sandboxed app — it maps device/file memory straight into the
  guest physical space as RAM), so KVM stamps the EPT **WB**. nvkvm routes the same
  host memory through an emulated **MMIO PCI-BAR window**, so KVM stamps it **UC**.
  Same VM, same EPT, same forwarding model — opposite EPT memory type, entirely
  because of RAM-memslot vs MMIO-BAR.

**So this is the existence proof that it is fixable:** nvproxy-KVM is a
forward-into-a-VM design with a GPA map and full WB on forwarded GPU sysmem.
nvkvm is different only in that the nvidia driver runs on the **host** (in the
stub), so DMA-able memory is **host** memory exposed to the guest via a window —
and we currently expose that window as a **PCI MMIO BAR**, which is the single
reason KVM marks it UC. The fix is to install the window as a **RAM** memslot
(`memory_region_init_ram_ptr` over `sparse_vmm_va`, ivshmem-style WB RAM BAR, drop
the raw `KVM_SET_USER_MEMORY_REGION`) → KVM stamps WB, matching gVisor. (On x86 the
guest-WB / host-WB / GPU-DMA views of that shared memory stay cache-coherent via
snooping, so WB is correct.) Constraint to respect: keep it to **one** big memslot
(the #55 reason for the single window — cuCtxCreate issues ~1500 device mmaps, one
memslot each blows the KVM slot count); a single RAM-region window satisfies both
WB and the slot budget.
