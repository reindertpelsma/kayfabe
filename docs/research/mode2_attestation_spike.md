# Mode-2 "Reverse Driver" — Hardware-Attestation Go/No-Go Spike

Status: research-only. No code changed.
Date: 2026-06-02
Question owner: nvkvm
Sources read (local): NVIDIA `open-gpu-kernel-modules` @ `610.43.02`
(`research_clones/ogkm/`), Linux/nouveau tree @ `6f3ed7fec`
(`research_clones/linux/drivers/gpu/drm/nouveau/`), gVisor nvproxy
(`gvisor/`). Web corroboration cited inline.

---

## TL;DR — VERDICT

**Hardware attestation is NOT a blocker for the stock-driver-on-fake-device
approach on consumer (GeForce/Turing→Blackwell) parts in their default,
non-Confidential-Compute mode.**

The NVIDIA kernel driver (open and proprietary share this `src/nvidia` core;
nouveau is an independent existence proof) never *itself* verifies a
silicon-backed secret during bring-up. Every gating step it performs is one of:

1. **Write** ucode/addresses/registers, **start** a processor, then
2. **Poll a register / mailbox / sysmem message-queue for a value** that says
   "the thing I started reached state X."

All cryptographic verification (the GSP/FWSEC/Booter PKC-RSA signature check)
happens *inside the silicon's RISC-V/Falcon bootROM* on a real chip. On a fake
device **there is no bootROM to run and nothing to satisfy** — we never boot a
real GSP, so there is no signature to forge. We simply *report* the success
register values the driver polls for, and answer the post-boot RPCs from an
impersonated GSP-RPC endpoint. This is precisely the "fake the boot" premise,
and the code confirms it is mechanically sound.

The single genuine SILICON-ATTESTATION feature in the tree —
**Confidential Compute / SPDM attestation** (`src/kernel/gpu/conf_compute/`,
Hopper/Blackwell) — is **opt-in**, absent on GeForce default boot, and simply
must not be enabled. It is not on the RmInitAdapter critical path.

So: **GO**, with the caveat that the hard engineering is the *address-space
virtualization and the full GSP-RPC surface*, not attestation.

---

## 1. What the driver checks during bring-up — classified

Bring-up path: `RmInitAdapter` (`arch/nvalloc/unix/src/osinit.c`) →
`gpuStatePreInit/Init` → GSP path `kgspBootstrap_TU102`
(`src/nvidia/src/kernel/gpu/gsp/arch/turing/kernel_gsp_tu102.c:522`).

Every gating check found, classified SILICON-ATTESTATION (blocks emulation) vs
SOFTWARE-NEGOTIATION (we can mirror a value):

