# w264 — THE PIN FIRED, AND THE TWO RESOLVERS DISAGREE ABOUT **EXISTENCE**

> ### STATUS — 2026-08-12 / **LIVE — MEASURED.** Four arms from **one** binary, source
> revision **`a4c46bb`**, **stamp-gated against the binary before booting**
> (`STAMP: [kayfabe-rev:a4c46bb1…] WANT: [kayfabe-rev:a4c46bb1…]` → `PASS`), plus a
> **content check** that asserts (not prints) the changed strings are in the archive.
> Bench `vh`, `NVIDIA GeForce RTX 3060` (GA106), host driver **580.159.04**.
> Graded against `docs/design/w264_pushbuffer_pin_prereg.md`, committed at `064b930`,
> **before the code existed**. `BUILD_RC=0`, all four `BOOT … RC=0`, `EXIT rc=0`,
> `ENOSPC/LLVM = 0` from the same invocations.

---

## 0. ★★★★★ THE HEADLINE — the rung fired, and it located the wall one layer deeper

**The pin ran on the addresses hardware faults on, and the address table does not have them.**

```
kayfabe: PB-PIN … proc=2 chan=12 pdb=0x201000 ring=0x200224000 entries=1024
  → SCAN 1024 of 1024 entries: 1 extent(s), unread=0 zero=1023 control_entries=0
  TABLE: 1 page(s) asked, 0 resolved in guest RAM,
         1 MISS [va=0x202c00000:Address(Miss { pdb: Pdb(2101248), va: GpuVa(8636071936) })],
         0 NOT-IN-GUEST-RAM
```

★★★ **THREE INDEPENDENT ADDRESS SETS, AND THEY ARE THE SAME EIGHT ADDRESSES:**

| the guest's own `gp[0]` entries | what the **pin** asked the table | where the **host engine** faulted |
|---|---|---|
| `0x202400000 … 0x203200000` | `0x202400000 … 0x203200000` | `0x2_02400000 … 0x2_03200000` |

⇒ **The pin asked about exactly the right addresses.** The table answered `Miss` on all eight,
while the **descent on the same log line** resolves each one to guest RAM
(`gp[0]@0x200224000=0x202c00000+0x20 pb=**S**:0x41d39000`).

★★★★★ **The two resolvers disagree about EXISTENCE, not about aperture** — `NOT-IN-GUEST-RAM =
0` on every row. This is `two_projections_of_one_fact_disagreeing`, and the pre-registration
named this exact cell and its reading **before the boot**:

> *"`→ UNRESOLVED … Miss` ⇒ ⊘ **the table does not bind the pushbuffer VA.** NOT a refutation
> of the mechanism — a statement that the *populate* side has a gap the descent does not. The
> next rung is then the populate pass, **and it has an address list**."*

⇒ **Leg 4 is not "join the pages" and it is not "pin the pages". It is: the address table's
POPULATE side does not learn the pushbuffer leaves.** The pin is built, correct, and asking
the right questions of an authority that has not been told.

---

## 1. ⊘⊘ WHAT THIS RUN REFUTED IN ITS OWN BRIEF — confirmed a third time, on fresh hardware

The rung brief said the eight `Xid` addresses are *"Vidmem … and `join_fb_leaf` already
reaches them"*. `[measured, w264, all four arms]`:

- Every `gp[0]` pushbuffer VA resolves **`pb=S:`** — **`S` = guest RAM**
  (`CeResolve::tag`'s own doc). Eight of eight.
- `FwdFault::PushbufferAperture { va: GpuVa(8592179200), aperture: Vidmem }` —
  `8592179200 = 0x200224000`, which is the **ring's** VA. The `Vidmem` in the brief is that
  fault's, and that fault is about the ring.
- The **pin** reports `0 NOT-IN-GUEST-RAM` on all eight rows: the table, when it is asked, does
  not call these addresses vidmem either. Nothing anywhere in this boot calls a pushbuffer page
  Vidmem.

⚠ **And the GPAs moved again.** Same eight VAs; `w263 off` → `0x2bc63000 …`, `w263 ring` →
`0x3d45f000 …`, `w264 pin` → `0x41539000 …`. Three boots, three sets of physical pages.
⊘ Any rung that hard-coded one of them would have read correctly on exactly one boot.

---

## 2. THE PRE-REGISTERED SCORECARD

★ Arms differ from the one above in **exactly one** variable, and the arming is read back out
of **each boot's own log** (`FB-JOIN arm=` / `GUEST-RING arm=` / `GUEST-PUSHBUF arm=`), not out
of the shell that exported it.

| arm | `FB_JOIN` | `GUEST_RING` | `GUEST_PUSHBUF` |
|---|---|---|---|
| `base` | off | off | off |
| `join` | shared | off | off |
| `ring` | shared | ring | off |
| `pin` | shared | ring | **pin** |

| # | observable | pred `pin` | **base** | **join** | **ring** | **pin** | |
|---|---|---|---|---|---|---|---|
| Q1 | `PB-PIN` lines | ≥ 1 | 0 | 0 | 0 | **8** | ✔ ★ the rung ran |
| Q2 | `PB-PIN NOT-IN-GUEST-RAM` | 0 | 0 | 0 | 0 | **0** | ✔ §0 |
| Q3 | `PB-PIN` runs `PINNED` | ≥ 1 | 0 | 0 | 0 | **0** | ⊘⊘ **REFUTED** — §3.1 |
| Q4 | `placed_as_asked=true` on pinned | all | 0 | 3 | 4 | **4** | n/a — no pb run reached a pin |
| Q5 | `PB-PIN … MISS` | 0 | 0 | 0 | 0 | **8** | ⊘⊘ **REFUTED** — §0, **the finding** |
| Q6 | `PB-PIN ⚠⚠ CAPPED` | 0 | 0 | 0 | 0 | **0** | ✔ (⊘ and §4 — a green here is *not exercised*) |
| Q7 | host `Xid` | **0** | 0 | 0 | **8** | **8** | ⊘ **REFUTED** — §3.2 |
| Q8 | `fbuserd GET=` non-zero | ≥ 1 | 0 | 0 | **1** | **1** | ✔ reproduces `w263` |
| Q9 | ring-pin verdict | 8 × `NOT IN GUEST RAM` | 8 × `UNRESOLVED` | 8 × `UNRESOLVED` | 8 × `NOT IN GUEST RAM` | **8 × `NOT IN GUEST RAM`** | ✔ |
| Q10 | `adopt=GUEST-RING` | 16 | 0 | 0 | 16 | **16** | ✔ |
| Q11 | `userd=GUEST-USERD` | 16 | 0 | 0 | 16 | **16** | ✔ |
| Q12 | `RmInitAdapter failed` | 0 | 0 | 0 | 0 | **0** | ✔ guest alive on every arm |
| Q13 | **`CUP2_RC`** | **124** | 124 | 124 | 124 | **124** | ✔ ★ movement predicted **0**, movement **0** |
| Q14 | `CE-SUBMIT` / `RETIRED` | 0 | 0 | 0 | 0 | **0** | ✔ leg 5 unbuilt |
| Q15 | `ENGINE-OBJECT seen/fwd/ref` | 34/32/2 | 34/32/2 | 34/32/2 | 34/32/2 | **34/32/2** | ✔ guard |
| Q16 | `BAR1 GP_PUT` advances | ~equal | 26 | 26 | 26 | **26** | ✔ guard, identical on all four |

`GR-BIRTH` **24** and guest `NVRM` **31** on all four arms. `totalMem=11959 MiB compute=8.6`
on all four.

---

## 3. ★★★ THE THREE ROWS THAT CARRY THE RUN

### 3.1 ⊘⊘ Q3 REFUTED — nothing pinned, and the reason is upstream of the pin

Predicted `≥ 1` at **0.6** — the lowest confidence on the page, and §3.1 of the
pre-registration named *"which resolver"* as *"the single point of failure"*. It was.

⊘ **Nothing about the pin chain was exercised past its first step**: no run was formed, the
hypervisor's layout was never asked, `pin_guest_ram` was never called. So `w264` says **nothing
whatever** about `resolve_guest_ram`, `GuestRamGrant`, `OS_DESCRIPTOR`, placement identity or
`SystemDataPlane` on this path. ⚠ A reader must not take `Q6c SystemDataPlane = 0` as evidence
the §12.26 wall would not fire — **it was never reached**. That is `a_diagnostic_gated_on_the
_failure` in the other direction: an absence produced by not getting there.

### 3.2 ⊘ Q7 REFUTED — 8 `Xid` on `pin`, the **same eight addresses**, unchanged from `ring`

Predicted 0 at **0.6**, on the hypothesis that a pin would install the missing PDE. Nothing was
pinned, so the prediction's premise never held and the row is **consistent, not surprising**.

★ Its value is as a **control**: `ring` → `pin` changed one variable and the `Xid` count,
addresses and engines are **identical**. ⇒ The pushbuffer pass is observationally inert on
everything except its own lines — which is what makes the `ring`→`pin` comparison clean (§3.3).

⊘ It remains **unmeasured** whether a *successful* pin would silence the fault: that needs the
host channel's VAS to be the one the pin writes into, and nothing here measures which VAS the
host channel is bound to.

### 3.3 ★★★★★ THE LADDER RESOLVES `w263`'s OWN QUALIFICATION — and that is a second result

`w263` §3.1 reported `FwdFault::PushbufferAperture` moving `0 → 8` between its arms, invoked
its own rule (*"if a fourth number moves the comparison is qualified"*), and could not say what
caused it — because its `off` arm had `KAYFABE_FB_JOIN=shared` and was **not** `w262`'s control.

The four-arm ladder answers it by construction:

| step | one variable changed | what moved |
|---|---|---|
| `base` → `join` | `FB_JOIN off→shared` | **only** `placed_as_asked` 0→3. Doorbells, refusals, `Xid`, `GET`, ring-pin verdict: **identical** |
| `join` → `ring` | `GUEST_RING off→ring` | doorbells served **183→175**, REFUSED **8→16**, `PushbufferAperture` **0→9**, `Xid` **0→8**, `GET` **0→1**, ring-pin `UNRESOLVED→NOT IN GUEST RAM` |
| `ring` → `pin` | `GUEST_PUSHBUF off→pin` | **`PB-PIN` lines 0→8. Nothing else.** |

⇒ ★★ **`PushbufferAperture` belongs to legs A2+B, not to the pushbuffer plane** — `w263`'s
qualification is discharged, at one revision, inside one campaign, with no cross-boot
comparison. ⊘ And `base`→`join` shows the FB join **alone** moves no doorbell-level number,
which was assumed and never measured.

★ **The `ring`→`pin` comparison is clean**: exactly one number moved. That is the discipline
`w263` asked for and could not perform on itself.

---

## 4. ⊘ WHERE THIS RUN IS WEAKER THAN IT LOOKS

- ⊘ **Q6 green is not evidence the cap is right.** `pages.len() == 1` on every doorbell (one
  non-zero GPFIFO entry per ring, `zero=1023`), so `PUSHBUF_MAX_EXTENTS`/`MAX_PAGES` were never
  approached. The cap's correctness rests on `pushbuffer_pin_tests`, not on this boot.
- ⚠⚠ **The binary predates a defect I found in my own code and fixed at `39de9b9`**: the
  report used a truncated **sample**'s length as a **count**. ⊘ `w264`'s numbers are unaffected
  and the reason is exact rather than hopeful — with one page per doorbell, sample length and
  count are equal by arithmetic. **Anything inherited from this boot at a different workload
  must be re-checked.** (Same shape as `w263` inheriting `75e8715`.)
- ⊘ **The `stride` row says `n/a (fewer than two extents)` on every line.** The instrument that
  would have caught an assumed stride was **not exercised** — one extent per ring cannot
  disagree with a stride. It is a print, never a read, so nothing depends on it; but it
  measured nothing here.
- ⊘ **Q4 `placed_as_asked=true` counts (0/3/4/4) are from OTHER pin sites**, not from the
  pushbuffer pass, which never reached a pin. The row is reported because it moved, not because
  it means anything about this rung.
- ⊘ **`ENGINE-OBJECT` and `BAR1 GP_PUT` are identical on all four arms** — a guard that held,
  and therefore evidence of nothing except that the guard held.

---

## 5. ⊘ WHAT THIS RUN CANNOT PROVE

- ⊘ **That a pin at these VAs would silence the `Xid`.** Nothing was pinned. And even a
  successful pin is measured in *our* table for *this* `pdb`; which VAS the **host channel** is
  bound to is `[NOT MEASURED]` by any rung so far.
- ⊘ **Whether the fetch is leg B's doing or leg A2's** — the brief's ITEM 2. `ring` and `pin`
  both carry B; `join` carries neither. **The ladder cannot separate them and was not built
  to.** §5 of the pre-registration costs the experiment and says why it is its own rung: leg
  B's arming is *inherited by construction* (`adopted_guest_userd` is reachable only from
  inside `adopted_guest_ring`'s `Some`), so *"A2 armed, B off"* needs a boolean on
  `plan_engine_object`'s public signature. ⚠ The brief called it *cheap* on the predecessor's
  word; it is cheap **only if a flag exists**, and none does.
- ⊘ **Anything about completions.** `CE-SUBMIT → RETIRED` = 0, as in ~127 prior logs.
- ⊘ **That the table's miss is a populate gap rather than a lifetime one.** *"Never learned"*
  and *"learned and pruned before we asked"* are not separated by a `Miss`. The next rung must
  distinguish them, and `PT-DECODE`'s `bound=6275 unwitnessed=6275` is where to start.
- ⊘ **That reading `gp[i]` at doorbell time reads what hardware later fetches.** This pass sees
  the extents present at that instant.

---

## 6. THE NEXT RUNG, AND IT HAS AN ADDRESS LIST

The pin is built and asking the right eight questions. What is missing is on the **populate**
side of the one authoritative table:

```
pdb 0x201000, VAs 0x202400000 0x202600000 0x202800000 0x202a00000
                 0x202c00000 0x202e00000 0x203000000 0x203200000
```

— which the **descent resolves** (`S:0x41539000 …`) and the **table misses**, in the same
process, on the same doorbell, on the same log line. ★ Two resolvers, one address, one of them
right. ⊘ *Which* one is right is not in question here — the descent walks the guest's real page
tables and hardware faulted at exactly the addresses it names — so the question is **why the
populate pass never learned the leaf the descent walks**, and `mode2_address_table_of_truth`'s
*miss = fault* rule means it cannot be papered over at the consumer.
