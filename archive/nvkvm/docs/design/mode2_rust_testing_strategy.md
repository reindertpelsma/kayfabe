# Mode-2 Rust rewrite — testing strategy

**Status:** design, 2026-07-22. Branch `consolidation`. Deliverable 3 of the pre-Rust gate.

**Mandate (decision #10, owner directive):** *"TESTING = first-class. The logic-only core is
DETERMINISTICALLY testable WITHOUT a GPU → build MEAN, HARD tests."* This doc makes that concrete,
inheriting the proven discipline of `/workspace/userspace-wireguard-socks` (uwgsocks):
**integration + malicious/fuzz-style + soak, three-tier CI cadence, ITERATE UNTIL GREEN, no merge on
red.**

**Reads with:** `mode2_rust_rewrite_architecture.md` (§4 crates, §4.5 oracle discipline),
`mode2_rewrite_consistency_audit.md` (the quirks that MUST become regression tests),
`mode2_abi_agnostic_layer.md` §6 (the version-resilience experiments V1–V3),
`mode2_address_table.md`. Governed by the **priority ladder** (decision #8): **security tests are the
highest bar**, then correctness comprehensiveness, then perf-parity.

**Ten test categories** across three cost tiers (§7). The through-line: **the code is testable by
construction because the core is a pure state machine over guest bytes** — testability is not a
tax bolted on, it is the same modularity argument that makes the rewrite hypervisor- and
arch-agnostic (§1).

---

## 1. Why the logic-only core is deterministically testable without a GPU

The rewrite's core is, by decisions #1/#2, a **pure state machine over guest-supplied bytes** — no
QEMU types, no syscalls, no OS knowledge. Everything effectful crosses one of three trait seams:
`Vmm` (hypervisor), `RmBackend` (host RM verbs), `Arch` (GPU-generation behavior). That single
architectural fact is what makes the core **deterministically testable with zero GPU**:

- **No hidden inputs.** The core's entire input is: bytes from `Vmm::gpa_read`, MMIO offsets/values,
  doorbell tokens, RPC messages, and `CoreEvent`s. Feed it a scripted sequence of those and its
  output (register reads, RPC responses, `Vmm::raise_irq` calls, `RmBackend` verb calls, host-op
  ordering) is a **pure function of the input** — replayable, diffable, deterministic.
- **The seams are the mock points.** A test supplies a `MockVmm`, `MockArch`, `MockRmBackend`
  (§4) and drives `Device::mmio_read/write/event` exactly as a real VMM would. No kernel, no
  `/dev/nvidia*`, no bench.
- **Testability drove the modularity, not vice versa.** The reason the address-table walk, vChid
  demux, completion model, and PT-capture live in the core (not the adapter) is *precisely* that
  they can then be tested as pure logic. A design where "does the loser's PD0[1] leaf resolve under
  its own PDB" requires a GPU boot is untestable at CI speed; a design where it is a `walk()` call on
  a synthetic FB image is a 1ms unit test. **The #14 saga — eight rounds, ~2min per bench boot — is
  the cost of NOT having this.**

**Consequence for the gate:** every quirk the C paid for in bench time (§2) becomes a
sub-millisecond deterministic unit test. The bench is reserved for the *irreducibly* HW-dependent
properties (§5) — real host-GPU execution, real completion timing, parity — nothing else.

---

## 2. (a) Regression tests — every C quirk becomes a unit test

The carried asset of this project is **not the C code — it is the captured quirk knowledge**
(decision, arch L13). Each hard-won fix below becomes a **named, deterministic unit test** the logic
core MUST pass, driven through the mock harness (§4). This is the "immediately test every quirk found
in the C" clause of decision #10 made concrete. A red here = a regression of a fix that cost days.

### 2.1 The #12 family (2nd-context / aperture / keying)

| Test | What it pins | Source |
|---|---|---|
| `t12_pdb_keyed_not_client` | A `hVASpace=0` GSP-managed channel resolves its VAS by **PDB**, never by client handle; two clients sharing one VAS map to **one** table entry. | table §13; `mode2_12_layered_status` cont.7 |
| `t12_minted_system_vas` | The device-default/system VAS gets a **QEMU-minted PDB** at device-alloc time; a `hVASpace=0` kernel channel resolves against it. | table §13.1 |
| `t12_own_vas_sema_resolve` | A CeUtils scrubber sema at a VA that faults under a stale foreign PDB is resolved via **content-validated own-VAS**, not a blind global `chan_pdb` fallback (the cont.33 sema-VAS-collapse). | `mode2_12_layered_status` cont.33 |
| `t12_finishpayload_forge_kernel_only` | The finishPayload forge is **typed to kernel/CeUtils channels**; forging a user GR/CE completion is **unrepresentable** (compile-time, via the `Traffic::System` enum). | fwd L5; arch §4.3.1 |
| `t12_2nd_ctx_gr_tsg_scheduled` | CTX2's fresh GR TSG is `GPFIFO_SCHEDULE`'d (not left off-runlist by a sticky one-shot `doorbell_setup`); per-`(Proc)` ExecPlane has no one-shot. | `mode2_12_layered_status` cont.34; arch ⚠4 |
| `t12_teardown_flush_compute_pins` | Compute-aperture sysmem pins flush on compute-channel teardown so CTX2 re-backs cleanly (no stale `st=0x51 ALREADY-MAPPED`). | `mode2_12_layered_status` cont.34(A) |
| `t12_gsp_reboot_handshake` | GSP suspend/reload across CTX1→CTX2 preserves seqNum, re-posts `GSP_INIT_DONE`, lowers WPR2 only on the guest's own SEC2 Booter Unload. | `mode2_12_layered_status` cont.32 (L2/L3a/L3c) |

### 2.2 The #13 family (512M-leaf / multi-iter realloc / CE-PT-write publication)

| Test | What it pins | Source |
|---|---|---|
| `t13_pd1_512m_leaf_walk` | The GMMU walker decodes a **GA10x PD1 entry with bit0=1 as a 512 MiB leaf** (not a PDE→INVALID→drop). Property-test against `kern_gmmu_fmt_ga10x.c`. | `mode2_13` round-4 part1; abi §3.1 B3 |
| `t13_every_page_size` | `GmmuFmt::page_sizes()` enumerates **every** real leaf (4K/64K/2M/512M[/256G]); a walk hitting an un-enumerated size is a **loud fault**, never a silent drop. | abi §4.2; #13 corollary L3 |
| `t13_ce_ptwrite_capture` | A CE copy/fill into a compute-VAS PT page is **latched dirty O(1)** and decoded **at the release semaphore** — the populate source when no bind-time RPC exists. | `mode2_13` fix_v6; arch L3 |
| `t13_decode_dirtied_directly` | At release, dirtied PT pages are **decoded directly from the page**, NOT via a root walk (leaf filled *then* linked a push later; a root walk can't yet reach it). Both build orders converge. | `mode2_13` fix_v6 "direct-decode" |
| `t13_xfer_none_guard` | A `DATA_TRANSFER_TYPE==NONE` launch is a no-op **only when** `!remap && !mscrub` (a CE MEMSET writes via remap/scrub — must not be zeroed). | `mode2_13` fix_v6 part4 |
| `t13_multiiter_realloc` | Five iterations of alloc→launch→sync→free at varying N (the `cup8_iter` shape) each resolve their remapped working set — no "runs=0 forever" after an iter boundary. | `mode2_13` repro |
| `t13_ctxcreate_safe_trigger` | The backing trigger fires for the compute-VAS remap but **NOT** during cuCtxCreate's kernel-CE scrubbing (the round-4 part2 regressor). Scope = compute VASes only. | `mode2_13` Opus cont.2 |

### 2.3 The #14 family (multi-process — the raison d'être)

| Test | What it pins | Source |
|---|---|---|
| `t14_identical_va_disjoint_proc` | Two `Proc`s with **identical guest VAs** (`0x200200000`) and **identical RM handles** (`0x5c000019`) resolve to **disjoint** VASes/backings by PDB/vChid. | `mode2_14` round-1 |
| `t14_vchid_demux` | 35 distinct doorbell tokens → 35 distinct `token[11:0]` vChids → each to exactly one `(Proc, Channel)`, zero collisions (E0 replayed as a unit test). | plan §1.4 E0 |
| `t14_ce_ptwrite_attribution` | A CE PT-write is attributed by **destination-FB-address → owning PDB** (not exec PDB); "REC != EXEC" is expected, not wrong-PDB capture. | `mode2_14` round-7 |
| `t14_per_proc_completion_no_starve` | A `Proc` that **submits nothing but polls** (`MC_SERVICE_INTERRUPTS`) still gets its pending completion re-posted, driven off **its own** poll — the round-8 starvation is impossible. | arch §4.3.2 |
| `t14_per_proc_exec_plane` | Each `Proc`'s GR channel/tsg/token/doorbell-setup is **its own** (nothing scalar/one-shot); both procs' GR TSGs schedule independently. | arch §4.3.1 ⚠4; #14 round-8 (both forks) |
| `t14_no_content_pick` | The blind content-pick and `bar1_wpg` MRU **do not exist**; an unresolved compute ring faults loudly, never picks another proc's PDB. | arch §4.3.1; table §6 |
| `t14_ring_pin_generation` | A per-channel ring pin is **generation-checked**, rejected if the pinned page reads torn/reused across channel-free (the round-5 #12 regression). | plan §4.1-4b |
| `t14_already_mapped_arena` | Two procs' identical VAs land in **disjoint GPA arenas** — `back_sys ALREADY-MAPPED` cannot occur. | arch §4.3.3; #14 QEMU log |

### 2.4 The address-table torn-walk + cross-cutting invariants

| Test | What it pins | Source |
|---|---|---|
| `taddr_miss_is_fault` | A lookup miss is a **loud fault**, never an opportunistic PDB walk (torn multi-level walk → wrong phys → cross-context leak). | table §6/§9 |
| `taddr_no_torn_read` | The PDB is read **only at the commit point** (release sema / invalidate), never mid-update. | table §5.1/§9 |
| `taddr_forward_only` | The table is **forward-populated**; there is no exec-time reverse-resolve entry point in the API (enforced by construction — the resolver takes only a `(Pdb, va)` lookup). | table §0 |
| `taddr_unmap_eager` | A removed/re-pointed range drops its stale host backing **before** the guest can reach it (unmap eager, map lazy, reclaim deferred). | table §5.2 |
| `taddr_membar_barrier` | The pushbuffer interpreter does not advance past a membar-invalidate until the table refresh is applied AND fenced work drained. | table §5.1 |
| `t11_userd_liveness` | An emulated-engine write to a page backing a **live host USERD** is a compile-time-visible case (type-distinguished), never a runtime accident. | fwd L5; #11 |

---

## 3. (c) Spec-compliant weird-order tests — protocol, not trace

Decision #4 ("correct-by-PROTOCOL, not by-trace") is made **testable** here. The C "works because the
guest did ops in that order"; the rewrite models the guest-driver protocol **contract** as a state
machine (from the ogkm source, not traces), so a reordered / retried / alternate-valid guest path
still reaches correct (partially fabricated) state. These tests generate op sequences that **normal
workflows don't hit but the spec allows** — especially the ones the C couldn't handle. A pass here is
the *proof* that the rewrite is protocol-faithful, not trace-brittle.

Concrete weird-order cases (each drives the mock harness §4 and asserts observable end-state):

- **`wo_pt_leaf_before_link` — leaf-then-link vs. link-then-leaf.** Publish a PT leaf page's PTEs
  *before* the PDE that links it into the tree, then in the opposite order. Both must converge to the
  same resolvable VA (the #13 direct-decode invariant, generalized: the C only handled one order).
- **`wo_promote_ctx_reorder` — Case-2 acks out of order.** Issue `PROMOTE_CTX` /
  `GET_CTX_BUFFER_INFO` before, interleaved with, and after the Case-1 alloc that actually creates
  the host object. The ack-only path must satisfy every ordering (fwd L2).
- **`wo_channel_alloc_then_immediate_doorbell` — no warm-up.** Ring a channel's doorbell on the
  first submission with no prior "settling" doorbells. Must resolve the ring via forward-populated
  binding, not a `bar1_wpg` MRU that assumes prior traffic (the round-7 compute-ring-resolution wall).
- **`wo_invalidate_absent` — map-and-use with no invalidate.** On the GSP-emulated compute path,
  publish a mapping via CE-PT-write with **no** `INVALIDATE_TLB`/`MEM_OP` (round-6: neither fires).
  The CE-release commit point must still populate the table; a use that references it must resolve.
- **`wo_retried_rpc` — duplicated / retried RPC.** Re-send an `RM_ALLOC` / `SET_PAGE_DIRECTORY`
  the guest RM might retry; the core must be **idempotent** (no double-backing, no seqNum desync).
- **`wo_teardown_midflight` — free while completion pending.** Free a channel's client root while a
  completion is in flight; the two-stage `retire()→Drop` + generation counters must prevent stale
  pin/cache consumption (L10; the P0 deferred-reap-at-quiesce lesson).
- **`wo_reordered_dup_edges` — dup-src after dup-dst.** Present the DUP_OBJECT graph edges in an
  order where the UVM handover is seen before the compute client; PDB-grouping must still anchor
  correctly (the round-3 transition-window bug, designed out by per-`Proc`-from-line-1).
- **`wo_alt_valid_pagesize` — same VA, different valid leaf size.** Map a VA as 2M, then (after
  unmap) as 64K or 512M; the walker must decode each without a stale size assumption.

**Why this category is load-bearing:** every one of these is a sequence the C either mishandled or
"happened to survive because the trace didn't hit it." The rewrite claims protocol-correctness;
these tests are the claim's teeth. They run at unit speed (mock harness), so the whole matrix is
cheap.

---

## 4. (b) The mock-GPU-arch harness — drive the core end-to-end, in-process

The centerpiece of GPU-free testing: a **deterministic, in-process fake GPU** the logic core drives
"as if real load." Three mock adapters implement the three seams:

```rust
/// A scripted hypervisor. Guest RAM + FB are plain byte buffers; MMIO/doorbells are
/// fed by the test; raise_irq / defer are recorded for assertions.
struct MockVmm {
    guest_ram: SparseBytes,          // gpa -> bytes (RPC queue, PT pages, sema targets)
    slots: SlotTable,                // map_guest / map_read_native / lock_region effects
    irqs: Vec<IrqSpec>,              // raise_irq log — assert completions delivered
    deferred: BinaryHeap<(Instant, CoreEvent)>, // virtual clock; no real timers
    now: Instant,                    // advanced explicitly by the test — determinism
}

/// A fake GPU generation. Encodes/decodes PTEs, tokens, RAMFC per a chosen regime
/// (VER2/Ampere by default; VER3/Hopper for the regime-boundary tests).
struct MockArch { fmt: GmmuFmtImpl, /* page sizes, token decode, class map */ }

/// A fake host RM. Records verb calls (alloc/control/map/free/dup), returns synthetic
/// handles, and can be scripted to fail (0x1b on a Case-2, NO_MEMORY on OOM) to test
/// the forwarding model's error handling WITHOUT a driver.
struct MockRmBackend {
    verbs: Vec<RmVerb>,              // the forwarded op log — assert Case-1 fwd / Case-2 absent
    fail: FailPolicy,                // scripted NV_ERR_* for negative paths
    host_objects: HandleMap,         // synthetic host handles + their VAs
}
```

**What the harness proves.** A test scripts a guest boot + a compute submission entirely as bytes
and events, then asserts on the recorded effects:

- **Boot:** feed the BAR0/RPC trace of a real fake-boot (recorded once from the C emulator, replayed
  as pure input — possible *because* the core is VMM-free, arch §4.5 step 2); assert identical
  register reads / RPC responses / `raise_irq` sequence.
- **Compute:** script a doorbell → assert the core rings the **right host token** on the **right
  `Proc`'s** isolate (via `MockRmBackend.verbs`), captures the CE-PT-write into the **owning PDB**,
  and posts the completion to the **polling** `Proc` (via `MockVmm.irqs`).
- **Multi-process:** instantiate two `Proc`s with identical VAs/handles (§2.3) and assert disjoint
  resolution — the whole #14 wall, at unit speed.

**The virtual clock is load-bearing:** `MockVmm.now` is advanced *explicitly*, so completion
re-delivery, deferred reap, and poll-kick budgets are deterministic — no flakes, no `sleep`. This is
the uwgsocks `chaosProxy`/single-socket discipline applied to time: the harness is
production-shaped but fully controlled.

---

## 5. (d) Real-GPU integration tests — the irreducibly HW-dependent bar

The mock harness proves *logic*; it cannot prove that a real host GPU *executes* the forwarded work
or that timing/parity hold. These properties **require the bench** (vast.ai serialized GPU host,
`ssh vh`/`vg`; fresh boot per clean run — emulated-GSP WPR2 resets only on full QEMU restart, arch
L12). Run **strictly serially** (concurrent tests / mid-ioctl SIGKILL wedge the GPU into D-state).

**What genuinely needs real HW (and why it can't be mocked):**

- **Un-forgeable compute correctness.** A real NxN fp32 matmul result is un-forgeable — QEMU never
  parses the compute pushbuffer and a software CE can only copy/memset (dataplane addendum
  2026-06-13). The `cup2 → cupctx2_min → cup8 → cup8_iter → 2×/3×/4× cup8` ladder, each rung
  **byte-exact** and cross-checked against a **host-native golden run** (never a green guest log —
  `mode2_real_forward_not_fake`).
- **The completion-sema-passthrough hypothesis (decision #7 / audit C3).** Whether passthrough semas
  + per-`Proc` completion queue close the residual loser-hang is a **bench** question — the
  first-milestone experiment. The mock harness can prove the *delivery logic* is per-process; only
  the bench proves the guest driver actually stops starving. **This test must disambiguate the two
  round-8 forks** (completion-delivery vs. GR-compute completion, audit N1).
- **CC-off plaintext submit (decision #5/#11).** That a CC-off GeForce / operator-controlled
  datacenter part never runs SPDM and accepts the faked GSP — verified on real silicon, not inferable
  from source alone (though source gates it, abi §5).
- **Nested-virt vs. bare-metal perf (audit N2).** The "~zero VMM traps" claim is bare-metal
  (`baremetal_32`); the bench must re-confirm parity on bare-metal and record the nested-virt tax
  (`execfwd` m582–584) — perf-parity is a real-HW property, ranked below correctness (priority ladder).
- **Real-app matrix.** LLM (llama.cpp CUDA) coherent output + PyTorch single-proc — the parity story.

**The bench ladder = the arch §4.5 acceptance ladder**, and each rung is **differential vs. the C
oracle** for single-process and **vs. host-native goldens** for output bytes.

---

## 6. Multi-process + security as first-class test dimensions

Per decisions #8/#9, these are **not** a category — they are **dimensions that cut across every
category**, and security is the **highest bar** on the priority ladder.

### 6.1 Multi-process dimension

Every applicable test runs at **N=1 and N≥2** `Proc`s. Single-process is the N=1 case of the only
code path (no `multiproc()` gate — arch L9), so N=1 must be **byte-identical** and N≥2 must not
regress it. Concurrency-specific tests: `t14_*` (§2.3) at the mock level; `2×/3×/4× cup8` all rc=0 +
`2× cup8_iter` (combines #13+#14) at the bench level (plan §5-P6 ladder). **Isolation smoke:** one
`Proc`'s fault/exit does not perturb another (mock: assert `Drop` reaps cleanly; bench: the Mode-1
reaper path, `teardown_hardening_done`).

### 6.2 Security dimension — the three boundaries, tested (highest bar)

Map to decision #9's three boundaries (arch §4.3.5). These are **blocking** in CI (uwgsocks
"Tier-1 chaos is BLOCKING, no `continue-on-error`" discipline):

- **Boundary 1 — userspace-process compromise.** **Fuzz the guest→core/isolate interface:** feed
  the core adversarial RPC messages, malformed pushbuffer method streams, out-of-range doorbell
  tokens, torn/overlapping PTEs, oversized alloc-param structs (the L11 truncation class). Assert:
  **memory-safe** (safe-Rust parse, no panic-as-DoS — bounded), **MISS=FAULT** (no confused-deputy
  resolution to another context's page), and **cannot reach its own isolate** beyond the
  unprivileged verb surface. Property/fuzz-tested (`cargo fuzz` targets on the `nvkvm-abi` decoder +
  the pushbuffer parser + the address-table lookup).
- **Boundary 2 — isolate compromise.** Assert an isolate can issue **only unprivileged host GPU
  ops** (a Case-2 control returns `0x1b`, never escalates — fwd §"0x1b lesson"); a compromised
  isolate reaches only the `Proc`(s) it serves (separate host process / fd table / handle namespace —
  assert via `MockRmBackend` handle-namespace isolation); the sandbox posture holds (seccomp
  allowlist, no `PROT_EXEC`, least-privilege RAM share — bench smoke on the real stub).
- **Boundary 3 — guest-kernel compromise.** Assert a compromised kernel forging PDBs / reshuffling
  isolate routing gains **no host reach** (every isolate unprivileged) and **no** intra-guest
  escalation it didn't already have; and that we add **no** VMM-side intra-VM access check (the
  reverted H-1 lesson — wrong layer).

**Adversarial + fuzz corpus** lives in a dedicated crate `tests/adversarial/` (the uwgsocks
`tests/malicious/` analogue): hostile RPC/pushbuffer/PTE generators + `cargo fuzz` targets. A crash,
a cross-context resolve, or an escalation is a **release blocker**.

---

## 7. The green-gate discipline — three tiers, iterate until green, no merge on red

Inherited from uwgsocks's three-tier cadence (its CLAUDE.md §"CI/testing model"). The split is
**load-bearing — do not collapse it**; the fast tier stays fast so devs never reflexively bypass it.

**Tier 1 — pre-commit / fast unit (seconds, `cargo test --lib`).** All pure-logic tests: the §2
regression corpus, §3 weird-order, §4 mock-harness end-to-end, the address-table property tests, the
walker property tests vs. ogkm formats (abi §6 V2 Axis-B). **No GPU, no bench, deterministic virtual
clock.** This tier alone catches every #12/#13/#14 regression the C paid bench-days for. Ruthlessly
`-short`-skip anything slower; a logic test that needs > ~1s is a design smell.

**Tier 2 — per-push CI (minutes, `cargo test` + `cargo fuzz` short runs + differential-vs-C).**
Adds: the **ABI differential oracle** (diff generated Rust ABI tables vs. the C's hand-coded
alloc-size/RPC tables — every discrepancy is a C bug or a generator bug, arch §4.5 step 1); the
`nvkvm-abi` codegen round-trip + `const_assert!(size_of == N)` checks; short fuzz sweeps on the
decoder/parser; the **"no concrete version/arch in a logic crate" lint** (a grep gate — no `V580`,
no `Ampere` in `nvkvm-core`/`-mmu`/`-fwd`/`-completion`, abi §6). Multi-process mock tests at N=2/3/4.

**Tier 3 — tag-triggered / bench (longer, serialized GPU + soak + chaos).** The real-GPU
integration ladder (§5), the **security bench smoke** (real stub sandbox), **soak** (repeated
alloc/launch/sync/free churn — the #13 pattern — and long-running concurrent `cup8` to catch leaks
across process churn, the R5 never-reaped-table class), and the **version-resilience drills** (abi §6
V1 "day-not-a-month" 580→575 re-target; V3 second-architecture drill). Bench runs **strictly serial**
(the harness enforces it, arch L12); chaos/soak are **BLOCKING** — a real failure stops the release
(the uwgsocks "no `continue-on-error` on real chaos" rule; the escape hatch is a manual re-run after
confirming an environment flake, never a pre-emptive silence).

**The green-gate rules (non-negotiable, uwgsocks-inherited):**

- **Iterate until green, then review.** Commit between milestones, push when a coherent batch is
  green — never per-test.
- **No merge on red.** A red Tier-1/Tier-2 blocks the merge; a red Tier-3 chaos/security blocks the
  release.
- **Tests must be mean and hard.** Production-faithful beats happy-path. If a test isn't exercising
  the real failure mode (e.g. a mock that resolves the loser's VA too easily), flag it and build the
  harder version — the #14 mock **must** reproduce the identical-VA/identical-handle collision, not a
  sanitized version.
- **Security is the highest bar.** A boundary-1/2/3 failure or a fuzz crash outranks every other
  signal (priority ladder, decision #8).
- **The C stays alive as the differential oracle** (arch §4.5 R1), not deleted — every single-process
  subsystem is "done" only when its differential tests pass against the C on the same traces.

---

## 8. Summary — the ten categories

| # | Category | Tier | GPU? | Proves |
|---|---|---|---|---|
| 1 | Regression (C-quirk) unit tests (§2) | 1 | no | every #12/#13/#14/address-table fix stays fixed |
| 2 | Mock-GPU-arch end-to-end (§4) | 1 | no | the core drives a fake GPU "as if real load" |
| 3 | Spec-compliant weird-order (§3) | 1 | no | protocol-correct, not trace-brittle (#4) |
| 4 | Address-table / walker property tests (§2.4, §5-abi) | 1–2 | no | MISS=FAULT, forward-only, every leaf size |
| 5 | ABI codegen + differential-vs-C oracle (§7) | 2 | no | generated ABI == ground truth; retires L11 |
| 6 | Multi-process concurrency (§6.1) | 1 (mock) / 3 (bench) | mixed | N=1 byte-identical, N≥2 no-regress, no starve |
| 7 | Security / adversarial / fuzz — 3 boundaries (§6.2) | 2 (fuzz) / 3 (smoke) | mixed | **highest bar**: no escape, no confused deputy |
| 8 | Real-GPU integration ladder (§5) | 3 | **yes** | un-forgeable compute, the #7 hypothesis, parity |
| 9 | Soak / churn / leak (§7) | 3 | yes | no cross-process/cross-teardown leaks (R5, L10) |
| 10 | Version-resilience drills V1–V3 (§7, abi §6) | 3 | mixed | day-not-a-month re-target; zero-core-edit new arch |

**The one-sentence strategy:** *make the core a pure state machine so that every quirk the C paid for
in bench-days becomes a sub-millisecond deterministic unit test (categories 1–5), reserve the
serialized bench for the irreducibly HW-dependent bar (categories 8–10), treat multi-process and
security as cross-cutting dimensions with security as the highest gate, and never merge on red.*
