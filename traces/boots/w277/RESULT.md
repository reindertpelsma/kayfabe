# w277 — RESULT: the 255 straddles are ONE FACT AT TWO GRANULARITIES, and the faulting VA is **BOUND**

**STATUS: LIVE — 2026-08-12.** Branch `w276-port-the-whole-vas-sweep`, boots at `55bfcdf`
(stamp gate PASS: the binary's `kayfabe-rev` equals HEAD exactly). Arms `w277_on` / `w277_off`.
Every number below was read from an artefact opened in this session; none is carried.

---

## ★★★★★ LEAD — FIVE THINGS CONTRADICT THE BRIEF, AND THE FIRST THREE ARE THE RUNG

### 1. ★★★★★ THE FAULTING VA IS **BOUND IN OUR OWN TABLE**. "Our mirror missed it" is DEAD.

`[measured, w277_on]` — the new `TABLE-DESCRIBES` row, joined offline against host `dmesg`:

```
0x7f58_4ce00000: table_rows_printed=88 table_runs=22 →
  ★★★★★ **BOUND** — our table holds 0x7f584ce00000+0x400000 covering it
```

and the same boot's guest-side picture, for contrast:

```
0x7f58_4ce00000: runs_printed=24 → ★★★ LEAF-PRESENT in a run 0x7f584ce00000+0x200000
0x7f58_4ce00000 in a refused_vas list? 0 row(s)
```

⇒ The brief's third state — *"neither bound nor refused … it fell through silently"* — **is
not a drop. It is `bound`.** `w276` could not distinguish *"already in the table"* from
*"lost"*, because a refusal list can only ever answer the second. The address table now prints
its own coverage and the answer is unambiguous: the guest describes the address, **and so do
we**, over the full `+0x400000` run rather than the `+0x200000` the guest's own picture had
reached at that instant.

⊘⊘ **This retires an entire lane.** `LEAF-PRESENT` was being read as *"the guest describes it,
we do not, so publish it"*. We already publish it. **Whatever the GR fault at
`0x…_?ce00000` is, it is not a missing entry in the address table**, and no amount of further
sweeping can address it. `[measured]` `swept_binds=0` over all 88 rows, `truncated_total=0`,
`pages_total=4616` — the sweep is complete, correct, and has nothing left to add.

### 2. ★★★★★ WHY 255 STRADDLE: **an EXTENT mismatch, not a page-size one — and NOTHING is wrong**

`[measured, w277_on]` — **173 of 176** census occurrences are the line below, verbatim (88 log
lines × 2 censuses each: `PT-DECODE` and `PT-SWEEP`; the other 3 are a `510` = 2 × 255 line
where two address spaces settled in one pass). `contradicting=0` on **every** row, **9 distinct
signatures, uncapped**:

```
straddles=255 contradicting=0 sigs={
  SameStartShorter/SameMemory/lvl4/leaf0x10000/live0x80000/pub0=1
  SameStartShorter/SameMemory/lvl5/leaf0x1000/live0x4000/pub0=1
  SameStartShorter/SameMemory/lvl5/leaf0x1000/live0x8600/pub0=1
  SameStartShorter/SameMemory/lvl5/leaf0x1000/live0xea000/pub0=1
  InsideLarger/SameMemory/lvl4/leaf0x10000/live0x80000/pub0=7
  InsideLarger/SameMemory/lvl5/leaf0x1000/live0x4000/pub0=3
  InsideLarger/SameMemory/lvl5/leaf0x1000/live0x8600/pub0=7
  InsideLarger/SameMemory/lvl5/leaf0x1000/live0xea000/pub0=233
  CrossesEnd/SameMemory/lvl5/leaf0x1000/live0x8600/pub0=1 }
first_straddle=[va=0x203e90000 size=0x10000 lvl=4 phys=0x2ef850000/Vidmem
                OVER live=[0x203e90000+0x80000 phys=0x2ef850000/Vidmem published=false]]
```

**Read the live extents: `0x4000`, `0x8600`, `0x80000`, `0xea000`.** Not one of them is a page
size this regime can express (`0x1000 / 0x10000 / 0x200000 / 0x2000_0000`), and `0x8600` is not
even a multiple of `0x1000`. **They are allocation lengths.** ⇒ the two shapes are **a whole
allocation held as ONE table row** versus **the guest's own page tables describing the same
bytes as N page rows**.

★★★ **The accounting is exact and leaves no residue** — four live bindings explain all 255:

| live extent | leaves that tile it | rows |
|---|---|---|
| `0xea000` = 958 464 B | 234 × 4 KiB | 1 `SameStartShorter` + 233 `InsideLarger` = **234** |
| `0x8600` = 34 304 B | 8 × 4 KiB + a 1 536-byte tail | 1 + 7 + **1 `CrossesEnd`** = **9** |
| `0x80000` = 512 KiB | 8 × 64 KiB | 1 + 7 = **8** |
| `0x4000` = 16 KiB | 4 × 4 KiB | 1 + 3 = **4** |
| | | **234 + 9 + 8 + 4 = 255** |

⇒ **ARM A2 (extent) FIRES. ARM A1 (page size) DOES NOT** — the leaf sizes *are* page sizes; it
is the **live** side that is not. **ARM A4 FIRES**: `contradicting=0` means the live binding
already resolves every one of those 255 base VAs to **exactly** the physical address and
aperture the leaf names (see `first_straddle`: `phys=0x2ef850000` on both sides, same start,
different length only). **ARM A3 (stale) does not fire** as a cause — the one `CrossesEnd` is
explained by the un-page-aligned extent, not by staleness. **ARM A5 (contradiction) does not
fire. ARM A6 (published) does not fire** — every signature is `pub0`. **ARM A9 does not fire**
— 9 signatures, no cap.

⇒ **The refusal is CORRECT and the wall is not behind it.** Two sources describe the same
memory identically; they disagree only about **row granularity**, and the address table holds
one shape per range by construction. Relaxing this refusal would have converted 255 correct
declines into 255 shreddings of a correct row — which is exactly what the brief warned against,
and it would have bought nothing, because the table's *answers* were already right.

### 3. ⊘⊘ AND THE 255 WERE **NEVER THE SWEEP'S** — the control produces them with the sweep OFF

`[measured, w277_off]` — `PT-SWEEP arm=off`, **0** `PT-SWEEP` lines, and yet:

```
85 × straddles=255 contradicting=0 sigs={ …the same 9 signatures… }
 3 × straddles=510 contradicting=0
```

⇒ **`w276`'s reading — *"our own table refused everything THE SWEEP produced"* — is wrong.**
Both passes call the same `ReachShadow::settle`, so both report the same **standing** refusal
set; the disarmed control shows it is the **dirty-driven decode pass** that proposes those 255
leaves, and it does so whether or not the whole-VAS sweep ever runs. The sweep neither creates
nor removes them.

⊘ Note what this does to `w276`'s headline *"the sweep added 255 mappings the dirty-driven pass
never publishes"*: it did not. The dirty-driven pass proposes them by itself, and is refused by
itself, on a boot where the sweep is off.

### 4. ⊘⊘ THE THIRD OUTCOME I BUILT AN INSTRUMENT FOR **DID NOT FIRE** — and the instrument is still the finding

`[measured, w277_on AND w277_off]` `shape_collisions=0 dup_leaves=0` on **every** row of both
arms (88 lines per arm, each carrying the `PT-DECODE` census; the armed arm also carries the
`PT-SWEEP` one). The desired-set key collision is **real in the code** and **absent in this
guest**. ⊘ A zero here is a fact about *this guest*, not about the counter:
`tests/tests/straddle_shapes.rs` builds the dual-PDE shape that produces one and asserts it is
recorded, so the counter is known to be able to fire.

★ And the *reason* it is absent is now sourced rather than guessed. RM's own walker guarantees
it cannot arise in a settled tree: `_mmuWalkResolveSubLevelConflicts` **invalidates the other
sub-table over the VA range** on every map, unmap and sparsify (`ogkm-580:
src/nvidia/src/libraries/mmu/mmu_walk.c:476-488, :1066-1092`), and the driver source **nowhere**
states what hardware would do if a VA *were* valid in both. ⇒ the silent overwrite in
`ReachShadow::settle` was a latent defect on a path the driver keeps clean, and it is now
**named** rather than repaired with an invented precedence rule.

### 5. ⊘⊘ A NUMBER IN THE COMMITTED `w276` RECORD IS WRONG, and it read as corroboration for eleven rungs

`w276/RESULT.md` glosses `GpuVa(8655536128)` as *"`0x2_0440_0000` — the 2 MiB region containing
the completion-semaphore page"*. **`8655536128` is `0x2_03E9_0000`. `0x2_0440_0000` is
`8661237760`.** The boot's own `refused_vas` list opens with `0x203e90000`, which the decimal
matches exactly. The refusals are **nowhere near** the semaphore page — they are inside
`0x200000000+0x40aa000`, and `first_straddle` above confirms it at the source.

⇒ A rendered address printed beside a decimal one is a **second source of truth for a value
that was already complete**, and the derived copy is what everyone read. Corrected in
`shim.rs`'s comment, above the sentence it corrects.

---

## WHERE THE LIVE BINDINGS COME FROM — an ELIMINATION over the tree's five bind sites

⚠ **Not measured. Derived, and stated as such**: the refusal carries the live binding's *shape*
and **not its producer**, which is the limit this run pre-registered. But the shape eliminates
four of the five sites that can put a row in an `AddressTable`:

| site | what it binds | excluded by |
|---|---|---|
| `kayfabe-mmu/src/walker.rs:populate` | `leaf.size` — always an enumerated page size | `live0x8600 / 0xea000 / 0x80000 / 0x4000` are **not** page sizes |
| `kayfabe-core/src/gpu.rs:3067` | `declared_by_guest(phys, Aperture::SysmemCoherent)` | the live aperture is **`Vidmem`** |
| `kayfabe-fwd/src/lib.rs:2146` | `real_gpu_memory(.., SysmemCoherent, HostBacking::whole(..))` | aperture **and** `published=false` |
| `kayfabe-fwd/src/lib.rs:2819` | `real_gpu_memory(.., Vidmem, HostBacking::whole(..))` | `published=false` |
| ★ **`kayfabe-core/src/promote.rs:1190`** | `declared_by_guest(r.phys, r.aperture)` at `r.len` — **arbitrary length, guest-declared aperture, `host: None`** | **not excluded** |

⇒ the live side is `promote_ctx`'s **GR context-buffer promotion**, binding each buffer at its
**declared allocation length**. The four extents are the right shape for that set
(`0xea000` ≈ 936 KiB main context, `0x80000` = 512 KiB, `0x8600`, `0x4000`).
⊘ Nothing in the log names the producer, so this is an argument from source, not a measurement.
**The instrument to add next is one field: which populate source placed the row.**

---

## ★★★ AND THE `CrossesEnd` ROW IS A REAL DEFECT — a **sub-page hole in our table**

`[measured, w277_on]`, the table's own runs for `proc=2 pdb=0x201000`:

```
TABLE-DESCRIBES … runs=7 0x200000000+0x40a5600, 0x2040a6000+0x4000, 0x204400000+0xc00000, …
GUEST-DESCRIBES … runs=6 0x200000000+0x40aa000,                     0x204400000+0xc00000, …
```

The guest describes **one** contiguous run to `0x2040aa000`. Our table describes it as **two**,
with a gap from **`0x2040a5600` to `0x2040a6000` — 2 560 bytes**. That is the tail of the
`0x8600`-long promotion (`0x20409d000 + 0x8600 = 0x2040a5600`): the guest maps the whole page
`0x2040a5000–0x2040a6000`, our binding stops **mid-page**, and the `CrossesEnd` refusal is
exactly what stops the page-granular leaf from covering the remainder.

⇒ **A byte-granular allocation length, bound as a table row, makes a range our own
`resolve` answers `Miss` for while the guest's page tables map it.** It is not the GR fault
(that VA is bound), but it is a live inconsistency with a named cause and a visible signature,
and it is the one thing on this rung that a fix should target.

---

## PRE-REGISTERED ARMS — how they fell

| arm | outcome |
|---|---|
| A1 page-size mismatch | ⊘ **did not fire** — leaf sizes are page sizes; the *live* extents are not |
| A2 **extent mismatch** | ★★★★★ **FIRED** — all four live extents are allocation lengths |
| A3 stale binding | ⊘ one `CrossesEnd`, explained by an un-page-aligned extent, not staleness |
| A4 **one fact at two granularities** | ★★★★★ **FIRED** — `contradicting=0` on every row |
| A5 two sources disagree | ⊘ **did not fire** — 0 `Contradicts` signatures |
| A6 live side host-published | ⊘ did not fire — every signature `pub0` |
| A7 aperture mismatch | ⊘ did not fire |
| A8 the 255 does not reproduce | ⊘ did not fire — **exactly 255**, accounted to the leaf |
| A9 >12 signatures | ⊘ did not fire — 9, uncapped |
| B1/B2 dropped at the collision site | ⊘ **did not fire** — `shape_collisions=0` |
| B3 ★ **the fault VA is BOUND** | ★★★★★ **FIRED** |
| B4 unbound, no collision, no refusal | ⊘ did not fire |
| B5 not measured (capped) | ⊘ did not fire — 0 cap markers |
| B6 duplicates without collisions | ⊘ did not fire — `dup_leaves=0` |
| C3 ★ **`124`, fault unmoved — PRE-REGISTERED AS EXPECTED** | ★ **FIRED** |
| C4 fault kind/engine changed | ⊘ did not fire — `Xid 31 / ENGINE GRAPHICS HUBCLIENT_FE / FAULT_PDE / ACCESS_TYPE_VIRT_WRITE / channel 0x00000009` **identical on both arms** |
| C5 the arms differ in a carried counter | ★ **FIRED on `OPERAND-PIN` (2 216 vs 224) — and it is the GUEST, not us.** See below. |

---

## ⊘⊘ A CARRIED COUNTER MOVED 14×, AND IT IS **NOT** AN INSTRUMENT REGRESSION

`OPERAND-PIN = 2216` (`w276b_on`: **156**; `w271_pin`/`w275_pin`: 156). On a build whose only
device change since `67025ca` is the `SEMA-WRITE` predicate's scoping, that reads as this
rung's instruments having changed behaviour. **They did not.** `[measured, both artefacts]`:

```
w276b_on:  operand run 1/1  va=0x204420000 gpa=0x41d5b000 len=131072     (68 rows, ALL "1/1")
w277_on:   operand run 1/32 va=0x204420000 gpa=0x2433b000 len=4096
           operand run 2/32 va=0x204421000 gpa=0x2433a000 len=4096  …    (2 080 rows at "/32")
```

**The same operand, at the same VA, is backed by 32 non-contiguous descending guest pages this
boot and by one contiguous 128 KiB block last boot.** That is the guest allocator's luck, and
`spans()` partitioning it correctly is the pin working, not failing.

⇒ ★ **`OPERAND-PIN` is a bad comparability counter**: it tracks guest physical fragmentation,
not our behaviour. Comparing it across boots invites exactly the false alarm it produced here.
⊘ I nearly reported it as a regression; the artefact is what stopped that.

---

## THE TWO ARMS, side by side — all read from each boot's own artefacts

| | `w277_on` | `w277_off` | baseline |
|---|---|---|---|
| arm assertions (8) | **PASS** | **PASS** (`PT-SWEEP arm=off`) | — |
| `PT-SWEEP` lines | 88 | **0** | control wants 0 |
| `TABLE-DESCRIBES` lines | 88 | **0** ⊘ | — |
| straddle census occurrences | 176 (`255` ×173, `510` ×3) | 88 (`255` ×85, `510` ×3) | — |
| `contradicting` | **0** | **0** | — |
| `shape_collisions` / `dup_leaves` | 0 / 0 | 0 / 0 | — |
| `swept_binds` total | **0** | n/a | 0 |
| `truncated_total` / `pages_total` | 0 / 4 616 | n/a | 0 / 4 616 |
| `DOORBELL-XLATE` | 88 | 88 | 88 (`w271/w274/w275/w276`) |
| doorbells served / forwarded | 201 / 12 | 201 / 12 | 201 / 12 (`w276`) |
| `OPERAND-PIN` | **2 216** | 224 | 156 (`w271/w275/w276b`) — see below |
| `FWD-RING` | **0** | — | 0 — the CE-PT-write producer still never fires |
| `BACKWARDS` transitions | **0** | **0** | 1 false positive in `w276b`, un-scoped predicate |
| `COMPLETION-WATCH … → OBSERVED` | **8**, `NOT-OBSERVED` **0** | — | 8/8 (`w276b`) |
| Xid | `31 … HUBCLIENT_FE @ 0x7f58_4ce00000 FAULT_PDE VIRT_WRITE ch 0x9` | `31 … HUBCLIENT_FE @ 0x7225_c6e00000 FAULT_PDE VIRT_WRITE ch 0x9` | same identity, `w276` ×3 |
| `CUP2_RC` | **124** | **124** | 124 |

★ Five ASLR bases across five boots — `0x7461_86…`, `0x7de8_e2…`, `0x7e6b_42…`, `0x7f58_4c…`,
`0x7225_c6…` — and **identical low 24 bits `0xe00000` on every one**. Graded by identity and
relative position, never by count.

⊘⊘ **AND THE GRADER OVER-COUNTS ONE ROW — caught by reading the artefact, not the summary.**
`w277_run.sh` (inherited from `w276_run.sh`) counts completions with
`grep -c 'COMPLETION-WATCH.*OBSERVED'`, which reports **97**. The real verdict count is **8**:
the pattern also matches two long prose lines (`GUEST-SEMA arm=pin …`, `RUNS: … SEMAPHORE
RUN(S) PLACED …`) that contain both words and no verdict. `grep -oE '→ (NOT-)?OBSERVED'` gives
`8 → OBSERVED`, `0 NOT-OBSERVED`. ⇒ **an unanchored count of a verdict is not a count of that
verdict**, the same class as the `CUP2_RC` / `GCC_CUP2_RC` anchor two sections down — and the
inflated number would have gone into this table if the row had not been re-derived from the
log. Pattern fixed in the runner; the number above is the corrected one.

⊘⊘ **AND A GAP IN MY OWN INSTRUMENT, found by the control.** `TABLE-DESCRIBES` is emitted from
`sweep_cpu_pt_tables`, so it is **silent when the sweep is disarmed** — the control's join
reports `⊘ NO TABLE PICTURE PRINTED — this is NOT 'unbound', it is NOT MEASURED`, which is the
honest answer the refusal-to-guess was built for, and also means **the control cannot
corroborate the `BOUND` verdict**. The table dump has nothing to do with the sweep and should
not be gated on it. Named, not fixed on this rung.

---

## ⊘⊘ WHAT THIS RUN CANNOT PROVE

- **It cannot say what the GR fault IS.** It removes one explanation (a missing address-table
  entry) and offers no replacement. The fault is `FAULT_PDE`, `ACCESS_TYPE_VIRT_WRITE`, from
  `HUBCLIENT_FE`, at a VA our table holds — so the next question is about **which page tables
  the host channel is walking**, not about ours.
- **It cannot name the producer of the live bindings** — that is an elimination over source, not
  a measurement, because no field carries it.
- **It cannot say which of two colliding declarations should win.** `shape_collisions=0` here,
  and the driver source has no precedence rule to be right against.
- **`contradicting=0` is one workload's answer.** It bounds this guest's `cuCtxCreate`, not the
  populate discipline in general.
- **The sub-page hole is not shown to matter.** It is a real inconsistency in our table; nothing
  here demonstrates that anything reads it.
- One workload, one chip (GA106), one driver (`580.159.04`), one boot per arm.
- `w277_off` shares this build; it is a control for the **flag**, not for the code.

---

## ARTEFACTS

| what | where |
|---|---|
| pre-registration | `traces/boots/w277/PREREGISTRATION.md` |
| boot logs, both arms | `traces/boots/w277/` |
| the runner + grader | `scripts/bench/w277_run.sh` |
| the refusal payload | `crates/kayfabe-mmu/src/walker.rs` (`Straddle`, `StraddleShape`, `StraddleAgreement`) |
| the third outcome | `crates/kayfabe-mmu/src/reach.rs` (`Settlement::shape_collisions`) |
| the table dump | `crates/kayfabe-rt/src/device.rs` (`vas_table_ranges`) |
| the tests | `tests/tests/straddle_shapes.rs` (7, all pass) |
