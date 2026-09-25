# `cuda/walk` — the walk kernel, and the suite that attacks it

> ### ⊘⊘⊘ SUPERSEDED IN PART 2026-09-25 (branch `v3-diff`) — THE DIFF IS AGAINST CONFIRMED PLACEMENTS, COMMITTED ON ACK
> The walk-vs-previous-walk delta this file describes (the `acked` generation handshake,
> `RESYNC`, `REMAP`, scope hints, the `delta/*`, `roundtrip/*`, `scope/*` cases and the
> `check-closure-negative` / `KF_BREAK_SEG_COALESCE` known-positives) is **gone**. Owner design +
> COMMIT-ON-ACK ruling: per VA-space object (a **slot**, never a PDB) the kernel keeps the
> placements the host confirmed; `kf_refresh(w, …, pdbs, slots, n, …)` reports each entry's diff
> against its slot — UNMAPs of whole placements, then MAPs of the pieces between kept ones, per
> page-size class (`KFWR_HF_DIFF`, `KFWR_V_PARTIAL` / `KFWR_V_OVERFLOW`); `kf_ack(w, gen, codes,
> nrun, resets, nreset)` stages one verdict per run, committed by the next refresh. The spec is
> `crates/kf-cuda/src/diffmodel.rs`; the suite's `diff/*` and `roundtrip/diff_*` cases (the latter
> against a C++ port of that model) replace the old ones, and `kf-gate9` holds the shipped PTX to
> the Rust model. Everything below about the WALK (I1–I3, the seam, hostile/race/differential) stands;
> read every "delta"/"resync"/"ack" passage as history.

**STATUS: LIVE (2026-09-14).** A standalone CUDA proving ground for the kernel of
`docs/design/dirty_tracking_without_uffd.md` §w720e, emitting the report of
`docs/design/the_walk_kernel_report_format.md`. Bare metal, no VM, no guest,
nothing from the kayfabe runtime — a GPU, a buffer, a kernel and assertions.

| file | what |
|---|---|
| `kf_walk.h` | the report ABI (the format doc's 64/32/32 structs) + the host API |
| `kf_walk.cu` | the **format descriptor**, the walk, the coalescer, the per-class diff, the report |
| `kf_tables.h` | a **host-side** GA10x VER2 table builder — test scaffolding |
| `kf_tests.cu` | **format seam**, correctness, the commit-on-ack **diff** (`diff/*`), round-trip against a port of the Rust model, hostile, racing, differential (synthetic **and real-driver**) |
| `kf_corpus.cpp` | builds the differential corpus (host-only, `g++`, no GPU) |
| `kf_real_tables.py` | ★ extracts page tables a **real NVIDIA driver** wrote, out of a `.rec` capture |
| `kf_report_emit.c` | ★ the C half of the C↔Rust report seam: `offsetof` manifest + a report |
| `kf_bench.cu` | ★ the walk's **cost**, against the design's own table sizes |
| `corpus/` | `corpus.bin` (15 synthetic images) + `real_ga106.bin` (5 **real** ones) + each one's `*_leaves.txt` (the Rust walker's decode) |
| `run_on_box.sh` | ships the source out of a commit, builds and runs, returns a log |
| `evidence/` | dated run logs, each naming its GPU and driver |

The Rust half of the differential is
`crates/kayfabe-mmu/tests/walk_kernel_differential.rs`, and the report ABI's own seam test is
`crates/kayfabe-mmu/tests/walk_report_seam.rs` against `kayfabe-mmu/src/walkreport.rs`.

## Building

```
make check          # invariants, build, PTX-only check, known-positive
./kf_tests          # the suite;  ./kf_tests hostile   filters
make kf_bench && ./kf_bench          # the cost measurement (see THE WALK'S ACTUAL COST)
./run_on_box.sh tag # from a workstation, against a disposable CUDA container
```

The real-driver corpus is regenerated locally, with no GPU:

```
python3 kf_real_tables.py                                   # -> corpus/real_ga106.bin
KF_WRITE_EXPECTED=1 cargo test -p kayfabe-mmu --test walk_kernel_differential
```

`nvcc -gencode arch=compute_75,code=compute_75` — **PTX only, no cubin**, so the
driver JITs forward onto any Turing-or-later part. `make check-ptx` asserts the
binary contains no SASS, which is what makes "it JITs forward" a checked claim
rather than a compiler flag someone believed in.

## The three invariants

They are properties of the **shape of the code**, and `make check-invariants`
asserts each one in the source rather than arguing for it.

**I1 — no loop terminates on guest data.** The descent is five literally nested
`for` loops with format-constant trip counts (4, 512, 512, 256, 32×16), one per
level. No `while`, no `goto`, no recursion on the guest-data path, and no way to
express a sixth level. ⇒ **A cycle is harmless rather than detected**: a PDE
pointing at its own page is read again one level down and then the nesting runs
out. `KFWR_R_TOO_DEEP` exists in the ABI as the name of a refusal this design
does not need, and every hostile case asserts it is **never set**.

**I2 — every dereference is preceded by a bounds check.** `KF_GPGA_DEREF` is the
only expression that touches the buffer; it appears exactly once outside its own
definition, inside `kf_load64`, immediately after the compare. A table's extent
and alignment are checked once more at each descent, so a table that merely
*straddles* the end is refused before its first entry.

