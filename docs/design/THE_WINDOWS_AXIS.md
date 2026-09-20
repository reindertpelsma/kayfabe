# The Windows axis — what a Windows guest actually changes

**STATUS: LIVE, 2026-09-20 (w821).** First survey of the `OS` axis (§0 of
`THE_ARCHITECTURE_v3.md`), which until now carried **zero coverage**. Sources: ogkm 610.43.02
and 580.159.04 (the shared RM core), plus lawfully published Microsoft/NVIDIA documentation,
vendor bulletins, open-source projects and conference talks.

⊘ **No leaked source or breach material was used.** Two cited items are published security
research that reverse-engineers a *shipped binary* (Google Project Zero; a public patcher repo) —
legitimate published work, flagged where used. One conference deck was encountered and
deliberately **not** relied on; nothing here depends on it.

---

## 0. ★★★ Why this is answerable at all — and exactly where the oracle stops

`[owner, 2026-09-20]` *"NVIDIA open-sourced ogkm under MIT/GPL and still had Windows code in it —
that's so valuable. They could have stripped it with `#if` blocks and then stripped when
distributing source for Linux, but they didn't."*

★ **Correct, and the reason is structural rather than an oversight.** `src/nvidia/` is **one RM
codebase compiled for several operating systems**. Stripping per-OS for the public drop would mean
maintaining a *divergent* public tree — permanently more expensive than shipping the seams. So
the seams ship: **40** `RMCFG_FEATURE_PLATFORM_WINDOWS` sites in 610, **45** `NVOS_IS_WINDOWS`,
~86 WDDM/LDDM comments, and Windows-only control commands documented as such in the SDK headers.

⇒ **ogkm is a genuine partial oracle for Windows behaviour** — the only one that exists, since
nouveau and envytools document the hardware and the Linux blob and say nothing about `nvlddmkm`.

⊘ **And here is precisely where it stops**, because the distinction decides which claims in this
document are sound:

| we CAN see | we CANNOT see |
|---|---|
| ★ **Policy and decisions** in the shared core — which branch is taken on Windows, and why | ⊘ **The Windows OS layer.** `arch/nvalloc/win/` is absent; only `unix` is published |
| Windows-only **control commands and their semantics** (`UPDATE_PDE_2`, `RESERVE_ENTRIES`) | ⊘ **The Windows `rmconfig` profile** — `rmconfig.h` hard-sets `RMCFG_FEATURE_PLATFORM_WINDOWS 0` and `NVOS_IS_WINDOWS 0` *unconditionally*, so every NVOC table here is the UNIX binding |
| **Comments** stating Windows behaviour in NVIDIA's own words | ⊘ Every `os*()` value on Windows — `osGetPageSize`, `osQueueDpc`, `osIsRaisedIRQL` |

★★★ **The `WindowsFirmwarePolicyArg` in §1 is the exact shape of that boundary, in one symbol:**
the **parameter** is public and threaded through the shared policy function, and the **struct it
points at is not defined anywhere in the published headers.** ⇒ They stripped the OS layer and
kept the seams *into* it. We can therefore see **that** a Windows-specific input exists and
**where** it applies — and never **what it contains**.

⚠ The practical rule this imposes on everything below: **a claim sourced from the shared core is
strong; a claim about what the Windows OS layer supplies is inference and is marked as such.**

---

## 1. ⊘⊘⊘ THE HEADLINE WAS WRONG — GSP is default-ON for Turing+, on every OS

