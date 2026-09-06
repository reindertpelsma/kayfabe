# ★★★★★ THE RAW-CLIENT RUNGS ARE SERVABLE IN THE GUEST — a `LAUNCH_DMA` liveness probe

**STATUS — 2026-09-06 — LIVE.** Supersedes the scope paragraph of
`w377_the_llm_wall_is_our_own_refusal.md` (*"the guest-side version of these questions needs a
`LAUNCH_DMA` probe and is **not built**"*) — it is built, and this file is where it is
described. Mechanism sections §1–§3 are read off this tree's own source and are checkable
without a GPU. §4 carries the measurements; every number in it names the arm it came from.

---

## §0 THE ONE-LINE PROBLEM

The w379 mapping-plane battery **passed 5/5 on bare metal and could not run in a Mode-2 guest
at all** — not "failed", *could not run*. So it was a control and not an iteration handle: it
told us the host driver permits what the guest does, and nothing whatever about our emulated
path.

## §1 WHY IT COULD NOT RUN — the blocker, from the source

Every w379 rung proves a VA is live the same way:
`HostRmBackend::submit_release_at` (`kayfabe-isolate-host/src/rm.rs`) writes the five-method
run `SEM_ADDR_LO / SEM_ADDR_HI / SEM_PAYLOAD / _ / SEM_EXECUTE_RELEASE_32BIT` — the **host
FIFO** semaphore. The chip codec decodes that as `kayfabe_arch::PushMethod::SemRelease`, and
`kayfabe-rt/src/ceutils.rs` walks past it on purpose, in the `else` arm of the `CeLaunchDma`
`let`:

> *"⊘ A `SemRelease` is deliberately NOT acted on here: it is the **host** semaphore four
> bytes below the finishPayload (§14.15), and advancing it would satisfy our own counters
> while the guest spins on the word above it."*

restated in `release_targets_of`:

> *"⊘ `SemRelease` is deliberately NOT here. It is the **host-FIFO** semaphore, four bytes
> from the engine-class one … Pinning the page a host-FIFO release names would be pinning a
> different address for a different reason and calling it the same source."*

⇒ In a guest, **every rung's positive control fails, every rung prints `NOTRUN`**, and the
battery's own verdict is `(C) UNINTERPRETABLE`. That is the emulator's **declared scope**, not
a defect, and reading it as a red would have been the mistake the w379 script's own header
warns about.

## §2 ★★★ `LAUNCH_DMA` **IS** SERVED — verified from source before anything was built

The brief that opened this lane said to verify the substitution rather than assume it, and the
verification is four links, all in this tree:

1. **The doorbell reaches the CPU executor at all.** `forwarding_plane_owns_ce(kind,
   has_vas_pdb, local_ce_is_the_only_executor)` (`kayfabe-qemu-raw/src/shim.rs`) is
   `has_vas_pdb && !local_ce_is_the_only_executor && kind.hosted_by() == Shadow`. With
   `KAYFABE_CE_EXECUTOR` at its **default `local`** the second term is false, so the
   forwarding plane owns nothing and `ceutils::run_submission` serves **every** CE doorbell.
   ⚠ Under `=host` an addressable USER-proc channel goes to the forwarding plane instead —
   which is why the guest arm sets the executor explicitly.
2. **Every operand is ours to execute.** `ceutils::WalkOperands::resolve_runs` pushes
   `Representability::Fabricated` for **every** run it resolves, and
   `Representability::Fabricated ⇒ CeExecutor::Ours` (`kayfabe-fwd/src/lib.rs`). So the
   `§14.8` guard — which refuses the whole submission if any span is `HostCe` — cannot fire on
   this path.
3. **The bytes actually move.** `cpu_ce::execute_ours_spans → execute_ours` does a real
   plane-to-plane copy through `read_plane`/`write_plane`, routed by aperture:
   `CpuPlane::Fb` → the emulated framebuffer, `CpuPlane::GuestRam` → `vmm.gpa_read/gpa_write`.
   Not a no-op, not a completion without work.
4. **And only then is the completion written.** `write_resolved_completion` runs after the
   spans, so the launch's own `SET_SEMAPHORE_A/B/PAYLOAD` release is a report on work that
   happened.

⇒ The substitution *"release MAGIC at `target_va`"* → *"CE-copy MAGIC **into** `target_va`"* is
sound, and it is **strictly more evidence**: a copy that lands proves the destination VA
resolved *and* a source VA resolved *and* the engine ran; a release proves only the first and
the third.

## §3 WHAT WAS BUILT

- **`HostRmBackend::submit_copy_at`** — one four-byte `LAUNCH_DMA` from an offset inside the
  channel's **own ring object** (`W381_COPY_SRC_OFFSET = 0x4000`, a page clear of the
  pushbuffer, the GPFIFO, the semaphore and `RACE_FENCE_OFFSET`) to the VA under test. The
  ring is mapped before the doorbell **by construction**, so a red is attributable to the
  destination alone — the reason `submit_fenced_release` puts its fence there too.
  ⊘ **It does not wait.** The caller polls the destination.