**I3 — output is capped and truncation is loud.** Three independent caps (runs
per address space, entry budget per walk, runs in the report), each setting
`KFWR_HF_TRUNCATED` and its own `refuse_mask` bit. **A truncated walk does not
install its table**, so it can never become the baseline a later delta is
computed against; the next refresh is a full resync. ⊘ 2026-09-25: a TRUNCATED report is
never COMMITTED — even an ack of it is ignored (`diff/truncated_is_never_committed`).


## ★★★★★ The format seam — one program, the layout as data

`THE_CONSTRAINTS.md` §21: *"our ptx must be Turing+ compatible … you need to
support both the turing/ada page tables as blackwell table, in same kayfabe …
I would avoid shipping two cuda program"*, and *"that kind of config you already
derived from ABI on host … I would call this setup data alongside table
version."*

The compile target was already right (`compute_75`, PTX-only). The **format** was
not: the kernel hardcoded VER2 bit positions, i.e. Turing→Ada, while Hopper and
Blackwell are VER3 — so deleting the host's format-polymorphic parsing would have
capped the product at Ada silently.

**`KfFormat` is now the only place a bit position is written down**, it is host
code that runs once, and everything below the seam reads it:

| the descriptor carries | e.g. VER2 |
|---|---|
| `dir[KF_DIRS]` — per level: `active`, `va_lo`, `entries`, `entry_bytes`, `leaf_ps` | PD3[48:47] … PD0[28:21], 16-byte dual |
| the two leaf tables: `big_va_lo`/`small_va_lo`, entry counts, widths, page-size codes | 20:16 ×32 and 20:12 ×512 |
| `valid_bit`, `ap_lo`/`ap_bits`, `pde_ap_invalid`, `pte_ap_map[4]`, `pde_ap_map[4]` | bit 0; 2:1; 0; `{vid,peer,sys,sysnc}` |
| `addr_sel[4]` + `addr_local`/`addr_sys` + `big_addr_local`/`big_addr_sys` as `{lo,bits,shift}` | `{8,25,12}`, `{8,46,12}`, `{4,29,8}`, `{4,50,8}` |
| `bit_volatile`, `bit_privilege`, `bit_read_only`, `bit_atomic_disable` | 3, 5, 6, 7 |
| `pcf` + `pcf_sparse` — the part that is *not* a moved field | unused on VER2 |
| `ps_log2[4]`, `root_align`, `first_dir`, `abi_version`, `table_version` | |

`make check-invariants` greps that **no VER2 geometry constant is left in the
kernel**, and the report's header now carries `ps_log2[4]` so the **host's parser
needs no format knowledge at all** — which is what the format doc asked for.

### ★★★ I1 survives "format as data", and here is why

The nesting is still literal — `KF_DIRS + 1` textually nested `for` loops — and
every trip count reads `i < KF_MAX_ENT && i < <descriptor>`. **A descriptor can
only make a loop shorter.** It cannot make one unbounded and it cannot add a
level, because the nesting depth is a property of the source text, not of the
data. `check-invariants` asserts both: at least five loops carry the compile-time
cap, and `KF_DIRS`/`KF_MAX_ENT` are `#define`s rather than descriptor fields.

`KF_DIRS` is **5**, one more directory slot than VER2 needs, because VER3 adds
`PD4[56]` on top. A shallower format marks the leading slots inactive and they
cost one pass-through iteration each.

### The two switches, and what they are actually for

⊘ The brief expected VER3's **PCF** to need a switch because it "replaces the
discrete permission bits". Read against `ogkm` `hopper/gh100/dev_mmu.h:498-530`,
**that is not what PCF does to the four fields we decode**: the enumerants are a
bit-field (`REGULAR_RW_ATOMIC_CACHED`=0 → `_UNCACHED`=1 → `PRIVILEGE_`=2 →
`_RO_`=4 → `_NO_ATOMIC_`=8), so volatile/privilege/read-only/atomic-disable simply
**moved** from bits 3,5,6,7 to bits 3,4,5,6. Field offsets cover them.

What genuinely is not a moved field is **SPARSE**: VER2 spells it *"valid clear,
VOLATILE bit set"* (a bit test); VER3 spells it as a **value** of PCF
(`_PTE_PCF_SPARSE = 1`, an equality against a 5-bit field). That, and the
"is this directory entry present" predicate, are the two `switch (table_version)`
sites — and both are **live**, because the report now counts sparse slots.

★ Both branches are **warp-uniform**: every thread of a launch walks the same
guest's tables in the same format, so `table_version` is the same value in every
lane and the branch costs a predicate, not a divergence. Divergence is the usual
objection to branching in a CUDA kernel and it does not apply here. Do not
"optimise" this into a template or a second kernel — §21 is explicit that we ship
one program.

### ⚠⚠ VER3 is a SKETCH. It has never run.

There is no Hopper or Blackwell in this project and nothing here has ever decoded
a VER3 table. `kf_format_ver3_untested()` is read off `ogkm` 610.43.02
(`hopper/gh100/dev_mmu.h:413-536`, `kern_gmmu_fmt_gh10x.c:33-115`), cited line by
line, and **`kf_create` refuses it** unless `KF_ALLOW_UNTESTED_VER3` is defined.

The only claim ever asserted about it is `make check-ver3-sketch`: the sketched
descriptor is **well formed** — its level count, fan-outs, entry widths, root
alignment and big/small coverage satisfy `kf_format_check`. That says nothing
about whether a VER3 table decodes correctly; it exists so a typo in the sketch
fails at build time rather than on Blackwell day one.

