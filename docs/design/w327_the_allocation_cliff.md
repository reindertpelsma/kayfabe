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

**There is no Xid** in the host dmesg and **no `NVRM` line** in the guest dmesg at the failure.
⇒ nothing reaches hardware.

> ### ⚠⚠ AND HERE IS THE INSTRUMENT MISTAKE THAT COST THIS RUNG SEVERAL HOURS, KEPT BECAUSE IT
> ### IS THE GENERAL SHAPE AND NOT A ONE-OFF
> My first move was a **token-set diff** of a failing boot's QEMU log against a passing one's —
> extract every uppercase run, normalise digits away, `comm` the two sorted sets. It returned
> **EMPTY IN BOTH DIRECTIONS**, and I wrote down *"the device never says anything different."*
> ⊘ **It says exactly the right thing, 32 times, and my diff was structurally unable to see
> it.** The refusal that explains the whole failure is `⚠ THE INSTALL REFUSED …
> why=\`that framebuffer range is already joined\`` — and it appears in **every** boot,
> passing ones included, at a **baseline of 16**. The failing boots print **26–32**.
> ⇒ **A VOCABULARY DIFF CANNOT SEE A QUANTITY**, and it cannot see an address. It is
> `a_count_cannot_see_a_substitution` turned inside out: there the count was blind to the
> substitution, here the *set of names* was blind to the count. ★ The fix that actually worked
> was to stop diffing and **ask the log about one address** — the failing `in_ptr` — which took
> one `grep` and gave the answer immediately.

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
| `w327u4b` | `4,64` (repeat, new binary) | **64** | — | yes | 4 |
| `w327u2` | `28,31` | 28 | **31** | **yes** | 28 |
| `w327x1b` | `28,64` | 28 | **64** | **yes** | 28 |
| `w327x2` | `16,31` | 16 | **31** | **yes** | 16 |
| `w327z1` | `28,31`, fill chunk **1 MiB** | 28 | **31** | **yes** | 28 |
| `w327z2` | `28,31`, fill chunk **2 MiB** | 28 | **31** | **yes** | 28 |
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
| `w327u4b` | `4,64` | **PASSES** — repeat of `u4` on the newer binary |

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

### 2.2 ★ n = 3 ON THE FAILURE ITSELF — it is deterministic, not a rate

`w327f1`, `w327f2`, `w327f3`: three identical boots of `28,31`. **3/3** report
`W327_LAST_OK_MIB=28  W327_FIRST_FAIL_MIB=31`, with
`BW_FILL_FAIL mib=31 at_element=2097152 … rc=0/719` and **zero Xid on both sides**, every time.
⊘ Stated because it is the failure mode this campaign has been burned by twice (w319's
intermittent `FAULT_PDE`): a state-dependent death with no Xid is exactly the shape that turns
out to be a rate. **Here it is not.** With `w327u2` that is four boots of
the same two-row list, all failing identically.

### 2.3 ★★★ THE FAILURE OFFSET IS AN ADDRESS, NOT AN OPERATION COUNT — three resolutions agree

Every failing row in this rung and in w322 reports `at_element=2097152`, byte offset
**`0x800000` = 8 MiB**. ⚠ **That number is the harness's own `FILL_CHUNK`**
(`scripts/bench/cup8bench.c`), i.e. *"the second chunk"*, and I matched it against
`MAX_PUSH_TOTAL_BYTES = 8 << 20` before reading that constant's definition site — where it
turns out to bound pushbuffer **method** bytes and to have nothing to do with operands.
**A candidate whose magnitude matches your measurement belongs to the instrument until proven
otherwise**, and here the magnitude *was* the instrument's own constant.

⇒ the chunk is now a knob (`BENCH_BW_FILL_CHUNK_MIB`, defaulted to 8 so an un-armed run is
byte-identical), and the same `28,31` list was run at three resolutions:

| boot | chunk | reported failure offset | ⇒ the buffer is good up to |
|---|---|---|---|
| `w327f1..3`, `w327u2` | 8 MiB | `0x800000` (chunk 2) | somewhere in [8, 16) MiB |
| `w327z2` | 2 MiB | **`0xc00000`** (chunk 7) | [12, 14) MiB |
| `w327z1` | 1 MiB | **`0xc00000`** (chunk 13) | **[12, 13) MiB** |

★ **All three are consistent and the two fine ones agree exactly: the allocation works for its
first 12 MiB and not past it.** ⊘ And this rules out an operation-count story on its own: at
1 MiB the fill issues **twelve** successful memsets before dying, at 8 MiB it issues **one** —
same address, different counts. The boundary is a place in the buffer.

## 3. ★★★★★ THE MECHANISM, SETTLED — A FREED ALLOCATION'S FRAMEBUFFER FRAMES ARE STILL
## JOINED, SO THE NEXT ALLOCATION THAT RECYCLES THEM CANNOT BE PUBLISHED

### 3.1 The refusal names itself, at the exact failing address

`w327z1` (`28,31`, fill chunked to **1 MiB** so the offset is resolved eight times finer):

```
BW_FILL_FAIL mib=31 at_element=3145728 of 8126464 (byte offset 0xc00000) rc=0/719
```

and in the device log, for the same buffer (`in_ptr = 0x79d4d2000000`):

```
leaf va=0x79d4d2000000 len=0x200000 fb_phys=0x6200000 → JOINED
leaf va=0x79d4d2200000 len=0x200000 fb_phys=0x6400000 → JOINED
leaf va=0x79d4d2400000 len=0x200000 fb_phys=0x6600000 → JOINED
leaf va=0x79d4d2600000 len=0x200000 fb_phys=0x6800000 → JOINED
leaf va=0x79d4d2800000 len=0x200000 fb_phys=0x6a00000 → JOINED
leaf va=0x79d4d2a00000 len=0x200000 fb_phys=0x6c00000 → JOINED
leaf va=0x79d4d2c00000            fb_phys=0x6e00000 → ⚠ THE INSTALL REFUSED
    why=`that framebuffer range is already joined; installing a second backing over it
         would give one leaf two memories again, which is the defect the join exists to end`
    — this device still serves that range from its own pages. ⊘ RELEASED and NOT bound
```

★★★★★ **`0x79d4d2c00000` is `in_ptr + 12 MiB`, and `0xc00000` is 12 MiB.** The fill dies at
**the first byte past the last leaf that could be joined**, exactly. Thirty-two consecutive
leaves — the whole rest of the buffer — carry the identical refusal.

