# ★★★★★ THE COVERAGE PREDICATE — `declared ⊆ published`, computed over every element

**STATUS — 2026-09-06 — LIVE.** Instrument work. It answers `w377` §3 blocker (5) and
**grades nothing by itself**: it is a precondition for grading blockers (1)–(4), not a fix
for any of them. ⚠ `w377`'s own §3 list still reads *"(5) The census cannot certify
coverage"*; that entry is **superseded by this document** and the fold could not be made in
place because `w377…refusal.md` lives only on branch `w377-late-map-race`.

## §1 WHAT WAS WRONG WITH THE CENSUS IT REPLACES

`traces/guest_boots/run_w376llmd_qemu.log`, two clauses of one `PT-DECODE` line:

```
refused_vas=[…24 addresses…] ⚠⚠ CAPPED at 24 of 255 distinct — an address ABSENT
                               from this list is NOT thereby un-refused
HOST-PUBLISHED [proc=0 pdb=0x2efa9c000 host_rows=0 of 6254 runs=0 …]
```

Neither can decide *"is publication complete?"*, and for two different reasons:

- **`host_rows=N of M` is a cardinality.** One 64 KiB row and sixteen 4 KiB rows are the
  **same coverage** and a different number, so the quantity moves without the fact moving and
  the fact moves without the quantity moving. Owner, 2026-09-06: *"the only thing that matters
  is that the same ranges are mapped, not the amount of exercised mmaps."*
- **`refused_vas` is a truncated sample**, and its own warning says so. A sample cannot certify
  a universal.

⇒ A fix and a coincidence read identically, which is what makes this blocking for everything
downstream rather than merely untidy.

★ And note the shape: **the cap and the computation were the same loop.** `vas_published_ranges`
built its run list with `.take(cap)` and that list *was* the answer, so the verdict inherited
the truncation. The warning string was honest and could not help — there was no exact number
standing beside the list for a reader to fall back on.

## §2 THE PREDICATE

Per `(proc, gpu, pdb)`, over the **whole** of both sets:

| name | definition |
|---|---|
| `declared` | union of the VA ranges our address table holds (`TABLE-DESCRIBES`) |
| `published` | union of ranges actually installed in the host VAS |
| `COVERED` | `declared ⊆ published` — one boolean per VAS, plus an aggregate |
| `residual` | `declared \ published`, merged interval list + **exact** byte and interval counts. **This is the number that must go to zero.** |
| `excess` | `published \ declared`, likewise. Not automatically wrong — RM rounds a mapping up to the memdesc's own page size (`w377` §2) — but never silent. |

`crates/kayfabe-util/src/coverage.rs` is the algebra (`IntervalSet`, `Coverage`,
`CoverageAggregate`); `kayfabe_rt::device::SharedDevice::vas_coverage` is the join;
`shim.rs`'s `vas_census` prints ` | COVERAGE <aggregate> | COVERAGE-VAS <per-vas>`.

## §3 ★★★ THE RULE: A CAP MAY TRUNCATE WHAT IS PRINTED, NEVER WHAT IS COMPUTED

`vas_coverage` **takes no cap at all**. `cap` exists only in `Coverage::render` /
`VasCoverage::render`, bounds only the printed interval lists, and every list it prints ends
`showing N of M` with `⚠ PRINT-TRUNCATED (the counts beside this list are EXACT and were
computed over ALL of it)`.

⚠ **Proved, not asserted.** `the_print_cap_truncates_the_list_and_never_the_verdict` renders
200 residual intervals at `cap=3` and asserts `COVERED`, `residual_bytes` and
`residual_intervals` are unchanged, and that the same `Coverage` rendered at `cap=1024`
produces a *different string* with *identical counts*.
`the_render_cap_never_reaches_the_verdict_through_the_device` asserts the property survives the
wiring.

## §4 ⊘ THREE THINGS THE OBVIOUS IMPLEMENTATION GETS WRONG

