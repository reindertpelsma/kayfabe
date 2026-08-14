# w313 RESULT — the bisect REPRODUCED, the restore LANDS, and the gate that missed it is now a check

`[measured 2026-08-14, vh2, real GA106, host driver 580.159.04]`
Bench tree `/workspace/kayfabe_w313` (fresh clone), `CARGO_TARGET_DIR=/workspace/bench/cargo-target-w313`,
`BENCH_LOCK` held on **vh2** for the whole rung. **Stamp gate (`STAMP == HEAD`) passed on every
boot below.** Branch `w313-restore-two`.

> ## ★★★★★ THE HEADLINE
>
> **`R33 arm 1` PASSES again, and `^CUP3_VAL=43` still holds.** Both, on the same restored tree,
> one boot each, arming read back from the device's own emissions.
>
> ⊘ **And the sibling's bisect was NOT inherited — it was reproduced here, both sides**, one
> boot per side, before a line of the restore was written.

---

## 1. THE BISECT, REPRODUCED — not inherited

| rev | what it is | R33 arm 1 | tag |
|---|---|---|---|
| `0ff3e1e2` | **master** | ⊘ **FAIL** | `w313master1` |
| `8d258daa` | merge w305 (last rev where all five are env-gated) | ★ **PASS** | `w313rev8d25` |
| `4f20c3c1` | **the restore** | ★ **PASS** | `w313restore1` |

Same clone, same `CARGO_TARGET_DIR`, same box, same harness (`w309_crit1.sh fresh`, hoisted to
`/workspace/w313_crit1.sh` so a rev that predates it can still be driven), same boot arm
(`drain`), boots minutes apart. **The only variable is the source revision.**

**master, verbatim** — byte-identical to what w309 recorded:

```
FAIL  R33 arm 1 COPY      = dst[0] 0x3f0011cc -> 0x3f0011cc (want 0xc0ffee33),
      dst[last] 0x3f0011cc (want 0xc0fff232), semaphore 0x00000000 (want 0x00000001),
      GP_GET 1 GP_PUT 1 — the entry WAS fetched and the methods did nothing:
      SET_OBJECT class, subchannel, or an operand that does not resolve
```

**`8d258daa`, verbatim:**

```
★     R33 arm 1 COPY      = 4096 bytes moved: dst[0] 0x3f0011cc -> 0xc0ffee33,
      dst[last] 0xc0fff232 (want 0xc0fff232), engine semaphore 0x00000001
      (declared 0x00000001), GP_GET 1 caught GP_PUT 1 — read back through an
      INDEPENDENT mapping (its own device node, its own mmap, a kernel-chosen address)
```

⇒ **Nothing in w309's §4 needed correcting.** The bisect is mine now.

---

## 2. ★★★★★ THE RESTORE — arm 1 PASSES **and** 43 HOLDS

### 2.1 `R33 arm 1`, on the restored tree (`w313restore1`, HEAD `4f20c3c1`, STAMP == HEAD)

```
★     R33 arm 1 COPY      = 4096 bytes moved: dst[0] 0x3f0011cc -> 0xc0ffee33,
      dst[last] 0xc0fff232 (want 0xc0fff232), engine semaphore 0x00000001
      (declared 0x00000001), GP_GET 1 caught GP_PUT 1 — read back through an
      INDEPENDENT mapping (its own device node, its own mmap, a kernel-chosen address)
```

**The arming, read from the device's own emissions and not from the environment I set:**

```
      VAS-PUBLISH arm=drain fb_join=shared host_isolates=true
      OPERAND-JOIN arm=join
      PT-SWEEP arm=on
      PT-SWEEP tasks=3 skipped=1 ran=3
      VAS-CENSUS procs=2
```

⊘ **Scoped, said plainly.** Arms 5 and 6 still `FAIL` on the restored tree — and they fail
**identically at `8d258daa`**, so they are w309's two filed defects (`pde_info` answers the same
block for every address; plane D `GET_MMU_FAULT_INFO` does not decode) and **not** something this
restore did. `R33_RC=1` on both revs for that reason. **Only arm 1 changed.**

### 2.2 `cup3`, on the same restored tree (`w313restorecup3`, HEAD `4f20c3c1`)

```
--- ★★★★★ CUP3_VAL=43
    the kernel line, verbatim = [CUP3_KERNEL_LINE=KERNEL rv=43 want=43 -> PASS]
    (A) ★★★★★ FIRST COMPUTE. 43 cannot be copied, filled, or forged.
```

Criterion **(E)** — the new one, `regression_check_e.sh`, `host_rows` printed and never graded:

```
    (E1) Xid = 0                          ✔ PASS   (delta file exists, 0 bytes)
    (E2) drain ran: ★DRAINED rows=229  Σpinned=18228  max_single=13313   ✔ PASS
    (E3) pass invariants: NOT ARMABLE=0  host_isolates=NO=0  sum_ok=false=0   ✔ PASS
=== (E) VERDICT = 0
```

⇒ **The restore does not trade one workload for the other.**

---

## 3. ★★★★★ WHY `OPERAND_JOIN` IS NOT INERT — the direct explanation, measured

