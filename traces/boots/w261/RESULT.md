# w261 — LEG A BOOTED ON A REAL GA106. The GR ring's own leaf is JOINED for the first time, and `cuCtxCreate` did not move.

> ### STATUS — 2026-08-12 / **LIVE — MEASURED.** One arm (`ring`). Bench `vh`, GA106
> (`NVIDIA GeForce RTX 3060`), host driver **580.159.04**, source revision **`a72e63d`**
> (printed by the boot script itself, `w261_ring_capture.log` line 2).
> Graded against `docs/design/guest_ring_and_userd_adoption_prereg.md`, written before the code.

Arms: `KAYFABE_GUEST_RING=ring KAYFABE_GR_ROUTE=passthrough KAYFABE_FB_JOIN=shared
KAYFABE_ISOLATES=real KAYFABE_CE_EXECUTOR=local NVKVM_RAM_BACKEND=memfd KAYFABE_GUEST_RAM=memfd
KAYFABE_RING_VIDMEM=1 KAYFABE_PT_WITNESS_EXEC=on KAYFABE_FB_BACKING=on`.

## The pre-registered scorecard

| # | observable | predicted (`ring`) | **measured** | |
|---|---|---|---|---|
| P1 | `CUP2_RC` | **124** | **124** | ✔ as registered — stalls after `cuDeviceTotalMem`, at `cuCtxCreate` |
| P4 | `GR-RING-JOIN` naming FB phys `0x1000000` | ≥ 1 | **24, and every one of them is `0x1000000`** | ★★★★★ **FIRST EVER** |
| P5 | host GR channels born after their ring joined | ≥ 1 | **24 `materialized_channel=true`**, each on the line **after** its own join | ✔ (see ⊘ below) |
| P9 | `RmInitAdapter` failures | 0 | **0** (`34 dmesg lines, 31 NVRM, 0 adapter`) | ✔ |
| P10 | host `Xid` | none | **0 Xid**, asserted by the harness | ✔ |
| P12 | guest `NVRM` lines | 31 | **31** | ✔ |
| P6/P7 | `GP_PUT` / `GP_GET` on a GR channel | 0 on this arm | **0** | ✔ — leg B unbuilt |
| P8 | `CE-SUBMIT → RETIRED` | 0 | **0** | ✔ |

`CAPTURE_RC=0`. Build `BUILD_RC=0`; `grep -c 'No space left on device\|LLVM ERROR'` = **0** over
both the build log and the capture log, counted from the same invocation as the status.

## ★★★★★ What is new, and it is exactly what `gr_doorbell_passthrough.md` §4.2 asked for

That section named *"the highest-value single question left on this path"*: the operand census
joined FB phys `0x400000 / 0x600000 / 0x800000` and **never** the ring's `0x1000000`, *"because
a ring is not an operand of the methods it carries."*

```text
kayfabe: GR-RING-JOIN proc=2 chan=0 ring=0x200200000 entries=1024 → ★★★★★ THE RING'S OWN LEAF
         IS JOINED: memory=0xcafe00.. host_va=0x200200000 fb_phys=0x1000000
kayfabe: ENGINE-OBJECT class=0xc7c0 client=0xc1d0000c parent=0x5c000019 params=16B → FORWARDED
         engine=GrCompute host_object=0xcafe000a materialized_channel=true reused=false
```

`ring=0x200200000 entries=1024` on client `0xc1d0000c` is **the channel this campaign walls
on** — 45 byte-identical `RING-ROSTER` rows over 45 boots since `w206`. All 24 joins land on
the one 2 MiB leaf at `fb_phys=0x1000000`, which is the leaf `guest_ring_adoption.md` §4
resolved with two independent resolvers over five boots.

★★★ **And the ordering the whole rung was shaped around holds on hardware**: the join is the
line **immediately before** the birth, for every one of the 24 channels. `narrow(g.memory)`
requires the object to exist before the channel is born, and it does.

## ⊘⊘ WHAT THIS BOOT DOES **NOT** SHOW — read before citing P5

**Leg A2's firing is NOT directly witnessed.** Nothing prints *"this channel was born with
`RingSource::Guest`"*. What the log establishes is: the ring's leaf joined (24×), the birth
followed it (24×), and the adapter's own refusal for a non-joined object
(`RING_NOT_A_JOINED_WINDOW`) fired **zero** times. ⊘ **Zero refusals is consistent with BOTH
`adopt: Some` succeeding AND `adopt: None` never being asked** — this log cannot tell them
apart. `our_census_counts_intent_the_driver_counts_attempts`, and it is my instrumentation
gap, not an inference to paper over. ⇒ **The next rung's cheapest, highest-value line is a
print at the lowering site.**

## ⊘ Two source-level refutations this boot produced, both worth more than P4

1. **The engine-object latch does not see only the GR channel.** The first six `GR-RING-JOIN`
   rows are all `proc=0` — the SYSTEM proc — and none of them adopts anything:
   `chan=0/3 class=0xc7b5 → NOTHING TO ADOPT: vas_pdb=None`;
   `chan=1 ring=0x420064000 entries=4096 → NOT A FRAMEBUFFER LEAF: GuestRam { gpa: 0x32a0000 }`;
   `chan=2 ring=0x0 entries=32 → NOT A FRAMEBUFFER LEAF: Unresolved(CeWalk … Fault)`.
   ★ Each is a **correct** refusal by name, and each is a live instance of something a doc had
   only argued: a sysmem ring really does belong to the guest-RAM pin and not to this source,
   and `gpFifoOffset = 0` really is a **value** (the golden-context channel) that must not be
   synthesised into a ring.
2. **`0 Xid` here is a MEASURED absence, not a zero-byte artefact.** `run_w261_ring_hostdmesg.log`
   is 0 bytes, and the harness's own line says why — `0 Xid → …hostdmesg.log`. ⚠ The host's
   *lifetime* `dmesg` carries 6 Xids from earlier boots; the per-boot capture is the one that
   is zero. ⊘ A `[ -f ]` or a `wc -c` on that file would have read as a broken capture.

## ⊘ What a green here still cannot prove

Everything in the pre-registration's §3.3, unchanged — plus the P5 caveat above. In
particular: no guest work executed, the completion plane is untouched, and leg B's ordering
claim is unfalsifiable by a successful boot **and its arm does not exist at all**
(`leg_b_userd_adoption_blocker.md`).
