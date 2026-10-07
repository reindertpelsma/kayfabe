# Mode-2 forwarding model — translate guest *intent*, don't replay privileged GSP internals

Status: governing principle (2026-06-11). Applies to all Mode-2 forwarding code in
`src/qemu/nvkvm_gpu_emul.c` and the host stub/isolate. Read alongside
`mode2_cuctxcreate_resume.md` (§0.3 map-vs-stub) and `mode2_compute_forwarding.md`.

## The thesis (what the "reverse driver" is doing)

In Mode-2 the guest runs the **stock, unmodified NVIDIA kernel driver + UVM** against an
**emulated GPU with a faked GSP**. By the time anything reaches us (BAR/register writes, the
GSP-RPC ring, DMA), the guest's kernel-RM has **already decomposed an unprivileged userspace
intent** (e.g. `cuCtxCreate`, `cuMemAlloc`, a kernel launch) into a stream of low-level
operations — many of them **privileged, GSP-internal** steps that only ever exist *inside* the
kernel-RM → GSP path.

The job of Mode-2 is **not** to replay those low-level steps on the host. It is to **recover the
original userspace-level intent and re-express it as the normal, unprivileged host userspace
operations** — exactly the operations Mode-1 forwards directly. The host's own kernel-RM then
legitimately re-derives all the privileged GSP steps internally. The host runs a **real GR/CUDA
context and real execution** (the whole point: Mode-2 must eventually run Mode-1 apps).

## Correctness criterion

Only the **observable end-states** must match a real system:

- the host kernel/RM state and the **real GPU execution**, and
- what the **guest GPU application** (libcuda and up) observes.

Everything between is free. The guest kernel module is **not a black box we must faithfully
re-execute** — it is a means to an end. It is perfectly correct for some guest-kernel operations
to **complete instantly as fakes** (they are internal side-effects of the guest's RM sequence),
**as long as** the *one* operation that actually carries the work triggers the real host-side
chain, and the final observables are right. Faking an internal side-effect ≠ faking the result.

## Two classes of forwarded operation

1. **Case 1 — the RPC *is* (essentially) the userspace op.** `GSP_RM_ALLOC` carries the same
   `NVOS64` alloc params as a userspace `RM_ALLOC`; `GSP_RM_CONTROL` carries the same cmd+params
   a userspace control would. These re-issue ~1:1 on the host through the isolate.
   **`nvkvm_m2_shadow_fwd` already does this** — it replays the guest's alloc stream on the host
   stub, which is why the host channel / compute object (`NVC7C0`) are created with **real host
   handles** and the host kernel-RM promotes the host channel's GR context itself.

2. **Case 2 — ROUTE_TO_PHYSICAL / GSP-internal controls with no userspace equivalent**
   (e.g. `NV2080_CTRL_CMD_GPU_PROMOTE_CTX` `0x2080012b`). These have no userspace ioctl because
   they only exist inside the kernel-RM → GSP path. **Do not replay them on the host** — an
   unprivileged userspace process issuing one gets `NV_ERR_INSUFFICIENT_PERMISSIONS (0x1b)`.
   Their *effect* is already achieved by the Case-1 forwarding (the host kernel-RM did its own
   PROMOTE_CTX for the host channel when we forwarded the channel/object alloc). Correct handling:
   **ack the guest (satisfy its post-op completion poll), do nothing on the host.**

## The `0x1b` lesson

`NV_ERR_INSUFFICIENT_PERMISSIONS` from the host stub is **not** "we lack a privilege we should
have." Normal userspace RM ops (alloc / map / submit) are fully unprivileged and work. `0x1b`
means **we forwarded at the wrong layer** — we tried to replay a Case-2 (privileged, GSP-internal)
control as a userspace control. The fix is never "gain privilege"; it is "translate back up to the
userspace intent" (usually Case-1 already did it, so: ack-only).

## Implications / current gaps

- **`PROMOTE_CTX` host-forward (M6.4) is wrong** — it is a Case-2 control. It should be ack-only.
  The host channel is already promoted by the host kernel-RM during the forwarded alloc.
- **The real remaining work is the *submission* intent**, not context setup:
  - guest "run this work" = it writes `GP_PUT` / rings its channel doorbell;
  - correct translation = mirror that into the **already-real host channel's** USERD and ring the
    **host** doorbell (the GP_PUT bridge), then let the host GPU's **real completion** (the GPU
    DMA-writing the real semaphore) flow back **unchanged**;
  - the completion semaphore must live in **shared, untrapped, WB-coherent sysmem** (the shared
    `memfd` page the host GPU DMAs to and the guest polls natively) — never a QEMU-trapped or
    emulated-FB page. A completion is a *real host-GPU write*, never a forged value (per the
    map-vs-stub rule). See `mode2_memory_model.md` and `mode2_compute_forwarding.md`.
  - `hostUSERD put=0` on the compute client = the submission intent is not reaching the host
    channel; that is the live keystone, not context promotion.

