> ⊘⊘⊘ **ARCHIVED — THIS DESCRIBES A SUPERSEDED ARCHITECTURE. IT IS REFERENCE, NOT CURRENT.**
> The live design is `docs/design/THE_DESIGN.md`. This file is kept because its *measurements*,
> its *ogkm findings* and its *reasoning* remain useful — its **architecture does not**. Do not
> implement from it, and do not cite it as current. Archived 2026-09-21 (w822).

# ⊘⊘⊘ TITLE REFUTED — DMA WRITES WORK; THIS PROBE IS WRONG

**STATUS: ⊘ SUPERSEDED BY ITS OWN EVIDENCE, 2026-09-18 (w756f) — within the hour, and by a
commit already in this tree.**

`2ddce6e2` (w711) is a recorded baseline where the **raw client, two concurrent clients,
`CUP3_VAL=43` and an LLM producing 16 tokens ALL PASS** while every framebuffer leaf is a memfd
exported as `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` — guest "video memory" is host RAM over PCIe.

★★★ **An LLM cannot emit tokens without the GPU writing its results.** If the engine could not
write an `OS_DESCRIPTOR`, that baseline could not exist. ⇒ **GPU→DMA writes work on this
hardware, in this codebase.** The `0x56` below is a defect in the probe written to measure it —
the FIFTH such defect in one session, after: no `cuInit`, a 16-byte params block, `_TYPE_RM`
sent as `0`, and `alloc_sysmem`'s `NO_MAP`.

⚠ **The lesson is the one already written down and I failed to apply it to myself:** when a
probe answers NO, suspect the probe. I had the disproof in the tree — `the_all_dma_baseline_commit`
— and wrote a design doc naming an open question against the platform instead of checking it.
⊘ The measurement below is kept verbatim because the probe's behaviour is still what has to be
fixed; only its ATTRIBUTION was wrong.

**Originally: LIVE — measured 2026-09-18 (w756e), vast 51402274, RTX 3090, host driver
580.159.04, `--bare-metal-suite`.**

---

## What passes

```
UR_LEG1[vidmem] mapped_retired=true  semaphore=0x00000001 payload=0x00000001
UR_UNMAP[vidmem] issued=true
UR_LEG2[vidmem] after_retired=false semaphore=0x00000000 payload=0x00000002
UR_RESULT[vidmem]=PASS
```

★ **Our unmap retires the translation**, proven by the copy engine rather than by a ledger.
The semaphore never moves on leg 2. This is the owner's test (*"it munmaps and then ce must
fault on address"*) and it is green on bare metal.

## What is refused, and exactly where

```
UR_RESULT[sysmem-dma]=UNMEASURED:probe:Other(86) at step [(dma_buffer done)]
DR_RESULT=UNMEASURED:probe:Other(86) at step [ce_copy leg2 V->B]
```

`Other(86)` = `0x56 NV_ERR_INSUFFICIENT_PERMISSIONS`.

⇒ The round trip gets **all the way to leg 2**. So:

| direction | path | result |
|---|---|---|
| **CPU → GPU** | sysmem `A` → vidmem `V` | ✔ the engine READ bytes only the CPU wrote |
| **GPU → CPU** | vidmem `V` → sysmem `B` | ⊘ refused `0x56` |

**The engine can read an `OS_DESCRIPTOR` over our memfd; writing into one is refused.**

## What has been ruled out

- ⊘ **Not `alloc_sysmem`'s `NO_MAP`.** That was w756d's bug and is fixed: DMA memory is now a
  sealed memfd placed in a reservation we own and described to RM, with the CPU side being our
  own mapping.
- ⊘ **Not a write-sealed memfd.** `SharedRam::create` applies
  `F_SEAL_SHRINK | F_SEAL_GROW | F_SEAL_SEAL` — **no `F_SEAL_WRITE`**.
- ⊘ **Not a read-only GPU mapping of ours.** `raw_map_dma` passes `flags = 0`.
- ⊘ **Not a read-only allocation flag.** `NVOS02_FLAGS_ALLOC_USER_READ_ONLY` (21:21) and
  `ALLOC_DEVICE_READ_ONLY` (22:22) are both `_NO` (0) in what we send.
- ⊘ **Not the host mapping's protection.** `map_fixed_in` uses `HostProt::ReadWrite`.

## Why it matters, and why R25 never saw it

`prove_os_descriptor` (R25) copies **memfd → vidmem** — read-only from the descriptor's side.
So every existing test of this path exercises the direction that works, and the write
direction has never been measured until now. ⚠ That is the same shape as everything else this
week: **the suite was complete over the direction it tested.**

⇒ And it is load-bearing, not academic: the emulated CE plane's whole job includes writing
**into** guest RAM. A CE that cannot write an `OS_DESCRIPTOR` cannot serve an H2D-shaped
completion into guest memory.

## What to fix — in the PROBE, not the platform

`2ddce6e2`'s working path is the reference: `join_fb_leaf` / `alias_fb_leaf` describe a memfd as
an `OS_DESCRIPTOR` and the engine writes into it. This probe differs from that path in at least
one way worth checking first — **it lets RM choose the destination VA** (`map_dma_both(…, None)`),
where `prove_os_descriptor` dictates one and its own docs say why:
*"every address this probe owns is DICTATED, and far away … letting RM choose put the probe's
OWN channel ring at the address the isolate's ring had just been freed from."*

⊘ Named as the first thing to check, **not** as the cause: the probe now prints its step, so the
next run narrows it by construction. What is already settled is that the platform is not the
subject.