**[RETRACTED w821, within the hour, by the owner: *"are you sure the stock Windows driver
doesn't turn GSP on when it's available? On Blackwell it's even required."*]**

The survey reported *"GSP is off by default on a stock Windows guest"* from community evidence.
⊘ **The shared RM core says otherwise, and it is the authority here** — the policy is computed in
`src/nvidia/src/kernel/gpu_mgr/`, which is **not** the Unix layer.

`gpumgrIsDeviceRmFirmwareCapable` (`gpu_mgr.c`):

```c
if (!hypervisorIsVgxHyper() && !_gpumgrIsRmFirmwareCapableChip(pmcBoot42))
    { bFirmwareCapable = NV_FALSE; goto finish; }     // capable ⇔ arch >= TU100
...
if (hypervisorIsVgxHyper()) { ...vGPU-specific chips... }
else                        { bEnabledByDefault = NV_TRUE; }   // ★ every other case
```

⇒ **For any Turing-or-newer chip outside a vGPU hypervisor, `bEnabledByDefault` is `NV_TRUE`.**
★ There is **no OS conditional in this function at all** — the sole platform carve-out is
PowerPC. And `gpumgrGetRmFirmwarePolicy` then requests firmware unless the registry *explicitly*
says `DISABLED`:

```c
*pbRequestFirmware = bFirmwareCapable &&
      (mode == MODE_ENABLED || (bEnableByDefault && mode != MODE_DISABLED));
```

### ⊘ And the registry key does not mean what the community advice implies

`nv-firmware-registry.h`: `MODE_DISABLED 0x0`, `MODE_ENABLED 0x1`, `MODE_DEFAULT 0x2`,
`POLICY_ALLOW_FALLBACK 0x10` — and the **default value is `0x12`**, i.e.
`MODE_DEFAULT | POLICY_ALLOW_FALLBACK`.

⇒ Setting `EnableGpuFirmware = 1` does **not** flip GSP from off to on. It sets `MODE_ENABLED`
(*force*) **and drops `ALLOW_FALLBACK`**, because `0x1` does not carry the `0x10` bit.

★★★ **Which supplies a better explanation of the community reports than "off by default":** GSP
was requested, **fell back to monolithic RM**, and setting `1` removed the fallback. The comment
in NVIDIA's own header describes exactly that mode — *"try to enable GPU firmware but fall back if
needed… this can result in a mixed mode configuration (ex: GPU0 has firmware enabled, but GPU1
does not)."*

### ⊘⊘⊘ AND THE RETRACTION ABOVE OVER-CORRECTED — the struct IS defined, and it changes the answer

**[CORRECTED AGAIN, same hour.]** I wrote that `WindowsFirmwarePolicyArg` was *"never read in the
open tree, and the struct is not defined in any published header."* ⊘ **The struct is defined**,
in `generated/g_gpu_mgr_nvoc.h:601`:

```c
typedef struct WindowsFirmwarePolicyArg {
    NvU32  devId;                            // PCI device id
    NvU32  ssId;                             // PCI subsystem id
    NvU32  bEnableGpuFirmwareOnWsServerSkus;
    NvBool bIsTccOrMcdm;
} WindowsFirmwarePolicyArg;
```

★★★ **Those four fields say what the Windows default depends on, and it is not uniform:**
the **SKU** (`devId` + `ssId`), whether this is a **workstation/server SKU** — matching the policy
bit `NV_REG_ENABLE_GPU_FIRMWARE_POLICY_DEFAULT_ON_WS_SERVER 0x20` in the registry header — and
whether the GPU is in **TCC/MCDM** rather than WDDM.

⇒ ★ **A per-SKU default with an explicit workstation/server enable is exactly the shape that
produces "GSP is on for my Quadro and off for my GeForce."** The community reports become
**plausible again**, for consumer SKUs specifically.

### ⊘⊘⊘ How I got the retraction wrong — I read a build with the branch compiled out

`gpumgrIsDeviceRmFirmwareCapable`'s body in this tree **never references `pWinRmFwPolicyArg`** —
it only writes `pbEnabledByDefault`. I read that body, saw no OS conditional, and concluded
*"there is no OS branch in the decision."*

⊘ **But `rmconfig.h` hard-sets `RMCFG_FEATURE_PLATFORM_WINDOWS 0`, so what I read is the
Linux-compiled view of a function whose Windows logic is absent from the public drop** — NVIDIA
kept the signature and the struct and stripped the body that uses them. ⚠ **The absence of a
Windows branch in a build where Windows is disabled is not evidence that no Windows branch
exists.** ★ Third instance of this tree's own lesson in one session, in a third disguise:
*an empty result is evidence of nothing, not evidence of emptiness.*

### ✔ So what IS established, stated at the right strength

| claim | status |
|---|---|
| **Windows runs as a GSP client** | ★ **Proven.** `RMCFG_FEATURE_PLATFORM_WINDOWS && IS_GSP_CLIENT(pGpu)` at `gpu_registry.c:224` and `gpu_user_shared_data.c:335`, selecting a constant **named for that configuration**: `NV_REG_STR_RM_RUSD_POLLING_INTERVAL_WINDOWS_GSP 250` (vs `_DEFAULT 500`). Plus `bGspNocatEnabled`, a Windows-only field in the GSP boot RPC itself |
| **The Windows default is per-SKU and per-mode** | ★ **Proven from the struct's fields** — `devId`, `ssId`, WS/server, TCC/MCDM |
| **Whether a stock GeForce Windows guest defaults GSP on** | ⊘ **UNRESOLVED — and §1.1 establishes that NOBODY HAS MEASURED IT** |
| **GSP is required on Blackwell — on Linux** | ★ **Yes, and the mechanism is in this tree.** `[owner]` *"proprietary Linux doesn't work on Blackwell, so that infers GSP is required anyway."* Confirmed: the open module **refuses a non-firmware-capable GPU by name** — `osapi.c:3721` calls `gpumgrIsDeviceRmFirmwareCapable` and on `NV_FALSE` prints *"installed in this system is not supported by open nvidia.ko"*. ⇒ Blackwell + open-modules-only + openrm-is-GSP-only ⇒ **GSP required** |
| **…and on Windows** | ⊘ **Does not transfer.** `nvlddmkm.sys` is not the open module, so the Linux chain says nothing about it. ⚠ But `devId`/`ssId` are policy inputs, so a per-SKU Blackwell default is expressible either way |

### 1.1 ⊘⊘⊘ What the public record actually contains — searched w821

`[owner]` *"maybe find on the internet. GSP Windows Turing+."* Done. The result is not an answer;
it is the discovery that **there is no published answer, and the folklore has no measured basis.**

**1. NVIDIA has published nothing about Windows.** The authoritative source is the driver README's
GSP chapter, and it is **Linux-only**: *"The GSP firmware will be used by default for all Turing
and later GPUs"* — stated for the Linux driver, configured by the **kernel module parameter**
`NVreg_EnableGpuFirmware`. ⊘ The document **makes no reference to Windows at all**, and NVIDIA
documents no Windows registry setting, Control Panel option or app control for it.

**2. ★★★ The community claim has no before-state behind it.** The primary thread — the one every
later citation traces back to — contains **zero documented observations of the default state**.
Posters report results *after* setting `EnableGpuFirmware=1`; nobody recorded what `nvidia-smi -q`
said **before**. ⇒ *"GSP is off by default on Windows"* is an **inference from the existence of a
registry key that people found worth setting**, not a measurement.

**3. ⚠ And the "N/A means disabled" reading is borrowed from the wrong document.** It comes from
NVIDIA's **Linux** README (*"a valid version if GSP firmware is enabled, or N/A if disabled"*)
applied to Windows `nvidia-smi`. ⊘ That is precisely the misapplication this tree already has a
lesson for: an unpopulated field is not a measured zero.

**4. ◐ One Blackwell datapoint cuts the other way.** A user on an **RTX 5090, driver 581.94**
reports a watchdog error that *"immediately stopped after disabling GSP"* — which implies GSP was
**on**, and that it is **disableable** on Blackwell/Windows. ⚠ Forum-grade, single report, and it
does not distinguish "on by default" from "on because they had enabled it".

**5. ★★★ The claim traced to its root — one user, one card, 2023, no vendor reply.** The NVIDIA
Developer Forums thread every chain leads back to is *"Enable GSP on Windows 11 on my 2080 Ti?"*,
**26 November 2023**. The poster observes `GSP Firmware Version : N/A`, finds `gsp_tu10x.bin` in
the DriverStore, and writes *"so it is disabled by default, but the driver has GSP binary."*
⊘ **No NVIDIA staff member replied.** One card, no driver version, no before/after, and the
conclusion rests entirely on the `N/A` reading. ⇒ **That single post is the origin of the whole
claim.**

★ **But the datapoint is at least ON-POINT, and it is worth saying why.** The RTX 2080 Ti is
**TU102 — Turing**, so it sits *inside* the capable set (`arch >= TU100`, §0.0.1); pre-Turing
would be Pascal and earlier. ⇒ The `N/A` is **not** explained away by "this die has no GSP".
★★★ And the blob the poster found is named **`gsp_tu10x.bin`** — firmware for *that very chip
family*, shipped in the Windows DriverStore. ⇒ The card was capable **and** its firmware was
present **and** it still reported nothing. That is the right kind of observation; what it lacks is
replication and a known-positive, not relevance.

**6. NVIDIA's ONLY published sentence connecting Windows and GSP is scoped to a different
product.** In the vGPU user guide's *"Disabling GSP Firmware"* task: *"For NVIDIA vGPU deployments
on Linux and all NVIDIA vGPU software deployments on Windows, omit this task."* ⊘ That is about
**NVIDIA vGPU software deployments** — the licensed vGPU/GRID product — not a stock GeForce
Windows guest. ⚠ It is suggestive (you would not tell people to skip a disable step for something
that is on and problematic), but it does not address our configuration and must not be stretched
to.

### ⊘⊘⊘ And the precise reason the claim is untested: it has no known-positive

The `N/A` inference may well be **right** — `nvidia-smi` is NVIDIA's own tool and largely shared,
so an empty field plausibly does mean "no GSP version to report". ⊘ **It has simply never been
checked against a known-positive**: nobody has published a Windows card reporting a GSP version
**without** the registry key, on any SKU.

★ And this tree already names that requirement: `a_census_zero_needs_a_known_positive`. A zero from
an instrument nobody has seen produce a non-zero, in that configuration, is not a measurement.
⚠ The guru3D posters who set the key *and then saw a version* prove the **instrument works on
Windows** — which is exactly what makes the missing observation cheap to take and inexcusable to
keep inferring.

⇒ ★★★★★ **The convergence is the useful part.** The open tree says the Windows default is decided
**per-SKU and per-mode** (`devId`, `ssId`, WS/server, TCC/MCDM). The public record says **nobody
has measured it**. Those two facts fit together exactly: *a per-SKU default is what produces
contradictory folklore*, because different people are correctly reporting different cards.

### ✔✔✔ SETTLED w821 — the owner measured it, and the known-positive was already on the record

> `[owner, 2026-09-20]` *"Confirmed. On an RTX 1660 Ti Windows PC, GSP firmware is N/A. It's
> real."*

★★★ **That closes it, and the reason is worth spelling out** — the measurement that was missing
was never the `N/A`; it was the **known-positive**, and the record already contained one:

| leg | evidence |
|---|---|
| the instrument **does** report a version on Windows | ★ the forum posters who set `EnableGpuFirmware=1` **and then saw a version**. ⇒ Windows `nvidia-smi` populates that field when there is something to report — so an `N/A` is **not** an unpopulated field |
| unmodified consumer Turing reports **nothing** | **RTX 1660 Ti (TU116)** `[owner, measured]` · **RTX 2080 Ti (TU102)** `[forum, 2023]` — two independent Turing consumer dies |
| both are **inside** the capability set | `arch >= TU100` (§0.0.1), and `gsp_tu10x.bin` ships in the DriverStore |

⇒ ✔ **GSP is off by default on consumer Turing under Windows.** `[MEASURED]`

⚠ **What is still NOT established**, and should not be quietly generalised: workstation/server
SKUs (`bEnableGpuFirmwareOnWsServerSkus` exists precisely because they may differ), Ada and
Blackwell consumer parts, and TCC/MCDM mode. ⇒ The struct says the default is **per-SKU**; we have
now measured **one corner of that space**, not the space.

★ And a note on how this resolved, because the pattern recurs: I spent three exchanges arguing
about the `N/A` reading when the thing that settled it was **the other leg of the instrument
check**. The known-positive existed in the same threads I had already read. ⊘ *A census zero needs
a known-positive* — and I kept re-examining the zero.

**Sources:**
[NVIDIA 580.65.06 GSP chapter](https://download.nvidia.com/XFree86/Linux-x86_64/580.65.06/README/gsp.html) ·
[guru3D: Enable GSP Firmware on Windows](https://forums.guru3d.com/threads/enable-gsp-firmware-on-windows.455714/) ·
[guru3D: How to disable NVIDIA's GSP Firmware on Windows](https://forums.guru3d.com/threads/how-to-disable-nvidias-gsp-firmware-on-windows.455267/) ·
[Overclock.net: RTX 5090 GSP Firmware](https://www.overclock.net/threads/rtx-5090-what-is-this-gsp-firmware-all-about-should-i-enable-this-hidden-feature.1817406/)

⚠ **And one thing the "Turing+" framing in that search should not be allowed to blur** `[owner:
"pre-Turing it can't be used"]`: **Turing+ is the HARDWARE capability gate**
(`_gpumgrIsRmFirmwareCapableChip`, `arch >= TU100`), not a Windows policy statement. Below Turing
there is no GSP to enable on any OS. ⇒ Everything uncertain in this section sits **inside** the
capable set, in the *default-on policy* — see `THE_ARCHITECTURE_v3.md` §0.0.1.

⇒ ★ **The practical position:** design for GSP being present, **detect and refuse the alternative**
(§1's fallback detector), and treat *"stock GeForce Windows defaults GSP off"* as an open risk to be
measured — not as a settled premise in either direction.

### ★★★ How the survey got it wrong, and it is this tree's own documented failure

The community evidence rested partly on `nvidia-smi -q` reporting `GSP Firmware Version : N/A`.
⊘ **That is not evidence GSP is off. It is evidence the field was not populated.**
⚠ This repo already carries that lesson, paid for once: *an empty capture is evidence of NOTHING,
not evidence of emptiness* — the `dlen=0` rows whose every checked instance was **contradicted** by
real hardware. ⇒ The survey reproduced the exact error the tree documents, from a different
direction.

### ★★ What replaces it, and it matters on every OS

⊘⊘ **The default policy ALLOWS SILENT FALLBACK TO MONOLITHIC RM.** That is `0x12`'s `0x10` bit,
and it is the default on Linux too.

⇒ **If a guest's RM falls back, our fake GSP is never used at all** — monolithic RM drives our
emulated registers directly, and every RPC-based assumption in this document evaporates **with no
error and no message**. ★ **[PROPOSE]** we must be able to detect that and refuse it by name. The
detector is cheap: a guest that has fallen back **never publishes a message queue** and never sends
`SET_GUEST_SYSTEM_INFO`, so *"boot progressed past the point where an RPC was due, and no queue
exists"* is a nameable, checkable state rather than a hang.

⇒ **Net effect on the Windows question:** §1 no longer argues that the premise fails. It argues
that GSP is requested on Turing+ regardless of OS, that a **fallback** path exists and is enabled
by default, and that whether NVIDIA's Windows driver takes it is **[UNVERIFIED]** and gated by a
struct we cannot see. ⚠ **Sections 2–4 below are unaffected** — they are about WDDM's ownership of
the page tables, TDR, and the absence of UVM, none of which depends on how GSP is enabled.

## 2. Under WDDM the OS owns the page tables — and we never see the writes

Microsoft, verbatim: *"**VidMm manages the GPU virtual address space of all processes. VidMm is
also responsible for allocating, growing, updating, ensuring residency, and freeing page
tables.**"* NVIDIA's side matches: VA spaces are `SHARED_MANAGEMENT` (*"management of the VA space
is shared with another component (e.g. driver layer, OS…)"*, `nvos.h:1384`), and ranges are
`NVOS32_ALLOC_FLAGS_EXTERNALLY_MANAGED` — *"Page tables for this allocation will be managed
outside of RM."*

⊘⊘⊘ **And the transport for individual PDEs is invisible to us.**
`NV0080_CTRL_CMD_DMA_UPDATE_PDE_2` (`0x80180f`) is documented **"only available on Windows and
MODS platforms"** and **formats the PDE into the caller's own buffer** rather than RM's directory.
Critically, it is **handled purely in guest CPU-RM and is NEVER RPC'd to GSP for a GSP client**
(`dma.c:320-324` RPCs only under `IS_VIRTUAL_WITHOUT_SRIOV`; `:340-355` does the work locally).
Same for `NV90F1_CTRL_CMD_VASPACE_RESERVE_ENTRIES` (`gpu_vaspace.c:4109`).

⇒ **The address model in Part 1 §4 assumes RM authorship of page tables. On Windows that
assumption is false**, and our fake GSP hears nothing.

★ **One thing survives:** `SET_PAGE_DIRECTORY` **is** RPC'd to GSP when `IS_GSP_CLIENT`
(`dma.c:507-521`). ⚠ But with different semantics — aperture typically `_VIDMEM` not
`_SYSMEM_COH`, arriving per device creation rather than per UVM VA space.

★★★ **And the silver lining, which is real:** unlike Linux, Windows *tells the driver every PTE it
wants written, explicitly and in order*, through `DxgkDdiBuildPagingBuffer` ops
(`UpdatePageTable`, `FlushTlb`, `MapApertureSegment`) submitted as **GPU-executed paging
buffers**. ⇒ If we can see paging-buffer submissions, that is a **richer** signal than the
doorbell-time page-table sweep the C artifact used. ⊘ **[UNVERIFIED]** whether NVIDIA's WDDM path
routes those through CE (visible to us), BAR1 CPU writes, or `UPDATE_PDE_2` (invisible). **This is
the single most important open question on this axis.**

---

## 3. TDR — a hard, OS-enforced 2-second deadline

| | Linux | Windows |
|---|---|---|
| graphics timeout | 4 s | ★ **2 s**, OS-enforced |
| compute timeout | 30 s | 2 s (5 s `TdrDdiDelay`) |
| consequence of exceeding | the driver soldiers on with stale TLBs | **adapter reset**; `VIDEO_TDR_FAILURE (0x116)` |
| repeat | — | ⊘ **five hangs in one minute ⇒ bugcheck `0x117`** |

RM's own comment concedes it: *"90 % of the overall hard limit of **2.0 seconds, imposed by WDDM
driver rules**"* (`os.c:2116`).

⇒ ⊘ **On Linux a slow emulator is merely slow. On Windows a completion path slower than 2 s is
diagnosed as a hung GPU.** ★ Cold boot is probably safe — device init gets a 60 s exception
(`thread_state.c:331`) — but **runtime is not**, and our data plane is currently a CPU copy loop.

⚠ Related and worse: Windows has a **first-class page-fault interrupt** carrying
`{FaultedVirtualAddress, PageTableLevel, FaultErrorCode, FaultedProcessHandle}` and the OS
*expects* an accurate fault path, using it to kill the offending context rather than the adapter.
⊘ The C artifact went green *"without servicing or forwarding a single GPU fault."* That is a
**Linux-only luxury**.

---

## 4. Four more that matter, briefly

- ⊘ **No UVM on Windows.** *"Currently, we don't support UVM on Windows/MODS"* (`pool_alloc.c:488`);
  in WDDM mode CUDA contexts are dxgkrnl contexts (`D3DKMT_CLIENTHINT_CUDA`). ⇒ The `UVM_*` ioctl
  stream **disappears** — and `UVM_MAP_EXTERNAL_ALLOCATION` is the lockstep boundary of our nvdiff
  oracle (221 of `cuCtxCreate`'s 479 ioctls).
- ⊘ **The OS manages the framebuffer** on non-TCC Windows — RM verbatim: *"Disabling in Non-TCC
  windows because **the OS manages FB**"* ⇒ **no scrub-on-free**, and RM stops choosing PTE kinds
  (`bRmToChooseKind = NV_FALSE`). ⇒ The scrub path in Part 5 is one a Windows guest largely does
  not take.
- ⊘ **Sysmem GPU mappings become 4 KB-granular** where Linux uses 64 KB/2 MB — a 16–512× PTE-count
  increase. ⚠ And Windows **guarantees** VA/PA 64 KB congruence and lets the driver *rely* on it, so
  a violating emulator faults with no obvious cause — the same class as this tree's existing
  congruence trap.
- ★ **`DEV_xxxx` must already be in NVIDIA's signed INF.** PnP binds by hardware id from a
  catalog-signed package, and *"even a single-byte change… invalidates the digital signature."*
  ⇒ **A gate before any of the above matters.**

★ **And one that lands directly on w821's Hopper work:** RM allocates the BAR1 doorbell region
**downwards from the top of BAR1 on Windows** — *"WAR for Bug 3564398, need to allocate doorbell
for windows differently"* ⇒ `BUS_MAP_FB_FLAGS_MAP_DOWNWARDS` for `MEMDESC_FLAGS_MAP_SYSCOH_OVER_BAR1`
(`mapping_cpu.c:506`), **the exact flag Hopper+ sets on the usermode/doorbell region**
(`kernel_fifo_gh100.c:123`). ⇒ Our BAR1 doorbell-page identification (Part 1 §2.1) must not assume
Linux allocation order.

---

## 5. ★★★ The escape hatch — TCC, and it collapses most of the table

**TCC mode is not WDDM at all** — it *"uses the Windows WDM driver model"* and disables graphics
(`nvidia-smi -dm 1`). ★ And nearly every Windows branch found in ogkm is gated on **`!TCC`**:
FBSR-WDDM mode, PMA client page tables, P2P mailbox, scrub-on-free. `nv_gpu_ops.c:2327` states it
directly: *"**Non-TCC mode on Windows implies WDDM mode.**"*

⇒ **In TCC: scrub-on-free returns, PMA-managed client page tables return, `EXTERNAL_HEAP_CONTROL`
is off, FBSR-WDDM is off, and dxgkrnl/VidMm/VidSch are entirely out of the loop.** §§2, 4 largely
evaporate. ⚠ TCC is not available on GeForce parts. (MCDM is the middle option — render-only, no
display, and it **requires an MMU**.)

**[PROPOSE]** If a Windows guest ever becomes a target, **TCC is dramatically the smallest one**,
and the order is: TCC → MCDM → WDDM.

---

## 6. ✔ What is the SAME — this bounds the work, and it is a lot

1. **The RM API ABI is identical** — `NVOS21/54/32/33/38`, handle semantics, class ids, control
   numbers. No Windows variants in `common/sdk/nvidia/inc/`.
2. **`NV01_EVENT_WIN32_EVENT` *is* `NV01_EVENT_OS_EVENT`** — literally a `#define`, value `0x79`.
3. ★ **The GSP boot sequence is OS-agnostic** — **zero** Windows conditionals in the bootstrap
   path; FWSEC/WPR2/booter/LibOS-args/msgq/`GSP_INIT_DONE` all shared. The only platform
   conditional makes Windows hold the API lock across *all* of GSP init ⇒ **strictly more serial,
   the easier case for us.**
4. **`GspSystemInfo` layout is identical**; 610 vs 580 differ by four fields — a version
   difference, not an OS one.
5. **`SET_REGISTRY` format is the same**; `nvrm_registry.h:24` says outright *"shared between
   Windows and Unix."*
6. ★★★ **The doorbell / work-submit path has NO Windows conditional in RM** — usermode region,
   `workSubmitToken`, `kfifoUpdateUsermodeDoorbell_*`, USERD, GPFIFO: **zero** Windows branches
   across `gpu/fifo/`. ⇒ **Part 1 §2 is OS-independent**, modulo §7's two unknowns.
7. **GMMU formats, PTE/PDE encodings, apertures, big-page selection** — chip-keyed, not OS-keyed.
8. **Hypervisor detection is shared and CPUID-based**, and ⊘ **there is no error-43-style refusal
   anywhere in the open RM core.**
9. **Robust channels / RC are enabled on Windows, GSP and Unix alike.**
10. **PMA is supported on all platforms** — WDDM disables *client page tables under PMA*, not PMA.

---

## 7. ★★★ A free gift — detecting a Windows guest from the FIRST RPC

`GSP_SET_SYSTEM_INFO` carries `NvBool bGspNocatEnabled`, and the **only** assignment in the entire
tree is:

```c
if (RMCFG_FEATURE_PLATFORM_WINDOWS) { rpcInfo->bGspNocatEnabled = NV_TRUE; }
```

`kernel_gsp.c:4624` (610) / `rpc.c:10650` (580) — present and identical in both generations.

⇒ **`bGspNocatEnabled == NV_TRUE` in the first RPC we ever receive ⟹ the guest RM was built for
Windows.** ⚠ It is an *enable* flag, not an OS field — **assert on it, do not build on it**. There
is no explicit OS identifier in `GspSystemInfo`.

★ **[PROPOSE] Implement this now, as a refusal, before any Windows work happens.** It costs one
comparison and converts *"a Windows guest silently does something strange"* into a named refusal at
the first message — which is exactly what `support_matrix_asymmetry` demands of an out-of-band
configuration.

---

## 8. ⚠ A correction to this project's own framing

`CLAUDE.md` describes nvkvm as *"WSL2-style NVIDIA GPU ioctl/RPC forwarding."* ⊘ Publicly, WSL2's
mechanism is **D3DKMT-shaped thunks over VM Bus to a host `dxgkrnl`** — the Windows Graphics Kernel
lead, XDC 2020: *"Level of abstraction is the **WDDM interface**… **Not a straight pass-through**…
**No data copy — only control information exchanged over VM bus**."*
⇒ **Mode 1 resembles WSL2's *shape*, not its *protocol*.** The phrase should be corrected where it
appears, because it invites the assumption that a documented design exists for what we do.

★ Related, and worth knowing: Microsoft's own answer to our problem statement is **not to run the
vendor KMD in the guest at all** — GPU-PV puts *"no KMD in the guest, only UMD… no video memory
manager (VidMm) or scheduler (VidSch) in the guest."* ⇒ **The industry cut line is at D3DKMT, not
at the hardware.** We are deliberately below it, and that is the thing that makes this project
unusual — and the reason no prior art exists for emulating an NVIDIA GPU to a Windows guest.

---

## 9. What could NOT be determined, and the first measurements to take

| question | status |
|---|---|
| The Windows OS layer itself (`osGetPageSize`, `osQueueDpc`, `osIsRaisedIRQL`, …) | ⊘ `arch/nvalloc/win/` is not published. **The 4 KB page-size finding rests on an inferred `osGetPageSize()==4096`** |
| The Windows `rmconfig` profile — which chips, classes, engines and HAL bindings | ⊘ openrm hard-sets `RMCFG_FEATURE_PLATFORM_WINDOWS 0`; every NVOC table here is the UNIX profile |
| Is the **GSP firmware image byte-identical** between Windows and Linux packages? | ⊘ Nothing speaks to it. If it differs, our WPR2/radix3 sizing constants do too |
| Is the **GSP-RM RPC protocol** the same on Windows? | ◐ **inference only** (one RM codebase). ⊘ **Do not assume our msgq work transfers without measuring** |
| What do dxgkrnl/VidMm actually issue, and in what order — CE, BAR1 CPU writes, or `UPDATE_PDE_2`? | ⊘ **The most important open question on this axis** (§2) |
| Is display mandatory for `nvlddmkm` to load? | ⊘ Cosmetic on Linux, plausibly fatal on WDDM. TCC/MCDM sidestep it |
| Does NVIDIA enable WDDM 3.2 **user-mode doorbells**? | ⊘ If so, the guest rings a doorbell from user mode with **no kernel transition** — a trap-per-doorbell design becomes a per-write exit on a user-mode hot path |
| Was the Code-43 CPUID check **removed** at R465, or gated? | ⊘ NVIDIA said a narrow configuration is *supported*; it never said the check was removed |

★ **[PROPOSE] The first two measurements, in order:**
1. **`D3DKMT_QUERY_GPUMMU_CAPS` on real hardware** — `VirtualAddressBitCount`, `PageTableLevelCount`,
   4 K vs 64 K segments, GpuMmu vs IoMmu. Cheap, and it decides §2 and the page-size finding.
2. **Check our emulated `DEV_xxxx` is already in NVIDIA's signed INF.** If it is not, nothing else
   on this axis is reachable with a stock guest.

---

## 10. ★ Could we support a GSP-DISABLED guest? — nouveau measured, w821

`[owner]` *"Maybe we can wire disabled GSP in kayfabe. Nouveau source has the registers that tell
us what NVIDIA writes outside GSP… worth looking from a nouveau clone how no-GSP would look."*

Cloned (`research_clones/nouveau-src`, sparse, 31 MB) and measured.

### 10.1 ✔ nouveau IS the right oracle, structurally

★★★ **nouveau implements both paths side by side, and they are separable by filename.** For each
subdevice there is a native per-chip file and a GSP-client file:

```
nvkm/subdev/devinit/tu102.c     ← native: drive the registers ourselves
nvkm/subdev/devinit/r535.c      ← GSP: ask the firmware
nvkm/subdev/gsp/rm/r535/*.c     ← the whole GSP-RM client (rpc.c, fifo.c, gr.c, disp.c)
```

⇒ A per-subdevice diff of `tu102.c` against `r535.c` shows **exactly what the driver must do
itself when there is no GSP**. That is a real, cheap, repeatable experiment.

### 10.2 The measurement

| | lines of C |
|---|---|
| `nvkm` total (the hardware abstraction) | **130 118** |
| — `subdev/` (init and management) | 66 964 |
| — `engine/` (runtime) | 56 863 |
| the **entire GSP subdev**, including the RPC client | **10 957** |
| native Turing+ **per-chip deltas** | 7 137 |

⚠ **The 7 137 is misleading and must not be quoted as the cost.** Turing's native support is a
thin delta on an inheritance chain — `fb/tu102.c` pulls `gf100_fb_dtor`, `gm200_fb_init`,
`gp100_fb_init_unkn`. ⇒ Emulating a native driver means the **whole accumulated register
interface**, not the per-chip delta.

★ **But the honest delta is much smaller than "all of it", for two reasons:**

- ⊘ **The runtime plane is GSP-independent and carries over unchanged.** `fifo` (7 906) and `ce`
  (756) are hardware, not firmware — consistent with ogkm, where `gpu/fifo/` has **no GSP
  conditional** beyond which side generates the token. ⇒ **Everything in Part 1 §2 — the doorbell
  plane, the token table, the trap classifier — is reused as-is.** The delta is **init only**.
- ◐ Roughly **22 400 lines** of `subdev/` is board and power management — `clk` 6 189, `bios`
  8 033, `i2c` 3 058, `therm` 2 614, `volt` 1 023, `gpio` 855, `mxm` 690. An **emulated** GPU has
  no board to manage; much of this stubs rather than ports.

⇒ **Order-of-magnitude delta: ~30 k lines of register-level init semantics**, of which we already
model a slice (`fb`, `instmem`, `bar`, `mmu`, `mc`, `fault`). ⚠ **A line count is a proxy for
surface, not a port estimate.**

### 10.3 ⊘⊘⊘ The real cost is not lines — it is losing the oracle

★★★★★ **nouveau documents the HARDWARE. It does not document NVIDIA's driver.**

Our entire method is *"be what NVIDIA's driver expects."* On the GSP path we have **ogkm — actual
NVIDIA source** telling us what the guest will send and what it does with the reply. ⊘ **For the
native Turing+ path, openrm contains nothing**: it is GSP-only. So a no-GSP mode means emulating
what NVIDIA's **closed monolithic RM** writes, in what order, with nouveau serving only as a
*register dictionary*.

⚠ And nouveau's native Turing support is **known-incomplete** — no reclocking, signed-firmware
limits. ⇒ *"nouveau can init Turing natively"* does **not** imply *"nouveau does what NVIDIA's RM
does."* It is a dictionary, not a behavioural trace.

### 10.4 What it would buy, stated fairly

- ✔ A Windows guest with GSP off, **if** that configuration is real (§1 — still unmeasured).
- ★★★ ⚠ **It dissolves the Turing floor.** `THE_ARCHITECTURE_v3.md` §0.0.1 argues Turing+ is an
  *architectural* boundary because below it there is no GSP to impersonate. A no-GSP mode makes
  **pre-Turing reachable**. ⇒ That is either a significant widening of the product or scope creep,
  and it is an **owner decision** — flagged because it was probably not the intent of the question.
- ★ **It deletes the entire GSP boot fiction** — FWSEC, WPR2, the booter, LibOS args, the msgq,
  the radix3 ELF, and the 139 821 PROM/VBIOS reads in `cap1_coldboot_hermetic`. ⚠ That fiction has
  cost real time (WPR2 state not resetting across boots; five `RmInitAdapter` cycles per launch),
  so the deletion is worth something concrete.

### 10.4a ⊘⊘⊘ THE ORACLE OBJECTION IS ANSWERED — mmiotrace, and it is still in mainline

**[w821]** §10.3 argued the real cost of a no-GSP mode is **losing the oracle**, since openrm has
no native Turing+ path and nouveau is only a register dictionary. `[owner]`:

> *"Nouveau has exact traces of what the proprietary driver does. Plus we can trace: we shadow-
> overwrite the MMIO map function in Linux so it all lands in a trap window, then we observe every
> trap and we know what it calls, what we must implement. If it's only init it's trivial."*

★★★ **That is `CONFIG_MMIOTRACE`, it is the technique nouveau was built with, and it is alive in
mainline today.** Verified against a kernel tree cloned **2026-09-20**: `arch/x86/mm/kmmio.c`,
`mmio-mod.c`, `testmmiotrace.c`, `Documentation/trace/mmiotrace.rst` — whose own opening reads
*"built for reverse engineering any memory-mapped IO device **with the Nouveau project as the
first real user**"*, linking to `nouveau.freedesktop.org/wiki/MmioTrace`.

⇒ ✔ **The objection does not stand.** We are not limited to nouveau's *knowledge* — we can
**generate the trace ourselves**, from the proprietary driver, on the exact chip we care about.
★ And the two sources compose into a complete oracle pair that neither is alone:

| source | gives |
|---|---|
| **nouveau** | ★ **semantics** — what each register *is*, the field layouts, the meaning |
| **mmiotrace** | ★ **behaviour** — what NVIDIA's driver actually writes, in what order, on this die |

⇒ **[PROPOSE] Adopt this as the standing method for the non-GSP plane**, exactly as the ogkm
differential is the standing method for the GSP plane.

⚠ **Its documented limits, so nobody discovers them at 3 a.m.:**
- **x86/x86_64 only** ✔ (our host is).
- ⊘ **It takes all but one CPU offline.** SMP tracing is unreliable and *silently* drops events.
  ⇒ We see a **uniprocessor** init, and concurrency is invisible. ★ Check the lost-event counter
  **every run** — this tree's own lesson about instruments that fail quietly applies directly.
- ⊘ It traps **`ioremap`'d kernel MMIO**. Userspace's `mmap` of the doorbell page does **not** go
  through `ioremap` and will not appear. ★ Irrelevant for init — which is the owner's point
  (*"if it's only init it's trivial"*) — and a **hard limit** for anything else.

### 10.5 **[PROPOSE]** A spike, not a commitment — and the decisive hour

⊘ Do not adopt this from a line count. ★ **Run the smallest decisive experiment first:** diff
`nvkm/subdev/devinit/tu102.c` against `nvkm/subdev/devinit/r535.c`. `devinit` is the *first* thing
either path does, it is small, and the diff shows the shape of every other subdevice's split.

⇒ If that diff reads as *"a bounded register sequence we could answer"*, the idea is live and the
next question is the oracle gap in §10.3. If it reads as *"arbitrary board bring-up"*, it is
answered, and the answer took an hour.

⊘ **[SUPERSEDED w821]** This section previously ended *"the prerequisite is still §1's one-hour
measurement; building a no-GSP mode for a configuration nobody has confirmed exists would be the
most expensive way to answer a question one command settles."* ✔ **The measurement was taken and
the configuration is real** (§1). The prerequisite is discharged.

### 10.6 ✔ OWNER RULING, w821 — the option stays open, and nouveau is the standing oracle

> *"Pre-Turing is still significant… So we get both guaranteed Windows and pre-Turing. Just ensure
> this remains open. And keep nouveau now as oracle for everything without GSP."*

| ruling | consequence |
|---|---|
| ✔ **The no-GSP mode stays OPEN** | Not a spike to be closed out. It is carried as a live design option, and §§2–9 of this document are its requirements list |
| ★★★ **Pre-Turing is a GOAL, not scope creep** | ⇒ `THE_ARCHITECTURE_v3.md` §0.0.1 is **amended**: the Turing floor is architectural *for the GSP plane*, and a no-GSP plane is exactly what lifts it. **Two planes, two floors** |
| ✔ **nouveau is the standing oracle for everything without GSP** | Same status ogkm holds for the GSP plane. `research_clones/nouveau-src` (sparse checkout of `drivers/gpu/drm/nouveau`) |
| ✔ **Mine ogkm for the leftovers** | Whatever residue it holds on Windows behaviour or non-GSP behaviour gets recorded; **the rest is nouveau** |
| ★★★★★ **GSP REMAINS THE PRIORITY TARGET** | `[owner]` *"it will be our more stable version as we have more source available."* ⇒ The no-GSP plane is **additive reach, never a replacement** |

★★★ **And that ranking is exactly right, for the reason §10.3 gave — which was wrong as a
blocker and right as a priority.** The two planes do not have equal evidence:

| plane | oracle | strength |
|---|---|---|
| **GSP** | **ogkm — actual NVIDIA source** | ★ tells us what the guest *will* send and what it does with the reply, **before** we build anything |
| **no-GSP** | nouveau (semantics) + mmiotrace (behaviour) | ◐ tells us what *one driver* did on *one die* in *one uniprocessor boot* |

⇒ ⊘ A trace is a **sample**; source is a **specification**. mmiotrace closes the gap enough to
make the no-GSP plane *tractable*, and not enough to make it *equally trustworthy*. ★ Stability
follows evidence, so GSP leads and the no-GSP plane follows it — and where the two planes share
machinery (the whole of Part 1 §2: doorbells, channels, pushbuffers), **the GSP plane's design is
the one that sets the shape.**

★ **And one corroboration the owner supplies from the sibling project:** `nvkvm-pv` established
that for the **userspace ioctl** surface, pre-Turing constants **are published in ogkm**, and it
got pre-Turing working on a subbranch. ⚠ Scope that precisely: it establishes the **constants**
exist, on the **Mode-1 ioctl** plane. It does **not** establish that per-chip *register-level HAL
implementations* for pre-Turing are present — that is a separate question, and it is Task B of the
ogkm residue survey now running.
