# ★★★★★ RESUME HERE — w392 overnight, 2026-09-09

**STATUS: LIVE.** Written for a compacted context. Everything below is measured unless marked.

## ★★★★★ THE STATE — 3 OF 4 ROWS GREEN (w392q, rev `b703e477`)
```
P1 rm-invalidate  ✔ VERIFIED over 4 round(s)
P2 uvm-memop      ✔ VERIFIED over 4 round(s)      ← the 4 KiB join fix (w392q)
P3 rpc-bind       ★★★ CONTENT MISMATCH: 0x9140000000 still the poison 0xdeadbeef after 3s
STALE RACE        ✔ VERIFIED over 2 round(s)
MEAN_FALSIFIER=PASS       W392D_OUTCOME=(F)
FbLeafGranularity refusals 0   OPERAND-JOIN(P2 token) 2 JOINED
host Xid 1 — and it is the FALSIFIER'S OWN unmapped VA. All real work completes without faulting.
```
**P3 is a NEW SHAPE: not a timeout, not a fault — the copy completes and the destination still holds
the poison.** It is the GR/compute lane; the CE lanes all pass. Its setup all succeeds (arm A's
negative control fires, arm B's `UVM_REGISTER_CHANNEL` is accepted, arm C's schedule then succeeds).
⇒ Look at whether the GR lane's operands are joined at all (`join_operand_fb_leaves` is the CE path)
and whether anything gates GR to `CpuCe`.

