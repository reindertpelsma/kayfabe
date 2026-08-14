# w329 — WIRING THE RELEASE: both halves land, and the trigger the source nominates is not the event that occurs

**STATUS: LIVE, 2026-08-15.** Measured on `vh2` (real GA106, RTX 3060, host driver
`580.159.04`, stock guest driver), branch `w329-wire-the-release` off master **`d859beb1`**.
Every number below carries the tag of the boot it came from. Logs: `traces/w329_release/`.

Parent: `w327_the_allocation_cliff.md`, whose §6 dependency this rung implements and whose
nominated trigger it **refutes**.

---

## 0. THE ANSWER IN EIGHT LINES

1. ★★★★★ **`KAYFABE_BENCH_BW=28,31` PASSES 3/3** (`w329sup1..3`, `last_ok=31
   first_fail=NONE`) and **`4,64` still passes** (`w329sup64`). `already joined` install
   refusals go **32 → 0**. ⇒ **outcome (A)**, and by a trigger the brief did not name.
2. ⊘⊘⊘ **The trigger the brief and the source both name fires ZERO times.** Once RE-MAPS are
   excluded — and they must be, §2.3 — `PublishedUnbind::RevokeWholeJoins` revokes **nothing**
   on any workload measured here (`revoked=0` on `28,31`, `4,64`, cup3, cup8, R33).
   *"The guest's own free/unmap of the range"* is **not an event this workload produces.**
3. ★★★★★ **THE REASON, from the boot's own census: CUDA'S SUBALLOCATOR DOES NOT UNMAP ON
   `cuMemFree`.** `GUEST-DESCRIBES` for the guest VAS ends as **one 140 MiB run**,
   `0x7af90e000000+0x8c00000`, which contains the freed buffer **and** the new one. The guest
   re-points the **physical frame** into a new VA and leaves the old VA's PTE naming it.
4. ⇒ **What occurs is an ALIAS**: a new leaf naming a framebuffer frame we already joined for a
   **different VA in the same address space**. Leg 2 (`=supersede`) takes the join over, 22
   times per `28,31` boot, `CAPPED=0 ABORTED=0 table_store_disagree=0`.
5. ⊘⊘ **AND THE NAIVE FORM OF LEG 1 REGRESSED A CASE THAT WORKED.** Unguarded it revoked
   `va=0x7d05d0200000` — the bandwidth workload's **live output buffer** — and `4,64` went from
   PASS to `rc=719`. The **same-binary control** (`w329bc`, `=off`, 2/2 PASS) proves the
   regression was ours and not master's drift.
6. ★★★ **The fix is one predicate: a RE-MAP IS NOT A REMOVAL.** `settle` emits a re-map as
   unbind+bind of the same VA in one settlement; excluding those restores `4,64` (`w329b2`,
   2/2, `remaps_refused=4`) and turns leg 1 inert (`w329a2`, `remaps_refused=8 revoked=0`,
   byte-identical to the OFF control).
7. ⊘⊘ **THE BRIEF'S PRE-REGISTERED KNOWN-POSITIVE DOES NOT FIRE ON THE ARM THAT WORKS.**
   *"`joined=` must FALL"* reads `falls=0` on every `supersede` boot, because a takeover
   releases and re-installs at the **same key** inside one call. A rung graded on that number
   alone would have called the fix a mask. The instrument that works is
   **`already_joined` 32 → 0** and `SUPERSEDED=22`.
8. ⚠ **What is still not proven** is leg 2's *choice of winner* when the guest describes one
   frame at two VAs (§4.1). It is defensible, measured, and not settled.

---

## 1. THE OWNERSHIP ARGUMENT — asked for, and answered before anything was relaxed

The brief's central demand: *"State the ownership argument: who owns the host object at the
moment of unbind, what proves no other row still references it, and what happens on a partial
extent."*

