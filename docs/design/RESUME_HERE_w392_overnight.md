# ★★★★★ RESUME HERE — w392 overnight, 2026-09-09

**STATUS: LIVE.** Written for a compacted context. Everything below is measured unless marked.

## THE ONE-LINE STATE
The raw mean client now **adopts the guest's ring** and the host engine **executes the guest's
pushbuffer** (`Xid 0 → 5 × Xid 31`). It still fails, and the remaining wall is **FB-JOIN ALIASING**:
one framebuffer page mapped at two guest VAs, host object bound at only one.

## THE GOAL CHAIN (owner, 2026-09-09)
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

## THE CURRENT WALL — fb-join aliasing
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
