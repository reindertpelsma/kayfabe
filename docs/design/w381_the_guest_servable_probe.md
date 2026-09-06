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

One binary, **md5 printed on both arms**, so *"the same program"* is a measurement and not an
assumption. RTX 3060 (GA106), host driver 580.159.04 **open** module, guest kernel
`6.8.0-138-generic`, guest driver 580.159.04, source `6bff78df`. Guest arm:
`KAYFABE_CE_EXECUTOR=local`, `KAYFABE_ISOLATES=real`, `KAYFABE_VAS_PUBLISH=drain`,
`KAYFABE_GR_ROUTE=passthrough`.

| rung | native / launch-dma | native / sem-release | **guest / launch-dma** | guest CONTROL |
|---|---|---|---|---|
| `map_propagation` (R2) | PASS | PASS | **PASS** | PASS |
| `alias_two_vas` (R1′)  | PASS | PASS | **PASS** | PASS |
| `alias_unmap` (R1″)    | PASS | PASS | **PASS** | PASS |
| `missing_page` (R3)    | PASS | PASS | **FAIL** | PASS |
| `map_stress` (R5)      | PASS | PASS | **PASS** | PASS |
| `rpc_mixed` (R4)       | PASS | PASS | **PASS** | PASS |
| `cross_client` (R5b)   | PASS | PASS | **PASS** | PASS |

```
W381_TABLE_ROW arm=native probe=launch-dma  pass=7 fail=0 notrun=0 seen=7 of=7
W381_TABLE_ROW arm=native probe=sem-release pass=7 fail=0 notrun=0 seen=7 of=7
W381_TABLE_ROW arm=guest  probe=launch-dma  pass=6 fail=1 notrun=0 seen=7 of=7
```

Census lines, guest vs native, verbatim and identical on both arms except where marked:

```
R4  allocated 12/12  placed exact 12/12  identity 12/12  CROSSTALK 0  never-read 0
    ordering 6/6  controls answered 12/12
R5  cycles 48/48  releases 186/186  placements exact 48/48  VAs recovered 44/44
    stale reads 0
R3  notifier  native: fired=true  status=0xffff except_type=0x1f engine=0x0001
              guest:  fired=false status=0x0000 except_type=0x0   engine=0x0000   ⊘ THE RED
```

★★★★★ **THE BLOCKER IS CLOSED.** Before this lane every guest cell would have read `NOTRUN`
behind a failed control and the battery's verdict would have been `(C) UNINTERPRETABLE`.
**Every guest control passed and every rung produced a verdict**, so every guest cell is an
attributable statement about the emulated path.

★ **The substitution is validated by its own control**: the two native columns are identical,
seven for seven. A copy probe that had measured something different from the release probe on
bare metal would have made the guest column meaningless.

★★★ **And the emulated mapping plane does what the driver does on six of seven rungs, to the
digit** — 12/12 identity with zero crosstalk across two apertures, 186/186 releases over 48
interleaved alloc/map/free cycles, 44/44 VAs recovered, 0 stale reads, two RM clients isolated
at one shared VA. The single divergence is named in §4.1.2.

### §4.1.1 ★★★★★ THE FIRST GUEST RUN HAD **TWO** REDS AND ONE OF THEM WAS **MINE**

⊘ This is the part of the lane worth keeping. The first guest boot returned
`pass=4 fail=2` — `rpc_mixed` **FAIL** (ordering 2/6) and `map_stress` **FAIL**
(releases 32/186, first failure at cycle 9 slot 2) — with `identity 12/12`,
`placements exact 48/48`, `VAs recovered 44/44` and `stale reads 0` beside them. The mapping
plane was demonstrably fine and the **submissions** were not.

**The two rungs failed at the same number, and the arithmetic is exact:**

| | first submission that did not land |
|---|---|
| `map_stress` — releases per cycle are `1,2,3,4,4,4,…`, so cycles 0–8 are 30 releases; the first two of cycle 9 land and slot 2 does not | **33** |
| `rpc_mixed` — 12 identity writes + 12 identity read-backs = 24; ordering writes 25–30; ordering read-backs 31–36, of which #0 and #1 land | **33** |

`PUSHBUFFER_SLOTS = (GPFIFO_OFFSET − PUSHBUFFER_OFFSET) / PUSHBUFFER_SLOT_BYTES = 0x1000/128
= **32**`.

⇒ `HostRmBackend::submit_entry` used **one** index for two different things: it wrote the
GPFIFO entry at that index *and* set `GP_PUT = (index + 1) % entries`. The index came from
`next_slot`, taken modulo `PUSHBUFFER_SLOTS` (32), while `entries` is **64**. So `GP_PUT` only
ever took the values `1..=32`, and on the 33rd submission it went **backwards**, `32 → 1`.

⊘⊘ **AND THE NATIVE ARM PASSED BOTH, 7/7, ON THE SAME BINARY.** Real hardware walks the
GPFIFO from `GP_GET` to `GP_PUT` and treats the never-written zero entries in between as
empty, so a backward `PUT` costs it a burst of no-ops and nothing else. Our emulated path
stops at the first entry its codec cannot decode and refuses **by name** —
`FwdFault::RingBroughtNoEntry`, which dominates that boot's logged refusals. **The wall was
the probe's and hardware was masking it.**

⚠ `submit_entry`'s own comment had already called it: *"Latent rather than live at this rung —
nothing here submits 64 times — which is exactly the kind of arithmetic that is wrong for a
year and then wrong at scale."* The w381 rungs are the first callers in this tree to submit
more than 32 times on one channel.

