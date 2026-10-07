# RESUME HERE — cold-start state, 2026-08-11 end of session

> # ⊘⊘ SUPERSEDED-BY `RESUME_HERE_2026_08_12.md` — GO THERE FIRST.
> **2026-08-12.** The night-run cron prompts still name *this* file and cannot be edited from a
> later session, so the redirect lives here. What changed: **the "next rung" below (give `HostGr`
> a passthrough server) is DONE** — and it turned out to be **leg C of three**. Legs A1+A2 are now
> built and hardware-witnessed (`w261`, `w262`); leg B is in flight. §3's standing debt is **still
> not discharged**, and every zero-movement result since was **pre-registered as zero**.
> Everything below remains true **as of 08-11** and is kept for its traps and its ledger.

> ### STATUS — 2026-08-11 / **SUPERSEDED 2026-08-12** (was: LIVE — the handoff doc).
> Written because the owner hit a usage limit; the next session starts with **no conversational
> context**. Everything needed to continue is here or linked from here.

---

## 1. WHERE THE NORTH STAR ACTUALLY IS

**`cuCtxCreate` hangs. `CUP2_RC=124`. Unchanged all day.** Nothing has ever executed:
`CE-SUBMIT → RETIRED` has never printed in ~127 committed logs.

**The wall, named exactly** (measured, `w260` and earlier):
- The guest waits on `SET_REPORT_SEMAPHORE` → GPU VA `0x2_0440fff0`, payload `1`, four words,
  `AWAKEN_ENABLE = 0` (⇒ **polled, not interrupt-driven** — F6 is *not* on this path).
- That address **resolves and is watched** — `COMPLETION-WATCH … NOT-OBSERVED`, 8×, every arm.
- **Nothing executes the GR pushbuffer.** The doorbell is refused at
  `FaultTag("Route::NotACopyEngineChannel")` (`shim.rs`), raised by `if route != DoorbellRoute::CpuCe`.
  ★ `DoorbellRoute::HostGr` has **two mentions tree-wide and ZERO consumers**.

⇒ ★★★ **THE NEXT RUNG IS: GIVE `HostGr` A SERVER.** Per the owner's ruling that server is
**passthrough**, not an executor: trap the doorbell write, translate the token, let the host GPU
fetch the guest's own ring.

---

### ★★★★★ UPDATE, LATE 2026-08-11 — THAT RUNG WAS BUILT, AND IT IS ONE LEG OF THREE

**Settled at the code, not inferred.** Making the host GPU execute the guest's GR work needs
**three** legs, and the doorbell is the third:

| leg | what it is | state |
|---|---|---|
| **A — the RING** | the host GR channel must be **born** over the guest's GPFIFO | ⚠ verb **BUILT** (`alloc_channel_over_guest_ring`, w230) — **ONE caller, the R31 probe.** The production birth path gives every GR channel `RingSource::Ours(None)` |
| **B — the CURSOR** | `GP_PUT` must be a word the **guest** advances ⇒ the guest's **USERD**, handed to RM **at creation** | ⊘ **NOT BUILT.** ★★★ RM **ZEROES** a caller-supplied USERD (#250) — adopting at first doorbell wipes the cursor that rang it |
| **C — the DOORBELL** | trap, translate guest token → host token, ring | ✔ **BUILT** `b734995`, branch `hostgr-passthrough-server`, default-off behind `KAYFABE_GR_ROUTE=passthrough` |

**Why C alone moves nothing**: a channel born `RingSource::Ours(None)` has its `gpFifoOffset` on
**our** ring object and its `GP_PUT` in **our** USERD, and the only writer of that word
(`submit_entry`, `rm.rs:4330`) refuses a handed-in ring **by name** (`RING_NOT_OURS`).
⇒ `GP_PUT == GP_GET` forever. Ringing the doorbell points hardware at a queue that is empty and
always will be.

★★★ **The tree already said this and nobody was reading it.** `GuestRing`'s own doc comment
(`rm.rs:653`): *"Nothing in this rung writes the guest's `GP_PUT` into our USERD, so the engine
still has nothing to fetch. **Adopting the ring and advancing the cursor are two rungs, and this is
the first.**"* And `guest_ring_adoption.md` §3 already said *"the host channel's birth has to
move"*. Fifth instance of *check whether the question is already answered* — but the **good**
shape: caught **before** the wrong thing was built.

⚠ **HONOUR THE PRE-REGISTRATION.** With only leg C, `CUP2_RC` moves by **ZERO** steps.
⊘ **That must NOT discharge §3's standing debt** (*"if the routed-doorbell boot also moves by one
step, doubt the model"*). **A one-legged stool falling over is not evidence against stools.** The
debt is discharged only by a boot with **A + B + C together.**

★★★ **The new blocker under leg A is a CALLER gap, not a missing primitive.** `join_fb_leaf` is
merged and proven on hardware, but it is driven by the **OPERAND** census — w260 joined FB phys
`0x400000 / 0x600000 / 0x800000`, while **the ring sits at `0x1000000` and is never presented,
because a ring is not an operand of the methods it carries.** ⇒ Give the join a **second source:
the channel's own `ring_va`.** ⚠ The ring lives in the **emulated framebuffer**, so the owner's
invariant — *no fake FB to a real GPU VA of an isolate except the scratchpad* — is directly in
that path.