⇒ **Adding VER3 for real is a descriptor plus two switch arms**, and the arms are
already written. Nothing else in the kernel has to move.

### The seam's own known-positives

"The descriptor is used" is unproven until a **wrong** descriptor breaks
something. `make check-seam-negative` perturbs the setup data by one bit at a time
and requires failure, with an unbroken control:

| flag | perturbs | must break |
|---|---|---|
| `-DKF_BAD_DESCRIPTOR` | `addr_local.lo` 8 → 9 | every decoded address |
| `-DKF_BAD_GEOMETRY` | `dir[3].va_lo` 29 → 28 | every VA under PD1 |
| `-DKF_BAD_ABI` | `abi_version` + 1 | `kf_create`, **at launch**, by name |

⚠ **One of the closure known-positives went vacuous during this refactor.**
`-DKF_BREAK_ORDER`'s injection site lived inside the walk loop the seam replaced,
so the flag silently became a no-op and its binary was identical to the control.
It was caught only because `check-closure-negative` *requires* that control to
fail. A control that merely reported would have gone on passing.

## ★★★★★ TABLES A REAL NVIDIA DRIVER WROTE — `differential/real_driver_tables`

⊘⊘⊘ Every other case in this suite builds its tables with `kf_tables.h`, **our own builder,
encoding our own understanding of VER2**. The differential against the Rust walker does not
close that gap: both decoders share the understanding. Only tables a real driver wrote can.

`corpus/real_ga106.bin` is **five address spaces a stock, unpatched NVIDIA open 580.159.04
guest driver built on a real GA106**, lifted out of `traces/cap1b_coldboot_hermetic_d6.rec`
by `kf_real_tables.py`. No new capture was needed — that trace records every trapped MMIO
access, and the driver builds its tables *in the stream*: the directory spine through the
**PRAMIN window** (BAR0 `0x700000..`, base latched at `0x1700`), the leaves through the
**BAR2 self-map**, whose addresses are *virtual* and must be translated through the very
tables being built.

★★★ **The self-consistency proof: all 177 856 BAR2 writes translate, with ZERO misses.** A
wrong field position, a wrong fan-out, or the dual PDE's two halves the wrong way round could
not do that. ★ Independently corroborated: the `UPDATE_BAR_PDE` RPC recorded **inside CPU-RM**
by a different instrument (`traces/rpctrace_ga106_boot1.bin`, fn 70, `barType=1`,
`levelShift=47`) carries `entryValue=0x2efbc302` — **bit-identical** to the root PDE
reconstructed here from MMIO.

`[measured 2026-09-14, RTX 3060]` **7 005 leaves, five images, agreed EXACTLY** between the
kernel and `kayfabe-mmu`'s Rust walker.

### ⊘⊘⊘ AND THE FINDING — real tables carry two fields our builder cannot produce

| field | real driver | `kfb_pte()` |
|---|---|---|
| `KIND` (63:56) | `9` on 6 017, `6` on 959, `0` on **22** | always 0 — unreachable |
| `COMPTAGLINE` (55:36) | non-zero on 6 017 | always 0 — unreachable |

**99.7 % of real leaf entries are encodings no case in this suite had ever contained.**
`kfb_pte()` only ever writes bits 0..7 and the address field.

- ✔ **The decode is unaffected**, and that is now checked rather than hoped: VER2's vidmem
  address field is 32:8, below both, and both decoders mask them off identically.
- ⊘ **The COALESCER is affected, and nothing had decided it.** Run identity is *(contiguous
  VA, contiguous GPGA, equal decoded flags)* and the decoded flags carry no `KIND`. On the
  real 12 GiB address space at root `0x2efa6c000`: **1 run emitted, 3 runs if `KIND` and
  `COMPTAGLINE` were part of the identity**, with two boundaries at which the guest changed
  memory kind across contiguous VA *and* contiguous physical addresses under identical
  permissions. That is very likely the right call — `KIND` is how the *GPU* interprets bytes,
  not where they live, and the host publishes addresses — but it was a **blind spot, not a
  decision**, and it should become one.

Everything else matched what the builder already produces: the sparse encoding is exactly
`VOL` set with `VALID` clear (8 358 occurrences), the dual PDE's big half is the LOW word and
the small half the HIGH word, every permission-bit combination the driver used is one
`kfb_pte()` can spell, and the observed PD0 slot shapes are all shapes the corpus already has.

⚠ **Scope.** These are the driver's **BAR2 / GSP-bootstrap** address spaces. They are *not*
the ~1872-page user/compute working set of w422 — that was a live count on a rented box and
the pages were never persisted anywhere in this tree. `cap1b` is a hermetic emulator capture,
so the *physical addresses* are the emulator's FB layout; the **entry encodings are the real
driver's**, and that is what is under test. `kf_real_tables.py`'s header states the rest.

## ★★★★★ The C↔Rust report seam — `crates/kayfabe-mmu/tests/walk_report_seam.rs`

⊘ Until w725, `grep -rn "ReportHeader\|kf_report" crates/ --include=*.rs` returned **nothing**.
The report — the struct increment 6 consumes — was declared in `kf_walk.h` and nowhere else,
and the differential above compares through a *corpus file*, so it never touched the report
ABI at all.

