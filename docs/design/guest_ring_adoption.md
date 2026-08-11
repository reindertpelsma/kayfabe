# §5.11 — ADOPTING THE GUEST'S RING (`w230`)

> Series note: §5.9 is `fb_cpu_view.md`, §5.10 is `executor_vas_separation.md`. This is the
> rung after them, and it builds on §5.10's `ExecutorVas` without re-opening it.

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

## 1b. The boots — and the arming control that fired

| tag | rev | isolates | fb_backing | guest-RAM crossing | doorbells | G6 lines |
|---|---|---|---|---|---|---|
| `w230a_0aea0f2_guestring` | `0aea0f2` | real | on | — | **191/183/8** | 0 |
| `w230b_65d7532_guestring` | `65d7532` | real | on | — | **191/183/8** | 0 |
| `w230c_65d7532_guestring_memfd` | `65d7532` | real | on | `NVKVM_RAM_BACKEND=memfd` only | **191/183/8** | **0** ⚠ |
| `w230d_65d7532_guestring_gram` | `65d7532` | real | on | `NVKVM_RAM_BACKEND` **+** `KAYFABE_GUEST_RAM` | **191/183/8** | **0** ⊘ |

★ The census is **byte-identical to `w229c`** on every row — `191 arrived, 183 served, 8
REFUSED by name; last token 0x00010001 (16 logged)` — with `GR-FB-BACKING` at 32 and a 34-line
`dmesg` carrying 31 `NVRM` lines each time. The guest's addresses did not move and the
guest's behaviour did not change. ⊘ `cup2` times out on every row and was expected to; this
rung does not move the guest.

⚠ **`w230c` is the arming trap, caught by an instrument rather than by luck.** It was booted
with `NVKVM_RAM_BACKEND=memfd`, which is the flag that makes **QEMU** back guest RAM with a
memfd — and the guest-RAM crossing is armed by a **second, separate** variable,
`KAYFABE_GUEST_RAM=memfd` (`shim.rs:7229`, read in `selected_guest_ram_source`). So `w230c`
looked completely healthy — full `dmesg`, printed census, `CAPTURE_RC=0` — while **not one
line of G6 ran**. It is kept as the arming control it accidentally is: the four rows differ
by exactly the flags in their column, and `w230d` is the one that executes the changed code.

⇒ ★ This is `w229`'s lesson landing a second time in two nights, on a *different* variable.
The general form: **the flag you know about is not the flag that arms the code you changed.**
The only thing that separates the two is grepping the log for the specific line you expect —
which is why the verification script asks "G6 executed?" as a count and not as a `[ -f ]`.

### 1c. ⊘⊘ AND THE ARMING WAS NOT THE REASON — G6's call site is UNREACHABLE on this bench

★★★ Arming `KAYFABE_GUEST_RAM=memfd` too (`w230d`) did **not** produce a `GUEST-RAM PIN`
line either, and the census of committed evidence says why:

| line | boots that contain it |
|---|---|
| `GUEST-RAM PIN` | **2 of 101** committed `_qemu.log` files — `run_w226a`, `run_w226c`, **once each** |
| `RING-PROJ` (its neighbour on the same fall-through) | 8 of 101 |
| either one, in `w229a/b/c` — the rung that WROTE the pin | **0** |

`pin_ring_guest_ram` is called only on the **forwarding fall-through** of
`SharedDoorbell::ring`, which is reached only when `try_ce_submission` declines — and it
declines only for a doorbell that is both `DoorbellRoute::CpuCe` **and** has a resolved
`vas_pdb` (`shim.rs:4126`). In this boot's shape the 8 refusals are `GrCompute` (refused
above that test) and the 183 served are `CpuCe` with **no** `vas_pdb`, so they are claimed
terminally. Nothing falls through.

⇒ **G6 is written, reviewed and unexecuted, and no environment variable can change that.**
It is a routing fact, not an arming fact. ⊘ Do not read the code as evidence.

★ What *is* evidence is the one line the pin ever printed, and it corroborates G6's premise
exactly — `run_w226a_qemu.log`:

