# S1 — may the host GR engine execute the guest's pushbuffer?

**Rung:** `w227` / `master`, based on the merge of `origin/completion-observer` (`c5f251d`).
**Question, from the brief:** open `shim.rs`'s `Route::NotACopyEngineChannel` refusal so the
real host GR engine runs `cuCtxCreate`'s pushbuffer and writes `0x2_0440fff0` itself.
**Answer: NO — and the thing that must exist first has a name, a shape, and now a number.**

The brief anticipated this outcome and sanctioned it: *"If the honest conclusion is 'this
cannot be opened safely without X', name X and stop; that is a passing result."* This
document names X, and — the part that was not asked for — **measures how far away it is, at
the guest's own addresses**, so the next rung inherits a count rather than an argument.

---

## 0. ★★★★★ WHAT I REFUTED FIRST, INCLUDING THE BRIEF

### 0.1 ⊘⊘ REFUTED: *"port the C's cont.34 FIX B"* — the C's fix presupposes an architecture we deliberately do not have

The brief's step 3 is precise and correctly cited: `how_the_c_passed_the_gr_wall.md` §4.3,
*"one-shot `GPFIFO_SCHEDULE` of the GR TSG, keyed `(client, tsg)`, in the doorbell-exec path.
Without it the GR channels ring (`GP_PUT=1`) and the host never consumes (`gp_get` stuck 0);
with it `gp_get` advances 0→4 and all 16 pool semas advance."* Every word of that is true of
the C.

⊘ **And it is not portable, because the two ports do not mean the same thing by "the
channel".** The C runs Mode 2: the *guest's own* ring, in the *guest's own* VA space, on the
real GPU — §4.1-4.2 of that same document say so, *"host-backed the pool page … resolved the
GR VAS PDB"*. `GPFIFO_SCHEDULE` was the last missing step because everything else was already
the guest's. In kayfabe the host channel is **ours**:

| | the C (Mode 2) | kayfabe |
|---|---|---|
| the ring the host engine reads | the **guest's**, host-backed | **ours**, allocated by `rm.rs:2439 alloc_channel_at` — 128 KiB device-local, GPFIFO at `+0x1000` |
| the VA space it is bound to | the **guest's** GR fvas | a host VAS we created |
| what puts methods in it | the guest, directly | **nothing** — `device.rs:1793-1797` says so in its own words |