★★★ **THIS IS WHY THE DIFFERENTIAL IS THE DELIVERABLE.** *"A guest-only red cannot distinguish
'we are broken' from 'the probe is wrong'"* stopped being a slogan on the first run: two of the
three reds were the probe, and only the native column could say so. The fix is `RingSlot { pb,
gp }` — two indices with two moduli, `GP_PUT` from the GPFIFO one — and it is **byte-identical
for the first 32 submissions on any channel**, so every previously committed arm is unchanged.
Re-measured after it: native 7/7 on both probes, guest 6/7 with the table above.

### §4.1.2 ★★★ THE ONE SURVIVING RED — R3: the guest CONTAINS a fault and never NAMES it

Identical rung, identical binary, opposite halves:

| | native | guest |
|---|---|---|
| bystander before | Landed | Landed |
| victim (never-mapped VA) | Lost | Lost |
| **error notifier** | `fired=true status=0xffff except_type=0x1f engine=0x0001` | **`fired=false status=0x0000 except_type=0x0`** |
| bystander after | Landed | Landed |
| host kernel `Xid (PCI:` records | **1** | **0** |

★ **Containment holds on both arms** — a channel in another address space kept landing across
the fault. What the emulated device does not do is **write the robust-channel record**.
`except_type 0x1f` is `ROBUST_CHANNEL_FIFO_ERROR_MMU_ERR_FLT`, the number a host kernel log
prints as `Xid 31`.

⇒ **A guest driver whose channel dies on a bad VA cannot learn that it died.** It sees a
semaphore that never advances, and it has no way to tell that from slow work. ⚠ `status == 0`
is *also* what an unwired notifier reads as, so the rung fails this bar on either reading —
both are failures of *"named, not silent"*.

### §4.1.3 ⚠ AN INSTRUMENT TRAP THIS LANE WALKED INTO — a capped sample read as a census

The guest boot's QEMU log carries **16 `SERVED-LOCAL`** doorbell lines, every one of them the
guest driver's own kernel CeUtils channels, while every ladder doorbell that appears in the
same list is a **refusal**. Read literally that says *the ladder's submissions were never
served* — which would contradict rungs that measured landed copies.

⊘ **It is a CAP, not a census.** `nvkvm.c` gates that line on
`s->doorbells_logged < NVKVM_DOORBELL_LOG_MAX` (**16**), and the dedicated refusal line has its
own separate budget. The **teardown census** is the real number:

```
doorbells=324   by engine: GrCompute=0 GrGraphics=0 Ce=324 NvEnc=0 NvDec=0 Other=0 unrouted=0
```

★ Same class as `a_global_print_cap_forges_an_absence`, and it nearly produced the confident
wrong sentence *"the guest's copies were all refused"* in this very document. **The rungs' own
verdicts are the primary evidence; the doorbell log is a sample of the first sixteen.**

### §4.1.4 ⊘ WHAT `alias_two_vas = PASS` IN THE GUEST DOES **NOT** SETTLE

The w377 correction named the LLM wall as **FB-join aliasing**: `install_join(phys, region)` is
keyed by physical frame alone, so one framebuffer frame can be host-backed at exactly one GPU
VA. R1′ passing in the guest is *not* a refutation of that, for a structural reason: this arm
ran `KAYFABE_CE_EXECUTOR=local`, so **the CPU executor served every copy out of the emulated
framebuffer and no host backing was needed for any of these VAs**. The FB-join store was never
on the path. ⇒ R1′ under `KAYFABE_CE_EXECUTOR=host`, where the operands must be host-backed, is
a **different measurement, and it is the one that bears on the w377 diagnosis.** It is not run
here.

### §4.1.5 ⊘ A CORRELATED OBSERVATION, RECORDED AND NOT ROOT-CAUSED

Every guest boot's `dmesg` carries, throughout:

```
NVRM: rpcRmApiAlloc_GSP: GspRmAlloc failed: … hClass=0x00000070 … status=0x00000056
NVRM: rpcRmApiFree_GSP:  GspRmFree  failed: … status=0x00000056
NVRM: nvAssertFailedNoLog: Assertion failed: (status == NV_OK) || … @ mem.c:180
```

`0x70` is `NV01_MEMORY_VIRTUAL` and `0x56` is `NV_ERR_NOT_SUPPORTED`: **our emulated GSP refuses
both the allocation and the free of the VA-space range object**, and the guest's RM asserts on
the free path. Every rung here allocates its own VA space, so it fires throughout — and every
rung still passes, so it is **not** blocking anything these rungs measure. ⚠ Recorded because
the next lane will want it, and labelled as an observation rather than a cause.

### §4.2 R4 — RPC-mixed allocations

`--rpc-mixed-allocs`. Six `NV01_MEMORY_LOCAL_USER` (vidmem) and six `NV01_MEMORY_SYSTEM`
(sysmem) objects, allocated **interleaved** with an `NV0080_CTRL_CMD_DMA_GET_PDE_INFO` after
every one, each fixed-mapped at a VA 4 GiB from its neighbours. Then a distinct magic written
into every object **by the engine**, every object read back **by the engine** into a poisoned
vidmem scratch, and finally the entire sysmem family torn down and the vidmem family
re-written.

★ **Native AND in the guest, byte-identical census, RTX 3060 / 580.159.04 open module,
source `6bff78df`:**

```
info  R4 census = allocated 12/12  placed exact 12/12  identity 12/12  CROSSTALK 0
                  never-read 0  ordering 6/6  controls answered 12/12 (ungraded)
★     R4 MIX IS INERT
```

⚠ The guest's *first* run of this rung read `ordering 2/6`, and that red was the probe's own
`GP_PUT` arithmetic — §4.1.1. The line above is the measurement after the fix.

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

★ **Native, same box and source; the guest arm passes this rung too:**

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