1. **`published` is not `Binding::host`.** `commit_pin_guest_ram` maps guest RAM into the host
   VAS at the guest's own VA, records it in `Vas::guest_ram_pins`, and sets `Binding::host`
   **never** (`vas_published_ranges`' 2026-08-13 correction). A predicate reading one field
   reports a confident residual over bytes that **are** mapped — the w290 wrong zero,
   re-derived as a boolean and therefore harder to doubt. ⇒ `published` is the **union** of
   both records, and it is a union and **not a sum**: an exact-extent pin appears in *both*,
   and adding them would report double the bytes actually mapped.
   `a_guest_ram_pin_publishes_even_though_it_sets_no_binding_host` drives both arms.
2. **A refinement covers.** `declared=[0,0x1000)` against `published=[0,0xea000)` is
   `COVERED`. This is precisely the `InsideLarger/SameMemory` shape the live straddle check
   **refuses** — 233 of `w376llmd`'s 255 refusals — so an instrument built to grade that
   bug's fix must not share its criterion. Extent equality is never asked here; only which
   **bytes** are mapped. `a_refinement_is_covered`.
3. **Adjacent intervals merge.** `[0,4k) ∪ [4k,8k)` is one interval, not two, or a single
   contiguous hole reports as two problems and two fixes. ⊘ With a one-byte-gap negative
   control, or the rule degenerates to *"everything is one interval"*.

## §5 ⊘ WHAT THIS INSTRUMENT STILL CANNOT SEE — read before trusting a green

- **`declared` has two records and they are different sets.** `TABLE-DESCRIBES` is what we
  **accepted**; a row `w377` blocker (1) refused is absent from it and therefore **can never
  appear in its residual**. ⇒ `TABLE⊆PUBLISHED COVERED=true` is compatible with 255 refused
  rows. The `GUEST⊆PUBLISHED` clause beside it uses `Vas::reach` and does see them — but its
  residual is expected to be large on a healthy boot (the guest describes kernel mappings we
  never publish), so it is a **trend, not a gate**. Neither clause alone is the answer; the
  refusal census remains load-bearing.
- **Both `COVERED`s are vacuously true over an empty declared set**, and both say so —
  `(TRIVIAL: declared is EMPTY …)`, `(TRIVIAL: GUEST-DESCRIBES is EMPTY)`, `NO LIVE ADDRESS
  SPACE`. `[measured, w378]` the guest-reach clause printed a **bare** `COVERED=true` in its
  first draft, and the sample output is what caught it: a reader scanning a column for green
  would have counted a fact about the decoder as a fact about publication.
- **`zero_len_rows` are invisible to the predicate by construction.** A zero-length row
  contributes nothing to `declared` and so can never be residual. It is counted on the line
  rather than left to be inferred from a mismatch.
- **Coverage is not reachability.** `declared ⊆ published` says the bytes are installed in the
  host VAS. It says nothing about whether the engine walks that VAS, whether the mapping is
  live at the moment of the walk, or whether the aperture is right. Same scope caveat as
  `bound=N` in `sweep_cpu_pt_tables`.
- **`covered_pct` is `Option`, and `None` prints `n/a(declared empty)`.** *"There was nothing
  to cover"* and *"everything was covered"* are different findings and a float cannot carry
  the difference.

## §6 SAMPLE OUTPUT — `[measured, w378, tests/tests/coverage_predicate.rs]`

The unpublished-table shape, which is `w376llmd`'s `host_rows=0 of 6254` in miniature:

```
[proc=1 gpu=0 pdb=0x200000 rows=2 host_rows=0 pins=0 zero_len_rows=0 row_bytes=0 pin_bytes=0
 TABLE⊆PUBLISHED COVERED=false declared=131072B/2i published=0B/0i covered_pct=0.0000%
 residual_bytes=131072 residual_intervals=2
 residual=[0x80000000+0x10000,0x80100000+0x10000] showing 2 of 2
 excess_bytes=0 excess_intervals=0 excess=[] showing 0 of 0
 | GUEST⊆PUBLISHED COVERED=true(TRIVIAL: GUEST-DESCRIBES is EMPTY) declared=0B
 residual_bytes=0 residual_intervals=0]
```

The truncation rule, visible in the output itself — **16 residual intervals, 8 printed, the
counts exact**:

```
… residual_intervals=16 residual=[0x80000000+0x10000,…,0x800e0000+0x10000] showing 8 of 16
  ⚠ PRINT-TRUNCATED (the counts beside this list are EXACT and were computed over ALL of it) …
```

The pin case — `host_rows=0` and `COVERED=true`, which is the whole of §4.1:

```
[… rows=2 host_rows=0 pins=1 row_bytes=0 pin_bytes=2162688
 TABLE⊆PUBLISHED COVERED=true declared=131072B/2i published=2162688B/1i covered_pct=100.0000%
 residual_bytes=0 residual_intervals=0 residual=[] showing 0 of 0
 excess_bytes=2031616 excess_intervals=1 excess=[0x90010000+0x1f0000] showing 1 of 1 …]
```

## §7 COST

One extra `Vas::reach::reachable_ranges()` traversal and two extra table scans per address
space per doorbell census, beyond what `vas_census` already does unconditionally. ⚠ Stated
rather than measured. `w315` puts 91.5 % of the doorbell trap in page-table + publication and
4.1 % in the real host forward, so the census is not the cost centre — but `w377` blocker (4)
is that publication is already budget-truncated on the vCPU thread, and *"not the cost centre"*
is not *"free"*. If the trap budget regresses, this is a thing that changed.