The brief asked whether the raw CE client produces non-zero candidates. **It does, and the
contrast is total.** Same binary, same flag, two workloads, one grep:

| boot | `OPERAND-JOIN-TABLE` lines | candidates |
|---|---|---|
| `w313restorecup3` (cup3, libcuda) | 96 | **`0 CANDIDATE(S)`** on every one — 91 read `ALREADY-JOINED` |
| `w313restore1` (raw CE client) | 2 | **`2 CANDIDATE(S)` on both** |

```
OPERAND-JOIN-TABLE: 2 page(s) asked, 0 MISS, 0 in guest RAM, 0 ALREADY JOINED,
  2 CANDIDATE(S) in the emulated framebuffer
  [va=0x120000000:Vidmem@0x10000/FakeFramebuffer va=0x120010000:Vidmem@0x20000/FakeFramebuffer]
OPERAND-JOIN-TABLE: 2 page(s) asked, 0 MISS, 0 in guest RAM, 0 ALREADY JOINED,
  2 CANDIDATE(S) in the emulated framebuffer
  [va=0x700100000:Vidmem@0x10000/FakeFramebuffer va=0x700200000:Vidmem@0x20000/FakeFramebuffer]
```

`0x120000000` / `0x120010000` are **arm 1's own operands** (its `OPERANDS` line names them);
`0x700100000` / `0x700200000` are arm 4's. ⇒ The raw CE client places its operands in the
**emulated framebuffer**, where the join is the thing that makes the guest's window and a real
host object one memory. cup3's operands resolve in guest RAM and are already joined by another
route, so its table is empty **by workload, not by mechanism**.

★ w304's sentence — *"all 96 `OPERAND-JOIN-TABLE` lines read `0 CANDIDATE(S)` … there was never
anything to join"* — is **exactly true of the boot it was measured on** and false as a statement
about the pass. That is the whole defect in one line, and it is now the fixture the new gate is
built around.

---

## 4. THE RESTORE DIFF — what came back and what stayed deleted

