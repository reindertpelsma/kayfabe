# PRE-REGISTRATION — R32, the framebuffer memfd JOIN

**STATUS: ★ SCORED ON HARDWARE 2026-08-11 (w260) — P1–P5, P7, P8 ALL HELD, P6 refuted as an
instrument fact. See §5.0.** Written and committed **before** the probe was built and before
any hardware ran it. Scored unedited: the scoring section is appended below the line, and
nothing above the line is touched afterwards.
⊘ **This STATUS line is the ONE exception, and it is edited on purpose.** It read *"LIVE
until scored"* for the whole of the day on which the probe stayed unrun, and `[measured
2026-08-11]` a research lane read this doc and concluded *"nobody has run it"* **after it had
been run**. A status block that cannot be corrected is the stale-doc failure `CLAUDE.md`
names, one level up. No prediction, confidence or refuter in §1–§4 has been touched.

Branch `fb-memfd-join`, based on `4428b6b`. Bench: `vh`, RTX 3060 GA106, host driver
580.159.04. Rung: `rmladder --fb-memfd-join` / `--fb-memfd-join-negative`. **No VM boot** —
this rung deliberately measures a property that does not need one.

---

## 1. What is being tested, and why it is not R25 again

R25 (`traces/real_ga106/rmladder_r25_osdescriptor_real_ga106.txt`) established, on this
exact GA106 and at both privilege levels, that a **sealed memfd** described to RM through
`NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` is placed at a dictated VA and read correctly by a real
copy engine: 65 536 of 65 536 bytes equal.

⊘ **R25 does not test the two things the framebuffer memfd proposal actually rests on.**

| | property | R25 | why it matters here |
|---|---|---|---|
| **J1** | **two independent mappings of one memfd are ONE memory across RM** — write through mapping **S**, describe mapping **I**, GPU reads what **S** wrote | ⊘ **no** — R25 writes and describes through the *same* `MappedRegion` | the shell holds the BAR view, the isolate holds the described view; they are different mappings in different processes. A design that only ever wrote through the described mapping would prove nothing about the shell's |
| **J2** | **GPU-write → CPU-read** through a described memfd | ⊘ **no** — R25 is CPU-write → GPU-read only | ★ this is the direction `cuCtxCreate` is stuck on. `COMPLETION-WATCH … NOT-OBSERVED samples=88` is a semaphore the **GPU must write** and the **guest must read**. Every byte of evidence this tree has for OS_DESCRIPTOR runs the other way |

⇒ R32 measures **J1 and J2 together**, over one memfd, with the vidmem object as the
GPU-side partner in both directions.

## 2. The chain, stated before it is written

```
   memfd  = SharedRam::create(SIZE)                 one sealed memfd, MFD_ALLOW_SEALING
   S      = Reservation A + map_fixed_in(memfd)     "the shell's BAR view"
   I      = Reservation B + map_fixed_in(memfd)     "the isolate's describe view"
                                                     ⇒ two DISJOINT host VAs, one file

   1. write P1 through S                            CPU stores, per word
   2. desc   = alloc_os_descriptor(I, 0, SIZE)      RM pins I's pages
   3. got_va = map_dma_both(desc, SIZE, FIXED at AT)
   4. dst    = alloc_device_local(SIZE)             real host vidmem
      dst_va = map_dma_both(dst, SIZE, anywhere)
   5. CE copy  got_va -> dst_va                     FORWARD
   6. compare dst through map_cpu against P1        ⇒ J1
   7. write P2 into dst through map_cpu             a DIFFERENT pattern
   8. CE copy  dst_va -> got_va                     REVERSE
   9. compare the memfd through S against P2        ⇒ J2
