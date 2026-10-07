# Windows analogue of the V3 guest doorbell module — feasibility research

**STATUS: RESEARCH, 2026-09-26.** Research only (web + the spec). Spec read:
`/workspace/kf-dbmod/docs/design/V3_GUEST_DOORBELL_MODULE.md` (identical copy in `kf-mgpu`; the path
`/workspace/kf-master/docs/design/...` given in the task does not exist). The spec itself says: *"A
Windows version is an end-stage goal (feasibility to be established then)."*

Legend: **[S]** = established by a cited source. **[I]** = my inference. **[M]** = unknown, must be
measured.

## Answer in one paragraph

**Not as designed, and probably not needed in that form.** The Linux module works because on Linux
**libcuda writes the doorbell from user mode** through a page the NVIDIA driver `mmap`s, and a
guest module can take over that VMA. On Windows under WDDM, **CUDA submits "with a call to the OS"**
even with hardware scheduling (HAGS) **[S1]**, so the doorbell is rung by the NVIDIA kernel driver
(nvlddmkm), via its own kernel BAR0 mapping **[I]**. No legitimate Windows interception point
exists for that write: WDDM has no supported miniport filter model **[S6]**, and redirecting another
driver's kernel mapping, or catching its write fault, would need kernel hooking. PatchGuard, HVCI and
signing policy forbid that. Separately, WDDM batches CUDA launches, so the number of doorbells per
LLM token on Windows is **unknown [M]** and may be far below Linux's ~1,008. A module is only
possible, and genuinely fitting, if and when NVIDIA ships WDDM 3.2 **user-mode work submission**
(doorbells mapped into the UMD by dxgkrnl) **[S2]**. Microsoft still labels that feature "under
development" **[S2]**, and I found no evidence that NVIDIA implements it **[M]**. Even then, the
mapping belongs to dxgkrnl and there is no documented hook to substitute it.

## 1. How NVIDIA on Windows submits work

- **Packet scheduling (HAGS off):** the OS schedules. CUDA batches launches into submissions
  **[S1]**. NVIDIA forum and community sources describe WDDM launch overhead that swings from about
  5 µs to 40–50 µs because of batching **[S7]**.
- **HAGS (WDDM 2.7+):** "hardware queues are directly exposed for a given context and the user mode
  driver (in this case, CUDA) is solely responsible for managing the work submissions". It "removes
  the need for batching" **[S1]**. **However**, NVIDIA states: *"submitting work to the GPU is still
  done with a call to the OS, just like in packet scheduling"* **[S1]** (NVIDIA blog, Aug 2021, WSL2
  context, describing the WDDM model). The KMD-side DDI is `DxgkDdiSubmitCommandToHwQueue`. It
  receives a DMA-buffer GPU VA per submission **[S5]**, so the KMD is invoked on every submission.
  ⇒ The doorbell write is issued by the KMD in the kernel **[I]**. Microsoft's own description of
  HAGS keeps submission in the OS and only moves quanta and context switching to the GPU **[S8]**.
- **WDDM 3.2 user-mode work submission (Windows 11 24H2):** this is the only WDDM path where user
  mode rings a doorbell. It works as follows:
  - The UMD creates an HWQueue with `UserModeSubmission` and calls `D3DKMTCreateDoorbell` and
    `D3DKMTConnectDoorbell`.
  - The KMD's `DxgkDdiConnectDoorbell` returns the physical doorbell location.
  - **dxgkrnl maps it into the process** as `DoorbellCpuVirtualAddress`. It rotates that VA to a
    dummy page on disconnect, D3 or victimization.
  - The UMD rings it with a plain store **[S2][S3][S4]**.
  - Microsoft says explicitly that the feature is **"still under development as of Windows 11,
    version 24H2 (WDDM 3.2)"**, is limited to render and compute queues, and must coexist with
    kernel-mode submission **[S2]**.
  - Microsoft also notes that it is expected to **"significantly benefit such applications if
    they're running inside a container or virtual machine"** **[S2]**. That is exactly our motive.
  - No source says NVIDIA sets `UserModeSubmissionSupported` **[M]**.
