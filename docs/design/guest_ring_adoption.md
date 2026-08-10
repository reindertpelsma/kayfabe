# Adopting the guest's ring — `w230`

> **The blocker, stated exactly.** We allocate a host channel with **its own** command
> queue, which stays empty, so the GPU consumes nothing forever. The guest is pushing into
> **its** queue, which our channel does not read.
>
> ⊘ **The fix is not a copier.** The owner's ruling, measured working in the C at our exact
> address: map the guest's queue and command buffers into the GPU's view at identical
> addresses and let hardware read them directly. Under that shape the missing verb is not
> *copy the methods* — it is **advance one 32-bit cursor**, because the methods are already
> where hardware looks.

This rung builds the **alloc side** of that: a host channel whose GPFIFO is an object we did
not allocate, at an address and an entry count its caller states. It does **not** build the
cursor bridge, does not open `Route::NotACopyEngineChannel`, and moves nothing on a guest
path. `cup2` does not pass and the completion watcher stays `NOT-OBSERVED`.

---

## 1. What was measured, and where

All on the bench GA106 (`NVIDIA GeForce RTX 3060`, host driver `580.159.04`), at revision
`b39f95f`, `euid 0`, evidence in `docs/reference/bench_evidence/w230_ladder_b39f95f.out`.

`kayfabe-rm-ladder --gpu 0 --guest-ring-channel` (**R31**), four arms, every address it owns
dictated and used by no other rung:

| arm | what it asks | result |
|---|---|---|
| **D** | is the one refusal this port makes reachable? | ★ `gpFifoEntries = 0` → `RING_ENTRIES_REFUSED`, **and the CPU-map counter did not move** — nothing was allocated on the way |
| **A** | will host RM build a channel over a `memfd` → `OS_DESCRIPTOR` it did not allocate? | ★ **YES.** Token `0x4`, ring placed **as asked** at `0x9_0000_0000`, told `gpFifoOffset = 0x9_0000_3000` and `gpFifoEntries = 4096`; building it asked RM for **exactly one** CPU mapping (USERD) |
| **B** | can the guest-backed ring be CPU-mapped at all? | ★ **No** — `NV_ESC_RM_MAP_MEMORY` refused `NV_ERR_NOT_SUPPORTED` (`0x56`), with the driver's own `NVRM: memMap_IMPL: CPU mapping not supported for addressSpace: 0x1` in the host `dmesg` |
| **C** | does RM resolve `gpFifoOffset` at alloc time? | ⚠⚠ **NO — ACCEPTED** at an address nothing was ever mapped at |

The rest of the ladder at the same revision: `R31_RC=0 R30_RC=0 R26_RC=0 R26N_RC=0 R25_RC=0
R25N_RC=0 R29_RC=0 LADDER_RC=0`, including `R30 arm C REFUSED`, `R26 dictated ring` (`GP_GET
1 caught GP_PUT 1`), `R26n CONTROL FIRED`, `R15 SEM LANDED`, `R17 CE COPY`.

---

## 2. The five gaps, and what each one turned out to be

| # | brief | what landed | ⊘ where the brief was wrong |
|---|---|---|---|
| **G1** | the ring object is a handle handed in | `RingSource::Guest(GuestRing)`; `alloc_device_local` runs only on the `Ours` arm; `ChannelParts::owner` decides who unmaps and frees | — |
| **G2** | `gp_fifo_offset` from the guest's declared layout | `RingLayout::gp_fifo_va`, an **absolute VA** passed through | ⊘ there is no "ring base + offset" on the guest side: the guest declares `gpFifoOffset` directly, so the pass-through is of one number, not two |
| **G3** | ours is 64, the oracle fixture carries **512** | the modulus is now `ChannelParts::layout.entries` in both `next_slot` and `submit_entry` | ★ **REFUTED: 512 is the fixture's, not the guest's.** `run_w229b_b66bd44_execvas_real_qemu.log` shows this guest declaring **32**, **1024** and **4096** — the ring behind the doorbells we forward is **4096**, 64× ours |
| **G4** | no CPU map; on a guest-RAM object it fails `NV_ERR_INVALID_ARGUMENT 0x1F` | `ChannelRings::ring` is an `Option`; `ring_store_u32`/`ring_load_u32` refuse `RING_NOT_OURS`; `RmConnection::cpu_maps` counts attempts at the door | ★ **the status is REFUTED: it is `NV_ERR_NOT_SUPPORTED 0x56`**, from `memMap_IMPL`, not `0x1F` from the `MAPPING_NO_MAP` path the brief cited — a different refusal in a different function |
| **G6** | pin the whole ring — a loop over guest-physical runs | the extent is **derived** (`entries × 8`), the walk splits at every guest-physical discontinuity, and one `OS_DESCRIPTOR` is pinned per contiguous run | ★ the loop alone would not have been enough: with `RING_PIN_BYTES = 4096` **and** a derived length missing, the old code pinned **one eighth** of the 4096-entry ring. The number was not conservative, it was unrelated |

`G5` was done at `8776992` (`ExecutorVas`) and is untouched.

---

