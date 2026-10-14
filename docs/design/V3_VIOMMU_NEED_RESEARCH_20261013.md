**STATUS: RESEARCH, 2026-10-13.**

# Does a Windows 11 guest need a (virtual) IOMMU? How much of a blocker is "no vIOMMU"?

Web research plus our own notes; no GPU, QEMU or host was used. Every external claim carries a URL and
the date it was read (all 2026-10-13). Labels: **DOCUMENTED** = vendor or maintainer documentation;
**REPORTED** = community or user report (credible, not authoritative); **INFERRED** = my reading, to be
tested before it is relied on. Page-summaries from the fetch tool were produced by a small model; the
quotes that matter below were cross-checked against a second source where one exists, and the
single-source ones say so.

**Owner's security view (recorded, not argued).** A vIOMMU is not needed for kayfabe's security:
kayfabe already accesses guest memory, and the kernel driver is trusted. This note therefore asks only
the compatibility question: what real Windows workloads need a guest IOMMU.

Context: `-machine pc`, `-cpu host,-vmx` on an AMD host, OVMF Secure Boot, TPM 2.0, no guest IOMMU.
Design baseline: `docs/design/V3_VIOMMU.md` (the 2026-10-08 DESIGN-ONLY note; the richer 2026-10-04
LIVE design with detection and fail-closed is on `origin/v3-viommu`, not read in full here). Owner
ruling 2026-10-03 (`docs/OWNER_RULINGS.md`, "Also the same day"): IOVA support must not become a hard
retrofit later. Nothing below contradicts that; it measures how soon the retrofit is forced.

## 1. Short answers

1. **Windows 11 itself, Credential Guard, VBS, HVCI (manual enable), WSL2, Docker Desktop, Windows
   Sandbox, DDA-as-guest: no guest IOMMU is required.** They need Secure Boot, TPM 2.0, and for
   everything virtualization-based, *nested virtualization with working Hyper-V enlightenments*.
   (DOCUMENTED for the requirement lists, REPORTED for KVM practice.)
2. **A guest IOMMU is required only in four narrow places:** (a) VBS configured with
   `RequirePlatformSecurityFeatures=3` ("Secure Boot with DMA protection"); (b) Windows' *automatic*
   Memory Integrity enablement on clean installs (its compatibility check lists "IOMMU required");
   (c) Kernel DMA Protection / "Memory access protection" showing ON; (d) anti-cheat products that
   test for IOMMU (Vanguard on-demand, FACEIT). All DOCUMENTED or REPORTED below.
3. **The anti-cheat group is not a real target:** the same products detect and refuse virtual
   machines (Vanguard VAN 9100; FACEIT states it wants bare metal). A vIOMMU would not change that.
4. **Cheapest thing that unblocks the most: nested virtualization + Hyper-V enlightenments, not a
   vIOMMU. Confirmed**, with a correction to the premise: on an AMD host the relevant switch is
   `kvm_amd nested=1` plus `svm` in the guest CPU (`-vmx` is a no-op on AMD); and Windows-on-KVM-AMD
   nested Hyper-V has had real stability problems (REPORTED). That is where the work is.
5. **A vIOMMU may do harm before it does good:** with a DMAR/IVRS table present, Windows can switch
   Dxgkrnl's IOMMU GPU-isolation on (Windows Sandbox / Application Guard "secure VM" creation), and
   the discrete-GPU driver then has to survive logical-address DMA. Today no trigger exists. And
   the one KVM report of a guest vIOMMU plus VBS says VBS then *failed* to start (single source).
6. **One correction for `V3_VIOMMU.md`:** it says WDDM's IoMmu model is for integrated GPUs and
   discrete NVIDIA uses GpuMmu. Both true, but incomplete: WDDM 3.0 additionally *requires discrete GPU
   drivers to support IOMMU DMA remapping* (a separate feature from the IoMmu memory model), and that
   is the Windows consumer of IOVAs we would face (row 8).

## 2. Decision table

Verdicts: **REQUIRES** a (v)IOMMU, **WORKS** without, **DEGRADES** without (feature present but a
sub-feature, default, or report line is missing).

