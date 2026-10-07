# w270 — ★★★★★ **THE GUEST'S OWN OPERAND AND HARDWARE'S FAULT ADDRESS AGREE TO THE BYTE.** The pin landed, `0x2_04420000` is gone, **the release wrote `2`** — and the wall moved 32 KiB, to `0x2_04428000`, because `ALREADY PINNED` is keyed on the VA and not on the extent

> ### STATUS — 2026-08-12 / **LIVE — MEASURED.** Two arms from **one** binary, source revision
> **`1b64729`**, stamp-gated before booting (`STAMP: [kayfabe-rev:1b647295…] WANT:
> [kayfabe-rev:1b647295…]` → `PASS`), content-checked on **24** strings including this rung's
> own seven, **plus a negative content check** (§0.1). **Ten** arming assertions per arm read
> out of that arm's own log — `GUEST-OPERAND`, `GR-ROUTE`, `GUEST-SEMA`, `GUEST-PUSHBUF`,
> `GUEST-RING`, `EXEC-WITNESS` — **all PASS on both**. Bench `vh`, `NVIDIA GeForce RTX 3060`
> (GA106), host driver **580.159.04**. `BUILD_RC=0`, both `BOOT … RC=0`, `W270 EXIT rc=0`,
> `ENOSPC/LLVM = 0` from the same invocations, 105 GB free. Graded against
> `docs/design/w270_the_operand_pin_prereg.md`, committed at **`905f289`, before any of this
> rung's code existed**. Branch `w270-the-operand-pin`.

---

## 0. ⊘⊘ LEAD WITH WHAT CONTRADICTS THE BRIEF — and two of the three cost no GPU time

### 0.1 ★★★★★ **MY OWN cap2b GUARD REFUSED THE FIRST BOOT — ON PROSE, AND IT WAS RIGHT**

`w270_run.sh` carries a **negative** content check: the faulting address must appear **zero**
times in the built binary. `[measured, first launch]` it fired and **refused to boot**:

```
★★★ cap2b GUARD: literal 0x2_04420000 in the binary = 2 (MUST be 0)
=== W270 EXIT rc=94 ===          ← 24 s after the build, no boot attempted
```

Both occurrences were **pure prose** — the `GUEST-OPERAND arm=` line and `guest_operand_from`'s
refusal message — and **neither was read by any decision**. The pin decodes; it does not
recall. So the mechanism was already correct.

★★★ **The guard is right to refuse anyway, and that is the finding.** A `strings` check cannot
tell a literal in a *sentence* from a literal in a *decision*. So the only property it can
certify is the stronger and simpler one — **the address is not available to any decision,
because it is not in the binary at all.** ⇒ Prose must not spend it.
⚠ ⊘ The tempting repair is the wrong one: a guard that **allowlisted** the two prose sites
would have passed and certified **nothing** — the next sentence to name the address would have
been allowlisted too, and a *decision* hiding behind one is exactly what the check exists to
prevent. Both sentences now describe the fault without the number; the knowledge lives in doc
comments, which never reach `.rodata`. Second launch: **`= 0`**.

### 0.2 ★★★★★ **`grep CUP2_RC=` MATCHES `GCC_CUP2_RC=0`, AND IT REPORTS SUCCESS ON A RUN THAT
MEASURED NOTHING**

`[measured, w270_off, caught before it reached this document]` I asked the **control** arm for
`CUP2_RC` with `grep -oE 'CUP2_RC=[0-9]+'` and it answered **`CUP2_RC=0`** — the campaign's
headline success value, the thing eight rungs have failed to produce. The only match in the
file was **`GCC_CUP2_RC=0`**: the guest **compiler's** exit status, which is `0` on every
healthy boot. `cup2`'s own line had not been written yet.

⇒ ★★★ **On a boot where the measurement did not happen, the naive grader reports SUCCESS.**
That is the zero-byte-artefact class pointed in the most consequential direction this campaign
has — and it would have been *corroborated* by appearing on the **control**, where a `0` is
least expected and therefore most exciting.
⚠ `tail -1` does **not** save it: it is right only while `cup2`'s real line happens to come
last. On any run where the hook aborts early, the `gcc` line is the **only** match.
⇒ Anchored to `(^|[^A-Z_])CUP2_RC=`, with an absent line rendered as *"⊘ NO cup2 EXIT LINE —
THE MEASUREMENT DID NOT HAPPEN. ⊘ This is NOT 0"*. The anchored answer on that same arm was
**`124`**.

### 0.3 ⊘⊘ **AND I NEARLY RECORDED A MISSING FILE AS "ZERO FAULTS"**

