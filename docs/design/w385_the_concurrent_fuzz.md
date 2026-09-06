# ★★★★★ THE MULTI-THREADED RAW CLIENT — `--concurrent-fuzz`, seeded, with five named invariants

**STATUS — 2026-09-06 — LIVE.** Adds a rung; supersedes nothing. It fills the gap
`w381_the_guest_servable_probe.md` §3 states about its own R5: *"⊘ Single-client only;
cross-client leakage is NOT covered by it"* — and the larger gap neither file named, which is
that **every rung in `kayfabe-rm-ladder` is single-threaded and sequential.** §1–§3 are read
off this tree's own source and are checkable without a GPU. §4 carries the measurements;
every number names the arm it came from.

---

## §0 THE ONE-LINE PROBLEM

`map_stress` (R5) does 186 releases and does them **one at a time**. `cross_client_leak`
(R5b) holds two RM clients and **never lets them run at the same time**. So the whole
battery's evidence is *"these verbs work when nothing else is happening"*, and four things
have never been reached at all:

- **`RmConnection`'s two host-side mutexes.** `objects` (the handle table and the monotonic
  `mint()` counter) and `rings` (the CPU mappings of every channel's pushbuffer, GPFIFO and
  semaphore). Their separation is a **stated discipline** — *"Two locks, each held for one
  kind of thing, is the R3 lock-rank discipline rather than a convenience"* (`rm.rs`, the
  `rings` field). ⊘ A discipline no two threads ever tested is a comment, not an invariant.
- **The address table under concurrent map / unmap / probe into ONE address space.**
- **Handle and VA recycling** with another thread allocating into the hole.
- **Ordering assumptions that hold only because nothing else was running.**

## §1 ⊘ WHAT THIS RUNG REACHES AND WHAT IT ONLY *TOUCHES*

⊘⊘ **CORRECTED DURING BRING-UP, 2026-09-06, and the first version of this section was wrong
in the more embarrassing direction — it under-claimed.** It said *"a raw-client fuzz cannot
fire an R1 ranked-lock assert, because `lockwitness` is not used in `kayfabe-isolate-host`"*.
The premise is true (`grep` of that crate is empty) and the conclusion does not follow, because
the assert is not in that crate — it is one layer down, on the syscall itself:

```rust
// crates/kayfabe-linux-raw/src/chardev_unsafe.rs:439-440
lockwitness::assert_lock_free("issuing an ioctl on a character device");
leafwitness::assert_leaf_free("issuing an ioctl on a character device");
```

Every RM verb this rung issues goes through `CharDevice::ioctl`, so **both** halves of the R1
discipline are on the path of **every single operation** — the ranked one and the adapter-leaf
one, whose panic message names `l1_concurrency.md §3.3` by hand.

★ **But armed is not the same as trippable, and that distinction is the honest scope.** Both
asserts are **thread-local**: they ask *"does THIS thread hold a lock right now"*. The raw
client's own path takes **no ranked lock at all**, so `assert_lock_free` passes trivially from
this binary no matter how the threads interleave — it is a tripwire that is armed and cannot
be triggered from here. `assert_leaf_free` is likewise decided by lexical scope inside
`rm.rs` (`mint` / `remember` take the `objects` guard and drop it *before* the ioctl, by
construction), so scheduling cannot make it fire either. ⇒ **Firing R1 as a live check needs a
caller that actually takes ranked locks** — the isolate and the shim — which is a different
binary and a follow-on, not this one.

What this rung stresses **for real**, as opposed to merely traversing:

| plane | reached? | by what |
|---|---|---|
| `RmConnection::objects` mutex — the handle table and the monotonic `mint()` | **yes, contended** | N threads, one `Arc<RmConnection>`; checked by `HANDLE_COLLISION` |
| `RmConnection::rings` mutex — every channel's CPU-mapped ring | **yes, contended** | every `submit_*` and `ring_store_u32` |
| RM's own per-client handle and VA allocators, inside `nvidia.ko` | **yes** | concurrent alloc / map / unmap / free |
| our address table under concurrency | **yes** | one shared VAS per client, private windows per worker |
| `lockwitness::assert_lock_free` (R1, ranked half) | **on the path, vacuous** | armed at every ioctl; the raw client holds no ranked lock |
| `leafwitness::assert_leaf_free` (R1, adapter half) | **on the path, vacuous** | same, and decided lexically rather than by schedule |