`kayfabe-mmu/src/walkreport.rs` is the parser (header + `PdbEntry[]` + `MapRun[]`,
bounds-checked, every refusal named, and **no format-version knowledge** — page size resolves
through the header's `ps_log2`). `kf_report_emit.c` is the C half: it prints the **C
compiler's own `offsetof`/`sizeof`/`_Alignof`** and every `KFWR_*` constant, then writes a
report *and* an independent account of what it put in each field. The test compares the Rust
pins against that compiler and the Rust parse against that account — neither side is something
Rust produced.

★ **Why pins and not a round trip:** `NVOS34` (`kayfabe-abi/src/submit.rs`) — a field at +12
instead of +16 encodes, decodes and round-trips **against itself**, with every byte wrong.
The known-positive `-DKF_SEAM_BREAK_LAYOUT` transplants that exact defect (a 4-byte hole
before `MapRun::flags`) and the test requires the catch **and** requires it to name that field
at +28, so a break caught for the wrong reason still fails.

⊘ **Nothing was at a wrong offset — but two things were wrong anyway.** `kf_walk.h`'s own
comment still claims the header reaches 64 bytes with `pad`, when it does so with
`sparse_slots` + `ps_log2[4]`; and **`kf_refresh` does not produce one report buffer at all**
— it does three `cudaMemcpy`s sized by *count* into three separate host arrays
(`kf_walk.cu:1286-1290`), so the format doc's *"a small output buffer"* does not yet exist.
The parser reads both shapes (`parse_parts` and the packed `parse`).


## ★★★★★ The parallel walk — level-synchronous, one warp per table

`[w725]` the walk was **one thread per address space**, and it cost **450 375 µs**
for the measured working set: 961 540 entries at 468 ns each — one dependent
memory round-trip per 8-byte entry, because a single thread cannot have two loads
in flight. At 1 178 refreshes a boot that is ~9 minutes of GPU time against a
~152 ms honest CPU path: **the kernel was slower than the thing it replaces**, so
`SINGLE_STORE_PLAN.md`'s premise did not hold.

**Only the DEPTH of a page-table walk is serial.** PD3 → PD2 → PD1 → PD0 → leaves
is five levels; the fan-out at each is large, and the leaf level — where
essentially every entry lives — is embarrassingly parallel once the leaf-table
addresses are known. So the walk is level-synchronous: **one kernel launch per
level**, each reading that whole level's tables at once.

```
seed      → one frontier entry per address space
per level → EXPAND  one warp per table: stage it in shared memory, all 32 lanes
                    decode, ballot for the count and for each child's rank
          → SCAN    exclusive prefix sum over the per-parent counts
          → COMPACT copy children to scan[parent] + rank  ← ORDER IS RESTORED HERE
dual      → the same, but the children are TASKS: one per PD0 slot
leaves    → one warp per task; each lane takes a contiguous range of big-page
            chunks, forms runs in it, and the warp joins the pieces
join      → runs that meet across a TASK boundary are re-joined
```

### `[measured, RTX 3060, median of 11, device-synchronised, same harness]`

| case | serial | parallel | |
|---|---:|---:|---|
| **working set** (1 872 PT pages, 961 540 entries) | 450 375 µs | **205.7 µs** | **2 190x** |
| **worst case** (12 GiB at 4 KiB, 3 152 900 entries) | 1 482 071 µs | **390.3 µs** | **3 797x** |
| real GA106 address space (2 116 entries) | 2 197 µs | **110.0 µs** | 20x |
| 16 address spaces in one refresh | 34 157 µs | **204.6 µs** | 167x |
| **1 178 refreshes a boot** | ~8.8 min | **0.24 s** | |

Phase breakdown of the working set (`-DKF_PHASES`): `seed 3 · expand 6/14/14/15 ·
scan 4/5/4/4 · compact 4/4/5/8 · leaf 85 · emit+join 18` µs.

⊘ **The fragmented cases are deliberately NOT optimised.** `frag_every2`
(958 464 runs from 961 540 entries) improved only 1.5x, and that is a choice: the
`1.4 µs × runs` term was derived by fragmenting on purpose, and **the real 12 GiB
guest address space from the driver trace emits one run**. Optimising the
per-entry descent was the whole job; contorting the design to make run-forming
cheap would trade a real input for a synthetic one.

### How the two measurements that mattered were got — and two wrong guesses

★ The first parallel version was **2 129 µs**, ten times the target, with ~610 µs
of it *fixed* — a 2 116-entry walk cost as much as a 961 540-entry one. Two
guesses at why (empty blocks past the real frontier; block-dispatch cost with
18 KB of shared memory) were **both wrong**: converting every kernel to a
grid-stride loop moved it by 16 µs, and dropping the grid from 512 blocks to 128
moved it by nothing.

`-DKF_PHASES` settled it in one run: `cudaEvent`s around each phase showed three
`expand` launches at **~158 µs each on a frontier of one entry**. That is 275
cycles per table entry — and `ptxas -v` says **0 bytes of spill**, so it was not
register pressure. It was simply the throughput one lane gets while thirty-one
sit idle. The same measurement then showed the leaf phase at 1 741 µs for the
same reason.

⇒ `__ballot_sync` in the expand (158 → 14 µs) and a warp-split coalescer in the
leaf phase (1 741 → 85 µs). **Measure before reasoning; the instrument was worth
more than either hypothesis.**

### How the compaction preserves run identity and ordering

- **Children are written at `scan[parent] + rank_within_parent`.** An `atomicAdd`
  reservation would have been simpler and would have scrambled the order the
  per-class diff and `walkdiff` both depend on. The scan is what makes the
  frontier — and therefore the task array — come out in exactly depth-first
  order.
- ⇒ **Concatenating each task's runs in task order reproduces the serial emission
  stream exactly**, including the big/small interleave inside a dual slot. The
  only runs the serial walk would have joined and a concatenation would not are
  those meeting at a boundary, and both boundary levels re-join them by the same
  rule: *the follower contributes one run fewer and adds its first run's LENGTH
  to the run its predecessor already wrote.*
- At the **task** boundary that add is a separate kernel launch; at the **lane**
  boundary it is after a `__syncwarp()`. Same reason both times: a plain store of
  `len` and an atomic add to it would otherwise race, and the store would win.
- An **empty** task or lane breaks a chain, and that is correct rather than
  convenient — it emits nothing only when its VA range holds no mapping, and a
  hole in VA breaks contiguity anyway.

⊘ **One known difference from the serial walk, in the hostile direction only.**
The serial coalescer carries its open run across a task that emitted *nothing*,
so a mapping either side of an all-refused task could join. That needs every leaf
in the task refused, which only a misaligned *large* leaf can do. In that one case
the parallel walk reports one more run. The mapping SET is identical, which is
what every consumer and every test asserts.

### The invariants, and what changed

**I1 is STRONGER, not weaker.** The depth of the walk is now the host's launch
loop, bounded by `KF_DIRS`, a compile-time constant: there is no recursion to
bound because **a level is a kernel launch**. A cycle in the guest's tables
produces frontier entries at the next level, and there is no level after the
last. `check-invariants` asserts the loop's exact form, that no `while` appears on
any parallel guest-data path, and that eight parallel loops carry the
`KF_MAX_FRONTIER` compile-time cap.

**I2 is unchanged, and now covers more.** `kf_win_load` is the single caller of
the single dereference, and every path — serial and parallel — goes through it.
⊘ **The load is still `volatile`.** The `race/*` cases depend on each dereference
being a real read of current memory, and the parallel walk gets its memory-level
parallelism from *threads* rather than from letting the compiler batch loads, so
nothing was given up for speed. All four racing cases pass unchanged.

**I3 gains two caps** — the frontier and the run staging area — both loud
(`KFWR_R_FRONTIER_CAP`). ⚠ It also gained a distinction it needed: running out of
**run** slots truncates the report but the walk must still write what fits, while
running out of **budget or frontier** stops the walk itself. Conflating them made
a run-cap truncation emit a report of uninitialised runs.

### The count and the write read ONE snapshot

Every table is read from GPGA **once**, into shared memory; the counting sweep and
the writing sweep both read that snapshot. ⊘ That is not an optimisation, it is
what makes the walk correct while the guest mutates the tables underneath it: two
sweeps reading global memory would disagree about how many runs a task has, and
the writer would then overrun its neighbour or leave a hole. It was the racing
cases that forced the design, not the benchmark.

## ★★★★★ Delta round-trip closure
> ⊘ **HISTORY (superseded 2026-09-25):** see the block at the top; the closure now checked is
> `roundtrip/diff_*` — report == model diff, then settled slot == walk.

    apply(model, deltas_from_walk_N) == full_walk_N        for all N

The nine `delta/*` cases each build one change and assert one expected delta.
Nothing accumulates, so nothing checks **closure** — and a stream of individually
plausible deltas can drift silently. `roundtrip/*` does the accumulating: keep a
host-side model of the mapping set, then for hundreds of seeded random steps
mutate the tables, walk, **apply the report to the model**, and compare against a
fresh full walk of the same tables.

`[measured 2026-09-14, RTX 3060]` **2000 steps over 8 seeds, 2000 closures
checked, 1105 non-empty deltas, 1525 steps that changed the mapping set, 416
steps with BOTH halves of a dual PDE live, max model 392 mappings, deepest walk
10 380 entries.** Benign and hostile streams, plus one under a live racer.

Three things make it worth more than the cases it complements:

- **It is not coupled to the shadow.** "A fresh full walk" comes from a *second*
  walker that is **never acked**, so every report it emits is a `RESYNC`, i.e.
  the full current state. If the open design question settles on the kernel
  returning state instead of deltas, `rt_full_state` is unchanged and
  `model_apply` becomes the host-side diff this same loop tests.
- **It asserts order-independence, not just closure.** Every report is applied
  forwards *and backwards* and both must give the same model. That is a stronger
  property than "apply them in the order given", and it is what the per-class
  segment diff exists to provide.
- **It says when it was vacuous.** Closures checked, deltas that were non-empty,
  steps that actually changed the mapping set, steps where both dual-PDE halves
  were live, maximum model size and deepest walk are all asserted above floors.
  A stream that scribbled the root would pass every closure check while the model
  stayed empty; these are what stop that reading as a result.

`roundtrip/under_racer` is **deliberately scoped**: under concurrent mutation
there is no "the tables at step N" for a second walk to agree with, so per step
it asserts only well-formedness and order-independence. The exact claim is made
once, at the end — stop the racer, take one more delta, apply it, and the model
must equal a fresh full walk of the now-quiesced tables. A racing round-trip that
asserted a specific value would be asserting nothing.

`roundtrip/truncated_is_never_a_delta` closes the loop on I3: a `TRUNCATED`
report is refused by the applier, the walker is not acked, and the next report is
a full resync that reconstructs the state exactly — **and the case demonstrates
the divergence**, by applying the truncated report to a copy of the model and
asserting the copy is now wrong.

### The five known-positives

`make check-negative` and `make check-closure-negative` rebuild the suite with
one decision removed at a time and **require the corresponding cases to fail**,
with an unbroken control each time. Without them these are all green over checks
nobody has ever seen fire.

| flag | removes | must break |
|---|---|---|
| `-DKF_BREAK_BOUNDS` | the two bounds checks | `hostile/pointer_past_end`, `hostile/bounds_window_respected` |
| `-DKF_OLD_MERGE` | restores the superseded whole-run merge join | `roundtrip/*`, on order-independence |
| `-DKF_BREAK_ORDER` | walks the big table downwards, so one class comes out descending in VA | `roundtrip/*`, on order-independence |
| `-DKF_DROP_UNMAP` | drops every `UNMAP`, and nothing else | `roundtrip/*`, on **closure itself** — and it must *not* disturb order-independence |

★ Each step also greps for *which* assertion fired: a negative control that fails
for a different reason than intended is worth nothing, and `-DKF_OLD_MERGE` and
`-DKF_BREAK_ORDER` both trip the order assertion *before* the closure one, which
is why `-DKF_DROP_UNMAP` exists at all — it is the only one that reaches
`apply(model, delta) != full walk`.

★ What `-DKF_BREAK_BOUNDS` showed is worth keeping: with the check removed, one
case **silently returned data from beyond the declared window** (it reported a
mapping at `gpga=0xdead000` that lives past the end) while the other raised *"an
illegal memory access"*. **An out-of-bounds read does not reliably fault.** That
is the whole argument for making the check structural rather than probable.

★ What it showed is worth keeping: with the check removed, one case **silently
returned data from beyond the declared window** (it reported a mapping at
`gpga=0xdead000` that lives past the end) while the other raised *"an illegal
memory access"*. **An out-of-bounds read does not reliably fault.** That is the
whole argument for making the check structural rather than probable.

## ★★★★★ MINIMALITY — the fewest mmaps, not merely the right ones `[w741]`

> *"If two BAR PTEs are adjacent, in GPGA and in BAR VA, then it is ONE consolidated mmap and
> not several … the goal is to minimise the amount of mmaps or VA-space maps of an RM object
> to its minimum while remaining correct."*

⊘ **Correctness was the only thing this suite ever asserted, and it is half the bar.** A report
that is right and twice as long costs twice the host mappings, and every oracle here — closure,
order-independence, the model comparison — is blind to that by construction.

**The minimality oracle** (`check_minimal`, `kf_tests.cu`) reads the OUTPUT: no two adjacent runs
in one address space may have the same `op`, the same `flags` (which is run identity, so class and
kind ride along), contiguous VA **and** contiguous GPGA. It does not care which of the four
coalescing sites lost the join.

**The op budget** (`st_ref_ops`) is the other half: a reference delta built page by page and
re-coalesced under the same rule, giving the fewest runs the report could have been. The kernel
may not exceed it. `[measured 2026-09-15, GTX 1650 sm_75]` over 120 randomised steps the kernel hit
that minimum **exactly** every time — `worst_slack=0`.

### The known-positive matrix — and what it measured that reading the code did not

Three flags each disable EXACTLY ONE coalescing site and nothing else. **The mapping set is
identical under all three**, which is the point: these are failures no correctness oracle can see.

| case | `KF_BREAK_COALESCE`<br>(leaf accumulators) | `KF_BREAK_SEG_COALESCE`<br>(the delta's `kf_seg_emit`) | `KF_BREAK_JOIN`<br>(task-boundary join) |
|---|---|---|---|
| `enlarge_at_end` / `_start` | **red** | green | green |
| `shrink_at_end` / `_start` | **red** | green | green |
| `drop_whole_run`, `add_between_runs` | **red** | green | green |
| `add_adjacent_forcing_merge` | **red** | green | green |
| `split_run_in_middle` | **red** | green | green |
| `one_run_replacing_two` | **red** | **red** | green |
| `enlarge_across_pt_boundary` | **red** | green | **red** |
| `coalesce_stress` | **red** | **red** | **red** |

⚠ **Read the second column.** Eight named cases stayed GREEN with the delta's own coalescer
deleted — because each of them produces ONE segment, and a coalescer that never sees a second
neighbour cannot be caught losing one. `one_run_replacing_two` (a `cur` run covering several
`prev` runs — the w722 shape) is the only named case that reaches it. ⊘ **That was found by
building the matrix, not by reading the code**, and it is the reason the matrix exists rather than
a single "the tests go red" claim.

⚠ And the third column is why `enlarge_across_pt_boundary` exists at all: a run that grows across a
page-table boundary is re-joined by a *different* mechanism from one growing inside a table
(`kf_par_heads` / `kf_par_join`), and every other delta case lives inside one `PT_SMALL` and
cannot reach it.

`make check-coalesce-negative` runs the whole matrix; it is part of `make check`.

### The host-side twin — and a real defect it found

`crates/kayfabe-mmu/src/walkdiff.rs` is this kernel's host-side alternate. It **cut at every
boundary either side introduced and never re-joined**, so one `cur` run replacing two `prev` runs
produced **two** `Remap`s describing one contiguous re-point. Closure held; every test in that
module passed; the two implementations disagreed about how many `mmap`s a given guest change
costs. Fixed by `coalesce_ops` there, under this same rule.

## What the format doc got wrong or left unbuildable

1. **`struct ReportHeader { // 64 B }` lists fields summing to 56.** Nothing can
   be built from a contradiction. This header adds `refuse_mask` (the doc's
   `reserved`, given a job) and 8 bytes of `pad` to reach the stated 64.
   `refuse_mask` is what lets a hostile test assert the refusal is *the one it
   intended* rather than merely that something was refused.
2. **"every `first_run + run_count <= run_count`"** is a typo for
   `<= header.run_count`. Implemented as the latter.
3. **A large leaf's target need not be aligned to its own page size.** VER2
   carries a 4 KiB-granular address field at *every* leaf level, so a hostile
   guest can spell a 512 MiB page whose physical base is 4 KiB-aligned and
   nothing else. The doc's validation rule *"every `len` non-zero and
   page-aligned"* has no counterpart for `gpga`, and the encoding is why it
   cannot have one. The kernel refuses such a leaf by name
   (`KFWR_R_MISALIGNED_LEAF`) rather than reporting a mapping nobody can stand
   behind. ⚠ It was found by the doc's own property-3 validator, not by an
   expectation — the validator caught a case the test author had not thought of.
4. **"an unaligned pointer" is inexpressible below the root.** Every VER2 table
   pointer is a bit-field shifted by exactly the bits its target's size needs
   (PDE `<< 12` into a 4096 B table; the dual PDE's big half `<< 8` into a 256 B
   table), so the alignment half of the check can only fire on the **root**,
   which arrives from outside the format. `hostile/unaligned_inexpressible`
   keeps that as a checked claim so a future format change breaks a test instead
   of silently making the check reachable.
5. **The doc does not say how a dual PDE's two halves are ordered**, and a report
   "produced already sorted" needs a total order because both sub-tables cover the
   *same* 2 MiB of VA. The walk emits **big before small inside each 64 KiB
   chunk** — (VA ascending, page size descending).
   ⊘ An earlier draft of this file said *"the diff's comparator must be the same
   one"*. **That coupling is now dissolved rather than merely verified**: the diff
   is computed **per page-size class**, so it does not depend on the cross-class
   order at all. What it does depend on is each class being ascending in VA, and
   that has its own known-positive (`-DKF_BREAK_ORDER`) and is asserted by 416
   round-trip steps in which both halves were live at once.

7. **A whole-run merge join does not satisfy closure, and the doc's format
   invites one.** Diffing prev against cur by comparing whole runs on the key
   `(va, page size)` — the obvious reading of *"a merge join — linear, no hashing,
   no allocation"* — emits, for a cur run that covers several prev runs, one
   `REMAP` followed by `UNMAP`s of the runs it has just replaced. Applying that
   report in order **loses those mappings**. All nine single-step `delta/*` cases
   pass against it. The diff is now computed **per page-size class at segment
   granularity**: `UNMAP` where prev covers and cur does not, `MAP` where cur
   covers and prev does not, `REMAP` where both cover and differ.

8. **⇒ A report's runs are order-independent, and the doc should say so.** Under
   the segment diff the `UNMAP` set and the `MAP`/`REMAP` set are disjoint in
   `(va, page-size class)` **by construction**, so a host may apply a report in
   any order — asserted directly, by applying every report backwards as well as
   forwards. ⊘ A consequence: the **report's** own order is now (page-size class
   ascending, VA ascending within a class), which is *not* the walk's order. The
   format doc's "produced already sorted" describes the kernel's table, not the
   delta.
6. **Scope pruning happens at 512 MiB granularity and no finer** (whole PD1
   subtrees and above). A hint is widened to that granule before use, which makes
   every carry-forward boundary aligned to a whole PD1 slot, so a clipped run can
   never be cut below its own page size. The doc's asymmetry — *the hint can only
   make the walk faster, never wrong* — is asserted directly by
   `scope/cannot_make_the_walk_wrong`.

## ★★★★★ THE WALK'S ACTUAL COST — measured, and it is not 67 µs

`[measured 2026-09-14, RTX 3060 sm_86, driver 550.107.02, median of 11 after 3 warmups;
evidence/w725_bench_2026_09_14_rtx3060_550.107.02.log]`

`dirty_tracking_without_uffd.md` and `gpga_is_one_reserved_object.md` size increment 6's
refresh budget on **"a full walk is ~67 µs"**. `kf_bench.cu` measures it against the two
sizes those docs name. ⊘ **The number is the deliverable; nothing here asserts a threshold.**

| | entries | runs | refresh | vs 67 µs |
|---|---:|---:|---:|---:|
| **measured working set** — 1872 PT pages, 7.3 MiB, 3.66 GiB at 4 KiB | 961 540 | 1 | **462 ms** | **6 894×** |
| **worst case** — 12 GiB at 4 KiB, 6144 PT pages, 24 MiB | 3 152 900 | 1 | **1.513 s** | **22 586×** |
| the same 12 GiB at **2 MiB** | 13 316 | 6 144 | **26.8 ms** | 400× |
| real GA106 driver tables, largest of five | 8 228 | 6 | 4.5 ms | |

⇒ **The refresh budget does not close.** 1178 refreshes a boot × 462 ms = **544 s ≈ 9 minutes
of GPU time per boot**, for the *measured* working set, not the worst case.

### The cost model, and it fits within 0.5 %

    t  ≈  0.48 µs × entries_visited  +  1.4 µs × runs

Obtained by holding entries constant at 961 540 and fragmenting the tables so the same walk
produces 1 → 29 952 → 119 808 → 479 232 → 958 464 runs (462 ms → 1.81 s). ⇒ **the descent
dominates** at any realistic run count; coalescing only becomes comparable when nearly every
page is its own run.

### ⊘⊘⊘ Three things the measurement kills, and one it opens

- ⊘ **"Most refreshes find nothing changed" does not rescue it.** An *acked* refresh over
  unchanged tables emits an empty delta and costs **466 ms** — the same. The descent happens
  regardless of whether anything moved; that is the design's own "liveness is derived from
  reachability, never tracked", priced.
- ⊘ **It is not a bandwidth problem, so it is not a hard floor.** 961 540 entries × 8 B =
  7.7 MB; at the 3060's ~360 GB/s that is **21 µs**. The 67 µs target is ~3× the bandwidth
  bound — **comfortably achievable in principle**. This implementation misses it by ~6 900×
  because it uses **one thread of the 43 008** the part can run.
- ★★★ **And the parallel axis proves exactly that.** One refresh over N address spaces, one
  thread each: **16× the entries for 1.18× the time** (29.9 ms → 35.2 ms; 448 → 33 ns/entry).
  The walk is **latency-bound, not bandwidth-bound**: each thread is stalled on a dependent
  global load almost all of the time. 480 ns at 1.78 GHz is **~854 cycles per 8-byte entry**,
  which is one full memory round-trip — consistent with `KF_GPGA_DEREF` reading through a
  `const volatile uint64_t *`, which by construction forbids the compiler from batching or
  reordering the loads within a page table. ⚠ Named as the mechanism the evidence points at,
  **not isolated**: no build with the qualifier removed has been measured, and it cannot
  simply be dropped — the `race/*` cases depend on seeing the guest's concurrent writes.

⇒ **The fix is parallelism, and the measurement says it will work**: the README's own
"parallel decomposition over (PD3, PD2) subtrees" is the right next step, and the 16-VAS
result is direct evidence that the hardware has the throughput sitting idle. A second,
independent lever is **page size** — the same 12 GiB costs 56× less at 2 MiB than at 4 KiB.

⚠ Do not read these as a tuned number. Nothing was tuned; `kf_bench` times the walk exactly as
`kf_tests` exercises it, which is the point.

## Deliberate scope limits

- **One thread per address space.** Correctness and a deterministic, already-
  sorted run order came first; the doc's *"a full walk is ~67 µs"* is not a claim
  this implementation makes — and is now **measured at 6 894× off** for the design's own
  working set (see above). A parallel decomposition over (PD3, PD2) subtrees is
  the obvious next step and would need a boundary-joining compaction.
- **The diff is single-threaded**, because it is a linear scan over two sorted
  lists per page-size class, producing a dense report.
- **`GONE` carries no runs.** The doc leaves the choice open; a whole-VAS teardown
  is one flag, not N unmaps.

## ★★★★★ TURING+ IS A HARD REQUIREMENT — what is met, and the one thing that is not

**Owner, 2026-09-14, stated three times:** *"our ptx must be Turing+ compatible."*

### ✔ Met and verified on the PTX side

| | status |
|---|---|
| compile target | `-gencode arch=compute_75,code=compute_75` — **Turing**, PTX embedded, **no cubin** |
| "it JITs forward" is checked, not believed | `make check-ptx` fails if `cuobjdump -sass` finds any `code for sm_` |
| arch-specific intrinsics | **none** — no `__shfl`/`__ballot`/`redux`/`cp.async`/`mbarrier`/cluster ops |
| forward-JIT demonstrated | ran on **sm_86** from the `compute_75` target |

⚠ **Never run on actual Turing silicon (sm_75).** sm_75 is the floor we claim; a container-hour
closes it. Until then "Turing+" is *"compiled for Turing, proven forward to Ampere"*.

### ⊘⊘⊘ THE REAL ARCHITECTURE LIMIT IS THE FORMAT, NOT THE PTX

This kernel hardcodes **VER2**, which covers **Turing → Ada**. **Hopper and Blackwell use VER3**
(PCF; `grep -c PCF` on the Pascal/Turing headers is 0). ⇒ **`grep -niE 'ver3|pcf' kf_walk.cu` →
nothing.**

★★★ **And that bears directly on deleting the host parsing code.** The host walker is
**format-polymorphic** — it takes `fmt: &dyn GmmuFmt` (`kayfabe-fwd/src/ptdecode.rs`,
`kayfabe-mmu/src/reach.rs:640`), so VER3 there is **a new impl, not a rewrite**. This kernel has no
such seam.

⇒ **Deleting the host walker today would delete the abstraction Blackwell needs**, and Blackwell is
goals 1 and 10 of the standing directive.

### ⇒ Sequencing

**Add the format seam to the kernel FIRST, then delete the host parsing.** A template specialised
per format, or a format enum switched at each decode site — cheap while only one format exists,
and much worse to retrofit into a working walker later. Same destination; it just stops the
deletion from quietly capping the product at Ada.
