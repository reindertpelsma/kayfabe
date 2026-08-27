# Feasibility Study: nvkvm Mode 2 — "GPU Device Simulation" (reverse nvkvm)

Status: research / design only. No source changed.
Author: GPU-virtualization architect (feasibility pass)
Date: 2026-05-30

## 0. Scope and definitions

- **Mode 1 (today).** The guest runs a matching NVIDIA userspace (libcuda) plus the
  nvkvm guest kernel module. The guest module forwards `/dev/nvidia*` *ioctls* over a
  paravirtual virtio link; the host translates and replays them against the real driver.
  Boundary = the **ioctl/RM-API** layer. Guest must be modified.
- **Mode 2 (this study).** Present a **simulated NVIDIA PCI device** to the guest so the
  **stock, unmodified** `open-gpu-kernel-modules` driver loads and drives *our* emulated
  hardware. The host turns the guest driver's hardware/GSP traffic into real operations
  on the host GPU. Boundary = somewhere at/below the **GSP-RPC / BAR0-MMIO** layer.
  Guest is unmodified (the strategic prize: stock Linux driver, ultimately Windows).

This document answers the four questions in the brief and gives a go/no-go.

Evidence is drawn from the on-host `open-gpu-kernel-modules` tree (`ssh vh`), the local
gVisor `nvproxy` ABI model, and public GSP-RM / vGPU documentation. File paths below are
relative to `open-gpu-kernel-modules/` on the vast host unless noted.

---

## 1. Interception boundary — what each option costs

The stock driver's life on Turing+ silicon is, from cold:

1. Read PCI config + **BAR0** (16 MB MMIO register aperture), **BAR1** (framebuffer
   aperture / window), **BAR3** (USERD/doorbell on some chips).
2. Read **fuses and BSI secure scratch** to learn secure-boot state
   (`GPU_REG_RD32(pGpu, NV_FUSE_OPT_SECURE_GSP_DEBUG_DIS)`,
   `NV_PGC6_BSI_SECURE_SCRATCH_14`; `src/.../gsp/arch/turing/kernel_gsp_tu102.c`).
3. **Extract the VBIOS from ROM**, parse **FWSEC**, run it to trigger **FRTS** and set up
   **WPR2** (write-protected region in framebuffer)
   (`kgspInitRm_IMPL`, `kgspExtractVbiosFromRom_HAL`, `kgspParseFwsecUcodeFromVbiosImg`;
   `src/.../gsp/kernel_gsp.c` ~line 3650+).
4. Load the **30–72 MB signed GSP-RM firmware blob** (`gsp_tu10x.bin` 30 MB,
   `gsp_ga10x.bin` 72 MB on this host) into framebuffer, boot the **falcon → SEC2 →
   RISC-V GSP** chain. The HS bootloader's signature is **verified by the silicon itself**
   ("root of trust", per NVIDIA GSP docs). FALCON mailboxes
   (`NV_PGSP_FALCON_MAILBOX0/1`) carry the boot args physical address.
5. After GSP-RM is live, the driver sets up a **shared-memory message-queue pair**
   (command/status) in system memory and rings the **doorbell** by writing
   `NV_PGSP_QUEUE_HEAD(queueIdx)` in BAR0
   (`kgspSetCmdQueueHead_TU102`; `src/.../gsp/message_queue_priv.h`). GSP raises **MSI-X**
   interrupts back. All RM work after this is **GSP-RPC messages**, not raw registers.
6. CUDA work then flows as **channel/USERD/doorbell** submissions plus GSP-RPCs for
   allocation/control.

Three candidate boundaries:

### (a) Full BAR0/MMIO + GSP-RPC ring emulation ("the driver thinks it owns a real GPU")
Must emulate: PCI config space + capabilities; the **entire BAR0 register model** the
driver touches during init (PMC, PBUS, PFB/MMU, PGC6/BSI scratch, FUSE, PGSP falcon,
PFALCON, EMEMC/EMEMD ports, timer, MC interrupt tree); **VBIOS ROM** content and the
**FWSEC/FRTS/WPR2** flow; **falcon/RISC-V boot** handshake; the **doorbell** register;
**MSI-X** delivery; **BAR1** framebuffer aperture; **USERD/doorbell** channel path. Then,
*below* the RPC ring, redirect the GSP-RPC messages to the host.

This is the maximal surface. The driver's `kgspBootstrap_HAL` expects the *real* secure
boot to complete (WPR2 comes up, falcon reports the RISC-V app version, GSP posts
`GSP_INIT_DONE`). You cannot fake that without either (i) the real silicon doing it, or
(ii) re-implementing NVIDIA's secure boot — which is signature-gated by hardware and is a
multi-person-year RE effort with no guarantee (see §3.1). **Verdict: emulating boot in
software is effectively infeasible.**

### (b) Intercept at the GSP-RPC ring only (boot GSP-RM normally, redirect RPCs)
Let the guest driver boot a *real* GSP-RM (so FRTS/WPR/falcon all run on real silicon),
then intercept the **command/status message queues** and **doorbell** and ship the RPC
messages to a host agent. Must emulate: only the **doorbell write trap**, the **queue
head/tail registers**, and **interrupt injection** — *if* you could give the guest a real
GPU to boot on. But a VM that is given a real GPU to boot GSP on **is just VFIO
passthrough** — there is no second GPU to boot per guest on commodity hardware. So option
(b) only makes sense if *one* host GSP-RM is shared by *many* guests. That means the guest
must NOT boot its own GSP; it must believe GSP booted while the host multiplexes a single
real GSP-RM. That is exactly NVIDIA's **vGPU** design, and it is gated on SR-IOV VFs +
the closed `vmioplugin`/`vgpu-manager`. See §2/§4. The RPC header itself
(`rpc_message_header_v03_00` in `generated/g_rpc-message-header.h`) literally calls the
peer *"the vmioplugin & guest RM"* — i.e. this ring was designed to be terminated by
NVIDIA's host-side vGPU plugin, not by us.

### (c) Mediated / SR-IOV partitioning (mdev / VF)
Use the GPU's own SR-IOV: enable VFs on the PF, expose each VF as a virtual GPU via
vfio-mdev. The host `vgpu-manager` boots one GSP-RM per VF (or a partitioned GSP), and the
guest's stock driver talks to its VF. Must emulate: essentially *nothing* in software —
the hardware + NVIDIA's host stack do it. But this **requires SR-IOV-capable silicon**
(Ampere datacenter and up: A100/A40/L40/H100…), the **NVIDIA vGPU host driver license**,
and is unavailable on the GeForce/commodity cards nvkvm targets. `vgpu_unlock`/Open-IOV
can spoof a consumer card as a Tesla to *get past the license check*, but the **SR-IOV VF
hardware path still has to exist**; pre-Ampere it patches the legacy mdev path, and on
Ampere+ it still needs the card's VF capability. It does not create SR-IOV where the
silicon lacks it.

