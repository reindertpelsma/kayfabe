# w266 PRE-REGISTRATION — LEG 5: pin the ONE page hardware named

> ### STATUS — 2026-08-12 / **LIVE — PRE-REGISTRATION, committed before the boot.**
> Branch `leg-5-completion-pin`, off `leg-4-populate-witness` = `f265025`.
> Predecessor: `traces/boots/w265/RESULT.md` (rev `2f02621`, two arms, real GA106).
> Authority: `mode2_address_table.md` §6 (C tree), `RESUME_HERE_2026_08_12.md` §1 leg 5.

---

## 0. ★★★★★ LEAD — WHAT CONTRADICTS THE BRIEF, BEFORE ANYTHING ELSE

The brief is **right about the rung and wrong about one load-bearing mechanism**, and the wrong
part changes what a green row can mean.

### 0.1 ⊘⊘ THE MECHANISM: the eight doorbells the pin runs on are **REFUSED**, and the hardware executed anyway

`[measured, traces/boots/w265/run_w265_on_qemu.log:629, 639, 649, 658, 668, 677, 686, 695]` every
one of the eight doorbells that carries `PB-PIN … PINNED` is followed, within one line, by:

```
DOORBELL-REFUSED #9 token 0x00020013 at +0xbb0090 [FwdFault::PushbufferAperture]
```

⇒ **This device rang nothing.** `CE-SUBMIT` is `0`, `forwarded` is about engine objects, and the
forwarding plane refused all eight submissions by name. And yet the host GPU faulted eight times
executing the guest's methods.

★★★ **The only mechanism that explains that is the one legs A2 + B already landed**: the host
channels were born over the **guest's own ring** (`adopt=GUEST-RING`, 16) with the **guest's own
USERD** (`userd=GUEST-USERD`, 16). The guest writes `GP_PUT` into a page the host channel's own
host engine reads, so the hardware **fetches without a doorbell from us**. `[corroborating,
w265_on]` `BAR1 GP_PUT — FIRST advance on USERD page +0x136000 (val=0x1), page 14 of at most 16`
appears immediately *before* each refused doorbell, and 66 such advances are counted.

⚠ **This matters for grading**: it means `CUP2_RC` movement does **not** require the forwarding
plane to stop refusing. The engine is already running. It also means **nothing in this rung's
control path gates the experiment**, so a null result cannot be blamed on the refusal.

### 0.2 ⊘ THE SOURCE: the faulting channels are **Ce**, the declaring channels are **GrCompute**

`[measured]` the eight `RING-PROJ` lines that precede the pins are `engine=Ce`, `chan=8…15`,
`pdb=0x201000`. The eight `COMPLETION-DECLARE` lines are `engine=GrCompute`, `chan=0…7`. They are
**different channels**, and the semaphore page is *shared*: `0x20440ff80 … 0x20440fff0` at a
16-byte stride, one 4 KiB page, `gpa 0x59a0f80 … 0x59a0ff0`.

⇒ The address this rung pins is sourced from the **GR** channels' declarations and pinned into
the **CE** channels' VA space. That is only correct if they share a VA space. `[measured]` every
CE channel prints `pdb=0x201000 vas=0x5c000007`, and every GR/CE pair in the `ENGINE-OBJECT`
census shares one client `0xc1d0000c`. **I believe they share it and the boot will say so**: if
they do not, the address table will answer `Miss` for the page in the CE channel's pdb and the
`SEMA-PIN … TABLE` row will print it. ⊘ **That is the falsifier, and it is not routed around.**

### 0.3 ★★★★★ AND THE CAPABILITY EXISTS — nothing was built that a boot has not already run

Per the brief's ninth-lane warning, I looked before writing. The result is **mixed and stated as
such**:

| piece | state before this rung |
|---|---|
| `SharedDevice::pin_guest_ram` → `plan_pin_guest_ram` → `OS_DESCRIPTOR` FIXED at the guest's VA | ✔ **BUILT and MEASURED WORKING on a live guest** (`PINNED` 0 → 8, `w265`) |
| the aperture check, the hypervisor-layout lookup, the negative control, the run coalescer | ✔ **BUILT**, shared verbatim with leg 4 |
| the semaphore VA, resolved, with its GPA, per channel | ✔ **ALREADY IN THE DEVICE** — `COMPLETION-DECLARE … site=GuestRam { gpa: … }` |
| **a way to read those addresses back out** | ⊘ **DID NOT EXIST.** `WatchList` had `declare`, `stats`, `live`, `sweep` — and no reader |
| **a consumer that presents guest-RAM addresses to the pin** | ⊘ **DID NOT EXIST.** `back_census_framebuffer_leaves` handles `Site::Framebuffer` only, and its own doc calls the `GuestRam` rows *"this pass's standing negative controls"* |

