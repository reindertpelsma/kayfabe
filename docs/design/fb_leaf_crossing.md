# §5.9 — THE SECOND CROSSING: real host vidmem under the guest's framebuffer leaves

**Rung:** `w228` / `master`, based on `3db634f` (the `w227c` census).
**Question, from the brief:** make `cuCtxCreate`'s three **framebuffer** operands resolve to
real host-visible backing, so the host GR engine *could* dereference them. Port the C's
GEN-2 shape: one blank host vidmem object per vidmem leaf, mapped **FIXED** at the guest's VA.

**Answer: built, and it lands on a real GA106.** What moved is the **address plane**, not the
guest: `CUP2_RC` is unchanged and the doorbell census is unchanged, both by construction —
nothing in this rung routes a doorbell or points an engine at anything.

Companions: `guest_ram_crossing.md` §5.8 (the **first** crossing — sysmem, the guest's own
pages), `gr_execution_boundary.md` (the census this rung is driven by, and the S1 refusal it
does **not** open), and `C: docs/design/mode2_fb_crossing_question.md` (the C's own
adjudication, settled 2026-06-04, built twice).

---

## 0. ★★★★★ WHAT I REFUTED FIRST, INCLUDING TWO OF MY OWN INSTRUMENTS

### 0.1 ⊘⊘ REFUTED, MINE: my first negative control tested nothing, and would have printed a success as though a guard had fired

The control I wrote first probed **one leaf past the highest leaf the census named**, on the
reasoning that the guest's page tables do not bind it, so the backing must refuse.

⊘ **`back_fb_leaf` does not walk the guest's page tables.** It is *given* the walk's answer by
its caller — that is the whole point of carrying `(va, len, phys)` as arguments rather than
re-deriving them. So "an address the guest does not bind" is not a proposition it can refuse:
an address the *address table* does not bind is precisely the ordinary **fresh-publish** path.
The control would have allocated a real host vidmem object at a fabricated address and
reported it as the guard firing.

★ Caught before the boot, by asking the question this campaign keeps having to ask: *which
line of code is this control expecting to execute?* There was no such line.

**What replaced it** is a control over the guard this rung actually exists for: take a leaf
that was **just backed** — so the address table demonstrably binds it at the walk's own
physical address — and re-ask for it naming a **different** physical address. The two sources
now disagree and `FbLeafDisagrees` must fire, printing both numbers and allocating nothing.
Every input is derived from what the boot itself produced.

### 0.2 ⊘⊘ REFUTED, MINE: `cargo check --workspace --all-targets` does not compile `host-isolates`

The census caller is behind `#[cfg(feature = "host-isolates")]`. A path naming
`kayfabe_fwd::` — a crate `kayfabe-qemu-raw` does **not** depend on — passed
`cargo check --workspace --all-targets`, `cargo clippy --workspace --all-targets` and the
entire test suite, and failed only on the bench.

⇒ **The local gate for anything under that cfg is `--features host-isolates`.** The
`--all-targets` in the invocation reads like completeness and quantifies over *targets*, not
over *features*; the two words are easy to conflate and the conflation is silent.

### 0.3 ⚠ The brief's framing survives, and its ratio is confirmed

*"3 of the 4 bound operands are in the emulated framebuffer"* — re-read off `w227c` and
unchanged. The brief also asked whether this rung would move the guest or explain why not.
**It does not move the guest, and the reason is structural rather than incidental**: §4.

---

## 1. What was built

One chain, four verbs, all of them ones this port already had except the first:

```
alloc_vidmem(len)                    ← NEW intent verb over the EXISTING alloc_device_local
      → map_gpu_va(vas, mem, len, at = the guest's own leaf VA)   ← FIXED, DMA_OFFSET_FIXED
      → AddressTable::bind(pdb, va, len, Binding { phys, Vidmem, host: Some(..) })
```