Reverted, from `f20ab952` (w304's deletion), **hunk-selectively**:

| restored | where |
|---|---|
| `PT_SWEEP_ENV`, `pt_sweep_from`, `selected_pt_sweep` | `shim.rs` |
| `SharedDoorbell::sweep_cpu_pt_tables`, `refusal_kind_va`, `PT_SWEEP_REFUSAL_CAP` | `shim.rs` |
| `SharedDevice::sweep_pt_tables_from` | `kayfabe-rt/src/device.rs` |
| `OperandJoinArm::Join`, `joins()`, `ALL` back to 3, `as_str`/parser accept `join` | `shim.rs` |
| the two `NOT ARMABLE` guards + the actual `join_one_fb_leaf` loop | `shim.rs` |
| the `PT-SWEEP arm=` banner | `shim.rs` (`Regs`) |
| `KAYFABE_PT_SWEEP=${…:-on}` and `KAYFABE_OPERAND_JOIN=${…:-join}` + their echo/readback rows | `w290p_run.sh`, `w297_cup3.sh` |
| the `plan_pt_sweep` "no production caller" STATUS block, removed (it has one again) | `ptdecode.rs` |

**Stayed deleted, correctly** — `pin_pushbuffer_guest_ram`, `pin_completion_guest_ram`,
`pin_operand_guest_ram`, `ce_release_pages`, `pushbuffer_pages` / `pushbuffer_runs` /
`PUSHBUF_MAX_EXTENTS`, the three `GUEST_*` env constants + enums + parsers + `SharedDoorbell`
fields, and the 418 lines of arm tests in `shim_logic.rs`.

### ★★★ TWO THINGS THAT ARE **NOT** A STRAIGHT REVERT, AND WHY

1. **w304's census un-gating is KEPT.** `GUEST-DESCRIBES / TABLE-DESCRIBES / HOST-PUBLISHED /
   PROMOTE-PARKED` used to be printed from *inside* `sweep_cpu_pt_tables`'s format string, so a
   boot with the sweep off printed no census at all and criterion (E) read that absence as a
   regressed address plane. `vas_census()` stays **unconditional**; the restored sweep prints its
   own separate ` | PT-SWEEP …` clause. Both appear on the `PT-DECODE` line, so a reader can tell
   *"the census ran and found nothing"* from *"the sweep was not armed"*. ⇒ The behaviour is
   restored; the reporting defect is not.
2. **`OperandJoinArm` is a THREE-arm selector again** (`off` / `assert` / `join`), and `join` is
   the default in `w290p_run.sh` — the value the passing boots actually ran. `assert` remains what
   it was: `off` plus the census.

⚠ **The correctness residual the sweep carries is unchanged and still stands** —
`ReachShadow::witness_swept`, owner ruling 2026-08-12. This restores a relaxation, deliberately,
because the alternative is a broken known-positive.

Clippy: clean on both feature arms except **one pre-existing `collapsible_if`** in
`vas_publish.publishes()` (untouched by this branch; present at master) and two in
`kayfabe-isolate-host`.

---

## 5. ★★★ THE GATE THAT MISSED IT — now a CHECK, not a paragraph

`scripts/bench/relaxation_inert_gate.sh`, with `scripts/bench/relaxation_inert_gate_selftest.sh`.

**Construction, not policy** — this tree's banked rule is that policy-shaped rules decay:

- **There is no single-workload mode.** `grade` takes **two** log paths positionally; `run` boots
  **two** arms from one ablation list. A caller cannot ask for half the gate; there is no flag.
- **A missing, empty or truncated log is `UNMEASURED` (exit 2), never a pass.** Each workload's
  own terminator (`^R33_RC=`, `^CUP3_RC=`) separates *"it ran and did not fire"* (**NOT-INERT** —
  a finding about the relaxation) from *"the artefact is short"* (**UNMEASURED** — a finding about
  the harness). ⚠ A truncated artefact reads as PRESENT and looks healthier than an empty one, so
  non-emptiness is never the test.
- **UNMEASURED outranks NOT-INERT.** If any plane is unmeasured, nothing is concluded — failing
  toward *"we do not know"* is the safe direction for a gate whose job is to stop a deletion.
- `INERT-ON-BOTH-PLANES` requires **both** known-positives to have FIRED, and the verdict line
  states its own scope: display, NVENC and multi-process are **unmeasured, not inert**.

### ★★★★★ The self-test's first case is w304's own evidence

`traces/w304_confirm/` holds w304's five arms. They are cup3 probe logs and there is **no raw-CE
log among them, because that boot never ran.** Graded by the new gate:

```
--- CASE 1 — ★★★★★ w304'S OWN EVIDENCE, GRADED BY THE NEW GATE.
  ✔ w304 ptsweep arm: cup3 only, no raw CE    exit=2  VERDICT = UNMEASURED
  ✔ w304 opjoin arm: cup3 only, no raw CE     exit=2  VERDICT = UNMEASURED
--- CASE 2 — the RESTORED tree: both planes fire
  ✔ w313 restore: cup3 43 + R33 arm 1 PASS    exit=0  VERDICT = INERT-ON-BOTH-PLANES
--- CASE 3 — MASTER: cup3 green, raw CE BROKEN  ← the case a one-workload gate called a pass
  ✔ master: cup3 43 + R33 arm 1 FAIL          exit=1  VERDICT = NOT-INERT
--- CASE 4 — artefact traps: empty / truncated / absent  → 3 × UNMEASURED
--- CASE 5 — grade with ONE log is refused (exit 64)
=== ★ SELF-TEST RESULT: failures = [0]
```

⇒ **The gate is watched failing, watched passing, and watched refusing** — over artefacts
committed in this tree, offline, in seconds, with no GPU. ⊘ A gate nobody has watched fail is a
wish.

### ★★ And its `run` half was exercised END TO END on the bench, not left as an unrun path

`relaxation_inert_gate.sh run w313gate` with **no ablation** (the committed baseline), two live
boots back to back, verbatim (`w313gate_runmode.log`):

```
=== w313 INERT GATE — ablation = []  tag=w313gate  repo=/workspace/kayfabe_w313
    workload 1  GR/COMPUTE  cup3 …  known-positive '^CUP3_VAL=43'          => [FIRED]
    workload 2  RAW CE      R33 arm 1 …  known-positive '★     R33 arm 1 COPY' => [FIRED]
W313 INERT-GATE VERDICT = INERT-ON-BOTH-PLANES
```

⇒ ★ **This is also the second independent measurement of both workloads on the restored tree**:
`w313restore1` + `w313gater33` for the raw CE plane, `w313restorecup3` + `w313gatecup3` for
cup3. **n=2 on each, four boots, same HEAD `e42f0f10`/`4f20c3c1` source.**

---

## 6. WHAT TURNED OUT WRONG, AND WHAT IS STILL OPEN

- ⊘ **Nothing in the brief turned out wrong.** The bisect held on both sides; both named
  relaxations were sufficient on their own; restoring them fixed arm 1; 43 survived. The
  possibility the brief flagged — *"restoring these two does not fix arm 1, which would mean the
  bisect caught a coincidence"* — **did not occur**.
- ★ **`n=2` on the restored tree, both workloads** (§5): the gate's `run` mode reran both arms
  and both fired again. The master / `8d258daa` contrast is `n=1` per side here, on top of
  w309's `2` and `1` — so the bisect stands at 3 and 2 across two lanes.
- ⊘ **Which of the two relaxations is doing the work is NOT decided here, and must not be read
  from this rung.** Both were restored together because each was independently sufficient to
  break arm 1 at `8d258daa`; this boot says the pair is sufficient to fix it and says nothing
  about either alone on the restored tree.
- ⊘ **Arms 5 and 6 remain broken** — w309's two filed defects, unchanged by this rung, and
  `pde_info` still answers the byte-identical block for every address. Nothing may be concluded
  from it until it varies with its argument.
- ⚠ `scripts/bench/w304_confirm.sh` is a w304-era artefact whose arms name the three deleted
  `GUEST_*` variables. It was not updated; it is a recorded harness, not a live one.
