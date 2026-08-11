# S1 — may the host GR engine execute the guest's pushbuffer?

⊘⊘⊘ **STATUS (2026-08-11): the DIAGNOSIS below is LIVE and still correct; ONE DEPENDENCY IT
NAMES HAS SINCE BEEN MET.** §4's table records the emulated-framebuffer crossing as *"does not
exist … not built"*. **It was built one rung later** (w228, `fb_leaf_crossing.md`, flag
`KAYFABE_FB_BACKING=on`), and `traces/boots/w247/` measures all three preconditions armed at
once: every address plane this workload needs now resolves to something the host GPU can address.
⇒ **The remaining blockers are properties 2 (CLOSED) and 3 (FAULTING/CONTAINED) — neither is an
addressing problem.** ★ Property 3 is the cheapest open item on the board and
`scripts/bench/gpu_fault_containment.sh` has never been asked it. See §16.99.

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
| 002-003 | `SET_SHADER_SHARED_MEMORY_WINDOW_A/B` | `0x7d1e_e900_0000` (⚠ **varies per boot** — `0x7f5c_a9000000` at `w227c`) | a 64-bit **window base** the guest chose; see §3.4 for why its per-boot variance is what identifies it |
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

⚠ **Three defects, all in the instrument, all found.** They are written up before the number
because they are the reason to believe the number: `suspect_the_instrument_first` says this
project's tests have been the defect about twenty times, and an instrument built tonight that
reported no trouble would be the one to distrust.

### 3.1 ★ DEFECT ONE, caught before the boot: the generalisation of a correct decoder is not a correct decoder

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

### 3.2 ⊘ DEFECT TWO, caught AT the boot: the first result was wrong, and its error pointed the ENCOURAGING way

`[measured 2026-08-10, boot `w227b_184df5f_census`, `KAYFABE_ISOLATES=real`, CE executor
`local`, RTX 3060 GA106, host driver 580.159.04]` — identical on all eight `GrCompute`
channels:

```
GR-ADDRESS-CENSUS proc=2 chan=0 class=0xc7c0 operands=2 bound=2 unbound=0 mme_dwords=39
      SET_VALID_SPAN_OVERFLOW_AREA   m=0x0200 sub=1 va=0x200000000  → Framebuffer { phys: 0x400000 }
      SET_REPORT_SEMAPHORE           m=0x1b00 sub=1 va=0x20440fff0  → GuestRam { gpa: 0x564fff0 }
```

⊘ **`operands=2` is wrong; the stream names five.** The guest writes address registers **two
different ways in one pushbuffer** — `SET_VALID_SPAN_OVERFLOW_AREA` as a single three-argument
`INCREASING` run, and `SET_TEX_HEADER_POOL` / `SET_TEX_SAMPLER_POOL` /
`SET_SHADER_SHARED_MEMORY_WINDOW` as **separate single-argument methods, one per half**. The
decoder read only the run spelling.

★★★★★ **And the failure pointed the encouraging way.** Three missing operands made the line
read `unbound=0` — *"every address the guest names already binds"*, the most positive answer
the instrument can produce. §5's arm **B** had named that outcome in advance as *"strictly
better than A **and the first thing to distrust**"*. Distrusting it is the only reason the
defect was found in the same hour instead of being written up as *"only closure separates us
from a containable GR channel."*

⇒ ★ **A falsifier that flags its own good news is worth writing.** Every falsifier in this
campaign so far has been armed against a disappointing result; this one earned its keep by
being armed against a pleasing one.

**The fix:** the decoder now models the hardware's **register file** rather than a set of runs
— expand each run per `SECOP` (`INCREASING` walks the method address, `NON_INCREASING` repeats
it), latch each half independently, emit an operand only when **both** halves have been
written. Spelling-independent by construction rather than by having seen the spellings.

### 3.3 ★★ DEFECT THREE, caught by the fix: the FIXTURE was a second implementation of the wire

The unit fixture built the MME loads with the `INCREASING` header helper. `[measured]` the
real ones are `NON_INCREASING` (`hdr=0x600f2046`, `0x60182046`) — so the corrected decoder,
being correct, walked 15 microcode dwords across fifteen consecutive registers, landed two of
them on `SET_GLOBAL_RENDER_ENABLE_A/B`, and **invented a sixth operand out of microcode**.