Mid-run I read the `pin` arm's `hostdmesg` log and got `bytes=` and `distinct addrs = []`, and
began writing *"the `Xid` is gone"*. ⊘ **The file did not exist** — the arm had not reached
teardown. `stat` on a missing file prints empty and `grep` on one matches nothing, which is
**byte-identical to a clean capture**. The control's own `HOST_DMESG_XID=1` / watermark
`984 → 985` is what a valid reading looks like. ⇒ The real `pin` artefact, once written, was
**228 bytes and contained an `Xid`** — the opposite of what I had nearly recorded. ★ Third
instance in one rung of *an absent artefact reading as a favourable measurement*.

### 0.4 ⊘ **THE BRIEF'S "CORRELATION ONLY" WAS ALREADY PROVEN ON DISK, AND THIS RUN CLOSES IT**

Pre-registered at §0.1: `run_w268_pass_qemu.log:980` already joined the faulting submission to
the polled slot by identity. This boot closes the remaining half **by construction** — the
same submission is now shown naming **both** the release target `0x20440ff70` *and* the
operand `0x204420000`, in two rows from two independent reads of the same doorbell.

---

## 1. ★★★★★ THE HEADLINE — the decode and the hardware agree, exactly

`[measured, w270_pin, `run_w270_pin_qemu.log`]`, chan 8's three CE doorbells, verbatim:

```
OPERAND-SOURCE-CE … chan=8 → methods=3  launches=1 opaque=1  release_only=1 physical=0 operand(s)=0 (0 write, 0 read) ⇒ 0 page(s)
OPERAND-SOURCE-CE … chan=8 → methods=11 launches=3 opaque=7  release_only=2 physical=0 operand(s)=1 (1 write, 0 read) [W@0x204420000+0x8000]  ⇒ 8 page(s)
OPERAND-SOURCE-CE … chan=8 → methods=19 launches=5 opaque=13 release_only=3 physical=0 operand(s)=2 (2 write, 0 read) [W@0x204420000+0x8000 W@0x204428000+0x8000] ⇒ 16 page(s)
```

⇒ ★★★★★ **The `methods=11 launches=3` submission — the one `w268` §3.2 identified as the
faulting one — declares, in the guest's own `OFFSET_OUT_UPPER`/`_LOWER`, a 32 KiB write at
`0x2_04420000`.** That is the address hardware faults on, **to the byte**, decoded by the
chip's own codec from bytes read at that doorbell, out of a binary in which the address does
not appear (§0.1).

★★ **A2 and A12 together are the strongest single result here**: A12 was the deliberately
widened arm for *"our codec and hardware disagree about the guest's own operand"* (`p = 0.12`).
It did **not** fire. ⇒ This is the first end-to-end validation the decode path has ever had
against an independent authority — the host GPU's own MMU.

⊘ **And it is `1 write, 0 read`.** `release_only = 2` of 3 launches; the third is a copy with
**no source operand at all** (`physical = 0`, so the source was not merely physical — the work
kind has none). ⇒ It is a **fill/scrub of 32 KiB of context buffer**, which is exactly what
*"CE work inside `cuCtxCreate`"* should look like.
★ **This discharges the cost I pre-registered at §1.2**: I warned that pinning both directions
would make attribution impossible if it went green. There is **no read operand**, so the rung
was a clean destination-only experiment after all.

### 1.1 ★★★★★ THE PIN LANDED, AND THE RELEASE THE FAULT WAS BLOCKING WAS WRITTEN

```
OPERAND-TABLE: 8 page(s) asked, 8 resolved in guest RAM, 0 MISS, 0 NOT-IN-GUEST-RAM
operand run 1/1 va=0x204420000 gpa=0x3fc1a000 len=32768 → PINNED memory=0xcafe0055
                 host_va=0x204420000 placed_as_asked=true          (1 fresh, 32768 bytes)
```

| reading | `w270_off` (control) | `w270_pin` | |
|---|---|---|---|
| `Xid` address | **`0x2_04420000`** | **`0x2_04428000`** | ★★★ **MOVED** |
| polled slot `0x20440ff70` | **`1`** | **`2`** | ★★★★★ **THE RELEASE WAS WRITTEN** |
| the wait wants | `2` (`limit=2 cached=1`) | **`3`** (`limit=3 cached=2`) | ★★★ **ADVANCED** |
| chan-8 CE doorbells | 2 | **3** | ★★ a submission that never existed before |
| `DOORBELL-XLATE` / `-STORE … WROTE` | 17 / 17 | **18 / 18** | |
| `COMPLETION-WATCH … OBSERVED` | 26 | **27** | |
| `OPERAND-PIN` lines | **0** | 20 | the control's expected absence, as a number |
| `CUP2_RC` (anchored, §0.2) | **124** | **124** | ⊘ |

