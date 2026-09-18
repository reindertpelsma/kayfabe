# The engine can READ our DMA memory and is refused when it WRITES

**STATUS: LIVE — measured 2026-09-18 (w756e), vast 51402274, RTX 3090, host driver
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

## Open — the next thing to establish

Whether the refusal is about (a) the descriptor's own permissions as RM sees them, (b) the
second (executor-space) mapping `map_dma_both` makes, or (c) the CE channel's client not having
write access to memory another client described. ⊘ Deliberately **not guessed**: this session
produced three wrong verdicts from reading a refusal as an answer, and `0x56` from an unknown
site is exactly that shape. The probe now names its step, so the next run can narrow it by
construction rather than by argument.