`[verified at `c5f251d`]` we already **allocate** host GR channels — `engine_type_for`
(`isolate-host/src/rm.rs:1751-1766`) maps `GrCompute | GrGraphics → ENGINE_TYPE_GRAPHICS`, and
`w226b`'s log shows eight `class=0xc7c0 → FORWARDED engine=GrCompute host_object=0xcafe…
materialized_channel=true`. We can already schedule a TSG (`rm.rs:2485`). ⇒ **Scheduling is
not what is missing.** Scheduling an empty host ring makes the host engine consume nothing,
correctly and forever. The missing thing is what would be *in* the ring, and that is the
whole of §1-§3 below.

★ The durable form: **a fix is portable only across ports that agree about what the fix
operates on.** `how_the_c_passed_the_gr_wall.md`'s own closing lesson is *"a ruling's date is
part of the citation"*; this adds **a ruling's architecture is part of the citation too.**

### 0.2 ⊘ REFUTED, and it is mine: *"the address plane was the gap"*

`the_ring_is_read_and_the_operand_refuses` and `w226b` between them left the reading *"the
address table binds the guest's poll word, so addressing is done and execution is the only
thing left."* `w226b` proved **one** address binds. §3 measures the rest, and the honest
statement is that the completion address was the **easy** one.

### 0.3 ★ The brief's own framing of the boundary is RIGHT and is sharper than it states

The brief says *"S1 IS A HOSTILE-GUEST BOUNDARY. Design before you open it."* §2 finds that
the boundary is not merely present but **singular**: there is exactly one containment surface
available for GR execution, and it is the VA space. Every other candidate — a method
allowlist, a class gate, an operand check — is defeated by one method in the measured stream.

---

## 1. ★★★ WHAT THE GUEST ACTUALLY SUBMITS — decoded, method by method

`[measured 2026-08-10, boots `w226b_534e1b3_cup2` and `w227a_c5f251d_control`, token
`0x00000007`, 86 methods, 864 bytes, ×8 `GrCompute` channels]`. Decoded against
`ogkm-580: clc7c0.h`:

| # | method | value | what it is |
|---|---|---|---|
| 000 | `SET_OBJECT` sub 1 | `0xc7c0` | `AMPERE_COMPUTE_B` |
| 002-003 | `SET_SHADER_SHARED_MEMORY_WINDOW_A/B` | `0x7d1e_e900_0000` | a 64-bit base **the guest chose** |
| 004 | `SET_SPA_VERSION` | `0x806` | SM 8.6 — GA106 |
| 005-068 | `SET_CWD_REF_COUNTER` ×64 | `0x5403f … 0x54000` | CWD reference counters, descending |
| 070 | `SET_VALID_SPAN_OVERFLOW_AREA_A/B/C` | `0x2_0000_0000`, len `0xa8000` | a 672 KiB span the engine may write |
| 072-077 | `LOAD_MME_INSTRUCTION_RAM_POINTER` / `…_RAM` ×15, then ×24 | **39 dwords** | ★★★★★ **guest-authored microcode** |
| 078 | `SET_OBJECT` sub 4 | `0xc7b5` | `AMPERE_DMA_COPY_B` — a second class in one stream |
| 079-081 | `SET_TEX_HEADER_POOL_A/B/C` | `0x100_0000_0000`, max index `0xfffff` | ★ **32 MiB** of texture headers the engine reads |
| 082-084 | `SET_TEX_SAMPLER_POOL_A/B/C` | `0x100_0200_0000` | sampler pool |
| 085 | `SET_REPORT_SEMAPHORE_A/B/C/D` | VA `0x2_0440_fff0`, payload `1`, `AWAKEN=0`, `FOUR_WORDS` | the word `cuCtxCreate` polls |

★★ **This settles a question two rungs old.** `dump_gr_pushbuffer_once`'s own doc says the
traffic is *"either user compute or golden-context initialisation — two completely different
rungs that the channel's class cannot separate."* It is **golden-context initialisation**:
MME microcode load, pool bases, cache configuration, and a release. There is **no launch
method and no QMD**, and `how_the_c_passed_the_gr_wall.md` §4 already ruled that this is *"not
a defect and not the blocker"* — GR context-init *is* methods plus a report semaphore.

---

## 2. ★★★★★ WHAT OPENING S1 WOULD ADMIT — and the one boundary that survives

To make the host GR engine execute these bytes, the bytes must reach a host ring bound to a
host VA space. From that instant, **every address in the table above is dereferenced by real
silicon in whatever VA space that channel is bound to.** Three admissions follow, in
increasing order of how badly they break the obvious mitigations.

### 2.1 A guest-chosen 64-bit GPU-engine **WRITE**

`SET_REPORT_SEMAPHORE` is a *write*: the engine stores the payload (and, with `FOUR_WORDS`, a
16-byte record) at a VA the guest's own bytes named. `SET_VALID_SPAN_OVERFLOW_AREA` is a
672 KiB region the engine may write. ⇒ opening the route without a closed VA space is an
**arbitrary GPU-engine write primitive at guest-chosen addresses**.

### 2.2 A guest-chosen 64-bit GPU-engine **READ**, at scale

`SET_TEX_HEADER_POOL` with `MAXIMUM_INDEX = 0xfffff` is 1 048 576 texture headers × 32 B =
**32 MiB** the engine will fetch from, at a base the guest picked, in a different aperture
(`0x100_…`) from the completion (`0x2_…`) — three unrelated bases in one 86-method stream.

### 2.3 ★★★★★ `LOAD_MME_INSTRUCTION_RAM` — and this is the one that decides the shape of any answer

39 dwords of **guest-authored microcode** for the **Macro Method Expander**, a programmable
unit in the GR front end whose **output is methods**.

⇒ **A method-level allowlist over this pushbuffer cannot be sound.** Whatever set of methods
an inspector approves, the MME program the same pushbuffer installs can emit methods that were
never in it. The inspector is not wrong about the bytes; it is answering a question about a
stream that the stream itself rewrites. ⊘ This is the `refuse_by_name_means_the_name_is_true`
failure one level up: a gate named *"only these methods run"* would be **false by
construction**, and a false gate is worse than none because downstream reasoning cites it.

### 2.4 ⇒ THE BOUNDARY IS THE VA SPACE, AND ONLY THE VA SPACE

| candidate boundary | verdict |
|---|---|
| method allowlist | ⊘ **unsound** — §2.3, the MME rewrites the stream |
| class gate (`0xc7c0` only) | ⊘ necessary, not sufficient — every admission above is *within* `0xc7c0` |
| operand range check at decode | ⊘ unsound for the same reason as the allowlist, plus it must be exhaustive over 17 address registers **and** whatever the MME emits |
| **a CLOSED guest-image VA space** | ★ **the only one that holds**, because it constrains the *engine*, not our reading of the bytes |

★ This is a genuinely good outcome, not a dead end: it means the containment argument does not
depend on our understanding of the compute class being complete — which it never will be.

### 2.5 ⚠ AND OPENING IT NAIVELY WOULD CREATE THE SECOND WRITER STEP 2 JUST OUTLAWED

`alloc_channel_at` places our own ring, USERD and semaphore **inside the VA space the channel
is given** (`rm.rs:2405-2438`; `ce_pushbuffer` releases at `ring_va + SEMAPHORE_OFFSET`,
`rm.rs:538/3358`). If a guest GR channel were bound to a VA space that also contains those
objects, the guest could aim its own `SET_REPORT_SEMAPHORE` at **our** completion semaphore.

⇒ That is a **second writer to our own completion word, authored by the guest** — precisely
the M5.38 shape whose absence this rung made structural
(`tests/tests/single_writer_census.rs`, `crates/kayfabe-rt/tests/ui/mint_a_release.rs`). ★ The
two halves of tonight's rung meet here: **Step 2's invariant is one of the reasons Step 3 must
not be opened yet**, and it would have been broken by the fix rather than by a later refactor.

---

## 3. ★★★★★ HOW FAR AWAY IS X? — the address census, measured

Arguing that a closed VA space is needed says nothing about how far we are from one. So this
rung built the instrument instead: `completion_watch::decode_address_operands` +
`ceutils::census_gr_addresses`, printed as `GR-ADDRESS-CENSUS`, one line per `GrCompute`
channel, **off the same ring read and the same walk** the observer already performs. It
resolves every address the pushbuffer names through the address table and reports where each
one landed — `GuestRam`, `Framebuffer` or `Unresolved`, with the walk's own name for the
refusal.

⊘ **It is print-only and it is not a step toward execution.** Knowing where an address lands
is not permission to let an engine dereference it. It decodes, resolves, and reports; it
produces no plan, lowers nothing to a host verb, and makes nothing executable.

### 3.1 The result

`[measured 2026-08-10, boot `w227b_<rev>_census`, `KAYFABE_ISOLATES=real`, CE executor
`local` — see §5]`

### 3.2 ★ The instrument bit me before the boot did

`SET_REPORT_SEMAPHORE_A_OFFSET_UPPER` is `7:0`, so `decode_report_semaphore` masks `_A` with
`0xff` — correct, and correct **only for that method**. The census was first written by
generalising it. `SET_TEX_HEADER_POOL_A_OFFSET_UPPER` is `16:0`, and the measured value
`A = 0x00000100` masks to **zero**: three of the five operands the guest names would have been
reported at VA `0x0`, which reads as *"the guest named nothing there"*. The unit test carrying
the measured `cuCtxCreate` stream went red on the first run and the mask became a per-row
column derived from the class header.

⇒ ★★ **The generalisation of a correct decoder is not a correct decoder**, and the wrong
answer here was `0` — the value that looks like absence. Recorded because
`a_wall_that_can_carry_no_name` is the same failure with the polarity flipped.

---

## 4. ⇒ X, STATED SO IT CAN BE BUILT

**X = a CLOSED GUEST-IMAGE VA SPACE for host GR channels**, with four properties, none of
which exists today:

1. **Complete** — every VA this guest's compute pushbuffers can name resolves. §3 measures
   the current shortfall.
2. **Closed** — *nothing that is not this guest's memory is mapped in it.* In particular the
   isolate's own ring, USERD and semaphore objects must **not** be reachable (§2.5), which
   means `alloc_channel_at`'s "put our control structures in the VAS we were handed" shape has
   to change for GR channels.
3. **Faulting** — an unmapped VA raises an MMU fault rather than aliasing anything, and that
   fault is contained to this channel's TSG. ⊘ **`[NOT MEASURED]`** whether a GR MMU fault on
   this bench is contained to one channel or takes the host GPU context with it;
   `scripts/bench/gpu_fault_containment.sh` exists and this question has not been asked of it.
4. **Per-guest** — one such space per guest process, never shared, or §2.1's write primitive
   crosses tenants.

**The named dependency for (1) and (2):** the guest-RAM crossing. `map_guest_ram` arm B
(`isolate-host/src/rm.rs:2816-2840`) already builds an `OS_DESCRIPTOR` over guest pages, and
`GuestRamGrant` landed at `28b7bb2` (R29: placed at `0x301400000` as asked, isolate mapping
reads `0x9a114001`). ⇒ **the primitive exists; the VA-space discipline built on it does not.**
The successor question — whether GR context buffers force a second crossing — is
`mode2_fb_crossing_question.md`, asserted here by name and not built.

⊘ **Do not read this as "just build a big VA space".** Property 2 is the hard one and it is a
*subtraction*: the value of the isolate today is partly that our objects and the guest's live
in one place. GR execution requires them not to.

---

## 5. THE FALSIFIER — written BEFORE the boot

| arm | what the log shows | reading |
|---|---|---|
| **A — a mixed census** (★ predicted) | `GR-ADDRESS-CENSUS … operands=5 bound=N unbound=5-N mme_dwords=39` with at least one `Unresolved` | X's size, quantified at the guest's own addresses for the first time |
| **B — everything binds** (`unbound=0`) | ⚠ **strictly better than A and the first thing to distrust.** It would mean property 1 is already met and only **closure** (property 2) separates us from a containable GR channel. Check the sites are plausible — a `GuestRam { gpa }` for a `0x7d1e_…` base is a resolver saying yes to something no guest allocated |
| **C — nothing binds** (`bound=0`) | the table binds the completion (`w226b`, reproduced at `w227a`) and nothing else, so §3 is a one-line answer and X is far |
| **D — no census line at all** | ⊘ the declare path did not reach it. **Absent by construction**; nothing below may be cited |
| **E — the doorbell census MOVES** | ⚠ ✱ **the costly arm.** `191/183/8`, `GrCompute=8 Ce=183`, `forwarded=0` at `w218`/`w220`/`w221`/`w222`/`w226b`/`w227a`. The census is read-only on the same read the observer already did; if these move, the finding is about my change |
| **F — `COMPLETION-WATCH` moves off `NOT-OBSERVED`** | ⚠ **nothing in this build runs GR work and the writer census is zero.** Do not report it as progress — find the writer |

### 5.1 The result

`[to be filled from `w227b`]`

---

## 6. ⊘ WHAT THIS RUNG DID NOT DO — read before citing it

1. ⊘ **It did not open S1.** `Route::NotACopyEngineChannel` is untouched.
2. ⊘ **It did not build a VA space, a GR codec, or a shadow channel.** §4 is a specification,
   not an implementation, and the fault-containment property (4.3) is explicitly unmeasured.
3. ⊘ **It says nothing about `§12.26` / `SystemDataPlane`.** `w226c`'s pin was refused because
   that doorbell is the guest **kernel's** own client `0xc1e00006` — the CE scrubber. ★ The
   completion this campaign is about is on the **user proc's** `GrCompute` path (`proc=2`,
   client `0xc1d0000c`), which fails somewhere else entirely. **The two are separate walls**;
   re-opening §12.26 is an owner decision and is not on this path. Established as a finding,
   not acted on.
4. ⊘ **The completion plane is still not delivered**, and the reason is unchanged from
   `completion_observer.md` §8.4: on our path the completion is not corrupted and not lost —
   **it is never produced, because the work never runs.**