| # | Check | Where | Mechanism | Class |
|---|-------|-------|-----------|-------|
| 1 | GFW (GPU FirmWare/VBIOS devinit) boot complete | `kgspWaitForGfwBootOk_TU102` → `gpuWaitForGfwBootComplete_TU102` `kern_gpu_tu102.c:447`; `_gpuIsGfwBootCompleted_TU102:391` | Reads `NV_PGC6_AON_SECURE_SCRATCH_GROUP_05_0_GFW_BOOT`, tests `_PROGRESS == _COMPLETED`, after checking the PLM scratch lowered. Pure register-value poll. | **SOFTWARE** — mirror the COMPLETED progress value + PLM-lowered bit. |
| 2 | RISC-V core present | `kflcnIsRiscvCpuEnabled` reads `NV_PFALCON_FALCON_HWCFG2`, tests `_RISCV == _ENABLE` `kernel_falcon_tu102.c:130` | Capability register read. | **SOFTWARE** — report the bit. |
| 3 | RISC-V core "active" after boot | `kflcnIsRiscvActive_TU102:139` reads `NV_PRISCV_RISCV_CORE_SWITCH_RISCV_STATUS`, tests `_ACTIVE_STAT == _ACTIVE` | This is the *only* signal the driver uses to conclude "GSP booted." The bootROM's signature verdict is invisible to the driver; it observes only this status bit. | **SOFTWARE** — report ACTIVE. **This is the crux: the driver does not see the signature result, only ACTIVE.** |
| 4 | FWSEC / Booter HS ucode "ran OK" | `kgspExecuteHsFalcon_TU102` `kernel_gsp_falcon_tu102.c:380`: writes IMEM/DMEM, `kflcnStartCpu`, `kflcnWaitForHalt`, reads `MAILBOX0/1` | Driver loads NVIDIA-signed ucode, starts the engine, waits for halt, reads mailbox result code. It never validates the signature; the silicon HS-bootROM would. | **SOFTWARE** — report halt + MAILBOX0 = NV_OK. (No real Falcon runs.) |
| 5 | WPR2 (write-protect region) brought up | `kgspIsWpr2Up_TU102:1172` reads `NV_PFB_PRI_MMU_WPR2_ADDR_HI`, nonzero ⇒ up | Register-value poll of a value the (faked) booter "set." | **SOFTWARE** — mirror nonzero WPR2 hi. |
| 6 | SEC2 reload handoff (resume path) | `_kgspIsReloadCompleted` reads `NV_PGC6_BSI_SECURE_SCRATCH_14`, tests `_BOOT_STAGE_3_HANDOFF == _DONE` `kernel_gsp_falcon_tu102.c:449` | Scratch-register value poll. | **SOFTWARE** — only matters on suspend/resume, not first boot. |
| 7 | GSP-RM init done | `kgspWaitForRmInitDone_IMPL` `kernel_gsp.c:6264` → `rpcRecvPoll(..., NV_VGPU_MSG_EVENT_GSP_INIT_DONE)` then asserts `rpc_result == NV_OK` | Polls the **sysmem message queue** (memory we own) for an RPC event, reads `rpc_result`. | **SOFTWARE** — our impersonated GSP-RPC endpoint posts INIT_DONE with rpc_result=NV_OK. |
| 8 | Confidential Compute attestation (SPDM/measurements) | `src/kernel/gpu/conf_compute/` (`conf_compute.c`, `ccsl.c`, `conf_compute_keystore_*`) | Real device-rooted attestation (SPDM cert chain, measurements). | **SILICON-ATTESTATION** — but **opt-in / Hopper+CC-mode only**, NOT on the GeForce default path. **Just leave it disabled.** |

**No fused device-ID / ECID / PDI read, no measured-boot, no HW root-of-trust
check appears on the RmInitAdapter critical path.** Chip identity that the
driver consumes (e.g. `pGpu->chipId0`, written into `NV_PFALCON_FALCON_RM` at
`kernel_falcon_tu102.c:191`) is a *value the driver reads from a boot/ID
register and propagates* — it is not cross-checked against a secret. We choose
what those ID registers report (must be self-consistent with the PCI IDs and
the chip the rest of our HAL claims to be).

---

## 2. Where the cryptographic verification actually lives (and why it's moot)

- The GSP/FWSEC/Booter binaries are NVIDIA-signed. The driver's role is purely
  to **patch the signature blob into the ucode image** and DMA it in — it does
  **not** compute or verify anything:
  - ogkm: signatures are sliced out of the ucode container and copied;
    `kgspExecuteHsFalcon` just loads DMEM "(note: signatures must already be
    patched)" (`kernel_gsp_falcon_tu102.c:191`) and the BL desc signature words
    are even zeroed in the kernel-side path (`:340-343`).
  - nouveau: `nvkm_gsp_fwsec_init` reads the desc, sets `fw->dmem_sign =
    desc->PKCDataOffset`, and calls `nvkm_falcon_fw_sign(...)` to **"Patch in
    signature"** from the VBIOS (`fwsec.c:244-253`). Per-arch
    `*_gsp_fwsec_signature` (e.g. `ga102.c:95`) just selects which signature
    bytes to patch. There is no verify step in the driver.
- The actual check is the **on-silicon RISC-V/Falcon bootROM**: SHA256 +
  PKCSv2.1 RSA over the image, then copy-to-SRAM-and-execute — confirmed by
  NVIDIA/open-IOV/nova-core documentation (see Sources). This bootROM **only
  exists on real silicon**. In Mode-2 we never instantiate it: the fake device
  has no GSP, so there is nothing for a bootROM to verify and nothing for us to
  forge. We jump straight to "RISCV_STATUS = ACTIVE + INIT_DONE RPC."