⇒ ★★★★★ **`cuCtxCreate`'s wait was satisfied and re-armed.** The slot the guest polls went
`1 → 2`, and the guest consumed it and now wants `3`. Every counter on the `pin` arm is
strictly greater. **This is the second measured advance in this campaign, and unlike `w269`'s
it was produced by code written for the purpose rather than by arming an existing flag.**

---

## 2. ★★★★★ WHY IT IS STILL 124 — and the answer is a named, proven, one-line defect

**`grep -c Xid` reads `1` on BOTH arms.** A count sees nothing here. The identity does:

```
off: ENGINE CE2 HUBCLIENT_CE0 faulted @ 0x2_04420000  FAULT_PTE ACCESS_TYPE_VIRT_WRITE
pin: ENGINE CE2 HUBCLIENT_CE0 faulted @ 0x2_04428000  FAULT_PTE ACCESS_TYPE_VIRT_WRITE
                                          ^^^^^^^^^^  exactly +0x8000
```

★★★ **`0x2_04428000` is the SECOND extent of the THIRD submission** — `W@0x204428000+0x8000`
in the `methods=19 launches=5` row above. And the log contains its own cause:

```
operand run 1/1 va=0x204420000 gpa=0x3fc1a000 len=32768 → PINNED         memory=0xcafe0055
operand run 1/1 va=0x204420000 gpa=0x3fc1a000 len=65536 → ALREADY PINNED memory=0xcafe0055
```

The two extents are **contiguous**, so `pushbuffer_runs` correctly coalesced them into **one
64 KiB run at the same base VA** — and `SharedDevice::pin_guest_ram` answered **`already`**,
returning the **same `memory=0xcafe0055` handle** it created for **32 KiB**.

⇒ ★★★★★ **THE IDEMPOTENCE KEY IS THE VA; THE EXTENT IS NOT PART OF IT.** Bytes
`0x2_04428000 … 0x2_0442ffff` were **never described to RM**, and the row that should have said
so said **`ALREADY PINNED (idempotent replay)`** — a green verdict, with `placed_as_asked=true`
beside it. The host GPU's fault address is the proof, and it is the only reason this is not
still invisible.

⚠ **This is the shape this tree has paid for before, in a new place**: a green supply row
holding a wall in place, and *the same word meaning a different predicate* — `already` is true
of the **address** and false of the **range**. ⊘ It is **not** a defect in this rung's source:
the decode named both extents correctly, the table resolved all 16 pages, and the coalescer
was right to merge them. The defect is one layer down, in the primitive all four sources share
— so it has been latent since `w265` and could only surface once a source produced a *growing*
run at a repeated base.

⇒ **The next rung is one predicate**, and it is named: make the pin's idempotence key the
`(va, len)` extent — or have it **extend** an existing shorter placement — and refuse, by name,
rather than replay, when a longer run meets a shorter mapping. ⊘ Do **not** simply widen the
first pin: the extent is the guest's number and the fix must be about the key, not the size.

---

## 3. GRADED AGAINST THE PRE-REGISTRATION (`905f289`)

| # | prediction | p | outcome |
|---|---|---|---|
| **A1** | `OPERAND-SOURCE-CE` on chan 8's `methods=11 launches=3`, ≥1 virtual write extent | .72 | ★★★ **FIRED** — exactly one |
| **A2** | that extent covers **`0x2_04420000`** | .62 | ★★★★★ **FIRED, to the byte** (`W@0x204420000+0x8000`) |
| **A3** | resolves in guest RAM, `0 MISS`, `0 NOT-IN-GUEST-RAM` | .80 | ★★★ **FIRED** — 8/8, then 16/16 |
| **A4** | ≥1 run `PINNED` on `pin`; **0** `OPERAND-PIN` lines on `off` | .78 | ★★★ **FIRED** — 32 768 fresh bytes; `0` vs `20` |
| **A5** | `Xid … @ 0x2_04420000` **GONE** on `pin`, by identity | .55 | ★★★ **FIRED** |
| **A6** | a **NEW** `Xid` at a different address | .35 | ★★★ **FIRED** — `0x2_04428000` |
| **A7** | the slot reads **`2`**, or the wait changes | .30 | ★★★★★ **FIRED, both halves** — `2`, and the wait now wants `3` |
| **A8** | ★ **`CUP2_RC ≠ 124`** | **.25** | ⊘ **FAILED** — `124` on both arms |
| **A9** | `off` replicates `w269b_pass` | .85 | ★★★ **FIRED** — `N=1`, `0x20440ff70`, value `1`, `Xid @ 0x2_04420000`, `RC=124` |
| **A10** | ★ **the pin lands, the fault clears, `CUP2_RC` still 124, wall at a NEW named address** | **.40** | ★★★★★ **FIRED — my registered central estimate, and it is what happened** |
| **A11** | ⊘ widened: the decode names **zero** write extents | .20 | ⊘ did not fire |
| **A12** | ⊘ widened: the decode names extents **NOT** including `0x2_04420000` | .12 | ⊘ did not fire — §1's headline |
| **A13** | carried guards unchanged | .85 | ★★ **FIRED** — `PT-DECODE bound = 19618` **identical on both arms**; `SEMA-PAGE-SLOT`, `pb run … PINNED`, `COMPLETION-WATCH` all present and **greater** on `pin` (the guest did more work) |
| **A14** | reader assertions still FAIL (unpaid a fourth time) | .85 | ⊘ **not gradeable** — this harness does not carry those two rows; the debt is **untouched**, not discharged |