⚠ The two "vacuous" rows are the ones to keep saying out loud: a green here is **not**
evidence about the ranked-lock discipline in `kayfabe-isolate`, and reporting it as such would
be the exact over-claim this file was written to avoid.

## §2 WHAT WAS BUILT

`--concurrent-fuzz`: `T` worker threads × `I` iterations across `C` RM clients.

- **One `HostRmBackend` per thread, one `Arc<RmConnection>` per client.** That shape is
  forced and is also the point: `HostRmBackend`'s mutating verbs take `&mut self` and its
  `slots` (GPFIFO cursor) map is **per-worker**, so a shared backend would be a harness bug;
  the `RmConnection` underneath, with its two mutexes, is **shared**, and is the thing under
  test. `HostRmBackend` is `Send + Sync` and this is statically enforced — `RmBackend: Send +
  Sync` (`kayfabe-isolate/src/lib.rs:786`), no `unsafe impl` anywhere in `rm.rs`.
- **One shared VAS per client, private VA windows per worker.** ★ Shared, deliberately: a
  per-thread address space would leave RM's per-VAS page tables uncontended, which is the
  plane the rung is named after. The private 64 GiB windows are what keep a violation
  *attributable* rather than merely observed.
- **The two clients name the SAME VA numbers.** The window is keyed on the worker's index
  *within its client*, not on its global id — so client 0 lane 2 and client 1 lane 2 ask for
  identical addresses. That is what makes invariant 4 an isolation statement instead of an
  accident of layout, and it is `cross_client_leak`'s `VA_SHARED` idea taken concurrent.
- **The verbs are the ones the existing rungs already do** — allocate, map at a chosen VA,
  map a **second** VA over the same memory (the w380 alias case), write with the engine and
  read back, unmap one alias, probe, free, and free-then-immediately-reallocate to force
  handle/VA reuse. Nothing is invented; the fuzz is the *schedule*, not the vocabulary.
- **Seeded jitter between and inside operations**, half as a `sleep` and half as a spin. ⊘
  A pure sleep parks the thread and stops contending; a race that only appears while two
  threads are genuinely on CPU together would never be sampled by one.

### §2.1 ★★★★★ CPU PINNING — the owner's correction, and why it is not a detail

Owner ruling, 2026-09-06: *"pinning to vcpu cores is very useful to test concurrency in
kayfabe as each vcpu is a host thread."*

In kayfabe **each guest vCPU IS a host thread**. So the concurrency the product must survive
is *those* threads contending: a small, fixed set — the bench guest is `-smp 3`
(`scripts/bench/boot_nvkvm.sh:44`) — preempting each other. ⊘ A fuzz whose threads land
wherever the scheduler puts them samples a **different topology**: on the 19-core bench box
they spread across idle cores, run *past* one another, and essentially never interleave
**inside** a critical section. That is precisely where a lock-order bug hides, so an unpinned
run can be green for the whole of it.

⇒ The rung's default thread count is **3, the guest's `-smp`**, not a round number, and it
runs **four arms as two pairs**:

| arm | threads | placement | role |
|---|---|---|---|
| `unpinned`   | `--fuzz-threads` (3) | wherever | ⊘ the control for `percore` |
| `percore`    | `--fuzz-cores` (3)   | one worker per core | ★ the guest's topology |
| `crowd-free` | 3 × cores (9)        | wherever | ⊘ the control for `crowd` |
| `crowd`      | 3 × cores (9)        | **all nine on cores 0–2** | ★★★ over-subscribed |

★★★ **`crowd` is the arm that matters.** More runnable workers than cores forces the
scheduler to preempt threads *while they hold a lock*, which un-pinned parallelism tends never
to produce. ⚠ Pinning here **removes** parallelism on purpose; it is not a performance knob.

⊘ **The unpinned arms are kept, at the same width, and are never replaced.** *"Pinning changed
the answer"* has to be a measurement, so the rung evaluates the pairing itself and prints
`FUZZ_PINNING_DELTA=<arm>` when a pinned arm reds while its same-width unpinned control is
green — which would say the unpinned arm was never a test of that.