**Who owns it at the moment of unbind: NOBODY, and that is by construction.**
`apply_settlement_as` removes the one `Binding` that named the object from the one
`AddressTable` that held it **in the same statement** that pushes the row into
`ApplyOutcome::revoked`. There is no instant at which the range is untabled and the caller has
not been handed the object — which is precisely the state `walker.rs:956-972` refused to
create when it wrote *"dropping the range from the table would leave the host object still
allocated and still mapped … with no core state naming it. That is worse than a leak."*
⇒ **That argument is entirely about the CALLER**, and it is true of a caller with no release
path and false of one that takes the host half with it. So the policy became a parameter and
`PublishedUnbind::Refuse` stayed the default for every caller that has none.

**What proves no other row references it — four independent facts, each checkable:**

| # | fact | site |
|---|---|---|
| 1 | the object is minted per leaf and bound as `HostBacking::whole` ⇒ `frees_object()`, **never an arena slice serving siblings at other offsets** | `bind_backed_fb_leaf`, `kayfabe-fwd/src/lib.rs:2900-2910` |
| 2 | a bind over a row that already carries a host object is **refused** (`GuestRamAddressTaken`), so one object cannot be reached from two VAs in one `Vas` | `kayfabe-fwd/src/lib.rs:2818-2830` |
| 3 | `AddressTable::bind` refuses **overlaps**, so no wider row covers it | `kayfabe-mmu/src/lib.rs` |
| 4 | `SparseFb::install_join` refuses **any** overlap, so a second join over the same framebuffer range never existed to be freed twice | `kayfabe-device/src/fbwin.rs:1087-1096` |

★ Fact 1 is not a new invention: it is the **same predicate** `kayfabe_fwd::unpublish_backing`
already gates its own `free` on, with the reason written at the site — *"`frees_object()` is
false for an arena slice: the object serves sibling bindings at other offsets, so freeing it
here would unmap the arena out from under them."* This rung reuses that regime rather than
adding a second reading of it.

**A partial extent: refused twice over, never handled.** `install_join` refuses any overlap so
a join is always a whole leaf, and `apply_settlement_as` revokes only a row whose **tabled
start equals the proposed VA**. Anything else keeps `UnbindsPublished` verbatim. This is
w291's exact-extent bound, applied to the other direction.

**And the guard was NARROWED, not deleted.** A row is revoked only when all four of: exact
extent, `frees_object()`, `BackingBytes::JoinsGuestWindow`, and the caller asked. ⇒ the
**18 228 guest-RAM pins** (`SoleBacking`) are untouched, and so is every published-GPA row
(whose reclaim also returns a GPA block and belongs to `unpublish_backing`). The population
this moves is exactly `joined_ranges`. `[measured]` `PUBCONFLICT_VAS[n=1331]` on a fix boot is
still 4 KiB-granular pin VAs (`0x7af917600000 … 0x7af917b32000`) — **all still refused**.

⚠ **The one thing the brief asked me to name and that is NOT proven: that a revoked row's VA
is dead.** §3 shows it usually is not. See §5.

---

## 2. LEG 1 — WHAT WAS BUILT, AND WHAT IT MEASURED

**The trigger.** `kayfabe_mmu::reach::apply_settlement_as` takes the host-published-unbind
policy as a parameter; `PublishedUnbind::RevokeWholeJoins` performs the unbind and returns the
host half in `ApplyOutcome::revoked`. Threaded through `commit_pt_decode_revoking` /
`commit_pt_sweep_revoking` → `SharedDevice::decode_pt_writes_revoking` /
`sweep_pt_tables_revoking` → the shim.

