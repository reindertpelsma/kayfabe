# Mode-2 ABI/architecture-agnostic layer — the versioning research

**Status:** design + research, 2026-07-22. Branch `consolidation`. Foundational **pre-rewrite**
research: the study that decides whether `nvkvm` Mode-2 survives NVIDIA's release cadence, and the
design that keeps the platform-agnostic Rust core from *ever* hard-coding a driver version or a GPU
architecture.

**Read first:** `mode2_rust_rewrite_architecture.md` (esp. Part 4's `nvkvm-abi` crate, §4.2 "ABI
codegen (how, concretely)", and lesson **L11 — ABI hardcoding is a standing tax; codegen from the
open kernel modules**). This doc is the deep-dive on the ABI/versioning layer that Part 4 names but
does not derive.

**Companions:** `mode2_address_table.md` (the GMMU/PTE consumer of Axis B), `nvidia_gpu_internals.md`,
memory `multi_driver_validated` (575 + 580 both HW-validated, auto-detect across 3 majors),
`multi_driver_support` (closed + open both must work; **the open driver is stricter — treat as
canonical**), `nvos64_abi_fix`, `abi_struct_truncation`, `gvisor_nvproxy_learnings`,
`rewrite_horizon_target` (pillar 4: codegen the ABI).

All bare `gvisor/...` and `research_clones/ogkm/...` cites are into the **vendored** trees at HEAD.
The vendored ogkm checkout is driver **610.43.02** (`research_clones/ogkm/version.mk:1`,
`NVIDIA_VERSION = 610.43.02`) — a *single snapshot*, which is itself a finding (see §2.4). Uncertain
claims are marked **"ASSUMPTION — verify."**

---

## TL;DR

The research-C proved Mode-2 works at host parity single-process; feasibility is **done**. The
dominant risk to a *product* is not capability, it is the **maintenance treadmill**: NVIDIA ships a
new driver every few weeks and a new GPU architecture every ~18 months, and if re-targeting is
expensive the project dies of entropy (L13), not of impossibility.

The core insight is to split the versioning surface along **two axes that version differently** and
must be handled differently:

- **Axis A — the DECLARATIVE ABI** (ioctl numbers, struct layouts/sizes, class IDs, control-command
  IDs, RPC message shapes). Scoped to **driver release** (575 vs 580 vs 610 — the project already
  hit this). **~85–90 % mechanically codegen-able** from the open kernel modules' *already-generated*
  headers. A new driver version costs a regenerate + a small hand-curated allowlist delta.
- **Axis B — the ARCHITECTURAL BEHAVIOR** (MMU/PTE format version, page sizes, USERD/RAMFC/instance-
  block layout, doorbell/work-submit-token/CHID encoding, GSP presence + RPC semantics, engine/
  runlist model, confidential compute). Scoped to **GPU generation** (Turing → Ampere → Ada → Hopper
  → Blackwell). **Behavioral, ~15 % of the surface but ~80 % of the risk** — not fully codegen-able;
  must live behind **arch traits**, one impl per generation.

**Conflating the two is the trap.** A layer that keys everything on "driver 580" re-does architecture
work every driver bump; a layer that keys everything on "Ampere" re-does struct work every driver
bump. The nvproxy reference (`gvisor/pkg/sentry/devices/nvproxy/version.go`) proves the Axis-A half is
tractable: it supports **17 driver versions** in one 1,242-line file via *inherit-then-mutate* deltas
of 14–51 lines each. This doc adopts that pattern for Axis A and adds an `Arch` trait set for Axis B.

**The single biggest version-agnosticism win:** a new *driver version* becomes a regenerate + an
allowlist diff (day-scale), because ~85–90 % of Axis A is codegen from headers NVIDIA already
generates for us. **The single biggest un-abstractable risk:** **Confidential Compute** (Hopper+/
Blackwell). When CC is *enabled in hardware*, on-silicon SPDM attestation rejects an unauthenticated/
faked GSP — the whole Mode-2 approach has a **hard ceiling** there. It is (today) an *optional* mode
that bare-metal GeForce ships **off**, so the near-term target is safe; but it is a strategic ceiling
to name honestly, not a bug to fix.

---

# 1. The two-axis versioning model

## 1.1 The axes

NVIDIA's surface changes along two independent clocks:

| | **Axis A — Declarative ABI** | **Axis B — Architectural behavior** |
|---|---|---|
| **Scoped to** | driver *release* (535, 575, 580, 610…) | GPU *generation* (Turing, Ampere, Ada, Hopper, Blackwell) |
| **Cadence** | every few weeks | every ~12–18 months |
| **What changes** | ioctl NRs, struct fields/sizes, class IDs, ctrl-cmd IDs, RPC message layouts, alloc-param sizes | MMU/PTE format, page sizes, USERD/RAMFC layout, doorbell/CHID encoding, GSP RPC semantics, runlist/TSG model, CC/attestation |
| **Nature** | **data** (shapes + numbers) | **behavior** (algorithms + protocols) |
| **Source of truth** | ogkm `generated/` headers + `sdk/nvidia/inc/` — NVIDIA *already codegens these* | ogkm HAL `.c` (`kern_gmmu_fmt_*`, kernel_channel, RAMFC) + our own RE ledger |
| **Handling** | **codegen** → versioned Rust ABI modules + version-dispatch | **abstraction** → `Arch` trait, one impl per generation |
| **Cost of a new instance** | regenerate + small allowlist delta (**cheap**) | one new trait impl (**bounded**) |
| **Codegen-able?** | ~85–90 % mechanical | ~15 % (formats as data); the rest is behavioral |

## 1.2 Why conflating them is the treadmill trap