**What leg C did buy, and it is not nothing**: the token question is **settled by code**. Guest→host
translation is a **plain field read** (`Channel::host_token: Option<u64>`), not a map lookup, and
**no hop reads the engine**. The C-era *"a guest token matching no host token"* is that `Option`
being `None` — *not materialized yet* — and it is already `Some` before the first doorbell.
⇒ *"Generalise the CE path to GR"* was **vacuous**: `SharedDevice::doorbell` was never
copy-engine-specific, and the missing production wiring was **one arm in one `if`**.
⚠ It **re-opens a path closed on evidence** at §16.65, hence armed, printed, controlled, default-off.

---

## 2. WHAT LANDED TODAY — `master` moved `d55187a → e758778` (first time in days)

| | |
|---|---|
| **Region kind DECLARED** | the owner's four-kind taxonomy on `Binding` (private fields, two constructors). The forbidden state — fake FB at a real GPU VA — is **unrepresentable**, not checked |
| **Channel kind DECLARED** | two axes, but **not independent**: host kind is a *function* of guest kind via `hosted_by()`; the bad cell `(Emulated, Shadow)` is unrepresentable |
| **`engineType` decoded off the wire** | first time ever — version-keyed `ChannelEngineWire`, +128 (580) / +136 (610) |
| **FB-leaf JOIN, ported + BOOTED** | `w260`, real GA106: **3 leaves `JOINED`, `placed_as_asked=true`, both directions agree over 1024 words**, negative control in the same binary |
| **R32 / J2 measured** | GPU-write → CPU-read through a described memfd, **65536/65536 bytes**, negative control fired. **Had never run on hardware.** |
| **Orphan gate triaged** | and found **partly vacuous** — see §5 |

**Evidence is committed**: `traces/boots/w260/` (18 files), `traces/real_ga106/rmladder_r32_fb_memfd_join*`.
⚠ Those commits were briefly **unreachable from any branch**; also pushed to `w260-boot-evidence`.

---

## 3. ★★★ THE MEASURED DECOMPOSITION — the memory half is done, the execution half is untouched

`w260`, three arms, all `RC=0`, `SMI_RC=0`, **0 `RmInitAdapter` failures**:

| arm | `JOINED` | `placed_as_asked` | `CUP2_RC` |
|---|---|---|---|
| off (control) | 0 | 0 | **124** |
| shared | **3** | **3 × true** | **124** |
| private (neg control) | 2 | 3 × true | **124** |

⇒ **The join works and `cuCtxCreate` did not move at all.** The pre-registered size-of-jump
prediction (*"large and discontinuous, or the model is incomplete"*) scored **ZERO**.
★ Read correctly: **the supply side is NECESSARY, NOT SUFFICIENT.** Giving the guest's framebuffer
bytes a shared home does nothing while nothing runs the pushbuffer.

⚠ **A STANDING DEBT**: the passthrough model now owes a boot where the doorbell **is** routed.
**If that one also moves by one step, the model itself is what to doubt.**

---

## 4. ⊘ WHAT IS STILL UNMEASURED — and why a green boot cannot fix it

★★★★★ **A GREEN BOOT CANNOT VALIDATE AN ORDERING FIX.** When the install *succeeds*, bind-before and
bind-after have **identical end states**. Three of the four `w260` unknowns are unmeasured **by
construction**, not by omission (pre-registered as P10 before the run):

1. **install→bind ordering** — needs **fault injection**
2. **the release path** — runs only on failure; nothing failed
3. **attempt-once `refused`** — needs two census operands in **one** leaf; got three in three leaves
4. ⚠ **the establishment copy has been VACUOUS on every leaf of every run** (`0 bytes over 0 pages`,
   twice now). The copy is the load-bearing half of the ordering-safety argument, so that argument
   **ships unwitnessed**.

⇒ **Next validation is FAULT INJECTION, not another boot.**

---

## 5. THE PROJECT'S STRUCTURAL CONDITION — read this before planning anything

**A very high ratio of proved-in-isolation to wired-together.** Eight built-and-unwired capabilities
found, five of them in one day.

⊘ **But the diagnosis "it is invisible" is WRONG** (measured): `git grep "no production caller" docs/`
returns **14 lines across 9 design docs**, several already holding findings a later lane paid to
rediscover. ⇒ **The failure is that the answer is scattered, undated, and nothing fails when it goes
stale.** The committed orphan sweep flipped 3 symbols orphan→live in **5 hours** — decaying *in the
reassuring direction*.

**The orphan gate misses 3 of 5 known orphans** (`w259-orphan-triage`, `42bee5b`): it asks **one-hop
cross-crate visibility**, not reachability. D5 = `cargo check` compiles **binaries**, so a probe
counts as production. It reports only the **outermost** orphan. And a severance via
`let (tx, _rx) = inbox()` is invisible to it **in principle**.
★ Its highest-value output is a **diff against the docs' own "zero production callers" claims**.