**Boundary conclusion.** The only boundary that is both (i) clean and (ii) does not require
re-implementing hardware secure boot is the **GSP-RPC message ring (b)** — but terminating
that ring for *many guests on one GPU* is precisely vGPU, which needs SR-IOV (c). Full
software emulation (a) founders on GSP boot. There is no commodity-hardware sweet spot in
options (a)/(b)/(c) as literally posed.

---

## 2. The GSP-RPC surface

**Count.** The RPC function table (`src/nvidia/inc/kernel/vgpu/rpc_global_enums.h`,
checked on host) defines **`NV_VGPU_MSG_FUNCTION_NUM_FUNCTIONS = 227`** request types
(0…226, with a handful marked deprecated/reserved) plus **`NV_VGPU_MSG_EVENT_NUM_EVENTS =
0x1023` (35 async events, 0x1000…0x1022)** that GSP posts back. The functionally hot ones
are few and familiar:

- `GSP_RM_CONTROL` (76) — wraps the entire **RM control-command** space (the same
  `NV*_CTRL_*` commands nvproxy/Mode 1 already understand). This is the workhorse.
- `GSP_RM_ALLOC` (103) — wraps `RM_ALLOC` (the class/handle allocator; same `nvos64`
  surface Mode 1 sanitizes).
- `RM_API_CONTROL` (204), `ALLOC_ROOT/MEMORY/VIDMEM/VIRTMEM`, `MAP_MEMORY[_DMA]`,
  `DUP_OBJECT`, `FREE`, `CTRL_GPFIFO_SCHEDULE`, `CTRL_GET_WORK_SUBMIT_TOKEN`,
  `CTRL_GPU_PROMOTE_CTX`, `SET_PAGE_DIRECTORY`, `INVALIDATE_TLB`, the `UVM_PAGING_CHANNEL_*`
  family (160–166), etc.

So the *semantic* surface is **the same RM/control/alloc ABI that Mode 1 and gVisor's
nvproxy already model** — `GSP_RM_CONTROL`/`GSP_RM_ALLOC` are thin RPC envelopes around the
exact `NVOS*`/`NV*_CTRL_*` payloads. **This is the key insight, and it holds**: we already
know this surface.

**Stability / versioning — worse than the ioctl ABI.** The RPC wire format is *not* a
stable contract; it is **co-generated and version-locked to a specific driver build**:

- The header is `rpc_message_header_v03_00` and the whole `g_rpc-*.h` / `g_rpcstructure*`
  family is **generated** (the `g_` prefix). Struct layouts are emitted per build.
- GSP-RM and CPU-RM are **two halves of one compiled RM**. NVIDIA ships them together and
  the public guidance is explicit that vGPU host (GSP/`vgpu-manager`) and guest driver
  must be from **compatible release branches**; cross-branch mixing "fails to load"
  (NVIDIA vGPU User Guide; Event ID 160 "Guest driver is incompatible with host driver").
  The on-host firmware blob is named by full driver version (`580.159.04/gsp_ga10x.bin`),
  and the open-RM CPU side asserts a matching GSP-RM build version at boot.
- Contrast: the *ioctl* ABI Mode 1 rides on is comparatively stable and nvproxy tracks it
  with ~39 discrete versions (`v535_104_05` … `v590_44_01` in `nvproxy/version.go`), each a
  curated diff. The RPC structs churn *at least* as fast and have **less** external
  documentation than the ioctl ABI, because they were never meant to cross a trust
  boundary we control.

**Can host GSP-RM execute guest-originated RPCs?** In principle the payloads are the same
RM operations, but three concrete impedance mismatches block a naive splice:

1. **Client/handle namespace.** RPCs carry `hClient/hObject/hParent` handles. In vGPU these
   live in a **per-guest (per-GFID)** namespace the host plugin maps into the real RM. Mode
   1 already had to solve almost this exact problem (see MEMORY: `rmclient_validate_strict_fix`,
   `hclient_not_fd_scoped`, cross-session handle reach C3/C4). The work is real but *known*.
2. **DMA address translation.** RPC payloads embed **guest-physical addresses** for the
   message queues, page tables (`SET_PAGE_DIRECTORY`, `UPDATE_GPU_PDES`,
   `DMA_FILL_PTE_MEM`, `TRANSLATE_GUEST_GPU_PTES`), USERD, semaphores, and DMA buffers. The
   real GSP/MMU must see **host-physical or IOMMU-mapped** addresses. There is an explicit
   `TRANSLATE_GUEST_GPU_PTES` RPC precisely because the *host* side is expected to fix up
   guest PTEs — i.e. NVIDIA's own design assumes a privileged host translator (the
   vgpu-manager) sits here. We would have to *be* that translator. This is the single
   biggest non-security hard part (§3).
3. **GSP partitioning / GFID.** A shared host GSP-RM tags work by **GFID** (function id).
   Without an SR-IOV VF there is no GFID to present, so a single host GSP-RM has no native
   way to keep N guests' contexts isolated.

---

## 3. Hard unknowns, ranked by risk

### #1 (highest) — GSP firmware boot on a simulated device
GSP-RM is a **30–72 MB signed RISC-V firmware** whose HS bootloader signature is verified
**by the GPU silicon**, after a VBIOS-driven **FWSEC → FRTS → WPR2** sequence that reads
fuses/BSI scratch and drives the falcon/SEC2 chain (`kgspInitRm_IMPL`,
`kgspBootstrap_HAL`, `kernel_gsp_frts_tu102.c`, `kernel_gsp_tu102.c`). You cannot run this
against a software-emulated BAR0: there is no silicon to verify the signature or run the
RISC-V core. Options: (a) let the guest boot GSP on *real* silicon — but that is
passthrough, one GPU per guest; or (b) make the guest believe GSP booted while a host GSP
serves it — that is vGPU and needs a VF. **No commodity software path exists.** This single
fact is close to decisive.

