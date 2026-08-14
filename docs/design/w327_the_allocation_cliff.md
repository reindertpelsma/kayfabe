# w327 — THE "32 MiB ALLOCATION CLIFF" IS NOT A CLIFF, NOT AT 32 MiB, AND NOT ABOUT SIZE

**STATUS: LIVE, 2026-08-14.** Measured on `vh2` (real GA106, RTX 3060, driver `580.159.04`,
stock guest driver), base pinned at **`df3043be`** (master = merge of w322). Every number below
is from a boot on that box on that day; the tag of each boot is given so a log can be found.

> ## ⊘⊘⊘ THIS DOCUMENT SUPERSEDES `w322_locate_the_operands.md` §6.5 AND THE PART OF w322's
> ## MERGE MESSAGE THAT REPEATS IT. Both say:
> > *"Every single allocation ≥ 32 MiB in the FB-leaf chain dies `rc=719` with
> > `budget_hit=true` — w319's DRAIN-BUDGET TRUNCATION REACHED BY SIZE — 3/3 boots."*
>
> **Three separate claims, and all three are wrong:**
> 1. ⊘ **NOT ≥ 32 MiB.** A lone `cuMemAlloc(64 MiB)` passes and streams at 3.04 GB/s (`w327b3`).
>    A lone 31 MiB passes at 2.961 GB/s (`w327b1`). `29,30,31` passes at all three (`w327b2`).
> 2. ⊘ **NOT A SIZE THRESHOLD AT ALL.** The same 31 MiB allocation passes alone and fails after
>    a 28 MiB one (`w327b1` vs `w327u2`); the failure has been observed at **29, 31 and 32 MiB**
>    in different boots and never at a fixed number.
> 3. ⊘ **NOT `budget_hit`, and not w319.** The `DRAIN-TIMING … budget_hit=true` line w322 quotes
>    is emitted by `SharedDevice::drain_retired_budgeted` (`kayfabe-rt/src/device.rs:1296`),
>    the **retired-proc disposal** drain, bounded by `RETIRED_DRAIN_BUDGET_US = 40_000`. It
>    publishes nothing and joins nothing. The publication pass's budget is
>    `VAS_PUBLISH_WALL_BUDGET = 2000 ms` and prints `⚠⚠ WALL BUDGET … EXHAUSTED`; w319's
>    guest-RAM drain is `VAS_DRAIN_WALL_BUDGET` and prints `⚠⚠ DRAIN WALL BUDGET … EXHAUSTED`.
>    **Three different budgets sharing the word "drain".** `budget_hit=true` appears on 5–7
>    lines of *every* boot in this rung, **including every passing one**, so it is not a
>    discriminator of anything.
>
> ★ The correction is not a criticism of the sweep: w322 measured 4, 16 and 32 MiB, and on a
> **power-of-two grid 16 and 32 are adjacent**. Its own §6.3 called the row `⊘ UNMEASURED`
> honestly. What did not survive is the *inference* drawn from three points, and the
> mechanism attached to it from a log line whose definition site was not read.

---

## 1. What actually happens

`cuMemAlloc` succeeds. The following `cuMemsetD32` over the buffer, issued by the bench in
8 MiB chunks, returns **0 from the memset and 719 (`CUDA_ERROR_LAUNCH_FAILED`) from the
sync**, on the **second** chunk. The CUDA context becomes sticky-dead, so every later row
reports `alloc_failed rc=719` and says nothing about its own size.

**There is no Xid** in the host dmesg, **no `NVRM` line** in the guest dmesg at the failure,
and **no named refusal** in our own device log: a token-set diff of a failing boot's QEMU log
against a passing one's returns **empty in both directions**. ⇒ nothing reaches hardware and
nothing in the shipped vocabulary says so.

## 2. The measurement — every boot, with its tag

| boot | list (MiB, in order) | last OK | first FAIL | `in_ptr` moved? | previous largest |
|---|---|---|---|---|---|
| `w327b1` | `31` | **31** | — | n/a (first alloc) | — |
| `w327b3` | `64` | **64** | — | n/a (first alloc) | — |
| `w327b2` | `29,30,31` | **31** | — | **no** (one VA) | 30 |
| `w327u1` | `31,31` | **31** | — | **no** | 31 |
| `w327r` | `4` ×16 | **4** | — | **no** | 4 |
| `w327s` | `16` ×8 | **16** | — | **no** | 16 |
| `w327u3` | `4,31` | **31** | — | yes | 4 |
| `w327u4` | `4,64` | **64** | — | yes | 4 |
| `w327u2` | `28,31` | 28 | **31** | **yes** | 28 |
| `w327x1b` | `28,64` | 28 | **64** | **yes** | 28 |
| `w327x2` | `16,31` | 16 | **31** | **yes** | 16 |
| `w327c1` | `16,24,28,29,30,31,32,40,64` (+`coalesce`) | 28 | **29** | **yes** | 28 |
| `w327a` | `16,17,18,20,22,24,28,31,32` | 28 | **31** | **yes** | 28 |
| w322 `bw`,`bw2` | `4,16,32` | 16 | **32** | **yes** | 16 |

