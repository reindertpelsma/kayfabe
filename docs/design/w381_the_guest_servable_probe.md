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

One binary, `md5 abb359d5859d97d46dcd97ab6742bc0e`, **printed on both arms** — so *"the same
program"* is a measurement and not an assumption. RTX 3060 (GA106), host driver 580.159.04
**open** module, guest kernel `6.8.0-138-generic`, guest driver 580.159.04, source
`d7081ad1`. Guest arm: `KAYFABE_CE_EXECUTOR=local`, `KAYFABE_ISOLATES=real`,
`KAYFABE_VAS_PUBLISH=drain`, `KAYFABE_GR_ROUTE=passthrough`.

| rung | native / launch-dma | native / sem-release | **guest / launch-dma** | guest CONTROL |
|---|---|---|---|---|
| `map_propagation` (R2) | PASS | PASS | **PASS** | PASS |
| `alias_two_vas` (R1′)  | PASS | PASS | **PASS** | PASS |
| `alias_unmap` (R1″)    | PASS | PASS | **PASS** | PASS |
| `rpc_mixed` (R4)       | PASS | PASS | **FAIL** | PASS |
| `cross_client` (R5b)   | PASS | PASS | **PASS** | PASS |
| `missing_page` (R3)    | PASS | PASS | **FAIL** | PASS |
| `map_stress` (R5)      | PASS | PASS | <!-- W381_R5GUEST --> | <!-- W381_R5GUESTCTL --> |

```
W381_TABLE_ROW arm=native probe=launch-dma  pass=7 fail=0 notrun=0 seen=7 of=7
W381_TABLE_ROW arm=native probe=sem-release pass=7 fail=0 notrun=0 seen=7 of=7
W381_TABLE_ROW arm=guest  probe=launch-dma  pass=4 fail=2 notrun=0 seen=6 of=7
```

★★★★★ **THE BLOCKER IS CLOSED.** Before this lane every one of those guest cells would have
read `NOTRUN` behind a failed control, and the battery's verdict would have been
`(C) UNINTERPRETABLE`. **Every guest control passed**, so every guest cell is an
attributable statement about the emulated path.

★ **And the substitution is validated by its own control**: the two native columns are
identical, seven for seven. A copy probe that had measured something different from the
release probe on bare metal would have made the guest column meaningless.

⊘ `map_stress` needed a second boot: at ~3.4 s per lost probe it did not fit the first run's
420 s inner deadline and the harness recorded `W381_RC=124` with **no verdict line** —
correctly reported as `seen=6 of=7` rather than as a pass or a fail.

### §4.1.1 ⚠ THE INSTRUMENT TRAP THIS RUN WALKED INTO — a capped sample read as a census

The guest boot's QEMU log carries **16 `SERVED-LOCAL` and 24 `REFUSED`** doorbell lines, and
every one of the 16 belongs to the guest driver's own kernel CeUtils channels
(`token 0x00010001/2`) while every ladder doorbell (`token 0x3/0x4`) that appears is a
**refusal**. Read literally that says *the ladder's submissions were never served* — which
would contradict four rungs that measured landed copies.

⊘ **It is a CAP, not a census.** `nvkvm.c` gates that line on
`s->doorbells_logged < NVKVM_DOORBELL_LOG_MAX`, and the dedicated refusal line has its own
separate budget. The **teardown census** is the real number:

```
doorbells=324   by engine: GrCompute=0 GrGraphics=0 Ce=324 NvEnc=0 NvDec=0 Other=0 unrouted=0
```

⇒ 324 CE doorbells arrived, all routed to CE, none unrouted; the log shows the first ~40.
★ Same class as `a_global_print_cap_forges_an_absence`, and it nearly produced the confident
wrong sentence *"the guest's copies were all refused"* in this very document. **The rungs'
own verdicts are the primary evidence; the doorbell log is a sample.**

### §4.1.2 ★★★ RED 1 — R3: the emulated device CONTAINS a fault and never NAMES it

Identical rung, identical binary, opposite halves:

| | native | guest |
|---|---|---|
| bystander before | Landed | Landed |
| victim (never-mapped VA) | Lost | Lost |
| **error notifier** | `fired=true status=0xffff except_type=0x1f engine=0x0001` | **`fired=false status=0x0000 except_type=0x0`** |
| bystander after | Landed | Landed |
| host kernel `Xid (PCI:` records | **1** | **0** |

★ **Containment holds on both arms** — a channel in another address space kept landing across
the fault. What the guest does not do is **write the robust-channel record**. `except_type
0x1f` is `ROBUST_CHANNEL_FIFO_ERROR_MMU_ERR_FLT`, the number a host kernel log prints as
`Xid 31`.

⇒ **A guest driver whose channel dies on a bad VA cannot learn that it died.** It sees a
semaphore that never advances and has no way to distinguish that from slow work. ⚠ And
`status == 0` is *also* what an unwired notifier reads as, so the rung fails this bar on
either reading — both are failures of *"named, not silent"*.

### §4.1.3 ★★★ RED 2 — R4: freeing one family broke four of six mappings in the other

```
guest:  allocated 12/12  placed exact 12/12  identity 12/12  CROSSTALK 0  never-read 0
        ordering 2/6   controls answered 12/12
native: allocated 12/12  placed exact 12/12  identity 12/12  CROSSTALK 0  never-read 0
        ordering 6/6   controls answered 12/12
```

★ **The identity half is perfect in the guest.** Twelve objects across two apertures, each
holding its own magic under an engine read-back, **zero crosstalk** — the aperture router
(`CpuPlane::Fb` vs `CpuPlane::GuestRam`) does not confuse the two number spaces.

⊘ **The ordering half is not.** After the whole sysmem family was unmapped and freed,
**VIDMEM #2, #3, #4 and #5 stopped round-tripping** while #0 and #1 kept working:

```
ordering: VIDMEM #2 at 0x0000000e11000000 stopped round-tripping after the whole SYSMEM
          family was freed (saw Some(3735880569), want 0x84102102)
… #3 at 0x0000000f11000000, #4 at 0x0000001011000000, #5 at 0x0000001111000000
```

`3735880569` = `0xDEAD0379` = the scratch **poison**, so the read-back copy did not land at
all — those VAs stopped being reachable by the engine, not merely stale. ⚠ **Two of six
survived**, so it is not an all-or-nothing channel death; something degraded partway through
the pass.

⊘ **NOT ROOT-CAUSED, and deliberately not guessed at.** One correlated observation is
recorded because the next lane will want it: the guest's own `dmesg` carries

```
NVRM: rpcRmApiAlloc_GSP: GspRmAlloc failed: … hClass=0x00000070 … status=0x00000056
NVRM: rpcRmApiFree_GSP:  GspRmFree  failed: … status=0x00000056
NVRM: nvAssertFailedNoLog: Assertion failed: (status == NV_OK) || … @ mem.c:180
```

`0x70` is `NV01_MEMORY_VIRTUAL` and `0x56` is `NV_ERR_NOT_SUPPORTED`: **our emulated GSP
refuses both the allocation and the free of the VA-space range object**, and the guest's RM
asserts on the free path. Every rung here allocates its own VA space, so this fires
throughout. ⚠ **Correlation only** — nothing here shows it is the cause of the four dead
mappings, and saying so would be the `pde_info` mistake in a new place.

### §4.1.4 ⊘ WHAT `alias_two_vas = PASS` IN THE GUEST DOES **NOT** SETTLE

The w377 correction named the LLM wall as **FB-join aliasing**: `install_join(phys, region)`
is keyed by physical frame alone, so one framebuffer frame can be host-backed at exactly one
GPU VA. R1′ passing in the guest is *not* a refutation of that, for a reason that is
structural rather than statistical: this arm ran `KAYFABE_CE_EXECUTOR=local`, so **the CPU
executor served every copy out of the emulated framebuffer and no host backing was needed for
any of these VAs**. The FB-join store was never on the path. ⇒ R1′ under
`KAYFABE_CE_EXECUTOR=host`, where the operands must be host-backed, is a **different
measurement and is the one that would bear on the w377 diagnosis.** It is not run here.

