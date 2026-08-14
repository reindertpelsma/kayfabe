# w298 — ABLATING THE ELEVEN RELAXATIONS AGAINST THE `43` KNOWN-POSITIVE

**STATUS: LIVE, 2026-08-14.** Measured on `vh` (RTX 3060 **GA106**, host driver open `580.159.04`),
source revision **`50a8ca28`** (= master `91f8b34b` + the harness commit in this branch). 16 boots,
one variable per boot except where stated. Every arm's grading log, probe log and host-dmesg delta
is in this directory; `w298_summary.txt` is the machine-written row-per-boot ledger.

## ★★★★★ FIRST — THE BASELINE REPRODUCED. `43` is now 2/2.

`^CUP3_VAL=43`, `^CUP3_RC=0`, ladder 8/8, `Xid=0`, `host_rows=18295 of 18309`, JIT present.
Byte-identical arming to w297. ⇒ every ablation below rests on a reproduced positive, not on one boot.

## The table

`FIRST ✘` is the first failed rung of cup3's own stage ladder.

> ### ⊘⊘ CORRECTED WITHIN THE HOUR — **THE `host_rows` COLUMN WAS SORTED LEXICOGRAPHICALLY, NOT
> ### NUMERICALLY, AND TWO CELLS WERE WRONG IN THE DIRECTION THAT FLATTERED A CONCLUSION.**
> The summary row built its `host_rows` field with `sort -u`, so `"host_rows=4 of 13348"` sorts
> **after** `"host_rows=18295 of 18309"` (`'4' > '1'`) and the *lexicographic last* was printed as
> if it were the peak. Two cells changed on re-derivation with `sort -n`:
> - **`GUEST_PUSHBUF=off`: `4/13348` → `18295/18309`.** Harmless — it strengthens the row (the
>   address plane was *unchanged*, not degraded).
> - ★ **`PT_WITNESS_EXEC=off`: `4/8` → `13342/13348`.** **Not harmless.** `4 of 8` supported a claim
>   in §3 that the address table was *empty*; `13342 of 13348` says it was **substantially
>   published** and the arm failed anyway. §3 is corrected below.
> ⚠ Same class this tree keeps paying for: the number was *present, plausible, and cited* — only
> re-deriving it with the right comparator showed it was not the number it was labelled as.
> The table below is the `sort -n` re-derivation.

| # | variable | w297 value → ablated | `^CUP3_VAL=` | first ✘ | Xid | host_rows (peak) | verdict |
|---|---|---|---|---|---|---|---|
| — | *(baseline)* | — | **43** | none | 0 | 18295/18309 | ★ reproduced |
| 1 | `KAYFABE_PT_SWEEP` | `on` → **`off`** | **43** | none | 0 | ⊘ no `HOST-PUBLISHED` line at all | ★★★★★ **INERT** |
| 2 | `KAYFABE_VAS_PUBLISH` | `drain` → **`assert`** | ⊘ ABSENT (`RC=1`) | `CTX OK` | 1 | 23/16425 | **LOAD-BEARING** |
| 3 | `KAYFABE_OPERAND_JOIN` | `join` → **`assert`** | **43** | none | 0 | 18295/18309 | ★★★ **INERT** |
| 4 | `KAYFABE_GR_ROUTE` | `passthrough` → **`refuse`** | ⊘ ABSENT (`RC=124`) | `CTX OK` | 0 | 13342/13348 | **LOAD-BEARING** |
| 5 | `KAYFABE_GUEST_RING` | `ring` → **`off`** | ⊘ ABSENT (`RC=124`) | `CTX OK` | 0 | 13342/13348 | **LOAD-BEARING** |
| 6 | `KAYFABE_GUEST_PUSHBUF` | `pin` → **`off`** | **43** | none | 0 | 18295/18309 | ★★★ **INERT** |
| 7 | `KAYFABE_GUEST_SEMA` | `pin` → **`off`** | **43** | none | 0 | 18295/18309 | ★★★ **INERT** |
| 8 | `KAYFABE_GUEST_OPERAND` | `pin` → **`off`** | **43** | none | 0 | 18295/18309 | ★★★ **INERT** |
| 9 | `KAYFABE_FB_JOIN` | `shared` → **`off`** | ⊘ ABSENT (`RC=124`) | `CTX OK` | 0 | 17 (of 13348) | LOAD-BEARING ⚠ *not single-variable* |
| 10 | `KAYFABE_ISOLATES` | `real` → **`stillborn`** | ⊘ ABSENT | — | ⊘ | ⊘ | ⊘ **NOT MEASURABLE** (realize refused) |
| 11 | `KAYFABE_CE_EXECUTOR` | `host` → **`local`** | ⊘ ABSENT (`RC=1`) | `CTX OK` | 0 | ⊘ absent | **LOAD-BEARING** |
| 12 | `KAYFABE_PT_WITNESS_EXEC` | `on` → **`off`** | ⊘ ABSENT (`RC=1`) | `CTX OK` | 1 | **13342/13348** | **LOAD-BEARING** |
| 13 | *(combined)* `PT_SWEEP=off` **+** `VAS_PUBLISH=assert` | | ⊘ ABSENT (`RC=1`) | `CTX OK` | 1 | ⊘ absent | see §2 |

