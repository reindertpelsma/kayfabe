# w270 — PRE-REGISTRATION: **the CE launch's own DESTINATION is the pin's fourth source**

> ### STATUS — 2026-08-12 / **LIVE — PRE-REGISTRATION.** Committed **before** any of this
> rung's code exists (`operand_targets_of`, `observe_ce_operand_targets`,
> `pin_operand_guest_ram`, `KAYFABE_GUEST_OPERAND`, `w270_run.sh`, `w270_grade.sh` — none of
> them are in the tree at this commit). Branch `w270-the-operand-pin`, off
> `w269-the-spin-address` = `c76c5bf`. Graded after the boot in `traces/boots/w270/RESULT.md`.

---

## 0. ⊘⊘ LEAD WITH WHAT CONTRADICTS THE BRIEF — and the first item is already on disk

### 0.1 ★★★★★ **THE CHAIN THE BRIEF CALLS "CORRELATION ONLY" IS ALREADY PROVEN BY IDENTITY**

`w269` RESULT §1.2 says, and the brief repeats it:

> ⊘ **It is NOT proven.** I have not shown the faulting submission is the one that would have
> written `2` into `0x2_0440ff70`; the correlation is by channel and by ordering only.

⊘ **That qualification is unnecessary. The joining row was written seven hours earlier**, in
`traces/boots/w268/run_w268_pass_qemu.log:980`:

```
SEMA-SOURCE-CE token=0x0001000f proc=2 chan=8 engine=Ce
  → methods=11 launches=3 opaque=7 release_target(s)=1 [0x20440ff70] ⇒ 1 page(s)
```

`methods=11 launches=3` **is** the faulting submission (`w268` §3.2 identifies it by exactly
that pair), and the same line reports that this submission's own `SET_SEMAPHORE_A`/`_B`
operands name **`0x20440ff70`** — the slot `w269` measured the guest polling for the value
`2`. ⇒ The link is **by the guest's own declaration read out of the faulting submission's own
bytes**, not by channel and ordering.

⚠ What remains genuinely unproven is narrower and should be said in its place: the *payload*
of that release is not printed (`release_target(s)` carries addresses, not payloads), so
*"this release would have written **2** specifically"* is still inferred — from the slot
holding `1` and the guest wanting `2`, with one release already delivered by the channel's
**first** submission (`methods=3 launches=1`, same target, `w268` §2).

### 0.2 ★★★ **THE SECOND GPFIFO ENTRY IS NOT WHERE THE BRIEF SAYS TO LOOK — IT IS WHERE THE
INSTRUMENT CANNOT LOOK**

The brief and `w268` §5 both say *"the second GPFIFO entry's own methods are the place to
look"*. ⊘ The `pbm[…]` dump **cannot** show them and never could: `PROBE_RING_BYTES` is
`GP_ENTRY_SIZE` — **one** entry — so `push_headers` is handed `gp[0]` only. The same log line
prints the ring scan, and it names both entries:

```
nonzero=[0]=0x0000220202400000 [1]=0x00004e0202400020
```

⇒ `gp[0]` = va `0x2_02400000`, 8 dwords (the 32-byte release-only block the `pbm` shows);
`gp[1]` = va `0x2_02400020`, **19 dwords**, never dumped. The **decoder** already reads both
(`methods=11` spans them); only the *printer* is bounded. ⇒ The rung is not *"read further"*,
it is *"report what the decoder already decoded"*.

### 0.3 ★★★★ **THE BRIEF'S ITEM "CHECK WHETHER THE EXISTING PIN SOURCES ALREADY COVER IT" —
ANSWER: NO, AND IT IS STRUCTURAL, NOT AN OVERSIGHT**

The guest-RAM pin has exactly three sources today, and none of them can name a copy
destination:

| pass | source | what it can name |
|---|---|---|
| `pin_ring_guest_ram` | the channel's **ring** VA | one address, the GPFIFO |
| `pin_pushbuffer_guest_ram` (`KAYFABE_GUEST_PUSHBUF`) | the **GPFIFO entries'** pushbuffer VAs | `0x2_02400000` |
| `pin_completion_guest_ram` (`KAYFABE_GUEST_SEMA`) | declared GR completions ∪ **CE release targets** | `0x2_0440f000` |

⊘ And the operand census that *would* have seen it is **class-gated away**:
`completion_watch::decode_address_operands` refuses every write on a subchannel not bound to
`AMPERE_COMPUTE_B` (`crates/kayfabe-rt/src/completion_watch.rs:342`), so a copy engine's
`OFFSET_OUT_UPPER`/`_LOWER` is invisible to it **by construction**; and its only consumer,
`back_census_framebuffer_leaves`, handles `Site::Framebuffer` and calls the guest-RAM rows its
*"standing negative controls"*.