This is the central reason attestation is not a blocker: **the trust boundary
NVIDIA built is silicon→firmware, not driver→firmware.** The driver is on the
*untrusted* side of that boundary and therefore performs no check we'd have to
defeat. Emulating the device means we replace the silicon side entirely, so the
check simply does not run.

---

## 3. Boot + GSP-RPC handshake we must ACK (secondary task)

Order the fake device/VMM must satisfy for `RmInitAdapter` to succeed
(normal first boot, `KGSP_BOOT_MODE_NORMAL`, from `kgspBootstrap_TU102`):

1. **Pre-GSP**: report GFW boot COMPLETED via the PGC6 secure-scratch
   `GFW_BOOT` register (#1), and the PLM read-protection-level0 bit lowered.
   Report `HWCFG2._RISCV = ENABLE` (#2).
2. **FWSEC (FRTS setup)**: driver writes IMEM/DMEM, starts CPU, waits halt,
   reads MAILBOX0. → ACK: halt + MAILBOX0 = NV_OK (#4). FRTS/WPR layout is in
   `kgspPopulateWprMeta_TU102` (see §4).
3. **Reset into RISC-V**: `kflcnResetIntoRiscv` (reset + set RISC-V mode). No
   value the driver waits on here beyond reset-finished status.
4. **Program LibOS boot args address** into mailbox regs
   (`kgspProgramLibosBootArgsAddr`) — driver *writes*; we just record the GPA.
5. **Booter Load** HS ucode: same write/start/halt/mailbox pattern (#4).
6. **`kgspSendInitRpcs`**: a short burst of init RPCs *before* OBJGPU creation —
   our GSP-RPC endpoint must consume these from the command queue and respond
   on the status queue.
7. Driver writes `FALCON_OS = appVersion`, then checks `kflcnIsRiscvActive`
   (#3) **or** processor-suspended. → ACK: RISCV_STATUS ACTIVE.
8. **`GspStatusQueueInit`**: links the sysmem status queue (memory we own).
9. **`kgspWaitForRmInitDone`** → `rpcRecvPoll(GSP_INIT_DONE)` and assert
   `rpc_result == NV_OK` (#7). → ACK: post a `GSP_INIT_DONE`
   (`rpc_init_done_v17_00`, `kernel_gsp.c:6293`) with result NV_OK and sane
   `bIsD3Hot`.

After this, the driver believes GSP-RM is live and all further device setup is
**RPC traffic over the sysmem command/status message queues** (the
`NV_VGPU_MSG_*` protocol, `kernel_gsp.c:1427+` event handlers). That RPC
surface — not attestation — is the large, real implementation cost: we must
either answer it from an emulated GSP-RM model or shim each RPC into the
existing Mode-1 ioctl-forwarding core. The message-queue transport itself is
plain shared sysmem ring buffers (`GspStatusQueueInit`,
`pMessageQueueInfo`) — fully software, no crypto.

nouveau corroborates the identical sequence with no extra checks:
`r535_gsp.c` patches FWSEC/booter signatures, loads booter
(`r535_gsp_booter_load`, line ~1362), asserts `nvkm_falcon_riscv_active`
(`:1121`, `:1788`), then `r535_gsp_rpc_poll(NV_VGPU_MSG_EVENT_GSP_INIT_DONE)`
(`:1791`). A fully open driver booting GSP-era GeForce is the existence proof
that no on-silicon-secret read by the *driver* is required.

---

## 4. Address programming — "virtualize + translate" plausibility (secondary)

The driver hands the GPU **physical/bus addresses** in a handful of
well-defined structures; raw guest addresses must be intercepted and translated
to host/stub addresses (the spike's premise). Confirmed touch-points:

- **WPR meta / FB layout**: `kgspPopulateWprMeta_TU102`
  (`kernel_gsp_tu102.c:754`) computes FB-physical offsets (WPR2 end, FRTS
  offset, bootloader offset, radix3 ELF) and `ct_assert(sizeof(*pWprMeta) ==
  256)`. These are the addresses the "booter" consumes — in Mode-2 they index
  emulated FB, so we record/translate them.
- **Booter Load args**: `_kgspGetBooterLoadArgs` passes
  `memdescGetPhysAddr(pWprMetaDescriptor, AT_GPU, 0)`
  (`kernel_gsp_tu102.c:487`) — a GPU-physical addr we must virtualize.
- **LibOS boot args address** programmed into GSP mailbox regs
  (`kgspProgramLibosBootArgsAddr`) — a sysmem GPA.
- **Message-queue / radix3 page-table descriptors**: `kgspCreateRadix3`,
  `sysmemAddrOfSuspendResumeData` etc. are sysmem physical addresses for the
  GSP's own page tables — again, addresses we own and translate.
- Downstream of boot, BAR1/instance-block/GPU-PTE programming flows through the
  normal RM/`memdesc`/`AT_GPU` path; gVisor nvproxy already shows the
  userspace-facing mmap/handle translation we reuse in Mode-1.

Nothing here reads a value back from "real" hardware that must equal a secret;
all are addresses *the driver chose and wrote*. So "record every address the
driver programs, translate GPA/GPU-phys/GPU-VA/IOMMU-bus → host stub address,
never let a raw guest address reach real silicon" is **mechanically
consistent** with how the code is structured. As the spike assumed, translation
correctness/coverage (esp. the radix3 GSP page tables and BAR1 windows) is the
engineering risk, not a fundamental blocker.

---

## 5. Residual risks / sharp edges (none are attestation)

1. **GSP-RPC surface is large and versioned.** The real cost is implementing/
   shimming `NV_VGPU_MSG_*`. Mitigate by forwarding into the working Mode-1
   core rather than re-modelling GSP-RM. This is "lots of work," not "blocked."
2. **Self-consistent chip identity.** Boot/ID registers (`chipId0`, HWCFG,
   PMC boot regs) and PCI IDs must all describe the *same* chip our HAL
   emulates; mismatches fail capability negotiation (SOFTWARE, but fiddly).
3. **Confidential Compute must stay off.** It is the only real silicon
   attestation; trivially avoided on GeForce default boot — but if a future
   target enables CC mode, that path *is* a hard blocker (SPDM cert chain rooted
   in the device). Out of scope for the GeForce target.
4. **Windows (closed KMD).** Same silicon trust model (driver patches sig,
   bootROM verifies), so the *attestation* conclusion is expected to carry over,
   but the closed Windows KMD's exact register/RPC poll set is unverified here
   and would need its own spike.
5. **Newer arch deltas.** Hopper/Blackwell add explicit RISC-V core-select and
   more BROM/scratch regs; the *pattern* (write/start/poll-for-value) is
   identical (`kernel_gsp_gh100.c`), but each arch's exact "ready" registers
   must be enumerated per target.

---

## 6. Conclusion

For the stated target (stock unmodified NVIDIA open/proprietary Linux driver,
GeForce-class GSP silicon, default non-CC mode):

- **Is hardware attestation a blocker? NO.** The driver performs zero
  silicon-secret verification on the bring-up path. All gating is
  software-mirrorable register/mailbox/RPC value matching (checks #1–#7), and
  nouveau independently boots the same GSP path with no driver-side secret read.
- **The only true silicon-attestation feature (Confidential Compute / SPDM) is
  opt-in and off by default** — partial answer to "is it partial": yes, exactly
  one subsystem is hard, and it is avoidable for this target.
- **The real difficulty is the GSP-RPC message-queue surface + address-space
  virtualization**, which the spike already identified as the core work and
  which the code structure shows is mechanically plausible.

Recommended next spike: prototype the boot-register/mailbox shim + sysmem
message-queue + `GSP_INIT_DONE` ACK against the open driver with heavy printk,
and measure how much of the post-INIT_DONE RPC stream can be satisfied by
forwarding into the Mode-1 core before first real compute.

---

## Sources (web corroboration of on-silicon PKC verification)

- [GPU Firmware — Open-IOV](https://open-iov.org/index.php/GPU_Firmware)
- [nova-core: process Booter and patch its signature (nouveau ML)](https://www.mail-archive.com/nouveau@lists.freedesktop.org/msg48208.html)
- [Secure Boot Details with PKC Protection — NVIDIA Docs](https://developer.nvidia.com/docs/drive/drive-os/6.0.5/public/drive-os-linux-sdk/common/topics/security_concepts/SecureBootDetailswithPKCProtection43.html)

Primary evidence is the local source trees cited file:line throughout
(`research_clones/ogkm/`, `research_clones/linux/.../nouveau/`).
