# w271 — PRE-REGISTRATION: **the pin's identity is the `(base, extent)` PAIR, not the base**

> ### STATUS — 2026-08-12 / **LIVE — PRE-REGISTRATION.** Committed **before** any of this
> rung's code exists (`FwdFault::GuestRamPinTooShort`, `FwdFault::GuestRamPinOverlaps`,
> `GuestRamPinned::described`, `pin_guest_run`, `w271_run.sh` — none of them are in the tree
> at this commit; `git grep GuestRamPinTooShort` is empty here). Branch `w271-the-extent-key`,
> off `w270-the-operand-pin` = `416088c`. Graded after the boot in
> `traces/boots/w271/RESULT.md`.

---

## 0. ⊘ LEAD WITH WHAT CONTRADICTS THE BRIEF — three items, all of them cheap

### 0.1 ★★ **THE BRIEF SAYS "CHECK ALL FOUR PIN SOURCES … THEY SHARE THE BUG". They share the
PRIMITIVE; only ONE of them can currently EXHIBIT it, and that is worth saying before the boot**

The four sources are the ring (`pin_ring_guest_ram`), the pushbuffer (`pin_pushbuffer_guest_ram`),
the completion page (`pin_sema_guest_ram`) and the operand (`pin_operand_guest_ram`). All four
call `SharedDevice::pin_guest_ram`, so all four inherit the VA-keyed idempotence. ⊘ But the
defect only *fires* when a source produces a **growing run at a repeated base**, and:

- the **ring** run is derived from `gpFifoEntries × 8`, a value fixed at channel allocation —
  it does not grow across doorbells;
- the **pushbuffer** runs are derived from the GPFIFO entries present at each doorbell, so a
  base **can** repeat with a longer length (entry 0 at `0x2_02400000`, then entries 0+1 spanning
  `0x2_02400000 … +0x40`) — ⚠ **this one can grow, and nobody has looked**;
- the **completion page** is one 4 KiB slot page per channel — fixed;
- the **operand** is the one `w270` caught.

⇒ The honest claim is *"one measured, one plausible, two structurally fixed"* — not *"four
latent bugs"*. The fix is still the right shape for all four, and this rung will print
`requested`/`described` on **all four** so the question stops being an argument.

### 0.2 ⊘ **`w270`'s OWN "NEXT RUNG" TEXT PROPOSES A FIX THIS PRE-REGISTRATION DOES NOT TAKE**

`traces/boots/w270/RESULT.md` §2 offers three shapes: key on `(va, len)`, **extend** an
existing shorter placement, or **refuse by name**. ⊘ *"Extend"* is not available: an
`OS_DESCRIPTOR` is built over a fixed page list at creation and RM has no verb to lengthen one.
The choice here is therefore **refuse by name in the core, and describe the remainder as a
second descriptor in the VMM** — which is exactly what the multi-run loop already does for a
*fragmented* run, so the growth case reuses the shape rather than inventing one.

★ This also keeps `kayfabe-fwd`'s stated boundary intact: **only the VMM may derive a grant**
(`plan_pin_guest_ram`'s own doc comment, and `GuestRamGrant::originated_by_the_vmm`'s name).
A core that split the grant itself would be re-deriving the hypervisor's number — the thing
`#238` was built to stop.

### 0.3 ⊘ **`grep -c Xid` IS STILL THE WRONG INSTRUMENT AND `w271` INHERITS THE FIX, NOT THE
BUG.** `w270`'s harness already grades the fault by **identity** (distinct addresses, engine,
client, access type). Nothing new is owed here; it is recorded so that a reader does not spend
the rung re-deriving it. Same for the anchored `(^|[^A-Z_])CUP2_RC=` pattern (§0.2 of `w270`).
⊘ **And a grep of every committed boot log for a report that inherited the unanchored form is
owed and is being paid before the boot** (§5, item 1).

---

## 1. THE PREDICATE UNDER TEST

`vas.guest_ram_pins` is a `BTreeMap<u64, GuestRamPin>` keyed on the VA, and `GuestRamPin`
**already carries `len`** — the extent has been recorded since `w265` and never read.
`plan_pin_guest_ram`'s idempotence arm does `get(&va.0)` and returns `already: true` for any
request at that VA, **whatever its length**.

⇒ The change is:

1. **exact base, `described >= requested`** → replay, as today (`already: true`), and the log
   now says `fully covered` and prints both numbers.