- **`RmBackend::alloc_vidmem`** — `NV01_MEMORY_LOCAL_USER` (class `0x0040`), `CONTIGUOUS |
  LOCATION_VIDMEM`. The same class the C issues (`C: nvkvm_gpu_emul.c:7286-7294`), and
  `RmConnection::alloc_device_local` has issued it since the channel work; this rung only
  gives it an intent name.
  ⊘ **Not `alloc_sysmem` with a flag.** That verb asks for `MAPPING_NO_MAP`, so its object is
  deliberately un-CPU-mappable — correct for a describe-only range and fatal for this one,
  because GEN-2's mature form is a *double* mapping. A leaf backed with sysmem allocates,
  maps, passes every check, and can never become the shared object.
- **`VerbPlan::PublishVidmem`** — structurally `Publish`'s twin; the unwind, the orphan sets
  and the `#102` placement check are identical **on purpose**, because a second subtly
  different copy of an unwind is how an orphan gets dropped.
- **`FwdFault::FbLeafDisagrees` / `FbLeafExtent` / `FbLeafGranularity`** — the three ways this
  chain refuses, all by name, all in the plan phase before a host object exists.

### 1.1 ★★★ The unit is the LEAF, and the type says so

Nothing in a pushbuffer states how long a `SET_TEX_SAMPLER_POOL` is. The guest names a base
address; the engine reads as far as its own state says. The only extent anything in this port
can state **from evidence** is the one the guest's own page tables bind — so
`completion_watch::FbLeaf { va, len, phys }` rides out of the same walk that resolved the
operand, and a buffer longer than its first leaf is backed only as far as that leaf goes.

⚠ **A real limit, carried in the type rather than inferred.** The C reaches the same unit from
the other direction: it coalesces adjacent leaves into runs and re-cuts the runs into 2 MiB
chunks (`C: :8472-8477`), one host object per chunk.

### 1.2 ⊘ The granularity refusal, and why it is a divergence from the C rather than an omission

The C rounds the allocation up to 64 KiB and registers **the rounded range**
(`C: :8242-8243`, `asize` vs `tsize`). For a leaf that is not a whole number of 64 KiB
granules, its object therefore claims up to **60 KiB of guest framebuffer address space past
the end of the leaf** — an overhang the establishment copy never fills and which the local
shadow can no longer answer for, because `nvkvm_fb_host_overlay` returns non-NULL for it.

Here that is `FbLeafGranularity`, refused by name. **The overhang is unrepresentable rather
than inherited.**

---

## 2. ★★★★★ THE PROPERTY THIS RUNG IS ABOUT — two sources for one fact

The leaf's `(va, len, phys)` has two independent sources:

| source | what it is | who reads it |
|---|---|---|
| the **guest's own page tables** | the authority on what the guest bound | the CE walker, on the census's single read |
| this proc's **`AddressTable`** | our mirror, forward-populated | `plan_back_fb_leaf` |

When both answer, they must answer the same. `plan_back_fb_leaf` refuses on any disagreement
— physical address **or** aperture **or** extent — and prints both readings.

⊘ **Preferring one is not a tiebreak, it is the bug.** Backing at the walk's address while the
table names another puts a real host GPU object under an address the guest reaches through the
other reading; backing at the table's address backs a leaf the guest's page tables do not have.
Both are silent. `two_projections_of_one_fact_disagreeing` has three prior instances in this
campaign and every one of them was resolved by a comment before it was resolved by a check.

`tests/tests/fb_leaf_backing.rs::the_walk_and_the_table_disagreeing_is_refused_by_name` asserts
that **no host verb is issued at all** in that case — the refusal is in the plan phase, so
there is nothing to orphan.

---

## 3. ⊘ WHAT THIS IS NOT — read before any green line

- **The object is BLANK.** Nothing seeds it, nothing copies into it. The C's `copy_content`
  (`C: :8281-8290`) is a separate one-time establishment bridge and needs a CPU view.