### #2 — Guest-physical ↔ host DMA translation (IOMMU/GPA) for GPU DMA
Every page-table, USERD, semaphore, and DMA pointer in the RPC stream is a **guest** address
that the real GPU MMU/GSP must not see verbatim. We would need to walk and rewrite guest
PDEs/PTEs into host-IOMMU space on the fly (`SET_PAGE_DIRECTORY`, `UPDATE_GPU_PDES`,
`DMA_FILL_PTE_MEM`, `TRANSLATE_GUEST_GPU_PTES`) and keep them coherent as the guest remaps.
Mode 1 sidesteps most of this by replaying high-level ioctls and using a GPA-window mmap
trick (MEMORY: `gpa_window_design`, `nvos56_fake_success`); Mode 2 would re-expose the raw
PTE plumbing. High risk, high effort, but *theoretically* tractable if #1 were solved.

### #3 — Security: the guest driver becomes an untrusted near-raw HW client
In Mode 1 the guest only emits ioctls that the host **sanitizes** (the whole audit history:
handle TOCTOU C1, cross-session reach C3/C4, seccomp, OOB #66, etc.). In Mode 2 the guest
emits **GSP-RPCs that include reg-op lists** (`GPU_EXEC_REG_OPS` 50,
`CTRL_DBG_EXEC_REG_OPS` 134, `CTRL_B0CC_EXEC_REG_OPS` 130), page-directory installs, and
fault-buffer registrations — i.e. near-raw hardware control. We'd have to validate a
**227-function** RPC surface (vs the already-large ioctl surface) where many messages carry
register/PTE arrays. nvproxy is *default-deny* and still finds this hard; a default-deny
RPC allowlist that preserves CUDA functionality is a large, ongoing security project. The
attack surface is strictly larger than Mode 1.

### #4 — Interrupt delivery (MSI-X) and the doorbell/queue protocol
GSP signals completion and async events (`POST_EVENT`, `RC_TRIGGERED`,
`MMU_FAULT_QUEUED`, `GSP_INIT_DONE`) via **MSI-X**; the driver rings work via
`NV_PGSP_QUEUE_HEAD` doorbell writes and polls the status queue. Trapping the doorbell MMIO
and injecting virtual MSI-X into the guest is **standard device-model work** (QEMU does this
for virtio/vfio) and is the *least* novel risk here — but it must be wired to the host
RPC agent's completion events with correct ordering and the queue's seqnum/checksum/auth-tag
discipline (`GSP_MSG_QUEUE_ELEMENT` carries a 16-byte **authTag** + AAD + checksum;
confidential-compute builds **encrypt/authenticate** queue elements, which we could not
forge). Medium risk on non-CC parts, blocking on CC parts.

### #5 — USERD/doorbell channel submission + BAR1 framebuffer aperture
Post-init CUDA submits via **USERD** rings and a **doorbell**; the **BAR1** aperture windows
VRAM into the guest. In a shared-GPU model these must be partitioned per guest and the
guest's BAR1 view mapped to a host VRAM slice with MMU enforcement. Mode 1 already proved
the data path works through the GPA window for memcpy/compute (MEMORY: `cumemcpy_first_pass`,
`ptxjit_version_match`); Mode 2 would re-derive it at the hardware-aperture level. Medium
risk *given* #1/#2 solved; otherwise moot.

(Doorbell MMIO trapping and BAR layout are not in the top 3 because they are well-trodden
device-model problems; the GPU-specific killers are boot (#1), DMA translation (#2), and
the enlarged trust surface (#3).)

---

## 4. Verdict

### Is Mode 2 feasible on commodity (non-SR-IOV) NVIDIA GPUs?
**No — not as "simulate a GPU and let the stock driver boot GSP against it."** The blocker
is structural, not merely laborious:

- GSP-RM is a large signed RISC-V firmware whose boot is **anchored in silicon**
  (hardware signature check, FWSEC/FRTS/WPR2, falcon/RISC-V). You cannot boot it against a
  software BAR0, and you cannot skip it on Turing+ (GSP is mandatory for the open module).
- Sharing **one real GSP-RM across many guests** is exactly NVIDIA vGPU, which is gated on
  **SR-IOV VFs** (Ampere+ datacenter) and the closed `vgpu-manager`/`vmioplugin`. The RPC
  ring's own header names the peer "the vmioplugin" — it was built for NVIDIA's host plugin
  to terminate, not us. `vgpu_unlock`/Open-IOV only defeat the *license/marketing* gate;
  they still ride the card's real VF/mdev hardware path and do not synthesize SR-IOV on
  silicon that lacks it.

So: **Mode 2 in its pure form effectively requires vGPU/SR-IOV-class hardware.** On the
GeForce/commodity GPUs nvkvm targets, a faithful "stock driver over a simulated device" is
not reachable by software alone.

### Is there a meaningful middle path?
Yes — and it is the only sane one: **a stock-driver-compatible *front* that reuses Mode-1
forwarding underneath.** Concretely, build a **paravirtual GSP transport**: expose a small
emulated PCI device whose "GSP" is a **stub that never boots real firmware**; instead it
**terminates the GSP-RPC ring in QEMU** and lowers each `GSP_RM_CONTROL`/`GSP_RM_ALLOC`/etc.
message into the **existing Mode-1 host replay path** (which already sanitizes the identical
`NVOS*`/`NV*_CTRL_*` payloads). The guest still needs a **shim** to (a) skip real GSP boot
and (b) present our queues — i.e. it is *not* a fully stock driver, but it could be a **thin
out-of-tree patch to `open-gpu-kernel-modules`** (force `IS_GSP_CLIENT`-style path to our
emulated GSP) rather than a bespoke module. Benefits: reuses the proven sanitizer, handle
namespace, GPA-window DMA, and CUDA data path; the guest userspace is **fully stock**
(libcuda unmodified). Cost: still a guest-side driver patch, so it does **not** deliver the
"completely unmodified driver" or "Windows guest" prize — Windows has no source to patch and
its GSP path cannot be redirected without the firmware boot we cannot provide.

A genuinely-unmodified-driver path (including Windows) is realistically only obtainable by
**buying into SR-IOV hardware** and either licensing NVIDIA vGPU or doing the Open-IOV-style
unlock on a VF-capable card — at which point nvkvm's paravirtual value proposition mostly
evaporates (you'd be a vGPU deployment, not a novel forwarder).

### Effort estimate (paravirtual-GSP middle path, the only viable build)
- **M0 – Bring-up spike (1.5–2 pm):** emulated PCI device + BAR0 trap + GSP message-queue
  pair + doorbell trap + MSI-X injection in QEMU; loopback an RPC. Prove a patched guest
  open-RM reaches "GSP_INIT_DONE" against our fake GSP without touching silicon.
- **M1 – RPC↔ioctl lowering (2–3 pm):** map `GSP_RM_CONTROL`/`GSP_RM_ALLOC`/alloc/map/free
  onto the existing Mode-1 replay + sanitizer; handle namespace + writeback parity.
- **M2 – DMA/PTE translation (3–4 pm, highest risk):** `SET_PAGE_DIRECTORY`/`UPDATE_GPU_PDES`/
  `TRANSLATE_GUEST_GPU_PTES`/USERD/BAR1 into the GPA-window model; channel submission.
- **M3 – Async events + RC/fault path (1.5 pm):** `POST_EVENT`/`RC_TRIGGERED`/
  `MMU_FAULT_QUEUED` back-channel, seqnum/checksum/authTag discipline (non-CC only).
- **M4 – Security hardening (2–3 pm, ongoing):** default-deny RPC allowlist over 227
  functions incl. reg-op/PTE validators; threat-model the larger surface.
- **M5 – Version tracking (recurring):** regenerate RPC struct bindings per driver build;
  expect churn ≥ the 39-version nvproxy treadmill.

**Total ≈ 11–16 person-months** to a single-GPU, single-(or-few)-guest demo that still
requires a guest-side open-RM patch — and it does **not** unlock Windows or
truly-stock-driver guests. Confidence is low on M2 (DMA/PTE) and on long-term version churn.

### Go / No-Go recommendation
**No-go on pure Mode 2 (simulated device, stock driver booting GSP).** It is blocked by
hardware-anchored GSP boot and is, in the shared-GPU form, a re-implementation of vGPU that
needs SR-IOV silicon nvkvm deliberately avoids.

**Conditional, lower-priority "explore later" on the paravirtual-GSP middle path** *only if*
a concrete customer needs to run a Linux distro's stock-packaged NVIDIA driver (no nvkvm
guest module) and is willing to accept a small out-of-tree open-RM patch. Even then it
re-uses Mode 1 underneath and inherits all of Mode 1's hard-won sanitizer/DMA work, so it is
better framed as a **future front-end for Mode 1**, not a second engine.

**Primary recommendation: keep investing in Mode 1.** Mode 1 already runs real CUDA, 7B-LLM
inference, multi-process, and containers through the forwarder (MEMORY: `llm_7b_inference_done`,
`multi_process_unblocked`, `container_toolkit_works`); its moat is the sanitizer + DMA model,
which Mode 2's only viable variant would have to reuse anyway. The marginal dollar is better
spent finishing Mode 1 hardening (teardown #80, RM-control allowlist per the nvproxy gap
analysis) than chasing a Mode 2 that either needs SR-IOV hardware or still ships a guest
patch.

---

## Appendix — key evidence pointers

- RPC function/event tables (227 fns / 35 events):
  `open-gpu-kernel-modules/src/nvidia/inc/kernel/vgpu/rpc_global_enums.h`.
- RPC message header (generated, per-build): `.../src/nvidia/generated/g_rpc-message-header.h`
  (`rpc_message_header_v03_00`; comment: "communication between the vmioplugin & guest RM").
- GSP message queue + doorbell + authTag: `.../inc/kernel/gpu/gsp/message_queue_priv.h`
  (`GSP_MSG_QUEUE_ELEMENT.authTagBuffer[16]`), `kgspSetCmdQueueHead_TU102`
  (`.../gsp/arch/turing/kernel_gsp_tu102.c`, `GPU_REG_WR32(pGpu, NV_PGSP_QUEUE_HEAD(...))`).
- GSP boot (VBIOS/FWSEC/FRTS/WPR2/falcon, signature): `.../gsp/kernel_gsp.c`
  (`kgspInitRm_IMPL`, `_kgspBootGspRm`, `kgspBootstrap_HAL`, `kgspExtractVbiosFromRom_HAL`,
  `pGspFw->pSignatureData/signatureSize`), `.../gsp/arch/turing/kernel_gsp_frts_tu102.c`.
- Firmware blob sizes (signed, silicon-run): host `/lib/firmware/nvidia/580.159.04/`
  `gsp_tu10x.bin` 30 MB, `gsp_ga10x.bin` 72 MB.
- ioctl ABI version treadmill for comparison: `gvisor/pkg/sentry/devices/nvproxy/version.go`
  (39 versions `v535_104_05`…`v590_44_01`).
- vGPU needs SR-IOV (Ampere+) + version-matched host/guest: NVIDIA vGPU User Guide;
  `vgpu_unlock`/Open-IOV defeat licensing, not the VF hardware requirement.

---

## Corrected framing: fake-the-boot, impersonate GSP, forward only compute

**Status: this section SUPERSEDES the §4 "No-go" verdict for the specific architecture below.**
The original study's blocker (§3.1, "GSP firmware boot is anchored in silicon, signature
verified by hardware") only bites if *we try to boot real GSP*. The corrected architecture
**never boots GSP in the guest at all**, so the silicon-signature blocker simply does not
apply — there is no falcon, no SEC2, no RISC-V core, no WPR2 on an emulated device, and
nothing checks a signature because **we author every register response**. Re-scoped below.

The host GPU is already fully booted by the host's real driver (real GSP runs on the host).
The guest runs a stock `open-gpu-kernel-modules` against an **emulated PCI device** that
nvkvm/QEMU presents. We make the guest's GSP-bootstrap code *believe* GSP came up by
synthesizing the registers/handshake it polls, then we **terminate the GSP-RPC ring in
software** and triage: management RPCs get a faked ACK; compute RPCs (RM_ALLOC / RM_CONTROL /
work-related) are lowered into the **existing Mode-1 sanitizer + GPA-window + handle-translation
core** that already runs CUDA and a 7B LLM today.

### CF.1 — Scope of the boot-fake (what we must synthesize before the driver sends RPCs)

Traced from `kgspInitRm_IMPL` (`src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:3619`). The init
path, in order, is the surface we must satisfy. Crucially, **most of it is gated on
`IS_GSP_CLIENT(pGpu)`** (`generated/g_gpu_nvoc.h:5451`, `(pGpu)->isGspClient`): the very
first line of `kgspInitRm_IMPL` is `if (!IS_GSP_CLIENT(pGpu)) return NV_OK;`. Whether the
heavy VBIOS/FWSEC/booter machinery even runs is a *property* set during early GPU detect,
which gives us two implementation levers (synthesize the registers so the stock path runs to
"GSP up", **or** present PCI IDs/scratch such that the driver picks a lighter path). The
concrete poll/read points the stock driver hits before it will issue RPCs:

1. **GFW (GPU firmware / devinit) boot-complete poll.** `kgspWaitForGfwBootOk_TU102`
   (`arch/turing/kernel_gsp_tu102.c:1239`) → `gpuWaitForGfwBootComplete_TU102`
   (`arch/turing/kern_gpu_tu102.c:453`). It (a) waits for a **falcon halt**
   (`kflcnWaitForHalt_HAL`) and (b) reads `NV_PGC6_AON_SECURE_SCRATCH_GROUP_05_0_GFW_BOOT`
   and checks the `_PROGRESS == _COMPLETED` field, after first checking the PLM register
   `NV_PGC6_AON_SECURE_SCRATCH_GROUP_05_PRIV_LEVEL_MASK` has `_READ_PROTECTION_LEVEL0_ENABLE`
   (`_gpuIsGfwBootCompleted_TU102`, kern_gpu_tu102.c:399–445). **To fake "GFW booted" we
   return: PLM reg with RPL0 enable set, and the GFW_BOOT scratch with PROGRESS=COMPLETED.**
   The falcon-halt wait is satisfied by returning the falcon idle/halt status bits. Two
   register values + one falcon status — trivial to synthesize.
2. **VBIOS / FWSEC / FRTS / WPR2.** `kgspExtractVbiosFromRom_HAL`,
   `kgspParseFwsecUcodeFromVbiosImg`, booter-ucode alloc, `kgspPrepareBootBinaryImage`,
   `_kgspPrepareGspRmBinaryImage` (kernel_gsp.c:3660–3760). **In the fake-boot model we do
   NOT run any of this** — there is no ROM to extract, no FWSEC to run, no WPR2 to build.
   We must make the driver *skip* it. Either: (i) report `NV_ERR_NOT_SUPPORTED` from the
   emulated ROM extract (the code at kernel_gsp.c:3690 already treats NOT_SUPPORTED as "OK,
   continue" — a stock, supported escape hatch), or (ii) clamp `isGspClient`/`bPartitionedFmc`
   so the booter path is bypassed. This is the part most likely to need a **thin out-of-tree
   guest patch** rather than pure register-faking, but the driver already has a
   NOT_SUPPORTED branch, so a no-VBIOS emulated device may walk it unmodified.
3. **The actual GSP "boot".** `_kgspBootGspRm` (called in the retry loop, kernel_gsp.c:3855)
   writes the boot-args physaddr into `NV_PGSP_FALCON_MAILBOX0/1`
   (`arch/turing/kernel_gsp_tu102.c:370`) and waits for the RISC-V app to come up. **We never
   run this** — instead our emulated device immediately presents an *already-initialized*
   message-queue pair and posts the `GSP_INIT_DONE` event so that
   `kgspWaitForRmInitDone_IMPL` (kernel_gsp.c:4863 → `rpcRecvPoll(... GSP_INIT_DONE)`:4878)
   returns OK. The driver's "GSP is up" signal is **literally just the GSP_INIT_DONE event
   (0x1001) arriving on the status queue** — we post it ourselves.
4. **Static GPU info the driver consumes after "boot".** Two RPCs:
   `NV_RM_RPC_SET_GUEST_SYSTEM_INFO` and `NV_RM_RPC_GET_GSP_STATIC_INFO` (kernel_gsp.c:3896,
   3905) → `_kgspInitGpuProperties` (kernel_gsp.c:5349) + `_kgspSetFwWprLayoutOffset`
   (kernel_gsp.c:3466). The payload is `GspStaticConfigInfo`
   (`inc/kernel/gpu/gsp/gsp_static_config.h`): grCapsBits, `fbRegionInfoParams`, engineCaps,
   `fb_length`/`fbio_mask`/`fb_bus_width`/`fb_ram_type`/`fbp_mask`/`l2_cache_size`,
   gpuNameString, the SKU bool flags (bIsTesla/bIsMobile/bIsMigSupported/…), ECID, and
   `fwWprLayoutOffset`. **We synthesize this by querying the HOST GPU** (same RM control
   commands the host driver already answers — these are exactly the `NV2080`/`NV0080` ctrls
   Mode 1 forwards) and editing per-guest fields (fb_length = guest's VRAM slice, GFID-ish
   bits zeroed). This is the largest *data* surface, but it is **static, host-sourced, and
   read-only** — not a live hardware model.

**Size / brittleness of the boot-fake.** The *register* surface is remarkably small: a
handful of PGC6 secure-scratch reads (GFW_BOOT + its PLM), falcon halt/status,
PMC/PBUS/boot-status scaffolding the driver touches during detect, the
`NV_PGSP_QUEUE_HEAD`/`QUEUE_TAIL` doorbell pair, and MSI-X config. The *data* surface
(GspStaticConfigInfo + GSP_FW_WPR_META layout) is bigger but is filled from the host GPU.
**Per-version churn**: the register names (PGC6 scratch, PGSP queue head) are stable across
Turing→Blackwell (same HAL family); the *RPC struct layouts* (GspStaticConfigInfo, and every
forwarded payload) are `g_`-generated and **version-locked** — this is the real treadmill,
identical in spirit to the 39-version nvproxy ABI treadmill (`gvisor/.../nvproxy/version.go`)
but tracking RPC structs instead of ioctl structs. Net: the boot-fake itself is **small and
not very brittle**; the ongoing cost is the same per-driver-build struct regeneration Mode 1
already lives with, now applied to the RPC structs.

### CF.2 — RPC ring mechanics (what our software GSP endpoint must implement)

Fully traced from `src/nvidia/src/kernel/gpu/gsp/message_queue_cpu.c` + `kernel_gsp.c`:

- **Queue setup.** `GspMsgQueueInit` (message_queue_cpu.c:180) builds a
  `MESSAGE_QUEUE_COLLECTION` with a command (TX) and status (RX) queue in **system memory**
  (guest RAM, so directly visible to QEMU), each driven by `msgqInit` over a shared header
  (`GSP_MSG_QUEUE_HEADER`, tx/rx `msgqTxHeader`/`msgqRxHeader`). `GspStatusQueueInit`
  (msgq_cpu.c:307) "waits for the other end to run msgqInit" — i.e. it expects **us** (the
  GSP side) to have initialized our half. We pre-init both halves in the emulated device.
- **Doorbell (driver → GSP).** After writing an element, `_kgspRpcSendMessage`
  (kernel_gsp.c:372) calls `GspMsgQueueSendCommand` then `kgspSetCmdQueueHead_HAL`
  (kernel_gsp.c:400) which on Turing is `kgspSetCmdQueueHead_TU102` →
  `GPU_REG_WR32(pGpu, NV_PGSP_QUEUE_HEAD(queueIdx), value)`
  (`arch/turing/kernel_gsp_tu102.c:352`). **This single BAR0 register write is the
  doorbell we trap.** On the trap we read the new TX-queue element(s) out of guest RAM.
- **Element format / integrity.** `GSP_MSG_QUEUE_ELEMENT` carries `seqNum`, `elemCount`,
  `checkSum` (a plain `_checkSum32` over the element — message_queue_cpu.c:508/586), and on
  **confidential-compute builds** an `authTagBuffer[16]` + `aadBuffer` with the body
  AES-encrypted (`ccslEncryptWithRotationChecks`, msgq_cpu.c:475). **Non-CC parts: we
  compute the same plaintext checksum and read the body directly. CC parts: blocked** (we
  cannot forge auth tags without the session key) — but CC is datacenter-only and irrelevant
  to GeForce targets.
- **Responses + events (GSP → driver).** We write status-queue elements (with correct
  seqNum/checksum) into guest RAM and **inject an MSI-X interrupt** so the driver's RX poll
  /ISR drains them. Async events are the same path with `NV_VGPU_MSG_EVENT_*` function codes
  (0x1001 GSP_INIT_DONE, 0x1003 POST_EVENT, 0x1004 RC_TRIGGERED, 0x1005 MMU_FAULT_QUEUED,
  0x1006 OS_ERROR_LOG, …). Doorbell-trap + virtual-MSI-X + a shared-RAM ring is **standard
  QEMU device-model work** (virtio/vfio do exactly this).

**Independent corroboration that this ring is RE-tractable:** the upstream **nouveau** driver
re-implements this exact GSP-RPC ring from scratch — `struct r535_gsp_msg` (the element
header) and `struct nvfw_gsp_rpc` (the RPC header), command/status queues, doorbell, MSI →
which is direct evidence a *third party* already terminates/originates this ring without
NVIDIA's host plugin. We are doing the mirror image (be the GSP end, not the CPU end).
(Linux nouveau GSP docs; Phoronix GSP-RM firmware coverage.)

### CF.3 — Triage of the 227 RPC functions (management→fake-ack vs compute→forward)

From `inc/kernel/vgpu/rpc_global_enums.h` (227 `NV_VGPU_MSG_FUNCTION_*`, 35 events
0x1000–0x1022). Rough family breakdown:

**FORWARD (compute / graphics / memory / work) → lower into the Mode-1 sanitizer.** These
carry the *identical* `NVOS*`/`NV*_CTRL_*` payloads Mode 1 already sanitizes:
- The two workhorses: **GSP_RM_CONTROL (76)** = the entire `NV*_CTRL_*` space, **GSP_RM_ALLOC
  (103)** = the `nvos64`/class allocator. Plus `RM_API_CONTROL (204)`.
- Alloc/map/free family: `ALLOC_ROOT (2)`, `ALLOC_MEMORY (4)`, `ALLOC_CHANNEL_DMA (6)`,
  `MAP_MEMORY (7)`, `ALLOC_OBJECT (9)`, `FREE (10)`, `ALLOC_VIDMEM (12)`,
  `MAP_MEMORY_DMA (14)/UNMAP* (13/15)`, `ALLOC_SUBDEVICE (19)`, `ALLOC_DYNAMIC_MEMORY (20)`,
  `DUP_OBJECT (21)`, `ALLOC_EVENT (23)`, `ALLOC_VIRTMEM (52)`, `ALLOC_SHARE_DEVICE (32)`.
- Work / channel / context: `CTRL_GPFIFO_SCHEDULE (97)`, `CTRL_GPFIFO_GET_WORK_SUBMIT_TOKEN
  (186)`/`SET_..._NOTIF_INDEX (187)`, `CTRL_GPU_PROMOTE_CTX (111)`, `CTRL_GPU_INITIALIZE_CTX
  (115)`, `CTRL_RESET_CHANNEL (88)`, `CTRL_PREEMPT (99)`, `CTRL_SET_TIMESLICE (98)`,
  `CTRL_STOP_CHANNEL (149)`, the GR ctxsw family (112–114), `CTRL_GR_GET_CTX_BUFFER_*
  (144/145)`, the UVM paging-channel family (160–166).
- DMA/PTE plumbing (forward, but **address-translate** — see CF.5): `DMA_FILL_PTE_MEM (27)`,
  `SET_PAGE_DIRECTORY (54)`, `UNSET_PAGE_DIRECTORY (79)`, `UPDATE_GPU_PDES (61)`,
  `UPDATE_PDE_2 (53)`, `TRANSLATE_GUEST_GPU_PTES (56)`, `INVALIDATE_TLB (200)`,
  `CTRL_DMA_SET_DEFAULT_VASPACE (120)`, `UPDATE_BAR_PDE (70)`.

**FAKE-ACK (hardware/system management — return OK without touching real HW)**: clocks/perf/
power/thermal/ECC/TDP/registry/system-info:
- `SET_GUEST_SYSTEM_INFO (1)`, `GSP_SET_SYSTEM_INFO (72)`, `SET_GUEST_SYSTEM_INFO_EXT (64)`,
  `SET_REGISTRY (73)` — accept and stash.
- `CTRL_PERF_BOOST (92)` (the famous cuCtxCreate=800 BOOST_TO_MAX; ack it),
  `PERF_GET_*` (40/42), `CTRL_PERF_RATED_TDP_* (151/152)`, `CTRL_PERF_LIMITS_SET_STATUS_V2
  (172)`, `CTRL_PERF_VPSTATES_GET_CONTROL (93)`, `GET_STATIC_PSTATE_INFO (55)`.
- `CTRL_CLK_GET_EXTENDED_INFO (91)`, `CTRL_GPU_QUERY_ECC_STATUS (201, deprecated)`,
  `ECC_NOTIFIER_WRITE_ACK (202)`, `CTRL_GET_LATEST_ECC_ADDRESSES (118)`, ZBC table ctrls
  (94–96/122/214), `TDR_SET_TIMEOUT_STATE (48)`, `UNLOADING_GUEST_DRIVER (47)`,
  `SAVE/RESTORE_HIBERNATION_DATA (181/182)`, display/NvFBC/NvENC session ctrls (45/46/86/87).
- **Static-info answers (fake, but populated from the HOST GPU):** `GET_GSP_STATIC_INFO (65)`,
  `GET_STATIC_INFO (51)`, `GET_STATIC_INFO2 (77)`, `GET_STATIC_DATA (207)`,
  `GET_CONSOLIDATED_GR_STATIC_INFO (156)`, `CTRL_FB_GET_INFO_V2 (142)`,
  `CTRL_GPU_GET_INFO_V2 (209)`, `GET_GSP_STATIC_PSTATE_INFO`, `CTRL_GET_CE_PCE_MASK (121)`,
  `CTRL_GRMGR_GET_GR_FS_INFO (148)`/`CTRL_FB_GET_FS_INFO (147)`. (Answer from host RM, then
  edit fb size/GFID fields per guest.)
- **NVLink / fabric / MIG / SR-IOV-internal / vGPU-plugin families** — N/A on a single
  GeForce; stub to NOT_SUPPORTED/OK (124/206/211, 179/180/194, 184/185/195, 173/205, …).

**SECURITY-CRITICAL (forward but must validate, do not blind-ack)** — these carry near-raw
HW: `GPU_EXEC_REG_OPS (50)`, `CTRL_B0CC_EXEC_REG_OPS (130)`, `CTRL_DBG_EXEC_REG_OPS (134)`,
the debugger SM-error/exception family (108–110, 132–139, 157), HWPM/PM-area reserve
(128/129/131/199/219–222), and the PTE/page-dir installs above. These are the RPC analog of
Mode 1's already-known dangerous ioctls.

**Confirmation of the key claim:** the forward set's payloads ARE the same `NVOS*`/`NV*_CTRL_*`
ABI Mode 1 sanitizes — `GSP_RM_CONTROL`/`GSP_RM_ALLOC` are thin RPC envelopes around those
exact structs, and gVisor's nvproxy models the very same surface. The triage is therefore a
**routing layer on top of the existing sanitizer**, not a new semantic engine. Ballpark of
the 227: ~40–60 must be genuinely forwarded for CUDA, ~30 are security-critical-but-forwarded,
the rest fake-ack or are N/A on commodity single-GPU.

### CF.4 — Work submission is OUTSIDE the RPC ring (direct MMIO doorbell)

Confirmed: post-init CUDA channel kickoff does **not** go through the GSP-RPC ring. It is a
**direct MMIO write to a doorbell page** in a BAR-mapped USERMODE aperture:
- The class is `*_USERMODE_A` (`clc361.h`: `NVC361_NV_USERMODE__SIZE = 65536` — a 64 KB
  aperture; the doorbell/notify register is `NVC361_NOTIFY_CHANNEL_PENDING = 0x90`).
  Allocated/mapped via `usermode_api.c` (`usrmodeConstruct`), which on modern chips
  (`HOPPER_USERMODE_A`+) maps the doorbell page from **BAR1** (`pKernelFifo->pBar1VF` /
  `pBar1PrivVF`, usermode_api.c:94–99) and falls back to a **BAR0** CPU mapping on
  BAR1-disabled/coherent platforms (usermode_api.c:85–91). USERD itself lives in a memory
  buffer; the doorbell is the MMIO write that says "channel N has new work in its GPFIFO."
- **Implication for us:** the guest libcuda will `mmap` this USERMODE/doorbell page and write
  to it on every submit. That write must reach the **host's real channel doorbell** for the
  corresponding host-side channel. So beyond the RPC ring we need **either** (a) trap the
  guest's doorbell MMIO and translate {guest channel/token} → host doorbell write (a
  per-submit VM-exit — correctness-simple, perf-costly), **or** (b) back the guest's doorbell
  page with the **host's real USERMODE doorbell page** via the GPA-window/MAP_FIXED trick
  Mode 1 already uses for memory, so guest writes land directly on real hardware with no exit.
  Option (b) is the performant target and is **the same GPA-window mechanism Mode 1 proved**
  for memcpy/compute (`gpa_window_design`, `cumemcpy_first_pass`). The work-submit *token*
  (`CTRL_GPFIFO_GET_WORK_SUBMIT_TOKEN`, RPC 186) is allocated on the host channel during the
  forwarded alloc, so the value the guest writes is already host-correct. This interacts
  cleanly with host real channels because the channels ARE host channels (created by the
  forwarded RM_ALLOC), just doorbell-rung from the guest.

### CF.5 — Honest re-verdict under the corrected framing

**Is a STOCK Linux guest driver viable on a commodity GeForce, never booting guest GSP,
faking management, forwarding compute into the Mode-1 core?** **Yes — plausibly viable**,
and the original "no-go" does **not** apply because the silicon-signature/GSP-boot blocker is
sidestepped entirely (we author the registers; nothing verifies a signature on an emulated
device). One honest caveat on the word "stock": the cleanest builds may still need a **thin
out-of-tree patch** to `open-gpu-kernel-modules` to take the no-VBIOS / skip-booter path
deterministically — though the existing `NV_ERR_NOT_SUPPORTED` escape at kernel_gsp.c:3690
plus an `isGspClient`-shaped detect hint suggests a *fully unmodified* Linux guest driver is
within reach if the emulated PCI device is shaped correctly. Guest **userspace (libcuda) is
fully stock** either way. This is strictly better than the §4 "middle path" framing: same
reuse of Mode 1, but now with a credible path to an *unmodified* guest kernel driver.

**Top 3 REAL risks (the GSP-signature risk is NOT one of them):**
1. **DMA / guest-PTE ↔ host-physical translation (highest).** Every `SET_PAGE_DIRECTORY (54)`,
   `UPDATE_GPU_PDES (61)`, `DMA_FILL_PTE_MEM (27)`, `TRANSLATE_GUEST_GPU_PTES (56)`, USERD,
   semaphore, and the BAR1 doorbell/aperture carries **guest-physical** addresses the real
   GPU MMU must never see verbatim. We must rewrite them into host-IOMMU/GPA-window space and
   keep them coherent across guest remaps. NVIDIA's own design assumes a privileged host
   translator here (the `TRANSLATE_GUEST_GPU_PTES` RPC exists *because* the host fixes up
   guest PTEs) — **we have to be that translator.** Mode 1 sidesteps most of this at the
   ioctl level; Mode 2 re-exposes the raw PTE plumbing. High effort, but tractable with the
   existing GPA-window.
2. **Doorbell / work-submit MMIO path (CF.4).** Getting the guest's USERMODE doorbell write
   to land on the right host channel performantly (GPA-window-backed page vs trap-per-submit)
   and keeping per-guest channels isolated. Standard device-model work, but perf-sensitive
   and must be airtight for isolation.
3. **Security of the forwarded compute RPCs + per-version churn (tie).** We now validate a
   **227-function** surface where several messages carry register-op arrays and PTE installs
   (CF.3 "security-critical" set). nvproxy is default-deny and still finds this hard; we need
   a default-deny RPC allowlist that preserves CUDA. AND the RPC structs are `g_`-generated /
   version-locked, so the regen treadmill (≥ the 39-version nvproxy cadence) is permanent.

**Why Windows is materially harder.** Everything above leaned on the **open-source** Linux
KMD: we read `kgspInitRm_IMPL`, the exact GFW-boot scratch fields, the doorbell register, the
static-info struct, and the NOT_SUPPORTED escape hatch from source. Windows ships a **closed
KMD** with no source — the boot-fake (which scratch regs it polls, in what order, what static
info it consumes, how it decides "GSP up") must be **reverse-engineered by observation**
(MMIO trace of a real boot under a recording hypervisor), and there is **no `NV_ERR_NOT_SUPPORTED`
branch we can read** to know the skip-booter path exists. The RPC wire format is shared
(GSP-RM is OS-agnostic), so the *forward* half ports; the *boot-fake* half is an RE project
per Windows driver build. Treat Linux-first as mandatory; Windows as a later, higher-risk RE
effort.

**Rough effort + milestones (corrected framing).** Re-using Mode-1's sanitizer/GPA-window/
handle core throughout:
- **CF-M0 Boot-fake spike (1–2 pm):** emulated PCI device + BAR0 trap; synthesize GFW_BOOT
  scratch + PLM + falcon-halt; pre-init the msgq pair; post `GSP_INIT_DONE`; answer
  `GET_GSP_STATIC_INFO`/`SET_GUEST_SYSTEM_INFO` from the host GPU. Goal: a (lightly-patched)
  stock open-RM reaches "GSP up" + GET_GSP_STATIC_INFO with **no silicon GSP boot**.
- **CF-M1 RPC↔Mode-1 lowering (2–3 pm):** doorbell-trap → drain TX queue → route per CF.3 →
  `GSP_RM_CONTROL`/`GSP_RM_ALLOC`/alloc/map/free into the existing sanitizer; status-queue
  writeback + MSI-X; handle-namespace parity (reuse `rmclient_validate`/`hclient` work).
- **CF-M2 DMA/PTE + doorbell datapath (3–5 pm, highest risk):** PTE/page-dir translation into
  the GPA window; USERMODE doorbell page backed by host doorbell (CF.4); channel kickoff.
- **CF-M3 Async events + RC/fault (1.5 pm):** POST_EVENT/RC_TRIGGERED/MMU_FAULT_QUEUED back-
  channel; seqnum/checksum discipline (non-CC).
- **CF-M4 Default-deny RPC allowlist + reg-op/PTE validators (2–3 pm, ongoing).**
- **CF-M5 "Truly unmodified guest" hardening (1–2 pm):** shape the emulated PCI device so the
  stock driver walks the NOT_SUPPORTED/skip-booter path with **zero** guest patch.
- **CF-M6 Per-build RPC struct regen (recurring).**

**Total ≈ 10–16 person-months** to a single-GPU, single-(few)-guest demo running stock
libcuda on a (near-)stock Linux open-RM with no guest GSP boot. This is comparable to the old
estimate but now buys a **credible unmodified-Linux-driver** outcome instead of a guaranteed
guest patch — because the GSP-boot blocker that drove the old no-go **does not exist when we
never boot GSP**. Windows remains a separate, RE-heavy follow-on. Confidence: medium-high on
CF-M0/M1 (small register surface + ring is nouveau-proven), medium on CF-M2 (DMA/PTE), low on
long-term version churn and on a *zero-patch* Linux guest (CF-M5).

### CF.6 — Evidence pointers (this section)
- Boot path / init: `src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c` — `kgspInitRm_IMPL`:3619,
  VBIOS/FWSEC/booter 3660–3760, NOT_SUPPORTED escape :3690, `_kgspBootGspRm` retry loop :3855,
  `SET_GUEST_SYSTEM_INFO`/`GET_GSP_STATIC_INFO` :3896/:3905, `_kgspInitGpuProperties`:5349,
  `kgspWaitForRmInitDone_IMPL`:4863 (`rpcRecvPoll ... GSP_INIT_DONE`:4878),
  `_kgspSetFwWprLayoutOffset`:3466.
- GFW boot poll regs: `src/nvidia/src/kernel/gpu/arch/turing/kern_gpu_tu102.c` —
  `_gpuIsGfwBootCompleted_TU102`:399 (PGC6 GROUP_05 PLM + GFW_BOOT PROGRESS_COMPLETED),
  `gpuWaitForGfwBootComplete_TU102`:453; `kgspWaitForGfwBootOk_TU102`
  (`arch/turing/kernel_gsp_tu102.c:1239`); falcon mailboxes
  `arch/turing/kernel_gsp_tu102.c:370`.
- `IS_GSP_CLIENT`: `generated/g_gpu_nvoc.h:5451` (`(pGpu)->isGspClient`).
- RPC ring: `src/nvidia/src/kernel/gpu/gsp/message_queue_cpu.c` — `GspMsgQueueInit`:180,
  `GspStatusQueueInit`:307, `GspMsgQueueSendCommand`:446 (checksum :508/:586, CC encrypt
  :475, authTag/aadBuffer), `GspMsgQueueReceiveStatus`:598; send wrapper + doorbell
  `src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:_kgspRpcSendMessage:372` →
  `kgspSetCmdQueueHead_TU102` `arch/turing/kernel_gsp_tu102.c:341` (`GPU_REG_WR32(...,
  NV_PGSP_QUEUE_HEAD(queueIdx), value)`:352).
- Static info struct: `src/nvidia/inc/kernel/gpu/gsp/gsp_static_config.h` (`GspStaticConfigInfo`:
  grCapsBits, fbRegionInfoParams, engineCaps, fb_length/fbio_mask/fb_ram_type/fbp_mask,
  gpuNameString, SKU bools, ECID, fwWprLayoutOffset).
- Work-submit doorbell (MMIO, not RPC): `src/common/sdk/nvidia/inc/class/clc361.h`
  (`NVC361_NV_USERMODE__SIZE 65536`, `NVC361_NOTIFY_CHANNEL_PENDING 0x90`);
  `src/nvidia/src/kernel/gpu/fifo/usermode_api.c` (BAR1 doorbell page :94–99, BAR0 fallback
  :85–91); work-submit token RPC 186 (`CTRL_GPFIFO_GET_WORK_SUBMIT_TOKEN`).
- RPC function/event table (227 fns / 35 events): `src/nvidia/inc/kernel/vgpu/rpc_global_enums.h`.
- Independent RE corroboration (the ring is reproducible without NVIDIA's host plugin):
  upstream **nouveau** GSP-RPC (`r535_gsp_msg`, `nvfw_gsp_rpc`, command/status queues,
  doorbell, MSI) — Linux nouveau GSP documentation; Phoronix "NVIDIA Upstreams Newer GSP
  Firmware For Open-Source Nouveau Driver".