★★ **And the placement is READ BACK, never assumed.** `pin_current_thread` returning `true` is
the kernel *accepting* a mask; `current_cores()` afterwards is the kernel *describing* what it
applied, and that is what the run prints (`observed core sets {"0", "1", "2"}` /
`{"0-2"}` / `{"0-18"}`). A pinned arm whose masks were refused grades **`NOTRUN`, not
`PASS`** — it would otherwise have run as the unpinned arm under a pinned arm's name and been
read as *"pinning changed nothing"*. That is the same class as the dirty-gate default which
had no caller for a month: **a default you never see exercised is a default you do not have.**

The syscalls live in `crates/kayfabe-linux-raw/src/affinity_unsafe.rs` (two wrappers, nothing
else), behind `kayfabe_linux_raw::affinity`. ⊘ Diagnostic-only: nothing in the shipped isolate
or shim pins anything — the VMM owns vCPU placement.

### §2.1b ★★★ SEEDED, OR IT IS NOT AN INSTRUMENT — and the honest half

Every choice comes from one SplitMix64 stream seeded by `--seed`, printed as `FUZZ_SEED=` on
**every** run including the ones that pass, and worker `t`'s stream is a pure function of
`(seed, t)`. So `--seed=N` replays the **decision sequence**.

⊘ **It does not replay the OS schedule, and the rung says so in its own output.** A red names
a reproducible *program*; the interleaving that made it red is not ours to reproduce. Stating
this here rather than discovering it later matters, because *"reproducible fuzz"* over threads
is a claim this tree cannot make and should not imply.

### §2.2 THE ORACLE — five invariants, each refused BY NAME

A stress test with no invariant only finds crashes. Every thread writes a **thread-unique,
client-tagged magic** (`0xF5 | client | tid | seq`), so a word that arrives from elsewhere is
*identifiable* rather than merely wrong.

| # | invariant | refusal name(s) |
|---|---|---|
| 1 | no VA is ever bound to two different memories at once | `VA_DOUBLE_BOUND` |
| 2 | a mapping is all-or-nothing, never observable half-installed | `MAP_NOT_ATOMIC` |
| 3 | every value written through one alias is readable through the others; mapping or unmapping the second alias does not revoke the first | `ALIAS_MISMATCH`, `ALIAS_REVOKED` |
| 4 | no client ever observes another client's magic | `CROSS_CLIENT_LEAK` |
| 5 | every release is observed; a freed handle is not resolvable; a VA comes back; a recycled object reads as its OWN sentinel | `RELEASE_LOST`, `FREED_HANDLE_RESOLVES`, `VA_NOT_RECOVERED`, `STALE_READ` |