- **There is NO CPU view, so there are TWO MEMORIES.** The guest's own framebuffer accesses at
  that physical address still go to the shell's fabricated aperture; the host object is
  reachable only by the host GPU. ★ This is the C's own `gpu_only` shape
  (`C: :7354-7368`) — chosen *there* because a CPU view consumes the host's 256 MiB BAR1
  (its *"proven D2 wall"*), and chosen *here* for a different reason: the isolate holds the
  mapping and the shell holds the framebuffer, and the descriptor that would join them
  (`Request::ExportBacking`, already on the wire) is not routed to this path.
  ⇒ **That is the successor rung, and it has a named mechanism already in the tree.**
- **NOTHING EXECUTES.** No doorbell is routed, no engine is pointed anywhere.
  `Route::NotACopyEngineChannel` refuses every `GrCompute` doorbell exactly as it did at
  `w227`. A host object existing at an address is not an engine dereferencing it.
- **This does not open S1.** `gr_execution_boundary.md`'s refusal stands, with its reason
  unchanged: the pushbuffer carries **39 dwords of guest-authored MME microcode** whose output
  *is* methods, so no method allowlist can be sound and the VA space is the only containment
  surface. This rung makes part of that surface real; it does not open the gate.

### 3.1 ⚠ The gap inherited knowingly

The C's re-back for VA→GPA re-binding is **sysmem only** (`C: :8396-8445`); its framebuffer
path has drop-on-free (`C: :2003-2021`) and no re-seed. This port inherits that: if the guest
unbinds a leaf and re-creates it at the same VA over a different frame, the host object stays
where it was. ⊘ **Named, not silently reproduced** — and it is a smaller hole here than there,
because `bind` refuses an overlap, so the stale binding is visible rather than replaced.

---

## 4. MEASURED — `w228a_82f9aa5_fbback`, real GA106, host driver 580.159.04

`[measured 2026-08-10, bench `vh`, RTX 3060 GA106, `KAYFABE_ISOLATES=real`,
`KAYFABE_FB_BACKING=on`, CE executor `local`]`
**Tree, QEMU binary and archive all stamped `82f9aa5324ed1f6e5e6593051e274219904109b3`** —
asserted on both artifacts before the boot, not read off a file that claims to record it.

### 4.1 ★★★★★ The three rows moved, in the same census that measured them

```
kayfabe: GR-ADDRESS-CENSUS proc=2 chan=0 class=0xc7c0 operands=5 bound=4 unbound=1 mme_dwords=39
      SET_VALID_SPAN_OVERFLOW_AREA  va=0x200000000   → Framebuffer { phys: 0x400000, leaf: {va 0x200000000,   len 0x200000, phys 0x400000} }
      SET_SHADER_SHARED_MEMORY_WINDOW va=0x7c50d9000000 → Unresolved(CeWalk … Fault)
      SET_TEX_SAMPLER_POOL          va=0x10002000000 → Framebuffer { phys: 0x800000, leaf: {va 0x10002000000, len 0x200000, phys 0x800000} }
      SET_TEX_HEADER_POOL           va=0x10000000000 → Framebuffer { phys: 0x600000, leaf: {va 0x10000000000, len 0x200000, phys 0x600000} }
      SET_REPORT_SEMAPHORE          va=0x20440fff0   → GuestRam { gpa: 0x3080ff0 }

kayfabe: GR-FB-BACKING proc=2 chan=0 SET_VALID_SPAN_OVERFLOW_AREA leaf va=0x200000000   len=0x200000 fb_phys=0x400000 → BACKED memory=0xcafe005e host_va=0x200000000   placed_as_asked=true
kayfabe: GR-FB-BACKING proc=2 chan=0 SET_TEX_SAMPLER_POOL          leaf va=0x10002000000 len=0x200000 fb_phys=0x800000 → BACKED memory=0xcafe005f host_va=0x10002000000 placed_as_asked=true
kayfabe: GR-FB-BACKING proc=2 chan=0 SET_TEX_HEADER_POOL           leaf va=0x10000000000 len=0x200000 fb_phys=0x600000 → BACKED memory=0xcafe0060 host_va=0x10000000000 placed_as_asked=true

kayfabe: GR-ADDRESS-CENSUS (RE-STATED AFTER BACKING) proc=2 chan=0 backed_leaves=3
      SET_VALID_SPAN_OVERFLOW_AREA  … → HostBackedFb { phys: 0x400000, leaf {…}, host_va: 0x200000000,   memory: 0xcafe005e }
      SET_TEX_SAMPLER_POOL          … → HostBackedFb { phys: 0x800000, leaf {…}, host_va: 0x10002000000, memory: 0xcafe005f }
      SET_TEX_HEADER_POOL           … → HostBackedFb { phys: 0x600000, leaf {…}, host_va: 0x10000000000, memory: 0xcafe0060 }
```