⊘ A false *positive*, manufactured inside the instrument that exists to count false negatives,
by a fixture that was a second description of the wire. `hdr_ni` now carries the measured
`SECOP` and two tests pin it.

### 3.4 ★★★★★ THE CORRECTED RESULT — arm A, and it moves the named dependency

`[measured 2026-08-10, boot `w227c_537894e_census2`, rev `537894e` (stamped in the binary),
`KAYFABE_ISOLATES=real`, CE executor `local`, RTX 3060 GA106, host driver 580.159.04]` —
**identical on all eight `GrCompute` channels**:

```
GR-ADDRESS-CENSUS proc=2 chan=0 class=0xc7c0 operands=5 bound=4 unbound=1 mme_dwords=39
  SET_VALID_SPAN_OVERFLOW_AREA     m=0x0200 va=0x2_00000000    → Framebuffer { phys: 0x400000 }
  SET_SHADER_SHARED_MEMORY_WINDOW  m=0x02a0 va=0x7f5c_a9000000 → Unresolved("CeWalk … Fault")
  SET_TEX_SAMPLER_POOL             m=0x155c va=0x100_02000000  → Framebuffer { phys: 0x800000 }
  SET_TEX_HEADER_POOL              m=0x1574 va=0x100_00000000  → Framebuffer { phys: 0x600000 }
  SET_REPORT_SEMAPHORE             m=0x1b00 va=0x2_0440fff0    → GuestRam { gpa: 0x43b0fff0 }
```

#### ★★★★★ THE HEADLINE: FOUR OF THE FIVE BIND, AND THREE OF THOSE FOUR ARE IN THE EMULATED FRAMEBUFFER

The completion observer's one address (`SET_REPORT_SEMAPHORE`) is the **only** operand in guest
RAM. Everything else the host GR engine would dereference — the texture header pool, the
sampler pool, the valid-span overflow area — resolves into **our emulated framebuffer**, which
is memory in the QEMU process that the host GPU has no mapping of and no way to acquire one.

⇒ **The FB crossing is not a successor question. It is the majority of the surface.**
`mode2_fb_crossing_question.md` was carried as *"the named successor — assert the dependency by
name if you reach it; do not build it."* This rung reached it and can now quantify it: **3 of
the 4 bindable operands, 75 %,** are on the far side of a crossing that does not exist.
`GuestRamGrant` (`28b7bb2`) crosses the **one** that is not.

★ That reverses the intuition this campaign has been carrying. The guest-RAM crossing was
treated as *the* enabling primitive because the completion lives there; the census says the
completion is the **exception**, and the context buffers — which is what a GR context *is* —
are somewhere else entirely.

#### ⊘ AND THE ONE `Unresolved` IS NOT A GAP — reading it as one would be the third mistake tonight

`SET_SHADER_SHARED_MEMORY_WINDOW` faults at `0x7f5c_a9000000`. Before calling that a hole in
the address table, note that the value **changes between boots**: `0x7d1e_e9000000` at `w226b`
and `w227a`, `0x7f5c_a9000000` at `w227c`. A `0x7f…` base that moves per run is an **ASLR'd
userspace address**, and `SET_SHADER_SHARED_MEMORY_WINDOW` is by its own name a **window base**
— the aperture against which generic addresses are disambiguated — not an allocation. Nothing
is expected to be mapped at a window base, so a walk that faults there is the table being
**right**.

⇒ ★★ `bound=4 unbound=1` therefore reads as **5 of 5 accounted for**, and the honest headline
is not *"one address is missing"* but *"every address is known, and 3 of 5 are in the wrong
plane."* ⊘ A census that reported `unbound` as a synonym for `missing` would have manufactured
a fourth false finding out of a correct measurement — the same shape as §3.2, one level up.
`[NOT MEASURED]` whether the window base is ever dereferenced by the engine; the reading above
rests on the method's name and on the value's per-boot variance, not on hardware behaviour.

#### The falsifier's costly arm did not fire