---

## 6. NEXT RUNGS, ORDERED

1. ⊘ ~~Give `HostGr` a passthrough server~~ — **DONE `b734995`.** See §1's update: it is **leg C of
   three**, and on its own it discharges nothing. Superseded by 1a/1b below.
1a. ★★★ **LEG A — birth the GR host channel over the guest's ring.** Two halves: give the FB join a
   **second source** (the channel's own `ring_va`, since the operand census can never present a
   ring), then point the production birth path at `alloc_channel_over_guest_ring`.
   ⚠ `guest_ring_census.rs:168` asserts that verb has exactly **one** caller — a deliberate
   tripwire. Adding a production caller turns it red; update it **deliberately**.
   ⊘ Do **not** build a GR "handler"; the owner ruled that is the thing that should not exist.
   ⚠ Ring resolution / pushbuffer reads / method decode are **DEBUG**: flag-gated, non-fatal, and
   they must **never gate** whether the doorbell is forwarded. Follow `dump_gr_pushbuffer_once`'s
   shape (*"PRINT-ONLY: advances no cursor, writes no state"*).
1b. ★★★ **LEG B — USERD adoption AT CHANNEL CREATION** (#250) — hardware-confirmed possible;
   ⚠ **RM ZEROES a caller-supplied USERD**, so adopting at first doorbell **wipes the cursor that
   caused the doorbell**. Never lazily. May need a new `hUserdMemory` hand-in arm in
   `alloc_channel_in`; if so that is a **primitive to build**, not a blocker.
3. **Fault injection** for §4's three unknowns.
4. The emulated arm must **schedule asynchronously**, not run on the vCPU thread (owner ruling);
   ⚠ measured today the trap **is** inline end to end: BQL → `regs_write` → `ring_doorbell` (RwLock
   held) → `run_submission` under the FSM mutex. ★ Polled completions need no delivery path;
   interrupt-driven ones are blocked on F6 (#235).

---

## 7. ⚠ TRAPS — every one measured today, each cost real cycles

- ★★★★★ **CHECK WHETHER THE QUESTION IS ALREADY ANSWERED.** Four instances in one session, one of
  them re-briefing a doc **the same session had committed hours earlier**. ⇒ Before dispatching:
  `git log --all --oneline --grep=<topic>` in **both** trees, **then** read the doc's STATUS block.
- **The bench**: `vh` = host (GA106 / 580.159.04), `vg` = guest. `cargo` at `/root/.cargo/bin/cargo`,
  **not on the non-interactive PATH**. `nvktap0` must pre-exist (it does). **Code arrives as GIT
  BUNDLES into a fresh `/workspace/kayfabe_w<NNN>` tree — there is no remote.** Guest needs ~38 s.
- **ENOSPC produces a FALSE GREEN, not a failure.** Always grep output for
  `No space left on device` / `LLVM ERROR`. ⊘ Never `$?` after a pipe — a lane reported clippy green
  when `$?` was `head`'s. ★ A COUNT and a STATUS must come from the **same invocation**.
- **The orphan gate MUTATES THE WORKING TREE** — it cannot run beside another cargo job.
- `git checkout <file>` on an **uncommitted** file silently discards every edit to it.
- **`143` ≠ `124`**: `143` = the job was killed; `124` = the **launcher** timed out while the job kept
  running. A nonzero exit from the thing that *started* the work says nothing about the work.
- `pgrep -x qemu-system-x86_64` can **never** match (comm truncates to 15 chars);
  `pgrep -f <literal>` **always** matches the asker. Use `qemu-system-x86` + the bracket trick + `ss`.
- Status codes in these logs are **HEX**. `0x56` is the forgiven `NOT_SUPPORTED`.

---

## 8. HONEST LEDGER — my own reliability this session

**Nine of my claims were refuted by measurement, and every refutation was worth more than the work it
replaced.** The pattern is one-directional: **I assumed things were missing when they were already
built** — the `RmInitAdapter` fork (fixed hours earlier, and I had *read* the fix), the doorbell token
(settled in a doc saying the opposite), the supply side (booted on hardware, and I left its branch off
an integration list), the watchdog audit (closed three days earlier).

⇒ **Weight my estimates of remaining work as upper bounds.** And when a lane returns *"already
answered"*, that is the most expensive possible outcome — it burns a full budget and returns no new
fact, while looking like diligence.

Related, all in `/root/.claude/projects/-workspace-nvidia-gpu-passthrough/memory/`:
`the_join_landed_and_the_wall_did_not_move`, `the_orphan_gate_asks_visibility_not_reachability`,
`check_whether_the_question_is_already_answered`, `the_row_and_the_store_are_two_facts`,
`rm_takes_a_guest_userd_and_zeroes_it`, `the_emulated_axis_is_bounded_and_the_watchdog_is_the_case`,
`the_c_did_content_passthrough_not_doorbell_passthrough`, `the_join_is_ruling_4_arriving`,
`a_second_source_of_truth_beside_a_complete_value`, `a_blocker_i_declared_was_already_fixed`.