Plus two structural ones: `WINDOW_ESCAPE` (a placement that drifted out of the asking
worker's window) and `WORKER_PANIC` — ★ the loudest possible red, and the one a witness
assert would produce, so it is caught at the `join` and named rather than swallowed.

⊘ **`Relocated` is deliberately NOT graded.** RM treating a fixed ask as a hint is a
statement about our address choice, not about the mapping plane, and folding it in would
manufacture reds out of legal allocator behaviour. ⊘ **`pde_info` is not used as a publication
oracle** (it answers at page-*table* granularity) and **`GP_GET` is never read** (it has no
writer anywhere in this workspace).

### §2.3 ★★ A DEADLOCK FAILS BY NAME, NEVER HANGS

A watchdog thread runs over the whole rung, with a per-worker stall check beside the global
deadline. On expiry it prints what **every** worker was last doing — verb, iteration, and how
long since its last heartbeat — then `FUZZ_REASON=DEADLOCK/WATCHDOG`,
`RUNG_concurrent_fuzz=FAIL`, and exits 3.

⊘ It ends the process rather than unwinding. A worker wedged in an RM ioctl cannot be joined,
cancelled or timed out, so the only honest choices are *"print what we know and die"* or
*"hang"* — and this tree has three recorded nontermination shapes that wedged CI instead of
failing, one of which held three binaries for 23 hours. The cost is stated rather than
hidden: **a watchdog kill leaks the run's RM objects.**

★ The watchdog needed one correction before it was sound, and it is worth recording because
it is the same class as everything else here: it was first keyed on each worker's own `done`
flag, and **a worker that returns early or panics never sets it** — so the watchdog would
have kept counting and eventually killed a process whose workers had all been collected. The
fix is a separate `stop` flag set by the **phase**, after the joins. `done` is the worker's
statement; `stop` is the phase's.

### §2.4 THE CONTROLS — the rung is these, not the loop

- **A positive control runs FIRST**: the same verbs at `T=1` with **no jitter**, unpinned, on
  one client. If it reds the rung is broken, not the system, and the verdict is `NOTRUN`.
- **★★★ A COVERAGE VETO, and it was paid for during bring-up.** The first working version drew
  a verb and a slot *independently*, so most draws landed on a slot in the wrong state:
  measured at `I=12`, **`idle=85` of 108 ops, `write=0`, `alias_prop=0`, `map_b=0`** — three
  arms reported `PASS` having **never run the engine at all**. Invariants 1, 3 and 4 are only
  tested when the engine runs, so those greens were about nothing. The fix is two things: the
  slot is now chosen from the ones that **admit** the drawn verb, and an arm with
  `engine_ops == 0` grades **`NOTRUN`**. ⚠ The verb *weights* are part of the oracle too —
  `alias_prop` drew **zero** times in 360 ops at the original table, so the one verb that
  tests the w380 alias property end to end was absent from a passing run; it is now weighted
  3/20 and counted separately as `alias_ops` on every arm line.
- **★★ THE WATCHDOG HAS ITS OWN NEGATIVE CONTROL.** A watchdog that has never fired is a
  comment. `w385_concurrent_fuzz.sh` runs one arm with an impossible deadline and asserts the
  rung prints `RUNG_concurrent_fuzz=FAIL`, `FUZZ_REASON=DEADLOCK/WATCHDOG` and a per-worker
  dump, rather than hanging.
- **★★★ `FUZZ_OVERLAP_PAIRS` is a second control and it VETOES a green.** It counts pairs of
  RM-verb intervals *from different threads* that actually intersected. A fuzz run that
  sampled **zero** overlap sampled no concurrency at all, and grading it `PASS` would report
  a finding never measured — the `dlen=0` failure, in a new costume. Zero overlap prints
  `NOTRUN` with `FUZZ_REASON=NO_CONCURRENCY_OBSERVED`.
- Every outcome is pre-registered, including the three ways to be **unmeasured**: no binary,
  no device, and a binary that ran and printed no verdict line are three distinguishable
  verdicts in `scripts/bench/w385_concurrent_fuzz.sh`.
- The rung reports a **distribution and `n`** (min/p50/p90/p99/max over every op, plus the
  per-verb census), never a summary number.

⚠ **A green proves less than it looks**, and the rung's own `PASS` line says so: it names the
op count, the thread count and the measured overlap as *"a budget, not a proof"*.

## §3 HOW TO RUN IT

```text
kayfabe-rm-ladder --gpu 0 --concurrent-fuzz \
    [--fuzz-threads 8] [--fuzz-iters 64] [--fuzz-clients 2] \
    [--fuzz-jitter-us 200] [--seed 0x...] [--fuzz-deadline 900] [--fuzz-stall 120]
```

`scripts/bench/w385_concurrent_fuzz.sh` walks a ladder of widths, grades each arm out of
printed lines only, and runs the **same seed twice** at one width — because *"the seed
replays"* is a claim the rung makes about itself, and an unchecked claim about an instrument
is what this tree has paid for most often.

## §4 THE MEASUREMENTS

RTX 3060 (GA106), host driver **580.159.04 open** kernel module, kernel `6.8.0-59-generic`,
19-core vast box, source `b16dca06`. Native arm only — this is a host-side rung and the guest
arm is a follow-on, not the deliverable.

### §4.1 ★★★★★ THE FIRST THING IT FOUND WAS A DEFECT IN ITSELF — and that is the result

At **T=32 / 4 cores**, seed `0x18d2cc842fe59937`: **66 `ALIAS_REVOKED` violations** — 22 in
each of the three 32-thread arms, **zero** in the 4-thread `percore` arm. Both halves of the
cause were ours, and both are the classes this tree has paid for before:

1. **THE GEOMETRY.** `alloc_vaspace` asks `vaSize = 0`, i.e. RM's default `FERMI_VASPACE_A`
   limit — **1 TiB** on this part. At the original 64 GiB per-lane stride, **lane 14's window
   begins at exactly `0x100_0000_0000`**, one range past the end of the address space, so
   every mapping in it was refused. The evidence was unambiguous in the log: every violation
   came from **tids 28 and 29 only**, and every VA named was inside that one window
   (`0x101…`–`0x106_0000_0000`). ⇒ Stride halved to 32 GiB (a window needs 28), and
   `W385_MAX_LANE` now refuses a lane above the ceiling **by name**
   (`WINDOW_ABOVE_VAS_LIMIT`), so the rung says *"you asked for more address space than
   exists"* instead of *"the alias property broke"*.
2. **THE GRADING.** Both `ALIAS_REVOKED` sites tested `!out.landed()`, which folds
   `W379Release::Refused` — *"the submission was declined and the engine was never asked"* —
   into *"the write was lost"*. ⊘ That is precisely what `W379Release`'s three-value shape
   exists to prevent, and **the only thing distinguishing the 66 fakes from a real revoke was
   the `(Refused)` printed inside their own message.** Both sites now match on `Lost`.

★ **Confirmed by replaying the seed after the fix**: the same seed produced a **byte-identical
program** — `ops=6400 engine_ops=1784 alias_ops=774` on every arm, unchanged — with
`refused` **1200 → 0** and `violations` **66 → 0**. A red, root-caused, and closed by its own
replay handle. ⚠ It was ours, not the system's; that is still the rung working, and it is
worth more than the greens below.

### §4.2 the ladder, after the fix — twelve arms, all graded on printed lines

| arm | T | I | cores | verdict | ops | overlap |
|---|---|---|---|---|---|---|
| `t3i128c3`  | 3  | 128  | 3 | PASS | 3 072 | 8 256 |
| `t3i512c3`  | 3  | 512  | 3 | PASS | 12 288 | 43 258 |
| `t6i256c3`  | 6  | 256  | 3 | PASS | 6 912 | 23 157 |
| `t12i128c3` | 12 | 128  | 3 | PASS | 4 992 | 28 829 |
| `t8i256c1`  | 8  | 256  | 1 | **NOTRUN** | 6 400 | 18 954 |
| `t16i256c2` | 16 | 256  | 2 | PASS | 12 800 | 95 444 |
| `t3i5000c3` | 3  | 5000 | 3 | PASS | 120 000 | 97 083 |
| `t24i300c3` | 24 | 300  | 3 | PASS | 22 500 | 299 868 |
| `t32i200c4` | 32 | 200  | 4 | PASS | 20 000 | 351 817 |
| `t48i200c4` | 48 | 200  | 4 | PASS | 29 600 | 615 195 |
| `replay-a/b`| 8  | 96   | 3 | PASS ×2 | 5 568 | 17 842 |

**Totals: 244 132 operations across 48 phases, of which 70 654 ran the engine and 21 418 were
the full alias property; 1 599 703 measured cross-thread RM-verb overlaps; 0 refusals; 0
invariant violations; 0 `Xid` in `dmesg` across every run.** Max width 48 threads on 4 cores.

⚠ **THAT IS A BUDGET, NOT A PROOF**, and the rung's own `PASS` line says so. 1.6 million
overlaps is 1.6 million *sampled* interleavings out of an unbounded space; absence of a red is
not absence of a race.

★ `t8i256c1` is `NOTRUN` **by design and it is the veto working**: at `cores = 1` the
`percore` arm is `T = 1` by construction, so it samples zero overlap and cannot be graded —
and the arm-level `NOTRUN` propagates rather than being averaged away by its three green
siblings. ⊘ The `crowd` arm at `cores = 1` — eight workers on **one** core — did run and
passed with 1 886 overlaps, which is the strongest preemption-inside-a-lock sample here.

### §4.3 the instruments, each checked rather than asserted

- **`W385_WATCHDOG=PASS`** on every ladder. Given a 5 s deadline against a 100 000-iteration
  workload it fired, printed `RUNG_concurrent_fuzz=FAIL` with `FUZZ_REASON=DEADLOCK/WATCHDOG`,
  dumped **8 worker rows** naming each one's last verb and iteration, and exited 3. ⇒ The
  no-hang guarantee is measured, not claimed.
- **`W385_REPLAY=SAME`** — two independent runs of seed `0x5EED0385` drew the **same verb
  census in every one of the four arms**, character for character. ⇒ `--seed=N` really does
  pin the decision sequence. ⊘ It does not pin the schedule: the two runs' overlap counts
  differed (8 788 vs 9 054), which is exactly the honest distinction §2.1b states.
- **Pinning is read back**, and the observed core sets are what the log prints:
  `{"0", "1", "2"}` for `percore`, `{"0-2"}` for `crowd`, `{"0-18"}` for the unpinned control.
  Every pinned arm reported `pinned=T/T`.
- **The distribution, not a summary.** At 48 threads on 4 cores the per-op cost separates the
  arms cleanly: `percore` (4 workers, 4 cores) `p50=548 µs p99=2 925 µs`, while `crowd`
  (48 workers, 4 cores) is `p50=6 652 µs p99=57 010 µs max=110 446 µs` — a 20× median and a
  94× tail, which is the over-subscription doing what it was asked to do.

### §4.3b ⚠ ONE HARNESS TRAP, MEASURED, BECAUSE IT READ EXACTLY LIKE A REGRESSION

The first `cargo test --workspace --no-fail-fast` on the bench box returned **19 failing
targets** against a documented baseline of **3**. Sixteen of the sixteen extra ones were
`traces/`-dependent (`cap1_differential`, `real_ga106_bodies`, `truncated_row_reads`,
`initctrl_census`, `replay_conformance`, …) and the cause was the **rsync that put the tree on
the box**: it carried `--exclude traces`, so a 93 MB directory the suite reads as its oracle
was simply not there. The failure message says so plainly once you look —
*"trace …/rpc_bodies_real_ga106.txt unreadable: No such file or directory"* — but the
**shape** of the result, a failure set that grew by 16 under a branch, reads as a regression
and nothing about the count itself distinguishes the two.

⇒ ★ **A test suite's failure set is a statement about the TREE it ran on, not only about the
diff.** An incomplete checkout manufactures reds that are indistinguishable from real ones by
count, and the number is the first thing anybody looks at. The re-run with `traces/` present
is the one that means anything, and *which tree it ran on* is part of the citation exactly as
`a_rulings_date_is_part_of_the_citation.md` says the date is.

### §4.3c ★★★★★ THE LANE TRIPPED THE META-GATE, AND THAT IS THE MOST VALUABLE RED IT CAUGHT

`crates/kayfabe-vmm-qemu/tests/gate_scope.rs` went **red on this branch and green on
`758a5752`**, with a message that says exactly what happened:

> *"★ kayfabe-linux-raw declares **93** relaxation(s) in ci.yml and the tree has **99**. A
> hand-maintained constant that stops matching the tree is a ratchet that has quietly become
> a comment"*

The six are the whole of `affinity_unsafe.rs`. ⊘ **This is the gate MEMORY.md names as the one
whose redness voids every other gate's coverage claim**, and the only reason it was seen is
that the suite was run at all — the diff itself compiles, formats and passes clippy without a
word about it.

★ **The number was bumped the way the gate demands, not the way that silences it.** `ci.yml`'s
own comment on the previous bump says *"bumping the number is the failure this gate exists to
catch"*, so all six new relaxations are read and recorded inline at the `AUDITED=` line: same
shape every time — an owned `cpu_set_t` on the function's own stack, unaliased, outliving the
call, with the syscall told the object's exact `size_of` and writing through no other pointer.

⚠ And the surrounding measurement needed a control of its own. My branch's failing set was
**7** targets and the brief's stated baseline was **3**, which reads as *"this branch broke
four things"*. Running the same targets on **pristine `758a5752`** showed **6 of the 7 already
red there** — the brief's `3` predates the `w383-doorbell-async` merge — leaving exactly one,
`gate_scope`, that was genuinely mine. ⇒ **A failure-set delta is only a delta against a
baseline measured on the same tree at the same commit**, and the number in a brief is a
citation with a date.

### §4.4 ⊘ WHAT THE GREENS DO **NOT** SAY

- Nothing about `l1_concurrency.md` §3.3 R1 as a **live** check — see §1. Both asserts are on
  the path of every ioctl and both are vacuous from a raw client.
- Nothing about the Mode-2 guest path: this is the native arm. The guest arm needs
  `--probe-launch-dma` (the rung already threads the selector through) and is a follow-on.
- Nothing about the completion plane: the probe is the w379 `SEM_RELEASE` by default, and
  `w381_the_guest_servable_probe.md` §3 already scopes what that can and cannot show.
- `pairing` reported *"placement did not change the answer"* on **every** arm. ⊘ That is a
  statement about these seeds and these widths, not a finding that pinning is unnecessary —
  the pinned arms exist so that the day it *does* change the answer, the change is visible.