★ Note the totals cluster: every arm that fails to get the compute channel running tops out at a
**13 348-row** VAS, while every passing arm reaches **18 309**. The 13 348 shape is therefore a
*consequence* of not getting there, not a cause — do not read it as a distinguishing cause.

⊘ **`^CUP3_VAL=` ABSENT is UNMEASURED, not 0 and not a failure value.** Rows 2, 4, 5, 9, 11, 12, 13
each carry a `^CUP3_RC=`, so the *program* terminated and was graded; only the computed value is
absent because cup3 never reached the kernel. Row 10 has no `CUP3_RC` either — nothing ran at all.

`RC=1` vs `RC=124` is a real distinction: `1` is cup3's own exit after `cuCtxCreate` returned
`unknown error (999)`; `124` is the inner `timeout 300` — the guest hung rather than erroring.

## §1 — THE HEADLINE: `PT_SWEEP`, the ONE correctness relaxation, IS INERT

`KAYFABE_PT_SWEEP=off` returned **43**, with the identical ladder and `Xid=0`.

This is the result the brief flagged as *"a big result if it survives"*. It survives. And the
supporting numbers say it is not a fluke of scheduling: the drain's actual work is **byte-identical**
between the baseline and the `PT_SWEEP=off` arm —

```
base     GUEST-RAM PIN pinned total = 54678    ★DRAINED rows = 229
ptsweep  GUEST-RAM PIN pinned total = 54678    ★DRAINED rows = 229
bothoff  GUEST-RAM PIN pinned total =  2256    ★DRAINED rows =   0
```

⇒ **the sweep contributed nothing that cup3 consumed.** What it *did* contribute is the framebuffer
row population that `HOST-PUBLISHED` reports: with the sweep off, `HOST-PUBLISHED` never printed at
all (`grep -c` = 0) and `host_rows` never appeared — and cup3 still crossed.

★★★ **THEREFORE `host_rows=18295 of 18309` IS NOT A REQUIREMENT OF THE PASS**, and w297's
pre-registered regression criterion (E) — *"`host_rows` != 18295/18309 ⇒ REGRESSION, checked even on
a green"* — **would have flagged this green as a regression.** It is not one. Criterion (E) is
measuring the sweep's output, not the pass's precondition, and should be re-scoped.

⚠ **Scope, stated plainly:** this says PT_SWEEP is inert **for cup3 on this workload and this boot
shape**. It does not say the sweep is useless; it says the thing this campaign has been treating as
load-bearing, and paying a correctness-gate relaxation for, is not on cup3's critical path.

## §2 — WHAT `VAS_PUBLISH=drain` IS HOLDING UP, BY IDENTITY

`assert` (census everything, publish nothing) faults, and the fault is exactly the one the arm was
commissioned against:

```
Xid (PCI:0000:00:07): 31   FAULT_PDE   ENGINE GRAPHICS   HUBCLIENT_FE   ACCESS_TYPE_VIRT_WRITE
faulted @ 0x70ab_a8e00000
    GUEST-DESCRIBES  OWNS-FAULT = YES proc=2 pdb=0x201000 run=0x70aba8e00000+0x400000
    TABLE-DESCRIBES  OWNS-FAULT = YES proc=2 pdb=0x201000 run=0x70aba8e00000+0x400000
    HOST-PUBLISHED   OWNS-FAULT = NO
```