2. **exact base, `described < requested`** → ⊘ **NOT a replay.** Refuse by name with
   `FwdFault::GuestRamPinTooShort { va, described, requested }`.
3. **no exact base, but an existing pin lies inside `[va, va+requested)`** → refuse by name
   with `FwdFault::GuestRamPinOverlaps { va, existing_base, existing_len, free_prefix }`,
   where `free_prefix` is how much of the request is clear of it. ⊘ Today this case would
   build a second `OS_DESCRIPTOR` over pages already described and ask RM for a *fixed* map at
   an occupied host VA — answered `0x51 NV_ERR_NO_MEMORY`, the status that cannot be told from
   exhaustion. It is the same identity defect from the other side.
4. The **VMM** (all four shim sources, via one shared `pin_guest_run` helper) turns those two
   refusals into a **remainder pin** — same run, same file offset arithmetic the run already
   licenses, at `va + described`. It loops until the whole requested extent is described, or
   until a refusal it cannot advance past.

⚠⚠ **The verdict words are part of the rung**, because `ALREADY PINNED … placed_as_asked=true`
was a **green verdict on a partial mapping** and that is this tree's most expensive recurring
failure class. Three words, never shared:

| situation | word |
|---|---|
| fresh, whole extent described by this call | `PINNED` |
| existing pin already covers the whole extent | `ALREADY PINNED (idempotent replay; fully covered)` |
| existing pin covered **part** of it; the rest is described now | `GREW` |

and **every** row prints `requested=<n> described=<n>` side by side, so a mismatch is *read*
rather than inferred.

---

## 2. WHAT COULD MAKE THIS RUNG MEASURE NOTHING

- ⊘ **The growth may not repeat.** `w270`'s three chan-8 doorbells are one boot. If this boot's
  submission 3 names its two extents at *different* bases, the `GREW` row never appears and A1
  fails for a reason that is not a defect in the fix.
- ⊘ **A remainder pin needs its own address-table resolution.** `w270` measured `16/16` pages
  resolved with `0 MISS`, so `0x2_04428000` is bound — but that was measured with the pin arm's
  arming, and a `MISS` here would refuse the remainder by a *different* name.
- ⊘ **RM may refuse the adjacent fixed map.** Two `OS_DESCRIPTOR`s at abutting VAs is a shape
  the bench has never run. `PlacementRefused` / `0x51` here would be a new finding, not this
  rung's answer.

---

## 3. REGISTERED PREDICTIONS

⚠ Five of the last six rungs had their **least-weighted** arm fire; `w270`'s central estimate
fired. The tails below are deliberately fat.

### 3.1 The mechanism

| # | prediction | p |
|---|---|---|
| **A1** | ≥1 `GREW` row on the `pin` arm, at base `0x2_04420000`, `requested=65536 described=32768`, remainder placed at `0x2_04428000` | **.78** |
| **A2** | `0` rows on either arm read `ALREADY PINNED` where `requested > described` — i.e. the false green is gone **by construction**, asserted as a count | **.90** |
| **A3** | the control arm (`w271_off`, operand pin unarmed) is a byte-for-byte replication of `w270_off`: `Xid @ 0x2_04420000`, slot `1`, `CUP2_RC=124` | **.80** |
| **A4** | ★ **no** non-operand source ever prints `GREW` (ring / pushbuffer / sema extents are stable) | **.62** |
| **A4′** | ⊘ widened: the **pushbuffer** source prints `GREW` at least once (§0.1 says it structurally can) | **.30** |
| **A5** | `OPERAND-PIN` fresh byte total on the `pin` arm > `w270`'s `32768` | **.75** |

### 3.2 The wall

| # | prediction | p |
|---|---|---|
| **B1** | `Xid @ 0x2_04428000` is **GONE** on the `pin` arm, by identity | **.70** |
| **B2** | **no `Xid` at all** on the `pin` arm (the whole fault class closed) | **.42** |
| **B3** | a **new** `Xid` at a **further** extent (`0x2_04430000` or beyond) | **.22** |
| **B4** | a new `Xid` at an address **not** in the `0x2_0442xxxx` family (a different engine/client, or a GR fault) | **.14** |
| **B5** | the polled slot `0x2_0440ff70` reads **≥ 3** | **.44** |
| **B6** | a **fourth** chan-8 CE doorbell appears (`methods` > 19) | **.40** |

### 3.3 `CUP2_RC` — the four the brief asked me to weigh