## DMA addressing (how the host GPU is pointed at guest memory)

GPU engines never use CPU addresses directly — they issue **GPU virtual addresses**; the GPU MMU
(GMMU) translates GPU-VA → a *bus/DMA address* via PTEs the driver builds. For a **sysmem** page
that bus address is what the GPU puts on PCIe:

- **No IOMMU (current setup):** the bus address **is the (guest) physical address**. In a KVM guest
  with no vIOMMU, `dma_map_page` returns guest-physical, so the **addresses the guest writes into
  its emulated GMMU PTEs are GPAs**. We translate `GPA → shared-RAM memfd offset → host VA →
  OS_DESCRIPTOR → host RM dma_map → host bus address → host GMMU PTE`. The host kernel does the
  host-side dma_map (incl. host IOMMU if any) — we only supply the right host pages. Verified: this
  guest runs **without** a vIOMMU (`/sys/class/iommu` empty, no `iommu_group` on the GPU, no DMAR),
  so GPA translation is correct as-is.

- **vIOMMU present (must support for generality):** if the guest is booted with `intel-iommu` /
  `virtio-iommu`, the guest's `dma_map` returns **IOVAs**, and the guest writes **IOVAs** (not GPAs)
  into its GMMU PTEs. This is generic PCIe-device behavior (IOMMU security groups), not
  NVIDIA-specific. We must then translate **IOVA → GPA** first (walk the vIOMMU's IOVA→GPA tables,
  which QEMU's emulated IOMMU maintains) before the existing GPA path. TODO: detect an active
  guest vIOMMU and insert the IOVA→GPA step ahead of the address-virtualization side-table.

Either way the goal is identical: the host GPU's PTE must point at the **same physical page** the
guest polls (shared memfd / KVM memslot), so a host-GPU write is seen by the guest natively. A
host-allocated *copy* (e.g. via `shadow_fwd` re-alloc) is wrong for shared semaphores/completions.

## Passthrough-except-the-doorbell (the data-plane architecture for forwarded channels)

**Delineation principle (decisive):** *a page guest userspace can write to cannot, by construction,
carry privileged content* — if it did, the driver would have made it a page userspace can't map. So:
- **userspace-accessible pages → PASSTHROUGH-SHARE** (one physical page, both guest CPU view and the
  host channel's GMMU view; no trap). USERD, GPFIFO ring, pushbuffers, completion semaphores.
- **kernel-only pages → TRAP / SIMULATE** (the privileged bits live here; we never need to share them
  with the host because we either forward the userspace op that triggers them, or simulate). e.g. the
  CE-scrubber's kernel-internal USERD.

For a **forwarded** compute channel the correct data plane is **passthrough everything except the
doorbell**:
- USERD/GPFIFO/pushbuffers/sema are each **one shared physical page**, mapped into the host channel's
  VAS at the **same GPU VA** the guest uses. Guest writes GP_PUT into the shared USERD natively
  (host GPU owns GP_GET — clean producer/consumer split, no field written by both: GP_PUT@0x8c guest,
  GP_GET@0x88 GPU-RO, per clc56f.h); the host GPU fetches the shared GPFIFO→pushbuffers, runs, writes
  the completion sema to the shared page; the guest polls it. **Zero QEMU mediation on the hot path.**
- **Only the doorbell traps** (USERMODE+0x90 MMIO): translate guest `(runlist<<16)|chid` token →
  the host channel's token (`c->host_token`, fetched via NVC36F_CTRL GET_WORK_SUBMIT_TOKEN 0xc36f0108)
  and ring the host USERMODE. Verbatim ring is wrong — guest/host chid don't always coincide
  (mode2_doorbell_chid.md §16.1); legacy host-allocates-chid is unreachable on the stock open driver
  (§12), so the trap+translate is mandatory (and is also the natural per-kick demux point).
- This DELETES, for forwarded channels: `nvkvm_chan_execute` (pushbuffer parsing), the GP_PUT
  "bridge" (which copied the consume-cursor, not the produce-index), and all QEMU-side semaphore
  writes. They exist only because the physical share was never finished.

**Sharing/mapping status (the actual remaining work):**
| object | guest aperture | host-VAS map | guest CPU view (untrapped) | status |
|---|---|---|---|---|
| pushbuffers | sysmem | FIXED map_dma @ guest VA | guest RAM (memfd) | DONE (`back_and_map_sys`, M5.19) |
| completion/tracking sema | sysmem | FIXED map_dma @ guest VA | guest RAM | DONE (same path) — but must be placed per-(client,VA) into every owning channel's host VAS (M5.34) |
| **USERD** | **vidmem (as=2)** | host USERD = this page | **MISSING real share** | uses `m2_fbback` overlay (trap-only) → userspace GP_PUT bypasses it → put=0 |
| **GPFIFO ring** | **vidmem** | FIXED map_dma @ gpFifoOffset | **MISSING real share** | overlay only |
| doorbell | BAR0 reg | host USERMODE+0x90 | trapped+translated | primitive exists (`doorbell_setup`) |

**The fix:** convert the vidmem USERD/GPFIFO from the FB-overlay to a real **KVM memslot** at the guest
GPA (reuse Mode-1 `nvkvm_mmap_create` + `nvkvm_mmap_map_to_guest`, `nvkvm_mmap_host.c`), backed by the
host channel's real USERD/GPFIFO object the stub already allocs+mmaps (`back_channel_userd` holds the
`qva`). Keep the **WB-coherency** fix on the shared page (`nvkvm_force_range_wb`, the #111 pattern) so
the guest's pre-doorbell GP_PUT store is globally visible before the trapped ring returns; keep the
**working-set-mapped ring gate** (ring only when every pushbuffer-referenced VA + the sema target is
mapped in the host VAS, else Xid 31/32). Sysmem objects keep `back_and_map_sys`.

**Two orthogonal blockers to test separately** (NOT fixed by the doorbell design): a residual GSP-event
wait (MC_SERVICE_INTERRUPTS / POST_EVENT) for the GSP/FECS-internal ctx-init completion, and the
copy-engine TSG `GPFIFO_SCHEDULE st=0x57` on the RTX 3060 (LCE/runlist topology) for matmul's DMA path.

## Anti-patterns (do not do these)

- Replaying a ROUTE_TO_PHYSICAL / GSP-internal control on the host stub.
- Forging a *completion value* the guest's userspace observes (forge only what is provably
  guest-kernel-internal and content-irrelevant; completions that gate userspace must be real host
  writes — see the CE-scrubber vs compute distinction in `mode2_execfwd_keystone_plan.md`).
- Treating a non-zero host status as "the bug" without first asking whether we should have issued
  that host op at all (Case-2 → we shouldn't).