The two axes are **nearly orthogonal in practice**: driver 575→580 on the *same* Ampere GPU changed
struct layouts and added control commands (Axis A) but touched **zero** MMU format code; conversely
Ampere→Hopper changed the PTE format from VER2 to VER3 (Axis B) while much of the ioctl surface
carried forward. The project has already lived both: the 575/580 staging pain (memory
`multi_driver_validated`, `guest_lib_version_staging`) was pure Axis A; the #13 GA10x 512M-leaf gap
(memory `mode2_13_multiiter_idle_hang`) was pure Axis B.

A design that does not separate them pays the cross-product tax: *N* drivers × *M* architectures of
hand-maintained tables. The C baseline is exactly this — a single ~9,600-line file with a hardcoded
`walk_pdb` VER2 walker (`src/qemu/nvkvm_gpu_emul.c:4751`, per the rewrite doc Part 3.1) and
hand-offset struct spelunking, retargetable to neither a new driver nor a new arch without editing
hot paths. The rewrite's value proposition **is** this separation: Axis A becomes a generated data
table selected at runtime; Axis B becomes a trait the pure-logic core is written against, never a
concrete generation.

The rest of this doc: §2 designs the Axis-A codegen; §3 is the **Axis-B architecture delta map** (the
centerpiece research); §4 is the Rust design that keeps the core agnostic; §5 is the honest ceiling;
§6 is the cheap experiment that proves resilience early.

---

# 2. Axis A — the declarative ABI + codegen strategy

## 2.1 The nvproxy reference: how *one* codebase supports many driver versions

gVisor's nvproxy is the closest existing solution to our Axis-A problem — a production system proxying
the NVIDIA ioctl ABI across many driver releases from one codebase. Mine it directly.

**The registry.** Every supported driver version is one entry in a global map
(`gvisor/pkg/sentry/devices/nvproxy/version.go:142`):

```go
var abis map[nvconf.DriverVersion]abiConAndChecksum          // version.go:142
```

keyed by a `DriverVersion{major, minor, patch}` (`nvconf/version.go`), each value pairing a *lazy
constructor* with installer checksums (`version.go:83`):

```go
type abiConAndChecksum struct { cons driverABIFunc; checksums Checksums }   // :83
type driverABIFunc func() *driverABI                                        // :36
```

**The ABI definition is four handler tables + an introspection hook** (`version.go:100`):

```go
type driverABI struct {
    frontendIoctl   map[uint32]frontendIoctlHandler        // by IOC_NR(cmd)
    uvmIoctl        map[uint32]uvmIoctlHandler             // by cmd
    controlCmd      map[uint32]controlCmdHandler           // by NVOS54_PARAMETERS.Cmd
    allocationClass map[nvgpu.ClassID]allocationClassHandler // by NVOS64_PARAMETERS.HClass
    getInfo         driverABIInfoFunc                      // struct/ioctl introspection
}
```

The doc-comment at `:88-99` names the exact four branch points that are versioned — a precise
enumeration of what "the ABI" *is* at the ioctl layer.

**Deltas are inherit-then-mutate, not copy.** A new version's constructor calls the previous version's
constructor to get a fully-populated `*driverABI`, then mutates only the changed keys. The base
version `v535_104_05` (`version.go:164`) is the only one built from scratch (~150 lines of full
tables); every later version is a small delta. Example shape (from the `v550_40_07` region,
~`:844-894`):

```go
v550_40_07 := func() *driverABI {
    abi := v545_23_06()                                   // inherit everything
    abi.frontendIoctl[nvgpu.NV_ESC_WAIT_OPEN_COMPLETE] = feHandler(...)   // add
    abi.controlCmd[nvgpu.NV0000_CTRL_CMD_GPU_ASYNC_ATTACH_ID] = ctrlHandler(...)
    abi.uvmIoctl[nvgpu.UVM_MIGRATE] = uvmHandler(...)
    // … ~17 more single-line mutations …
    prevGetInfo := abi.getInfo
    abi.getInfo = func() *DriverABIInfo { info := prevGetInfo(); /* +metadata */; return info }
    return abi
}
```

Registered via `addDriverABI(major, minor, patch, checksumX86_64, checksumARM64, cons)`
(`version.go:149`), called **17 times** in `Init()` (`grep -c addDriverABI version.go` = 17; base +
16 deltas). **A whole new driver release is 14–51 lines of delta.** That is the number the product
thesis lives or dies on, and nvproxy demonstrates it in production.

**Allowlists ride on the handler, not a separate list.** Each handler struct carries a `capSet`
capability bitmask (`handlers.go`); `feHandler(rmControl, compUtil)` /
`feHandler(rmAllocMemory, compUtil|nvconf.CapGraphics)` (`version.go:184-190`) fold the allowlist
*into* the table entry, and dispatch checks `capSet & capsEnabled` before running. **Un-listed = not
handled = denied** (nvproxy is default-deny; our project defaults to allow with a curated allowlist —
memory `nvproxy_gap_analysis` — a gap the rewrite should close by adopting this per-entry model).

**The split we inherit:** the *tables* (which cmd → which handler at which version, with which caps)
are declarative and delta-coded; the *handler bodies* (`rmControl`, `rmAllocMemory`, `rmDupObject`,
pointer translation) are hand-written logic shared across versions. Versions change the **routing**,
rarely the **logic**. This is exactly the codegen boundary for our Axis A.

## 2.2 The codegen source of truth: NVIDIA already generates it for us

The single most important Axis-A finding: **NVIDIA codegens the ABI themselves, and the outputs are
in the vendored open kernel modules.** We do not have to reverse struct layouts — we consume NVIDIA's
own generator output.