⇒ **THE CAUSE.** The guest freed the previous (28 MiB) allocation; its framebuffer frames went
back to the guest's own allocator; the guest handed some of them to the new buffer. **Our join
of those frames was never released**, so `SparseFb::install_join`
(`crates/kayfabe-device/src/fbwin.rs:1069-1091`) refuses the new backing with `ALREADY_JOINED`
— *correctly*, because two backings over one frame is the two-memories defect the join exists
to end. The leaf therefore stays **fabricated** (served from the emulator's own pages), the
host engine's operand has no host backing there, and the `cuMemsetD32` that first touches it
kills the channel.

### 3.2 Why the release never happens — and the source says so in advance

`join_operand_fb_leaves` carries a table headed *"★★ CLEANUP — named now, because a join
without a release is a leak"* (`crates/kayfabe-qemu-raw/src/shim.rs:7422-7437`). It names the
owner, the unit, the lifetime, **the event that ends it** (*"the guest's own free/unmap of the
range, seen as the page-table leaf ceasing to bind"*) and the primitive
(`SharedDevice::release_unadopted_fb_leaf`, *"already stages the unmap; the missing half is the
**trigger**, not the mechanism"*) — and then states:

> ⊘ **Not wired this rung**, and the shape admits it rather than assuming it away.

★★★ **w327 is the measurement of what that costs.** The join count is append-only and visibly
so: across `w327a` the device's `joined=` reading climbs `0 → 4 → 29 → 31 → 34 → 35 → 43 → 67 →
… → 83` over nine allocate/free cycles and **never once falls**.

⊘ And the trigger is blocked by a second refusal on the same plane: the page-table leaf *does*
cease to bind, but `apply_settlement` **refuses to unbind a host-published range** —
`PopulateRefusal::UnbindsPublished` (`crates/kayfabe-mmu/src/reach.rs:807-820`,
`crates/kayfabe-mmu/src/walker.rs:958-975`), whose own doc says *"Unpublishing needs a worker
and an unmap verb, i.e. the forwarding plane. So the refusal is the answer, and the binding
stays."* ⇒ **the exact event the cleanup table nominates as the trigger is the event the
address table is built to swallow.** That is the map/revoke asymmetry `w323` names as a type,
observed end to end.

### 3.3 The mechanism predicts every row of §2, including the ones that refuted my earlier guesses

| observation | why |
|---|---|
| a lone 31 or 64 MiB passes | nothing has been freed, so no frame is stale-joined |
| the VA must MOVE | if it does not, the leaf re-binds to the **same** `fb_phys` and the existing join is a legitimate replay (`already`), not a collision |
| the predecessor must be LARGE | a 4 MiB predecessor occupies ~2 frames; a 28 MiB one occupies 14, so a recycling allocator is far likelier to hit one |
| the NEW allocation's size is irrelevant | `4,64` passes and `28,64` fails — the collision is with the *predecessor's* frames |
| 16 × 4 MiB and 8 × 16 MiB pass | same VA every row ⇒ same frames ⇒ replay, never collision |
| the failing size is 29/31/32/64 by boot | it is whichever row is the first to land on a recycled frame |
| **no Xid, no NVRM line, no fault** | nothing reaches hardware: the operand is simply still fabricated |
| the first *N* MiB works | the frames below the first collision were free to join |

### 3.4 What was ruled OUT, with the number
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

**The dependency, stated for `w326` (the publish plane), and it is a TRIGGER, not a mechanism.**
`join_operand_fb_leaves`' own cleanup table says the primitive exists
(`SharedDevice::release_unadopted_fb_leaf`, *"already stages the unmap; the missing half is the
trigger, not the mechanism"*) and names the event that should fire it: *"the guest's own
free/unmap of the range, seen as the page-table leaf ceasing to bind"*. **Two things must land
together, and landing either alone does nothing:**

1. **The trigger.** When a leaf stops binding, release the join keyed on its `fb_phys`.
2. **The unbind must be allowed to happen at all.** Today `apply_settlement` refuses it
   (`UnbindsPublished`), so the event the trigger listens for is swallowed one layer down.
   ⇒ these are the same fix, and doing (1) without (2) leaves it dead code.

★ **The cheapest falsifier, already built and already exercised:** `KAYFABE_BENCH_BW=28,31`
must stop failing; the device's `joined=` reading must **fall** across an allocate/free cycle
instead of climbing monotonically 0 → 83; and `w327u4b`'s `4,64` must still pass. Three
numbers, one boot each, and the two-row repro is 3/3 deterministic so a green is meaningful.

★ **The cheapest falsifier for that fix, already built:** `KAYFABE_BENCH_BW=28,31` must stop
failing, `PUBCONFLICT_VAS[n=…]` must fall from 1339 toward 0, and `w327u4`'s `4,64` must
still pass. Three numbers, one boot each.

## 6.1 ★★★★★ CORRECTNESS ABOVE THE OLD "CEILING" — it does not merely allocate, it COMPUTES

The brief's sharpest requirement: *"a raised ceiling must be shown to still COMPUTE CORRECTLY
at the new size, not merely to allocate. A big allocation that returns success and produces
wrong values is the worse failure."* Since the ceiling turned out not to exist, the equivalent
statement is *does a size above w322's claimed 32 MiB compute bit-exactly*.

`w327big` — cup8 at **N = 3072**, i.e. three **36 MiB** operands (3072² × 4 B = 37 748 736 B),
one CUDA context, three timed iterations, every one verified:

```
B3072_MAXERR=0
BSUM N=3072 iters=3 med_ms=1713.958 gflops=33.829 … bad=0 maxerr=0
BENCH_VERDICT: PASS (every timed iteration verified)
GUEST_BENCH_TOTAL_BAD=0  GUEST_SIZES_DONE=1  GUEST_XID_COUNT=0   host Xid = 0
```

⇒ **`bad=0 maxerr=0` on 36 MiB operands — bit-exact, on a size w322 reported as fatal.** ⊘ And
note what this does *not* say: cup8 allocates its three buffers once and keeps them, so it
never performs the allocate → free → allocate-elsewhere sequence §2.1 identifies. It is
evidence that the size is fine, and it is **not** evidence that the defect is gone.

## 6.2 GRADING — three workloads at n = 3, and a known-positive that FIRED

| workload | n | result |
|---|---|---|
| `^CUP3_VAL=43` (GR/compute, libcuda) | **3** boots | `CUP3_VAL=43  CUP3_RC=0` ×3 |
| `^CUP8_BAD=0 ^CUP8_MAXERR=0` | **3** boots | `CUP8_BAD=0 CUP8_MAXERR=0` ×3 |
| `R33 arm 1` (raw CE, no libcuda, own VAS) | **3** boots | fired ×3, byte-identical: `4096 bytes moved, dst[last] 0xc0fff232 (want 0xc0fff232), engine semaphore 0x00000001 (declared 0x00000001), GP_GET 1 caught GP_PUT 1` |
| cup8 at N=3072 (36 MiB) | 1 boot, 3 iters | `bad=0 maxerr=0`, `Xid=0` |
| every `bw` row that measured | 21 boots | `bad=0` in **every** row |

★★ **`R33` is LIVE for this rung, not vacuous.** The brief warned it was vacuous for w321's fix
(`asked=0`); here it does its own CE round trip through its own VA space and prints the full
COPY line on all three boots. ⊘ Its `R33_RC=1` is the *fresh* arm provoking a fault on purpose
and is expected — it is graded on the client's own words, never on the rc.

**Offline suite, `cargo test --workspace --features host-isolates --no-fail-fast` on `vh2` at
`15d52b10`: 2926 passed, 6 failed across 3 targets** —
`a_device_with_no_fb_source_refuses_the_vidmem_ring`,
`a_guest_doorbell_reaches_the_host_completion_observer`,
`a_second_doorbell_over_an_unchanged_ring_forwards_nothing`,
`a_wired_device_refuses_a_framebuffer_page_nothing_ever_wrote`,
`the_logic_crates_carry_no_unnamed_guest_os_assumption`,
`the_observers_negative_verdict_refuses_the_guest_doorbell`.
⇒ **exactly master's stated baseline of 3 targets / 6 tests. This rung adds none.**

★ **And it briefly added a SEVENTH, which turned out to be the most useful thing the suite
did all day.** `every_unserviced_id_a_boot_recorded_is_classified` went red the moment this
rung's boot logs entered the tree, naming `0x83de030c`
(`NV83DE_CTRL_CMD_DEBUG_READ_ALL_SM_ERROR_STATES`) and every boot that recorded it — and that
list is **exactly the 11 failing boots and none of the 10 passing ones**. It is libcuda's error
path: the guest asking *which* fault killed its kernel, reached only after the launch is
already lost. Now carried in `LEDGER` with what this rung believes about it
(`tests/tests/admitted_is_served.rs`). ⊘ Not served, and explicitly **not** read as inert.

★★★★★ **AND THE KNOWN-POSITIVE FIRED**, which w322 could not get (`VOID` on its one attempt):
`w327n` ran `BENCH_NOLAUNCH=1`, the arming assertion found `BENCH_MODE=NOLAUNCH` **present**,
and the verifier reported `BENCH_NOLAUNCH_TOTAL_BAD=3670016 > 0`.
⇒ **every `bad=0` above is asserted, not inherited.** ⊘ This is the guest-side control w322
listed as *"the first thing to run next"*; it is now shown alive inside the guest.

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