★★★★★ **THE FAILING SIZE IS 29, 31, 32 OR 64 DEPENDING ON THE BOOT, AND THE SAME ALLOCATION
IS BOTH.** Any statement of the form *"the ceiling is N MiB"* is refuted by this table. The
pre-registered deliverable — *"the ceiling as a number at ≥ 3 sizes"* — is answerable only as:
**there is no size ceiling. A 64 MiB `cuMemAlloc` allocates, fills and reads correctly in one
boot and dies `rc=719` in another; a 29 MiB one does the same.**

★★★★★ **AND THE SINGLE-VARIABLE PAIR SETTLES IT WITHOUT ANY INFERENCE:**

| boot | list | 64 MiB row |
|---|---|---|
| `w327u4` | `4,64` | **PASSES**, 22.130 ms, `bad=0` |
| `w327x1b` | `28,64` | **FAILS**, `rc=0/719` at byte offset `0x800000` |

**Same allocation, same size, same binary, same box, same hour. The only difference is the
size of the row before it.** ⇒ the axis is not the allocation being made; it is the state left
behind by the one before.

### 2.1 What separates the two halves of that table

Two conditions hold on **every** failing row and on **no** passing row:

1. **The buffer's virtual address MOVED** — CUDA could not fit the request beside the previous
   one and carved a new VA region. This is visible in the workload's own `BW_BEGIN` line and
   needs none of our instrumentation:
   `w327a` rows 1–7 all at `in_ptr=0x751a6a400000`, row 8 at `0x751a64000000`;
   `w327c1` rows 1–3 at `0x7b00fe400000`, row 4 at `0x7b00f8000000`;
   w322 rows 1–2 at `0x79ef94400000`, row 3 at `0x79ef8e000000`;
   `w327b2` — never moves, all three pass.
2. **The previously largest allocation was ≥ 16 MiB.** `w327u3` (`4,31`) and `w327u4`
   (`4,64`) both MOVE and both PASS, with a 4 MiB predecessor.

⚠ **Condition 1 alone is NOT sufficient, and that was a prediction of mine that its own test
refuted.** I pre-registered *"`4,64` moves the region, therefore it fails"*; it passed at
64 MiB, 22.13 ms, identical to the lone-64 row. The refutation is what produced condition 2.

**Two further axes were pre-registered and are REFUTED, each by its own arm:**
- ⊘ **allocate/free CYCLES.** `w327r` = sixteen consecutive 4 MiB rows: all pass. The VA never
  moves, so sixteen cycles cost nothing.
- ⊘ **CUMULATIVE BYTES.** `w327s` = eight consecutive 16 MiB rows = **128 MiB** allocated and
  freed: all pass, `bad=0` on every row. That is four times the cumulative traffic of the
  three-row `4,16,32` list that fails.

⇒ **Minimal reproducer: `KAYFABE_BENCH_BW=28,31` — two rows** (`w327u2`). A two-row device log
is readable; the nine-row one this started from is not.

## 3. The mechanism — ⊘ STILL UNATTRIBUTED. What follows is a candidate and its evidence, and
## the section ends by saying why the evidence does not yet close.

### 3.1 A published range can never be released or repointed in this build

`apply_settlement` (`crates/kayfabe-mmu/src/reach.rs:807-820`) **refuses** to unbind a range
whose binding is host-published, and `populate` refuses to repoint one:

- `PopulateRefusal::UnbindsPublished` (`crates/kayfabe-mmu/src/walker.rs:958-975`) — its own
  doc: *"Unpublishing needs a worker and an unmap verb, i.e. the forwarding plane. So the
  refusal is the answer, and the binding stays."*
- `PopulateRefusal::RepointsPublished` — *"the mirror image … a different act with the same
  consequence."*

⇒ When the guest frees a buffer and CUDA later reuses that virtual address for a different
allocation, **our table keeps the stale published binding**, and the new allocation's leaves
are refused rather than repointed. This is the map/revoke asymmetry `w323` names as a type and
`w326` is rebuilding the publish plane around.

### 3.2 What 1339 refused unbinds look like, at the address level

With the new `PUBCONFLICT_VAS` list (§5) armed, `w327x2` (`16,31`, FAILS at 31) prints, on the
pass that matters:

```
PUBCONFLICT_VAS[n=1339
  lowest =[0x753544000000,0x753544200000,0x753544400000,0x753544600000,0x753544800000,
           0x753544a00000,0x753544c00000,0x753544e00000,0x753547600000,0x753547601000,…]
  highest=[0x753547b32000,0x753547b31000,0x753547b30000,…]]
```

★ Read against the workload's own `BW_BEGIN` lines — row 1 `in_ptr=0x753544400000`,
`out_ptr=0x753544200000`; row 2 `in_ptr=0x75353e000000` — **the 1339 addresses we refused to
release are the FIRST row's region**, `0x753544000000 … 0x753547b32000`. The guest freed that
buffer; it asked us to unbind 1339 leaves; we refused every one **because they were published**,
and they are still in the table when row 2 runs.

⚠ **And here is the part that does NOT fit the obvious story, stated because it is the part
that matters:** the *failing* row's own base, `0x75353e000000`, is **NOT** in that list — it
sorts below `0x753544000000` and would have appeared in `lowest=` if it were. So *"the new
allocation lands on a stale published binding"* is **not** what `w327x2` shows. What it does
show is that **`0x753544200000` — the address row 2's `out` buffer is allocated at — IS in the
list.** ⊘ In `w327c1` the very first refusal of the pass *was* the failing `in_ptr`
(`UnbindsPublished { va: 0x7b00f8000000 }`, five times, log lines 3888–5774), so the two
failing boots do not agree about which buffer collides. **Both readings are recorded; neither
is promoted.**

### 3.3 What the counts say, and how far they go

`RepointsPublished` separates the two halves cleanly over six boots:

| boot | outcome | max `"RepointsPublished": N` | max `"UnbindsPublished": N` |
|---|---|---|---|
| `w327a` | FAIL | **8** | 1339 |
| `w327c1` | FAIL | **8** | 1339 |
| `w327u4` | pass | 4 | 1335 |
| `w327b1` | pass | 2 | 1333 |
| `w327b2` | pass | 2 | 1333 |
| `w327b3` | pass | 2 | 1333 |

★ And in `w327c1` the **first** refusal of the pass is
`UnbindsPublished { va: GpuVa(135244191629312) }` = **`0x7b00f8000000`** — *exactly* the
`in_ptr` of the row that failed, refused five times (log lines 3888, 4583, 4975, 5358, 5774)
before the publication pass reaches that VA at line 5780.

⚠⚠⚠ **AND HERE IS WHY §3 DOES NOT CLOSE, STATED AS PLAINLY AS I CAN.**
`UnbindsPublished` is **1333 in every passing boot and 1339 in the failing ones — a delta of
six** on a base of thirteen hundred. The refusal to release a published range is therefore a
**universal background condition of every boot this campaign has ever taken**, green ones
included; `w327b1`'s own first refusal is at a guest CUDA VA (`0x7d30ac000000`). It cannot, on
its own, be why one boot dies and another does not. `RepointsPublished` separates 8 from ≤4,
which is six data points and a difference of four — precisely the shape this tree banks as
*"a candidate whose magnitude matches your measurement belongs to the instrument until proven
otherwise"*, and I am not promoting it.

⇒ **The trigger is measured to 14 boots and a single-variable pair (§2). The mechanism is
not.** What §3 establishes is that the plane the trigger points at — publish with no revoke —
is real, is documented in its own source as deliberate-for-now, and is the plane `w326` is
rebuilding. What it does not establish is the step from *"1339 stale published leaves"* to
*"this particular `cuMemsetD32` returns 719."*

### 3.4 What was ruled OUT, with the number

| candidate | number | why it cannot bind here |
|---|---|---|
| `VAS_PUBLISH_LEAF_BUDGET` = 4096 candidates | 4096 × 64 KiB = **256 MiB** | the `capped=2048` seen on every boot is the **system proc's** 12 GiB identity VAS (6144 candidates × 2 MiB), which `§12.26` never publishes anyway; it fires identically on passing boots |
| `MAX_PUSH_TOTAL_BYTES` = `8 << 20` | 8 MiB | pushbuffer **method** bytes. It matches the failure offset only because 8 MiB is `cup8bench.c:589`'s own `FILL_CHUNK`. **Instrument, confirmed.** |
| `SWEEP_FRAMES_MAX` = 8192 | 8192 × 4 KiB = **32 MiB** | pure magnitude coincidence: bounds a diagnostic ring sweep, touches no join |
| `OUR_SLOT_BUDGET` = 64 KVM slots | 64 × 64 KiB = 4 MiB | not on this path — a joined leaf goes to `SparseFb.joined`, never `install_ram_window` |
| `MAX_CE_SPANS` = 4096 | 256 MiB at 64 KiB rows | the 8 MiB-chunked memset makes 128 spans |
| a `u32`/shift truncation | — | `len` is `u64` end to end through `Reply::JoinedBacking`, `Nvos46Parameters.length`, `alloc_os_descriptor`, `SharedRam::create`; the one narrowing (`u32::try_from(sub.len)`) **refuses** and only above 4 GiB |
| the CE **fill** refusal (`ce_copy` rejects `CeSource::Constant`, `kayfabe-isolate-host/src/rm.rs:4858`, `:6716`) | — | a real refusal, but `NOT_ON_THIS_RUNG`, `CeSource::Constant`, `CeWork::Fill` and `Constant` appear **zero times** in all four inspected QEMU logs. ⊘ Recorded as *not fired here*, **not** as ruled out: an absent string is not proof the path was untaken |