- **`HostRmBackend::submit_copy_va`** — the same launch with an arbitrary **source** VA. It
  exists because half of R4's objects have no CPU view at all (§4.2), so the read-back has to
  be the engine's.
- **`W381Probe`** — the selector, threaded through all four engine rungs. Default is the w379
  primitive so every committed arm stays byte-comparable; `--probe-launch-dma` switches it,
  `--w381` selects the whole battery on it in one flag, and **`W381_PROBE=` is printed before
  any rung runs**. A battery whose primitive is not on its own log cannot be compared to the
  other half of its differential.
- **R4 `--rpc-mixed-allocs`** and **R5b `--cross-client-leak`** — the two gaps w379 named
  honestly. §4.2 and §4.3.
- **`scripts/bench/w381_guest_servable.sh`** (one arm), **`w381_hook.sh`** (the guest half,
  driven by `boot_capture.sh`), **`w381_differential.sh`** (all three arms, one table).

### ★★★ ORDER THE OBSERVABLES — the `GP_GET` lesson, applied

`USERD_GP_GET` has **no writer anywhere in this workspace** (five occurrences, all reads; the
only USERD store is `USERD_GP_PUT`), and an emulated device has no PBDMA, so in a Mode-2 guest
`gp_get == 0` is the only value that word can hold **on every configuration**. A rung that
graded on it produced a confident, plausible, always-wrong `NeverFetched`.

⇒ The probe's **primary** observable is *did the payload land at the address under test*, read
through an independent mapping. The channel's own retirement semaphore is exposed as
`w381_retired()` and may **only qualify a red** — it never turns a red green or a green red.

## §4 THE MEASUREMENTS

### §4.1 the differential table

<!-- W381_TABLE -->

### §4.2 R4 — RPC-mixed allocations

<!-- W381_R4 -->

### §4.3 R5b — cross-client leakage

<!-- W381_R5B -->

## §5 WHAT THIS STILL DOES NOT MEASURE — scope, stated up front

- ⊘ **The ladder builds its OWN `FERMI_VASPACE_A` inside the guest and probes THAT VAS.**
  Inherited verbatim from `r33_hook_ce_client.sh`, and just as true here: it says nothing
  about the VA the GR engine faults on during `cuCtxCreate`, because that channel belongs to
  the guest driver's own client with its own PDB. **A probe in the wrong address space is this
  campaign's recorded failure shape.**
- ⊘ **The guest arm measures the CPU copy-engine emulator, not the forwarding plane.** That is
  what `KAYFABE_CE_EXECUTOR=local` selects, and it is the arm the blocker is about. The `host`
  arm is a different measurement and needs its own row.
- ⊘ **`pde_info` is recorded and never graded.** `NV0080_CTRL_CMD_DMA_GET_PDE_INFO` answers
  `PDE_COVERS` for any VA inside a live page *table*, including ones the run never mapped;
  `pte_info` is `NV_ERR_TEST_ONLY_CODE_NOT_ENABLED` on every release driver. The rungs grade
  on `probe_va` and on hardware behaviour.
- ⊘ **R4 does not measure which requests cross to the emulated GSP.** It interleaves three
  request families known to take different paths inside RM and asserts the mix is inert.
  Asserting the crossing would need an instrument inside the device; claiming it without one
  would be the `pde_info` mistake in a new place.