- **CUDA driver modes:**
  - **TCC** (non-WDDM, datacenter and pro GPUs only) "reduces the CUDA kernel launch overhead"
    **[S9]**. It is plausibly Linux-like direct user-mode doorbells **[I]**; not documented.
  - **MCDM** became the default instead of TCC from R595. It has "slightly higher" submission
    latency than TCC, and NVIDIA is "actively working on bringing it on par (both on WDDM and MCDM)
    with TCC and Linux native" **[S10]**. ⇒ NVIDIA itself treats WDDM and MCDM submission as slower
    than Linux, which is consistent with a kernel transition per submission **[I]**.
- **GSP on Windows:** it can be forced on with registry value `EnableGpuFirmware=1` under the
  display class key. `nvidia-smi -q` then shows a GSP firmware version **[S11]** (community source;
  NVIDIA documents the Linux side **[S12]**). **GSP does not change who rings the doorbell.** In the
  open RM (ogkm, cited in the spec §3), kernel rings are a direct usermode-register write
  (`kfifoRingChannelDoorBell_GA100` → `GPU_VREG_WR32`), not a GSP RPC, and user rings are a user
  store. The doorbell is a BAR0 usermode-page write in both cases **[I, from ogkm source]**.
  ⚠ The spec §5 relies on the userspace doorbell mapping being made when the stock driver `mmap`s
  the usermode object. On Windows that mapping may not exist at all.
  - Aside, and important for Windows at all: kayfabe emulates a **GSP**. A Windows guest therefore
    has to run nvlddmkm in GSP mode, so the owner's "can be forced" is a prerequisite for any Windows
    guest, not only for this module **[I]**.

## 2. If user mode wrote the doorbell (WDDM 3.2 UM submission, or TCC)

- **Who maps it:** under WDDM 3.2, **dxgkrnl** maps and rotates the doorbell CPU VA, using the
  physical address that the KMD returns from `DxgkDdiConnectDoorbell` **[S2][S4]**. Under TCC,
  nvlddmkm maps it through its own private interface **[I]**.
- **Interception options on Windows:**
  - **PnP upper filter on the adapter devnode.** It sees IRPs, not DXGK DDIs. Dxgkrnl is the port
    driver and the miniport registers through `DxgkInitialize`. "no WDDM miniport filters are
    supported" **[S6]**. Interposing on `DxgkInitialize` or the DDI table is a hack, not a model
    **[S6]**.
  - **KMDOD:** a display-only miniport for a *different* adapter. It does not apply.
  - **Hooking dxgkrnl, the memory manager, or the page-fault path:** PatchGuard territory **[I]**.
  - **Editing the process's PTEs from a third-party driver:** undocumented memory-manager internals.
    It is fragile across builds, and dxgkrnl's rotate (dummy ↔ physical on disconnect or D3) would
    undo it or race with it **[I]**.
  - There is **no documented API** to substitute a page in another component's user mapping **[I]**.
- **GPU-PV (Hyper-V) is the relevant precedent, but it is not a hook.** In a GPU-PV guest *"There's
  no KMD in the guest, only UMD"*. The guest UMD's calls go over VMBus to the host KMD, including
  `D3DKMTSubmitCommand` and `D3DKMTSubmitCommandToHwQueue` **[S13]**. NVIDIA confirms that all GPU
  ops in WSL2 are "serialized through VMBUS" **[S1]**. So Microsoft's own VM GPU design pays a VMBus
  round trip per submission. The UM-submission doc is the stated future fix **[S2]**. Consequences:
  - GPU-PV does **not** help kayfabe, because kayfabe's premise is the stock KMD in the guest.
  - It does show that Microsoft's sanctioned route to "no exit per doorbell" is **WDDM UM
    submission**, not a third-party module.