⇒ The honest sentence is: **the primitive was built, the addresses were in memory, and the two
were not connected.** This rung is 1 reader (`WatchList::declared_sites`) + 1 consumer
(`SharedDoorbell::pin_completion_guest_ram`, which is leg 4's function with a different source)
+ 1 arm. **Not a new mechanism.**

---

## 1. THE ARM — one variable

`KAYFABE_GUEST_SEMA`, `off` (default, refuses `on`/`1`) | `pin`. **Fourth** selector, not a rider
on `KAYFABE_GUEST_PUSHBUF`, so the two arms differ in exactly one variable —
`w263`'s RESULT §3.1 records what the alternative costs.

```
arm   FB_JOIN  GUEST_RING  GUEST_PUSHBUF  PT_WITNESS_EXEC  GUEST_SEMA
off   shared   ring        pin            on               (unset)   <= byte-for-byte w265's `on`
on    shared   ring        pin            on               pin       <= THE RUNG
```

★ The `off` arm is **`w265`'s `on` arm re-run at a new revision**, which makes it a cross-revision
guard as well as a control: if its numbers move, the comparison is void and I will say so before
reading anything else.

---

## 2. ★★★ WHAT I EXPECT, AND WITH WHAT CONFIDENCE

The brief asks for this explicitly and asks me not to flinch. **I predict `CUP2_RC` does NOT
move, at `p = 0.72`** — a sixth consecutive zero — and I predict **the `Xid` identity DOES move**,
at `p = 0.6`. Those are different rows and the reasoning differs.

### 2.1 Why the `Xid` should move (`p = 0.6` that the fault leaves `0x2_0440f000`)

The chain is short and every link is measured: the engine faults `FAULT_PDE` writing that page; a
`FAULT_PDE` is a missing directory entry; the pin installs one at exactly that VA (that is what
`placed_as_asked=true` means); and the identical chain at the identical VA granularity **just
worked** for the eight pushbuffer pages. The 0.4 is not doubt about the mechanism — it is
`w265`'s own standing caveat, printed by the pin about itself: *"this says NOTHING about whether
the host channel is bound to a VA space in which those VAs resolve."*

### 2.2 Why `CUP2_RC` should NOT move (`p = 0.72`)

★ **Because a write landing is not a wait being satisfied, and I can name three separate things
that must ALSO be true, none of which this rung touches.**

1. The engine must reach the semaphore **method** with everything before it succeeding. The
   pushbuffer pages are pinned, but nothing has ever measured that the *operands* those methods
   dereference resolve — `GR-ADDRESS-CENSUS` reports `bound=4 unbound=1` with
   `SET_SHADER_SHARED_MEMORY_WINDOW` at `0x7edca1000000` **Unresolved**. A fault on the next
   operand simply moves the address again.
2. The payload must be **1** and must land at the **exact 16-byte slot** the guest polls, not
   merely somewhere in the page. Eight channels poll eight different slots.
3. The guest must **see** it. `AWAKEN_ENABLE = 0` ⇒ polled, so the guest's CPU read must observe
   the engine's write — a coherence question across the `OS_DESCRIPTOR` mapping that **nothing in
   this tree has measured**.

⇒ The modal outcome I expect is **`SEMA-PIN` green, `Xid` at a new address, `COMPLETION-WATCH
NOT-OBSERVED` unchanged, `CUP2_RC = 124`.** That is progress and it is not the north star.

### 2.3 ⊘ The distinction the brief demanded, stated as two rows

| question | the row that answers it | what it cannot say |
|---|---|---|
| *is the semaphore page writable?* | `SEMA-PIN … PINNED … placed_as_asked=true` **and** the `Xid` no longer naming `0x2_0440f000` | nothing about a value |
| *was the guest's wait satisfied?* | **`COMPLETION-WATCH … OBSERVED`** — the declared payload appeared at the declared address, read out of guest RAM by the observer | it is the ONLY row that can precede `CUP2_RC` moving |

⚠ They can dissociate in **both** directions: a page can be writable with nothing written to it
(most likely), and — much less likely — the observer could see a payload while `cup2` still hangs
on something later.

---

## 3. THE PRE-REGISTERED SCORECARD

⊘ Graded on **identity rows, not counts**, per `w265`'s own lesson: `grep -c Xid` returned 8 on
both arms while five facts changed underneath it.

| # | observable | pred `on` | why it is here |
|---|---|---|---|
| S1 | `GUEST-SEMA arm=` | `pin` on `on`, `off` on `off` | the arming **as the device saw it**, out of each boot's own log |
| S2 | `SEMA-PIN` lines | ≥1 on `on`, **0** on `off` | ⊘ the control's expected result is *no line at all* |
| S3 | `SOURCE … declared completion(s)` | **8** | the reader works; 0 would mean an ordering failure, not an absent guest |
| S4 | `page(s) after de-duplication` | **1** | ★ the eight 16-byte-strided targets are ONE page |
| S5 | `TABLE … resolved in guest RAM` | **1** | the table knows the page in the **CE** channel's pdb (§0.2's falsifier) |
| S6 | `TABLE … MISS` | **0** | a MISS here refutes §0.2 and is the interesting failure |
| S7 | `NOT-IN-GUEST-RAM` | **0** | `miss = fault` held; a vidmem answer must refuse by name |
| S8 | `PINNED` (fresh) | **1** | one page, first doorbell |
| S9 | `ALREADY PINNED (idempotent replay)` | **7** | ★ the other seven doorbells replay; ⊘ this row is why fresh and replay are counted apart |
| S10 | `placed_as_asked=true` on every sema run | **yes** | a FIXED map that landed elsewhere is not this pin |
| S11 | NEGATIVE CONTROL `REFUSED BY NAME` | **yes** | derived one page past the top of every stated run |
| S12 | **`Xid` COUNT** | 8 → ? | ⊘ **KNOWN BLIND** — kept only so its blindness is on the record |
| S12a | **`Xid` ENGINE** | `CE2`/`CE3` → ? | identity row |
| S12b | **`Xid` CLIENT** | `HUBCLIENT_CE0/CE1` → ? | identity row |
| S12c | **`Xid` DISTINCT ADDRESSES** | ★ **`0x2_0440f000` GONE** | ★★★ **the rung's own claim** |
| S12d | **`Xid` ACCESS TYPE** | `VIRT_WRITE` → ? | identity row |
| S13 | **`COMPLETION-WATCH` verdict** | `NOT-OBSERVED` ×8, `last_seen=0x0` | ★★★★★ **the row that separates "writable" from "satisfied"** |
| S14 | **`CUP2_RC`** | **124** (`p = .72`) | §2.2 |
| S15 | `CE-SUBMIT` / `RETIRED` | **0/0** | ⊘ nothing here submits; a non-zero would be a surprise worth its own rung |
| S16 | `PB-PIN … PINNED` | **8** on BOTH arms | guard — leg 4 must be unchanged |
| S17 | `PT-DECODE bound` / `unwitnessed` (1st pass) | 19 615 / 19 874 on BOTH | guard — cross-revision |
| S18 | `refusals` (`StraddlesLiveBinding`) | **255** on BOTH | ⚠ the carried-forward unpaid cost; if it moves, this rung touched something it should not have |
| S19 | `RmInitAdapter failed` | **0** | guest alive on both |
| S20 | guest `NVRM` / `GR-BIRTH` | 31 / 24 on BOTH | guard |
| S21 | `ENGINE-OBJECT seen/fwd/ref` (last) | 34/32/2 on BOTH | guard |
| S22 | doorbells `REFUSED` | 16 on BOTH | ⊘ §0.1 — this rung does not change the refusal, and must not |