- **`research_clones/ogkm/src/nvidia/generated/g_sdk-structures.h`** — autogenerated (header:
  "WARNING: This is an autogenerated file. DO NOT EDIT."), **5,812 lines / 466 struct definitions**.
  Contains the *versioned* alloc/control param structs, e.g.
  `NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_PARAMS_v1E_05` — the version suffix (`_v1E_05`) is NVIDIA's own
  ABI-versioning marker, machine-readable.
- **`research_clones/ogkm/src/nvidia/generated/g_rpc-structures.h`** — autogenerated, **12,037 lines
  / 213 struct definitions**: the GSP-RPC message payload layouts, versioned (`_v03_00` etc.).
- **`research_clones/ogkm/src/nvidia/generated/g_rpc-message-header.h`** — the RPC envelope
  `rpc_message_header_v03_00 { header_version, signature, length, function, rpc_result, sequence, … }`.
- **`research_clones/ogkm/src/nvidia/inc/kernel/vgpu/rpc_global_enums.h`** — **230 RPC function IDs**
  and **32 event IDs** via an `X(RM, NAME, id)` X-macro (e.g. `X(RM, ALLOC_ROOT, 2)`,
  `X(RM, CTRL_GPFIFO_GET_WORK_SUBMIT_TOKEN, 186)`, `NV_VGPU_MSG_EVENT_GSP_INIT_DONE = 0x1001`).
  Trivially parseable.
- **`research_clones/ogkm/src/common/sdk/nvidia/inc/class/cl*.h`** — class IDs + alloc-param structs,
  each tagged `NVxxxx_ALLOC_PARAMETERS_MESSAGE_ID` (e.g. `#define NV01_DEVICE_0 (0x80U)`).
- **`research_clones/ogkm/src/common/sdk/nvidia/inc/ctrl/ctrl*.h`** — control command IDs
  (`NVxxxx_CTRL_CMD_*`, on the order of ~2,000 defines) + their param structs.
- **`research_clones/ogkm/src/common/sdk/nvidia/inc/nvos.h`** — the NVOS ioctl param structs
  (`NVOS21/54/64/…`) + status codes.