★ **`placed_as_asked=true` on all three.** RM honoured `DMA_OFFSET_FIXED` and put each object
at the *guest's* number, in this proc's own host VAS. ⊘ Not an echo: `map_gpu_va`'s contract
converts a different answer into `RmError::PlacementRefused` and unwinds, and
`tests/tests/guest_ram_pin.rs`'s `Relocating` backend is the standing proof that the check can
fail.

★ **The leaf is 2 MiB**, which is what the three framebuffer addresses spaced exactly `0x200000`
apart already suggested. The granularity gate (§1.2) therefore never fired on this workload —
it is a guard, not a filter.

### 4.2 The tallies, over the whole boot

| | count | what it means |
|---|---|---|
| `GR-FB-BACKING` lines | **32** | 8 GR channels × (3 leaves + 1 control) |
| `→ BACKED` | **3** | the work, done once |
| `→ ALREADY BACKED` | **21** | 8×3 − 3: channels 1–7 replay |
| distinct host objects | **3** (`0xcafe005e/5f/60`) | ⊘ **not 24** — the idempotent replay is real, and all eight channels share one `Vas` |
| ✅ negative control fired | **8** | once per channel |
| ⚠ control did not fire | **0** | |
| rows still `Framebuffer{…}` after re-statement | **0** | |
| host `Xid` | **0** | |

### 4.3 ★★★★★ The negative control, on real core state

```
kayfabe: GR-FB-BACKING proc=2 chan=0 ✅ NEGATIVE CONTROL: leaf va=0x200000000 re-asked with
  the WRONG framebuffer address 0x600000 → REFUSED BY NAME as `FbLeafDisagrees`
  walked=(6291456, Vidmem) tabled=(4194304, Vidmem) — both numbers printed, neither used,
  and no second object allocated
```

Both readings in the line, neither adopted, and the distinct-object count above is the
independent evidence that nothing was allocated behind it.

### 4.4 ★★ The two rows that must NOT move, and did not

`SET_REPORT_SEMAPHORE` stayed `GuestRam` and `SET_SHADER_SHARED_MEMORY_WINDOW` stayed
`Unresolved`, in every one of the 8 re-stated censuses. The first is the **first** crossing's
business and a framebuffer pass that claimed it would be backing sysmem with vidmem; the
second is ASLR'd and varies per boot (`0x7f5ca9000000` at `w227c`, `0x7c50d9000000` here), and
a pass that "resolved" it would be inventing an address. ⊘ Structural rather than lucky: the
backing loop's `match` has exactly one arm.

### 4.5 ⊘ AND THE GUEST DID NOT MOVE — by construction

`CUP2_RC` and the doorbell census are unchanged from `w227c`. That is the **expected** result
and it is not a disappointment to be explained away: this rung publishes addresses into a host
VA space and routes no doorbell, so there is no mechanism by which `cuCtxCreate` could have
progressed. `Route::NotACopyEngineChannel` refuses every `GrCompute` doorbell in this log
exactly as it did in the last one.

★ **What changed is falsifiability.** Before this rung, "the host GR engine cannot dereference
the guest's operands" was an argument. Now 3 of the 4 bound operands resolve to real host
objects at the guest's own addresses, and the remaining distance is countable rather than
arguable.