---

## 4. ⊘ WHAT THIS RUN WILL NOT BE ABLE TO PROVE, WHATEVER IT SHOWS

- ⊘ **That the pin is what moved any `Xid`.** One variable is armed, so attribution is to **the
  arm**; the arm has one consumer, which is narrower than `w265`'s (13 343 bindings), but it is
  still an arm and not a page.
- ⊘ **That the guest's CPU can SEE an engine write to a pinned page.** No row here reads the
  guest's view of the page after a write. `COMPLETION-WATCH` reads it through the **VMM's** guest
  RAM, not through the guest's own mapping.
- ⊘ **That the 255 `StraddlesLiveBinding` refusals are benign.** Carried forward, still unpaid.
- ⊘ **That a page-table page first written after its doorbell would be witnessed.**
  `by-executor=39` bounds the leg-4 fix and this rung does not test it.
- ⊘ **Anything about multi-page semaphore pools.** `pages.len() == 1` here, so `PUSHBUF_MAX_PAGES`
  is unexercised on this boot and rests on the arm tests.
- ⊘ **That the GR and CE channels share a VA space** — the boot can only show that the *address
  resolves in the CE channel's pdb*, which is the operative fact, not the general claim.
- ⊘ **Ordering under a different workload.** The source is the watch list, which the GrCompute
  doorbells fill. `[measured, w265]` they all precede the first CE doorbell. A workload that
  reversed that order would print `NO PAGE TO PIN` — the pass says so in that line rather than
  reading as "the guest declared nothing".
