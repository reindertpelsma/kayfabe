# The LLM lane — state, provisioning recipe, and what bears on the wall

**STATUS: LIVE (2026-09-13, at `ae364d82` / w660).** Written to answer the owner's direct
question *"does the LLM pass?"* from evidence rather than from a stale memory. Supersedes
nothing; it is the first doc on this lane's *operational* state as opposed to its defects.
⚠ **This doc contains NO new boot.** Every "measured" row below is a re-reading of an existing
artefact; every "argued" row is code reading. The split is stated per claim, on purpose.

> ## ★★★★★ THE ONE-LINE ANSWER
> **The LLM does not pass, and it has also not been asked.** The lane is **UNPROVISIONED** on
> the current bench box, so no run since w528 exists in either direction. The last real
> measurement (w527/w528, 2026-09-12) is **`0 tokens`**.
>
> ★★★★★ **AND ONE NEW THING IS MEASURED HERE, WITHOUT A BOOT.** w656 wires sync point (3) at
> exactly the mechanism w415/w425 blame for the LLM — and its guest-RAM drain **acquires a
> target ZERO times across six consecutive boots**, the last of them the current HEAD boot,
> while firing 233 times. The arm is `Drain`; the behaviour is a bounded sample. The only
> production callers pass `seen = None`, and `facts` is in scope at the call site. **§4.2.**

---

## 1. Is the lane provisioned? — NO, and the check that said so was looking at the wrong machine

`[measured 2026-09-13, vast 50835305]`

