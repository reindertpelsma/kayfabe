> ⊘⊘⊘ **ARCHIVED — THIS DESCRIBES A SUPERSEDED ARCHITECTURE. IT IS REFERENCE, NOT CURRENT.**
> The live design is `docs/design/THE_DESIGN.md`. This file is kept because its *measurements*,
> its *ogkm findings* and its *reasoning* remain useful — its **architecture does not**. Do not
> implement from it, and do not cite it as current. Archived 2026-09-21 (w822).

# What hardware actually references — GA10x, cited

**STATUS: LIVE, 2026-09-19.** Derived from `ogkm-580.159.04` with `file:line` for every claim.

> **Owner, 2026-09-19:** *"For this, the specs have to be crystal clear what is/not referenced
> on hardware."*

⊘ **Scope.** `ogkm/src/nvidia` is **CPU-RM only**. On a GSP-offloaded GA102/GA106 the code that
writes a channel's instance block and builds runlist entries lives in **GSP firmware, not in
this tree**. Proof: the PDB-commit callback is a no-op for a GSP client —
`gmmu_walk.c:663-667` *"Noop inside a guest or CPU RM."* — and `kgmmuInstBlkInit` has **zero
channel callers** here (all callers are BAR1/BAR2/FLA/HWPM).

---

## The reference graph

```
                        ┌───────────────────────────────────────────┐
  SOFTWARE ONLY         │  hClient / hVASpace / hChannel / hTSG     │
  (the GPU never        │  hContextShare                            │
   sees these)          └───────────────┬───────────────────────────┘
                                        │ RM + GSP resource server
                                        │ resolve handles → objects
                                        ▼
                        ┌───────────────────────────────────────────┐
                        │  OBJVASPACE  (RM's object)                │
                        │    owns → the page directory tree         │
                        └───────────────┬───────────────────────────┘
                                        │ memdescGetPhysAddr(pPDB)
  ══════════════════════════════════════╪═══════════════════════════════  the line
  HARDWARE CONSUMES                     ▼          everything below is read by the GPU
                        ┌───────────────────────────────────────────┐
                        │  INSTANCE BLOCK  (RAMIN, 4 KiB, 4 KiB-al) │
                        │                                           │
   runlist CHAN entry ─▶│  dw0..127   RAMFC  (PBDMA save area)      │──▶ ring base lives
   carries its PA       │             ⊘ layout NOT in open source   │    in here (unpublished)
                        │                                           │
                        │  dw128/129  PAGE_DIR_BASE_LO/HI  ─────────┼──▶ ROOT of the tree
                        │             + _TARGET (aperture)          │    (legacy / veid=BASE)
                        │             + _VOL, USE_NEW_PT_FORMAT,    │
                        │               BIG_PAGE_SIZE               │
                        │  dw130/131  ADR_LIMIT_LO/HI  (⊘ skipped   │
                        │             on Ampere: HAL is a stub)     │
                        │  dw135      ENABLE_ATS / PASID            │
                        │                                           │
                        │  dw168+4i   SC_PAGE_DIR_BASE(i)  i=0..63  │──▶ 64 ROOTS, one per
                        │             same field set, per subcontext│    VEID. Work carries
                        └───────────────────────────────────────────┘    the VEID that picks
                                        │                                one.
                                        ▼
                        ┌───────────────────────────────────────────┐
                        │  PAGE DIRECTORY TREE — one per VA space   │
                        │  PD3(47,4) → PD2(38,512) → PD1(29,512)    │
                        │            → PD0(21,256)  ← DUAL entry    │
                        │                 ├─ ADDRESS_BIG   → PT_BIG (16,32)   64 KiB leaves
                        │                 └─ ADDRESS_SMALL → PT_SMALL(12,512)  4 KiB leaves
                        │  PDE = {address, aperture, vol}  no VA, no tag       │
                        │  PTE = {VALID, APERTURE, VOL, PRIV, READ_ONLY,       │
                        │         ATOMIC_DISABLE, ADDRESS, KIND, COMPTAGLINE}  │
                        └───────────────────────────────────────────┘

  SEPARATE PLANES, no VA translation:

    USERD  ── physical + aperture ──▶ GP_GET @0x88, GP_PUT @0x8c
               runlist CHAN_USERD_PTR_LO/HI_HW · NV_PPBDMA_USERD_ADDR

    RUNLIST ─ TSG entry + CHAN entries, 16 B each, 1 KiB-aligned on GA102/106
               ⊘ entry BIT LAYOUT not in the open tree

    DOORBELL ─ token = {runlist_id, chid}          kernel_fifo_ga100.c:224-227
               ⊘ carries NO address-space identity
```

---

## The table — is it hardware's, or ours?

| thing | hardware reads it? | where |
|---|---|---|
| **PDB tuple** `{phys addr, aperture, vol, PT-format-v2, big-page-size}` | **YES** | instance block dw128/129, or `SC_PAGE_DIR_BASE(veid)` |
| **VEID** (subcontext id) | **YES** — selects which of 64 PDB slots | `kernel_channel_group.c:554-556`; faults report it |
| **Instance-block physical address** | **YES** — it *is* the channel's hw identity | MMU faults report it: `gv100/dev_fb.h:102-104` |
| **USERD physical address + aperture** | **YES**, no translation | `kernel_channel_gv100.c:204-216` |
| GPFIFO ring base | **YES**, as a **GPU VA** ⇒ needs the PDB | `ctrl2080fifo.h:809` *"Gpfifo Virtual Offset"* |
| Pushbuffer (GP entry target) | **YES**, as a **GPU VA** ⇒ needs the PDB | `clc56f.h:265-284`, 40-bit on Ampere |
| CHID, runlist id | **YES** | doorbell token, CHRAM |
| `hVASpace`, `hChannel`, `hClient`, `hTSG` | **NO** | RM/GSP software only |
| TSG id | runlist/preempt registers only; **not** in RAMIN | `NV_RUNLIST_PREEMPT_TYPE_TSG` |

---

## ★★★★★ Three consequences for this port

**(a) A VA space's identity is the PDB TUPLE, not `hVASpace` and not the bare address.**
The address says *where* the root is; the rest says *which memory* and *how to decode it*.
`0x201000` in vidmem and in sysmem are different trees; 64 KiB vs 128 KiB big pages decode the
same bytes differently (`L_PT_BIG` shift 16 vs 17). ⊘ Keying on the handle keys on a name the
GPU never sees; keying on the bare address silently merges distinct spaces.

**(b) Two channels sharing one PDB are INDISTINGUISHABLE to the MMU, and RM produces that on
purpose.** Legacy SYNC+ASYNC ctxshares put one `pVAS` in two VEID slots;
`NV0080_..._SET_PAGE_DIRECTORY` has an `_ALL_CHANNELS` flag for stamping one PDB into many.
⇒ isolates key **per PDB**, never per instance block. The instance block is the *channel's*
identity, and with VEID one of them can span 64 spaces.

**(c) On GSP we cannot OBSERVE the VEID binding.** RAMFC and runlist entries are written by
firmware; their layouts are not published. ⇒ which PDB slot a channel uses has to be tracked
from the **object model** (which ctxshare it was created under), not by watching hardware
structures. ⚠ That is a different source from everything else in our address plane, and as of
w774 we do not model it at all: `veid`/`subctx` appears nowhere in `ceresolve.rs` or `gpu.rs`.
