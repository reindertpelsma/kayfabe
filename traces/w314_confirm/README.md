# `traces/w314_confirm/` — the bench confirmation of w310's guest-RAM pin release

> **STATUS: LIVE — 2026-08-14 (w314).** Branch `w314-pin-release-confirm`, rev `28882ec2`
> (= w310 rebased onto master `eb3d99ad`, plus this rung's two observational additions).
> Bench **`vh`**, real GA106, host driver open `580.159.04`.
> Verdict: **criteria A–G all PASS, 4/4 green boots** — and two findings the criteria did not
> ask for, both larger than the confirmation.

---

## 1. What was run

| tag | tree | rev | what |
|---|---|---|---|
| `w314cup3` | branch | `28882ec2` | two-workload gate, arm 1 (cup3) — **RED, and it was a flake** |
| `w314r33` | branch | `28882ec2` | two-workload gate, arm 2 (R33 arm 1, raw CE) — **arm 1 FIRED** |
| `w314basecup3` | **clean master** | `eb3d99ad` | control — **RED, byte-identically** |
| `w314basecup3b` | **clean master** | `eb3d99ad` | control n=2 — **`CUP3_VAL=43`** |
| `w314br1..4` | branch | `28882ec2` | cup3 ×4 — **43, 43, 43, 43** |
| `w314bt1..4` | **master + the REAP-TIMING instrument and nothing else** | `dd696ae6` | cup3 ×4 — **43, 43, 43, 43** |

⊘ **A `run_*_hostdmesg.log` of ZERO BYTES is the normal green.** It is a per-boot *delta*, so
an empty file means "no host driver message during this boot", not a truncated capture. The
two 228-byte ones are the two flaked boots, and they carry the fault.

---

## 2. ★★★★★ FINDING 1 — **cup3 IS FLAKY ON `vh`, AND IT IS FLAKY ON MASTER TOO**

`[measured 2026-08-14, vh, real GA106]`

| arm | boots | `^CUP3_VAL=43` | red |
|---|---|---|---|
| branch `28882ec2` | 5 | **4** | 1 (`w314cup3`) |
| clean master `eb3d99ad` (+instrument) | 5 | **4** | 1 (`w314basecup3`) |

**Both reds carry the SAME failure, field for field:**

```
FAIL cuCtxCreate(&ctx,0,d) -> unspecified launch failure (719)
CUP3_VAL=NO_KERNEL_LINE   CUP3_RC=1
NVRM: Xid (PCI:0000:00:07): 31, ..., channel 0x02000015, MMU Fault:
      ENGINE CE3 HUBCLIENT_CE1 faulted @ 0x2_0440f000  FAULT_PDE ACCESS_TYPE_VIRT_WRITE
```

and the guest `dmesg` of the branch's red boot is **byte-identical (modulo timestamps) to the
control's** — and *also* byte-identical to `traces/w297_cup3/run_w297cup3_dmesg.log`, the
**green** w297 boot. ⇒ the 31 `NVRM` assertion lines are the baseline, not a signature.

### ⊘⊘⊘ What this costs, and it is not small

**One cup3 boot is not a measurement.** Had this rung graded its first boot and stopped — which
is exactly what w310's criterion A, w297's, w298's thirteen arms and w304's five all do — it
would have reported **`(D) 43 regresses ⇒ the release is not safe as built`** and stopped, on a
release that is in fact green 4/4. The control boot is the only reason that did not happen.

⇒ **A single-boot `^CUP3_VAL=43` grade has a ~20 % false-negative rate on this box today.**
Every rung graded on n=1 carries it. `n>=2`, or a same-hour control on the other arm, or both.

⚠ Unattributed: I did not find the cause. Both reds were among the first boots of their batch
after a long idle, but `w314br1` was also a first-of-batch and came back green, so
*"first boot after idle"* is **not** established. It is a rate, not a mechanism.

---

## 3. ★★★★★ FINDING 2 — **THE UNBOUNDED BQL DISPOSAL IS NOT "CLOSE TO BITING". IT IS AT 92.6 %.**

`docs/design/guest_ram_pin_release.md` §5 names a **pre-existing** clause-(b) violation —
w303's armed reap runs an unbounded disposal inside `Regs::write`, under the QEMU BQL, with
every vCPU halted — and derives *"~3 s"* **by arithmetic**. This rung measured it, with the
identical instrument on both trees so the attribution is a measurement and not an argument.

`[measured 2026-08-14, vh, `max_reap_us` = longest single `reap_retired()` inside `Regs::write`]`

| arm | boot 1 | boot 2 | boot 3 | boot 4 | mean | worst vs the **4 000 ms** `scrubberDestruct` budget |
|---|---|---|---|---|---|---|
| master `eb3d99ad` + instrument only | 2 648 366 | 2 918 210 | 2 666 893 | 2 772 771 | **2.75 s** | **73.0 %** |
| w310 branch `28882ec2` | 3 336 519 | 3 702 806 | 3 263 826 | 3 250 535 | **3.39 s** | **92.6 %** |

**The ranges do not overlap** (master max 2.918 s < branch min 3.264 s), n=4 each.

- ★★★ **Master alone already halts every vCPU for 2.65–2.92 s.** That is the violation, it
  is w303's, and it exists without this rung. The arithmetic was right.
- ★★ **w310 adds ~637 ms, +23 %**, taking the worst observed boot to within **297 ms** of the
  timeout. ⊘ On the *standard* workload, on a *green* boot. Not a stress case.
- ⊘⊘ **And w310's own cost estimate for that addition is low by ~20×.** §5 says the increment
  is *"one `munmap` per pin — a local syscall (~1–2 µs), not a host RM ioctl round trip"*.
  Measured: 637 ms / 18 228 pins = **~35 µs per pin**. `munmap` of a `MAP_SHARED` memfd window
  that RM has `pin_user_pages`-pinned is not a cheap local syscall.

### ⊘ AND THE CRITERION THAT WAS SUPPOSED TO CATCH THIS **CANNOT**

Criterion D — *"guest `dmesg` carries no soft lockup and no RCU stall"* — **passed on every
boot, including the 3.70 s one.** It had to: Linux's soft-lockup watchdog fires at
`2 × watchdog_thresh` ≈ **20 s**, five times the budget that actually matters. ⇒ **D is a
witness for a catastrophe, not for this.** It is necessary and it is nowhere near sufficient,
and reading its green as *"the disposal is bounded in practice"* would have been wrong by a
factor of five. The number is the instrument; the stall detector is not.

---

## 4. The criteria, A–G, as w310 wrote them — verdicts

Graded by `scripts/bench/w314_grade.sh` (known-positives in `w314_grade_selftest.sh`, nine
fixtures, offline). **All four green branch boots grade `VERDICT = 0`.**

| # | criterion | verdict | the number |
|---|---|---|---|
| A | `^CUP3_VAL=43` | ✔ **PASS ×4** | `CUP3_VAL=43  CUP3_RC=0` |
| B | `regression_check_e.sh` | ✔ **PASS** | E1 `Xid=0` · E2 `★DRAINED rows` ≥1, `Σpinned` ≥1 · E3 all 0 |
| C | `PIN-RELEASE released=N`, `N>0` | ✔ **PASS** | **`released=18228`**, identical on all 4 boots |
| D | no soft lockup / no RCU stall | ✔ **PASS** | `stall-signature lines = 0`, `NVRM lines = 31` — ⚠ see §3 |
| E | `refused_no_host_vas=0` | ✔ **PASS** | `0` |
| F | no new `Xid` **classes** | ✔ **PASS** | observed class set **empty**, = the baseline's |
| G | `REAP` and `PIN-RELEASE` both present | ✔ **PASS** | `REAP` ×2, `PIN-RELEASE` ×1 |

★ And the **second workload**, which the brief added and w310's list does not contain:
**`R33 arm 1` FIRED** on the branch — 4096 bytes moved, `GP_GET 1` caught `GP_PUT 1`, read back
through an independent mapping. The raw-CE plane is green **with the release active**.

`rows_deduped=18228` — **every** released pin was also an exact-extent address-table row. ⇒ the
double-free door §4 of the design describes is not hypothetical: without the dedupe, all 18 228
host objects would have been freed twice. The count equalling `released` exactly is what a
correct dedupe looks like on today's default arm, where the run-pin population is ~0.

---

## 5. ⊘ Where the pre-registered criteria mis-scope, found by running them

1. **`W313 INERT-GATE VERDICT = NOT-INERT` on the gate run is an artefact of the flake**, not a
   result: cup3's arm read `RAN-BUT-ABSENT` on the one red boot. The gate is right; the input
   was one boot.
2. **Criteria A, B and F are cup3-only and must not be run over the R33 arm.** The `fresh` arm
   of `w309_crit1.sh` **provokes a fault on purpose** (`CONTROL-NEVER-LANDED` is its
   pre-registered outcome), so its boot legitimately carries `Xid 31 CE0 … @ 0x7_00100000` —
   arm 4's own control operand. `w314_grade.sh` grades it anyway and reports `(B) REGRESSION`
   and `(F) FAIL`; **both are false on that arm** and the drive log says so inline.
3. **`regression_check_e.sh` has a cosmetic defect**: its informational `host_rows` list uses
   `sort -t= -k2 -n`, which errors with `separator must be exactly one character long: ''`, so
   that list never prints. Informational only, never graded; named, not fixed, because editing
   a criterion during its own confirmation is the diff nobody can review.
4. **The `PIN-RELEASE` line reads the narrower of the two available tallies.**
   `Regs::write` calls `SharedDevice::pin_reclaim_gone()`, which is `Spine::pin_reclaim_gone`
   alone — pins reclaimed from procs that have **left the live set**. `Gpu::pin_reclaim()`
   (`gpu.rs:5295`) sums that *plus* `system` *plus* every live proc, and is not exposed on
   `SharedDevice`. ⇒ **the shape w310 built this rung for — a `Vas` that dies while its proc
   lives — increments a counter the boot line cannot see.** It did not bite here (the proc
   exits and the whole tally lands at once) but it is exactly the ABSENT-vs-ZERO hole criterion
   C spends a paragraph warning about, sitting in the emitter rather than in the reader.