```

★ **Step 9 reads through `S`, never through `I`.** Reading back through the mapping that was
described would leave "did the other mapping see it" unasked — which is the whole of J1/J2.

★ **P1 ≠ P2 ≠ 0, all three distinguishable.** At step 9 the three failure modes print
differently: `0x00000000` = the copy never landed; `P1`-shaped = we are reading our own step-1
write and the reverse copy did nothing; `P2`-shaped = the GPU wrote and `S` read it.
⊘ A control that only distinguished "P2" from "not P2" would score the second case as a
generic failure and lose the one fact that names it.

## 3. ★ PREDICTIONS — scored unedited

| # | prediction | confidence | what refutes it |
|---|---|---|---|
| **P1** | The OS_DESCRIPTOR alloc **succeeds** over `I` | 0.97 | any `Err` from step 2 |
| **P2** | `placed_as_asked` — RM honours `DMA_OFFSET_FIXED` for described memory | 0.95 | `got_va != AT` |
| **P3** | **J1 holds**: every word the GPU read equals what mapping **S** wrote | **0.93** | any forward mismatch |
| **P4** | **J2 holds**: every word mapping **S** reads back equals `P2`, i.e. what the GPU wrote | **0.85** | any reverse mismatch |
| **P5** | `reverse_before == P1` — non-vacuity: the memfd held the *old* pattern immediately before the reverse copy | 0.95 | `reverse_before` is `0` or already `P2` |
| **P6** | The two mappings land at **different host VAs** (`s_addr != i_addr`) and the probe says so | 0.99 | equal addresses ⇒ the rung tested one mapping twice and is VOID |
| **P7** | The negative control (`S` never written, forward arm) **mismatches at word 0** with `got == 0` | 0.9 | a match, or a mismatch at a later word |
| **P8** | No `Xid` on the host, and the probe completes without the CE timing out | 0.9 | dmesg `Xid`, or `submit.semaphore != payload` |

★ **P4 is the one that carries the rung.** It is lower than P3 on purpose: GPU→sysmem writes
have to become visible to a CPU mapping of the same shmem folios, and nothing in this tree
has ever measured that. The two live risks are (i) the GPU's L2 not being flushed to sysmem
by the time the release semaphore retires, and (ii) `COHERENCY_CACHED` being wrong for a
buffer the GPU writes rather than reads.

⚠ **If P4 fails, that is the deliverable, not a setback.** It would mean the memfd route
delivers the guest's writes to the engine but not the engine's writes back to the guest —
which is precisely the semaphore, and would send the FB-memfd port back to the drawing board
while leaving the vidmem + `map_cpu` route (which is measured, §4) standing.

## 4. ⊘ What this rung does NOT establish, stated before it runs

- **It is not the boot falsifier.** The brief's falsifier — the framebuffer operands stop
  being blank in a live boot — needs the port. This measures the primitive the port would be
  built on, so that a failed boot could not be blamed on it.
- **It does not test two PROCESSES.** `S` and `I` are two mappings in one process. The
  cross-process case adds `SCM_RIGHTS` (built, both directions) and nothing else about the
  memory: two `mmap`s of one fd are the same folios whether or not an address space boundary
  sits between them. ⚠ Stating that as *reasoning* rather than as measurement is deliberate.
- **It does not test sparsity.** `SIZE` is fully materialized by step 1. Hole-punching under
  a live pin is refused by design (see the answer doc §4.2) and is not exercised here.
- **It says nothing about the shell's BAR trap path**, which is a separate seam (`FbStore`)
  and a separate rung.

---
<!-- SCORING GOES BELOW THIS LINE. NOTHING ABOVE IT IS EDITED AFTER THE FIRST BOOT. -->

## 5. SCORING — 2026-08-11

### 5.0 ★★★★★ CORRECTION, 2026-08-11 (w260) — THE PROBE HAS NOW RUN, AND EVERY OPEN ROW HELD

⊘⊘ **The paragraph in §5.1 below is SUPERSEDED and is kept only so the reader meets the
correction before the claim.** It says *"the probe was never run on hardware"*. That was true
when written and is **false as of 2026-08-11T16:11:44+00:00**.

`[measured 2026-08-11T16:11:44+00:00, bench `vh`, RTX 3060 GA106, host driver 580.159.04,
`REV_UNDER_TEST=62ab8755245b3c320de8365b08e3da4f1031292a`, stamp asserted equal to
`git rev-parse HEAD` before the probe was allowed to run, `PROBE_RC=0`]`
Evidence, committed: `traces/real_ga106/rmladder_r32_fb_memfd_join_real_ga106.txt` and
`…_negative_real_ga106.txt`.

| # | prediction | conf | verdict | the line that decides it |
|---|---|---|---|---|
| **P1** | OS_DESCRIPTOR alloc succeeds over `I` | 0.97 | ✅ **HELD** | the forward arm reached `placed at 0x0000000301400000` — the alloc is upstream of it |
| **P2** | `placed_as_asked` (`DMA_OFFSET_FIXED` honoured) | 0.95 | ✅ **HELD** | `placed at 0x0000000301400000 AS ASKED` |
| **P3** | **J1** — the GPU reads what mapping **S** wrote | 0.93 | ✅ **HELD** | `CE retired (sem 0x00000001), dst[0] 0xa112fffe -> 0x5eed0001, and 65536 of 65536 bytes compared EQUAL` |
| **P4** | ★ **J2** — **S** reads back what the GPU wrote | **0.85** | ✅ **HELD** | `CE retired (sem 0x00000002); … 65536 of 65536 bytes read back through S EQUAL — the GPU WROTE and the OTHER mapping READ IT` |
| **P5** | non-vacuity: `reverse_before == P1` | 0.95 | ✅ **HELD** | `the memfd held 0x5eed0001 through S immediately before the copy` — i.e. `P1`, ⊘ neither `0` nor already `P2` |
| **P6** | two mappings at different host VAs, *and the probe says so* | 0.99 | ⊘ **REFUTED** | unchanged — see §5.2; an instrument fact, not a system fact |
| **P7** | negative control mismatches at **word 0** with `got == 0` | 0.9 | ✅ **HELD** | `S was never written and the CE delivered 0x00000000 at word 0 where the pattern would have been 0x5eed0001` |
| **P8** | no host `Xid`, no CE timeout | 0.9 | ✅ **HELD** | both CEs retired (`sem` `0x1`/`0x2`), `PROBE_RC=0`, and **zero `Xid` in the run's window** |

★★★ **P4 is the row that carried the rung, and it is the one that matters beyond it.** The
doc's own §3 says *"this is the direction `cuCtxCreate` is stuck on … Every byte of evidence
this tree has for OS_DESCRIPTOR runs the other way."* It no longer does. **GPU-write →
CPU-read through a described memfd is measured**, 65 536/65 536 bytes, with a negative control
that fired. Both named risks — L2 not flushed to sysmem by semaphore retire, and a wrong
`COHERENCY_CACHED` for a GPU-written buffer — are **refuted by measurement** rather than
argued away.

⚠ **How P8 was scored, because the obvious way is wrong.** `dmesg | grep -ci xid` on this box
returns **6** — and all six predate this run by **≥10 hours** (latest at kernel `223896`,
against a run at ≈`260526`; the two most recent are `gpu_wedge_probe`'s, and three older ones
are earlier `kayfabe-rm-ladd` rungs). A raw count over a long-lived ring buffer is a **campaign
total, not this run's**, which is `boot_capture.sh`'s own watermark lesson applied to a probe
that does not use it. ⊘ A `Xid` count is not attributable without a time window.

⊘ **What this still does NOT establish** — §4's four limits are untouched by this measurement.
In particular it remains **one process, two mappings**: the cross-process case is still
reasoned (`SCM_RIGHTS`, two `mmap`s of one fd are the same folios), not measured. The `w260`
boot is where that gets exercised.

### 5.1 ⊘ SUPERSEDED by §5.0 — the state before the probe ran

⊘ **P1–P5, P7, P8: UNSCORED. The probe was never run on hardware.** The rung was reframed
mid-flight (the owner's four-kind GPGA taxonomy, `gpga_region_kind.md`) and the bench sync
was cut short by a timeout. There is no measurement, and none of these rows may be filled in
from reasoning. They stay open.

### 5.2 ★★★ P6 is scored, and it FAILED — as a fact about my instrument, not about the system

> **P6** — *"The two mappings land at different host VAs (`s_addr != i_addr`) **and the probe
> says so**"*, confidence 0.99.

⊘ **The probe cannot say so, and no probe in this crate ever can.**
`MappedRegion::addr_at` is **`pub(crate)`** by a deliberate refusal
(`kayfabe-linux-raw/src/mapping_unsafe.rs:549`, *"refusal 3 of §4.2.1 holding … No
representation of it crosses the crate boundary"*). The address is patched into an ioctl
argument by `Indirect` and **scrubbed back out** before the caller sees it again. A probe in
`kayfabe-isolate-host` has no way to obtain either mapping's address.

★ **I predicted an observation the architecture forbids**, at 0.99, and only writing the code
revealed it. Had P6 been a step rather than a prediction, the natural repair would have been
to widen `addr_at`'s visibility — deleting a refusal in order to satisfy a control.

**What replaced it**, and it is stronger than an address comparison: `FbJoinEvidence::joined()`
— write a probe word through `I`, read it through `S`, **before RM is in the path at all**.
That measures *sharing*, which is the property J1 and J2 actually need; distinctness stays
structural (two `Reservation::new` calls are two independent `mmap`s). ⊘ An address
comparison would have shown the mappings were *different* and said nothing about whether they
were the *same memory*.

⚠ Note which way the error ran: a comparison of two addresses is the kind of check that reads
as rigorous and would have been reported as a passed control. The property it tests is not the
property the rung needs.