- **Signing:**
  - Test-signing mode works for lab guests. Under HVCI a binary must still be test-signed.
  - Attestation signing is "for testing purposes only". It works on Windows 10/11 desktop, needs an
    EV certificate, and gives "no assurances". Server ≥2016 does not accept attested device or
    filter drivers **[S14]**.
  - Microsoft is also removing trust for the legacy cross-signed program. Evaluation mode starts with
    the April 2026 servicing release on Windows 11 24H2 and later and on Server 2025 **[S15]**.
  - ⇒ A shippable Windows guest driver means an attestation-signed or WHCP-signed driver. A driver
    that hooks the kernel cannot pass WHCP **[I]**.

## 3. If the KMD rings the doorbell (the expected case today)

- The write comes from nvlddmkm's kernel mapping of BAR0 (MmMapIoSpace-style) **[I]**. A separate
  driver cannot redirect or fault-trap that mapping without modifying another driver's page tables or
  hooking the #PF path. That is not legitimate: PatchGuard, HVCI and non-WHCP-able code **[I]**.
  **No Windows module equivalent exists in this case.**
- **The kayfabe-side alternative (expose the host doorbell page at the guest's BAR0 doorbell
  location, with guest chid = host chid) is not viable in general [I]:**
  1. The token is guest-computed from the guest chid (plus runlist and encoding per family; spec §3).
     kayfabe would need to *choose* the host chid. Unprivileged host RM channel allocation does not
     let the client pick the chid **[I]**. Host chids are shared with other host processes and VMs,
     so collisions are certain.
  2. The doorbell is one page shared by all channels. Exposing it writable removes the
     Passthrough/Emulated split that the design depends on: Translated and kernel channels *must*
     trap.
  3. It hands guest *kernel* and user code a raw host doorbell with no table at all. That is a wider
     version of the risk the spec already accepts, and it has no opportunistic fallback.
- **What does transfer to Windows with no guest code [I]:**
  - make each exit cheaper: in-kernel ioeventfd / KVM fast MMIO for the doorbell page (spec §1
    gives about 20 µs nested and about 2 µs non-nested);
  - host-side coalescing, since a doorbell only makes the GPU re-read GP_PUT.
  These are OS-agnostic and also help Linux guests without the module.

## 4. Conclusion

| Windows guest configuration | Who rings | Module possible? |
|---|---|---|
| WDDM, HAGS off (packet scheduling) | KMD, batched **[S1][I]** | **No.** Also likely **less needed**, because batching means fewer doorbells **[M]** |
| WDDM, HAGS on (WDDM 2.7–3.1) | KMD, via OS submit call per submission **[S1][S5][I]** | **No** legitimate interception point |
| WDDM 3.2 user-mode submission (if NVIDIA ever enables it) | UMD store to a dxgkrnl-mapped doorbell **[S2]** | **Possible in principle, with heavy caveats.** No documented page-substitution hook; dxgkrnl rotates the VA; research/test-signed only |
| TCC / MCDM | TCC probably UMD **[I]**; MCDM WDDM-like **[S10][I]** | Same obstacles; TCC is datacenter/pro only |

**Verdict: "not possible as a legitimate Windows driver today; possibly unnecessary."** The answer
is *known* on the architecture side, which is established by sources. It is *unknown* on the numbers
and on NVIDIA's WDDM 3.2 support.