⊘⊘ **`/opt/llm` IS A PATH INSIDE THE GUEST, NOT ON THE HOST.** `provision_guest_llm.sh`
installs the venv, the torch wheel and the model **into `guest.qcow2`**, over a plain QEMU with
slirp. Nothing is ever written to `/opt/llm` on the host. So `ssh vh 'ls /opt/llm'` returns
`No such file or directory` on a **fully provisioned** box just as loudly as on a bare one —
it answers a question nobody asked, and its answer reads as a verdict.
⚠ Exactly [[the serial log is NOT where the driver's output is]]: every signal says the
evidence is there; only looking at the right filesystem shows it is not.

**The evidence that actually settles it, all host-side and GPU-free:**

| witness | value | reading |
|---|---|---|
| `/workspace/bench/llmprov_serial.log` | **absent** | `provision_guest_llm.sh` writes this on every run. It has **never run on this box.** |
| `/workspace/bench/llmprov.pid` | **absent** | same |
| any `*llm*` file in `/workspace/bench/` | **none** | no LLM boot has ever been captured here |
| `provision_box.sh` → does it call the LLM provisioner? | **no** | `grep -i llm` over all three `provision_*.sh` returns nothing. The LLM lane is **not part of the standard box recipe** — it is a separate, explicit step, and nothing reminds you. |
| box age | provisioned `02:40`, guest image built `02:41`, ~11.6 h old | a fresh box, so nothing survives from an older one |
| `/root/kayfabe` revision on the box | `54c51ba6` (w659) | the tree is current; only the guest's *contents* are missing |

⇒ **(a) The lane cannot RUN. It is not (b) provisioned-and-failing.** Any LLM verdict produced
on this box today would be `(E) UNMEASURED` — the false green
[[the_llm_lane_needs_its_own_guest_provisioning]] records, where `hook finished: rc=0` sits over
a workload that never executed.

★ Independently corroborated: `THE_TEN_GOALS_STATUS.md:298` (written today at w660) already says
*"lane is **unprovisioned** on this box"*. Two observers, two routes, same answer.

### ⊘ A misremembered filename cost part of this session, and it was not misremembered

`scripts/bench/provision_guest_llm.sh` **exists, is committed, and is at HEAD** (5906 bytes,
present on the box too). The search that concluded *"I could NOT find a `provision_guest_llm.sh`
despite memory naming one"* looked in `archive/nvkvm/` — the C-era tree — and in
`scripts/mode2_diag/`. The file is in **`scripts/bench/`**, the Rust-era location.
⇒ ★ **Agent memory was RIGHT and the search was wrong.** Before recording *"memory named a file
that never existed"*, run `git ls-files | grep -i <name>`: it is one command and it is decisive.
This is the mirror image of [[check_whether_the_question_is_already_answered]] — the question was
answered, in the obvious place, under the exact name memory gave.

---

## 2. The provisioning recipe — fully recovered, and it is cheap

`scripts/bench/provision_guest_llm.sh`, `[measured w526]` **~7 minutes** end to end.

| | |
|---|---|
| **model** | `Qwen/Qwen2-0.5B-Instruct` (override with `LLM_MODEL`) — ~1 GB |
| **runtime** | **PyTorch + transformers**, NOT llama.cpp and NOT vLLM |
| **CUDA deps** | the `cu124` torch wheel from `download.pytorch.org` (~2.5 GB down), plus `transformers` + `accelerate`; `python3-venv`/`python3-pip` from apt. venv at `/home/ubuntu/llmvenv` |
| **where** | inside `guest.qcow2`, over a **plain** QEMU (slirp, `hostfwd 2222`), **not** the nvkvm device |
| **model cache** | pre-downloaded to `/opt/llm/hf` so the graded run is not a network test |
| **runner** | `/opt/llm/run_llm.py`, one greppable grading line per fact |
| **self-assertion** | ends with a **CPU control generate**; refuses to finish (`exit 4`) unless it passes, so a later GPU failure is attributable to us and not to a bad wheel |

⚠ **Two traps it already encodes, both paid for:** `-cpu host` is **required** (without it QEMU
picks `qemu64`, NumPy dies on the missing `X86_V2` baseline at the first numeric import, far from
the cause); and it **refuses to start if a QEMU is already running**, because the bench is
serialized.

**The graded run** is a separate step: boot with
`POST_CAPTURE_HOOK=scripts/bench/w392_llm.sh` (or the wrapper `w409_llm_boot.sh`, which exports
the arm set including **`KAYFABE_FB_JOIN=shared`** — load-bearing, see §4).
`w392_llm.sh` grades the GPU text against a **same-boot CPU run of the same model**, so a token
count alone cannot pass: `(F)` FORGED-PASS is a pre-registered outcome, because
`LLM_TOKENS=16` has passed over garbage text before.

### What this doc adds under `scripts/` (no `.rs` touched)

- **`scripts/bench/llm_lane_preflight.sh`** — NEW. Answers *"is the lane provisioned?"* **without
  taking the GPU or booting anything**, and is safe to run beside a live bench boot. Three
  states, three exit codes: `0` PROVISIONED, `10` UNPROVISIONED, **`20` UNKNOWN**. ⊘ *"I could
  not tell"* is deliberately **not** zero — a preflight that exits 0 when it checked nothing is
  the very false green this lane has already been burned by. It prefers the live guest
  (authoritative) and falls back to a receipt.
- **`scripts/bench/provision_guest_llm.sh`** — three additive changes:
  - writes **`/workspace/bench/llm_lane.receipt`** (model, date, CPU-control result, kernel
    before/after, import versions, revision). The point of the receipt is that it is readable
    **with the guest powered off**; without it, answering *"is the lane provisioned?"* costs a
    ~25-minute boot to learn a fact that never changes. The preflight compares its mtime against
    `guest.qcow2`, so **a rebuilt image invalidates the receipt automatically**.
  - **pins the guest kernel** (`mask unattended-upgrades`/`apt-daily*`, `apt-mark hold linux-*`)
    *before* `apt-get update`. `[measured w383]` the guest upgraded itself `6.8.0-138 → -139`
    mid-campaign and the next boot said `modprobe: FATAL: Module nvidia not found`. This
    provisioner runs the exact trigger. It also compares `uname -r` before/after and **exits 6**
    if the kernel moved, because that state is UNMEASURED, not a device failure.
  - **asserts the import**, not the pip exit status — `pip -q install … | tail -3` loses its
    status to the pipe ([[a_check_that_reports_is_not_a_check_that_gates]]), and a wheel that
    unpacked but cannot import is a state pip calls success.

⊘ **Neither was run.** Both need the bench idle; a boot was in flight throughout.

---

## 3. What "the LLM wall" is — a LADDER of causes, not one defect

Reading these as one bug is the confusion this section exists to prevent. Each rung was real,
each was fixed or dissolved, and the next one appeared behind it.

| rung | the wall, as measured then | status |
|---|---|---|
| w349 | four **cudart-only controls** (`0x20809009`, `0x20809001`, `0x20809064`, `0x20802209`), each *independently* fatal — proven by refusing them in-band on bare metal | addressed |
| w377 | our own refusal predicate | addressed |
| **w383** | ★ **THE LLM PASSED** — `LLM_OK=1 LLM_TOKENS=16`, 0 Xids, stock guest driver, on `KAYFABE_DOORBELL_ASYNC=off`. Cause of the unblock: a dirty-gate default that had been **unreachable since it was written** | the only known green |
| w415 | **unpinned guest-RAM operands**: `guest_ram=13313` rows of 13 348 in the app VAS, `pins=0`; a real CE reads them and finds no PDE | ⊘ open |
| w425/w426 | ★★★ it is a **RACE**: the pin and the fault land in the **same second** (pin took 383 ms of it). The boot issues **ZERO** map RPCs ⇒ *"no synchronization point saves it"*; prescribed fix = the owner's **GPGA reservation** (eager, global backing) | ⊘ open; GPGA is `STATUS: LIVE` as a design, **not implemented** |
| w467 | the async doorbell arm — recorded at w383 as *regressing* the LLM — became the **default**. A known trade coming due | in force |
| **w527/w528** | **the last measurement.** `0 tokens`; CUDA fails at the **4×4 matmul**, before any weight transfer, with `RuntimeError: CUDA error: CUDA-capable device(s) is/are busy or unavailable`; **two `Xid 31` per python process**, both `FAULT_PDE`. Same on `on` **and** `nocoalesce` ⇒ ⊘ w383's coalescing explanation **REFUTED**. The `off` control **no longer boots the adapter at all** | ⊘ open |

⚠ **Two things w527/w528 established that must not be re-derived:** the same boot's **CPU
oracle produced 16 tokens**, so the workload is sound and the failure is ours; and the guest's
`status=0x56` refusals in that log are the **modprobe-time baseline**, present identically in
boots where `cup8` is bit-exact green ([[a_refusal_needs_a_negative_control]]).

⊘ **The lane is also not blocked on traps.** `VCPU-BLOCKING none`, `inline_exceptions=0`,
`worst_trap ≈ 4.5 ms`. ⚠ But that census is over a workload that produced zero tokens, so
*"the LLM lane is slow-trap free"* may **not** be claimed — [[every_row_verified_over_zero_rows]].

---

## 4. Does w656 bear on the wall? — ★ IT IS AIMED AT EXACTLY THE RIGHT MECHANISM, AND APPEARS TO MISS BY ONE ARGUMENT

### 4.1 Why it is the right target (ARGUED, from code + the recorded traces)

w415/w425 name the LLM's root cause precisely: **guest-RAM operand rows that our publication
never backs, raced against a real engine.** And w415 names *why* the pass that would back them
never runs — *"the pin pass never runs in ANY run: it is gated on the VAS being **doorbelled**,
and `drain_asked=` is empty everywhere."*

The guest's own signal that a mapping is ready is the **UVM kernel CE channel's completion**;
after it, the guest rings a real engine at those mappings. The tree says so itself
(`crates/kayfabe-rt/src/ceutils.rs:627-643`): *"the work it serves for UVM's kernel channel IS
page-table writes, and the mapping those writes describe must reach the host GPU before the
guest — released by the completion — rings an engine at it."*

**w656 (`639024fb`) inserts `refresh_page_tables` + `publish_vas_rows(Drain)` at exactly that
point** — inside `try_ce_submission`, after the CE run, **before the deferred release words are
written** (`shim.rs:8874-8885`, then `:8918`). It even picks `Drain` over `Publish` for the
stated reason that *"`Publish` does framebuffer leaves and nothing else, and silently disables
the only pass that pins guest-RAM rows."*

⇒ ★ **This is the first mechanism in the tree that gives w415's pin pass a trigger that can
actually fire**, and it is *not* refuted by w426. w426's *"sync point (3) has nothing to hold"*
was measured against **RM interface calls** (*"`uvm_mmu.c` makes zero RM interface calls"*);
w656 hooks the **channel's completion**, not an RM call. ⚠ **Different implementation of the
same numbered sync point — so w426's ruling does not transfer.**
[[a_rulings_date_is_part_of_the_citation]]: ask what architecture it decided against.

### 4.2 ⊘⊘⊘ WHY IT MISSES — the drain is ARMED WITH NO TARGET, **ON EVERY BOOT, MEASURED**

`[★ MEASURED 2026-09-13 on vast 50835305 — six consecutive boots, w647a → w660a, the last of
them the CURRENT HEAD boot. This section was written as an argument and was upgraded by the
§5.1 grep before this doc was filed.]`

```
                       CE-LOCAL-REFRESH   "DRAIN ARMED BUT NO TARGET"   "★ DRAIN TARGET = proc"
run_w660a_qemu.log            233                    1812                        0
run_w658a_qemu.log            232                    1811                        0
run_w657a_qemu.log            232                    1813                        0
run_w653a_qemu.log            237                    1579                        0
run_w651a_qemu.log            168                    1530                        0
run_w647a_qemu.log            230                    1578                        0
```

⇒ ★★★★★ **Sync point (3) FIRES — 233 times in the HEAD boot — and the drain acquires a target
ZERO times, in any boot, ever.** The pass that w415 named as the LLM's missing mechanism is
selected on every firing and given nothing to work on every time.
★ `NOT ARMABLE: KAYFABE_FB_JOIN` is **0** in all six ⇒ the framebuffer join *is* armed, so this
is **not** the second, independent inertness described in §6. The publish half runs; the drain
half has no target. One cause, and it is the argument below.

**The mechanism, read at `ae364d82`:**

The call is `ctx.publish_vas_rows(token, **None**, w)` (`shim.rs:8881`). The second parameter is
`seen: Option<&CeChannelFacts>`, and it is **the only thing that names which VAS to drain**
(`shim.rs:12440`):

```rust
let drain_target = if self.vas_publish.drains_doorbelled_vas() {
    seen.and_then(|f| f.vas_pdb.map(|p| (f.proc, p)))
} else { None };
```

With `seen = None` the match falls to `(true, None, _)`, whose own text is the verdict
(`shim.rs:12448-12452`):

> `⊘ DRAIN ARMED BUT NO TARGET: this doorbell resolved NO channel facts, so the VAS it is about
> has no name here. Every VAS got the bounded sample — ⚠ THIS LINE IS NOT A DRAIN`

⇒ **The arm is `Drain`; the behaviour is the bounded `VAS_PINRATE_ROWS`-row sample.** The
branch that would print `★ DRAIN TARGET = proc=… pdb=…` (`shim.rs:12470`) is **not reachable
from production**: all **four** call sites pass `None` (`shim.rs:5448`, `:5543`, `:5646`,
`:8881`) — and the six boots above are the hardware confirming it, `0` for `0`.
★ Note the counters agree with the code exactly: **1812 "no target" lines** against **0 targets**
is what four `None`-passing call sites predict, so the reading is closed, not merely consistent.

★★★ **And `facts` is in scope at that exact call site** — it is bound at the top of
`try_ce_submission` (`shim.rs:8309`) and used two lines below the call, in the log line that
prints `facts.proc.0` and `facts.chan.0`. So `Some(&facts)` was available and `None` was passed.

⊘ **This is [[the_working_set_gate_is_vacuous_in_production]] again, verbatim** — w476: *"the
gate meant to stop it has never executed because the only production caller passes `&[]`."*
Here the only production callers pass `None`. ⚠ Note how it survives review: the call site's
comment argues correctly and at length for `Drain` over `Publish`, so the **arm** is audited and
the **argument that selects its target** is not. An audited decision one line above an unaudited
one reads as an audited block.

### 4.3 The other three changes since w527, ranked

| change | bears on the LLM? | argued / measured |
|---|---|---|
| **w556** `dbeb7b7c` — a VA space is keyed by its **RM object**, not its page-directory base | ★★ **plausibly, and it is the best-fitting signature.** w555 attributes `FAULT_PDE` to 11-of-12 VA spaces sharing the key `Pdb(0)`; **both** of w527's faults are `FAULT_PDE`. It landed **21:12 on 2026-09-12, AFTER the w527/w528 boots** ⇒ the LLM has never run on a tree with it. ⊘ But w556's own text: *"a rootless space still cannot be SWEPT … it is now **nameable and addressable, not populated**"* — so it removes the silent cross-space resolve, not the missing rows | argued; **w556 was never booted** (its commit records tests only: 1213 passed, unchanged both ways) |
| counter page, BAR1/BAR2 at zero traps, DoorbellTable (w596→w659) | ⊘ **unlikely.** These are trap-cost work on a lane already measured trap-healthy at w527 (`VCPU-BLOCKING none`) | argued |
| `KAYFABE_DOORBELL_INLINE` default → `on` (w659) | ⊘ **unknown, and a NEW risk.** It claims only `Route::Passthrough` tokens, so it should not touch the emulated UVM channel — but its safety is explicitly *"one-way dependent"* on sync point (3), which §4.2 argues is half-inert | argued |

### 4.4 ⚠ The verdict, with the split stated

- **MEASURED:** the lane is unprovisioned; w527/w528's `0 tokens` / two `FAULT_PDE` Xids /
  4×4 failure; w556 and w656 both landed after those boots; **neither has ever been run against
  the LLM**; every `(P)` cited for w656–w660 is the **raw client**, not the LLM.
- **ARGUED (all of §4.1–4.3):** that w656 targets the right mechanism; that its drain is
  target-less; that w426's refutation does not transfer; that w556 fits the `FAULT_PDE`
  signature. **None of this is a boot.**
- ⊘ **And one thing neither supports:** the w527 stop is at the **4×4 matmul** with
  *"CUDA-capable device(s) is/are busy or unavailable"* — an **init/device-availability** class
  failure, before any operand at scale. A publication-ordering fix does not obviously reach it.
  ⇒ It is entirely possible that w656 and w556 both help the *w415 race* and the LLM still
  fails at the same early point for an unrelated reason. **Do not let the elegance of the §4.1
  argument pre-commit the reading of the next boot.**

---

## 5. The single cheapest experiment that settles it

★ **Two greps and one boot — and the greps come first, because they need no GPU at all.**

1. ~~**⊘ FREE, DO THIS FIRST — falsify §4.2 without booting anything.**~~ ★ **DONE, 2026-09-13
   — §4.2 is CONFIRMED**, see the table there. Kept here because it is the reusable instrument:
   in any boot log from w656 onward (`/workspace/bench/run_*_qemu.log` on the bench box):
   ```
   grep -c 'CE-LOCAL-REFRESH'                  # did sync point (3) fire at all?
   grep -o 'DRAIN ARMED BUT NO TARGET'         # ⇒ §4.2 CONFIRMED, the drain is inert
   grep -o '★ DRAIN TARGET = proc=[0-9]*'      # ⇒ §4.2 REFUTED, it really drains
   grep -o 'NOT ARMABLE: KAYFABE_FB_JOIN'      # ⇒ the publish half was inert for a 2nd reason
   ```
   These four lines decided §4.2 **in seconds, on logs that already existed**, before anyone
   spent 30 minutes provisioning. ⚠ w651a/w653a/w658a/w660a logs are **not in the repo** — they
   live in `/workspace/bench` on the box, which is why this had to be asked of the box and not
   of `traces/`. ⇒ ★ **Run this grep on every boot from now on**: a sync point that fires 233
   times and does its job 0 times is invisible in a `(P)` verdict.

2. **Then, one provision + one boot.** `provision_guest_llm.sh` (~7–20 min, bench idle), then a
   single boot with `w409_llm_boot.sh`. ★ **Grade it on `W392_MINMM_SUM`, not on the token
   count** — the 4×4 discriminator runs **before** the model, costs seconds, and is exactly where
   w527 stopped:
   - `MINMM_SUM=64` (was `ABSENT`) ⇒ **the early wall moved** between w528 and HEAD. Re-grade
     end-to-end; the LLM is back in play.
   - `MINMM_SUM=ABSENT` with the same `device(s) is/are busy or unavailable` ⇒ the wall is the
     **init/availability class**, and w656 **and** w556 are both cheaply **exonerated** — stop
     arguing about publication ordering and chase the device open.
   ⊘ Read `W392_XIDS=before/after-minmm/after-llm` alongside it: *"failed AND faulted"* and
   *"failed cleanly"* are different diagnoses, and only the delta separates them.

⚠ **Before that boot, run `scripts/bench/llm_lane_preflight.sh`.** If it exits `10` or `20`, the
boot cannot produce an LLM result and the 25 minutes are spent to learn nothing.

---

## 6. What I could not determine

- ⊘ **Whether the guest image contains `/opt/llm`.** The guest was unreachable on the tap
  throughout (`No route to host`; `nvktap0` **DOWN**) during a live boot, and inspecting a
  `guest.qcow2` that a running QEMU holds open is not safe. The conclusion in §1 rests on
  host-side provenance (no `llmprov_serial.log`, no LLM artefacts, a box 11.6 h old, the
  provisioner absent from the standard recipe) — **strong, convergent, and still indirect.**
  `llm_lane_preflight.sh` closes this permanently from the next provisioning run onward.
- ★ ~~Whether sync point (3) has ever actually fired.~~ **ANSWERED: yes, 233× in the HEAD boot,
  and its drain never acquires a target.** §4.2.
- ★ ~~Whether `KAYFABE_FB_JOIN=shared` was set on the w656–w660 boots.~~ **ANSWERED: it was** —
  `NOT ARMABLE: KAYFABE_FB_JOIN` is `0` in all six boots. ⇒ the second, independent inertness
  does **not** apply; §4.2's conclusion rests on the `seen = None` argument alone.
- ⊘ **Whether fixing §4.2 would change the LLM's outcome.** It is a defect in a mechanism aimed
  at the LLM's recorded root cause; that is **not** the same as being the LLM's blocker. The
  w527 stop (§4.4) is at a stage this does not obviously reach, and the lane must be provisioned
  before anyone can say. ⚠ Do not let a real, measured defect become an assumed cause —
  [[a_falsifier_that_cant_tell_THE_blocker_from_the_ONLY_blocker]].
- ⊘ **Whether GPGA — w425's prescribed fix — has been implemented.** `gpga_is_one_reserved_object.md`
  is `STATUS: LIVE (2026-09-10)` as a *design*; nothing in the w596→w660 commit range implements
  it, but I did not audit the allocator to be certain.