### 3.5 The table is right; only the publication is not

`TABLE-DESCRIBES` for the failing boot carries the whole failing buffer —
`0x7b00f8000000+0x1e00000` (30 MiB) — and `GUEST-DESCRIBES` carries the identical run. ⇒
**population is not the defect**, exactly as `the_table_is_right_and_the_host_vas_is_empty`
already established for the GR fault. The defect is on the publication/revocation plane.

## 4. `KAYFABE_DRAIN_BATCH=coalesce` — MEASURED, and it is NOT sufficient

`w327c1` ran `16,24,28,29,30,31,32,40,64` with `KAYFABE_DRAIN_BATCH=coalesce` and failed at
**29 MiB** — a *lower* size than the default arm's 31, because the arm's own earlier rows fill
CUDA's VA arena differently, not because coalescing made anything worse. ⇒ the brief's *"test
whether it already raises the cliff — that is one boot and may make most of this rung
unnecessary"* was the right instruction and the answer is **no**.

## 5. ⚠ The instrument defect this rung found and fixed

`refusal_vas` is a `BTreeSet` and the printer does `.take(PT_SWEEP_REFUSAL_CAP)` — i.e. it
walks **ascending**. Every boot of this campaign therefore printed the same two dozen
`0x203e…`/`0x203f…` kernel addresses, while the guest's operands live at `0x7xxx_xxxx_xxxx`
and **can never appear in the list**. The one question the list exists to answer — *is the
faulting buffer among the refused VAs* — is structurally unanswerable from it.

★ Fixed print-only: `RepointsPublished` + `UnbindsPublished` get their own `PUBCONFLICT_VAS`
list, printed **from both ends** with its full count. ⊘ Kind-filtered rather than cap-raised;
raising the cap would emit 1339 addresses per pass and bury the answer instead.

## 6. What this blocks, and the specific dependency for `w326`

⊘ **This does not block the north-star LLM workload for the reason w322 gave.** Large single
allocations are fine: 64 MiB allocates, fills and reads at 3.04 GB/s. What is not fine is
**allocate → free → allocate at a different VA**, which is what any real allocator does
constantly, so the practical impact is at least as bad — it is just not size-shaped and no
size limit will fix it.

**The dependency, stated for `w326` (the publish plane):** the fix is a **revoke verb** —
`UnbindsPublished`/`RepointsPublished` must become *"unpublish, then unbind/repoint"* instead
of *"refuse and keep the binding"*. Nothing in this rung can raise a ceiling without it,
because there is no ceiling to raise: the defect is that a published range has no way back.

★ **The cheapest falsifier for that fix, already built:** `KAYFABE_BENCH_BW=28,31` must stop
failing, `PUBCONFLICT_VAS[n=…]` must fall from 1339 toward 0, and `w327u4`'s `4,64` must
still pass. Three numbers, one boot each.

## 7. ⊘ WHAT THIS RUNG DID NOT DO, AND WHY

**No fix was attempted.** The brief pre-registered *"an emergent budget exhaustion wants
resumability or fewer chains; a hard constant wants finding and fixing the constant"* — and the
measurement says it is **neither**. There is no budget to resume and no constant to fix: the
defect is that the table has a `publish` and no `revoke`, which is a plane, not a number.
Building half a revoke path here would collide head-on with `w326`, which is rebuilding exactly
that plane on `vh`.

⊘ **And the pre-registered `(A)`/`(B)`/`(C)` are all inapplicable rather than merely false**:
(A) needs a ceiling that coalescing raises — coalescing does not, and there is no ceiling;
(B) needs a constant — none survives §3.4; (C) needs a *second* ceiling above a raised first
one. The honest letter is **(D)**, with the correction that the reason is not *"it cannot be
raised without the publish-plane rework"* but *"there is nothing size-shaped here to raise."*