## 3. ★★★ THE ORDERING CHANGE — designed, and then REFUTED before it was written

### 3.1 What the brief asked for

> The host GR channel is materialized at engine-object-alloc time (`kayfabe-fwd/src/lib.rs`,
> `commit_engine_object`). Under this shape it cannot be created until its ring's
> address→physical binding exists, and that binding is committed on the doorbell path. ⇒ The
> host channel's birth moves to the first doorbell.

That relocation is expensive and was flagged as such: several driver allocations, a fixed
mapping, a bind, a token control and a schedule would move onto **a vCPU thread holding the
big lock**, inside the **6-second** service budget measured for the whole `cuCtxCreate`
path.

### 3.2 The design, stated as asked

Had it been necessary, it would have been:

1. `plan_engine_object` stops emitting `VerbPlan::EngineObject`'s lazy-channel arm and
   refuses `FwdFault::Unmaterialized` when `Channel::host_channel` is `None`. The engine
   object's alloc then **cannot** be the first materialization.
2. `plan_doorbell` grows a preceding phase — pin the ring's runs, then create the channel
   with `GuestRing`, then the deferred engine objects, then schedule, then ring — all inside
   one `round_trip`, so the whole thing is one worker checkout and one commit.
3. Because a sibling vCPU can be doing the same thing for the same channel, the commit stays
   a compare-and-swap on `Channel::host_channel` with `Stale::Rebound` on the loser, exactly
   as `commit_doorbell` already does; the loser frees its duplicate rather than overwriting.
4. The latency is bounded by doing the *pin* on the doorbell and nothing else: the engine
   objects and the schedule would be replayed from the channel's idempotency table.

### 3.3 ⊘ It is not necessary, and the reason is sourced and then measured

**Sourced, before the run.** The open driver forwards `gpFifoOffset` to GSP without
resolving it (`ogkm-580: src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:2664`), and RM
*itself* allocates a channel with `gpFifoOffset = 0`, saying why:

> *"Set the gpFifoOffset to zero intentionally since we only need this channel to be
> created, but will not submit any work to it. So it's fine not to provide a valid offset
> here."* — `ogkm-580: src/nvidia/src/kernel/gpu/gr/kernel_graphics.c:2420-2424`

**Measured, R31 arm C.** The same channel alloc with `gpFifoOffset = 0xB_0000_0000` — an
address nothing was ever mapped at, in an address space the probe allocated itself — was
**accepted**.

⇒ **A host channel does not need its ring bound in order to be born.** The binding is needed
when hardware **fetches**, which is after the doorbell — the same doorbell that commits the
pin. So the channel may keep being born exactly where it is born now, and the two numbers it
needs (`gpFifoOffset`, `gpFifoEntries`) are already in the core at that moment: the guest
declared them in its **own** channel alloc, and `kayfabe_core::rmgraph::GpFifoRing` has held
them since long before any doorbell.

⚠ **What this does not say.** It says nothing about `GPFIFO_SCHEDULE`, which is a different
call at a different time and may well validate; and nothing about what the engine does when
it fetches from an unbound VA (that is a host fault, `Xid 31 FAULT_PDE`, and it is the
failure mode the pin exists to prevent). ⇒ The ordering constraint that survives is
**pin-before-doorbell**, which is already where the pin is.

---

## 4. ⊘ What is NOT built, said plainly

- **The cursor bridge (G8).** Nothing writes the guest's `GP_PUT` into the host channel's
  USERD, so a channel built this way is accepted by RM, schedulable, and **fetches
  nothing**. `alloc_channel_over_guest_ring`'s own docs say so at the call site.
- **Any guest path.** `alloc_channel_over_guest_ring` has exactly one caller, the R31 probe.
  `plan_doorbell` and `commit_engine_object` are byte-for-byte unchanged, and the doorbell
  census is the invariant that says so.
- **The wall.** Every doorbell that reaches the pin on this bench belongs to the **system
  proc**, and `l1_concurrency.md` §12.26 gives it no data plane. G6's walk therefore
  resolves every run and is refused at the pin, by name. Re-opening §12.26 is an owner
  decision and is not this rung's to take.

---

## 5. Re-running it

```sh
# on the bench, as root, no guest and no QEMU
./target/release/kayfabe-rm-ladder --gpu 0 --guest-ring-channel ; echo R31_RC=$?
# the regression set
./target/release/kayfabe-rm-ladder --gpu 0 --executor-vas-alias        # R30
./target/release/kayfabe-rm-ladder --gpu 0 --dictated-ring             # R26
./target/release/kayfabe-rm-ladder --gpu 0 --dictated-ring-negative    # R26n
./target/release/kayfabe-rm-ladder --gpu 0 --osdesc-probe              # R25
./target/release/kayfabe-rm-ladder --gpu 0 --osdesc-negative           # R25n
./target/release/kayfabe-rm-ladder --gpu 0 --guest-ram-pin             # R29
./target/release/kayfabe-rm-ladder --gpu 0                             # R7..R17
```

⚠ R30 arm C provokes a real `Xid 31 FAULT_PDE` when the boundary holds; R31 provokes none —
it schedules nothing and rings nothing.