| | `w218` | `w220` | `w221` | `w222` | `w226b` | **`w227a`** | **`w227b`** | **`w227c`** |
|---|---|---|---|---|---|---|---|---|
| doorbells | 191/183/8 | 191/183/8 | 191/183/8 | 191/183/8 | 191/183/8 | **191/183/8** | **191/183/8** | **191/183/8** |
| by engine | — | `Gr=8 Ce=183` | same | same | same | **same** | **same** | **same** |
| `forwarded` | 0 | 0 | 0 | 0 | 0 | **0** | **0** | **0** |
| `SMI_RC` / `CUP2_RC` | 0 / TO | 0 / TO | 0 / TO | 0 / TO | 0 / TO | **0 / TO** | **0 / TO** | **0 / TO** |
| `COMPLETION-WATCH` | — | — | — | — | 8× NOT-OBS | **8× NOT-OBS** | **8× NOT-OBS** | **8× NOT-OBS** |

**Byte-identical across three boots of this rung.** A print-and-resolve instrument must move
nothing, and it moved nothing.


---

## 4. ⇒ X, STATED SO IT CAN BE BUILT

**X = a CLOSED GUEST-IMAGE VA SPACE for host GR channels**, with four properties, none of
which exists today:

1. **Complete** — every VA this guest's compute pushbuffers can name resolves **to something
   the host GPU can address**. ★ §3.4 sharpens this: our address *table* is already complete
   for this workload (5 of 5 accounted for), so the shortfall is **not** a binding problem —
   it is that 3 of the 5 bind into a plane the host GPU cannot reach.
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

**The named dependencies for (1), and §3.4 changed which one leads.** There are **two**
crossings, not one:

| plane | operands | crossing |
|---|---|---|
| guest RAM | 1 of 5 (`SET_REPORT_SEMAPHORE`) | ★ **exists** — `map_guest_ram` arm B (`isolate-host/src/rm.rs:2816-2840`) builds an `OS_DESCRIPTOR` over guest pages, and `GuestRamGrant` landed at `28b7bb2` (R29: placed at `0x301400000` as asked, isolate mapping reads `0x9a114001`) |
| the **emulated framebuffer** | ★★★ **3 of 5** (`SET_TEX_HEADER_POOL`, `SET_TEX_SAMPLER_POOL`, `SET_VALID_SPAN_OVERFLOW_AREA`) | ⊘⊘⊘ **CORRECTED — IT WAS BUILT THE NEXT RUNG.** `fb_leaf_crossing.md` (w228, `82f9aa5`): real host `NV01_MEMORY_LOCAL_USER` vidmem per leaf, mapped **FIXED** at the guest's own VA, behind **`KAYFABE_FB_BACKING=on`** (default off). `[measured 2026-08-11, `traces/boots/w247/`]` `placed_as_asked=true` ×24, `HostBackedFb` ×24. ⚠ The text below — *"does not exist … not built"* — was true only at `c5f251d`. |

⇒ **The primitive that exists covers the minority of the surface.** The FB crossing was
carried as a successor question; the census makes it the load-bearing one, because a GR
context *is* its context buffers and they are all on that side.

⚠ And it is not the same crossing twice. Guest RAM is memory the hypervisor holds for the life
of the machine and can hand out as an `OwnedFd`; the emulated framebuffer is a **store inside
the QEMU process** (`kayfabe_device::FbStore`) with no fd, no page-aligned host backing
contract, and a first-writer census that assumes we are the only writer. Making it
host-GPU-addressable is a different design, not a second call to the same function.

⊘ **Do not read this as "just build a big VA space".** Property 2 is the hard one and it is a
*subtraction*: the value of the isolate today is partly that our objects and the guest's live
in one place. GR execution requires them not to.

### 4.1 ⇒ THE NEXT RUNG, ORDERED — and none of these is "open the route"

The census reorders the work. In dependency order, cheapest and most falsifiable first:

1. ★★ **Ask the fault-containment question** (property 3). `scripts/bench/gpu_fault_containment.sh`
   exists. Until it is answered, *every* plan above is conditional on a `[NOT MEASURED]`, and
   it is the one item here that can be measured **without building anything**.