⇒ `w268` §3.2's *"no source in this device has ever named it"* is exact, and this rung builds
the fourth source rather than arming a built-and-orphaned one. ★ That makes it **unlike** the
last four rungs, and the difference is stated here so the RESULT cannot claim otherwise.

---

## 1. THE MECHANISM — one new source, the same pin, the same authority

★ **No new primitive.** `SharedDevice::pin_guest_ram` has now placed bytes on a live guest
three times (`w265` eight pushbuffer runs, `w266`/`w268` the completion page). This is its
**fourth source**, built in the shape `w268`'s CE release source established:

1. **decode** — `PushMethod::CeLaunchDma` already carries `dst`, `src`, `len`,
   `dst_is_virtual`, `src_is_virtual`, `work`, assembled by the **chip's own** codec
   (`Ga10xPushbuffer::ce_launch`) with an anti-`unwrap_or_default` rule on every operand. A
   new pure function `ceutils::operand_targets_of` collects the **virtual** `dst` extents
   (writes) and, for `CeWork::Copy` only, the virtual `src` extents (reads).
2. **page** — each extent is expanded to the 4 KiB pages `[va, va+len)` covers. ⚠ Extent, not
   base page: a copy longer than a page faults on its later pages too, and pinning only the
   first would produce a green supply row beside a live fault.
3. **table** — every page goes through `SharedDevice::resolve` exactly as the other two passes
   do. **`miss = fault` is unchanged**; this source cannot smuggle a page past the authority.
4. **pin** — one `OS_DESCRIPTOR` per contiguous run, `Prot::ReadWrite`, mapped FIXED at the
   guest's own VA.

### 1.1 ⊘⊘ WHY THIS IS NOT THE `cap2b` CLASS

`0x2_04420000` appears **nowhere** in this rung's code. Every address comes from the guest's
own method stream, read at this doorbell, framed by `kayfabe_abi::submit`, decoded by
`kayfabe_arch::PushbufferAbi::decode_run`. The literal is used **only** in the grader, to ask
*"did the address the guest declared happen to be the one hardware faulted on?"* — which is a
**question about agreement**, and answering it needs both numbers.

### 1.2 ⚠ WHAT IT DELIBERATELY WIDENS, AND WHY IT IS SAID HERE

The pass pins **both** the write extents and the read extents. A destination pin alone would
retire the fault we can see and expose a `VIRT_READ` fault on the source one boot later.
⊘ **Cost, pre-registered:** if the submission retires, this rung **cannot** say which operand
class mattered. The two are counted and printed separately so the `Xid`'s own `ACCESS_TYPE`
can still attribute a *surviving* fault.

---

## 2. THE ARMS — two boots, ONE variable

|arm|`FB_JOIN`|`GUEST_RING`|`GUEST_PUSHBUF`|`PT_WITNESS_EXEC`|`GUEST_SEMA`|`GR_ROUTE`|**`GUEST_OPERAND`**|
|---|---|---|---|---|---|---|---|
|`w270_off`|shared|ring|pin|on|pin|**passthrough**|(unset)|
|`w270_pin`|shared|ring|pin|on|pin|**passthrough**|**`pin`**|

★ **Both arms carry `GR_ROUTE=passthrough`**, because the wall under test only exists on that
arm: `refuse` never reaches chan 8's second submission at all. ⇒ `w270_off` is a **replication
of `w269b_pass`** and is graded as one (P8 below). ⊘ The route is **still not defaulted**.

---

## 3. PREDICTIONS — registered before the code exists

⚠ **Eight consecutive rungs predicted `CUP2_RC = 124` and were right.** The brief asks me to
reason fresh rather than inherit the streak, and to widen the low arms because **four
consecutive rungs have had their least-weighted arm fire**. Both are done below.