These files are FINN-schema output (NVIDIA's interface-definition tool). The `.def`/`.finn` *source*
templates are **stripped** from the open-source drop (only the generated `.h` survive), so we cannot
re-run NVIDIA's generator — but the generated `.h` are stable, regular, and are the exact contract the
driver compiles against. **This directly retires the L11 bug class:** every incident behind L11
(`cuCtxCreate`-401 missing alloc-size, three Vulkan enum gaps, nvos64 field order,
`abi_struct_truncation`) was a *hand-transcribed* struct/size that a generator reading these headers
would never get wrong.

## 2.3 Our codegen plan

A build-time dev tool **`nvkvm-abi-gen`** (per rewrite doc §4.2) parses the ogkm headers **per
driver-release tag** and emits `#[repr(C)]` Rust:

1. **Structs** — every NVOS/class/ctrl/RPC param struct → `#[repr(C)]` Rust struct with a
   `const_assert!(size_of::<T>() == N)` static size check (the `abi_parity` discipline made
   compile-time — memory `abi_struct_truncation`). Field offsets/alignment come from the header
   (`NV_ALIGN_BYTES(8)` markers preserved).
2. **Constants** — class IDs, control-command IDs, ioctl NRs, RPC function/event IDs → typed Rust
   consts/enums (parse `cl*.h`, `ctrl*.h`, `rpc_global_enums.h`).
3. **Alloc-param size table** — class → param size (the L11 root cause) as a generated map, so
   "unknown class" is a loud miss, never a silent truncation.
4. **RPC message layout table** — function ID → message struct + version suffix, from
   `g_rpc-structures.h` + `rpc_global_enums.h`.

Emitted **per `(driver_version)`** as versioned modules behind a `DriverVersion` enum, plus a
**version-dispatch layer** (`trait DriverAbi`, §4.1) the core selects at runtime from the detected
guest driver version — reusing the project's existing `abi_profile` auto-detect that already spans 3
majors (memory `multi_driver_validated`; the emulated device advertises a version, so *we choose it*
— a simplification vs nvproxy, which must match a host driver it does not control).

**Two hard rules** (carried from rewrite doc §4.2): (1) generated code is **committed and
diff-reviewed** — the generator is a dev tool, not a build dependency, so a bad header parse is caught
in review, not at a customer's boot; (2) the generated RM table **is the coverage report** —
enumerated-vs-exercised, making `rewrite_horizon_target`'s "all RM commands covered" measurable.

**Mechanical vs hand-curated (the honest boundary):**

- **Mechanical (~85–90 % of Axis A):** struct layouts + sizes, class/ctrl/ioctl/RPC IDs, alloc-param
  sizes, RPC message shapes. Pure header transcription.
- **Hand-curated (~10–15 %):** (a) the **allowlist / cap-set per command** — nvproxy hand-writes
  which controls are forwardable and their capability class; this is a *security policy* decision, not
  derivable from a header, and maps onto our Case-1-forward vs Case-2-ack-only split (rewrite L2);
  (b) the **handler bodies with pointer translation** (embedded-pointer chasing, fd remap) — shared
  across versions, written once; (c) **behavioral controls** that must not be blindly forwarded
  (GSP-internal — rewrite L2). The generator produces the *shapes and the routing table skeleton*;
  the human fills the *policy* — exactly nvproxy's tables-vs-handlers split.

## 2.4 The single-snapshot caveat (a real finding)

The vendored ogkm is **one tag (610.43.02)**, not a per-release history. Codegen "per driver-release
tag" therefore requires **checking out each ogkm git tag** from NVIDIA's public
`open-gpu-kernel-modules` repo (they *are* tagged per release, e.g. `575.x`, `580.x`) and running the
generator against each — a mechanical CI job, but it must be built. **ASSUMPTION — verify:** that
every driver release the *closed* driver ships also has a matching *open* kmod tag with identical ABI
(memory `multi_driver_support` treats open as canonical *and stricter*; the residual risk is a
closed-only release with no open tag, or a closed/open ABI skew — must be spot-checked with
`abi_parity` against a live closed driver, as the project already does).

---

# 3. ★ Axis B — the architecture delta map

This is the core research the owner asked for: **what actually changes, generation to generation,
across the ABI-relevant behavioral dimensions** — grounded in the ogkm HAL sources. It is the map that
tells the rewrite *how many* `Arch` impls it needs and *what each must override*.

## 3.0 The MMU format version is the spine

The single most load-bearing Axis-B fact is the GMMU format version, defined at
`research_clones/ogkm/src/nvidia/inc/libraries/mmu/gmmu_fmt.h`:

```
#define GMMU_FMT_VERSION_1   1      // :197   Fermi/Kepler/Maxwell
#define GMMU_FMT_VERSION_2   2      // :202   Pascal … Ada (5–6 levels, 49-bit VA)
#define GMMU_FMT_VERSION_3   3      // :207   Hopper+ (7 levels, 57-bit VA, PCF, unified addr)
```

Every arch's format init is a small HAL override that (a) picks a version and (b) tweaks page-size /
level flags. The pattern is *inherit-then-flag*, structurally identical to nvproxy's Axis-A deltas —
each generation's `kgmmuFmtInitLevels_*` calls the *previous* generation's and sets one flag. This is
the key that makes Axis B tractable: **the deltas are small and localized**, so an `Arch` trait need
only override the handful of dimensions that actually change.

## 3.1 The delta table (Turing → Ampere → Ada → Hopper → Blackwell)

Five architectures mapped across ten ABI-relevant dimensions. **Codegen-able?** = is this dimension a
*data shape* an Axis-A generator can emit (◆), or a *behavior* needing an `Arch` trait method (▲)?
Sources cited per row; `research_clones/ogkm/...mmu/arch/<gen>/kern_gmmu_fmt_<gen>.c` abbreviated
`fmt_<gen>.c`.

| # | Dimension | **Turing** (TU10x) | **Ampere** (GA10x) | **Ada** (AD10x) | **Hopper** (GH10x) | **Blackwell** (GB10x) | Codegen? |
|---|---|---|---|---|---|---|:---:|
| **B1** | **MMU/PTE format version** | **VER2** (`fmt_tu10x.c:33`) | **VER2** (`fmt_ga10x.c`) | **VER2** *(reuses GA10x — no arch MMU dir; ASSUMPTION—verify)* | **VER3** (`fmt_gh10x.c:59`) | **VER3** (`fmt_gb10x.c`) | ◆ format tables / ▲ walker |
| **B2** | **Page-table levels / VA bits** | 5-level, 49-bit (inherits GP10X) | 5-level, 49-bit | 5-level, 49-bit | **7-level, 57-bit** (+PD4 root `:63`) | 7-level, 57-bit | ▲ walker |
| **B3** | **Page sizes (leaf)** | 4K / 64K / 2M | + **512M** (`fmt_ga10x.c:55` `pLevels[2].bPageTable=NV_TRUE`) | 4K/64K/2M/512M (via GA10x) | 4K/64K/2M/512M | + **256G** (`fmt_gb10x.c:60` `pLevels[2].bPageTable=NV_TRUE`) | ▲ walker (#13 was exactly a missing size) |
| **B4** | **PDE/PTE encoding** | dual-PDE, separate vidmem/sysmem addr fields; discontiguous peer/comptag field (`fmt_tu10x.c:37-49`) | same as TU10x (VER2) | same (VER2) | **unified `fldAddr`**, **PCF fields** (`fldPdePcf`/`fldPtePcf` replace VOLATILE/RO/PRIV/ATOMIC bits), **no comptagline** (`fmt_gh10x.c:131-180`) | same as Hopper (VER3) | ◆ field descriptors / ▲ encode-decode |
| **B5** | **USERD / instance-block / RAMFC layout** | Volta+ USERD (separate from RAMFC); RAMFC/RAMIN per Turing | Ampere RAMFC | Ada (≈Ampere; ASSUMPTION—verify) | Hopper RAMFC (new fields for CC/MIG) — ASSUMPTION—verify | Blackwell RAMFC — ASSUMPTION—verify | ◆ offsets / ▲ liveness rules (#11) |
| **B6** | **Doorbell / work-submit token / CHID** | usermode doorbell; token encodes channel | token encodes target channel (E0: `token[11:0]`=vChid, proven — rewrite §1.1) | ≈Ampere (ASSUMPTION—verify) | ≈Ampere/Hopper (ASSUMPTION—verify) | ASSUMPTION—verify | ▲ token decode (rewrite `by_vchid`) |
| **B7** | **GSP presence + RPC semantics** | **pre-GSP** (Turing has no GSP-RM offload by default) | **GSP-RM** (faked-GSP boot proven — memory `mode2_keystone_gsp_init_done`) | GSP-RM | GSP-RM (+ CC-gated RPC) | GSP-RM | ▲ boot FSM + RPC (behavioral RE) |
| **B8** | **Engine / runlist / TSG model** | TSG + runlist; GR/CE engines | TSG + runlist (per-`(client,tsg)` sched — #12) | ≈Ampere | + MIG partitioning maturity; per-engine runlists | Blackwell (2nd-gen MIG; ASSUMPTION—verify) | ▲ scheduling |
| **B9** | **Compute / DMA-copy / GR class IDs** | Turing class set (e.g. `TURING_*`, `TURING_DMA_COPY_A`) | Ampere classes (`AMPERE_*`, `AMPERE_DMA_COPY_B` — the #? DMA-copy sanitizer entries) | Ada classes (`ADA_*`) | Hopper classes (`HOPPER_*`) | Blackwell classes (`BLACKWELL_*`) | ◆ **pure Axis-A** (from `cl*.h`) |
| **B10** | **Confidential Compute / attestation** | none | none (GA100 has CC infra; GeForce off) | none (GeForce) | **CC + SPDM stack present** (`.../gpu/conf_compute/`, `.../gpu/spdm/`) — optional, HW-gated | **CC + SPDM** (GB100 class) | ▲ **CEILING — see §5** |

**Reading the table.** The dominant structural break is **Ampere/Ada → Hopper: VER2 → VER3** (B1/B2/
B4). Turing→Ampere and Ampere→Ada are *near-free* (Ampere = GA10x adds the 512M leaf; Ada reuses
Ampere's format entirely). Blackwell = Hopper's VER3 + a 256G leaf. So across five generations there
are effectively **two MMU regimes** (VER2 and VER3) plus per-generation leaf-size and class-ID deltas —
which means an `Arch` trait set needs **two heavyweight walker impls, not five**, with thin per-gen
subclasses for the leaf-size and class-ID deltas.

## 3.2 Per-dimension notes (grounded)

**B1–B4 — MMU (the well-grounded part).** The `kern_gmmu_fmt_*` sources confirm the delta pattern
directly:

- Ampere: `kgmmuFmtInitLevels_GA10X` calls `kgmmuFmtInitLevels_GP10X(...)` then
  `pLevels[2].bPageTable = NV_TRUE` (`fmt_ga10x.c:52,55`) — "PD1 can now hold a PTE pointing to a
  512MB Page" (`:54`). **This one flag is the #13 bug**: the C walker lacked PD1-512M-leaf support and
  silently dropped PT writes (memory `mode2_13_multiiter_idle_hang`). It is *one line* in the HAL and
  *weeks* of debugging when missed — the strongest possible argument for grounding the walker in these
  sources rather than guessing.
- Blackwell: `kgmmuFmtInitLevels_GB10X` calls `kgmmuFmtInitLevels_GH10X(...)` then
  `pLevels[2].bPageTable = NV_TRUE` for a **256GB** leaf (`fmt_gb10x.c:57,60`) — the *identical*
  pattern one regime up.
- Hopper VER3 is the real break: 7 levels with a new PD4 root (`fmt_gh10x.c:63`), 57-bit VA, a
  **unified address field** `fldAddr` (no separate vidmem/sysmem — `:131-180`), and **PCF** (Page
  Class Format) fields `fldPdePcf`/`fldPtePcf` replacing the individual VOLATILE/READ_ONLY/PRIVILEGE/
  ATOMIC_DISABLE permission bits, and comptagline is *gone* from the PTE. A VER2 decoder cannot read a
  VER3 PTE — this is why Axis B needs a *behavioral* trait, not a struct swap.

The field *descriptors* themselves (bit positions of aperture/valid/addr) live in `NV_MMU_VER2_*` /
`NV_MMU_VER3_*` defines and **are codegen-able as data** (◆) — but the **walker algorithm** that
strides levels, picks the right leaf size, and encodes/decodes a PTE is **behavioral** (▲). So B1/B4
split: format-as-data is Axis-A-like; walk-as-algorithm is Axis-B.

**B5 — USERD/RAMFC.** Grounded for the layout *offsets* (codegen from the `dev_ram*.h` / class
headers), but the **liveness rules** are behavioral and RE-derived: the #11 USERD-wipe bug (an
emulated CE zero-fill clobbered a *live* host USERD page — rewrite L5, fixed by
`nvkvm_fb_is_live_userd`) is a per-arch *semantic*, not a layout. Marked ASSUMPTION—verify for
Hopper/Blackwell because we have not booted those; CC/MIG add RAMFC fields there.

**B6 — doorbell/token.** Ampere is proven (E0: `token[11:0]` = vChid, one per channel, zero
collisions — rewrite §1.1). Turing predates the usermode-doorbell submit model in its earliest form;
Ada/Hopper/Blackwell are assumed to carry the Ampere encoding forward but are **unverified** — the
`Arch::decode_doorbell` trait method localizes this exactly.

**B7 — GSP.** The cleanest generational break after the MMU: **Turing has no GSP-RM offload** in the
stock configuration (RM runs on the CPU), so the entire faked-GSP boot FSM (rewrite `nvkvm-gsp`) is
**inapplicable to Turing** — Turing would need a different bring-up path (direct RM emulation) or is
simply out of scope. Ampere onward is GSP-RM, which is what the project targets and what
`mode2_keystone_gsp_init_done` proved. **This is behavioral to the core** and the least codegen-able
dimension (see §5).

**B8 — runlist/TSG.** Per-`(client,tsg)` scheduling is the #12 fix (rewrite L7); the *model* (TSG +
runlist + doorbell) is stable Ampere→Blackwell, but MIG partitioning matures across Hopper/Blackwell
and changes engine/runlist multiplicity — ASSUMPTION—verify for those.

**B9 — class IDs.** The *one Axis-B dimension that is pure Axis A*: compute/DMA-copy/GR class IDs are
just `#define`s in `cl*.h`, fully codegen-able. The *behavior* keyed off them (which sanitizer
entries, which runlist — the `AMPERE_DMA_COPY` engineType fix, memory `dma_copy_class_alloc_params`)
is the Axis-B part; the IDs themselves regenerate for free.

**B10 — CC/attestation.** See §5 — the ceiling.

## 3.3 Coverage honesty

**Well-grounded (vendored source directly confirms):** B1–B4 for Turing/Ampere/Hopper/Blackwell (the
`kern_gmmu_fmt_*` files exist and were read); B9 (class headers exist); B10 presence (CC/SPDM dirs
exist). **Ada is inferred** (B1–B6): there is **no `mmu/arch/ada` directory** and no `ad10x` MMU
override — the Ada GPU file `.../gpu/arch/ada/kern_gpu_ad102.c` exists but does not override
`kgmmuFmtInitLevels`, so Ada **reuses GA10x VER2** (strong evidence, but **ASSUMPTION — verify** on
real Ada silicon). **Hopper/Blackwell B5–B8** are marked ASSUMPTION—verify: the format files exist,
but USERD liveness, doorbell encoding, and runlist multiplicity there are not project-tested (we have
only booted Ampere — GA106/GA10x — end-to-end).

---

# 4. The Rust design: how the core stays agnostic

## 4.1 `nvkvm-abi` = codegen'd declarative ABI + a `DriverAbi` dispatch (Axis A)

Per rewrite doc §4.2, the `nvkvm-abi` crate holds the **generated, committed** per-version modules and
exposes a runtime dispatch trait:

```rust
/// Axis A. One impl per generated driver version; selected at runtime from the
/// detected/advertised guest driver version (reuses abi_profile auto-detect).
pub trait DriverAbi {
    fn version(&self) -> DriverVersion;
    /// nvproxy-style versioned handler tables (the four branch points, version.go:88-99):
    fn frontend_ioctl(&self, nr: u32) -> Option<&IoctlHandler>;
    fn uvm_ioctl(&self, cmd: u32) -> Option<&UvmHandler>;
    fn control_cmd(&self, cmd: u32) -> Option<&ControlHandler>;   // + allowlist/cap in handler
    fn alloc_class(&self, cls: ClassId) -> Option<&AllocHandler>; // + generated param size
}
```

Each generated version module is a full table; a **new version** is a regenerate that emits a new
module — but we adopt nvproxy's **inherit-then-mutate** so the *reviewable diff* is only the delta
(the generator can emit `impl DriverAbi for V580 { ... }` as `V575`'s tables plus the changed
entries, mirroring `version.go:844`). Handlers carry a `CapSet` allowlist field exactly like nvproxy's
`capSet` — closing the project's default-allow gap (`nvproxy_gap_analysis`).

**Adding a new driver version** = run `nvkvm-abi-gen` against the new ogkm tag → new module + a
hand-curated allowlist/behavioral-control delta → review the diff. **Day-scale.**

## 4.2 The `Arch` trait set = Axis-B behavior (one impl per generation)

The pure-logic core (`nvkvm-mmu` address table, `nvkvm-fwd` forwarder, `nvkvm-completion`,
`nvkvm-isolate`) programs against an `Arch` abstraction and **never** against a concrete generation:

```rust
/// Axis B. One impl per GPU generation; the ~two MMU regimes (VER2/VER3) share a
/// walker with per-gen leaf-size/class deltas, so this is ~2 heavy impls + thin subclasses.
pub trait Arch {
    // B1–B4: MMU. The walker strides levels + decodes PTEs for THIS format version.
    fn mmu(&self) -> &dyn GmmuFmt;            // GMMU_FMT_VERSION_{2,3}; page sizes; PDE/PTE codec
    // B5: USERD/RAMFC accessors + liveness (the #11 rule as a typed method).
    fn userd(&self) -> &dyn UserdModel;
    // B6: doorbell/work-submit token → (channel, submit) decode (E0's vChid extraction).
    fn decode_doorbell(&self, token: u64) -> DoorbellTarget;
    // B7: GSP presence + RPC dialect (None for pre-GSP Turing).
    fn gsp(&self) -> Option<&dyn GspDialect>;
    // B8: engine/runlist/TSG scheduling model.
    fn scheduling(&self) -> &dyn RunlistModel;
    // B9: class-ID set (compute/CE/GR) — sourced from nvkvm-abi (codegen), surfaced here.
    fn classes(&self) -> &ClassMap;
}

pub trait GmmuFmt {                            // the Axis-B core: #13 lives or dies here
    fn versions(&self) -> GmmuVersion;         // VER2 | VER3
    fn page_sizes(&self) -> &[PageSize];       // 4K,64K,2M,512M[,256G] — MUST include every real leaf
    fn walk(&self, root: Pdb, va: u64, read: &dyn FbRead) -> WalkResult; // MISS=FAULT (L1)
    fn decode_pte(&self, raw: u128) -> Pte;    // VER2 dual-field vs VER3 unified+PCF
}
```

Concrete impls: `Turing` (VER2, no-GSP — likely out-of-scope stub), `Ampere` (VER2 + 512M, GSP —
the tested target), `Ada` (VER2, reuses Ampere `GmmuFmt`), `Hopper` (VER3 + CC-aware), `Blackwell`
(VER3 + 256G). Because Ada reuses Ampere's format and Blackwell reuses Hopper's, the **heavy** work is
the two `GmmuFmt` regimes; the per-gen `Arch` impls are thin.

**Adding a new architecture** = one new `Arch` impl (and, if a new MMU regime, one new `GmmuFmt`).
Bounded, not open-ended — and the core does not change, because it only ever calls `arch.mmu().walk()`,
`arch.decode_doorbell()`, etc.

## 4.3 The composition: core against traits, never versions

`nvkvm-core`'s `Gpu` holds `driver: Box<dyn DriverAbi>` (Axis A) and `arch: Box<dyn Arch>` (Axis B),
both selected once at device realize from the advertised generation + driver version. The address
table, forwarder, completion engine, and isolate manager (rewrite §4.2 crates) are written purely
against these traits + the `Proc` model. **No `if version == 580`, no `if arch == Ampere`, anywhere in
the logic crates** — the two-axis split is enforced by the type system: a logic crate that names a
concrete version/arch fails review.

This also means the two axes compose cleanly at the cross-product without cross-product *code*: driver
580 on Ampere is `(V580 DriverAbi, Ampere Arch)`; driver 580 on Hopper is `(V580 DriverAbi, Hopper
Arch)` — same core, two independent selections. That is the whole point.

---

# 5. What is NOT abstractable + honest risk

The ABI-agnostic layer **shrinks** the treadmill; it does not eliminate it. Quantified:

**What it buys (the win).** ~85–90 % of Axis A becomes free (regenerate from headers NVIDIA already
generates); a new driver version drops from "stage libs + hunt struct truncations for days"
(the 575/580 experience, memory `guest_lib_version_staging`, `abi_struct_truncation`) to a
regenerate + allowlist diff. A new architecture in an *existing* MMU regime (Ada on VER2, Blackwell
on VER3) is a thin `Arch` impl. The #13-class silent-drop bug is designed out: `GmmuFmt::page_sizes`
*must* enumerate every leaf, and MISS=FAULT (L1) makes a missing one loud.

**What it does NOT buy (the residuals):**

1. **GSP-RPC *semantics* are behavioral, not shapes (B7).** Codegen gives the message *layouts*
   (`g_rpc-structures.h`, 213 structs) and *function IDs* (`rpc_global_enums.h`, 230 IDs) — the
   **shapes**. It does *not* give the **protocol**: which RPC the guest RM sends when, what an
   ack must contain to unblock a poll, the seqNum-ring discipline, the WPR2/FWSEC/booter mailbox
   latch semantics (rewrite L13's quirk ledger), the Case-1-forward-vs-Case-2-ack-only decisions
   (L2). Every one of those was *reverse-engineered by watching a live driver*, not read from a
   header. **A GSP-RPC redesign between major versions still costs behavioral RE** — codegen can't
   read intent out of a struct. This is the irreducible core of the project's value *and* its
   irreducible maintenance cost. Mitigation is L13: every quirk is a spec paragraph + a differential
   test, so re-derivation after a GSP change is *bounded and checked*, not open-ended.

2. **Confidential Compute is a hard ceiling, not a treadmill item (B10 — the biggest un-abstractable
   risk).** The vendored tree confirms the machinery: `research_clones/ogkm/src/nvidia/src/kernel/
   gpu/conf_compute/` (`conf_compute.c`, `ccsl.h`) + a full SPDM stack under `.../gpu/spdm/`, gated by
   `gpuIsCCEnabledInHw_HAL()` / `gpuIsProtectedPcieEnabledInHw_HAL()` and the `bOsCCEnabled` registry
   key. **The whole Mode-2 approach depends on the GPU accepting a faked/emulated GSP.** When CC is
   *enabled in hardware*, the GPU performs **on-silicon SPDM attestation** of the GSP firmware — an
   unauthenticated/faked GSP is cryptographically rejected; faking the attestation is infeasible
   (it is the entire point of the silicon root-of-trust — consistent with `mode2_attestation_spike_GO`,
   which found *no* silicon-secret checks at bring-up **precisely because CC was off**). So:
   - **Near-term target is SAFE:** bare-metal / GeForce ships CC **off** (`bOsCCEnabled` default; CC
     is a datacenter Hopper/Blackwell + Protected-PCIe feature). Ampere/Ada GeForce — the project's
     actual target — has no CC gate. **ASSUMPTION — verify** that no future *consumer* driver flips CC
     on by default.
   - **Datacenter Hopper/Blackwell with CC ON is likely UNREACHABLE** by faked-GSP Mode-2. Honest
     verdict: **Blackwell-with-CC is a strategic ceiling.** Reaching it would require either the CC
     mode be disabled (host firmware / not our call), or a fundamentally different approach (real GSP +
     CC — which defeats the unprivileged-forwarding premise). This should be **stated in the product
     positioning**, not treated as a fixable bug: the product's addressable market is
     **non-CC GPUs** (all GeForce; datacenter parts with CC disabled).
   - **★ But CC-off IS reachable on datacenter parts you control (decision #11 — good news for the
     market).** CC is an **opt-in, per-GPU/VM mode that ships OFF by default even on H100/Blackwell**
     (`bOsCCEnabled` default; gated behind `gpuIsCCEnabledInHw_HAL` / Protected-PCIe). So an operator
     who **controls the host GPU configuration** can run **CC-OFF on datacenter silicon** and Mode-2
     works there exactly as on GeForce. The wall is **only** "guest demands CC **and** host enforces
     CC" — which you hit only if the product's value-prop *is* confidential compute (it is not). Net:
     the addressable market is **non-CC GPUs = all GeForce + any datacenter part in an
     operator-controlled CC-off deployment**, not merely consumer parts. **ASSUMPTION — verify** no
     future consumer *or* datacenter default flips CC on.

3. **The behavioral RE residual is per-*generation*, not per-*version*.** Turing↔Ampere GSP dialect,
   Ampere→Hopper VER3 walker + PCF encode/decode, MIG runlist multiplicity — these land as new `Arch`
   trait work, and the *first* boot of a new generation is where the undocumented behavior surfaces
   (as it did for Ampere over ~14 months). The trait design makes it **bounded** (one impl, one file)
   and the differential harness (§6) makes it **checked** — but the first-contact RE cost is real and
   should be budgeted per new architecture, not assumed to be codegen-free.

**Net:** the layer converts an *unbounded per-driver-version* cost into a *bounded per-generation*
cost, and caps the addressable market at non-CC GPUs. That is a survivable treadmill; the un-mitigated
baseline (hand-maintained tables + a hardcoded walker) is not.

---

# 6. The validation milestone — proving version-resilience early

The product thesis is only real if version-resilience is *measured*, cheaply, before the rewrite is
deep. The concrete experiment (per rewrite doc §4.5's oracle discipline):

**Experiment V1 — "the day-not-a-month" drill (Axis A).** Bring the `nvkvm-abi` codegen up on the
current bench driver (**580.x** — memory `multi_driver_validated`, the HOST is on 580.159.04), boot
single-process Mode-2 green. Then **re-target driver 575** (or the vendored 610): run `nvkvm-abi-gen`
against that ogkm tag, regenerate, curate the allowlist delta, rebuild. **Measure the hand-work** —
lines of hand-written delta, and wall-clock to green. **"Resilient" = a day; "dead" = a month.**
Target: **< 1 day, < ~50 lines hand-delta** (nvproxy's 14–51-line deltas are the existence proof this
is achievable). This experiment needs **no new GPU** — it is the same Ampere silicon, different driver
ABI — so it is cheap and runs early on the serialized bench.

**Experiment V2 — the differential oracle (both axes).** Per rewrite doc §4.5: the C emulator is the
**single-process differential oracle**. Concretely:
- **Axis A:** diff the *generated* Rust ABI tables against the C's hand-coded alloc-size/RPC tables
  (rewrite §4.5 step 1) — every discrepancy is a C bug or a generator bug, both found before any port.
  This is the *first* thing to build; it validates the generator with zero GPU time.
- **Axis B:** property-test each `GmmuFmt::walk` against the ogkm `kern_gmmu_fmt_*` format definitions
  (the same source that defines the driver's own walker), and **differential-walk the same FB images**
  the C walker walks (replay #13's banked traces — rewrite §4.5 step 3). A VER2 impl that reproduces
  the C's byte-exact walk on the #13 corpus, *and* a VER3 impl that walks a synthetic Hopper PT
  correctly, proves the trait abstraction holds across the regime boundary.

**Experiment V3 — the second-architecture drill (Axis B, later, aspirational).** When Ada or Blackwell
silicon is available: implement the `Arch` impl (thin for Ada = reuse Ampere `GmmuFmt`; heavier for
Blackwell = VER3 + 256G), and **measure**: does the core change at all? (It must not.) How many lines
is the new `Arch` impl? "Resilient" for a *new architecture in an existing MMU regime* = a small impl,
no core edits; for a *new regime* = one new `GmmuFmt` + no core edits. If the core needs edits, the
abstraction leaked — that is the failure signal to catch early.

**Definition of "version-resilient" (the acceptance bar):**
- New **driver version**: regenerate + ≤ ~50-line hand-delta, single-process green in **< 1 day**,
  differential-clean vs the previous version's C oracle.
- New **architecture (existing MMU regime)**: one thin `Arch` impl, **zero** logic-crate edits.
- New **architecture (new MMU regime)**: one new `GmmuFmt` + one `Arch` impl, **zero** logic-crate
  edits; walker property-tested against the ogkm format source.
- Everywhere: the pure-logic core is provably free of concrete version/arch names (a lint/grep gate
  in CI — "no `V580`, no `Ampere` in `nvkvm-core`/`-mmu`/`-fwd`/`-completion`").

---

# 7. Summary

- **Two axes, handled differently.** Axis A (declarative, driver-scoped) → **codegen** from the
  ogkm `generated/` headers NVIDIA already produces, dispatched behind `DriverAbi` in nvproxy's
  inherit-then-mutate style. Axis B (behavioral, generation-scoped) → **abstraction** behind an
  `Arch`/`GmmuFmt` trait set, one impl per generation, ~two heavy MMU regimes across five gens.
- **The win:** a new driver version becomes a regenerate + allowlist diff (day-scale), retiring the
  entire L11 hand-transcription bug class; a new architecture becomes a bounded trait impl with **zero
  core edits**.
- **The ceiling:** Confidential Compute (Hopper+/Blackwell, HW-gated) rejects a faked GSP via
  on-silicon attestation — a **hard, un-abstractable limit**. Near-term GeForce/non-CC target is safe;
  datacenter-CC parts are out of reach and must be positioned as such, not chased.
- **The residual:** GSP-RPC *semantics* (not shapes) remain behavioral RE per generation — codegen
  gives structs, never protocol. Mitigated (not removed) by L13's quirk-as-spec+test discipline.
- **Proof:** the cheap V1 "day-not-a-month" re-target drill (same silicon, 580→575) + the C
  differential oracle validate resilience before the rewrite is deep.

---

*Appendix — vendored sources cited:* `gvisor/pkg/sentry/devices/nvproxy/version.go` (Axis-A registry:
`:36,:83,:100,:142,:149,:162,:844`), `nvconf/version.go`, `handlers.go`;
`research_clones/ogkm/version.mk:1` (610.43.02);
`research_clones/ogkm/src/nvidia/inc/libraries/mmu/gmmu_fmt.h:197,:202,:207` (format versions);
`.../src/kernel/gpu/mmu/arch/{turing,ampere,hopper,blackwell}/kern_gmmu_fmt_*.c` (per-gen deltas —
Ampere `fmt_ga10x.c:52,55` 512M; Blackwell `fmt_gb10x.c:57,60` 256G; Hopper `fmt_gh10x.c:59,131-180`
VER3); `.../generated/g_sdk-structures.h` (466 structs), `g_rpc-structures.h` (213),
`g_rpc-message-header.h`, `.../inc/kernel/vgpu/rpc_global_enums.h` (230 fn IDs);
`.../src/common/sdk/nvidia/inc/{class,ctrl}/`, `nvos.h`; `.../src/kernel/gpu/conf_compute/`,
`.../src/kernel/gpu/spdm/` (CC ceiling). Cross-ref: `mode2_rust_rewrite_architecture.md` (L1/L2/L11/
L13, §4.2), `mode2_address_table.md`, memory `multi_driver_validated`/`multi_driver_support`/
`mode2_13_multiiter_idle_hang`/`mode2_attestation_spike_GO`.