★ **A8 failed and A10 fired, and A10 is the one I called my central estimate in the
pre-registration.** ⊘ Nine consecutive rungs have now predicted `CUP2_RC = 124` and been right.
★★ But the calibration note that matters is different this time: **A10 was registered at `.40`
as a first-class prediction precisely so a `124` could not be reported as *"expected"* without
also reporting that the fault was expected to clear.** Both halves came in, which is the first
time this campaign's modal prediction has been the *informative* one rather than the null.

★★ **A13 deserves its own line**: `bound = 19618` on **both** arms means the address table's
population is identical, so the `Xid`'s move is attributable to **this arm** and not to a
binding change — the attribution `w265` could not make about its own result.

---

## 4. ⊘ WHAT THIS RUN CANNOT PROVE

1. ⊘ **That fixing the idempotence key will retire the third submission.** §2 is a complete
   causal story with the fault address as its evidence, and it is still a story: nothing here
   places the missing 32 KiB and re-measures. ⚠ A fourth extent may follow the third.
2. ⊘ **That the release which wrote `2` is the one the faulting submission owed.** The slot
   advanced and the faulting address cleared on the same arm; the *payload* of a release is
   still not printed (carried unpaid from the pre-registration §4.2).
3. ⊘ **Which of the wait's needs is now unmet beyond release `3`.** `N items = 1` throughout,
   so there is one item — but `w269` §5.1's limit stands: one unsatisfied item explains the
   spin without being the only one.
4. ⊘ **Correctness of any byte the fill wrote.** There is no oracle on this path, and a
   `CeWork::Fill` that wrote the wrong pattern would look identical here.
5. ⊘ **That `refuse` is safe to default.** Two boots. The route stays `refuse`.
6. ⊘ **Anything about the error/event plane** — `RcTriggered` and `w212`'s F6 masked leaf are
   untouched. ★ They are **no longer the leading candidate**, because this rung's wall has a
   measured address and a named mechanism.
7. ⊘ **The full orphan-gate mutation scan is unpaid** — two attempts were killed by my own
   launcher (`143`, which per `CLAUDE.md` is the job and not the work). Reachability was
   checked directly instead: every new verb has a production caller in the chain
   `doorbell → pin_operand_guest_ram → ce_operand_pages → observe_ce_operand_targets →
   operand_targets_of`. ⚠ That answers *visibility*, which is the gate's own stated question,
   but it is not the gate.

---

## 5. THE NEXT RUNG

1. ★★★★★ **THE IDEMPOTENCE KEY** (§2). One predicate, one primitive, and the fault address is
   the falsifier: if `0x2_04428000` clears and the wall moves again, the mechanism is confirmed
   and the loop is *"each submission names one more extent"* — which would make the **fourth**
   observation the one that decides whether this converges or recedes. ⚠ Pre-register that
   explicitly: three data points in one direction is where this campaign has been most wrong.
2. ★★★ **Print the release PAYLOAD** beside `release_target(s)`. It is the one number that
   would turn §4.2's inference into a measurement, and `CeCompletion::payload` is already
   decoded and thrown away at the print — the same defect `w267` found in `push_headers`, one
   plane over.
3. ★★ **Pay the reader assertions** (`GR-CURSOR-READER stopped`, `PAGE-READER ASSERTION`),
   owed since `w267` §3.2 and not carried by this harness at all.
4. ⊘ **Do NOT default the route**, and do not build `RcTriggered` pre-emptively (§4.6).