2. ★★★ **Scope the FB crossing** — `mode2_fb_crossing_question.md`, now known to be 75 % of the
   surface. The question is not *"can we OS_DESCRIPTOR the FbStore"* but *"what is the
   framebuffer, such that a host GPU can address it"* — today it is a process-local store with
   no fd and a first-writer census that assumes a single writer.
3. **Then** property 2 (closure): move the isolate's ring/USERD/semaphore out of any VAS a
   guest channel is bound to. ⊘ This is a change to `alloc_channel_at` that has nothing to do
   with GR and can be done, and tested, on the CE path where there is already a working
   executor to regress against.
4. ⊘ **Only then** is *"open S1"* a question with a defensible answer.

⚠ **And the instrument is already in the tree for step 2's falsifier**: `GR-ADDRESS-CENSUS`
prints the plane of every operand, so *"the FB crossing works"* has an existing, measured
before-shape — `bound=4` with **3 × `Framebuffer`** — and a green shape nobody has to invent.

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

### 5.1 The result — scored

| arm | fired? | |
|---|---|---|
| **A — a mixed census** | ★ **YES**, at `w227c`: `operands=5 bound=4 unbound=1 mme_dwords=39` | the predicted arm, and §3.4 reads it |
| **B — everything binds** | ⚠ **YES at `w227b`, and it was FALSE** — `operands=2 unbound=0`. Arm B's own instruction (*"the first thing to distrust"*) is what found the decoder defect (§3.2) | ★★★ the arm that paid for itself |
| **C — nothing binds** | no | |
| **D — no census line** | no — 8 lines, one per channel, at both `w227b` and `w227c` | |
| **E — the doorbell census moves** | ⊘ **no.** `191/183/8`, `GrCompute=8 Ce=183`, `forwarded=0`, byte-identical across `w227a`/`w227b`/`w227c` | the costly arm, and it is clean |
| **F — `COMPLETION-WATCH` moves off `NOT-OBSERVED`** | ⊘ **no.** 8 × `NOT-OBSERVED`, `last_seen=0x00000000`, on all three boots | as expected: nothing runs GR work |

### 5.2 ★ THE WRITER CENSUS, RE-RUN — still ZERO, both instruments

- **static** — `tests/tests/single_writer_census.rs`, 7 tests green at `537894e`. The whole
  workspace's guest-visible write surface is the pinned set: **two** `gpa_write` production
  sites (the CE/completion funnel and the emulated-GSP RAM port), **zero** production callers
  of the raw-address completion door, `write_plane` still private.
- **dynamic** — `w227a`/`w227b`/`w227c`, 8 channels each, `last_seen=0x00000000` over 81-88
  samples per channel spanning the whole `cuCtxCreate` wall. **A page with two writers is a
  page with at least one write.**

⇒ ⊘ **We are still not reproducing M5.38.** The completion is not corrupted and not lost. It
is **never produced**, because the work never runs — and §2 is the reason the work must not be
made to run yet.

---

## 6. ⊘ WHAT THIS RUNG DID NOT DO — read before citing it

1. ⊘ **It did not open S1.** `Route::NotACopyEngineChannel` is untouched.
2. ⊘ **It did not build a VA space, a GR codec, or a shadow channel.** §4 is a specification,
   not an implementation, and the fault-containment property (4.3) is explicitly unmeasured.
3. ⊘ **It says nothing about `§12.26` / `SystemDataPlane`, and the SEPARATION is now measured
   rather than argued.** `w226c`'s pin was refused because that doorbell belongs to the guest
   **kernel's** own client. `[measured, `w227c_537894e_census2`]` the two populations do not
   overlap at all: every `GrCompute` channel in the boot is client **`0xc1d0000c`**, `proc=2`
   — libcuda's — while the CE scrubber's channels are **`0xc1e00005`/`0xc1e00006`**, `proc=0`,
   the kernel's. ★ **The completion this campaign is about is on the user proc's path and
   fails somewhere else entirely.** Re-opening §12.26 is an owner decision and is not on this
   path. Established as a finding,
   not acted on.
4. ⊘ **The completion plane is still not delivered**, and the reason is unchanged from
   `completion_observer.md` §8.4: on our path the completion is not corrupted and not lost —
   **it is never produced, because the work never runs.**