| # | outcome | p | reasoning |
|---|---|---|---|
| **C1** | **`CUP2_RC = 0`** — cup2 passes | **.10** | the extents would have to be the *whole* remaining story of `cuCtxCreate`, and `cuCtxCreate` is the largest single call in the program |
| **C2** | **`RC = 1`** — a bounded error out of `cuCtxCreate` | **.07** | nothing on this path has ever produced a bounded failure; every wall has been a spin |
| **C3** | **`RC = 124`, fault moved to a FURTHER extent** | **.22** | ⊘ I weight this **below** `w270`'s implied reading, and the reason is mechanical: the pin runs **at the doorbell**, over every operand the pushbuffer names *at that moment*, so a submission's new extent is described **before** that submission executes. `w270`'s recurrence was caused by the idempotence bug, not by ordering — so *"one more page every rung"* should stop, and if it does not, the cause is something other than the key |
| **C4** | **`RC = 124`, fault gone, wait still unsatisfied** | **.42** | ★ **my central estimate.** The supply side has been *necessary-not-sufficient* on every rung since `w260`; the completion plane has no oracle; and `w270` §4.3 stands — one satisfied item does not mean one item |
| **C5** | ⊘ **`RC = 124`, fault UNCHANGED at `0x2_04428000`** — the fix did not take | **.12** | the arming, the coalescer, or a `MISS` on the remainder |
| **C6** | ⊘ widened: `RC` is something else entirely (139/134/a guest crash) | **.07** | two abutting `OS_DESCRIPTOR`s is a shape the bench has never run |

★ **Called: C4, at `.42`.** ⚠ And the honest note beside it — `C4` is the *modal* answer for the
sixth rung running, so it is the prediction with the least information in it. The informative
pair is **B1 ∧ ¬B3** (`.70` × the complement of `.22`): *the extent story CLOSES rather than
recedes*. If that pair fires and `RC` is still `124`, the supply side is finished as a line of
attack and `F6` / `RcTriggered` (task #235) becomes the main line **on evidence** rather than by
elimination.

### 3.4 Carried guards (a move here changes what the comparison means)

| # | prediction | p |
|---|---|---|
| **D1** | `PT-DECODE bound` identical on both arms | **.85** |
| **D2** | `DOORBELL-XLATE`/`-STORE … WROTE` equal counts, and `≥ 18` on `pin` | **.75** |
| **D3** | `SEMA-PAGE-SLOT`, `pb run … PINNED`, `COMPLETION-WATCH … OBSERVED` all present on both | **.88** |
| **D4** | the offline suite is green and includes **new** extent-key tests that FAIL against `416088c` | **.92** |

---

## 4. ⊘ WHAT THIS RUNG WILL NOT BE ABLE TO PROVE, WHATEVER IT MEASURES

1. ⊘ **That the bytes the fill wrote are correct.** There is no oracle on this path. Carried
   from `w270` §4.4, unchanged.
2. ⊘ **That the release which advances the slot is the one the faulting submission owed.** The
   payload is still not printed — carried unpaid from `w270` §5.2 and *deliberately not* taken
   here, because this rung must be one predicate.
3. ⊘ **That the overlap arm (§1 case 3) is exercised.** It is a refusal for a case the bench has
   never produced; the offline tests are its only evidence, and a mock backend is exactly as
   strong as `guest_ram_pin.rs`'s own preamble says it is.
4. ⊘ **Anything about multi-process or a second context.** One guest, one `cup2`.
5. ⊘ **That `GR_ROUTE=passthrough` is safe to default.** Two more boots do not change that.
6. ⊘ **The full orphan-gate mutation scan**, unpaid since `w270` §4.7 — attempted again here,
   and if the launcher kills it a third time (`143`) that fact is reported as a `143`, not as a
   pass.

---

## 5. OWED BEFORE THE BOOT

1. ★★ Grep **every** committed boot log and RESULT for a `CUP2_RC` reading that could have come
   from the unanchored pattern, and say what was found — including *"nothing"*.
2. ★★ Offline tests that fail at `416088c` and pass here: growth at a repeated base, a covered
   replay staying a replay, the overlap refusal, and **the loop terminating** when `described`
   is `0`.
3. ★ `w271_run.sh` carries `w270`'s stamp gate, content check, anchored `CUP2_RC`, missing-file
   guard, and the `cap2b` negative check — with `0x2_04428000` added to the negative check,
   since it is now the address the rung is *about* and must not be a literal in the device.