| # | prediction | p |
|---|---|---|
| **A1** | `OPERAND-SOURCE-CE` prints on chan 8's second doorbell (`methods=11 launches=3`) and names **≥ 1 virtual write extent** | **.72** |
| **A2** | that extent covers **`0x2_04420000`** — the guest's declaration and hardware's fault agree | **.62** |
| **A3** | the page **resolves in guest RAM** (`0 MISS`, `0 NOT-IN-GUEST-RAM`) — it shares the 2 MiB leaf `0x2_04400000` with `0x2_0440f000`, which resolved at `w268` | **.80** |
| **A4** | ≥ 1 run **`PINNED`** (not merely `ALREADY PINNED`) on the `pin` arm; **0** `OPERAND-PIN` lines on `off` | **.78** |
| **A5** | `Xid 31 … CE2 … HUBCLIENT_CE0 … VIRT_WRITE @ 0x2_04420000` is **GONE** on `pin` — graded by **identity** (engine, client, distinct addresses, access type), never by count | **.55** |
| **A6** | a **NEW** `Xid` appears on `pin` at a **different** address and/or a different access type | **.35** |
| **A7** | the spin probe reads `0x2_0440ff70` = **`2`**, or the wait list is **empty / N changed** | **.30** |
| **A8** | ★ **`CUP2_RC ≠ 124`** on the `pin` arm | **.25** |
| **A8a** | └ of which `RC = 0` (`cup2` passes end to end) | .09 |
| **A8b** | └ of which `RC = 1` or another bounded non-zero (`cuCtxCreate` *returns an error*) | .16 |
| **A9** | `w270_off` **replicates `w269b_pass`**: `Xid` at `0x2_04420000`, `CUP2_RC=124`, probe reads `N items = 1`, wants `2`, holds `1` | **.85** |
| **A10** | ★ **THE MODAL OUTCOME BY MY OWN MODEL**: the pin lands (A1–A4 fire), the fault at `0x2_04420000` clears (A5), and **`CUP2_RC` is still 124** with the wall at a **new named address** | **.40** |
| **A11** | ⊘ **DELIBERATELY WIDENED LOW ARM #1**: the decode names **ZERO** write extents — all three launches are `CeRelease` — so `0x2_04420000` does **not** come from a copy destination and this rung's whole mechanism is off-target | **.20** |
| **A12** | ⊘ **DELIBERATELY WIDENED LOW ARM #2**: the decode names write extents that do **NOT** include `0x2_04420000` — our codec and hardware **disagree about the guest's own operand**. ★ The most informative failure available; it would indict the codec, not the pin | **.12** |
| **A13** | carried guards unchanged on both arms: `COMPLETION-WATCH … OBSERVED = 8`, eight `GR-REPORT-SEMAPHORE` slots at `payload=1`, `PB-PIN`/`SEMA-PIN` green | **.85** |
| **A14** | `GR-CURSOR-READER stopped` / `PAGE-READER ASSERTION` still **FAIL** (unpaid a fourth time — `w267` §3.2, `w268` §3.3, `w269` §5.6) | **.85** |

### 3.1 ★★★ HOW I GOT TO **.25** FOR A8, AND WHY IT IS NOT THE STREAK

The brief is right that this rung is structurally different: the wait is **one** item, it
awaits **one** value, that slot has **already received `1` from hardware**, and the one known
obstacle is a fault at **one** page. If nothing else stood behind it, `.6`–`.7` would be
honest.

⊘ What holds it down is this campaign's own measured base rate: `w263 → w264 → w265 → w266 →
w267 → w268 → w269` each **removed the then-only named obstacle and revealed the next one**,
seven times running, on seven different planes. The chain to a passing `cup2` needs *all* of:
the decode naming the operand (`.72`), the page being in guest RAM and resolving (`.85`), the
submission retiring rather than faulting on something else (`.7`), the release carrying
payload `2` (`.9`), that item being the wait's only unsatisfied one (`.8`), and nothing
further inside `cuCtxCreate` blocking (`.55`). That product is `≈ .17`; I round **up** to
`.25` because the conjunction's terms are positively correlated (a guest whose operands we
decode correctly is a guest whose *next* operands we probably also decode correctly) and
because the brief's calibration note is a standing instruction to widen.

⇒ **A10 at `.40` is my actual central estimate**, and it is registered as a first-class
prediction rather than as a fallback, so a `124` cannot be reported as *"expected"* without
also reporting that the *fault* was expected to clear.

### 3.2 WHAT EACH `CUP2_RC` WOULD MEAN

- **`0`** — `cup2` passes. The first end-to-end pass in the campaign. ⚠ It would still say
  **nothing** about correctness of the bytes (no oracle here), about the `refuse` default, or
  about multi-process.
- **`1`** — `cuCtxCreate` **returns an error**. ★ Also progress and it must be reported as
  such: a bounded failure names the next thing, where a hang does not.
- **`124`** — re-read the wait list. Two sub-cases the grader must separate: the fault
  **cleared** (A10 — the wall moved, read the new `Xid` and the new polled address) versus the
  fault **survived** (A5 failed — the pin did not reach the page hardware wants, and the next
  candidate is `RcTriggered` / `w212`'s F6 masked leaf, task #235, which this rung does **not**
  pre-emptively build).

---

## 4. ⊘ WHAT THIS RUNG WILL NOT BE ABLE TO PROVE — registered in advance

1. ⊘ **Which operand class mattered**, if it goes green (§1.2).
2. ⊘ **The release's payload.** `release_target(s)` prints addresses, not payloads, and this
   rung does not add that. *"The release would have written 2"* stays inferred.
3. ⊘ **Correctness of any byte the copy moves.** There is no oracle on this path.
4. ⊘ **That the pin is what changed the `Xid`**, if the arm also changes binding counts —
   `w265`'s attribution lesson. The grader must read `PT-DECODE bound` on both arms.
5. ⊘ **Anything about the error/event plane** — `RcTriggered` and `w212`'s F6 masked leaf are
   untouched and stay untouched.
6. ⊘ **That `refuse` is safe to default.** Not this rung, not any two boots.