```text
GUEST-RAM PIN token=0x00010002 … pdb=0x2efa9c000 ring=0x420064000 gpa=0x23092000
    → file offset 0x23092000 (4096 bytes) → REFUSED SystemDataPlane
```

`ring=0x420064000` is the **4096-entry** ring (`RING-PROJ`, same boot family) — 32 KiB — and
the pin named **4096 bytes**. One eighth, in the only measurement that exists.

⇒ ★ **The next rung's first question is not the cursor; it is why `vas_pdb` is `None` for
every served doorbell on this bench when `w226` resolved it** (`pdb=0x2efa9c000`, printed
above). Until that is answered, everything on the forwarding fall-through — the pin, the
ring projection, the CPU page-table decode — is code nothing reaches.

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

### 3.4 ★★ AMENDED BY `R32` (2026-08-11) — an ordering constraint DOES exist, and it is USERD's

⊘ Everything in §3.3 stands: the **ring** imposes no ordering on the channel's birth.
`w233`/`R32` then measured the other object and found one that does.

On a GSP-client GPU with USERD in **sysmem**, `kchannelAllocOrDescribeInstMem` **zeroes the
512 bytes** at alloc time (`ogkm-580: …/fifo/kernel_channel.c:2342-2358` →
`kfifoSetupUserD_GM107`), and `R32` measured it landing on a caller-supplied `OS_DESCRIPTOR`
block: **all 512 bytes zeroed, attributable to the `userdOffset` we named**. ⇒ if the block
handed in is the guest's **live** USERD, the host channel's birth **wipes the guest's own
cursors**, silently, with the alloc still returning `NV_OK`.

⇒ ★ *"born at the first doorbell"* is safe for the **queue** and unsafe for the **cursors**,
because the first doorbell is by definition after the guest wrote `GP_PUT`. See
`docs/design/guest_userd_adoption.md` §3.1 for the three shapes that resolve it.

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

---

## 6. Evidence, by file

| what | where |
|---|---|
| R31 alone, first run | `docs/reference/bench_evidence/w230_r31_b39f95f.out` |
| the full ladder at the rung's first revision | `docs/reference/bench_evidence/w230_ladder_b39f95f.out` |
| ★ the full ladder at the **final** revision | `docs/reference/bench_evidence/w230_ladder_65d7532.out` |
| the four boots | `traces/guest_boots/run_w230{a,b,c,d}_*_{qemu,dmesg,probe,serial,isolate}.log` |
| ⚠ the flake attribution | `docs/reference/bench_evidence/w230_flake_attribution.out` |

### 6.1 ⚠ `cargo test --workspace` is NOT clean on this bench, and it is not this rung's

`WS_RC=101` at `65d7532`, and the failure is one test out of 149 in
`kayfabe-linux-raw::spawn_unsafe::tests`, with `execve` returning **errno 26 (`ETXTBSY`)`**.
Attribution, measured and on disk:

- a **different** test fails on each full-suite run (`one_image_spawns_more_than_once`, then
  `an_image_that_is_not_an_executable_fails_the_exec`);
- the same tests pass **3/3 when run alone**;
- ★ **the control**: the **untouched `w229` tree at `b66bd44`**, which contains none of this
  rung's code, fails the same module with the same errno in **2 of 6** repeats;
- and the same suite was `WS_RC=0` earlier tonight at `b39f95f`.

⇒ Flaky, pre-existing, and about test parallelism (a writable fd on an image another thread
is `execve`ing), not about this rung. `deterministic_failure_indicts_the_test; flaky indicts
the system` — recorded rather than "fixed", because silencing someone else's race at the end
of a rung is how a real defect gets a `#[ignore]`.

⊘ `tests/` (`TS_RC=0`) and the **single-writer census (7/7, `SW_RC=0`)** are clean at
`65d7532`.

⚠ The ladder was re-run at `65d7532` and not merely at `b39f95f`, because the last commit
touched `submit_entry` — which is the submission path `R15`, `R17`, `R26` and `R30` all go
through. A green ladder at the earlier revision would have been a green ladder for different
code.