## SUPERSEDED — 2 OF 3 (w392p, rev `9977197c`)
```
P1          → ✔ VERIFIED over 4 round(s)        ← FIRST EVER GREEN ROW IN THE GUEST
STALE RACE  → ✔ VERIFIED over 2 round(s)
P2          → ⊘ REFUSED … 0x9080000000 NEVER RETIRED
MEAN_FALSIFIER=PASS
GR-BIRTH: adopt=GUEST-RING x8, userd=GUEST-USERD x8   ← BOTH halves are the guest's, at last
BIRTH-AT-ALLOC proc=2 kind=Passthrough x5+           ← births now at CHANNEL ALLOCATION
host Xid = 2, and BOTH are accounted for:
   CE0 @ 0xa0_00000000  = the FALSIFIER's deliberately-unmapped VA (why the falsifier passes)
   CE2 @ 0x90_80000000  = P2's operand — the ONE real failure left
```
**The remaining wall is the UVM lane only.** `UVM published the ring object at 0x9000000000` and
`COPY(2) bound to the UVM-owned space, runlist 1` — a VA space **`nvidia-uvm` owns and RM does not
manage** (`RingSource::OursPlaced`'s documented case). P2's operand at `0x9080000000` is not in the
host VAS, so CE2 faults `FAULT_PDE`.

## SUPERSEDED ONE-LINE STATE (kept for the trail)
The raw mean client now **adopts the guest's ring** and the host engine **executes the guest's
pushbuffer** (`Xid 0 → 5 × Xid 31`). It still fails, and the remaining wall is **FB-JOIN ALIASING**:
one framebuffer page mapped at two guest VAs, host object bound at only one.

## ✅ TEST SUITE + CI STATE (measured at `f60f793c`, per-target, 258 targets)
**250 GREEN / 8 RED at HEAD → 252 / 6 after repair.** Enumerated per target with rc captured
without a pipe — `cargo test --workspace` reports a stopping point, not a result.
- **Fixed:** `e2_doorbell` (2, superseded contract → now pins BOTH refusals by name);
  `publish_census` (1, the predicted 4 KiB-granule consequence → trip rows made sub-page);
  ★ **`l1_mean` was SIGABRT-ing over its own results** — a `VerbHold` timeout panicked **while
  holding the mutex** → `PoisonError` in a `Drop` → double panic → abort masked the entire binary.
  Mock hold-locks made poison-tolerant; now 35 pass / 11 fail where **nothing** was reportable.
- **Still red (6), all classified:** `doorbell_reaches_the_completion_observer` (3) and
  `ring_out_of_our_own_framebuffer` (2) are **pre-existing** w287 severance; plus
  `admitted_is_served` (2, missing trace data), `unranked_locks` (1), `guest_ring_census` (2) —
  identical at the pre-tonight baseline `e570ad90`.
- **11 `l1_mean` reds need the Dup/LateMerge RULING** (7×) or a shared-helper change to
  `ring_gpa_for` (2×) — deliberately not rewritten.
- ⚠ **CI has been RED on every push since 2026-07-30** (last green `30510591257`) — **not tonight's
  doing**; current failure is runner-environment (`execve errno 13`), and HEAD gets **further** than
  the baseline, which failed earlier on a compile error. `fmt` red (tonight's part is
  `kayfabe-device`), `clippy` red mostly at baseline.
- ⊘ **Clippy's "`release_store_join_carrying_bytes` never used" is a FALSE ALARM** — it runs on
  default features, where the whole `host-isolates` block is compiled out. The call is live at
  `shim.rs:11710`.

## ⚠ UNBOOTED AS OF THIS WRITING
`f60f793c` (w392t, **settle-before-birth** — the P3 fix) is committed and **has never been booted**.
That is the next bench action after the LLM run frees the box.

## ★ THE FULL GOAL CHAIN (owner, 2026-09-09 ~04:00 CEST) — in order
1. **LLM works and all client gates green** (P1 ✔ P2 ✔ STALE RACE ✔; **P3 still red**; LLM text is
   byte-identical to the CPU oracle but a **ledger-printed graded verdict is still owed**)
2. **all tests and CI work**
3. **LLM parity** (tok/s against a real host)
4. **the compute workloads `nvkvm-pv` can run**
5. **porting to all driver archs**
Use Fable as heavily as needed. ⏱ **GENUINE STOP AT 07:00 CEST = 05:00 UTC**, whatever is reached.

## SUPERSEDED GOAL CHAIN
1. get `--uvm-mean` passing in the guest ← **HERE**
2. then get LLM tokens flowing
3. then parity tok/s
**Stop at 07:00 CEST.** Rent vast boxes freely; **kill them all at the end**. Box in use: `50260029`
(alias `w391`, GA106, `/dev/kvm`, source at `/root/kayfabe`).

## HOW TO RUN IT (every one of these was got wrong once tonight)
```bash
# 1. ship code:  local /workspace/kf-docs → bundle → box
git bundle create $S/kf.bundle master && scp $S/kf.bundle w391:/root/kf.bundle
ssh w391 'cd /root/kayfabe && git fetch -q /root/kf.bundle master && git checkout -q FETCH_HEAD'
# 2. BUILD *WITH THE FEATURE* — the default is empty ON PURPOSE
KAYFABE_SHIM_FEATURES=host-isolates bash scripts/bench/provision_bench_tree.sh
#    assert:  grep -c "archive features: host-isolates" /tmp/trackA.log   MUST be >= 1
#    ⊘ build_qom_shim.sh writes to /tmp/trackA.log, NOT to your own log. I asserted the wrong
#      file once and read a GOOD build as a failure.
# 3. BOOT *ARMED* — a fresh boot_capture.sh invocation inherits NOTHING
export KAYFABE_ISOLATES=real KAYFABE_GUEST_RAM=memfd NVKVM_RAM_BACKEND=memfd \
       KAYFABE_FB_JOIN=shared KAYFABE_GUEST_RING=ring KAYFABE_GR_ROUTE=passthrough \
       KAYFABE_OPERAND_JOIN=join KAYFABE_PT_SWEEP=on KAYFABE_VAS_PUBLISH=drain \
       KAYFABE_PT_WITNESS_EXEC=on KAYFABE_CE_EXECUTOR=host BOOT_TIMEOUT=180
POST_CAPTURE_HOOK=scripts/bench/w392d_mean_hook.sh \
  UVM_BIN=/workspace/bench/cargo-target-w290/release/kayfabe-rm-ladder \
  bash scripts/bench/boot_capture.sh <tag>
```
⚠ **`boot_capture` exits `rc=5` on an evidence-persist check even when the run is perfect.** Read
the artefacts, not the rc. ⚠ Verify arming from the boot's own banners: `GUEST-RING arm=ring
host_isolates=true`, `FB-JOIN arm=shared`, `GR-ROUTE arm=passthrough`, `VAS-PUBLISH arm=drain`.
`arm=off` anywhere ⇒ **you measured the negative control** (I did this and reported it as a result).

## WHAT WAS FIXED TONIGHT (commits `41a79d42a`, `d0783ab6`, `c117991c`)
1. **`VerbPlan::Doorbell` gained `adopt`**; `plan_doorbell` now decides **by channel kind**:
   `Emulated ⇒ None` (ours is correct — we drive that ring, it runs our function bodies);
   `Passthrough ⇒ adopted_guest_ring(..)` (ours is **illegal by construction**).
   ⊘ It previously passed a literal `None` regardless of kind.
2. **The adoption gate was mis-scoped.** `!= JoinsGuestWindow` exists to refuse
   `ShadowsGuestMemory` — which **cannot be constructed** (`Binding::real_gpu_memory` refuses it;
   *"there is no spelling of this state that reaches `AddressTable::bind`"*). So it actually
   refused `GuestPhysDma + SoleBacking` = **the guest's own RAM pinned**, the shape
   `GuestRing::memory`'s doc calls production. Now accepts **`JoinsGuestWindow` OR
   (`GuestPhysDma` + `SoleBacking`)**, still refusing `RealGpuMemory + SoleBacking` (our scratchpad
   — adopting that would be a shadow under another name).

## ★★★★★ THE CAUSE OF THE REMAINING FAILURE — I CAUSED IT, AND MEMORY HAD THE ANSWER
**w392j adopted the guest's RING *and its USERD* on a doorbell birth. The USERD half is a measured
data-corruption hazard.** `rm_takes_a_guest_userd_and_zeroes_it` (w233, real GA106, `ad6bb9f`,
host Xid 0/0): host RM **accepts** a caller-supplied USERD through
`NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` and then **ZEROES all 512 bytes**, alloc still returning
`NV_OK`. Its own words: *"a doorbell birth is by definition AFTER the guest has written `GP_PUT`
⇒ adopt the guest's USERD at first doorbell and you destroy the cursor that caused the doorbell."*

**The trace fits exactly:** doorbell 1 rang with `GP_PUT=1` and the engine ran **nothing**;
doorbell 2 set `PUT=2` and the engine then executed entry 0 — **40 ms after the guest had already
moved that entry's source frame**. That is where all five `Xid 31` come from.
⇒ **`w392o` (`4f64dc44`) adopts the RING and passes `userd: None`. MEASURED (boot `w392o`):**
```
adopt=GUEST-RING x7    userd=NOT-ASKED x7    host Xid = 0    client (F), all rows NEVER RETIRED
```
★ **The mistimed faults are GONE** (Xid 31 x5 → 0) — the USERD adoption really was their cause.
⊘ **But there are still no completions**, and `GuestRing`'s own doc predicted exactly this:
*"it does not make the channel runnable. **Nothing in this rung writes the guest's `GP_PUT` into
our USERD, so the engine still has nothing to fetch.** Adopting the ring and advancing the cursor
are two rungs, and this is the first."* `rm.rs:1286` names the symptom: *"nobody ever advances:
`GP_PUT == GP_GET` forever, scheduled, doorbelled, and reporting no error."*
⇒ **DO NOT build a cursor bridge.** Birth-at-channel-allocation subsumes it: adopt the USERD at
CREATION (before the guest writes, so RM's zeroing is harmless) and thereafter the guest's own
`GP_PUT` writes land directly in the USERD the engine reads. No mirroring, no cursor to sync.
⚠ The memory's real conclusion is stronger: **adoption must happen at CHANNEL CREATION, never
lazily.** Ring-late is safe; cursor-late is not. Moving the birth is the proper fix.

## THE EARLIER WALL — fb-join aliasing (FIXED in `001e2e4a`, untested)
```
va=0x8080000000 : Vidmem@0x50000   va=0x80c0000000 : Vidmem@0x50000    ← ONE page, TWO VAs
va=0x8480000000 : Vidmem@0x110000  va=0x84c0000000 : Vidmem@0x110000
MMU Fault: ENGINE CE0 HUBCLIENT_CE1  FAULT_PDE ACCESS_TYPE_VIRT_READ, at FIVE VAs on a
regular 0x2_00000000 stride:  0x80_80000000  0x82_80000000  0x84_80000000  0x86_80000000
                              0x88_80000000
```
★ **The stride is the tell.** Five faults, exactly 8 GiB apart, one per client slot — this is
systematic ("the second VA of every pair is never bound"), not a sporadic race.
Four fb pages aliased at two VAs each; **both faults are the unbound half of a pair**. `ALIAS`
fired 3×, `remaps_refused=0`. `FbLeafBacking::Aliased` (w380) is built for exactly this and is not
applied to every alias of a joined leaf. Same class as the LLM wall.

## ✔ CONTROL, w392k (rev `6ae36bda`, shadow type removed)
Behaviourally **identical** to w392j: `7 × adopt=GUEST-RING`, `ADOPTABLE 7`, `5 × Xid 31`,
`MEAN_FALSIFIER=PASS`, outcome `(F)`. ⇒ deleting `ShadowsGuestMemory` perturbs nothing observable,
which is what removing an unconstructible state should do. **Bench archive compiles with
`host-isolates` at that rev (`RC=0`).**

## ⊘ FAILED EXPERIMENT (not a result)
`KAYFABE_JOIN_RELEASE=off` to isolate the release: boot died `rc=3`, *"no adapter output … The
adapter was never exercised, so this capture cannot support any claim about where the boot stops.
⊘ Do not cite it."* Matches the memory note that `off` reproduces w327's death. **Not a usable
isolation arm.** ★ Note the harness refused to let me cite it — that refusal is the feature.

## ★★★★★ LLM, MEASURED HONESTLY (w392llm5, `LLM_TIMEOUT=2700`, rev `ded5d262`)
**THE CORRUPTION REPRODUCES.** First run all night not killed mid-load:
```
W392_GPU_TOKENS=16  W392_GPU_RC=0   LLM_MS=930910.3   => ~0.017 tok/s
W392_GPU_TEXT=[ ，ize'sus(,.- A的  : ]                  <- GARBAGE
W392_CPU_TEXT=[ ______. A. Paris B. London C. New York D]
W392_MINMM_SUM=64        W392_XIDS=21/21/21  (nothing faulted)
W392_OUTCOME=(F-scale) ★★★★★ FORGED-PASS, AND IT SCALES
```
⇒ completion **without error and without correctness** — memory mapped, holding wrong contents.
★★★ **The discriminator is now SAME-BOOT and sharp:** a 4×4 matmul through this exact stack is
**bit-correct** (`MINMM_SUM=64`) while a 290-shard model is garbage ⇒ **not the arithmetic path;
it scales with the number/size of allocations.**
⊘ **0.017 tok/s** is the parity starting point. `nvkvm-pv`'s published 0.97–0.99x rows are **Mode 1**
and a different workload — not a Mode-2 baseline.
⚠ **`LLM_TIMEOUT` is in SECONDS.** `LLM_MS` is read by nothing; passing it kills the GPU arm at the
900 s default, mid-load, which is what produced every earlier bogus verdict.

## SUPERSEDED RETRACTION — "THE LLM PASSES" WAS FALSE. I COMPARED THE ORACLE TO ITSELF.
**The GPU arm never produced any text.** Its output ends mid-progress-bar and is killed:
```
LLM_DEVICE=cuda  TORCH_CUDA_AVAILABLE=True
Loading weights:  63%|######3        <- OUTPUT ENDS HERE
HOST_XID_AFTER_GPU=21
--- CPU oracle run ---
LLM_DEVICE=cpu … LLM_TEXT= ______. A. Paris B. London C. New York D   LLM_TOKENS=16  LLM_RC=0
```
The whole log has **exactly one** `LLM_TEXT` and **one** `LLM_TOKENS`, both the **CPU arm's**. The
text I quoted as the GPU's was the oracle's own output.

★★★ **CAUSE, and it is mine:** `TMO=${LLM_TIMEOUT:-900}` (`w392_llm.sh:42`). I passed **`LLM_MS`**,
which the script **never reads**, so every LLM run tonight was killed at 900 s partway through
loading 290 shards. **The variable is `LLM_TIMEOUT` and it is in SECONDS.**

⊘⊘ **AND THE GRADER WAS RIGHT.** `(E) UNMEASURED — the GPU run printed no LLM_TOKENS line at all`
was exactly correct, and `c6788ab6` changed a **working instrument** on a false premise (I blamed a
`^` anchor, then a progress bar; the line is a clean 18-char `    LLM_TOKENS=16`). The `pick()`
change is harmless but **its commit message is a lie**; `ed435c02` is the correction.
★ **NEVER repair an instrument that is reporting a result you dislike until you have read what it
actually saw.**

⚠ **Withdrawn with it:** *"the 2 GiB starved the loader, so the corruption is not reproducing"* is
**unsupported**. Still measured and standing: the 2 GiB OOM was real and 16 GiB removed it;
`MINMM_SUM=64`; Xid counts unchanged across the run.

## SUPERSEDED CLAIM (kept for the trail) — "THE LLM PASSES" (w392llm3)
```
Loading weights: 100%|##########| 290/290      the model FULLY LOADS (was dying at 135/290)
GPU:  LLM_TEXT= ______. A. Paris B. London C. New York D    LLM_TOKENS=16   LLM_OK=1
CPU:  W392_CPU_TEXT=[ ______. A. Paris B. London C. New York D]
```
**BYTE-IDENTICAL to the same-boot CPU oracle. 16 tokens. No corruption.** The campaign's standing
*"16 tokens of garbage text"* story **is not reproducing**: the 2 GiB guest starved the loader.

⊘⊘ **AND THE GRADER PRINTED `(E) UNMEASURED` OVER IT.** `pick()` was `sed -n "s/^$2=//p"` — anchored
at `^` — while the GPU arm's output arrives **indented**, so every GPU field read `ABSENT`
(`GPU_TOKENS=ABSENT`, `GPU_TEXT=[]`). The CPU arm parsed only because its lines are flush-left.
★ **A grader whose extractor is anchored more tightly than its input reports UNMEASURED for
SUCCESS.** Fixed in `scripts/bench/w392_llm.sh` (`c6788ab6`); a graded re-run is still owed.

## SUPERSEDED — LLM BASELINE ON TONIGHT'S BUILD (w392llm, rev `6ae36bda`)
```
W392_GPU_TOKENS=0   W392_GPU_RC=1        the GPU run ERRORED, generated nothing
W392_CPU_TOKENS=16  W392_CPU_RC=0        same-boot CPU oracle healthy
W392_MINMM_SUM=64                        ★ the small compute path is BIT-CORRECT
W392_XIDS=17/17/17  (before / after-4x4 / after-gpu)
W392_OUTCOME=(Z) the GPU run produced 0 tokens — it did not generate.
```
★★★★★ **CAUSE FOUND, AND IT IS NOT KAYFABE: THE GUEST HAD 2 GiB OF RAM.**
```
LLM_EXC=RuntimeError: CUDA error: out of memory     (at weight shard 135 of 290)
NVRM: Out of memory [NV_ERR_NO_MEMORY] (0x51) from _memdescAllocInternal @ mem_desc.c
NVRM: Out of memory [NV_ERR_NO_MEMORY] (0x51) from rmStatus @ system_mem.c:356
boot_nvkvm.sh:30   NVKVM_RAM_MB=${NVKVM_RAM_MB:-2048}     ⇒ -m 2048
host: 49 GiB total, 47 GiB available
```
`system_mem.c` is **system** memory — the guest's RM could not allocate **guest RAM**, not VRAM.
Qwen2-0.5B's weights alone are ~2 GB and load shard-by-shard into guest RAM before reaching the
GPU; it dies at 135/290, almost exactly halfway. ⇒ **raise `NVKVM_RAM_MB`** (it drives BOTH `-m`
and the memfd `size=`, which QEMU requires to match exactly). Re-run underway at 16384.
★★★ **CONFIRMED at 16 GiB (`w392llm2`): the OOM is GONE** — `out of memory` occurrences **0**
(was: died at weight shard 135/290). New outcome `(E) UNMEASURED` — **no `LLM_TOKENS` line at all**,
i.e. it now runs long enough to exceed the 900 s budget instead of dying. `MINMM_SUM=64` still
correct, `XIDS=17/17/17` still unchanged (nothing faults). ⇒ **next: raise `LLM_MS` to ~2700000**
(w383 measured `LLM_MS=722820` for a full run, so 900 s was never enough once loading succeeded).

⊘ **This is a harness configuration limit, NOT a kayfabe defect**, and it means the campaign's
"LLM corruption" story needs re-checking on a guest that is not starved.

★ **Two dead ends on the way, both mine, both the same class:** I read `12 × Rm(NoMemory)` and
`112 × 0x51` out of the qemu log and built a whole theory on the C's *"0x51 on a FIXED map means
already-mapped"* semantic. **Every one of those hits was the same warning TEMPLATE** — the join
refusal message literally contains *"⚠ If this is `Rm(NoMemory)` it is status 0x51…"*. The real
refusals were 12 × `SystemDataPlane` on kernel channels, correct by design. ⇒ **grep counts of a
string that appears in a caveat are not counts of an event.**
★ **`MINMM_SUM=64` means basic GPU compute through kayfabe is sound on this build.**
★★ **`17/17/17`** — seventeen Xids existed **before** the LLM started and **neither** the 4×4 nor
the LLM added one. ⇒ the LLM is **not faulting**; it fails earlier with `rc=1`. That is a different
failure from this campaign's previous *"16 tokens of garbage text"* — 0-with-an-error is honest
rather than forged, but it is **not yet diagnosed**. ⚠ Do not assume tonight's changes caused it;
no LLM run was taken on the pre-change build tonight, so there is **no same-build control**.

## OWNER RULINGS FROM TONIGHT — these overturn older docs
- **Ours vs guest is decided by KIND, not by timing.** Emulated: our fake ring, `GP_PUT`/`GP_GET`
  are fictions, work runs as **our own function bodies**; real work goes to the **scratchpad**.
  Passthrough: the guest drives its own ring, hardware writes `GP_GET`, **we never parse it**.
- **The only addressing obligation**: whatever an operation touches must be mapped in the host
  channel we actually execute on. Fake FB is **DMA-mappable**; vidmem-vs-fake-FB is an
  **optimization**, never a correctness requirement.
- **An unpublished GPFIFO page is LEGAL** (rare) ⇒ debug-only. The defect is the *silent fallback*.
- **Bare-metal pass + guest fail ⇒ kayfabe bug**, unless you can show the client broke NVIDIA's
  contract (rare, needs proof).
- **Kill `ShadowsGuestMemory`** (agreed; Fable was mid-removal — check `git status`).
- ⊘ **`guest_ring_adoption.md` §3's "birth must move to the doorbell" is REFUTED by its own §3.3**
  (sourced: RM allocates channels with `gpFifoOffset=0` on purpose; measured: R31 arm C accepted a
  never-mapped address). A host channel does **not** need its ring bound to be born.

## ⚠⚠⚠ SECOND OPEN QUESTION — A REAL SEMANTIC CHANGE FROM w392p, NEEDS A RULING
**Birth-at-alloc makes a proc "touched" at CHANNEL ALLOCATION rather than at its first doorbell.**
Measured consequence (in `l1_mean`): the **user↔user `Dup` with `LateMerge` now REFUSES** where it
was legal. ⇒ **If a real guest ever dups between user clients AFTER allocating channels (IPC), w392p
turned that from legal into a refusal.** That is a behaviour change for guest processes sharing
handles, not a test artifact — and a green suite would have hidden it.
⊘ Not fixed, not worked around. It needs a decision: is "touched at alloc" correct (and the Dup
refusal right), or must the touch stay at first doorbell?

## ⚠⚠ OPEN QUESTION FOR THE OWNER — FIRST THING IN THE MORNING
**Owner asked 2026-09-09 ~01:20 CEST:** *"we don't do publish at doorbells more right? that's
removed? including no trap to bar 1/2 or guest declared ram?"*

**Measured answer: publish at doorbells is NOT removed.**
- `ring()` (`shim.rs:5133`) still calls `decode_cpu_pt_writes()` **and** `sweep_cpu_pt_tables()` on
  every doorbell.
- `KAYFABE_VAS_PUBLISH=drain` additionally does whole-VAS publication + a guest-RAM pin drain there.
- **BAR1/BAR2 DO appear untrapped** (as the owner expected): no Rust MMIO handlers registered for
  them, and although `bar1_writes`/`bar2_writes` exist as audit fields the boot emits none. Only
  BAR0 is the trapped register plane.

⇒ **EVERY result tonight was measured under the LEGACY arming**, because that is the arming w392d
used and the only one with a known-positive (`CUP3_VAL=43`). If publication-at-doorbell is meant to
be gone, the `JOIN-RELEASE` release defect below may be an artifact of a path that should not run
at all, and the right next experiment is the client **without** `VAS_PUBLISH`/`PT_SWEEP`.
⚠ Counter-evidence not to discard: w390 measured compute **dead** without publication.

## TRAPS PAID FOR TONIGHT
- `Aperture::Vidmem` ≠ `FbLeafBacking::Vidmem` ≠ `BackingBytes::*` — three types, one word. I read
  a census's `Vidmem` (an *aperture*) as a *backing* and nearly rewrote the wrong function.
- A count over one instrument's output is **not** a census (`kind=` prints only at `forward_ring`,
  so emulated channels were invisible and I reported "0 emulated" against a real count of 2–6).
- Log **prose** is not a mechanism. Five wrong root causes tonight, every one from reading a
  message instead of the code that emitted it.
