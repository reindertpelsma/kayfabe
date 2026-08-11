# PRE-REGISTRATION — R32, the framebuffer memfd JOIN

**STATUS: PRE-REGISTERED 2026-08-11. LIVE until scored.** Written and committed **before**
the probe was built and before any hardware ran it. Scored unedited: the scoring section is
appended below the line, and nothing above the line is touched afterwards.

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

## 5. SCORING

*(to be filled from the bench run)*
