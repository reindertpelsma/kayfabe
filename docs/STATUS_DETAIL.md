# Status in detail — what runs, what does not, and how it was measured

> ### STATUS — 2026-09-06 / **LIVE**
>
> This file is the long-form status that used to live in `README.md`. It was moved here on
> 2026-09-06 so the front page could be read by a human in a minute; **nothing was deleted**,
> and the text below is preserved verbatim from the README at `04860c81` except where a
> heading was added to separate a section.
>
> ⚠ Every measurement here carries its own date and revision. Where a dated measurement and a
> prose summary disagree, believe the measurement — that is the house rule, and this file
> exists to keep both visible rather than to quietly resolve them.

---

## 1. Work in progress — do not run any of this yet

Kayfabe is under **active development** and is published early, deliberately: so the
approach can be read and argued with, not because it is finished or usable.

**Nothing in this repository is ready to run.** There is no supported build, no
install path and no stable entry point. Interfaces, crate layout, on-disk formats and
the design docs all change without notice, and the code is expected to be broken at
any given commit. Do not point it at hardware you care about.

## 2. The test suite does not pass clean

**`cargo test --workspace` does not pass clean.** Measured at `ee50148d`, 2026-09-06:
**2949 pass, 9 fail, across 258 test binaries.**

⚠ **Reproduce it with `--no-fail-fast`, or you will not get that number.** Plain
`cargo test --workspace` **stops at the first failing target**: measured the same day, it
ran **18** of the 258 binaries and reported **2** failures — a stopping point wearing the
costume of a result. Check `grep -c '^test result:'` on the output; if it is not ~258, the
run is void whatever it printed.

⊘ **This block previously read "1554 pass, 1 fails" at `06bbfd9e`.** That pass count is far
below a complete run's and pairs with exactly one failure, which is the signature of the
truncation above — so it was most likely never a whole-suite measurement. Corrected rather
than quietly updated, because the number's *shape* is the more useful warning.

### ⊘⊘⊘ AND THE EXPLANATION THAT USED TO STAND HERE WAS WRONG — corrected 2026-09-06.

This block said the failures were *"a bookkeeping gap, not a functional defect."* **They are
not.** That sentence was inherited across several rewrites and never re-checked against what
the tests actually assert. Read in full, **at least 6 of the 9 are functional, safety or
structural defects**, including two the project explicitly forbids:

- `a_guest_doorbell_reaches_the_host_completion_observer` — *"**THE SEVERANCE.** … `Served`
  here means: we rang a doorbell on a host channel **into which the guest's methods were
  never copied**."*
- `the_observers_negative_verdict_refuses_the_guest_doorbell` — *"The engine never released
  the semaphore and the guest was told `Served` … a caller that discards the verdict
  **forges the completion**."* ⚠ **Completion forgery is the one thing this project rules
  out by name.**
- `a_wired_device_refuses_a_framebuffer_page_nothing_ever_wrote` — *"reads 4 KiB of zeros
  and reports SERVED."*
- `a_device_with_no_fb_source_refuses_the_vidmem_ring` — an unregistered device must refuse
  and does not.
- `the_logic_crates_carry_no_unnamed_guest_os_assumption` — a live guest-OS **axis**
  violation (`kayfabe-abi/src/submit.rs:5095`).
- `every_unranked_lock_a_vcpu_thread_can_hold_is_classified` — a **new unranked lock on the
  vCPU path**; *"a wait beneath this will pass every assertion and stall the register
  plane."*
- `the_audited_crate_list_matches_the_tree_and_is_used_by_all_three_sub_gates` — the
  **meta-gate**, reporting that another gate has gone slack: 91 declared relaxations against
  93 in the tree, *"a ratchet that has quietly become a comment."*

The remaining 1–2 genuinely are ledger bookkeeping, including control `0x83de030c`
(`NV83DE_CTRL_CMD_DEBUG_READ_ALL_SM_ERROR_STATES`), documented in
`design/w329_wiring_the_release.md`.

★★★ **CONFIRMED, and it was already adjudicated.** The four doorbell/ring failures are
**one cause, not four**, and `da86fc26` (w296, 2026-08-14) named it three weeks before this
block was written: *"NOT FIXED, DELIBERATELY. Per the rung's own rule: a red that turns out
to be a real product decision gets NAMED and STOPPED at… These five need an owner ruling."*

The cause is `8cca3502` (w287), which scoped ring-content forwarding **off** passthrough
channels — a change w296 judged *right on its own terms*. The resulting severance is proven
by construction rather than observed:

> `Emulated` ⟺ anchor is `SYSTEM_ANCHOR` ⟺ routes to `SYSTEM_PROC` ⟹ §12.26 refuses all
> three `Binding::host` sites ⟹ no operand can be host-backed ⟹ `HostCe` unreachable ⟹
> `await_semaphore` unreachable.

⇒ **On the kind whose ring we read, no operand may be host-backed; on the kind whose
operands may be host-backed, we do not read the ring.**

⊘ So these are **not defects that slipped through** — they are a blocker that was found,
adjudicated and deliberately stopped at, and they are red *because the tests are doing
their job*. They must not be edited to pass. What they await is a design ruling, not a
repair.

⚠ **Why this correction is kept rather than quietly replaced:** explaining a failure away
is how a project loses an instrument. The question is never *"is my explanation
plausible"* — it is *"if I am wrong, what now goes unnoticed?"* Here the answer was: the
completion-forgery guard, the residency guard, and two axis gates.

## 3. What actually works today

`archive/nvkvm/` is the **C research prototype**. This repo is the **Rust rewrite**.
Both have run real CUDA workloads on real hardware; they are at different stages.

All C figures below are **Mode-2** results (emulated GPU + faked GSP — the same thing
kayfabe rebuilds). The C repo also holds **Mode-1** work, a different design whose
maintained descendant is [nvkvm-pv](https://github.com/reindertpelsma/nvkvm-pv); no
Mode-1 number appears here.

| | C prototype — **Mode 2** | kayfabe (this repo) |
|---|:---:|:---:|
| Stock unmodified guest driver vs emulated GPU + faked GSP | ✅ | ✅ |
| Real RM ioctl to `/dev/nvidiactl` on hardware | ✅ | ✅ 2026-07-29 |
| `cup3` — context create / launch | ✅ | ✅ `CUP3_VAL=43`, all 8 perf arms |
| `cup8` — matmul, PTX JIT, closed-form check | ✅ N=1024 `bad=0 maxerr=0` | ✅ `N=2048 bad=0 maxerr=0 → PASS`, and N=3072 (36 MiB operands) × 12 iterations |
| llama.cpp inference (Qwen2 GGUF) | ✅ 49.9 tok/s vs 47.5 host-native | ❌ not run |
| PyTorch 2.5.1, 50-step training loop | ✅ byte-correct, `rc=0` | ◐ CUDA runtime initialises and `torch.cuda.is_available()` is True, but the model does not run — `_cuda_init()` raises "CUDA unknown error", 0 tokens |
| **Performance vs native** | ✅ ~zero forwarding overhead on bare metal | ❌ **22–81× off native** for large kernels; small ones dominated by our own doorbell handler |
| `nvidia-smi` enumerates (`SMI_RC=0`) | ✅ | ✅ |
| `nvidia-smi` process table lists running processes | ❌ | ❌ `No running processes found` |
| Host isolate runs **unprivileged** | — | ✅ `CapEff/CapPrm/CapBnd = 0`, `NoNewPrivs: 1`, uid 65534 |
| Portable across hypervisor versions | — | ✅ same overlay built into QEMU **9.2.0 and 10.2.4**, both booted |
| **Concurrent multi-process** — bug #14's own shape | ❌ one CUDA process per QEMU lifetime | ✅ **branch only** — two guest CUDA processes both `=43`, incl. a staggered arm starting the 2nd *inside* the 1st's `cuCtxCreate` |
| Sequential CUDA processes in one boot | — | ◐ **branch only** — 3 pass, the 4th fails |

**The full account is the architecture paper** — [`whitepaper/kayfabe_architecture.pdf`](whitepaper/kayfabe_architecture.pdf), written to be attacked, with roughly half of it about what does not work, is not built, or is not known. It is the best thing to read next.

**Hardware.** C: bare metal, RTX 3050 (GA106), host open driver 595.71.05, 2026-06-16
([`MILESTONES.md`](../archive/nvkvm/docs/MILESTONES.md)). Kayfabe: GA106, driver 580.159.04,
bench rebuilt 2026-08-19 ([`w330_the_bench_rebuild_and_three_flags.md`](design/w330_the_bench_rebuild_and_three_flags.md),
[`RESUME_HERE_2026_08_15.md`](design/RESUME_HERE_2026_08_15.md)).

**⚠ The multi-process rows are on a branch, not on `master`.** Both were measured on
`origin/w337-gpu-name-seam`: the concurrent result is **w299**, 2026-08-14, rev `f459cffa`
(`whitepaper/kayfabe_architecture.tex:2021`); the sequential one is 2026-08-20, rev
`cca2eb4b`. Both on GA106. `master` carries neither, and **`master`'s own whitepaper still
states the opposite** — that the multi-process property is unmeasured on hardware
(`:2417`, `:2911`). Those lines predate w299 and were never updated. Believe the dated
measurement, not the stale paragraph.

The staggered arm is what makes the concurrent result bug #14's scenario rather than a
lookalike: #14 was *two concurrent apps hang at `cuCtxCreate`*, and that arm deliberately
starts the second process inside that exact window, gated on the first's own print rather
than a timer.

**None of this is a multi-tenancy claim.** There is **no tenant axis at all** — no
`VmId`, `TenantId` or `GuestId` exists anywhere in `crates/`. Cross-VM separation is the
incidental consequence of two host processes being separated by the host NVIDIA driver,
whose own cross-client check is defeated by a shared euid. **No two-VM run has ever been
attempted.** Multi-tenancy is the thesis; it is not yet a result.

Two qualifiers that stand: the **4th** sequential process fails, and follow-up work found
the ceiling is **device opens, not processes** — `nvidia-smi` ×8 with no CUDA anywhere
fails from the 5th onward. "Two guests" and "two isolates" are **not recorded anywhere**
and are not claimed here.

## 4. The one defining constraint

**The logic core is a pure state machine over guest-supplied bytes** — no OS, no
syscalls, no hypervisor types, no wall clock, no NVIDIA struct layouts, no
driver-version or GPU-generation constants. Everything effectful crosses a trait seam:

- `Vmm` / `Device` / `Present` (hypervisor + display adapter) — `crates/kayfabe-vmm`
- `Arch` / `GmmuFmt` / `UserdModel` / `PushbufferAbi` (GPU-generation behavior, "Axis B")
  — `crates/kayfabe-arch`
- `RmBackend` / `Isolate` / `IsolateFactory` (unprivileged host RM + sandbox) —
  `crates/kayfabe-isolate`
- `DriverAbi` (driver-version wire layouts, "Axis A") — `crates/kayfabe-abi`

The logic crates (`kayfabe-core`, `-mmu`, `-fwd`, `-completion`) are written **only**
against those traits. Every seam's *only* implementations today are the deterministic
mocks in `kayfabe-mocks` — **no real adapter (Linux, QEMU, or NVIDIA arch) exists yet**,
by design: the layers below descend next (L1 Linux OS → L2 QEMU → L3 per-arch codegen).
Bringing up a real GPU generation will be `impl Arch for <Gen>` in an adapter crate with
zero edits to any logic crate; `MockArch` (deliberately non-NVIDIA encodings) is the
standing proof of that seam.

> ⊘ **CORRECTED 2026-09-06 during the README rewrite — the paragraph above is stale in two
> places, and it is kept rather than edited because the drift is the useful part.**
> `crates/kayfabe-vmm-kvm`, `crates/kayfabe-vmm-qemu`/`kayfabe-qemu-raw` and
> `crates/kayfabe-isolate-host` are real adapters in the tree today, so *"no real adapter
> (Linux, QEMU, or NVIDIA arch) exists yet"* is false of the first two axes and true only of
> the NVIDIA-arch axis — and §5's own layer table, three sections down, already contradicted
> it. `ARCHITECTURE.md`'s port table carries the same staleness for the `Isolate` row
> (*"trait-only (mock-implemented)"*) while `kayfabe-isolate-host` implements it.

`#![forbid(unsafe_code)]` is a workspace lint (zero unsafe blocks anywhere); every core
type is compile-time-asserted `Send + Sync`.

> ⊘ **CORRECTED 2026-09-06 — "zero unsafe blocks anywhere" is false, and the exception is
> deliberate and gated.** `unsafe_code = "forbid"` is the workspace lint, and exactly **two**
> crates relax it in their own manifests: `kayfabe-linux-raw` (86 `unsafe {` blocks) and
> `kayfabe-qemu-raw` (29). Both are audited crates by design — every file using the keyword
> must be named `*_unsafe.rs`, every block carries a `// SAFETY:` comment
> (`undocumented_unsafe_blocks = "deny"`), and CI ratchets the relaxation count. See
> `crates/kayfabe-linux-raw/src/lib.rs`'s module docs and `crates/kayfabe-qemu-raw/Cargo.toml`
> decision Q2. The true statement is *"no unsafe outside the two audited raw crates"*.

## 5. Layers: L0–L2 built and running on hardware; L3 deliberately absent

The stack is layered L0 (pure logic core) → L1 (Linux OS layer / threaded shell) →
L2 (QEMU / VMM adapter) → L3 (graphics pipeline).

| layer | state |
|---|---|
| **L0** — pure logic core | complete |
| **L1** — Linux OS layer / threaded shell | built |
| **L2** — QEMU / VMM adapter | **built and run** — the same overlay compiled into QEMU **9.2.0 and 10.2.4** and booted on both (`design/l2_qemu_adapter.md:1240`) |
| **L3** — real Vulkan/GL pipeline | **deliberately absent**; the seams are done and typed (`design/core_state_and_consolidation.md:204`) |

It is **past mock-only**: a real RM ioctl landed 2026-07-29, and `cup3`/`cup8` are green on
GA106 — see the table above for exactly what ran and where.

⚠ Some in-repo material has not caught up with this. `design/l1_architecture_diagram.py`
still renders "L2 — QEMU / VMM adapter … NOT BUILT", and `master`'s whitepaper still calls
the multi-process property unmeasured. Prefer dated measurements over prose; where they
disagree, the measurement is newer.

L0/L1 in detail — the core that the hardware work sits on, mock-tested and mutation-gated:

- the `RmGraph` source of truth (refcounted RESOURCE/HANDLE split, DUP aliasing,
  order-tolerant parked facts, capacity-bounded) + pure projections (`Proc` grouping,
  `(GpuId, Pdb)` / `(GpuId, VChid)` routing);
- the `Gpu`/`Proc`/`Vas`/`Channel` runtime spine — per-process ownership of all four
  planes (address / execution / completion / isolate + GPA arena), transactional
  `apply` with rollback, retire-eager/reap-deferred lifecycle, full multi-GPU axis
  (per-`(Proc, GpuId)` isolates + arenas, per-target GPA windows + delivery);
- the per-`Vas` address table (forward-populate only, MISS=FAULT) and the ONE
  structurally-gated doorbell ring path (the #14 fix, unbypassable by construction);
- the per-proc completion plane (poll-driven re-delivery — the starvation fix) + the
  mapped-fence arm with the #12 jump guard;
- the ONE pushbuffer parser (CE-PT-write capture, sem-release, TLB-invalidate,
  everything else opaque), the Case-1 forward / Case-2 ack-only control split, the
  engine-aware channel alloc (GR-1) and the typed `Present`/`SurfaceHandle` seam (GR-2).

## 6. What is not built

What is **not** built (stubs / skeletons, documented in their `lib.rs`): ~~`kayfabe-abi`
(Axis-A codegen — trait shape only)~~ — **★ corrected 2026-07-27: `kayfabe-abi` is
built, not a stub.** It carries a working offline generator, generated `#[repr(C)]`
structs, a version-dispatch decode surface and its own oracle tests
(`crates/kayfabe-abi/{gen,src/generated,tests}`) — the "shape only" line was true when
written and has been false since the codegen landed; found by the whitepaper's
verification pass. Still genuinely unbuilt: the GMMU walker (`kayfabe-mmu::walker` —
`FbRead` trait + `WalkResult` enum, no walk loop). ~~And `kayfabe-gsp` (GSP boot FSM)
— [unverified 2026-07-27: that crate is under active construction; read its own
`lib.rs`, not this line].~~ **★ corrected 2026-07-28: `kayfabe-gsp` is BUILT** (S0–S5,
8 modules, ~3,550 L). What is genuinely unbuilt there is **reboot/resume** (S6–S8,
hardware-blocked) and — more importantly — **its bridge to the core**: `RpcCommand` has
zero references outside the crate, so nothing yet turns a decoded RPC into an `RmEvent`. ~~`kayfabe-trace` (a one-method trait)~~ — **built**: the typed `TraceEvent`
vocabulary (`mode2_gsp_port_plan.md` §6), the `TraceSink` port, one-counter total ordering,
perf-budget counters and the projection differential. Its plane call sites are *not* yet
threaded; it is driven from the conformance suite's seam observer
(`tests/tests/trace_replay.rs`).

On top of it, the **L1 threaded shell** (`kayfabe-rt`) is built and hardened: ranked locks
with always-on R1/R3 asserts, plan/execute/commit at every verb site, the bounded N-worker
isolate pool, the completion-source reactor as a pure core port, condemned components, and
the conservation ledger. **L1-M2 (the real OS shell — reactor, `kayfabe-linux-raw`, the
`Vmm` seam, the reclamation lifecycle) is designed and part-built**:
`design/l1_concurrency.md` and `design/l1_os_shell.md` are the live documents, and
their §12 / §14 contact logs are where the design has been *wrong* — read those before
trusting any summary, including this one.

## 7. Verification, CI and mutation score

**Verification:** the suite is in the **500s** as of 2026-07-27 and green (unit +
integration + proptest fuzz + concurrency stress + soak; nothing `#[ignore]`d, the
measured-slow tests gated on `KAYFABE_SLOW=1` and skipped loudly otherwise), clippy `-D
warnings` clean, fmt clean. `cargo test --workspace` is the count of record — a literal
number here rots within the week, which is exactly the drift this paragraph kept
producing (★ corrected 2026-07-27: said "283 tests"; found by the whitepaper's
verification pass).

> ⚠ The paragraph above is a 2026-07-27 snapshot and §2's 2026-09-06 measurement supersedes
> its "green": the workspace is **258 test binaries, 2949 pass / 9 fail**, and the failures
> are adjudicated rather than unknown.

CI is **six jobs**, not the four gates this line used to name (★ corrected 2026-07-27:
`.github/workflows/ci.yml` is the list of record) — `stable` (build, test, clippy, fmt,
and the boundary/vocabulary/unsafe-surface/GPA-accessor/unsafe-containment/KVM-floor
greps: eleven steps in that job alone), `aarch64`, `nightly-fuzz`, `slow`, `tsan`
(ThreadSanitizer over `concurrency_stress`, `rt_shell`, `l1_verb_seam`, `l1_mean` — **65**
`#[test]` functions in those four targets as of 2026-07-27; the "0 races" result is from
the first campaign, which counted 28 tests, and has not been re-run since), and `mutants`.

**Mutation score: not quotable right now** (★ corrected 2026-07-27: this paragraph used to
quote **99.2%** L0 and **92.44%** L1 with a 91% CI floor as settled). The gate's scope
changed on 2026-07-27 from four hand-picked paths to every production crate, and the
workflow marks the 91% threshold *pending re-derivation* in its own words. The prior
numbers are not wrong; they describe a different population. Read
`design/core_mutation_gate.md` — including what must be re-run before any score is
quoted again — rather than a number here.

Fifteen real core bugs were found and fixed **pre-hardware** by the adversarial suites
(fuzz, security invariants I1–I4, determinism differential, mutation gate); the L1 suites
have since found more, including a refcount bug in the source of truth and a
use-after-free introduced by a leak *fix*.

**What is measured on real hardware lives in `docs/reference/`** — not in the design docs, so
a wrong fact is corrected once: `rm_semantics_measured.md` (RM/UVM semantics, with the driver
version caveat) and `mode2_bench_lifecycle.md` (the C artifact's teardown behaviour). The C is
a **single-process** Mode-2 oracle — measured, §1 of that file.

Which claims in this tree are *measured*, which are *read out of the driver's source*, and
which are neither is not left to the reader: `design/claim_ledger.md` is the census, and
CI enforces it. Reading the open kernel modules tells you what the driver **does**; only a
live boot with real work tells you what **happens**, and a comment that says "measured" when
it means "read" converts an inference into a fact with no experiment behind it. Run
`scripts/claim_ledger.py`.

## 8. The authoritative test run is not GitHub CI

★★★ **GitHub CI is opportunistic convenience, not the definition of green** (owner ruling,
2026-07-30). The authoritative run is `scripts/run_full_suite.sh` on real hardware: it runs
the whole `stable` job plus the five other CI jobs plus the phases CI structurally cannot do
(real KVM, real namespaces, the vendored ogkm trees, a real GPU), and it ends in a
`RAN / FAILED / SKIPPED / ACKNOWLEDGED` ledger where every skip is named with its unmet
requirement. Exit 0 means *everything this box can run, ran*.
**`reference/full_suite_on_real_hardware.md`** is the census — which gated families
exist, what each requires, what it does when the requirement is absent, and (★) what had
never run anywhere at all.

`KAYFABE_SLOW` is the ONE slow-test switch (doc: `tests/src/lib.rs`). It is env-only
because Rust's libtest takes no custom CLI flags; with it unset every gated test
prints a `SKIPPED (slow): … set KAYFABE_SLOW=1` line rather than silently vanishing —
nothing in the suite is `#[ignore]`d. (★ corrected 2026-07-27: this said "the two gated
tests"; membership has grown since it was measured — the `skip_slow!` call sites are the
list of record. Found by the whitepaper's verification pass.)