★ **The join answers the campaign's standing question directly**: the guest describes the run, **our
own table describes the run**, and the **host VAS does not have it** — `FAULT_PDE`, i.e. the descent
died above the leaf, because there is no directory there at all. This is `the_table_is_right_and_the_host_vas_is_empty`
reproduced on a compute workload, and it is why publication is necessary.

The combined arm (#13) settles the interaction: `PT_SWEEP=off` **+** `VAS_PUBLISH=assert` fails the
same way (`FAULT_PDE GRAPHICS/FE VIRT_WRITE @ 0x75b9_b2e00000`, a different but structurally
identical VA — these are per-boot ASLR'd UVM ranges). ⇒ **the publication is load-bearing on its
own; it is not merely cleaning up after the sweep.** The hypothesis *"the sweep creates the
population the publication then has to carry"* is **refuted** by that boot.

★ And note **which half** of `drain` carries it: the guest-RAM pin totals above show `drain`'s
value is in the **guest-RAM pin of the doorbelled VAS** (54 678 pages), not in the framebuffer row
publication (`host_rows`), which was absent from a passing boot.

## §3 — the other load-bearing arms, by identity

- **`GR_ROUTE=refuse`** — 10 × the named refusal **`Route::NotACopyEngineChannel`**, no fault at all
  (nothing was forwarded, so nothing could fault). Doorbells collapse `GrCompute 125→8`, `Ce 355→183`.
  Guest hangs (`RC=124`). This is the arm working exactly as documented.
- **`GUEST_RING=off`** — **zero `GR-RING-JOIN` lines** (baseline: 97). No named refusal, no fault,
  and the *same* doorbell collapse as `GR_ROUTE=refuse` (`GrCompute=8, Ce=183`) and the same
  `host_rows=13342/13348`. ⇒ leg A and leg C fail **identically from the outside**: without the
  guest's own ring adopted at channel birth, the host channel has nothing to fetch, so the
  passthrough route has nothing to carry. The two are not independent in effect even though the
  source keeps them as separate variables.
- **`CE_EXECUTOR=local`** — the plane essentially stops: **`GrCompute=0 Ce=2`** doorbells for the
  whole boot, `REFUSED host_chan=NONE NoVas(ChanId(0))`, `cuCtxCreate → 999`.
  ⚠ **This contradicts the flag's own doc-comment**, which calls `local` *"the default, and the only
  value under which a guest has ever reached `cuCtxCreate` with a live isolate plane installed"* and
  documents `host` as the arm that died three times (`Other(19270)`, `RmInitAdapter failed`). That
  text records a **w231-era** measurement and is now **inverted**: on this revision `host` passes and
  `local` fails. A dated STATUS block is owed on `CE_EXECUTOR_ENV`'s doc.
- **`PT_WITNESS_EXEC=off`** — `FAULT_PDE GRAPHICS/FE VIRT_WRITE @ 0x7328_14e00000`, arm witnessed in
  force by **88 × `EXEC-WITNESS DISARMED`** emissions.
  > ⊘ **CORRECTED — the first draft of this bullet said "the address plane is **empty**:
  > `host_rows=4 of 8`", and that was the lexicographic-sort artefact described above.** The real
  > peak is **`13342 of 13348`**, and `PT-SWEEP` **did** run (`tasks=4 skipped=0 ran=4` appears
  > beside the `tasks=0 … ran=0` reading the summary's `tail -1` happened to catch). ⇒ the tidy
  > story — *"unwitnessed executor pages ⇒ `ReachShadow` refuses to bind ⇒ the table stays empty
  > ⇒ fault"* — **is not what this boot shows.** The table was substantially populated and
  > published and the engine faulted anyway.

  What the arm *is* measured to do: it never reaches the **18 309**-row VAS shape every passing boot
  has, stalling at the 13 348 shape shared by every failing arm, and it faults `FAULT_PDE` — the
  descent dying above the leaf. ⚠ **Why** is not established here. `PT_WITNESS_EXEC` is load-bearing
  on the evidence (green→red, one variable), but the *mechanism* claimed by the source's doc-comment
  (an empty table) is **not corroborated by this boot** and needs its own measurement.
- ★ On the ordering claim: `PT_WITNESS_EXEC` feeds the queue `decode_cpu_pt_writes` drains, which is
  what arms the sweep, so it is upstream of `PT_SWEEP` — the §1 ablation is only meaningful *because*
  the witness stayed armed. That dependency is stated in the source; this rung did not test it
  directly and does not claim to have.

## §4 — the two arms that are NOT clean single-variable ablations

- **`FB_JOIN=off` disarms three things.** The qemu log carries **16 ×
  `⊘ NOT ARMABLE: KAYFABE_FB_JOIN is 'off'. ⊘ Nothing was asked of the host`** — the interlocks in
  `join_operand_fb_leaves` and `publish_vas_rows` refuse to realize rather than silently downgrading
  to a private-anonymous mapping. ⇒ this boot ablated `FB_JOIN` **and** `OPERAND_JOIN` **and**
  `VAS_PUBLISH` at once. Its red is therefore *attributable to the set*, not to `FB_JOIN`, and given
  #2 is independently load-bearing the outcome was over-determined. Scored LOAD-BEARING, flagged.
- **`ISOLATES=stillborn` cannot be measured against this harness at all.** The device **refused to
  realize**, by name and correctly:
  > `nvkvm: the register plane refused to build (3): KAYFABE_GUEST_RAM=memfd asks for guest memory
  > to cross into an isolate, and KAYFABE_ISOLATES is 'stillborn' — the plane that retires every
  > isolate at birth. There is nothing to grant it to.`

  QEMU never started (`run_w298isolates_qemu.log` = 720 bytes). ⊘ This is **UNMEASURED for cup3**,
  not a load-bearing finding — and it is the right behaviour: a run that quietly granted nothing
  would be indistinguishable from its own negative control. Measuring it would require also unsetting
  `KAYFABE_GUEST_RAM`, i.e. a two-variable boot, and `ISOLATES` is the forwarding plane itself rather
  than a relaxation carried on top of it.

## §5 — CANDIDATES TO COME OFF MASTER

Four arms are **inert for cup3** — carried, and never needed on this path:

| arm | why it is a deletion candidate |
|---|---|
| **`KAYFABE_PT_SWEEP=on`** | ★★★★★ the highest-value one: it is the **only correctness-gate relaxation in the file** (the source says so three times) and cup3 does not need it. |
| **`KAYFABE_OPERAND_JOIN=join`** | green with `assert`, `host_rows` **unchanged** at 18295/18309. |
| **`KAYFABE_GUEST_PUSHBUF=pin`** | green with `off`. |
| **`KAYFABE_GUEST_SEMA=pin`** | green with `off`, `host_rows` unchanged. |
| **`KAYFABE_GUEST_OPERAND=pin`** | green with `off`, `host_rows` unchanged. |

⚠ **Do not delete on one boot.** Each was ablated once. The three guest-RAM pins (6, 7, 8) are
*supply-side* passes the source says "gate nothing" — consistent with them being invisible to cup3 —
but they were each commissioned against a **specific measured fault** (pushbuffer-VA `HUBCLIENT_ESC`
reads; the `0x2_0440f000` semaphore write; the `0x2_04420000` operand write). The honest reading is
that **`VAS_PUBLISH=drain`'s whole-VAS guest-RAM pin now covers what they were each pinning
individually** — 54 678 pages is a superset — which is a *consolidation* opportunity, not an
"unnecessary all along". Confirm by ablating each a second time, and by checking whether `drain`'s
pin set contains the specific pages each of the three would have pinned.

## §6 — ★ WHAT THIS BRIEF GOT WRONG

- ★★★ **"`PT_SWEEP` … relaxes a gate whose stated mitigation is the dirty-driven re-sweep, and the
  rule recorded in this tree is 'run both or neither'."** — **There is no such rule.** Searched the
  whole of `shim.rs` for `re-sweep|resweep|dirty-driven|both or neither|mitigat`. What exists is
  (a) the *dirty signal* (`shim.rs:4703`, `:9713-9715` — `decode_cpu_pt_writes` "is the source of the
  dirty signal this sweep re-arms on"), and (b) a one-directional ordering statement at `:9720-9722`:
  *"Running the sweep without the decode pass would have the relaxation and not its mitigation. The
  order here — decode first, sweep second — is what makes a write that landed this window arm the
  sweep in the same doorbell rather than the next one."* The mitigation is **the decode pass plus the
  ordering**, not a re-sweep — and **`decode_cpu_pt_writes` has no flag at all** (called
  unconditionally at `:4711`), so "run both or neither" is **not a configuration any boot can
  violate**. The premise was not merely mis-stated; it described a knob that does not exist.
- ⚠ **"`VAS_PUBLISH=drain` is the one that moved the wall (`host_rows` 4 → 18 295). Expect it to be
  load-bearing."** — load-bearing, yes. But `host_rows` turned out to be the **wrong witness for it**:
  a boot with **no `HOST-PUBLISHED` line at all** passed at 43 (§1). The number that tracks the pass
  is the guest-RAM pin total, not the published-row count.
- ⚠ **"Eleven single-ablations plus a baseline is one sitting."** — it was, in wall time (16 boots in
  ~50 minutes, ~2.5 min each), but **eleven were not eleven**: `FB_JOIN` ablates three arms at once
  and `ISOLATES` cannot be ablated in this harness at all. Nine of the eleven are cleanly ablatable.

## §7 — HARNESS DEFECTS THIS RUNG HIT

1. ★★★★★ **The eleven were UNCONDITIONAL `export`s — an ablation was NOT EXPRESSIBLE.** Setting
   `KAYFABE_PT_SWEEP=off` in the environment got `on` anyway, and the log then faithfully recorded
   `on`. The w297 fix made the report *able to see* the arms; it did not make them *settable*. Fixed
   in this branch (`${V:-default}`, every default the w297 value byte for byte). **This is the same
   defect class one layer down**: w297 fixed "the report cannot see the arms", w298 fixed "the caller
   cannot set them" — and the second was invisible *because* the first was fixed, since the report
   would have dutifully printed the value that overrode you.
2. ★★★ **`KAYFABE_PT_WITNESS_EXEC` was armed but not enumerated** by the "EVERY RELAXATION THAT WAS
   ON" block, so that heading was false by one even after the w297 fix. Now 12 rows, asserted
   non-empty per boot (`relaxation report rows = 12`) rather than merely printed.
3. ⊘⊘ **The summary row's `host_rows` field sorted LEXICOGRAPHICALLY.** `sort -u` on
   `host_rows=<n> of <m>` puts `4 of 13348` after `18295 of 18309`, so the field printed the
   lexicographic last under a column labelled as the peak — and it was wrong in the direction that
   supported a conclusion (§3's "the table is empty"). ★ Fix: sort numerically on the extracted
   integer, or print the whole distinct set rather than one representative. ⚠ Note what did **not**
   catch it: the value was present, well-formed, plausible, and *cited* — every signal said it was
   measured. Only re-deriving it with a different comparator showed it was not the number its label
   claimed.
4. ⊘ **The 30-minute artefact-staleness guard ate three arms.** No Rust changes between arms ⇒ cargo
   did nothing (`Finished in 0.08s`) ⇒ after ~33 min of boots `build_qom_shim.sh` correctly refused:
   *"is more than 30 minutes old — cargo did not rebuild it and this script will not install an
   archive it did not just produce."* Arms 10–12 exited **`rc=92` at the build, never booted**. The
   guard is right; the harness was wrong to assume one build serves twelve boots. Re-run with a
   `touch` of the crate root per arm (`w298_seq2.sh`). ⚠ **Those three rows were `⊘ABSENT` for every
   field and `relaxation report rows = 0`** — the ledger said "unmeasured" in six places at once,
   which is the only reason this was not read as three quiet failures.

## §8 — files

`w298_summary.txt` — the row-per-boot ledger (start marker + terminator per arm).
`w298<arm>.log` — the full w290p/w297 grading output for that arm.
`run_w298<arm>_probe.log` — cup3's own stage ladder and `^CUP3_VAL=`.
`run_w298<arm>_hostdmesg.log` — per-boot host dmesg **delta**; ⊘ 0 bytes is the normal green.
`run_w298<arm>_qemu.log.gz` — full device log, kept for `base`, `ptsweep`, `vaspub`, `bothoff`,
`ptwitness` only (~4.8 MB each raw).
`w298_seq.sh` / `w298_seq2.sh` — the exact sequences that produced the above.