### 4.6 ★★★ THE UNARMED CONTROL BOOT — `w228b_82f9aa5_control`

**Same binary, same tree, same stamp, `KAYFABE_FB_BACKING` unset.**

| | armed (`w228a`) | control (`w228b`) |
|---|---|---|
| `GR-FB-BACKING` lines | 32 | **0** |
| `HostBackedFb` rows | 24 | **0** |
| `RE-STATED` blocks | 8 | **0** |
| `GR-ADDRESS-CENSUS` blocks | 8 | **8** |
| `→ Framebuffer { … }` rows | 0 after re-statement | **24** (8 × 3, none backed) |
| distinct host vidmem objects | 3 | **0** |

★ The unarmed path is **silent, not merely quiet** — it prints no line the armed run does not,
so the two logs differ by exactly the thing under test. ⊘ That was a deliberate choice at the
arming check; a "backing disabled" line would have made the control a *different* log rather
than a subset of one.

### 4.7 ★★★ THE NEXT BLOCKER, BY NAME, FROM THIS BOOT'S OWN EVIDENCE

Not inferred from the last rung and not read off a design document — three lines of
`run_w228a_82f9aa5_fbback_qemu.log`:

```
nvkvm: doorbells: 191 arrived, 183 served, 8 REFUSED by name
nvkvm:   of the served: 183 local (CPU CE, end witnessed), 0 forwarded (host channel rung)
nvkvm:   by engine: GrCompute=8 GrGraphics=0 Ce=183 …
nvkvm: first doorbell refusal [Route::NotACopyEngineChannel] …
COMPLETION-WATCH proc=2 chan=0 va=0x20440fff0 payload=0x00000001 → NOT-OBSERVED samples=88
  last_seen=0x00000000 … ⊘ the address WAS readable and the declared payload never appeared
```

⇒ **`Route::NotACopyEngineChannel`, 8 of 8.** Every `GrCompute` doorbell `cuCtxCreate` rings is
refused; `forwarded = 0`; nothing ever executes the pushbuffer; the observer read the guest's
own declared semaphore **88 times** and saw `0x00000000` every time. `cup2`'s stdout stops
after `cuDeviceTotalMem` and `pid=1515 state=Rl` — a **userspace spin inside `cuCtxCreate`**,
exactly where the census fired.

⊘ **And the doorbell census is byte-identical to `w218`/`w227c`** — 191/183/8. That is the
control on this rung's own claim: the address plane changed and the submission plane did not,
which is what a rung that touches only the address plane must produce. If those numbers had
moved, something here would be doing more than it says.

★★ **What this rung changes about that blocker is its REASON, not its status.**
`gr_execution_boundary.md` refused to open it because the VA space is the only sound
containment surface for a stream carrying 39 dwords of guest-authored MME microcode, and that
surface was not real: three of the four addresses the engine would dereference were backed by
nothing a host engine could reach. Three of them now are. ⊘ **This does not open the gate** —
`SET_SHADER_SHARED_MEMORY_WINDOW` is still `Unresolved`, there is still no CPU view, and no
sweep enumerates the leaves an operand does *not* name. It removes one of the reasons the gate
could not be opened, and leaves the others standing and countable.

---

## 5. What the next rung inherits

1. **The CPU view** — `Request::ExportBacking` is *"perform the mapping and hand back
   memory"* and is already the one request whose reply may carry a descriptor. Joining the
   isolate's mapping of the vidmem object to the shell's `SparseFb` at that framebuffer
   physical address is GEN-2's second half and closes the two-memories gap.
2. **Coalescing** — one object per leaf is the safe unit, not the efficient one. The C's
   run-coalescing plus 2 MiB re-cut is the shape to port once the CPU view exists, and it
   needs the leaf enumeration this rung does not have (we back only the leaves an operand
   names, never a whole page-table sweep).
3. **The FB re-back** for VA→GPA re-binding (§3.1).
4. ⊘ **Not** the doorbell gate. See §3.