| # | Item | Verdict without vIOMMU | Evidence (class, source) |
|---|---|---|---|
| 1 | Windows 11 itself (incl. Secure Boot, TPM 2.0) | **WORKS** | Requirements list UEFI Secure Boot capable, TPM 2.0, WDDM 2.0 GPU; no IOMMU. "Windows 11 is also supported on a virtual machine"; VM needs Gen 2, Secure Boot, vTPM (DOCUMENTED, https://learn.microsoft.com/en-us/windows/whats-new/windows-11-requirements). Our own guest: Windows 11 build 26100, Secure Boot true, TPM 2.0, VBS 0 (MEASURED, `V3_WINDOWS_DISCOVERY.md` check boot). |
| 2a | VBS / HVCI, manual enable (default `RequirePlatformSecurityFeatures`=1) | **WORKS** if nested virt works | "Secure Boot" setting: "A computer without IOMMUs will simply have Secure Boot enabled" (DOCUMENTED, https://learn.microsoft.com/en-us/windows/security/hardware-security/enable-virtualization-based-protection-of-code-integrity). VBS "works on VMs that have either nested virtualization support or Guest VSM" (DOCUMENTED, https://learn.microsoft.com/en-us/windows-hardware/design/device-experiences/oem-vbs). On KVM: "nested virtualization (vmx for Intel) is required for Memory Integrity (VBS)"; `-cpu host,vmx,+hv-passthrough` ran it, 11% to ~3% CPU idle with `hv-passthrough` (REPORTED, Proxmox forum thread, robertlukan 2025-02-26 and 2026-04-04/05: https://forum.proxmox.com/threads/virtual-machine-config-windows-11-pro-memory-integrity-does-it-require-nested-virtualization.168788/latest). The same thread has no vIOMMU in its working configs. |
| 2b | VBS with "Secure Boot and DMA protection" (value 3) | **REQUIRES** | "Any computer without IOMMUs will not have VBS or memory integrity protection" if value 3 is chosen (DOCUMENTED, same Learn page as 2a). The OEM VBS table lists IOMMU as a hardware requirement: "All I/O devices capable of DMA must be behind an IOMMU" (DOCUMENTED, oem-vbs page). The VBS hardware list is a spec for OEM hardware; the enable page above is what actually gates a running VBS. DISA STIG WN11-CC-000070 accepts level 1 *or* 3, so a STIG-checked image passes without an IOMMU (DOCUMENTED, https://stigaview.com/products/win11/v2r7/WN11-CC-000070, V2R7, archival). |
| 2c | Memory Integrity *default auto-enable* on clean install | **REQUIRES** (so it will not turn on by itself) | Sysprep's HVCI compatibility bits include "3: IOMMU required", "4: MBEC/GMET required", "7: ACPI WSMT table required" (DOCUMENTED, https://learn.microsoft.com/en-us/windows-hardware/design/device-experiences/oem-hvci-enablement). Microsoft expands default-on to more PCs from October 2026, criteria "not fully specified" (REPORTED, https://www.guru3d.com/story/microsoft-will-bring-memory-integrity-protection-to-more-windows-11-pcs-in-october/). INFERRED: our guest stays "off by default, on by policy"; a hardened image must enable it explicitly. MBEC/GMET and DMA protection are also reported missing under QEMU/KVM even where VBS runs (see 3). |
| 2d | VBS on KVM *with* a guest vIOMMU | **Not shown to help; one report of harm** | Proxmox user, vIOMMU=intel: "none of the virtualization security features can be enabled", Memory Integrity "back in off state" after reboot; vIOMMU=virtio caused an unknown PCI device and high CPU (REPORTED, single thread, dorseymet 2024-05-10: https://forum.proxmox.com/goto/post?id=663586). The same thread lists "DMA Protection, SMM Security Mitigations, MBEC" as unavailable on QEMU/KVM yet available on ESXi, and a later user got VBS on a Ryzen host by running Microsoft's DG-Readiness tool (REPORTED, wbedard 2024-11-19, same URL). No report found of VBS running with `amd-iommu` (see unknowns). |
| 3a | Credential Guard | **WORKS** (needs VBS + Secure Boot) | Requirements: VBS and Secure Boot; TPM, UEFI lock recommended, not required. "The Hyper-V host must have an IOMMU" is a *host* requirement for Hyper-V VMs; guest must be Gen 2 (DOCUMENTED, https://learn.microsoft.com/en-us/windows/security/identity-protection/credential-guard/). Edition: Enterprise/Education only, not Pro. Default-on in 22H2+ for domain-joined machines meeting hardware requirements (same page). |
| 3b | System Guard Secure Launch / SMM protection / Secured-core PC | **Not achievable in a VM, IOMMU or not** | Needs DRTM (Intel TXT SINIT ACM, or AMD DRTM driver, Memory Guard, Secure Processor anti-rollback), a *discrete* TPM 2.0 on Intel, and Windows DMA Protection (DOCUMENTED, https://learn.microsoft.com/en-us/windows/security/hardware-security/how-hardware-based-root-of-trust-helps-protect-windows). INFERRED: QEMU/KVM guests have none of the DRTM/SMM hardware; a vIOMMU does not supply it. Compliance scanners that demand Secured-core will fail regardless. |
| 4 | Kernel DMA Protection (Memory access protection) | **DEGRADES** (reads OFF; protects nothing in a VM) | Protects "against external peripherals" on hot-plug PCIe: Thunderbolt, USB4, CFexpress, M.2 slots; "Virtualization-based Security (VBS) isn't required"; needs an IOMMU, UEFI support, and `msinfo32` shows ON (DOCUMENTED, https://learn.microsoft.com/en-us/windows/security/hardware-security/kernel-dma-protection-for-thunderbolt). Platform needs: DMAR table with `DMA_CTRL_PLATFORM_OPT_IN_FLAG` (Intel) or IVRS "DMA remap support" bit (AMD), `_OSC` native PCIe control, `_DSD` tagging of external ports (DOCUMENTED, https://learn.microsoft.com/en-us/windows-hardware/design/device-experiences/oem-kernel-dma-protection). INFERRED: a VM with no external ports has nothing for it to block; the only user-visible effect is the OFF line. KVM report: libvirt `intel` vIOMMU with intremap, caching_mode and iotlb on, Kernel DMA Protection still OFF (REPORTED, https://discourse.nixos.org/t/enable-kernel-dma-protection-on-windows-guest/18910, 2022-04-29; the user concluded the VT-d platform opt-in was missing). |
| 5a | Hyper-V role, WSL2, Docker Desktop (WSL2 backend), Windows Sandbox in the guest | **WORKS** if nested virt works; none needs a vIOMMU | WSL2 in a VM: "make sure that the virtual machine has nested virtualization enabled" (DOCUMENTED, https://learn.microsoft.com/en-us/windows/wsl/faq). Docker Desktop: SLAT + hardware virtualization; in a VM, enable nested virtualization on the hypervisor first; no IOMMU mentioned (DOCUMENTED, https://docs.docker.com/desktop/setup/install/windows-install/). Windows Sandbox: "relies on the Microsoft hypervisor", supports virtual GPU (DOCUMENTED, https://learn.microsoft.com/en-us/windows/security/application-security/application-isolation/windows-sandbox/). Microsoft does **not support** Hyper-V on a non-Microsoft hypervisor ("might be possible ... isn't supported") (DOCUMENTED, https://learn.microsoft.com/en-us/windows-server/virtualization/hyper-v/nested-virtualization). Support statement, not a technical block. |
| 5b | Nested virtualization on KVM *AMD* host | **WORKS with caveats (the real work)** | `kvm_amd` nested SVM has been default-on in the kernel since 2009 (DOCUMENTED, commit "KVM: SVM: enable nested svm by default" 4b6e4dca70: https://git.evolution-x.org/anierin/msm-5.15/commit/4b6e4dca701; verify on our host kernel anyway). Windows 11 nested Hyper-V on AMD: VMs failed to boot, then froze at 100% CPU with Hyper-V on, fixed by Proxmox 7.2 / kernel 5.15.35 plus Hyper-V enlightenments (REPORTED, https://forum.proxmox.com/threads/nested-hyper-v-virtualization-inhibits-vm-startup.106475/latest, 2022-03 to 2022-05). Unraid on Threadripper Pro (kernel 6.1.23): "unimplemented wrmsr: 0xc0010115", Windows said nested virtualization unsupported, unresolved (REPORTED, https://forums.unraid.net/topic/138451-hyperv-nested-virtualization-on-amd-threadripper-pro, 2023-04 to 2023-09; a CPU-section rewrite helped one EPYC user). QEMU enlightenments for nested: `hv-vapic, hv-vpindex, hv-synic, hv-stimer, hv-tlbflush, hv-ipi, hv-emsr-bitmap`; `hv-evmcs` is Intel-only; `hv-passthrough` is for development, blocks migration, and "does not enable hv-syndbg which can prevent certain Windows guests from booting" (DOCUMENTED, https://www.qemu.org/docs/master/system/i386/hyperv.html). |
| 5c | Hyper-V Discrete Device Assignment *inside* our guest | **N/A** | DDA needs the Hyper-V host's IOMMU and root-complex ACS ("force all PCIe traffic through the IOMMU"); supports "Graphics Adapters and NVMe" (DOCUMENTED, https://learn.microsoft.com/en-us/windows-server/virtualization/hyper-v/plan/plan-for-deploying-devices-using-discrete-device-assignment). It would need an L1 guest vIOMMU *and* a nested passthrough of our emulated GPU; not a user workload to plan for. DDA *to* a VM is a way Microsoft hosts pass GPUs; kayfabe is the GPU presenter, not a DDA target. |
| 6 | Anti-cheat / DRM | **REQUIRES IOMMU on some products, but they also refuse VMs: informational, not a target** | Riot Vanguard security requirements page lists TPM 2.0, Secure Boot, Memory Integrity (HVCI/VBS), IOMMU (DOCUMENTED, https://support.riotgames.com/riot/performance/vanguard-security-requirements, page dated 2026-05-31; the page says nothing on how IOMMU is checked). Vanguard On-Demand (from 2026-06-24): Windows 11 25H2+, Secure Boot, TPM 2.0, VBS, HVCI, IOMMU (REPORTED, https://finalboss.io/valorant-vanguard-windows-11-on-demand-anti-cheat-security-checklist, 2026-07-29); "running Valorant from inside a virtual machine ... can trigger a restriction" (same page); VAN 9100 = unsupported virtualized environment (REPORTED, https://support.riotgames.com/riot/client/error-van-9100, search snippet). FACEIT: TPM 2.0 + Secure Boot from 2025-11-25; IOMMU and virtualization (SVM) already required for players above 3000 Elo; virtualization is checked "to verify that you're running ... on a real, bare metal machine and not a virtual machine"; VBS/HVCI expected later (REPORTED, https://refrag.gg/blog/cs2-secure-boot-tpm-guide; FACEIT's own FAQ returned HTTP 403: https://support.faceit.com/hc/en-us/articles/23117181142556). EA Javelin (Battlefield 6): Secure Boot + TPM 2.0; virtual machines and emulation are among the cheats it targets (REPORTED, https://www.thesixthaxis.com/2025/08/07/battlefield-6-and-black-ops-7-both-need-secure-boot-and-tpm-2-0-for-pc-anti-cheat/). BattlEye generally does not support VMs; EAC/BattlEye support Proton on an opt-in basis (REPORTED, https://pcgamer.com/battleye-anti-cheat-confirms-steam-deck-support). No EAC/BattlEye IOMMU requirement found. |
| 7 | Professional apps (CAD, Adobe, EDA, Office; CUDA, OpenGL, D3D, Vulkan) | **WORKS** (no requirement found) | A search for CAD/Adobe/Office products requiring an IOMMU or VBS found none; the hits were generic Microsoft VBS pages. This is absence of evidence, not a survey (INFERRED). GPU compute and graphics APIs sit on WDDM/RM and do not touch the guest IOMMU unless Dxgkrnl isolation is on (row 8). Host-side IOMMU matters for kayfabe itself (our USERD wall under host `DMA-FQ`, `STATUS_AND_HANDOFF.md`), which is unrelated to a *guest* vIOMMU, as `V3_VIOMMU.md` §1 already says. |
| 8a | WDDM memory models: GpuMmu vs IoMmu on a discrete NVIDIA GPU | **WORKS** (GpuMmu) | WDDM 2.0 has two models; a driver sets `GpuMmuSupported` or `IoMmuSupported`, never both (DOCUMENTED, https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/d3dkmddi/ns-d3dkmddi-_dxgk_vidmmcaps). IoMmu: process address space shared with the CPU, GPU sends PASID plus virtual address to a compliant IOMMU (DOCUMENTED, https://learn.microsoft.com/en-us/windows-hardware/drivers/display/iommu-model); "suitable for integrated GPUs" per the GpuMmu/IoMmu overview (DOCUMENTED, https://learn.microsoft.com/en-sg/WINDOWS-HARDWARE/DRIVERS/display/gpu-virtual-memory-in-wddm-2-0). INFERRED, not verified from the closed driver: a discrete NVIDIA GPU with its own page tables is GpuMmu. We MEASURED nothing on this; `THE_WINDOWS_AXIS.md` line 500 lists "GpuMmu vs IoMmu" as a cheap check not yet recorded. |
| 8b | WDDM IOMMU GPU isolation / DMA remapping (a different feature from 8a) | **WORKS today; becomes a consumer of IOVAs if a vIOMMU appears** | WDDM 2.4: Dxgkrnl can restrict GPU access to system memory with the IOMMU; "By default, this feature is only enabled for PCs where Windows Defender Application Guard is enabled", or via registry `GraphicsDrivers\IOMMUFlags`; when a *secure VM* is created the IOMMU is activated and "starting a secure virtual machine fails if the driver doesn't provide IOMMU support"; the staging key is "disabled by default" (DOCUMENTED, https://learn.microsoft.com/en-us/windows-hardware/drivers/display/iommu-based-gpu-isolation). WDDM 3.0 (Windows 11 22H2): logical (non 1:1) remapping; "Discrete GPUs must support IOMMU DMA remapping, which is a requirement for WDDM 3.0 certification"; Dxgkrnl enables it at start-up if the GPU cannot reach all installed RAM; needs `MapAperture2Supported`, `DXGK_IOMMU_CAPS`, the `ADL` paging operation (DOCUMENTED, https://learn.microsoft.com/en-us/windows-hardware/drivers/display/iommu-dma-remapping). `IoMmuSecureModeRequired`: "IOMMU hardware and isolation support ... is required or the OS will otherwise not start the adapter" (DOCUMENTED, vidmmcaps page above). Observation: the NVIDIA driver starts in our IOMMU-less guest, so it does not set that bit (INFERRED from run 223 reaching the desktop; not a cap dump). The generic PCI opt-in page warns "DMA remapping isn't supported for graphics device drivers" (DOCUMENTED, https://learn.microsoft.com/windows-hardware/drivers/pci/enabling-dma-remapping-for-device-drivers); graphics goes through the WDDM path above, not that INF key. |
| 8c | Does enabling VBS/HVCI change how WDDM programs the GPU's DMA? | **No documented change; display driver must be HVCI-compatible** | HVCI requires "memory integrity-compatible drivers" (HLK HVCI Readiness test, Driver Verifier code-integrity checks) (DOCUMENTED, oem-vbs page). The documented DMA-side lever is the separate IOMMU isolation of 8b, triggered by Application Guard / secure-VM creation or a registry key, not by HVCI itself. INFERRED: with VBS level 1 and no IOMMU, HVCI constrains executable kernel pages of the driver (code), not GPU DMA. Needs a Windows test (below). |
| 9 | If we ever add one: QEMU / OVMF / AMD-Vi specifics | see section 3 | |

## 3. QEMU and Windows specifics (row 9)

- **Machine type.** `intel-iommu` is "only supported by `-machine q35`" (DOCUMENTED, QEMU docs text as
  quoted in https://patchew.org/QEMU/20210611185500.343525-1-peterx@redhat.com/). Moving from `pc` to
  q35 changes the PCI topology, ACPI tables and chipset the Windows image was built on; INFERRED: every
  existing Windows result (device placement, BAR layout, boot) would need re-validation. I could not
  open the QEMU `amd-iommu` page (HTTP 404), so whether `amd-iommu` works on `pc` is unverified.
- **Intel on an AMD host.** The vIOMMU is emulated, so `intel-iommu` runs on any host CPU; INFERRED
  that Windows checks only the ACPI DMAR content, not the CPU vendor. Not verified by a source.
  Windows' AMD path reads the IVRS table instead (DOCUMENTED: IVRS "DMA remap support" bit, OEM page
  above). **No report was found of Windows using a QEMU `amd-iommu`.**
- **Options that matter.** `intremap=on` needs `kernel-irqchip=off|split` ("full kernel-irqchip is not
  yet supported") and is required for full x2APIC; `caching-mode=on` makes every guest DMA mapping
  generate an IOTLB invalidation and is required for vfio-pci; `device-iotlb` is for virtio/vhost with
  `ats=on`; `aw-bits` 39 or 48 (DOCUMENTED, same patchew page). A Windows test hit "Segment 0 has no
  DMA remapping capable IOMMU units" until the DMA drain capabilities were declared; fixed in QEMU 4.0+
  defaults, and the SVVP pass used `x-aw-bits=48,caching-mode=off` (REPORTED, QEMU commit text at
  https://git.cs.tu-dortmund.de/SYS-OSS/FRET-qemu/commit/ccc23bb08a84f3709b08cc10ffd4e819832fae6d).
  Interrupt remapping is not listed as a Windows Kernel DMA Protection requirement in the OEM page
  (INFERRED from absence).
- **AMD vIOMMU in QEMU.** The `dma-remap` property governs MAP-notifier registration (needed so VFIO
  can replay guest I/O page tables); it needs the vIOMMU's NpCache capability, "enabled by default"
  (DOCUMENTED, commit 7e24cddf, 2025-10-05: https://qemu.googlesource.com/qemu/+/7e24cddf1f7941825e195e3239f0345d81927c14).
  Our own `origin/v3-viommu` design (2026-10-04, read from QEMU 10.2.4 source) records that the
  default for `dma-remap` is off, which is the fail-open case it already handles; I did not
  re-verify that in QEMU source. AMD `xtsup`/IVHD type 0x11 export x2APIC support
  (REPORTED, QEMU patch: https://git.cs.tu-dortmund.de/SYS-OSS/FRET-qemu/commit/328a11a08a70ca9e565cee807eb74e1e59e1b5d9).
  Passthrough devices under emulated amd-iommu were demonstrated with NICs (REPORTED, 2020 series:
  https://patchew.org/QEMU/20200928200506.75441-1-wei.huang2@amd.com/).
- **OVMF.** No source found that OVMF needs anything for the DMAR/IVRS tables; QEMU builds them in
  ACPI. Unverified (unknown 3).
- **Cross-check of our invalidation design** (`V3_VIOMMU.md` §3, §5). QEMU's API is an
  `IOMMUMemoryRegion` that translates, plus notifiers registered on it, `memory_region_notify_iommu()`
  for changes, and `memory_region_iommu_replay()` (DOCUMENTED, https://www.qemu.org/docs/master/devel/memory.html).
  That matches the note's `translate` + `subscribe_invalidate` split. Two refinements, both INFERRED:
  (1) VFIO needs *MAP* events (hence `caching-mode=on` / AMD `dma-remap=on`) because it pre-programs the
  host IOMMU; kayfabe resolves at PTE-commit time and compares re-translation (§3), so it needs only
  *UNMAP/invalidation* events, which a guest must send whenever a present translation changes. A
  kayfabe consumer could therefore avoid `caching-mode=on`, which is the less-tested Windows
  configuration. (2) The note's "compare instead of trusting the event" rule is the right fail-safe,
  because with `caching-mode=off` a missed MAP is by definition not signaled. The note's lazy stale
  window is consistent with the Linux DMA API's batched (flush-queue) unmaps, but a Windows guest's
  behavior here is unknown.
- **VFIO-side facts for later.** The VFIO type1 interface uses `VFIO_IOMMU_MAP_DMA` and unmap, and a
  `dma_unmap` callback for drivers (DOCUMENTED, https://docs.kernel.org/driver-api/vfio.html); our host
  USERD wall is unrelated (host group in `DMA-FQ`).

## 4. Findings by question

### (a) How much of a blocker, ranked by who is hurt

1. **Security-hardened enterprise images: medium, and mostly not an IOMMU problem.** Their needs are
   VBS, HVCI and Credential Guard, which need nested virtualization; level-1 VBS passes the STIG.
   What a vIOMMU would add is "Kernel DMA Protection: ON" and "VBS with DMA protection" lines on
   audit forms that demand level 3, and the automatic HVCI enablement. Secured-core / System Guard is
   impossible in a VM either way.
2. **WSL2 / Docker / Windows Sandbox users: medium-high, never an IOMMU problem.** The blocker is
   nested virtualization on KVM-AMD with Windows 11 (REPORTED instability; Microsoft's support
   statement excludes non-Microsoft hosts).
3. **Gamers with anti-cheat: high in appearance, nil in practice.** Vanguard On-Demand and FACEIT
   want IOMMU *and* refuse VMs; a vIOMMU does not make us admissible. Informational only.
4. **Professional users (CAD, Adobe, EDA, Office, CUDA/D3D/Vulkan): low.** No requirement found.
   Their risk is correctness of the GPU path, not IOMMU presence.

### (b) Cheapest thing that unblocks the most

Confirmed: **nested virtualization plus Hyper-V enlightenments, not a vIOMMU.** Concretely, for the
AMD host: verify `/sys/module/kvm_amd/parameters/nested`, give the guest `svm` (drop the meaningless
`-vmx`; on Intel hosts it would be `+vmx`), add the nested set from the QEMU Hyper-V page
(`hv-vapic, hv-vpindex, hv-synic, hv-stimer, hv-tlbflush, hv-ipi, hv-emsr-bitmap`; no `hv-evmcs`), try
`hv-passthrough` as a development shortcut only, and then run `bcdedit /set hypervisorlaunchtype auto`,
enable VBS at level 1, and read `Win32_DeviceGuard` (`VirtualizationBasedSecurityStatus` = 2). This
unlocks Memory Integrity, Credential Guard, WSL2, Docker Desktop and Windows Sandbox in one stroke.
Caveat that is ours alone: **the existing Windows work ran arm A (no Hyper-V enlightenments)**;
`V3_WINDOWS_DISCOVERY.md` lines 156 and 407 note "RM takes Hyper-V branches" when enlightenments are
on, so arm B is itself unfinished. Under VBS the root partition becomes a guest of Windows' own
hypervisor, which changes timing, TLB flush and MSR behavior that the kf3 traps depend on
(INFERRED); also an extra nested level in front of every BAR0 trap makes the "about 20 µs nested"
doorbell cost in `V3_WINDOWS_DOORBELL_RESEARCH.md` line 126 apply to the whole guest (that
note's own figure).

### (c) Conditions under which a vIOMMU becomes necessary

- A customer or policy requires **VBS level 3**, **Kernel DMA Protection ON**, or **automatic** Memory
  Integrity, and nothing else will satisfy the auditor.
- An anti-cheat or DRM product that checks IOMMU *and* tolerates VMs appears (none found).
- Microsoft makes `IoMmuSecureModeRequired`-style isolation a default for Windows Sandbox / Application
  Guard / future "secure VM" features on discrete GPUs, so that Dxgkrnl insists on an IOMMU before
  starting the adapter or the secure VM (the staging key is "disabled by default" today; this can
  change).
- Windows' **Windows 11 22H2 DMA-remapping** path is forced for our GPU (e.g. a guest with more RAM than
  the GPU's address reach). Our model reports its own visible-address bits, so this is avoidable by
  declaring them wide (INFERRED).
- A stacked scenario needing a guest IOMMU for its own sake: nested DDA or nested VFIO of our emulated
  GPU; a Linux guest running `intel_iommu=on` strict (that is the Linux, not the Windows, case, and
  the `v3-viommu` branch already refuses unsupported regimes).

### (d) Unknowns

1. **Does the NVIDIA Windows driver set `IoMmuSecureModeSupported`, `DXGK_IOMMU_CAPS` and
   `MapAperture2Supported`?** Public documentation says discrete WDDM 3.0 drivers must; the driver is
   closed and we do not reverse-engineer it (`feedback_reverse_engineering_boundary`). The only
   legitimate way is behavioral: boot the guest with a DMAR table and observe whether Dxgkrnl attaches
   an IOMMU domain and what addresses RM hands the device.
2. **VBS/HVCI on our exact stack** (AMD host, `-machine pc`, OVMF, vTPM, enlightenments) is untested
   by us; all KVM evidence is Intel-leaning Proxmox threads. Needs: `kvm_amd nested`, `svm`, nested
   Hyper-V on, `Win32_DeviceGuard`.
3. **OVMF and ACPI**: whether any firmware change is needed for DMAR/IVRS; whether Windows accepts a
   QEMU `amd-iommu` IVRS at all (no report found). Also whether Windows tolerates `caching-mode=on`.
4. **Whether HVCI + the NVIDIA driver + kf3 traps interact** (MMIO traps under a hypervisor-in-the-way;
   page-protection of the driver's BAR mappings).
5. **Professional-app survey** was a search, not a test; ISV documents (Autodesk, SolidWorks, Adobe,
   Synopsys/Cadence) were not read individually.
6. **Anti-cheat details**: Vanguard's IOMMU check method is unpublished; the FACEIT official FAQ was
   unreachable (HTTP 403), so FACEIT facts are second-hand; dates for 2026 changes come from news
   sites.
7. **Windows build dependence**: VBS rules and default-on criteria changed in 2026 (Memory Integrity
   default expansion from October 2026, Vanguard requiring Windows 11 25H2); the guest here is
   build 26100 (24H2). Re-check on 25H2.
8. **Interrupt remapping**: whether Windows needs it when the guest has more than 255 vCPUs; not
   relevant to current sizes.

## 5. Recommended next steps (cheapest first)

1. Record the `Win32_DeviceGuard` baseline in the current Windows guest (all fields), and the
   `msinfo32` "Kernel DMA Protection" line, so later arms have a comparison. No code change.
2. Arm B run: nested set above, VBS level 1, Memory Integrity on; observe boot, TDR behavior and
   BAR0-trap rate. Read the arm-B notes in `V3_WINDOWS_DISCOVERY.md` first.
3. WSL2 smoke (`wsl --install`, `wsl -l -v`) and Docker Desktop in the same guest.
4. Only after 1-3: the one-day spike already scoped in `V3_VIOMMU.md` §4, now with the Windows
   question first (q35 + `intel-iommu,intremap=on` on a *scratch* image: does Dxgkrnl attach a
   domain, does the GPU still start).
5. Fold the WDDM correction (row 8b) into `V3_VIOMMU.md` above its first paragraph, per doc hygiene.

## 6. Sources (all read 2026-10-13)

Microsoft Learn: windows-11-requirements; oem-vbs; enable-virtualization-based-protection-of-code-integrity;
oem-hvci-enablement; credential-guard index; how-hardware-based-root-of-trust-helps-protect-windows;
kernel-dma-protection-for-thunderbolt; oem-kernel-dma-protection; WDDM iommu-model,
iommu-based-gpu-isolation, iommu-dma-remapping, DXGK_VIDMMCAPS; pci/enabling-dma-remapping-for-device-drivers;
nested-virtualization; wsl/faq; windows-sandbox; plan-for-deploying-devices-using-discrete-device-assignment.
QEMU: docs/system/i386/hyperv.html; docs/devel/memory.html; patchew intel-iommu docs patch; amd_iommu
dma-remap commit. Kernel: docs.kernel.org vfio. Forums and news: Proxmox threads (Memory Integrity /
nested Hyper-V / VBS in Windows guests), NixOS discourse, Unraid forum, Riot support, finalboss.io,
refrag.gg, guru3d, thesixthaxis, stigaview (DISA STIG). Full URLs are inline in section 2 and 3.
