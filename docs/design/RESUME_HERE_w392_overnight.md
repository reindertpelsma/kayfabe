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
MMU Fault: ENGINE CE0 HUBCLIENT_CE1 faulted @ 0x80_80000000  FAULT_PDE ACCESS_TYPE_VIRT_READ
MMU Fault: ENGINE CE0 HUBCLIENT_CE1 faulted @ 0x84_80000000  FAULT_PDE ACCESS_TYPE_VIRT_READ
```
Four fb pages aliased at two VAs each; **both faults are the unbound half of a pair**. `ALIAS`
fired 3×, `remaps_refused=0`. `FbLeafBacking::Aliased` (w380) is built for exactly this and is not
applied to every alias of a joined leaf. Same class as the LLM wall.

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

## TRAPS PAID FOR TONIGHT
- `Aperture::Vidmem` ≠ `FbLeafBacking::Vidmem` ≠ `BackingBytes::*` — three types, one word. I read
  a census's `Vidmem` (an *aperture*) as a *backing* and nearly rewrote the wrong function.
- A count over one instrument's output is **not** a census (`kind=` prints only at `forward_ring`,
  so emulated channels were invisible and I reported "0 emulated" against a real count of 2–6).
- Log **prose** is not a mechanism. Five wrong root causes tonight, every one from reading a
  message instead of the code that emitted it.