**The release.** `FbStore::release_join` (`install_join`'s inverse) for the guest's view, and
`SharedDevice::revoke_published_fb_leaf` for the host object, executed together in
`SharedDoorbell::release_revoked_joins` in **this order and no other**: table row → store →
host unmap+free → `drain_pending_releases()` **in the same trap**.

- ⚠ The store half must go **before** the host unmap, or the framebuffer store is serving
  bytes out of a region that no longer exists — a `SIGBUS` inside a guest MMIO access with no
  other detector.
- ⚠ The `munmap` is performed **outside the plane lock**, because `RegPlane::join_fb` already
  paid for that once: `[measured 2026-08-13, boot w289j]` dropping a region under the lock
  fired `lockwitness`' R1 assert inside an `extern "C"` QEMU callback as a **non-unwinding
  panic** and aborted the whole VMM, guest-reachably.
- ⊘ **Synchronous, per the direction ruling.** RM's TLB invalidate is *inside* the unmap
  ioctl, so deferring the revocation defers the invalidate by the same interval — a GMMU leak
  window. `drain_pending_releases` is called in the trap, not left to `w326`'s 250 ms tick.
  ⚠ Its cost: those host verbs run **with the BQL held**, which is `w323`'s
  `inline_exceptions` going up. That is the ruling's stated price, not a surprise.

### 2.1 THE TWO-ARM MEASUREMENT — one binary, `KAYFABE_JOIN_RELEASE` the only difference

| | **arm A** `w329a1..a3` (fix, default) | **arm C** `w329c1..c3` (`=off`, w327's state) |
|---|---|---|
| `revoked / released / stranded` | **8 / 8 / 0** | 0 / 0 / 0 |
| `drained` (orphan halves disposed) | **17** | 0 |
| `table_store_disagree` | **0** | 0 |
| ★ `JOINTRAJ falls` | **2** | **0** |
| `already joined` install refusals | **21** | **32** |
| `BW last_ok / first_fail` | 28 / **31** | 28 / **31** |
| first bad byte | **`0x0`** | `0x800000` |
| determinism | 3/3 byte-identical | 3/3 byte-identical |

★★★ **The known-positive fired**: `joined_ranges` fell `43 → 42` and `68 → 61` on every fix
boot and **never** on a control boot. `drained=17` for 8 revocations is `8 × 2` orphan halves
(one unmap + one free each) plus one pre-existing staged item — the arithmetic closes.

★ **`still_desired=1`** on two of the three fix boots: the lossy sub-case — a frame revoked
while the same settlement also wanted it bound — **fired, was counted, and was not assumed
absent.**

⊘⊘ **And the failure did not move in the direction that matters.** It moved **earlier**
(`0x0` instead of `0x800000`), because releasing 8 joins changed which of the new buffer's
leaves collides first. **Leg 1 alone does not help this workload at all.**

### 2.2 ⊘⊘⊘ AND IT REGRESSED A CASE THAT WORKED — `4,64`

| boot | list | release | result |
|---|---|---|---|
| `w327u4`, `w327u4b` | `4,64` | (did not exist) | **PASS**, 64 MiB at 22.13 ms |
| **`w329b1`** | `4,64` | leg 1, default ON | ⊘ **FAIL** — `last_ok=4 first_fail=64`, `rc=0/719` at byte `0x0` |

with `revoked=4 released=4 stranded=0 drained=9 still_desired=1 falls=2` and
`already_joined=17` (baseline 16). ⇒ **leg 1 revoked four joins and one of them was a frame
this settlement still wanted bound**, and the 64 MiB row that used to work no longer does.

★★★ **This is the hazard the guard existed for, in its milder form.** No double free is
expressible (§1) and none occurred — `stranded=0`, `table_store_disagree=0` on every boot. What
occurred is a **revoke of a translation that was still live**, which is the map/revoke
asymmetry landing on the dangerous side. ⇒ the pre-registered letter for leg 1 is **(B)**, not
(C): *"the release is unsafe as the guard suspected — name the aliasing case."* §3 names it.

⚠ **The control this needs, and it is pre-registered rather than assumed:** `4,64` with
`KAYFABE_JOIN_RELEASE=off` on the **same binary**. Master moved from `df3043be` (w327's base)
to `d859beb1`, so *"the release broke it"* and *"master drifted"* are both live readings until
that one boot separates them. `scripts/bench/w329_followup.sh` asks it **first**.

### 2.3 ★★★★★ AND THE REVOKED ADDRESS NAMES THE MECHANISM — it is a RE-MAP, not a removal

`w329b1`'s two revocation events, with the workload's own two rows beside them:

```
BW_BEGIN mib=4  ... in_ptr=0x7d05d0400000  out_ptr=0x7d05d0200000
BW_BEGIN mib=64 ... in_ptr=0x7d05c8000000  out_ptr=0x7d05d0200000     ← the SAME out buffer
revoked=1 ... still_desired=1 first=[va=0x7d05d0000000 fb_phys=0x1e00000 ...]
revoked=3 ... still_desired=0 first=[va=0x7d05d0200000 fb_phys=0x2000000 ...]   ← THAT VA
```

★★★★★ **`0x7d05d0200000` is the output buffer, and it is live across BOTH rows.** The
settlement proposed an unbind of it because the guest **re-mapped** it — freed and re-allocated
the same VA onto **different frames** — and `ReachShadow::settle` emits a re-map as
`unbinds.push(va); binds.push(leaf)` **in one settlement** (`reach.rs:741-748`), because the
table's own discipline is unmap-eager.

⇒ ⊘⊘ **`PublishedUnbind::RevokeWholeJoins` cannot tell a REMOVAL from a RE-MAP**, and it
treats both as *"this range is finished"*. For a removal that is right. For a re-map the right
action is a **re-point**: release the old frame's join *and* carry the row forward to the new
frame — which is the `RepointsPublished` refusal one function over, not this one.
★ `still_desired` is exactly that population, and it is why the counter was added before any
of this was known: the first revocation of the boot has `still_desired=1`.

⇒ **The narrow fix is one line of policy**: revoke only unbinds that are **not** accompanied by
a bind of the same VA in the same settlement. That is `Settlement::binds` keyed by VA rather
than by `phys`, and it removes the whole re-map population from the revoke path. ⊘ Untested at
the time of writing — stated as a mechanism with both sides cited, which is what §5 of this
document exists to keep honest.

---

## 3. ⊘⊘⊘ THE REFUTATION — the nominated trigger is not the event that occurs

`join_operand_fb_leaves`' cleanup table names the ending event as *"the guest's own free/unmap
of the range, seen as the page-table leaf ceasing to bind"*, and the brief repeats it. **On
this workload that event happens ZERO times.**

⊘⊘ It looked like eight, and the eight were the bug. `w329a1`'s `revoked=8` were all
**re-maps** — `w329a2` re-runs the identical list with the §2.3 guard in and reports
`revoked=0 remaps_refused=8`. ⇒ *"the trigger fires rarely"* was itself an artefact of
mis-classifying the only thing it ever caught.

**The evidence is the boot's own `GUEST-DESCRIBES` census** (`w329a1`, last pass):

```
[proc=2 gpu=0 pdb=0x201000 sweeps=13 trunc=0 runs=6
   0x200000000+0x40aa000, 0x204400000+0xc00000, 0x10000000000+0x200000,
   0x10002000000+0x200000, 0x7af90e000000+0x8c00000, 0x7af916e00000+0x800000]
```

and the workload's own two rows:

```
BW_BEGIN mib=28 ... in_ptr=0x7af914400000 out_ptr=0x7af914200000
BW_BEGIN mib=31 ... in_ptr=0x7af90e000000 out_ptr=0x7af914200000
```

★★★★★ **`0x7af90e000000+0x8c00000` spans `0x7af90e000000 … 0x7af916c00000`. It contains BOTH
buffers.** After `cuMemFree(28 MiB)` the guest still describes the freed VA range — CUDA's
suballocator returns the block to its own pool and **does not unmap**. What it does is hand the
**physical frames** to the next allocation at a **new VA**, leaving the old VA's PTE naming
them.

⇒ Our table is faithful and therefore holds **two rows for one frame**, and only the first can
carry the join. The 21 surviving refusals are exactly that: at the new `in_ptr`, at
`fb_phys=0x6200000 … 0x7c00000` and `0x2000000 … 0x2c00000` — **the same framebuffer offsets
`w327z1` recorded**, one allocation later.

★ Note what this does to `w327`'s own model: its row *"the VA must MOVE"* is right, and its
reason — *"if it does not, the leaf re-binds to the same `fb_phys` and the existing join is a
legitimate replay"* — is right. What neither of us saw is that when the VA moves, **the old VA
does not go away.**

⚠ **This is a class, not a detail.** A cleanup table that names an ending event is a
*hypothesis about the guest*, and it was never measured against one. It sat in the source as a
plan for four rungs and read as settled because it was specific.

---

## 4. LEG 2 — SUPERSEDE, and it is the honest completion

`KAYFABE_JOIN_RELEASE=supersede`. At the join site, **before anything is minted**
(`join_one_fb_leaf` step 0), if a join is already installed at this leaf's `fb_phys` and is
owned by a **different VA in the same address space**, take it over: unbind the old row, tell
the shadow, release the join, unmap+free the old host object, drain — then let the ordinary
four-step join run once and install cleanly.

- ⊘ Ordered **first** rather than at the `ALREADY_JOINED` refusal, because `RegPlane::join_fb`
  consumes the region on refusal: a retry there would need a second host object *and* a second
  `mmap`.
- ⊘ **Scoped to one address space.** A join owned by another `Vas` belongs to another isolate;
  *"the guest re-pointed it"* is not a statement anyone can make across that boundary. Those
  keep the old refusal.
- ★ **The shadow is told** (`ReachShadow::confirm_unbind`). A table without the row and a
  shadow that still claims it is a permanent `Miss`: the next `settle` would compare
  `published == desired`, propose nothing, and the VA would never rebind.
- ⚠ **Capped at 4 takeovers per frame per device life.** The superseded row is re-proposed by
  the next settlement (the guest still describes that VA) and becomes a publication candidate
  again — so an **uncapped** takeover is a ping-pong of host RM verbs on every doorbell. The
  cap is printed when it is reached.

### 4.1 ⚠ WHAT LEG 2 DOES NOT PROVE — stated, because this is the risky half

**That the old VA is dead.** The guest describes both. The device can serve only one, because
one frame carries one join. Today it serves the **old** VA and starves the new one; leg 2
serves the **new** one and starves the old. **Neither is correct in general.** The newest is
chosen for the reason `ReachShadow::settle` already chooses it for shape collisions — the
guest's most recent page-table write is its most recent statement about what that frame is
for — and an engine still pointed at the old VA takes a **contained** GPU fault, the cheap side
of the map/revoke asymmetry.

⇒ **What would make it safe rather than merely defensible**, in order of cost:

1. **A second signal.** An RPC `FREE` of the memory object behind the old VA (row 2a of
   `release_hint_census_and_the_reclamation_gap.md`) would corroborate that the old row is
   stale. `FreeUnknown x8` proves those RPCs arrive; nothing today joins them to a VA.
2. **Per-frame join multiplicity.** The real defect is that a frame carries **one** join while
   the guest may describe it at **two** VAs. A join that could be mapped at two host VAs — one
   `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR`, two `NV_ESC_RM_MAP_MEMORY_DMA` at two fixed addresses —
   removes the choice entirely. That is a `back_fb_leaf` change, not a policy change.
3. **Not preferable: keying reclaim off an invalidate.** `w325` §1.5 already settled that
   coherence is not liveness, in both directions.

### 4.2 ★★★★★ LEG 2, MEASURED — the target, 3/3

| arm | list | `KAYFABE_JOIN_RELEASE` | last_ok | first_fail | `already joined` | `SUPERSEDED` | `revoked` | `remaps_refused` |
|---|---|---|---|---|---|---|---|---|
| `w329c1..3` | `28,31` | **off** (w327's state) | 28 | **31** ×3 | 32 | – | 0 | – |
| `w329a1..3` | `28,31` | on, **no re-map guard** | 28 | **31** ×3 | 21 | – | **8** | – |
| `w329a2 1..2` | `28,31` | on, guard | 28 | **31** ×2 | 32 | 0 | **0** | **8** |
| ★ `w329sup1..3` | `28,31` | **supersede** | **31** | **NONE ×3** | **0** | **22** | 0 | – |
| `w329bc1..2` | `4,64` | off | 64 | NONE ×2 | 16 | – | 0 | – |
| ⊘ `w329b1` | `4,64` | on, **no guard** | **4** | **64** | 17 | – | **4** | – |
| `w329b2 1..2` | `4,64` | on, guard | 64 | NONE ×2 | 16 | 0 | 0 | **4** |
| ★ `w329sup64` | `4,64` | **supersede** | **64** | NONE | **0** | 18 | 0 | – |

★★★ **`already joined` is the instrument that works**: `32 → 21 → 32 → 0` tracks exactly what
each arm does, while `JOINTRAJ falls` reads `2 / 2 / 0 / 0` and is **highest on the arm that
fixes nothing**. ⊘ `CAPPED=0` and `ABORTED=0` on every `supersede` boot: the ping-pong bound
was never approached and the store and the table never disagreed.

⚠ **`revoked=0` on the winning arm.** Leg 1 contributes nothing to the fix. It is kept because
it is *correct* — a genuine removal must release its join — and because `remaps_refused` is the
only measurement anyone has of how big the unbuilt re-point population is.

---

## 5. ⊘ WHAT WAS WRONG IN THE BRIEF, AND IN THIS TREE

1. ⊘⊘⊘ **"The trigger it nominates is swallowed one layer down by `UnbindsPublished`."** True,
   and **not the reason the defect survives**. Un-swallowing it (this rung) releases 8 leaves
   and leaves 21 colliding. The nominated event is the wrong event (§3).
2. ⊘ **"Either half alone is dead code."** True of the two halves *the brief named*, and the
   pair is still not sufficient. "Both halves land" was necessary and is not the target.
3. ⊘⊘⊘ **"my claim that the guard is safe to relax at all, which is unproven"** — the brief
   asked for this to be graded, and **the brief was right to doubt it.** Split into two
   claims, because they got different answers:
   - **No double free is expressible, and none occurred.** `stranded=0`,
     `table_store_disagree=0` on every boot of every arm; the ownership argument (§1) holds;
     the population moved is `joined_ranges` and nothing else. ★ This half survived.
   - ⊘ **But the relaxation REVOKED A LIVE TRANSLATION and regressed `4,64`** (§2.2). *"Safe"*
     in the sense the guard was written for — no dangling host object — is not *"safe"* in the
     sense that matters to a workload. **I over-claimed this in the first draft of this
     document and the next arm refuted it within the hour.** ⚠ Same class as the brief's own
     warning: *a self-correction is a claim like any other* — and so is a self-clearance.
4. ⊘ **The offline baseline in the brief is stale.** It says *"Stable red set is 6 tests on 3
   targets"*. **Measured at master `d859beb1` on `vh2`: 7 tests on 4 targets.** The seventh is
   `every_unserviced_id_a_boot_recorded_is_classified`, failing with *"these ids are listed but
   no committed boot log ever recorded them: `0x83de030c`"* — `w327` added the `LEDGER` row and
   committed excerpts rather than the boot log that carries the id. **Verified by checking out
   `d859beb1` and running that target alone**, so it is master's, not this rung's.
5. ⊘⊘⊘ **The brief's pre-registered known-positive is the WRONG INSTRUMENT for the mechanism
   that works.** *"`joined=` FALLS across a cycle"* and *"a green `28,31` with `joined=` still
   climbing means you masked it"* — measured, `falls` reads **2 on the arm that fixes nothing**
   and **0 on the arm that fixes it 3/3**, because a takeover releases and re-installs at the
   **same key inside one call**, so no sampling point ever sees the dip. ⇒ A rung graded on it
   alone would have accepted arm A and rejected `supersede`. ★ The instrument that separates
   every arm is `already joined` install refusals: **32 / 21 / 32 / 0**.
   ⚠ Same class as `a_count_cannot_see_a_substitution` and as w327's own *"a vocabulary diff
   cannot see a quantity"* — here a **rate-of-change** instrument is blind to a
   release-and-reacquire, which is exactly the shape a takeover has.
6. ⚠ **A bash parser quirk that cost a bisect**: an apostrophe inside `${VAR:-default}`
   **inside double quotes** opens a quote bash never closes. `w329's fix` as a default value
   made a 90-line script fail `bash -n` at its **last** line, so every bisect pointed away from
   the site. ⇒ same class as *"read the definition site, not the name"*: the error's **location**
   was as misleading as its text.
7. ⚠⚠ **A counter that PRINTS and cannot be GREPPED.** `remaps_refused=` was emitted with
   **seventeen spaces** before its value on every one of 280 lines, so
   `grep -o 'remaps_refused=[0-9]*'` captured nothing and the arm reported `⊘UNMEASURED` on a
   boot where the counter fired. ★★★ **The cause was the EDITOR, not the code**: a Python
   heredoc rewriting Rust consumed the Rust line-continuation backslash as its own line
   continuation, producing a **valid** Rust string with the spaces baked in — no compile error,
   no clippy warning, nothing to fail. ⊘ The number was measured, printed, and read as
   unmeasured, which is the `dlen=0` class arriving from the opposite direction.

---

## 6. GRADING

All from `traces/w329_release/w329_arm_*.log`, `vh2`, 2026-08-14/15. ⊘ Every arm here ran with
`KAYFABE_JOIN_RELEASE` at its **default (`on`, leg 1)** unless the row says otherwise.

| workload | n | result | live? |
|---|---|---|---|
| `^CUP3_VAL=43` | **3** | `CUP3_VAL=43 CUP3_RC=0` ×3, `Xid=0`, `unserviced_distinct=40`, `host_rows=18297` | ★ LIVE — `revoked=2 released=2` on every boot, so the release fired on the graded workload rather than beside it |
| `^CUP8_BAD=0 ^CUP8_MAXERR=0` | 2 | *(filled below)* | |
| `R33 arm 1` | 2 | *(filled below)* | |
| cup8 at N=3072 (36 MiB) | 1 | *(filled below)* | |
| ★ known-positive `BENCH_NOLAUNCH` | 1 | **`BENCH_MODE=NOLAUNCH` PRESENT and `BENCH_NOLAUNCH_TOTAL_BAD=3670016`** | ★★★ FIRED ⇒ every `bad=0` here is asserted, not inherited |

**Offline suite** (`cargo test --workspace --features host-isolates --no-fail-fast`, `vh2`):
**7 failed across 4 targets** — and that is **master's**, not this rung's: §5 item 4 records
the check that established it. This rung adds **7 new passing tests** (4 in
`tests/tests/reachability.rs`, 3 in `crates/kayfabe-device/tests/fb_join.rs`) and no new red.

---

## 7. WHAT SHIPS, AND WHAT THE DEFAULT MUST BE

⊘ **`KAYFABE_JOIN_RELEASE` must default to `off` until the `4,64` control (§2.2) answers**, and
`on` must not be the default while a workload that passed at `w327` fails with it armed. The
mechanism is built, tested offline, and measured on hardware; what is not established is that
it is a net improvement on any workload this campaign runs.

★ **The three things this rung leaves standing, in priority order:**

1. ★★★ **A frame's join must be mappable at every VA the guest names it at.** One host object,
   N host VA mappings. That removes the choice §4.1 has to make and dissolves both the
   collision and the ping-pong. It is a `back_fb_leaf` change, not a policy change.
2. ★★ **A RE-POINT path for published rows.** `remaps_refused` now counts the population; today
   a re-mapped published row is frozen at its old frame forever.
3. ★ **Corroborate a revoke with an RPC `FREE`.** Row 2a of the reclamation-gap census: those
   RPCs demonstrably arrive (`FreeUnknown x8`); nothing joins them to a VA.