**Recommended approach:**
1. **Measure before designing.** kayfabe already traps every doorbell. On a Windows guest (GSP
   forced on, HAGS on and off), count doorbells per LLM token and read the **guest CPL at each
   doorbell exit** (vCPU CS.RPL/CPL from the exit's register state). CPL0 means the KMD rings, which
   is expected and means no module is possible. CPL3 means user mode rings (UM submission or a
   private path), which reopens the module question. This is one cheap bench run and it settles
   both unknowns.
2. **Invest in OS-agnostic exit cost** (ioeventfd/fast-MMIO doorbell and host-side coalescing). It
   benefits Windows *and* Linux-without-module.
3. **Watch WDDM user-mode submission.** If NVIDIA enables it, the natural design is not a module.
   Instead, kayfabe's emulated GPU reports the doorbell physical address through the (guest) KMD's
   `DxgkDdiConnectDoorbell`, so dxgkrnl maps a kayfabe-provided page. That is still blocked by the
   KMD being NVIDIA's stock binary, so treat it as speculative **[I]**.

**Main risks:**
- NVIDIA may never ship UM submission on GeForce WDDM.
- Any kernel-hook approach fails WHCP and breaks on PatchGuard/HVCI updates.
- The token/chid identity trick collides with the shared host and destroys the Emulated route.
- The Windows doorbell rate may already be low enough that the whole question is moot. Measure
  first.

## Sources

- [S1] NVIDIA, *Leveling up CUDA Performance on WSL2 with New Enhancements* (2021-08-10) — https://developer.nvidia.com/blog/leveling-up-cuda-performance-on-wsl2-with-new-enhancements/
- [S2] Microsoft Learn, *User-Mode Work Submission* — https://learn.microsoft.com/en-us/windows-hardware/drivers/display/user-mode-work-submission
- [S3] Microsoft Learn, *D3DKMTCreateDoorbell* — https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/d3dkmthk/nf-d3dkmthk-d3dkmtcreatedoorbell ; *D3DKMTConnectDoorbell* — https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/d3dkmthk/nf-d3dkmthk-d3dkmtconnectdoorbell
- [S4] Microsoft Learn, *DxgkDdiCreateDoorbell* — https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/d3dkmddi/nc-d3dkmddi-dxgkddi_createdoorbell
- [S5] Microsoft Learn, *DXGKARG_SUBMITCOMMANDTOHWQUEUE* — https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/d3dkmddi/ns-d3dkmddi-_dxgkarg_submitcommandtohwqueue
- [S6] OSR NTDEV, *Filter WDDM miniport driver - not getting calls* — https://community.osr.com/t/filter-wddm-miniport-driver-not-getting-calls/43034
- [S7] Search summary of NVIDIA dev-forum threads on WDDM batching (e.g. https://forums.developer.nvidia.com/t/gap-between-some-thread-calls/35377) — secondary
- [S8] Microsoft DirectX blog, *Hardware Accelerated GPU Scheduling* — https://devblogs.microsoft.com/directx/hardware-accelerated-gpu-scheduling/
- [S9] Nsight VSE reference (driver modes) — https://docs.nvidia.com/nsight-visual-studio-edition/reference/index.html (via search summary)
- [S10] NVIDIA, *CUDA 13.2 Introduces Enhanced CUDA Tile Support…* (MCDM default from R595; latency quote) — https://developer.nvidia.com/blog/cuda-13-2-introduces-enhanced-cuda-tile-support-and-new-python-features/
- [S11] Guru3D forums, *Enable GSP Firmware on Windows* — https://forums.guru3d.com/threads/enable-gsp-firmware-on-windows.455714/ (community, not NVIDIA-official)
- [S12] NVIDIA Linux driver README, *GSP Firmware* — https://download.nvidia.com/XFree86/Linux-x86_64/580.65.06/README/gsp.html
- [S13] Microsoft Learn, *GPU paravirtualization* — https://learn.microsoft.com/en-us/windows-hardware/drivers/display/gpu-paravirtualization
- [S14] Microsoft Learn, *Driver Signing Options* — https://learn.microsoft.com/en-us/windows-hardware/drivers/dashboard/driver-signing-offerings
- [S15] Microsoft Tech Community, *Removing trust for the cross-signed driver program* — https://techcommunity.microsoft.com/blog/windows-itpro-blog/advancing-windows-driver-security-removing-trust-for-the-cross-signed-driver-pro/4504818 (content via search summary; the page did not render)
