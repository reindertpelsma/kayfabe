# w281 PRE-REGISTRATION — the PUSHBUFFER read out of our own framebuffer

**STATUS: LIVE — 2026-08-12.** Written and committed **before** the boots. Branch
`w281-lift-the-pushbuffer-refuse` off `d800cac` (the recovered `w280` RESULT).

**The rung:** `w279` ended at `FwdFault::PushbufferAperture { va: 0x1_2002_0000 }` — the ring
is read through the join, the **pushbuffer the ring points at** is refused by a hard-coded
`VidmemRoute::Refuse` placed there so that boot could attribute the ring's bytes. `w280`
measured that the raw client's pushbuffer is `pb=V:0x40000` — **vidmem** — so that gate is on
the client's path and on nothing else (all 16 of `cup2`'s are `pb=S:`). **This lifts it.**

---

## ⊘⊘ WHAT WAS ALREADY BUILT, CHECKED BEFORE WRITING ANY CODE

Twenty-one consecutive lanes found their premise already built. Checked this time:

- `git log --all --oneline` on the branch, and `traces/boots/w280/`. ★ **The `w280` boots had
  already been RUN** — by the lane that was OOM-killed before it could commit them. They were
  recovered from `vh` and committed as `d800cac` rather than re-run. **One boot saved.**
- `grep -rn "read_pushbuffer" --include=*.rs` — one production call site
  (`kayfabe-rt/src/device.rs`), no existing vidmem path, no existing flag.
- `plan_gpfifo_ring` / `fetch_ring_bytes` — **the precedent exists** and this rung copies its
  shape rather than inventing one.

## ★★★ THE CHANGE, AND WHY IT IS NOT A ONE-LINE EDIT

Reading a vidmem pushbuffer needs `FbBytes`, whose production impl takes the plane mutex —
`LockRank::Plane`, **rank 0** — while `route_act` holds ranks 1 and 2. `check_acquire` refuses
that `core → plane` acquisition **by name**, so the unsplit shape **cannot run**. ⇒ the same
three phases `w235` forced on the ring: `plan_pushbuffer` (under the locks; every translation
refusal raised there) → `fetch_pushbuffer` (every ranked guard dropped) → `decode_pushbuffer`.

⚠ **The TOCTOU is accepted, not dissolved**, and it contradicts a comment that declined it —
see §"OWNER RULING" below.

★ **Its own flag**, per `w279`'s ruling: `KAYFABE_PUSHBUF_VIDMEM`, *not* `KAYFABE_RING_VIDMEM`.
Necessary-not-sufficient alone — the bytes still come from route B's `FbSource`, so both must
be on. Printed as `armed=` **and** `reachable=`.

## THE ARMING — one variable, and it is the NEW FLAG

Device source identical to `e360e29` on all arms. All carry `w271_pin`'s six arms + route B.

| arm | order | workload | `RING_VIDMEM` | `PUSHBUF_VIDMEM` | why |
|---|---|---|---|---|---|
| `w281_client` | **first** | raw CE client (53 ioctls) | on | **on** | the rung |
| `w281_clientoff` | second | raw CE client | on | **off** | the same-revision control — isolates the new flag |

★ The rung's arm runs **first** (`w277`'s rule): if the session is cut short, the control is
what is missing, and a control is recoverable where the measurement is not.

⊘ **`cup2` is NOT run this rung, deliberately.** `w280` measured 16/16 `pb=S:` — this gate is
not on `cup2`'s path, so a `cup2` arm could only produce a `CUP2_RC=124` that is already
committed twice. The owner's mandate is *"iterate until the raw client passes, THEN test
`cuCtxCreate`"*; `cup2` returns when the client's three criteria are met or when a rung
produces a device change that could plausibly touch it.

## ARMS — graded by ADDRESS and by IDENTITY, never by count

⚠ `w278`→`w279` is the standing case: same fault variant, different VA, opposite meaning.

