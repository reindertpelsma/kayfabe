# The divergence ledger — where `kayfabe` differs from the C, why, and what it cost

**Status:** read-only audit, 2026-08-11, in answer to the owner's *"I am worried the rewrite gets too
much drift."* Source + committed evidence only. No bench, no build, no boot.
**Verdict: CONVERGING** — and the counter-metric is worse than the owner said.

---

## 0. ⊘ TWO OF THE FIVE SEED ROWS DID NOT SURVIVE — and both were mine

**0.1 ⊘⊘ "Per-process isolates" is NOT a divergence. It is the C's design, inherited.**
`C: src/qemu/nvkvm_isolate.h:4` — *"One isolate per guest userspace mm."* `nvkvm_isolate.c` is 2 031
lines and spawns real `clone(CLONE_NEWUSER|CLONE_NEWPID)` children. ⇒ **Row deleted.** The true fact
underneath is smaller: the C's *Mode-2 bench* runs one CUDA process per QEMU lifetime because the
second fails `cuInit → 999` — **a teardown defect in the C**, i.e. a limit on our *oracle*, not a
divergence in our *design*.

**0.2 ⊘⊘ "The C is default-accept" is HALF WRONG — and I told the owner the unqualified version.**
The C ships **four default-deny allowlists** (`nvkvm_ctrl_allowlist.h:20-22` — *"Anything not matched
here … is DENIED, matching nvproxy's posture"*; `nvkvm_fe_alloc_allowlist.h:12`; UVM; DRM/NVKMS),
landed at `d8bd78b`. ★ It is default-**accept only where it FABRICATES a reply as a fake GPU**
(`nvkvm_gpu_emul.c:3435`), never where it **forwards to the real driver**. ⇒ Row kept, **re-scoped**.
⚠ And the C **tried the blanket fix and reverted it** (`:3436-3446`, *"regresses `cuInit`"*) — evidence
*for* the C, not against.

---

## 1. THE LEDGER — 8 divergences survive

| # | divergence | why | cost | reversible? |
|---|---|---|---|---|
| **D1** | refuse by name on the fabricated-reply path | *"collapsing them is how the C ended up answering everything `NV_OK`"* | a category of walls the C never had; 8 of 191 doorbells refused by name | **REVERSIBLE** — one `match` arm |
| **D2** | ★ framebuffer leaf = host **system** memory, not card memory | card memory **cannot** carry a guest-reachable CPU view — measured `0x56`, *"CPU mapping not supported for addressSpace: 0x1"* | ⚠ **UNPRICED** — see §3 | **LOAD-BEARING**, forced by D4 |
| **D3** | a separate GPU address space for our own machinery | ★★ hazard **measured exploitable**: a copy engine in the guest's space read our own semaphore payload back; after the fix, `Xid 31 FAULT_PDE` at exactly that address | every publish placed twice; the fix had its own defect | reversible mechanically, **load-bearing in evidence** |
| **D4** | ★★★ **the device fd never crosses to the VMM** | the isolate's founding rationale | **this fork generates D2 and half of D8** | **LOAD-BEARING — this is the architecture** |
| **D5** | `#![forbid(unsafe_code)]`, 22 of 23 crates | repo rule 1 | 13 compile-fail rows pin the seam | load-bearing (language choice) |
| **D6** | no GPU generation named in any logic crate | a version floor is legitimate on the VMM axis and nowhere else | a 4 451-line mock arch exists to make it testable | load-bearing |
| **D7** | ★ the verification machinery | *"every rule is a generalisation of a specific incident, cited"* | **§2** | **REVERSIBLE** |
| **D8** | guest RAM is *instructed*, never self-mapped | consequence of D4 | a whole protocol + export registry | load-bearing (D4's shape) |

**8 divergences · 5 load-bearing · 4 of the 8 are ONE decision (D4) seen four ways.**
★★ **Divergences with a measured, guest-visible cost: ZERO.**

---

## 2. ★★ D7 PRICED HARDEST — the ratio, not the anecdote

| | `kayfabe` | the C |
|---|---|---|
| implementation | **104 265** lines | **44 783** |
| doc comments inside `src` | **64 887** (38 % of every `src` line) | — |
| all verification | ~**164 000** | 12 917 |
| **verification : implementation** | ★ **≈ 0.97 : 1** | **≈ 0.29 : 1** |
| tests / gates / oracles | 2 679 `#[test]`, 12 CI gates, 23 oracles, 13 compile-fail rows | 0 |

⇒ **3.3× the C's ratio.** That is the price, and it is legible.

**What it caught — 8 named:** 24 real mutation gaps; a claim-ledger census (236 measured / 354
inferred / 32 assumed / **66 conflated**); ★ **a pool slot owning an address space** — clippy, the whole
non-GPU suite and *five hardware ladder rungs* were green while it was broken, because they all run one
worker; a census under-reporting caught by our own *distrust-the-good-news* arm; ★ **two boots reported
as evidence in which not one line of the changed code ran**; a **vacuous** establishment measurement
that said so instead of printing a green line; a compile-fail suite that was silently half-dead; and
**three probes that answered their own question, one inverting the verdict.**

⊘ **The honest reading, and it cuts toward the owner's worry: SIX of the eight are the machinery
catching a defect in the machinery.** Only two — the pool-slot address space and the 24 mutation gaps —
found defects in the **product**. ⇒ **~164 000 lines of verification, 8 catches, 2 in the product.**
The owner decides whether that is worth it; this ledger does not.

---

## 3. ⚠ THE SINGLE LARGEST HOLE — D2's cost has never been measured

`fb_join.md` §4.1 calls it *"a **performance** divergence … not a correctness one."* ⇒ `grep` for
`GB/s`, `bandwidth`, `latency`, `PCIe` across the four docs that would carry it: **zero hits, in either
tree, in any unit.** ⊘ The nearest prior is a *Mode-1* measurement of a *different* cause (0.1 GB/s vs
10.2 GB/s, ~100×, later recovered) — **it must not be quoted as if it were this.** Offered only so the
order of magnitude such things reach is on the record.

---

## 4. ★ THE TREND: CONVERGING — 5 of 5 last-day questions resolved TOWARD the C

| question | resolution |
|---|---|
| point, or copy? | **the C's shape**, chosen *on a C measurement at our exact address* |
| is there a second framebuffer crossing? | **the C built it twice** — *"the null result is not available"* |
| do we owe a completion plane? | **no** — the guest asks for no interrupt, and the C never called its own poll |
| must we inspect guest methods? | **retired** — zero privileged methods in the compute classes |
| whose queue does the host channel run over? | **the guest's** — host RM builds a channel over an object it did not allocate |

⇒ **The owner's worry is not confirmed by the record.** The drift is smaller than it feels, and the
direction of travel is *back toward the C*, repeatedly, usually with a C measurement as the reason.

---

## 5. ⚠ THE COUNTER-METRIC — a ledger that only exonerates is worthless

**5.1 The guest has not moved, and it is worse than "~9 boots."** A census over **all 106 committed
boot logs**: **24 rows** read exactly `191 arrived, 183 served, 8 REFUSED`, from `w210` (08-10) through
`w230d` (08-11); the three `fb-join` arms report the same triple. ⇒ ★ **27 boots, not 9.** Every one of
the 17 `cup2` boots from `w218` on reads **`CUP2_RC=TIMEOUT`**.

**5.2 The split, counted over 45 commits since `w218`:** ~20 capability, ~25 machinery/controls/record.
⇒ **~45 % capability, ~55 % correctness-of-our-own-machinery.**
★★★ **And of the 45 commits, ZERO moved the doorbell census.** Every capability commit landed on a boot
printing `191/183/8` and `CUP2_RC=TIMEOUT`.

**5.3 ★ But the stall's cause is NOT any divergence in this ledger.** From the S1 audit:
> *"**The decisive fact is not a gate. It is a MISSING VERB** … it rings the isolate's host channel,
> **which the guest's methods are never copied into** … Scheduling an empty host ring makes the host
> engine consume nothing, correctly and forever."*

⇒ **The guest is blocked by one unbuilt verb**, named identically in three independent places. Every
rung since `w218` built **address-plane** machinery — memory the engine could reach — while the
**submission-plane** verb that would make it read anything stayed unwritten. ⚠ **Twenty-seven boots of
address-plane work is a coherent programme; it is also twenty-seven boots in which the one thing that
would move the guest was, by each rung's own admission, out of scope.**