### §4.2 R4 — RPC-mixed allocations

`--rpc-mixed-allocs`. Six `NV01_MEMORY_LOCAL_USER` (vidmem) and six `NV01_MEMORY_SYSTEM`
(sysmem) objects, allocated **interleaved** with an `NV0080_CTRL_CMD_DMA_GET_PDE_INFO` after
every one, each fixed-mapped at a VA 4 GiB from its neighbours. Then a distinct magic written
into every object **by the engine**, every object read back **by the engine** into a poisoned
vidmem scratch, and finally the entire sysmem family torn down and the vidmem family
re-written.

★ **Native, RTX 3060 / 580.159.04 open module, source `d7081ad1`:**

```
info  R4 census = allocated 12/12  placed exact 12/12  identity 12/12  CROSSTALK 0
                  never-read 0  ordering 6/6  controls answered 12/12 (ungraded)
★     R4 MIX IS INERT
```

⊘ **THE FIRST VERSION OF THIS RUNG DIED, AND THE WAY IT DIED IS THE FINDING.** It wrote its
sentinel with `fill_words` — a CPU store through `NV_ESC_RM_MAP_MEMORY` — and **every one of
the six sysmem objects refused the mapping**, six identical failures and
`RUNGCTL_rpc_mixed=FAIL`. `RmBackend::alloc_sysmem` passes `NVOS02_FLAGS_MAPPING_NO_MAP`, and
`HostRmBackend::alloc_probe_local`'s own docs had already recorded exactly this, measured
2026-08-03: *"a published backing is opaque to the CPU in both directions, by design"*.
⇒ **A rung that mixes apertures cannot grade both halves through a CPU load**, and one that
quietly graded the half it *could* read would be *"every row verified"* over half the rows.
Hence `submit_copy_va` and the scratch: the readback is the engine's, works in every aperture,
and proves the engine can **read** each address under test as well as write it.

### §4.3 R5b — cross-client leakage

`--cross-client-leak`. A **second** `RmConnection` — its own `/dev/nvidiactl` fd, its own
`NV01_ROOT` — with its own VA space, its own channel, and an object mapped at **the same
64-bit GPU VA** as client A's. The shared VA number is the whole rung: if the number alone
were enough to reach memory, this is the arrangement in which it would show.

★ **Native, same box and source:**

```
info  R5b hClient A/B      = 0xc1d00054 / 0xc1d00055
info  R5b N1 B sees A's VA = Ok(Free) at 0x1c11000000 (A HAS IT MAPPED)
ok    R5b A after B        = A still holds MAGIC_A 0xc5c000a1
ok    R5b B after B        = B holds MAGIC_B 0xc5c000b1
ok    R5b foreign handle   = B mapping A's raw object 0xcafe002e: refused Other(87)
info  R5b census = controls A=true B=true  B-sees-A's-VA-free=true  leak A<-B=false
                   leak B<-A=false  ordering=true  foreign handle refused=true
★     R5b ISOLATED
```

#### ★★★ THE FOREIGN-HANDLE ARM REACHES RM — I ASSUMED IT COULD NOT

This arm shipped **ungraded** in its first version, on the reading that
`HostRmBackend::narrow` refuses a foreign `HostHandle` before any ioctl is issued, so the arm
would only measure our own bookkeeping. ⊘ **That reading was false, and the source says so in
three lines**: `narrow` is `u32::try_from(h.raw())` and nothing else — no isolate id, no table.
So A's raw handle really does travel into `NV_ESC_RM_MAP_MEMORY_DMA` on **B's** descriptor, and
RM answers **`0x57` = `NV_ERR_OBJECT_NOT_FOUND`**: the handle names live memory under A and
does not exist under B. ⇒ **RM scopes object handles per client, at the driver**, and the rung
now grades it.

⚠ The lesson is the mistake, not the result. *Suspecting* the instrument sent the arm to
`ungraded`; **reading** the instrument fixed it. `suspect_the_instrument_first` is a
disposition, not a conclusion.

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