| # | prediction | how it is read | weight |
|---|---|---|---|
| **H1** ★★★★ *(the rung's own question)* | `PushbufferAperture` at `va 0x1_2002_0000` is **GONE**, and a new, differently-named refusal appears | every `FwdFault` by name **and** by VA | most likely |
| **H2** ★★★ | the new device line `FWD-PUSHBUF … → VIDMEM RUNS PLANNED` appears with `pb_vidmem=true fb_source=true` | the line itself; ⊘ absent ⇒ the route was never taken and every H below is unmeasured | most likely |
| **H3** ★★★★★ *(the milestone)* | the client's **three** criteria in the guest: `GP_GET` catches `GP_PUT`; the bytes moved (read back); the semaphore carries the declared payload | `R33 arm 1 COPY` verbatim, and `R33_RC=0` | ⊘ **not predicted** — see "what this cannot decide" |
| **H4** ★★★ | the methods DECODE: the descent prints `pbm[16w of 64B]` with `SET_OBJECT 0xc7b5` and the semaphore `0x120022000` — the same words the client independently printed | the `pbm[..]` decode in the doorbell descent | likely |
| **H5** ⚠ *(the regression arm)* | a channel whose pushbuffer is vidmem and **blank** now decodes to `Opaque` methods and forwards nothing — no CE span, no semaphore | `CE-SUBMIT` by `by=`/`src=`, `FWD-RING … spans=` | ⇒ **a non-zero `spans` from a blank page would be forbidden #2** |
| **H6** | the wall moves to the **CE operand / semaphore** — the next address after the pushbuffer | the new refusal's VA vs `0x120022000` (semaphore) and the operand ranges | likely |
| **H7** ⊘ *(the low arm, named)* | **nothing changes** — the refusal stays at `0x1_2002_0000` ⇒ the flag is not reaching the path | H2's line absent, refusal VA unchanged | ⇒ the widening is not wired, not "cup2's wall is different" |
| **H8** | the control (`clientoff`) is **identical to `w280_client`** | refusal VA `0x1_2002_0000`, `R33_RC=1` | ⇒ if the control differs, the device changed under me and **every arm is void** |
| **H9** ⚠ | a **VOID boot** — no client, empty artefact reading as favourable | the known-positives below | the trap that already bit `w279` |
| **H10** | boot fails / hangs / `ENOSPC` | not a data point | |

### ⊘ THE VOID GUARDS — asserted before any zero is read

`w279`'s first attempt was a void boot that printed **its own predicted success**. Every zero
here is gated on a known-positive **on its own grep**:

- `GUEST_MD5` equal to the host-side md5, `GUEST_EXECUTABLE=yes`, `total=53 failed=0`.
- `RING-PROJ lines = [N]` — 0 ⇒ no doorbell descent happened at all.
- `fbRING rows = [N]` — 0 ⇒ VOID, not "no join".
- ★ **`PUSHBUF-VIDMEM` armed line present with the right polarity on BOTH arms** — an unarmed
  control that is silently armed is the same defect as an armed run silently unarmed.
- the runner asserts the **artefact** (`[ -s "$CLIENT" ]`, non-empty md5), never `cargo`'s RC.
- `R33_RC` **anchored** (`^R33_RC=`); the unanchored form matches the info banner.

## ⊘⊘ WHAT THESE BOOTS CANNOT DECIDE, stated before they run

- **They cannot make the client pass.** The pushbuffer is one of at least four addresses the
  submission needs (ring, pushbuffer, semaphore, CE operands). `w279`'s ring fix moved the wall
  one address; this is expected to move it one more. **Naming which of the three criteria is
  met is the deliverable, not a green.**
- **They cannot see the completion plane** — it still has no oracle.
- **They cannot say anything about `cup2`.** Its pushbuffers are sysmem; this gate is not on
  its path, measured.
- ⚠ The `BAR1 GP_PUT` witness has **measured false positives**. Labelled, never filtered.
- One workload, one chip (GA106), one driver (`580.159.04`), one boot per arm.

## ★★★ OWNER RULING REQUESTED — stated BEFORE the boot, not after a green

The brief says to escalate rather than quietly widen. **One item qualifies**, and it is not
the flag:

> **`device.rs`'s own doc declined this split in writing**: *"Not split into
> plan/execute/commit, deliberately … The split would mean resolving addresses, dropping the
> lock, then fetching method bytes through a translation the guest was free to invalidate in
> the gap — a TOCTOU built on purpose."*

I have **overridden that comment**, corrected it in place, and here is the reasoning, so the
owner can reverse it if the trade is judged wrong:

- The comment's premise was **guest RAM only**. A vidmem read needs the rank-0 plane mutex,
  and `check_acquire` refuses it beneath the core's ranks **by name** — so the unsplit shape
  is not "riskier", it is **inoperable**.
- A **deadlock a rank checker refuses by name** strictly dominates a stale read.
- The exposure is **bounded and is not a privilege escape**: the runs are computed from that
  channel's own table under the lock and never recomputed outside it, so the worst case is
  reading a page **the guest itself named** and has since unmapped. Never a page it never owned.
- **The ring has run under exactly this exposure since `w235`** with the same justification.

⇒ If the owner wants the TOCTOU closed rather than accepted, the fix is a generation counter
on the address table checked at commit — real work, and it should be its own rung. **Nothing
here forges a completion, reads host memory, needs root, or weakens hostile-guest isolation.**
