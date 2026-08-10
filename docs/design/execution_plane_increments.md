# The execution plane — the increments from here to a guest CE copy on the host GPU

> **Status: E0-E6 BUILT, 2026-08-03.** Written at master `cf3aae9`; increment **E0** built
> and measured at `e10a6bf` on the RTX 3060 bench, **E3** at `6e4f66f`, **E0b + E1** at
> `853a311` (§6), **E2** at `5c1f501` (§7), **E4/E5** at `ee3a8c3` (§8-§9) and **E6** at
> `147c069` (§10). Every row is `[src]` at its revision, `[measured]` with a named run and
> revision, or explicitly `[assumed]`.
> ⚠ **E5 was PARTIAL** (§9.2: the CE-page-table-write source reached a root page only);
> **E8 closed it** (§12) — the decode's learned page-table pages are now published into the
> device-global ownership index in a fourth, rank-0 phase, so a guest CE write into a *leaf*
> table is witnessed and its leaf binds. ⊘ Suite only — no boot has been spent on E8, and the
> guest boot still stops at `mem_utils.c:2006` on `0xa06f0103`, which is the execution plane.
> ⊘ ★★★ **THAT SENTENCE IS FALSE AND WAS FALSE WHEN WRITTEN.** `[measured]` boot `m1`,
> 2026-08-07, master `809b040`, RTX 3060/GA106 vast `47029542`
> (`docs/reference/bench_evidence/m1_809b040_dmesg.log`): the guest stops at
> **`mem_utils.c:2022`, `_memmgrMemUtilsScrubInitRegisterCallback: event notification control
> failed`** — exactly where `ff5278d` put it on **2026-08-03**. `0xa06f0103` has been SERVED
> since `3ab1305`, and `0xa06f0104` appears **0 times** in all four captured logs: the guest
> never issues it, because `bUseVasForCeCopy` is false.
> ⇒ **Every E9 increment designed after 08-03 was designed against a wall already cleared.**
> Read §13.7 before acting on anything else in this section.
> ★★★ **§10.5 arm B remains a wall**: the real isolate plane aborts QEMU under R1, at the
> base revision as well as at this one.

## 0. Why this document exists now, and what it replaces

`docs/reference/bench_rebuild_notes.md` §5 enumerated eight gaps between the bench and a
first forwarded operation, and ordered them with this judgement:

> ★★ **Row 8 is the one that orders the work.** Rows 1-7 are a day of wiring; there is no
> point paying for them until the emulated boot reaches a doorbell, because until then
> there is no guest intent to forward.

**That ordering is now wrong, and the boot is what refuted it.** `[measured]` 2026-08-01 at
rev `e10a6bf` on the RTX 3060 box — `docs/reference/bench_evidence/e10a6bf_run_e0real2_dmesg.log`,
reproduced identically in the negative-control run, so this is not an artefact of the
change this document ships:

```
NVRM: _memmgrMemUtilsScrubInitScheduleChannel: Unable to schedule channel, status: 56
NVRM: … NV_ERR_GENERIC … from _memmgrMemUtilsScrubInitScheduleChannel(…) @ mem_utils.c:2006
NVRM: … from memmgrMemUtilsChannelSchedulingSetup(…)                    @ mem_utils_gm107.c:1027
NVRM: nvAssertFailedNoLog: Assertion failed: status == NV_OK            @ ce_utils.c:304
NVRM: … from objCreate(&pScrubber->pCeUtils, …)                         @ mem_scrub.c:181
NVRM: … from memmgrScrubHandlePostSchedulingEnable_HAL(…)               @ mem_mgr.c:487
NVRM: nvAssertFailedNoLog: Assertion failed: 0                          @ kernel_fifo.c:3129
NVRM: RmInitNvDevice: *** Cannot load state into the device
```

The boot has climbed past `RmInitAdapter`'s early classes and now dies constructing
`CeUtils` for the **scrubber** — one `NV_ERR_NOT_SUPPORTED` on a channel *schedule*
(`0x56`) propagating to the fatal `kernel_fifo.c:3129` assert. ★ That single line is why
the sweep's amputation rule does not apply here: `kernel_fifo.c:3126-3131` is fatal for
**any** non-`NV_OK`. Four separate walls before it —
`0xa06f0103` (schedule), `0xa06f0104` (bind), `0xc36f0108` (token) and the index-35 event
arming — were diagnosed at `66230a1` as **one requirement asked four times**: put a channel
on a runlist, arm its completion, hand back its doorbell.

So there **is** guest intent to forward, the guest is asking for it four different ways, and
the escape hatches are closed by citation — `!IS_SILICON` is unreachable
(`bIsSimulation`/`bIsFmodel`/`bIsRtlsim` are declared in `g_gpu_nvoc.h` and assigned
nowhere; `PDB_PROP_GPU_EMULATION` is never set), and `kernel_fifo.c:3126-3131` is fatal for
**any** non-`NV_OK` in this phase, so the pre-init sweep's amputation rule
(`preinit_sweep_loop.md`) does not apply. ⊘ The C artifact's per-control synthesis ladder
does not rescue us either: that ladder is for *incomplete captures*, and the C reached this
same point and needed a **real CE**.

⇒ **Rows 1–7 are now the critical path.** This document is their order.

## 1. The target, stated as a predicate that can fail

**Done** = the guest driver's own CE copy is executed by the host GPU, and the guest reads
back what the host GPU wrote.

The acceptance predicate already exists and has already passed on this hardware: it is
`kayfabe_isolate_host::rm::CeEvidence::copied()` (`crates/kayfabe-isolate-host/src/rm.rs:1529`),
the R17 rung's predicate — *before ≠ expected, after == expected, last word == expected,
and the engine's release semaphore landed*, with the read-back done through an
**independent** second CPU mapping. `[measured]` `docs/reference/bench_evidence/rm-ladder-419afe8.out`,
RTX 3060 GA106, host 580.159.04 open, rev `419afe8`:

```
★     R15 SEM LANDED      = sem 0xbeef5ea1 … the GPU consumed our ring and released our semaphore
★     R17 CE COPY         = 4096 bytes … read back through an INDEPENDENT mapping
```

★★ **The whole plan is therefore "move the driver of that predicate from the ladder to the
guest".** Nothing below invents a new acceptance instrument, and E6's acceptance is
literally R17's, re-driven. `only_live_boots_are_proof` and
`isolate_the_drivers_own_checks` both point the same way: reuse the harness that has
already been seen to pass *and* to fail.

## 2. The increments

Each row names an acceptance test **that could fail**, and — separately — the *control*
that makes a green mean something. A row whose acceptance has no control is marked so.

| # | increment | acceptance that could fail | control |
|---|---|---|---|
| **E0** ✅ | the crates join: `kayfabe-qemu-raw` can name `kayfabe-isolate-host`, and a runtime selector chooses the isolate plane. Realizing the device materializes a **real sandboxed child** that completes RM bring-up on the host GPU | a live boot with `KAYFABE_ISOLATES=real` shows a `kayfabe-isolate` child of QEMU holding `/dev/nvidiactl` + `/dev/nvidia0` + an RM-served `/dev/nvidia0` mapping | the **same binary**, variable unset → no child, no fds. **`[measured]` §3.5** |
| **E0b** ✅ | ★ the spawn becomes **lazy**, so the first *guest* `GSP_RM_ALLOC` is what materializes the isolate — the increment E0's own measurement created (§3.6) | the child's first sighting is **after** the device-open phase line, not 28 s before it | variable unset → still zero children. **`[measured]` §6** |
| **E1** ✅ | a **failed** real isolate stops being indistinguishable from the stillborn one (bench gap 7) | a refusal reported at the `Isolate` seam with a **kind** (`spawn-failed` ≠ `no-plane`) and a sentence, printed by the device at teardown | the same archive with a working plane reports `0 refusing`. **`[measured]` §6** |
| **E2** ✅ | the doorbell reaches the core: a guest MMIO write to the usermode doorbell aperture arrives at `kayfabe_rt::SharedDevice::doorbell` | a boot in which a guest doorbell write produces a `DoorbellOutcome`-or-named-`FwdFault`, counted | a non-doorbell BAR write in the same run produces neither. **`[measured]` §7** |
| **E3** ✅ | ★ **`Ga10xArch::decode_doorbell` is built** and validated against real silicon | a token RM itself hands a channel decodes to that channel's own vChid, on hardware | a token from a *different* channel must decode to a different vChid — and a fabricated token must decode to `None`. **DONE — `doorbell_token_encoding.md`** |
| **E4** ✅ | GA10x `UserdModel` + `PushbufferAbi` replace `UnbuiltUserd`/`UnbuiltPushbuffer` | `read_pushbuffer` over bytes NVIDIA's own macros encode yields `LAUNCH_DMA`/`SEM_EXECUTE` at the offsets the guest wrote them | garbage bytes must **fault**, not decode to a plausible method. **DONE — §7, and it REFUTED the seam: `CeLaunchDma` is not decodable per-method at all** |
| **E5** ✅ | the address table is populated from the guest's own bindings, so the CE operands resolve in the isolate's host VAS | a guest VA that *was* bound resolves; the copy's operands are found | ★ a VA that was **never bound** must FAULT (`mode2_address_table.md`: miss = fault, never a reverse-resolve) — source 1 whole at §9; **source 2 closed by E8, §12** |
| **E8** ✅ | ★ the phase-shape change §9.2 refused to build blind: the decode's learned page-table pages are PUBLISHED into the device-global ownership index, at rank 0, after every rank-1 guard is released | a guest CE write into a **leaf** page table is classified, witnessed, and its leaf binds — the second write in a two-write sequence | ⊘ the **identical ring parsed before the publish** must yield zero page-table writes, or the acceptance is vacuous. **§12** |
| **E6** ✅ | the join: guest CE copy → `plan_ce` → `Worker::execute` → `HostRmBackend::ce_copy_outcome` | `CeEvidence::copied()` — R17's predicate | the same archive with `KAYFABE_ISOLATES` unset must **not** produce it. **`[measured]` §10** — ★ and the control found a HANG |
| **E7** ✅ | ★★★ the R1 fix §10.5 names: the isolate spawn moves **out** of the device lock — decide under rank 0, spawn with zero locks, install with R5 re-validation | a live boot with `KAYFABE_ISOLATES=real` + `host-isolates` that does **not** abort, and reaches a driver wall instead of a `SIGABRT` | the same archive with the variable unset must behave exactly as it does today. **`[measured]` §11** |

### 2.1 ★★★ The riskiest increment is E3, and not for the reason it looks

The obvious candidate is E6 (most moving parts) or E5 (most design). Both are wrong.

**E3 — the doorbell token decode — is the riskiest, because it is the only increment whose
wrong answer is silent, and because both of the project's standing oracles are blind to
it.**

- A wrong decode does not fail; it **routes a guest's ring to another channel**. On the
  Mode-2 path we are the GSP, so there is no second party to notice. `ga10x.rs:29` already
  says exactly this about the seams it refuses: *"a plausible answer on any of them is
  worse than a refusal"*, and *"a wrong answer is a silent memory-safety fact about the
  guest"*.
- **The mock cannot catch it.** `MockArch::token_for` (`kayfabe-mocks/src/lib.rs:137`) is
  the *inverse of the mock's own decode* — an invented encoding round-tripping against
  itself. `never_let_a_test_use_the_thing_under_test_as_its_own_observer` names this exact
  shape, and the project has already had a planted mutation survive it.
- **The C artifact cannot catch it either.** `c_rust_trace_differential.md` records that
  the **completion plane has NO C oracle** — the C *forges* completions — and that
  forwarding-mode traces are non-hermetic by construction (`pci_dma_map` is an
  uninstrumented channel). A green differential across a doorbell says nothing about where
  the ring went.
- The one instrument that *can* settle it is the one
  `isolate_the_drivers_own_checks.md` prescribes: build the expected token out of **RM's
  own encoder** (`ogkm-580` `kfifo`'s work-submit-token construction) and, separately, ask a
  **real GA106** for the token it hands a channel we allocated — `HostRmBackend::alloc_channel`
  already returns `(HostHandle, u64)` where the `u64` *is* that token
  (`crates/kayfabe-isolate/src/lib.rs:447`), and `rm-ladder` R16 has printed one.
  ⚠ `ogkm_is_versioned`: the vendored tree is 610.43.02 and the bench runs 580.159.04, and
  they already disagree about the GSP queue. Cite the 580 tag.

★★★ **BUILT, 2026-08-01 — `docs/design/doorbell_token_encoding.md`.** Both instruments
above were built. What they settled, and what they did not, in one line each:

- `tests/oracle/worksubmit_token_oracle.c` compiles
  `kfifoGenerateWorkSubmitTokenHal_GA100` (HAL binding derived from
  `g_kernel_fifo_nvoc.c`) and sweeps it: `VECTOR` is **11:0**, `RUNLIST_ID` is **22:16**,
  the widest token RM can emit is `0x007f_0fff`. **This is what settled the encoding.**
- `kayfabe-rm-ladder --doorbell-census` (rung `R13c`,
  `docs/reference/bench_evidence/doorbell-census-ba74151.out`, RTX 3060 / GA106 /
  580.159.04) pairs six real tokens with the chids RM's own `CHID_MGR` reports — an
  instrument the token cannot leak into. It confirmed the low field and **could not**
  reach the runlist ids: `GET_DEVICE_INFO_TABLE` is `KERNEL_PRIVILEGED` and answered
  `InsufficientPermissions` to root. The refusal is recorded rather than worked around.
- ⊘ The paragraph above this one, which said a wrong decode has *"no second party to
  notice"*, is still true of the **product path** — it is now false of the **test path**,
  and `doorbell_token_encoding.md` §5 tabulates exactly which wrongnesses are loud and
  which three are still silent (the VF/SR-IOV rewrite, a stale-but-live channel, and every
  other generation).

⚠ Second-place risk, named because it is *cheap to underestimate*: **E5**, for a reason
that is not correctness but shape. `[measured]` on the C artifact, 2026-07-22, audit S3 —
recorded as the ★ CORRECTION in `C: docs/design/mode2_address_table.md` §5 and carried in
memory `read_at_invalidate_is_false_on_compute_path`: on the Mode-2 GSP-emulated compute
path **both** invalidate transports counted **zero** (`INVALIDATE_TLB` RPC fn=200 = 0;
`MEM_OP`/`MMU_TLB_INVALIDATE` pushbuffer method = 0), as did `DMA_FILL_PTE_MEM`. So the
table's second populate source is the **observed CE page-table write**, latched at the CE
release semaphore. E5 therefore cannot be built as "watch for invalidates"; it has to
witness a CE write, and that is a dependency on E4 that the row above does not show.

### 2.2 What is deliberately NOT on this list

- **Multi-process / #14.** `mode2_multiprocess_isolate.md` is deferred, and E0–E6 all run
  one guest process.
- **Interrupt delivery.** `[measured]` `IrqRaise == 1` across the whole of C `cap1` with
  **zero** `IRQSCLR` writes; the boot's completion signalling is a separate wall with its
  own document.
- **Removing `StillbornIsolates` as the default.** It stays the default through E6. The
  selector is how a bench opts in; the shipped archive does not.

## 3. E0 — BUILT. What it is, and what it is not

### 3.1 The change

Three files, no `unsafe`, no ABI change:

1. `crates/kayfabe-qemu-raw/Cargo.toml` — `kayfabe-isolate-host` as an **optional**
   dependency behind a new `host-isolates` feature, **off by default**. The feature governs
   *linkage*; it changes no runtime behaviour on its own. (Why a feature and not a plain
   dependency: the isolate crate's `build.rs` embeds a musl-linked binary, so an
   unconditional dependency would put `rustup target add` in the build path of the
   hypervisor archive — including for the aarch64 cross-check job that has no musl std.)
2. `crates/kayfabe-qemu-raw/src/shim.rs` — `IsolatePlane` {`Stillborn`, `Loopback`,
   `Real`}, selected by `KAYFABE_ISOLATES` (`shim::ISOLATE_PLANE_ENV`), **defaulting to
   `Stillborn`**, with `isolate_plane_from` (pure) and `isolate_factory` (builds it).
3. `crates/kayfabe-qemu-raw/tests/shim_logic.rs` — the selector's tests.

★★ **The feature and the selector are deliberately two different things.** One archive,
built with `host-isolates`, can run *both* arms of a negative control, differing in nothing
but an environment variable. If the feature were the switch, the evidence run and its
control would be two different binaries and the control would prove much less.

⊘ **No fallback anywhere.** An unrecognised `KAYFABE_ISOLATES` value is a **refusal to
realize the device**, not a quiet `Stillborn`; a host plane asked of an archive built
without the feature is likewise a named refusal. A selector that degraded silently would
make a misspelled evidence run and its own control indistinguishable, which is precisely
the failure `suspect_the_instrument_first` catalogues.

### 3.2 ⚠ How a device-path action issues a real host RM verb

> ★★★ **CORRECTED BY THE BOOT, 2026-08-01 — read §3.5 first.** This section was written
> before the measurement and its heading used to read *"Why a guest `GSP_RM_ALLOC` is
> enough to issue a real host RM verb"*. **Step 3 below is right about the call graph and
> wrong about the trigger**: `Gpu::realize` already installs the system proc's isolate, so
> the spawn happens when QEMU realizes the device and a guest alloc finds it already
> there. The chain is kept, because every step of it is what actually runs; only the
> claimed cause was wrong.

`[src]` at `cf3aae9`:

1. The guest's GSP command queue write reaches `RegPlane`, which the shim built with the
   object-model link (`shim.rs:1076`, `object_policy`).
2. `kayfabe_rmrpc::ObjectPolicy` turns `GSP_RM_ALLOC` into a `kayfabe_core::rmgraph::RmEvent`
   and calls `Gpu::apply`.
3. ⚠ **This is the step that was wrong.** `Gpu::realize` itself calls
   `ensure_proc_target(&mut system, GpuId::ZERO)`, so the system proc's isolate exists
   before any guest traffic; `Gpu::apply`'s step 3b then installs each live proc's
   per-`(Proc, GpuId)` isolate —
   `crates/kayfabe-core/src/gpu.rs:2324` and `:2344` (the **system** proc, which is the
   guest kernel's own objects), and `ensure_proc_target` at `:1292` — by calling
   `IsolateFactory::spawn`. Both sites are `or_insert_with`, so for a guest whose clients
   all land on the system proc they are **no-ops**.
4. With `KAYFABE_ISOLATES=real`, that factory is `HostIsolateFactory`, whose `spawn` runs
   `build_isolate` (`crates/kayfabe-isolate-host/src/isolate.rs:919`): `clone` into new
   user/pid/net/ipc/uts/mount namespaces, `execveat` a sealed memfd, then **block on a
   per-worker hello handshake**.
5. Inside the child, `build_backends` (`crates/kayfabe-isolate-host/src/child.rs:212`)
   runs `sandbox::enter` and then `RmConnection::open` (`crates/kayfabe-isolate-host/src/rm.rs:640`),
   which is rungs **R0–R6b**: `openat(nvidiactl)`, `openat(nvidia0)`,
   `NV_ESC_CHECK_VERSION_STR`, `NV_ESC_REGISTER_FD`, `NV01_ROOT_CLIENT`, `NV01_DEVICE_0`,
   `NV20_SUBDEVICE_0`, then the usermode window.

★★ **It fails CLOSED, and that is what makes the evidence readable.** `RmConnection::open`
is a `?`-chain of `rung(...)`; any failed RM ioctl aborts it, `build_backends` propagates,
the hello frame carries `Reply::Failed`, and `build_isolate` returns `Err` ⇒
`HostIsolate::stillborn` ⇒ **the child is not alive**. So a *live* `kayfabe-isolate --rm
real` child, parented to QEMU, holding `/dev/nvidiactl` and `/dev/nvidia0`, is a
**sufficient** witness that real host RM allocations succeeded. There is no arrangement in
which the child survives without them.

### 3.3 ⚠ Hazards E0 introduces, named here rather than discovered later

- **`IsolateFactory::spawn` blocks, on a vCPU thread.** `spawn` waits for one hello frame
  per worker (4). It is called from inside `Gpu::apply`, i.e. from the vCPU servicing a GSP
  RPC. `[assumed]` this is a few milliseconds and the guest tolerates it; `[measured]` in
  §3.5 by the boot completing. R1 is not violated — no ranked lock is held and no ioctl is
  issued in the parent — but the *latency* is new, and a guest that times out its RPC would
  present as a boot regression with no obvious cause. E1 should make spawn lazy or
  asynchronous.
- **The selector is process-global**, so two `nvkvm-gpu` devices in one hypervisor would
  share a plane. Correct for the bench, wrong for a product; E1 moves it to a QOM property.
- **One child per `(Proc, GpuId)`, each with 4 worker threads and its own RM client**, and
  `[measured]` (`host_execution_plane.md` §2.2, rev `419afe8`) RM's alloc/free lock is
  **device-wide**, so those workers buy blast-radius containment and **not** throughput. A
  design that budgets N× from N workers is budgeting against a measurement that says 1×.
- **`HostIsolate::Drop` kills the child**, and `IsolateBox::drop` asserts R1
  lock-freedom. A proc reaped under a lock would now panic a hypervisor where before it
  dropped a no-op.

### 3.4 What E0's tests cover — and, first, what they do not

⊘ The unit tests in `tests/shim_logic.rs` drive the **pure** half (`isolate_plane_from`,
`isolate_factory`). They never read the environment variable (a test that mutated a
process-global would race its own binary) and they never spawn a `real` isolate (there is
no GPU on a CI runner). **The env-read arm and the spawn are covered only by the live-boot
pair in §3.5.** If that pair is absent from this document, treat the seam as untested where
it matters.

What they do cover, and what each would catch:

- the default is `Stillborn` — catches a default that moved;
- every plane round-trips, quantified over `IsolatePlane::ALL` and with a distinctness
  check, so the loop cannot pass vacuously (`gates_quantified_over_a_list`);
- ten near-miss values (`""`, `"Real"`, `"real "`, `"1"`, …) refuse, and the refusal text
  must **name every plane in `ALL`** — so a fourth plane added without a message update
  turns this red;
- the stillborn factory is asserted **at the seam** (`spawn` → `is_retired`, `pool_size ==
  0`, `checkout() == None`), not by its type name;
- with the feature off, both host planes are a **named** refusal mentioning
  `host-isolates`; with it on, both **build**.

### 3.5 E0 — the evidence, and ★★★ the claim it FALSIFIED

See §5 for the transcripts. The headline, stated before the good news:

⊘ **E0 did NOT reach "a real host RM verb caused by a guest action". It reached "caused by
the device path", which is weaker, and §3.2 above — written before the boot — asserted the
stronger thing.** The boot refuted it, and the refutation is the most useful thing in this
document.

`[measured]` 2026-08-01, RTX 3060 GA106, host 580.159.04 open, archive rev `e10a6bf`:

| run | `KAYFABE_ISOLATES` | distinct isolate children | first sighting |
|---|---|---|---|
| `e0ctl2` | *unset* | **0** | — |
| `e0real2` | `real` | **1** | **t+3 s** |
| `e0real3` | `real` | **1** | **t+3 s** |

The guest reached a login prompt at **t+27–32 s** and the device was opened (the act that
runs `RmInitAdapter` and issues every `GSP_RM_ALLOC` in the run) at **t+30–34 s**. The
child appeared **~28 seconds earlier**, and its argv says `--proc 0` — `Gpu::SYSTEM_PROC`.

⇒ **The spawn is `Gpu::realize`'s** (`crates/kayfabe-core/src/gpu.rs`, `realize` calls
`ensure_proc_target(&mut system, GpuId::ZERO)` unconditionally), i.e. it happens when QEMU
realizes the `nvkvm-gpu` device, **before the guest exists**. And the guest's own
`GSP_RM_ALLOC`s cannot spawn a second one: the guest kernel's clients land on the *system*
proc, whose isolate `or_insert_with` already made. `[measured]` at rev `e10a6bf`, runs
`e0real2` and `e0real3`: **1** distinct child across the whole run, in two independent
runs (`docs/reference/bench_evidence/e10a6bf_run_e0real2_isolate.log`,
`d6caffa_run_e0real3_isolate.log`).

★★ **The instrument nearly hid this.** The first version of the witness latched only the
*first* sighting and reported "★ AN ISOLATE CHILD EXISTED", which reads as the strong
claim. Dumping *every* pid **with a `t+` stamp against `boot_capture`'s own phase lines**
is what turned an encouraging sentence into a falsification. `suspect_the_instrument_first`.

#### What IS established, exactly

**A real host RM verb chain is issued as a consequence of the device path**, in the shipped
archive, with no mock anywhere:

```
cmdline: kayfabe-isolate --proc 0 --gpu 0 --workers 4 --rm real --park none
ppid   : <the qemu-system-x86_64 running the guest>
  fd  9 -> /dev/nvidiactl
  fd 10 -> /dev/nvidia0
  fd 12 -> /dev/nvidia0
  746f08922000-746f08932000 rw-s 00000000 00:05 491   /dev/nvidia0
CapEff/CapPrm/CapBnd 0000000000000000   NoNewPrivs: 1   Seccomp: 0
user/pid/net/mnt/ipc/uts namespaces all distinct from the host's
```

The **64 KiB shared mapping of `/dev/nvidia0`** is the load-bearing line. It is the
usermode BAR0 window, and RM only serves that `NV_ESC_RM_MAP_MEMORY` against an object
reached through a live client → device → subdevice chain. A capability-less process in six
fresh namespaces cannot fabricate it. Together with §3.2's fail-closed argument, the
mapping means rungs **R0–R6b really ran and really succeeded on the host GPU**:
`NV_ESC_CHECK_VERSION_STR`, `NV_ESC_REGISTER_FD`, `NV01_ROOT_CLIENT`, `NV01_DEVICE_0`,
`NV20_SUBDEVICE_0`, usermode map.

The negative control is the same binary with the variable unset: **zero** children, zero
descriptors, and a `dmesg` that stops at the identical line
(`RmInitAdapter failed! (0x25:0xffff:1249)`) — so the selector added a host process and
changed nothing the guest could see, which is exactly what E0 should do.

★ The `plane→RmMode` mapping is asserted **on hardware** by the witness itself
(`--rm <plane>` in the child's argv, plus the two RM nodes and the mapping for `real`). No
Rust test can see it — `isolate_factory` returns a `Box<dyn IsolateFactory>` and the
`RmMode` inside is not observable — so this is the only instrument that covers it, and
`scripts/bite_isolate_selector.py` says so rather than planting a bite that cannot fire.

#### ⊘ What this does NOT establish

1. **Not caused by the guest.** See above. The bar "a real host verb because the guest did
   something" is **not met**; it needs **E0b** below.
2. **No forwarding.** No `VerbPlan` was executed, no doorbell rung, no pushbuffer parsed,
   no `ce_copy`. The verbs witnessed are the isolate's own bring-up.
3. **Nothing about the boot.** The wall is unmoved and identical in both arms. E0 buys
   capability, not progress.
4. **Nothing about latency or concurrency under load.** One isolate, one spawn, one boot.
5. **The env-read arm is covered only by these boots.** The unit tests drive the pure
   decision function and deliberately never touch the process-global.

### 3.6 E0b — the increment E0's own boot created

**Make the isolate's spawn LAZY, so the first guest `GSP_RM_ALLOC` is what materializes
it.** This is a named increment rather than a detail because of one number: `[measured]`
at rev `e10a6bf`, runs `e0real2`/`e0real3`, the isolate child's first sighting is
**t+3 s** and the device open is **t+30–34 s**, so nothing the guest does can carry the
"caused by the guest" claim while the spawn is eager.

- **Acceptance that could fail:** a boot with `KAYFABE_ISOLATES=real` in which the isolate
  child's first sighting is **after** `boot_capture`'s "opening the device" line, not
  before it. The instrument already exists and already discriminates — it is what produced
  the table above.
- **Control:** the same boot with the variable unset still shows zero children.
- **Why it is worth doing rather than redefining the bar:** it also removes the
  §3.3 hazard, because a spawn that happens at the *first guest alloc* is a spawn that
  never happens at all for a guest that never allocates — which is every guest that fails
  earlier, and every device that is realized and never used.
- ⚠ It is not free: `IsolateFactory::spawn` is infallible by signature and `Gpu::realize`
  currently uses that to make step 3b infallible-by-construction (`gpu.rs` §12.18). Making
  materialization lazy must not turn an infallible installation into a fallible one on a
  path with no error channel. Read that comment before touching it.

### 3.7 E0b — BUILT. The change, and the one thing it deliberately did NOT move

`ensure_proc_target` was **split in two**, and the split is the whole increment:

- **`Spine::ensure_proc_arena`** — `ensure_target` + the GPA carve. This is what
  `Gpu::realize` calls now. The arena is address-space bookkeeping; it is the thing realize
  can legitimately fail on, and carving it early keeps `GpuError::Gpa` a **realize-time**
  refusal instead of a surprise on the guest's first RPC. Realize's failure mode is
  therefore unchanged.
- **`Spine::materialize_isolate`** — idempotent, **infallible**, and the only site in the
  device that calls `IsolateFactory::spawn`. `Spine::apply` calls it for the system proc on
  its **`Ok` arm**; `Spine::refresh` step 3b calls it for each live proc, which it always
  did. The §3.6 hazard is answered by construction rather than by care: there is no new
  error channel because `spawn` still has none, and a factory that cannot spawn hands back a
  **refusing** isolate — which is exactly what E1 makes readable.

⊘ **On the `Ok` arm only, and that is a decision.** `Spine::apply` is transactional: a
refused event moves nothing. It must not be the one thing that buys a guest a host process
either. The cost is stated rather than discovered: a boot whose every event is refused now
shows **zero** children — and that is a *distinguishable* outcome, not a silent one, because
`isolates: 0 materialized` is printed (§3.8).

★★ **Multi-process is untouched, and this is the seam the owner's 2026-08-01 ruling
(*"yes no rewrite for multi process"*) is about.** The *system* proc is the guest **kernel**'s
objects and is singular by construction (`Gpu::SYSTEM_PROC`, `SYSTEM_ANCHOR`, a reserved
client `RmGraph::apply` refuses as guest input) — it is not "the guest", and E0b did not
collapse anything into it. Every guest **process** still gets its own `(Proc, GpuId)` isolate
through step 3b, keyed on the projection's `ProcAnchor`, i.e. the **anchor client** — one of
the three keys `proc_is_not_a_set_of_rm_clients` measured as correct, and never a raw pid
(two concurrent CUDA processes share one dup-DST client, so a pid key aliases them). The
property is quantified over in
`tests/tests/isolate_spawn_is_guest_caused.rs::two_guest_processes_get_their_own_isolates_and_none_exists_at_realize`:
two guest processes ⇒ **three** distinct isolate sessions, **zero** of them at realize.

⊘ **What the bench could NOT show, stated because it is the honest half.** The boot stops at
`RmInitAdapter`, long before a second guest process exists, so every hardware arm below has
exactly **one** `Proc` — the system one. On hardware the multi-process property is therefore
**`[unmeasured]`**, and it is recorded as unmeasured rather than borrowed from the suite:
what carries it is the test named above (green at `853a311`, and bitten — `BL9` collapses
the per-process id onto `SYSTEM_PROC` and it goes red) plus `[src]` the anchor-client key in
`Spine::plan_refresh`. The first arm that could measure it is E6 driven by two guest
processes, and nothing here has been run against one.

### 3.8 E1 — BUILT. What became visible, and where

Three layers, each with its own reason:

1. **`Isolate::refusal(&self) -> Option<IsolateRefusal>`** — a **required** trait method
   (no default, so a new implementor cannot forget it), carrying a `RefusalKind` **and** a
   sentence. `StillbornIsolate` answers `NoPlane`; `HostIsolate` answers `SpawnFailed` iff
   its `spawn_error` is set; `MockIsolate` answers `None`, deliberately.
   ★ **A kind and not only a string**, because a check keyed on a word is satisfied by
   writing the word — and because `"this build has no forwarding plane"` and `"clone
   failed"` are *different diagnoses*, only one of which means the host is wrong.
2. **`Gpu::isolate_census()` → `IsolateCensus`** — `materialized` / `live` / `no_plane` /
   `spawn_failed` / one sentence, with `SpawnFailed` outranking `NoPlane` for the single
   line a report has room for. Published through `SharedIsolateCensus`, the same
   clonable-handle shape `SharedRefusalCensus` already uses and for the same reason: the
   policy that owns the `Gpu` is boxed into the chain and unreachable afterwards.
3. **The wire ABI (bumped to 11) and the device's teardown report** — five fields and one
   `info_report`, printed **unconditionally, all-zeros included**, for the reason every other
   block in `nvkvm_report_registers` is.

⊘ `HostIsolate::spawn_error`'s own docs already said *"a composition root should check it at
realize"*. No composition root could: the core holds `dyn Isolate` and that accessor is on
the concrete type. E1 is that sentence made reachable — and E0b removes the "at realize"
half of it, because after E0b there is nothing to check at realize.

## 4. Order of work, and the one thing that should be re-decided

E0 → E0b → E1 → **E3** → E2 → E4 → E5 → E6. **E0, E0b, E1, E3, E2 and E4 are done**; E5 is next.

★ E3 is pulled ahead of E2 deliberately. E2 (an ABI entry point for the doorbell) is
mechanical and can be written at any time; E3 is the increment most likely to be *wrong for
months without anything going red*, so it should be paid for while there is still a
hardware bench to settle it on, and while `kayfabe-rm-ladder` — which can allocate a
channel and print the token RM assigned it — is a live, passing instrument.
`c_rust_trace_differential.md` already records that the C oracle is **perishable**.

⚠ **To re-decide before E4** *(the question, as it stood)*: whether the pushbuffer codec is
transcribed from `ogkm` or generated from it. The GMMU format went the transcription route
at `#149` and `955c79a` then had to build "a GMMU-format oracle out of NVIDIA's OWN encoder"
to catch five bites transcription missed. That is evidence for generating, and it is a
bigger decision than a row in a table.

★★ **DECIDED, 2026-08-02, and the answer is neither — it is *transcribed and then judged by
a compiled oracle*, which is what `#149` actually did.** The reasoning, so the next
generation's codec does not re-litigate it:

- A *generator* over `clc56f.h`/`clc7b5.h` would have to parse `hi:lo` field extents out of
  `#define`s and emit Rust. That is a second implementation of `DRF_*`, written by us, and
  a bug in **it** produces confidently wrong constants across every field at once — which
  is worse than a typo in one.
- The oracle route gets the same guarantee for less: the constants stay hand-written and
  cited, and `tests/oracle/pushbuffer_abi_oracle.c` **compiles NVIDIA's own macros over
  NVIDIA's own header** and both encodes and decodes with them. A wrong constant is a red
  test naming the field.
- `[measured]` it works: `scripts/bite_pushbuffer_codec.py` plants 26 defects and the oracle
  arm alone catches 23 of them, including every wrong bit position. See §7.
- ⊘ The honest cost, and it is real: **the oracle is only as available as the vendored
  driver checkout**, so on a runner without one this whole family SKIPs. That is the same
  bargain the VBIOS, GMMU and token oracles already struck, recorded again rather than
  rediscovered.

## 5. Evidence log

### E0, 2026-08-01 — RTX 3060 GA106, host driver 580.159.04 **open**, vast `46494693`

- **Archive under test:** `kayfabe-rev:e10a6bf48084facf0a99540c6989e1d4d7961412`, read out
  of the QEMU binary with `strings`, and recorded in the head of every witness log.
  Built `KAYFABE_SHIM_FEATURES=host-isolates scripts/build_qom_shim.sh`.
- **Host driver:** stock DKMS, verified before the runs — banner
  `NVRM: loading NVIDIA UNIX Open Kernel Module for x86_64 580.159.04 Release Build`, and
  **zero** `nvkvm`/`kayfabe` symbols in `/proc/kallsyms`.
- **Committed transcripts** (`docs/reference/bench_evidence/`):
  | file | what it is |
  |---|---|
  | `e10a6bf_run_e0ctl2_isolate.log` | ★ the **negative control** — variable unset, `0` children |
  | `e10a6bf_run_e0real2_isolate.log` | `KAYFABE_ISOLATES=real` — 1 child at t+3 s, fds + mapping |
  | `d6caffa_run_e0real3_isolate.log` | the repeat, with the `plane→RmMode` assertion active (`ok`) |
  | `e10a6bf_run_e0real2_qemu.log` | the device's own report for the `real` run: 92 commands decoded, 20 unserviced |
  | `e10a6bf_run_e0real2_dmesg.log` | the guest driver's ring buffer — the wall, identical to the control's |
- **Harness:** `scripts/bench/e0_isolate_witness.sh <tag> <plane>`, which wraps
  `scripts/bench/boot_capture.sh` and samples the host at 2 Hz. It **exits non-zero** if
  the plane and the spawned child's `--rm` argument disagree, if `real` produced no
  RM-served `/dev/nvidia0` mapping, or if a refusing plane spawned anything.
- **Suite, default arm** (`host-isolates` OFF, the shipped configuration), `[measured]` at
  `d6caffa` on the 38-core box: `cargo test --workspace --no-fail-fast`, **1835 passed, 0
  failed**, `KVM-GATE: RAN` markers **56** with `KAYFABE_NO_KVM` unset.
- **Suite, feature-on arm**, `[measured]` at `1041050` on the RTX 3060 box (the only box
  with the musl target the isolate image needs):
  `cargo test -p kayfabe-qemu-raw --features host-isolates --test shim_logic` →
  **41 passed, 0 failed**, including `with_the_feature_both_host_planes_build_a_factory`.
  ⊘ This arm is **not** in CI: nothing enables the feature there, so the linkage half of E0
  is covered by this one command and by the bench build, not by a gate.
- **Bites:** `scripts/bite_isolate_selector.py` — **5/5 fired**, run twice: at `d6caffa`
  and again against the final committed content at `fd4d467` (`BS1` default
  moves to `Real`; `BS2` unknown value degrades silently; `BS3` case-insensitive parse;
  `BS4` host plane degrades instead of refusing without the feature; `BS5` refusal stops
  naming the planes). Restored-tree sanity check GREEN.



## 6. Evidence log — E0b and E1

### 2026-08-01 — RTX 3060 GA106, host driver 580.159.04 **open** (stock DKMS), vast `46494693`

- **Archive under test:** `kayfabe-rev:853a311695e6c46613deb7b4be2a2f6eaaf70520`, read out of
  the QEMU binary with `strings` and recorded in the head of every witness log; built
  `KAYFABE_SHIM_FEATURES=host-isolates scripts/build_qom_shim.sh`. `REV_UNDER_TEST` in each
  log is `85c4513` — the witness script gained its settled re-dump between the build and the
  runs, and each log lists the differing file (`scripts/bench/e0_isolate_witness.sh`, nothing
  under `crates/`) so a reader can judge instead of assuming.
- **Host driver, verified before the runs:** banner
  `NVRM: loading NVIDIA UNIX Open Kernel Module for x86_64 580.159.04 Release Build`,
  `dkms status` → `nvidia/580.159.04 … installed`, and **zero** `nvkvm`/`kayfabe` symbols in
  `/proc/kallsyms` **and** in `nvidia.ko` itself.

#### ★★★ E0b — the bar, and the artifact that could have shown otherwise

| run | `KAYFABE_ISOLATES` | distinct children | **first sighting** | **device open** | verdict |
|---|---|---|---|---|---|
| `e0breal2` | `real` | 1 | **t+33 s** | t+31 s | ★ spawn **follows** |
| `e0breal3` | `real` | 1 | **t+33 s** | t+30 s | ★ spawn **follows** |
| `e0bctl2` | *unset* | **0** | — | t+30 s | control holds |
| `e0bfail1` | `real`, `user.max_user_namespaces=0` | **0** | — | t+30 s | ⊘ **E0b check RED** |

Compare E0 at rev `e10a6bf`: child **t+3 s**, device open **t+30–34 s**. The ordering has
inverted, on the same bench, with the same harness.

★★ **The check is an assertion now, not a table for a human to compare.**
`e0_isolate_witness.sh` computes the minimum over every sighting's `t+` stamp and the `t+`
stamp of `boot_capture.sh`'s own *"opening the device"* phase line, and exits non-zero if the
first is smaller — or if either is missing, which is an **instrument** failure and is worded
as one. ⊘ Neither number is written by the device, the archive or the core: the sightings
come from scanning host `/proc` at 2 Hz, the phase line from `boot_capture`'s stdout stamped
by the wrapper.

⊘ **The artifact that could have shown otherwise, and did.** `e0bfail1` is the same
instrument on a boot where no isolate could spawn, and its verdict line reads
`★ FAILED: no isolate child was ever sighted, so the lazy spawn cannot be distinguished from
a spawn that never happens (t_open=30s)`. A check that could not produce that sentence would
not be a check. `e0breal1` is kept for the same reason from the other direction: its
`plane→RmMode` arm went **red** because the 2 Hz sampler latched the child **mid-bring-up**
(fd 9 = `/dev/nvidiactl`, no `/dev/nvidia0`, no mapping, `Threads: 1`) while the device's own
report said the isolate was live and not refusing. ★ **The instrument was wrong, not the
run** — `suspect_the_instrument_first` — and the fix is the SETTLED re-dump, which leaves the
first-sighting stamp (E0b's whole content) untouched.

#### ★★★ E1 — the same archive, three arms, three different sentences

The device's own teardown line, verbatim, from `run_<tag>_qemu.log`:

```
e0breal2  nvkvm: isolates: 1 materialized, 1 live, 0 refusing (0 no-plane, 0 spawn-failed)
e0bctl2   nvkvm: isolates: 1 materialized, 1 live, 1 refusing (1 no-plane, 0 spawn-failed)
          nvkvm:   isolate refusal [no-plane] this build has no forwarding plane: the object
                   model accepts protocol facts and no host verb can be issued
e0bfail1  nvkvm: isolates: 1 materialized, 1 live, 1 refusing (0 no-plane, 1 spawn-failed)
          nvkvm:   isolate refusal [spawn-failed] spawning the embedded isolate: clone failed
                   (errno 28)
```

★★ **`e0bfail1` is the increment.** From **outside** the process it is indistinguishable from
a healthy plane-less boot: `boot_capture rc=0`, zero isolate children, the identical
`RmInitAdapter failed! (0x25:0xffff:1249)` wall. That is precisely gap 7 — before E1 a real
host failure *was* "nothing happened". The fault was injected at the **host**, not in the
code: `sudo sysctl -w user.max_user_namespaces=0` before the run (restored to `201823`
after), so `clone(CLONE_NEWUSER)` fails and `build_isolate` returns the host's own error.
Nothing about the archive differs between the three rows.

⊘ **The control is the first row**, and it is the one that stops this being a reporter that
always reports something: a working plane prints `0 refusing` and **no** refusal line at all.

⊘ **What the census does NOT establish.** It is written by the code under test, so
`1 materialized` says *whether* a spawn happened and never *why* — the attribution is the
`/proc` timeline's alone. And `0 refusing` is a statement about the isolate's **bring-up**
(rungs R0–R6b), not about any forwarded work: no `VerbPlan` ran, no doorbell was rung.

#### Files

`docs/reference/bench_evidence/`, all prefixed `853a311_`:

| file | what it is |
|---|---|
| `run_e0breal2_isolate.log` | ★ the E0b acceptance — first sighting t+33 s vs device open t+31 s, settled dump with `/dev/nvidiactl`, `/dev/nvidia0` and the 64 KiB `rw-s` mapping |
| `run_e0breal3_isolate.log` | the independent repeat |
| `run_e0bctl2_isolate.log` | ★ the **negative control** — variable unset, 0 children, and the `no-plane` line |
| `run_e0bfail1_isolate.log` | ★★ the **E1 arm** — a real host spawn failure, invisible from outside and named by the device |
| `run_e0breal1_isolate.log` | ⊘ the instrument failure, kept: the first sighting caught the child mid-bring-up |
| `run_e0breal2_qemu.log`, `run_e0bctl2_qemu.log`, `run_e0bfail1_qemu.log` | the device's own reports, the three isolate lines above |
| `run_e0breal2_dmesg.log` | the guest driver's ring buffer — the wall, unmoved and identical in all four arms |

#### Suite, gates, bites, ledger

- **Suite**, `[measured]` at `853a311` on the **RTX 3060 box** (the KVM bench):
  `cargo test --workspace --no-fail-fast` with `KAYFABE_NO_KVM` **unset** → **1851 passed,
  0 failed**, `KVM-GATE: RAN` markers **56**, `SANDBOX-GATE: RAN` **10**.
- **Gates**, `[measured]` at `853a311` on the 38-core box:
  `./scripts/ci_gates.sh --all` → `ALL GATES CLEAN (21 steps, floor 21 for --all mode)`.
- **Claim ledger**: `scripts/claim_ledger.py --gate` → 382 unattributed / 66 conflated /
  17 bare-hardware, i.e. the baseline, unmoved.

⚠ **A measurement hazard this increment hit, recorded because it produced a confidently
wrong suite run.** The bite harness rewrites two source files and `touch`es them
(`bite_harness_must_touch_after_restore`); syncing a working tree onto that box afterwards
with `rsync -a` puts the LOCAL — **older** — mtimes back, and cargo's freshness check then
serves the rlib it built from the *bitten* sources. The result was a `--workspace` run
reporting **two failures in files that pass in isolation**, with byte-identical sources on
both ends (`md5sum` agreed). ⇒ **`find … -exec touch {} +` after any sync onto a box a bite
harness has run on**, and treat "passes alone, fails in the suite" as an mtime question
before it is a code question.
- **Bites:** `scripts/bite_lazy_isolate.py` — **9/9 fired**, restored-tree sanity GREEN.
  ⚠ Reported honestly: the **first** run of this harness fired only **6/9**, and none of the
  three misses was a code finding — two `--exact` filters were missing their
  `isolate::tests::` module path (*"TEST DID NOT RUN — filter matched nothing"*) and one
  planted replacement did not compile. All three would have read as *"the guard is not
  needed"* to a careless reader; the harness reports them as their own outcomes precisely so
  they cannot.
- ★ **A red test that was a real finding, and the pin moved rather than the bar.**
  `gsp_rm_alloc.rs::the_ports_object_model_realizes_with_no_forwarding_plane` asserted
  `gpu.system.isolates.contains_key(&GpuId::ZERO)` — it was **pinning the defect**. It is now
  `…_and_no_isolate`, asserting the negation plus the arena that realize legitimately still
  carves, and a second test (`the_first_guest_alloc_materializes_the_ports_isolate`) was added
  so the negation cannot be satisfied by a plane that never spawns at all.

#### ⊘ What E0b and E1 do NOT establish

1. **No forwarding.** No `VerbPlan` executed, no doorbell rung, no pushbuffer parsed, no
   `ce_copy`. The verbs witnessed are still the isolate's own bring-up (R0–R6b).
2. **Nothing about the boot.** The wall is unmoved: `RmInitAdapter failed!
   (0x25:0xffff:1249)`, identical in all four arms. E0b buys attribution, E1 buys
   visibility; neither buys progress.
3. **Nothing about multi-process on hardware.** Every bench arm has exactly one `Proc` (the
   system one) because the boot never reaches a second guest process. The per-process claim
   is carried by the suite and by the projection's anchor-client key — see §3.7.
4. **Nothing about latency or concurrency under load.** One isolate, one spawn, one boot.
   The spawn now happens on a vCPU thread servicing a guest RPC *during* `RmInitAdapter`
   rather than at realize, so its blocking hello handshake is on a guest-visible path for the
   first time. `[measured]` only that three boots completed; no timing was taken.
5. **`isolates_materialized` is not an attribution instrument** and must never be cited as
   one — see §3.7 and the field's own docs.

## 7. Evidence log — E2

### 2026-08-01/02 — RTX 3060 GA106, host driver 580.159.04 **open** (stock DKMS), vast `46494693`

- **Archive under test:** `kayfabe-rev:5c1f501d003d121034154731bd6c9ed692565894`, read out of
  the QEMU binary with `strings` and recorded in the head of every `*_e2.log`. Built
  `KAYFABE_SHIM_FEATURES=host-isolates scripts/build_qom_shim.sh`. ⊘ The stamp carries **no**
  `-dirty` suffix and the sha is the branch tip, so the binary is this content at this
  revision — the one thing `CLAUDE.md`'s standing warning is about ("the bench silently
  served a binary built from `862c7c2` for weeks").
  ⊘ **`5c1f501` is a CODE-ONLY commit and this section is not in it**, deliberately: the
  measurement has to bind to the content that was measured, so the evidence and this log
  land in the commits *after* it. `git diff --name-only 5c1f501 HEAD` is `docs/…` plus
  exactly one source file — `crates/kayfabe-qemu-raw/tests/shim_logic.rs`, a **test target
  that is not linked into the archive** (the wire-size mirror; the full suite caught it and
  §"Suite" below says so). Nothing the archive contains differs. ★ Two earlier revisions (`80fabd7`, `ec6feed`)
  produced the identical three-arm result and were discarded rather than cited: the first
  had a stray build directory in the commit, and the second was voided by a `cargo fmt`
  run **after** the boot. A verification binds to CONTENT at a REVISION, and an edit
  between measuring and claiming voids it however cosmetic it is.
- **Host driver, verified in the same session:** banner `NVRM: loading NVIDIA UNIX Open
  Kernel Module for x86_64 580.159.04 Release Build`, `dkms status` →
  `nvidia/580.159.04 … installed`, and **zero** `nvkvm`/`kayfabe` symbols in
  `/proc/kallsyms`.
- **Harness:** `scripts/bench/e2_doorbell_witness.sh <tag>`, which builds
  `scripts/bench/e2_doorbell_poke.c` statically, boots through `boot_capture.sh` (new
  `POST_CAPTURE_HOOK` phase, so the experiment runs on a **live** guest and before the
  poweroff that flushes the device's report), stages the tool into the guest, and issues
  **two** guest MMIO writes through **one** `mmap` of `resource0`, four bytes apart.

#### ★★★ The three arms

`[measured]` at `5c1f501`, and reproduced identically at two discarded predecessors — so the
result does not rest on one boot of one binary.

| run | what the guest wrote | `arrived` | `served` | `refused` | verdict |
|---|---|---|---|---|---|
| `e2run1` | `+0xbb0094` then `+0xbb0090` | **1** | 0 | **1** | ★ acceptance holds |
| `e2run2` | the same, independently | **1** | 0 | **1** | ★ the repeat |
| `e2ctl1` | `+0xbb0094` **only** | **0** | 0 | 0 | ⊘ **the harness goes RED** |

The device's own line, verbatim from `run_e2run1_qemu.log`:

```
2026-08-01T22:28:33.011648Z … nvkvm: DOORBELL token 0x00070005 at +0xbb0090 REFUSED [FwdFault::UnknownVchid]
2026-08-01T22:28:42.714021Z … nvkvm: doorbells: 1 arrived, 0 served, 1 REFUSED by name; last token 0x00070005 (1 logged)
2026-08-01T22:28:42.714058Z … nvkvm:   first doorbell refusal [FwdFault::UnknownVchid] UnknownVchid { gpu: GpuId(0), vchid: VChid(5) }
```

★ `VChid(5)` is the payload, and it is the **decode** (E3's encoding: `VECTOR` 11:0 of
`0x0007_0005`) reached through a real `route_doorbell` against a real spine. No wiring that
dropped the write, and no port that was never installed, can produce that sentence: an
unwired plane answers `Device::NoDoorbellPort`, from a different crate, and the harness
fails by name on it.

#### ★★★ Attribution — and why the counters alone would not have been enough

`a_boolean_witness_cannot_attribute`. The device writes `doorbells: 1 arrived`, so it can
say *whether* and never *why*. Three stamps, from three writers, settle it (`run_e2run1`):

| stamp | written by |
|---|---|
| control window opened `22:28:28.890223Z` | the harness, on the host |
| doorbell window `22:28:32.457621Z .. 22:28:33.023650Z` | the harness, on the host |
| **arrival `22:28:33.011648Z`** | QEMU, `-msg timestamp=on` |

The arrival falls **inside** the doorbell window and **outside** the control's, and the
harness additionally asserts that **zero** arrivals are stamped before the first window
opened (`e2_doorbell_witness.sh`, the `NBEFORE` check). That second half is the one that
matters: it excludes *"something in the boot rang one"*, and it settles a fact this document
had until now only read out of a source tree —

> ⊘ **The guest driver rings no doorbell at this wall.**
> `[measured]` 2026-08-01 at rev `5c1f501`, runs `e2run1`, `e2run2` and `e2ctl1` on the
> RTX 3060 bench —
> `docs/reference/bench_evidence/5c1f501_run_e2ctl1_qemu.log` reports
> `nvkvm: doorbells: 0 arrived` across the whole of boot + `modprobe` + the device open, and
> the two acceptance runs report exactly the **one** arrival their own poke made, stamped
> inside their own window.
> `[src]` for the mechanism, and only for the mechanism: `kfifoRingChannelDoorBell_HAL` is
> reached from `ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/channel_utils.c:557`, after the
> channel *schedule* that this bench's `run_e2run1_dmesg.log` shows failing `0x56`.

⇒ therefore the ring in these runs is issued **deliberately, by guest userspace**, through
the same physical offset in the same BAR the driver's own `GPU_VREG_WR32` uses — the 64 KiB
usermode window, reached through sysfs instead of through RM. The device cannot tell the two
apart, which is the point: there is one classification and both rings arrive at it.

#### ★★ The control, and why it is more than "the counter stayed at zero"

Two writes, one `mmap`, one process, one instruction shape, **the same value**, four bytes
apart — differing in the **offset** and in nothing else. The control is issued **first**, so
a device that counted any BAR write would already be non-zero before the doorbell. Exactly
one arrival was counted, in both acceptance runs.

⊘ And `e2ctl1` is the artifact that could have shown otherwise: the identical boot with the
doorbell store suppressed (`E2_SKIP_DOORBELL=1`) — same control write, same driver load,
same wall — produces `0 arrived, 0 served, 0 REFUSED` and the harness's own verdict line
reads:

```
★ FAILED (acceptance): expected EXACTLY ONE arrival (the poke), got 0.
```

A check that could not produce that sentence would not be a check.

#### Files

`docs/reference/bench_evidence/`, all prefixed `5c1f501_`:

| file | what it is |
|---|---|
| `run_e2run1_e2.log` | ★ the acceptance verdict, with the three attribution stamps |
| `run_e2run2_e2.log` | the independent repeat |
| `run_e2ctl1_e2.log` | ⊘ **the negative control** — the harness going RED on a suppressed poke |
| `run_e2run1_qemu.log`, `run_e2run2_qemu.log`, `run_e2ctl1_qemu.log` | the device's own reports, the lines quoted above |
| `run_e2run1_probe.log` etc. | the hook's transcript: the guest's `E2POKE before/after` stamps, `CONTROL_RC=0`, `DOORBELL_RC=0` |
| `run_e2run1_dmesg.log` etc. | the guest driver's ring buffer — the wall, unmoved and identical in all three arms |

#### ⊘ What E2 does NOT establish

1. **No forwarding, still.** `served == 0` in every arm, and it must be: serving a doorbell
   needs a channel on the spine, which needs a guest that allocated one. E2 buys the
   transport and the refusal vocabulary. `UnknownVchid` is the **expected** answer here and
   becomes a bug only after E5.
2. **Nothing about the boot.** The wall is unmoved: `RmInitAdapter failed!
   (0x25:0xffff:1249)`, identical in all three arms and identical to E0b's.
3. **The ring is guest *userspace*, not the guest *driver*.** Stated above, measured above,
   and it is the honest half: E2's claim is about the transport, not about RM's intent.
4. **The two consumers' shared object model is `[src]`, not measured.** The doorbell port
   and the object bridge are two `Arc::clone`s of one `SharedDevice`
   (`shim.rs`, `Regs::create`), and the behavioural witness — declare a channel through the
   bridge, ring its vChid through the doorbell — is an **E6** assertion, because nothing in
   this port can inject an `RmEvent` chain. What guards it meanwhile is
   `crates/kayfabe-qemu-raw/tests/e2_doorbell.rs::the_archive_realizes_exactly_one_object_model`,
   which quantifies over the composition root's own source: **one** `Gpu::new`, **one**
   `SharedDevice::new`, both consumers built from a clone of that one handle. ★ It is the
   defect nothing else would catch — a second `Gpu` leaves `UnknownVchid` as the permanent
   answer with every test still green — and `scripts/bite_e2_doorbell.py` **B10** plants it.
5. **Nothing about concurrency.** One ring per boot. The port is called with no plane lock
   held (`RegPlane::write` classifies the doorbell *before* taking the FSM mutex, because
   `SharedDevice::doorbell` can block on the isolate pool's gate) — that is `[src]` and a
   documented requirement, not a measurement.

#### Suite, gates, bites

- **Suite**, `[measured]` at `d8a5a6e` on the **RTX 3060 box** (the KVM bench):
  `cargo test --workspace --no-fail-fast` with `KAYFABE_NO_KVM` **unset** → **1884 passed,
  0 failed, 1 ignored**, `KVM-GATE: RAN` markers **56**, `SANDBOX-GATE: RAN` **10**.
  ★ The run **before** the fix was `1883 passed, 1 failed` — `shim_logic.rs`'s wire-size
  mirror, which `cargo check --workspace --all-targets` compiles and never runs. It is
  recorded here rather than quietly fixed: it is the one gate that covers a C-layout change
  this ABI's runtime `struct_size` handshake does not reach.
- **Gates**, `[measured]` at `d8a5a6e` on the RTX 3060 box:
  `./scripts/ci_gates.sh --all` → `ALL GATES CLEAN (21 steps, floor 21 for --all mode)`.
- **Claim ledger**: `scripts/claim_ledger.py --gate` → **382 unattributed / 66 conflated /
  17 bare-hardware**, i.e. the baseline, unmoved. ⊘ It went **red at 384** on the first
  draft of this section and was fixed by **attributing** the two sites — naming the runs and
  the evidence file — never by raising the ceiling.
- **Bites:** `scripts/bite_e2_doorbell.py` — **10/10 caught**, restored-tree sanity GREEN,
  and the *split* is the finding: B3 (token not masked) and B8 (the default port becomes a
  silent sink) are caught by the **device** arm only; B9 (the root never installs the port)
  and B10 (the doorbell rings a second object model) by the **shim** arm only. A harness with
  one arm would have missed four of ten.

## 8. E4 — BUILT. The USERD model, the pushbuffer codec, and the seam it refuted

### 8.1 What was built

`kayfabe_chips::Ga10xArch` now answers `userd()` with **`Ga10xUserd`** and `pushbuffer()`
with **`Ga10xPushbuffer`**. `UnbuiltUserd` / `UnbuiltPushbuffer` remain in the crate — as
`UnbuiltGmmu` did after `#149` — because the statement they make is one an adapter may still
need to make; nothing in the shipped `Arch` uses them.

Every bit position lives in `kayfabe_abi::submit`, cited to `ogkm-580`, per decision #2's
quarantine: `USERD_SIZE`, `gp_entry_decode`, `method_header_decode`, the `sec_op` module and
the `fifo`/`ce` field constants are new there; `kayfabe-chips` maps them into the core's
vocabulary and transcribes nothing.

`[src]` at `e43bc71`+E4:

| seam | answer | refusal it keeps |
|---|---|---|
| `userd_size` | **512** | — |
| `gp_get_offset` / `gp_put_offset` | `0x88` / `0x8C` | both asserted **inside** the window |
| `method_len` | the header's own count, per `SEC_OP` | `0` for the two encodings `NVC56F_DMA_SEC_OP` enumerates **without a format** (`RESERVED6`, `GRP2_USE_TERT` with an unenumerated `TERT_OP`) |
| `decode_method` | `SetObject`, `SemRelease`, `TlbInvalidate` | `Opaque` for everything else, including **every** CE method |
| `gpfifo_entries` | one `PushRange` per entry | **nothing at all** for a ring that is not whole entries; **nothing** for a control entry (`LENGTH == 0`) |

⚠ **`MAX_PUSH_RANGE_BYTES` / `MAX_PUSH_TOTAL_BYTES` are untouched.** E4 changed what produces
a range's length, not what bounds the read of it; `a_maximal_gpfifo_length_is_a_bounded_read_and_not_an_allocation`
pins that a maximal entry (8 MiB − 4, the largest the 21-bit field holds) is still a bounded
read.

### 8.2 ★★★ The result that matters, and it is a REFUTATION

**A real `AMPERE_DMA_COPY_B` copy is FIVE separate method runs, and `LAUNCH_DMA` carries
none of its operands.** `OFFSET_IN_UPPER…OFFSET_OUT_LOWER` are one earlier run,
`LINE_LENGTH_IN`/`LINE_COUNT` another, `SET_SEMAPHORE_A/B/PAYLOAD` a third; `LAUNCH_DMA`
itself is a header and **one word of flags**. `[src]` `kayfabe_isolate_host::rm::ce_pushbuffer`,
which is the encoder a real GA106 executed at rung R17, and `[measured]` from the class
header itself by `tests/tests/pushbuffer_abi_oracle.rs::the_ce_pushbuffer_is_five_runs_and_launch_dma_carries_no_operands`.

`PushbufferAbi::decode_method(&self, header, args)` is **per-method and stateless**. It is
therefore *structurally incapable* of producing a `PushMethod::CeLaunchDma` whose `dst`,
`src` and `len` are anything but invented.

⊘ **`MockArch` hid this for the whole life of the seam.** `mock_method::CE_LAUNCH_DMA` packs
destination, source, length and work kind into **one** method's arguments — an encoding no
NVIDIA chip has. The seam looked sufficient for exactly as long as its only implementer was
the mock. This is the same shape as §2.1's finding about `MockArch::token_for`, one seam
along, and it is the third time a mock's invented encoding has made a seam look finished.

**So E4 refuses:** `LAUNCH_DMA` decodes to `PushMethod::Opaque`. That is the posture this
module's own header demands (*"a plausible answer on any of them is worse than a refusal"*),
and it is stated here rather than buried because it is a **dependency E5 and E6 did not
know they had**:

★★ **OWNER DECISION NEEDED BEFORE E5.** The address plane cannot be populated from a CE
page-table write (§2.1's second-place risk, and the *only* populate source the C measured on
the compute path) while a CE write is undecodable. Three options, with the cost of each:

1. **Give `PushbufferAbi` a run-aware entry point** — e.g. a provided
   `decode_run(&self, &[(u32, Vec<u32>)]) -> Vec<PushMethod>` defaulting to a `decode_method`
   map. Additive, no mock breakage, and the GA10x impl accumulates `SET_OBJECT` →
   `OFFSET_*` → `LINE_*` → `LAUNCH_DMA` into one `CeLaunchDma`. ~~**Cost:** the accumulator is
   *state across methods*, so a run split across two GPFIFO ranges needs a decision about
   what a partial run means (the C's answer, and the safe one, is that it means nothing).~~
   ★ **CHOSEN, and this stated cost is STRUCK — see §8.2.1.**
2. **Move the accumulation into `kayfabe-fwd`'s `apply_pushbuffer`** and keep the arch seam
   per-method, with the arch supplying only "which method address is this". **Cost:** the
   accumulation logic becomes core and therefore arch-shaped, which is what the Axis-B split
   exists to prevent.
3. **Leave it refusing and drive E6 from the isolate's own encoder.** **Cost:** E6's green
   would then say nothing about the guest's pushbuffer, which is the whole north star.

⊘ Nothing here guesses between them. Option 1 is the shape this document's author would
pick; it is not what E4 shipped, and E4 did not widen its own scope to decide it.

### 8.2.1 ★★★ OWNER RULING 2026-08-02 — option 1, and "stateless" was never a requirement

> *"stateless isn't a requirement, it should be following the protocol, so if the protocol
> holds a state to emulate an action, so may you do, even if later you replay the thing on
> real hardware after translation whilst before you had to keep it yourself because you
> genuinely didn't know if this one needs emulation (privileged op of kernel)."*

**Option 1 is chosen.** Two things follow, and the second is the load-bearing one.

**1. The statelessness of `decode_method` was an ARTIFACT, not a principle.** Nobody decided
it; it is how the trait happened to get written, and §8.2 then reasoned *from* it as though it
were a constraint. **The protocol is stateful:** the engine holds method state across runs, a
copy is assembled into it, and `LAUNCH_DMA` fires what has accumulated. Mirroring that is
*following* the protocol. A seam that cannot hold state cannot express what the hardware does.

**2. ★★★ The accumulator is the CLASSIFICATION BUFFER, and that is WHY the state must be
held.** At the moment the methods arrive we do not yet know the op's **disposition** — whether
it is one we must **emulate** (a privileged kernel operation: the page-table write) or one we
**translate and replay for real** on the host GPU. That is unanswerable until the op is
*complete*. So the state is kept not as an optimisation but because **the question "does this
need emulating?" has no answer until all of it has arrived.**

★ This is already latent in the port, which is evidence the shape is right: `kayfabe-fwd`
carries `plain_copy` / `src_is_virtual` / `dst_is_virtual` / `ChannelOrigin::User`, and the
note at `lib.rs:2128` records that a page-table write is *"virtual-destination and would pass
any purely address-based test"*. The classifier exists **and already needs the whole op**.

#### ⊘ The stated cost above is STRUCK, and replaced by a different one

*"A run split across two GPFIFO ranges needs a decision about what a partial run means — the
C's answer, and the safe one, is that it means nothing."* **Hardware does not do that.** Engine
method state persists across GPFIFO entries; a partial run is not meaningless, it is
**incomplete state awaiting more methods**. The C's answer was a workaround, not the protocol,
and repeating it would have made us diverge from the thing we are emulating in order to avoid a
cost that is not real. ⇒ Option 1 is **cheaper** than its own write-up claimed.

**The real cost, which replaces it:** state a guest drives is state a guest can abuse. Bound the
accumulator the way the hardware bounds it — **finite, per-channel, and reset exactly where the
engine resets**. Unbounded accumulation over a hostile ring is the failure mode to design
against; "partial runs" never were.

⊘ **This ruling does NOT bear on the VA/GPA defect** (`mock_fidelity_audit.md`, finding A):
that a GPFIFO entry names a GPU *virtual* address is **upstream** of statefulness. Bytes must be
fetched from the right address before there is anything to accumulate, and a faithful
accumulator over bytes read from the wrong place is wrong **silently**. See §8.2.2.

### 8.2.2 ⚠ THE ORDERING QUESTION §8.2.1 DOES NOT ANSWER — is VA == GPA at this wall?

> ★★★ **ANSWERED — see §8.2.3.** It does **not** hold. This section is kept as the
> statement of the question and of why it had to be a boot; every `[unmeasured]` below is
> superseded there.

`read_pushbuffer` fetches the method bytes with `guest_read(vmm, r.gpa, &mut buf)`, where
`r.gpa` came from a GPFIFO entry that holds a **GPU virtual address**
(`ogkm-580: kernel-open/nvidia-uvm/uvm_channel.c:996,1006`, field `NVC56F_GP_ENTRY0_GET 31:2` /
`_GET_HI 7:0` at `clc56f.h:270,272`). That the field is virtual is `[inferred]` from those
citations and nothing else.

Whether the mismatch is currently *live* or merely *latent* is a separate and open question:
**does the guest's pushbuffer VA equal its GPA at the `RmInitAdapter` wall?** No run has been
performed either way — this is `[unmeasured]`, and it is why step 1 below exists.

Early in boot, before much is bound, it may hold **by accident** — which is precisely why
neither the mock fixtures (whose invented entry stores a real GPA) nor the GA10x fixtures
(which place method bytes *at* the address the entry names) could ever have caught it.

- **If it holds:** `decode_run` can be built and trusted now; the VA→GPA resolution is a
  correctness debt that detonates later, once real bindings exist.
- **If it does not:** it blocks immediately, because a faithful accumulator over bytes read
  from the wrong address is wrong *silently* — the worst shape.

⇒ It was settled by experiment rather than argument — two boots differing only in guest RAM,
`[run: docs/reference/bench_evidence/c93930d_run_e5ring{1,2g}_*.log, 2026-08-02, vast 46529600]`.

★★★ **CORRECTED 2026-08-02, after the owner asked why this blocks anything — IT DOES NOT, and
the paragraph that stood here had a false premise.** It said *"source (1) — bind-time RPC/ioctl
bindings — needs no table"*, quoting `mode2_address_table.md`, which describes the **C
artifact's** model. **On a GSP-client part that source has no producer at all:**
`MAP_MEMORY_DMA`/`UNMAP_MEMORY_DMA` are HAL stubs, so `RmEvent::MapMemoryDma` is never
constructed from the wire (`kayfabe_rmrpc` module docs, three independent oracles;
`decode_map_memory_dma` has no caller outside tests). ⚠ `Gpu::sync_rpc_mappings` still *runs* —
over an empty set — which is exactly why the wrong name survived in two places: **a live code
path with no live input reads as a live source.**

**The two populate sources on this wire are `GPU_PROMOTE_CTX` (built, `#93`) and the observed
CE page-table write (needed `decode_run`, now landed).** Both are done or unblocked.

⇒ **So VA ≠ GPA was never a blocker, and it is not a finding either** — it is the premise the
address table exists for (`mode2_address_table.md`: the table *is* the guest's TLB, miss =
fault). What is real is narrower and plainer: **`read_pushbuffer` does not translate**, and that
is ordinary work (§8.2.3). The genuinely useful part of the measurement was methodological —
**a single 8 GiB boot reads GREEN because the untranslated address happens to be a legal GPA**,
so only the RAM differential could see it.

### 8.2.3 ★★★ THE ANSWER (boots `e5ring1`/`e5ring2g`, rev `c93930d`) — it does NOT hold, and it is LATENT not LIVE

`[measured]` 2026-08-02, two boots at rev `c93930d` on vast `46529600` (RTX 3060 / GA106,
host driver 580.159.04 open, guest **stock** 580.159.04, `/dev/kvm`). Evidence on disk:
`docs/reference/bench_evidence/c93930d_run_e5ring1_*` and `…_e5ring2g_*`.

**The instrument.** §8.2.2 asked for the *entry-named* address, and that turned out to be
unreachable: the read path is not on any live path (below), so instrumenting it would have
produced an **empty** capture — and an empty capture is evidence of nothing
(`c_oracle_empty_rows_are_wrong`). What *is* reachable at this wall is the address one level
up, and it is the same address: the guest's channel alloc carries `gpFifoOffset`, and

| what | `[src]` |
|---|---|
| the field is a **virtual** offset | `ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080fifo.h:809`, *"Gpfifo Virtual Offset"* |
| at *this wall* it is `pChannel->pbGpuVA + pChannel->channelPbSize` | `ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/arch/maxwell/mem_utils_gm107.c:1232` |
| `pbGpuVA` is the `dmaOffset` an `NV04_MAP_MEMORY_DMA` returned | `:842` |
| and a GPFIFO **entry**'s `GET` is `pChannel->pbGpuVA + gpOffset`, packed into `GP_ENTRY0_GET`/`GP_ENTRY1_GET_HI` | `:1871-1879` |

⇒ the ring base and every entry address are the **same kind of address in the same
allocation**, so measuring one measures the other.

**The result.**

| boot | guest RAM (`e820`, the guest's own view) | `gpFifoOffset` the guest declared |
|---|---|---|
| `e5ring1` | 8 GiB — usable `0x1_0000_0000-0x2_7fff_ffff` | `0x1_2006_4000` |
| `e5ring2g` | 2 GiB — usable to `0x7ffd_bfff`, **nothing above 4 GiB at all** | `0x1_2006_4000` |

**Byte-identical across a 4× change in guest RAM, and at 2 GiB it names memory the guest
does not have.** It is not derived from guest physical layout. ⇒ **VA ≠ GPA. The
confusion is real.**

★★★ **And the 8 GiB row is the dangerous one, which is why "they matched" would have been
the wrong reading.** At the bench's normal 8 GiB, `0x1_2006_4000` *is* a legal guest-physical
address — so `Vmm::gpa_read` **succeeds** and hands back whatever guest RAM lives there. The
failure is not a fault; it is bytes. A single 8 GiB boot that only asked *"does the read
succeed?"* would have reported green.

⊘ **LATENT, not live, and that is the whole reason E5 was safe to build on top.** Two
independent facts:

1. **No live caller.** `kayfabe_rt::device::SharedDevice::parse_pushbuffer` is the only
   in-lock-legal entry point into `read_pushbuffer`, and nothing in `kayfabe-qemu-raw` or
   `kayfabe-shell` calls it. No guest byte is fetched through a `PushRange` on a boot.
2. **The guest never submits.** Both boots report `doorbells: 0 arrived, 0 served, 0
   REFUSED` across boot + `modprobe` + device open — unchanged from `5c1f501`. The channel
   `SCHEDULE` fails `0x56` before `kfifoUpdateUsermodeDoorbell` is reached.

⊘ **There is no second number to compare against, and that is a finding of its own.** The
binding that would resolve this VA is a `MAP_MEMORY_DMA`, and that RPC is a **HAL stub on
every GSP-client part** — `RmEvent::MapMemoryDma` has no producer on this wire at all
(`kayfabe-rmrpc` crate docs §2.7). So resolution cannot come from a table lookup of bindings
we have seen; it needs a **GMMU walk through the issuing channel's PDB**, which is machinery
`kayfabe_device`'s BAR2 aperture already has (`bar2_reads`/`bar2_writes` resolved 111/21 128
accesses on the same boots) but which `read_pushbuffer` — a phase that deliberately holds no
proc — cannot reach today.

⇒ ★ **The top-priority follow-up, stated as a shape rather than a wish:** `PushRange` must
say *what kind of address it carries*, and `read_pushbuffer` must refuse a virtual one by
name until a resolver exists. That is a phase-shape change (the resolve needs the channel's
`Vas`), it vacates the GA10x half of `pushbuffer_ga10x_hostile.rs`'s fault corpus unless the
corpus moves with it, and it was **out of E5's scope** — recorded here rather than done
half-way.

### 8.2.4 E5 — BUILT: `decode_run`, and what it refuses

`PushbufferAbi::decode_run(&self, &mut MethodState, &[(u32, Vec<u32>)]) -> Vec<PushMethod>`
is a **provided** method defaulting to a per-method map onto `decode_method`, so no mock
and no other generation changed. `Ga10xPushbuffer` overrides it and accumulates
`SET_OBJECT` → `OFFSET_*` → `LINE_*` → `LAUNCH_DMA` into one `PushMethod::CeLaunchDma` with
the driver's own operands. `kayfabe_fwd::apply_pushbuffer` calls it with
`Channel::method_state`.

**The bound, as the ruling demanded — finite, per-channel, reset where the engine resets:**
`MethodState` is a fixed `SUBCHANNELS(8) × METHOD_SLOTS(16)` array plus a written-bitmap and
eight class bindings. No heap, no map, no key the guest supplies, so there is no limit to
enforce and none to get wrong. It lives on `Channel`, so it dies with the channel;
`SET_OBJECT` clears the subchannel it rebinds, because method `0x400` on a new class is not
method `0x400` on the old one. Every accessor is total.

**The refusals grew rather than shrank.** A launch decodes to `Opaque` when: nothing is
bound, the bound class is not `AMPERE_DMA_COPY_B`, `DATA_TRANSFER_TYPE == NONE`,
`MULTI_LINE_ENABLE == TRUE` (a strided region is not the contiguous `len` the core means),
`REMAP_ENABLE == TRUE` (the fill pattern is in `SET_REMAP_CONST_A` under a component map we
do not validate — `CeWork::Fill`'s own docs say the pattern is part of the fact), **or any
operand was never written**. `0` is a legal offset and a legal length, so written-ness is
tracked separately from value; a `CeLaunchDma` assembled from `unwrap_or_default()` is E4's
invented destination one increment later.

★★★ **A C bug found on the way, and it is in the execute predicate.** The C reads
`bool mscrub = (d >> 23) & 1; /* MEMORY_SCRUB_ENABLE [23] */`
(`C: src/qemu/nvkvm_gpu_emul.c:6208`) and uses it at `:6310`. `MEMORY_SCRUB_ENABLE` **does
not exist on `NVC7B5`** — `grep -c MEMORY_SCRUB clc7b5.h` is `0`; it is
`NVC8B5_LAUNCH_DMA_MEMORY_SCRUB_ENABLE` at `23:23`, a **Hopper** class
(`ogkm-580: clc8b5.h:84-86`). On Ampere, bit 23 is the top half of `VPRMODE` (`23:22`,
`clc7b5.h:146-148`), and neither enumerated `VPRMODE` value sets it — so on the part the C
actually ran, `mscrub` is a constant `false` and its `!mscrub` conjunct is **vacuous**. ⇒
this port produces `CeWork::Scrub` from a GA10x launch **never**, and the constant is absent
rather than present-and-wrong.

**Instruments, and two gaps they found in themselves** (`suspect_the_instrument_first`):

- The compiled oracle (`tests/oracle/pushbuffer_abi_oracle.c`) emits whole CE runs packed
  with `DRF_NUM`/`DRF_DEF` and their readback with `DRF_VAL`, so every operand assertion
  compares our accumulator against *the driver's* extraction. 6 new tests, 12 → **18**;
  `ci.yml`'s `PUSHBUFFER-ORACLE-gate` floor moved 12 → 18. No new gate step, so
  `GATE_STEPS_ALL_MIN` is untouched.
- ⊘ **Gap 1, MEASURED.** *"Fill the operands with `unwrap_or_default()` instead of
  refusing"* was **missed by everything** in the first sweep: every prefix of a complete run
  stops *before* the launch, so no prefix ever asked a launch to fire with nothing latched.
  Three explicitly incomplete runs (`refuse_no_operands`, `_no_length`, `_no_offsets`) are
  what make that bite red.
- ⊘ **Gap 2, MEASURED.** *"Read `OFFSET_IN_UPPER` as a full 32 bits"* was **missed by
  everything**, because `DRF_NUM` masks its argument to `16:0` — so no emitted word ever had
  a bit above 16 and `word & 0x1FFFF` agreed with a bare `word` everywhere. Hardware does not
  zero that register for you. A `dirty_upper` sweep (bits 17..31 raw-set) is what makes the
  mask load-bearing. Same shape as the `FETCH`-bit gap E4 records, one class along: *a sweep
  that varies only the fields you thought of is not a sweep.*
- The ungated control (`tests/tests/pushbuffer_ga10x_hostile.rs`) gained four tests, and one
  of them was **rewritten after reporting `launches fired: 0` on both arms** — a random word
  is a valid `LAUNCH_DMA` header about one time in 2^29, so the assertion inside the loop
  never executed. It is now a **shadow differential**: real runs at random subchannels in
  random order with random operand values, and the codec must fire exactly when the stream
  wrote, with exactly what the stream wrote. `fired=127 refused=1251` — both arms reached,
  and the numbers are printed so a future vacuity is visible.
- 10 planted defects, 10 caught (`unwrap_or_default`, the 32-bit and the 8-bit operand mask,
  no-clear-on-bind, subchannel-0-always, dropped multi-line/remap/transfer/object checks,
  src↔dst swap, slot off-by-one).

### 8.3 The instrument: `tests/oracle/pushbuffer_abi_oracle.c`

The fourth compiled oracle in the tree, built exactly as the VBIOS / GMMU / token ones are:
`tests/build.rs` hands `cc` **absolute paths** into a checkout beside this repository,
nothing is vendored, an absent tree is a **loud skip** and a present-but-unbuildable one is a
**hard error**.

What is NVIDIA's, and it is all of the arithmetic:

- `class/clc56f.h` + `class/clc7b5.h` — every `GP_ENTRY*`, `DMA_*`, `SEM_*`, `MEM_OP_*` and
  `LAUNCH_DMA` field extent.
- `nvmisc.h`'s `DRF_NUM` / `DRF_DEF` — the **encode** side.
- ★★ `nvmisc.h`'s `DRF_VAL` — the **decode** side. Every assertion compares *our* decode
  against *NVIDIA's* decode of the same word, never against the value the harness was called
  with. That is what makes a sweep past a field's end meaningful: an address of 2^41 cannot
  survive a 40-bit entry, NVIDIA's extractor says what does survive, and a decoder reporting
  anything else has invented a field.
- `SF_OFFSET`/`SF_SHIFT`/`SF_MASK`, sliced byte-for-byte out of `generated/g_gpu_access_nvoc.h`
  — the macros that turn `NV_RAMUSERD_GP_GET`'s `(34*32+31):(34*32+0)` into a byte offset.
- `kfifoGetUserdSizeAlign_<HAL>`, sliced out of the file the driver's **own** dispatch table
  binds for `GA106`, with that file's **own** `published/…` includes.

★★ **The USERD binding is itself a finding.** `kfifoGetUserdSizeAlign` is halified two ways;
only `T234D`/`T264D` get their own arm and **GA106 falls to the fallback, which is
Maxwell's** (`kfifoGetUserdSizeAlign_GM107`, `*pSize = 1<<NV_RAMUSERD_BASE_SHIFT`). So an
Ampere channel's USERD is sized out of `published/maxwell/gm107/dev_ram.h`, and
`published/ampere/ga102/dev_ram.h` — the obvious place to look — contains **no `NV_RAMUSERD`
at all**. Reading the chip's own header and stopping there is exactly the mistake
`a_table_does_not_decide_behaviour` records; deriving the binding is what avoids it.

⊘ **This family has no `ci.yml` reached-count step**, unlike the other three oracles. Adding
one moves `scripts/ci_gates.sh --all`'s pinned step floor, which E4 was told to leave at 21
and which another agent is editing concurrently. **Until that step exists nothing stops these
tests vanishing from CI and from a developer box at the same time**; the `PUSHBUFFER-ORACLE-GATE:
RAN/SKIPPED` markers are emitted and greppable, and the floor is the missing half.

### 8.4 The control, and why it is a separate ungated file

`tests/tests/pushbuffer_ga10x_hostile.rs` is E4's stated control — *garbage must fault, not
decode to a plausible method* — and it deliberately does **not** depend on the vendored tree,
because it guards the property that costs a memory-safety fact when it breaks and must
therefore run on a CI runner too.

Refusal lives at three levels, and the file exercises all three:

| level | vocabulary | test |
|---|---|---|
| ring | **no `PushRange`** | `a_ring_that_is_not_whole_entries_yields_nothing`, `a_gpfifo_control_entry_yields_no_range` |
| range | `FwdFault` out of `read_pushbuffer` | `garbage_gpfifo_entries_fault_and_never_manufacture_a_method` |
| method | `PushMethod::Opaque` | `garbage_method_words_decode_to_nothing_the_core_acts_on`, the three `every_near_miss_of_*` |

★★ **The instrument checks itself.** `MockVmm::new()` declares the *whole* 64-bit space RAM,
so a hostile GPFIFO entry would read zeros and succeed and the fault arm would never fire.
`a_narrow_vmm_is_what_makes_the_fault_arm_reachable` asserts the narrowing is load-bearing by
running the same ring through both.

⊘ **What the corpus does NOT prove**: that no 32-bit word can ever decode to a modelled
method. A header *is* a 32-bit word. What it proves is that over 4 096 hostile rings and
2 048 noise ranges **none did**, and — the non-probabilistic half — that every *near miss* of
a real run, one field changed at a time, refuses.

### 8.5 Bites — and the two findings they produced about this work

`scripts/bite_pushbuffer_codec.py`, 26 planted defects across `submit.rs` and `ga10x.rs`,
four arms. `[measured]` at `77dde5d` on the 38-core build box, against content verified
byte-identical to `git show HEAD:` on both ends:

```
25/25 live bites caught. Per arm: oracle=23, hostile=11, abi=9, port=3.
14 caught by the ORACLE and not the control; 2 by the CONTROL and not the oracle — the two
are guarding different things and neither substitutes for the other.
1 rows are EQUIVALENT MUTANTS (required to stay green; 0 did not).
```

★★ **The two-instrument split is the number worth reading.** Thirteen defects are *wrong
bit positions* and only the compiled oracle sees them — the control cannot, because a
decoder with a shifted mask still refuses garbage perfectly well. Two are *loosened
refusals* (the exact-argument-count check, the incrementing-framing check) and only the
control sees them — the oracle cannot, because it only ever feeds well-formed runs. A
project that had built one and not the other would have had a green suite over a real
defect either way.

⊘ **The `port` arm caught 3 of 25.** `tests/gsp_rm_alloc.rs`'s tripwire on the shipped
`Arch` is a *generation-swap* check, not a codec check, and this is that stated as a
number rather than as a hope.

★★★ **Two findings, both about the instruments rather than the code, and both are the
reason the harness exists.**

1. **The oracle had a blind spot the first run found.** `GP_ENTRY0_GET` is `31:2`, so bits
   `1:0` are not the address — bit 0 is `FETCH`. Every entry in the first sweep used
   `FETCH_UNCONDITIONAL`, so `entry0 & 0xFFFF_FFFC` and a bare `entry0` agreed **everywhere**
   and the bite "read the FETCH bit as address bit 0" was **MISSED BY EVERYTHING**. The
   oracle now emits `FETCH_CONDITIONAL` cases (and one with bit 1 raw-set) and the bite is
   caught. ⊘ A sweep that varies only the fields you thought of is not a sweep.
2. **The bite harness itself was contaminated across files**, and it was caught only because
   an `EQUIVALENT` row went red. The harness bites two files; its first version restored only
   the file it had just bitten, leaving the *other* holding the previous bite — so every row
   whose predecessor bit a different file was measured against **two** defects. The tell was
   bite 25, a provably behaviour-preserving rewrite in `submit.rs`, reported RED because bite
   24's fabricated `CeLaunchDma` was still in `ga10x.rs`. The harness now restores every file
   before each bite. ⊘ The single-file harness this was copied from cannot exhibit this,
   which is why nothing in the pattern guarded against it. `suspect_the_instrument_first`.

### 8.6 Evidence

- **Suite**, `[measured]` at `77dde5d` on the 38-core build box:
  `cargo test --workspace --no-fail-fast` with `KAYFABE_NO_KVM` **unset** →
  **1895 passed, 0 failed, 1 ignored**; `KVM-GATE: RAN` **56**, `SANDBOX-GATE: RAN` **10**,
  `PUSHBUFFER-ORACLE-GATE: RAN` **12** (`GMMU` 15, `TOKEN` 2, `VBIOS` 13, all unmoved).
- **Baseline** for comparison, `[measured]` at `e43bc71` on the same box, same command:
  **1866 passed, 0 failed, 1 ignored**, `KVM-GATE: RAN` **56**. ⇒ +29 tests, all new.
- **Gates**, `[measured]` at `77dde5d`: `./scripts/ci_gates.sh --all` →
  `ALL GATES CLEAN (21 steps, floor 21 for --all mode)`. ⊘ No step was added, so the pinned
  floor is untouched — see §8.3 for what that costs.
- **Claim ledger**: `scripts/claim_ledger.py --gate` → 382 unattributed / 66 conflated /
  17 bare-hardware, i.e. the baseline, unmoved.

### 8.7 ⊘ What E4 does NOT establish

1. **No boot.** `only_live_boots_are_proof`: every number here comes from NVIDIA's macros
   compiled on a build box. **No guest has been booted against this codec**, and nothing here
   says a real driver's pushbuffer parses. The first arm that could measure it is E6.
2. **No forwarding.** No `VerbPlan` ran, no doorbell was rung, no `ce_copy`.
3. **Nothing about a CE copy's operands** — see §8.2, which is the point.
4. **Nothing about any other generation.** The bindings are derived for `GA106`;
   `Ad10xArch`/`Gh100Arch` still delegate these two seams to `MockArch`'s invented ones.
5. **Nothing about a run split across GPFIFO ranges.** `read_pushbuffer` decodes each range
   independently, so a method run straddling two entries yields a short argument list in the
   first and a header-shaped datum in the second. The codec refuses both (exact-count check;
   whatever the datum sizes to is `Opaque` unless it is genuinely a modelled run), but *that
   the guest never does this* is `[assumed]`, not measured.

## 9. §8.2.3 CLOSED + E5 — the translation, and the wall the join measured

### 9.1 `read_pushbuffer` TRANSLATES, and refuses what it cannot

§8.2.3 recorded the shape of the fix and left it out of E5's scope. It is built now.

**The type is what changed.** `kayfabe_arch::PushRange::gpa: u64` is
`PushRange::va: GpuVa` — *"a GPA-typed field holding a VA is the whole bug"*.

⊘ **A newtype, deliberately NOT an `enum { Gpa, GpuVa }`.** A `Gpa` arm would be a
supported way back into the untranslated read, and nothing on this wire produces one:
`pChannel->pbGpuVA` is assigned **unconditionally** from a `MAP_MEMORY_DMA` `dmaOffset`
(`ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/arch/maxwell/mem_utils_gm107.c:842`), every
entry is `pbGpuVA + gpOffset` (`:1871-1879`), and the control field is *"Gpfifo Virtual
Offset"* (`ogkm-580: ctrl2080fifo.h:809`).

An architecture that genuinely had a physical GPFIFO would need a seam of its own, and
⊘ **no such part has been observed** — which is a reason to leave no variant for it, not
a reason to leave one nobody checked.

**Resolution goes through the address table, per `mode2_address_table.md`: the table IS the
guest's TLB, miss = FAULT, no reverse-resolve, no heuristic.** The refusals, each its own
name because each is a different plane saying something different:

| what | refusal |
|---|---|
| the channel declares no VAS (`vas_pdb == None`), or its `Vas` is gone | `FwdFault::NoVas` / `UnknownPdb` — *there is no table to miss in* |
| no binding covers the VA | `FwdFault::Address(AddressFault::Miss)`, naming the exact faulting VA |
| the binding resolves into video or peer memory | **`FwdFault::PushbufferAperture`** (new) — `Binding::phys` is a guest *framebuffer* offset there, and handing it to `Vmm::gpa_read` would read the guest RAM page that happens to share the number |
| the range cuts into more than `MAX_PUSH_SPANS` (4096) bindings | **`FwdFault::PushTooFragmented`** (new) — loud, never a truncated read |
| the read itself | `FwdFault::GpaRead` / `NonRamGpa`, unchanged |

★ **A VA range is contiguous; the memory behind it need not be.** `push_range_gpas`
partitions the (already length-capped) range across bindings and reads each run separately.
Resolving once and reading `len` bytes from the first binding's physical address would run
off its end into whatever guest page follows — the same *"the read succeeded, the bytes are
wrong"* failure as the untranslated read, one level down.

`MAX_PUSH_RANGE_BYTES` / `MAX_PUSH_TOTAL_BYTES` are untouched (boundary-1: a hostile length
is still a bounded read). The clamp happens **before** translation, so a hostile length
cannot make the walk traverse the whole table.

#### ★★ The phase-shape change, stated rather than slipped in

Translating needs the issuing channel's `Vas`, which needs the proc, so the guest-memory
read **moved from `route_act`'s ROUTE phase into its ACT phase**. Three things about that:

1. **No new lock is acquired.** `SharedDevice::route_act` takes device-read (rank 0) then
   that proc's mutex (rank 1) for one operation; the read moved from between those two
   acquisitions to after the second. The lock *set* and the rank *order* of the whole entry
   point are unchanged.
2. **The in-lock-legality argument is unchanged, because it never mentioned a rank.**
   `Vmm::gpa_read` is legal here only because the port refuses a GPA that is not host RAM
   (`FwdFault::NonRamGpa`) — a backend that served a device-aimed GPA would take the VMM's
   global lock *beneath one of ours*, which is `l1_os_shell.md` §6.3's ABBA whether the lock
   above it is rank 0 or rank 1.
3. ⊘ **Deliberately NOT split into plan/execute/commit.** R1 forces that shape for
   *blocking* calls; `gpa_read` is a bounded copy out of a mapped window. Splitting would
   mean resolving addresses, dropping the lock, then fetching method bytes through a
   translation the guest was free to invalidate in the gap — a TOCTOU built on purpose.

★★★ **An instrument noticed the move before any human did.** `l1_mean.rs`'s lock-depth
witness asserted `(0, 1)` — *"exactly 1: the route phase holds rank 0 and nothing else while
it reads guest memory"* — and went red at `(0, 2)`. It is updated by **attributing** the
change, still as an exact equality, and it now differs by lock mode (Sharded 2, Degenerate
1, which reaches the proc through the single device write guard). The lock-depth field is
consequently normalised out of the cross-mode differential, because it is a fact about the
lock *configuration* and the guest cannot observe our lock mode.

#### ★★ The mock's GPFIFO entry now names a VA, and that is the fourth `mock_fidelity` instance

`kayfabe_mocks::MockPushbuffer`'s invented 16-byte entry stored a genuine **GPA** — an
encoding no NVIDIA chip has — which is precisely why no mock-driven test could ever see the
question. It names a VA now. Every mock-driven pushbuffer fixture therefore binds its ring
first (`kayfabe_tests::bind_ring`), which is what the guest's own driver does, and the
fixture's VA is deliberately **biased away from the GPA** (`kayfabe_tests::PB_VA_BIAS`,
2^39 — above every fixture GPA, 4-byte aligned, and inside the 40 address bits a real
`GP_ENTRY0_GET`/`GET_HI` has). An identity fixture could not tell a translating read from
the untranslated one it replaces.

#### ★ The hostile corpus MOVED rather than shrank

`pushbuffer_ga10x_hostile.rs`'s range-level row (`GpaRead`/`NonRamGpa`) would have become
**unreachable**: an entry naming an arbitrary number now refuses at the address table, one
layer earlier. Its fixture grew a GA10x-class process and exactly one wide binding, so both
layers stay live, and the file's own header now tabulates **four** refusal levels instead of
three. The same move was made in `c_bug_regressions.rs`'s near-`u64::MAX` regression (which
now asserts the address-table miss **and** the `GpaRead` it was minimized from),
`security_boundary.rs`'s length flood, and `l1_mean.rs`'s five hostile descriptor shapes.

### 9.2 E5 — what is BUILT, and ★★★ the wall the join ran into

`tests/tests/e5_address_table_join.rs` is the join: neither `promote_ctx.rs` nor
`pt_decode.rs` proves that the two sources land in **one** table a copy-engine command then
resolves against, and a composition is exactly what per-source tests cannot assert.

**Source 1 — `GPU_PROMOTE_CTX` — is whole.** A promoted range resolves at its own offset,
both operands of a CE copy are **found** (`Representability::Fabricated`, neither end
`Untracked`), publishing moves the operand to `HostBacked` **at the identical VA** (`#102`'s
identity law, whole-table audited), and a range bound in one `Vas` does not resolve in
another on the same proc (the C's #12 collision class).

**The control holds at all three places the law is enforced**, asserted by variant:
`AddressTable::resolve` → `AddressFault::Miss`; `read_pushbuffer` → the same, before a guest
byte is fetched, over a `Vmm` that would have served the number; `gate_working_set` → the
#14 ring gate. With the non-vacuity half: a bound, published VA passes the same gate.

★★★ **Source 2 — the observed CE page-table write — reaches a ROOT PAGE AND NO FURTHER, and
this is the finding.** `[measured]` 2026-08-02 at rev `4e8960f`, by
`tests/tests/e5_address_table_join.rs::the_ce_pt_write_source_can_witness_only_a_root_page_today`
— which fails if any of the four links below stops holding. The chain, first link first:

1. `classify_ce` emits a `PtWrite` only for a destination `Spine::pt_page_owner` recognises,
   and that index (`Spine::pt_roots`) holds **roots only** — its own doc says so:
   *"seeded from each live `Vas`'s **root**… Deeper levels are forward-populated by the
   decode at the guest's commit point (**the next stage**)"*.
2. `latch_pt_writes` is the only writer of `Vas::pt_pages`.
3. `plan_pt_decode` drains `pt_pages` and is the only caller of `ReachShadow::witness`.
4. `settle` binds a leaf only from a page that is reachable **and** witnessed.

⇒ a guest CE write into a *leaf* table is classified as ordinary data, forwarded, and never
witnessed; every leaf under it stays `unwitnessed`, which is a MISS, which is a FAULT. The
bytes decode correctly and the metadata chain **is** learned forward — only the binding is
withheld. So the compute working set's leaves cannot arrive through this source today.

⊘ **Not worked around.** Hand-latching the leaf page — which `pt_decode.rs`'s fixtures do,
correctly, to exercise the decoder — would have made the join report something the live path
cannot perform (`mock_fidelity_both_directions`). It is asserted by name
(`the_ce_pt_write_source_can_witness_only_a_root_page_today`) and goes **red** the day the
next stage lands, which is the only way an absence stays visible.

⊘ **And it was not built here.** Closing it needs a device-global index of page-table pages
*learned* by a decode, but `commit_pt_decode` holds only `&mut Proc` (rank 1) and publishing
into the spine (rank 0) from there inverts the lock order. That is a phase-shape change of
its own — an increment, not an edit — and inventing one inside E5 is the shape this document
exists to refuse.

### 9.3 Evidence — the boot, and ★ the wall is UNCHANGED

`[measured]` 2026-08-02, rev **`ee3a8c3`**, vast `46529600` (RTX 3060 / GA106, host driver
580.159.04 open, guest **stock** 580.159.04, `/dev/kvm`). Boot `e5va1`, captured by
`scripts/bench/boot_capture.sh`; on disk at
`docs/reference/bench_evidence/ee3a8c3_run_e5va1_{dmesg,probe,qemu}.log`. The QEMU binary
was verified stamped `kayfabe-rev:ee3a8c3f5015f4b2adcbe9829a0cd7b59b14087d` before the run,
and the harness's own content check passed (26 dmesg lines, 22 `NVRM`, 3 `RmInitAdapter`).

**The wall is where it was**, and the chain is the one `c93930d`'s boot printed:

```
NVRM: _memmgrMemUtilsScrubInitScheduleChannel: Unable to schedule channel, status: 56
NVRM: … memmgrMemUtilsChannelSchedulingSetup … @ mem_utils_gm107.c:1027
NVRM: nvAssertFailedNoLog: Assertion failed: status == NV_OK @ ce_utils.c:304
NVRM: … objCreate(&pScrubber->pCeUtils, …) @ mem_scrub.c:181
NVRM: … scrubberConstruct(pGpu, pHeap) @ mem_mgr_scrub_gp100.c:63
NVRM: RmInitNvDevice: *** Cannot load state into the device
NVRM: GPU 0000:00:03.0: RmInitAdapter failed! (0x25:0xffff:1249)
```

★ **The `dmesg` is byte-identical to boot `e5ring1` at `c93930d` modulo kernel
timestamps** — `diff` over both files with the `[    n.nnnnnn]` prefix stripped is **empty**,
26 lines each. **No new wall, and no regression.** That is the expected outcome and it is
worth saying plainly rather than implying: `read_pushbuffer` is still **latent** on the
product path (nothing in `kayfabe-qemu-raw`/`kayfabe-shell` calls `parse_pushbuffer`, and
the guest still never submits — the channel `SCHEDULE` fails `0x56` before
`kfifoUpdateUsermodeDoorbell` is reached), so a boot **cannot** have moved. What this run
buys is the other half of `suspect_the_instrument_first`: the change did not break the boot
either, and the claim that it is latent is now a measurement rather than a reading.

#### Suite, gates, ledger

- **Suite**, `[measured]` at `ee3a8c3` on the 4-core dev box, `KAYFABE_NO_KVM` unset:
  `cargo test --workspace --no-fail-fast` → **1960 passed, 0 failed, 1 ignored**;
  `KVM-GATE: RAN` **56**.
- **Baseline** for comparison, `[measured]` at `c582da3` on the same box, same command:
  **1954 passed, 0 failed, 1 ignored**, `KVM-GATE: RAN` **56**. ⇒ **+6**, and all six are
  `e5_address_table_join.rs`. Every other file in the change was *rewired*, not extended:
  the corpora moved with the translation rather than growing beside it.
- **Gates**, `[measured]` at `ee3a8c3`: `./scripts/ci_gates.sh --all` →
  `ALL GATES CLEAN (22 steps, floor 22 for --all mode)`. No gated oracle family was added,
  so `GATE_STEPS_ALL_MIN` and `ci.yml`'s floor are both untouched.
- **Claim ledger**: 382 unattributed / 66 conflated / 17 bare-hardware — the ceiling and
  both bars, unmoved. The three sites §9 first added were fixed by **attributing** them
  (a test name, a revision and a date; and by splitting an `ogkm` citation out of the
  sentence that carried the word *measurement*), never by raising a bar.

### 9.4 ⊘ What §9 does NOT establish

1. **The boot proves the wall did not move, and nothing more.** `only_live_boots_are_proof`
   cuts both ways: an unchanged `dmesg` says the port still reaches exactly as far as it
   did, not that any of §9.1's translation ran. The guest never submits at this wall, so
   `read_pushbuffer` remains **latent** on the product path — what changed is that it is
   now *correct* when it becomes live.
2. **Nothing about the aperture refusal on real traffic.** `FwdFault::PushbufferAperture` is
   reachable and tested, but no measurement says whether a real GA106 guest ever puts its
   pushbuffer in vidmem. `ogkm-580: mem_utils_gm107.c:812-820` has RM refusing *"USERD in
   sysmem and PushBuffer/GPFIFO in vidmem"* as a WAR, which is suggestive and is **not** the
   same statement.
3. **Nothing about E6.** No `VerbPlan` ran, no doorbell was rung, no `ce_copy`.

## 10. E6 — BUILT. The join, its acceptance on hardware, and ★★★ the wall that replaced it

### 10.1 What was missing, and it was not an algorithm

`PushbufferOutcome::ce_spans` — the partitioned copy-engine request `apply_pushbuffer` has
produced since `#102` — had **no caller anywhere in the workspace**. `plan_ce_split` built
the isolate's instruction and nothing called *it* either. So every copy the core recovered
from a guest ring was computed, asserted about in tests, and dropped.

`[src]` at `147c069`:

| seam | what it is |
|---|---|
| `kayfabe_fwd::plan_ce` / `commit_ce` / `exec_ce` | the three R1 phases for a partitioned request |
| `kayfabe_fwd::submit_ring` | `parse_pushbuffer` + `exec_ce`, single-threaded |
| `kayfabe_rt::SharedDevice::forward_ce` / `submit_ring` | the L1 form — each half takes and releases its own locks, so the host verb runs lock-free |

**Three refusals `plan_ce` owns**, each a different plane speaking: `UnknownChannel`
(nothing to submit *on* — deliberately not folded into the doorbell's device-wide
`UnknownVchid`), `NoTarget` (no executor), and **`NoHostVas`** (the channel's `Vas` was
never host-published, so the addresses denote nothing in any host address space).
⊘ The last one is a refusal and **not** a materialization: allocating an empty host VAS to
let the chain proceed would point a real engine at addresses that resolve to nothing in it,
which is `Xid 31 FAULT_PDE` arrived at by *our* choice rather than by the guest's.

`commit_ce` adopts **nothing** — `VerbPlan::CeSplit` mints no handle — so its whole content
is R5 attribution: the bytes have already moved by the time it runs, and refusing is not
undoing but declining to file the result against a channel that no longer exists. It is
never retryable: re-running a copy that executed performs it **twice**.

### 10.2 ★★★ The CONTROL found a defect, and it is a HANG rather than a wrong answer

E6's row says *"the same boot with `KAYFABE_ISOLATES` unset must not produce it"*. The
shipped archive's default plane is `StillbornIsolates`. Writing that control found this:

> `kayfabe_isolate::Isolate::checkout` answers `None` for **two** conditions its own docs
> run together — *"the pool is saturated (or the isolate is retired and refuses new
> checkouts)"*. `kayfabe_fwd::checkout` passed both up as `Ok(None)`, and
> `SharedDevice::verb_op` treats that as **backpressure**: release the locks, park on the
> pool gate, re-enter. A stillborn isolate has pool width **zero** and is retired at birth,
> so the generation never moves and **the vCPU thread parks forever**.

It was unreachable before E6 only because nothing had ever routed as far as a checkout: the
shipped doorbell died at `UnknownVchid` first. The join is what makes a submission reach the
pool, and the configuration that hangs is the **shipped default**.

⇒ `FwdFault::IsolateRetired`, decided by one predicate (`kayfabe_fwd::never_serves`) used at
**both** checkout doors.

★★ **And the first fix was wrong in the way this project keeps being bitten.** It changed
`checkout` only. Every unit test went green — and
`a_permanently_dead_isolate_is_REFUSED_and_does_not_park_forever` still **timed out**,
because L1 does not go through `checkout`: `Staged::check_out` calls `checkout_and_drain`,
which reaches `Proc::checkout_with_pending_release` directly. *A mutation must be shown to
change BEHAVIOUR on the path the product takes, not merely to change bytes.* The predicate
is one function now, cited at both doors, for exactly that reason.

### 10.3 ★★★ Debt Q24 — discharged BEHAVIOURALLY

E2 recorded it: *"the behavioural witness — declare a channel through the bridge, ring its
vChid through the doorbell — is an **E6** assertion, because nothing in this port can inject
an `RmEvent` chain."* `Regs::object_model()` is that injection point — the composition
root's own `Arc<SharedDevice>`, the same one the boxed object policy declares into and the
same one `SharedDoorbell` rings.

`crates/kayfabe-qemu-raw/tests/e2_doorbell.rs::the_doorbell_reaches_the_same_object_model_the_bridge_declares_into`:
with nothing declared the ring is `FwdFault::UnknownVchid`; after a GA10x channel is
declared through the object model the **same token** comes back
`FwdFault::IsolateRetired` — the first refusal downstream of routing in that port's life.

★ `[measured]` 2026-08-02 on the dev box at rev `147c069`, by planting the defect in
`Regs::create` (a second `object_policy()` whose `SharedDevice` is handed to
`SharedDoorbell`) and running `cargo test -p kayfabe-qemu-raw --test e2_doorbell`: the new
behavioural test reads `UnknownVchid` on both sides and goes red, *and* the old
source-quantified `the_archive_realizes_exactly_one_object_model` goes red. Restored tree:
8 passed, 0 failed.

★ **The token is `0` because the fixture's channel DECLARES chid 0** — a choice, since
2026-08-03. It used to be forced: `Ga10xArch::vchid_from_userd_flags` answered `VChid(0)`
for **every** channel, a stated refusal, so `VChid(0)` was the only vChid a GA10x channel
could be filed under. Task #174 settled the decode the way E3's token was settled — by
compiling RM's own writer, reader, recombination **and** eheap granularity as a fifth
oracle (`tests/oracle/userd_chid_oracle.c`, `tests/tests/userd_chid_oracle.rs`) — and the
fixture now builds its flags word with the encoder that oracle holds in place. A channel at
any chid routes; this fixture exercises one of them. See §10.6.

### 10.4 ★★★ The acceptance, ON HARDWARE — `CeEvidence::copied() == true`

`[measured]` 2026-08-03, vast `46529600` (RTX 3060 / GA106, host driver 580.159.04 open),
**twice, at two revisions**: rev `147c069`
(`docs/reference/bench_evidence/147c069_e6_hw_join.out`) and rev **`5511cda`**
(`…/5511cda_e6_hw_join.out`), byte-identical evidence. `tests/tests/e6_hw_join.rs`,
`GPU-GATE: RAN`:

```
★ E6 ACCEPTANCE: CeEvidence::copied() == true — CeEvidence { before: 1592614637,
  after: 3237998080, after_last: 3237999103, expect_after: 3237998080,
  expect_after_last: 3237999103, bytes: 4096,
  submit: SubmitOutcome { semaphore: 2, gp_get: 2, gp_put: 2 }, payload: 2 }
```

`before` is the sentinel `0x5EED_5EED`; `after` is `0xC0FF_EE00`; `after_last` is
`0xC0FF_F1FF` = the ramp's 1024th word; `gp_get` met `gp_put` and the engine released the
payload. **This is R17's predicate, not a re-derivation of it** — the fourth conjunct comes
from the join's own `CeWitness`, which records what `ce_copy_outcome` observed.

What differs from R17 is **one thing**: the copy is not hand-built. It is recovered from a
guest's GA10x GPFIFO entry (`kayfabe_abi::submit::gp_entry`) naming five real CE method
runs, by `read_pushbuffer` → `decode_run` → `partition_ce` → `plan_ce` → `Worker::execute`.

★ **The predicate was watched to FAIL first**, on the same hardware, with a mutation proven
live: `partition_ce`'s source set to the destination. The copy still *executed* — semaphore
2, `gp_get == gp_put == 2` — and the evidence read
`after: 1592614637, after_last: 1592614637`, i.e. *"the engine ran a copy that moved
nothing"*. Restored tree: green again. A second mutation (destination + 0x1000) went red one
guard earlier, at the join itself (`Rm(Other(19275))` — the copy never retired).

#### ⊘ Two arms, because a published backing is CPU-OPAQUE BY DESIGN

`[measured]` on the RTX 3060 bench at rev `5ac789d`, run `e6_hw_join`, and it changed the
shape of the test: `NV_ESC_RM_MAP_MEMORY` on a backing minted by `RmBackend::alloc_sysmem`
is refused **`NV_ERR_INVALID_ARGUMENT` (`0x1F`)**, so the first draft of this file died at
`fill_words`.

The reason is a reading, and it is stated as one. `alloc_sysmem` passes
`NVOS02_FLAGS_MAPPING_NO_MAP`; that constant's own docs call it *"right for a data buffer
the GPU alone touches"*, and `[src] ogkm-580: src/nvidia/arch/nvalloc/unix/src/escape.c:342-345`
is where the frontend declines to build an `mmap` context around the descriptor.

⇒ a published backing is opaque to the CPU **in both directions** — the sentinel cannot be
written and the answer cannot be read. Relaxing `NO_MAP` to make the diagnostic work would
be changing the product to fit its instrument.

| arm | operands | what it establishes |
|---|---|---|
| **1** | published through `SharedDevice::publish_backing` at the **guest's own** VAs | address identity holds on real hardware (`host_va == guest VA`), the operands resolve **`HostBacked`**, the plan chooses a real engine, and the engine **retires** (`SubmitOutcome::landed`) |
| **2** | device-local, mapped by hand into the **same** host VAS the channel's `Vas` holds | the **bytes** — `CeEvidence::copied()` in full |

⊘ Arm 2's operands are `Untracked`, not `HostBacked`, so arm 2 says nothing about the
address plane; arm 1 does. Neither substitutes for the other. The `NO_MAP` refusal is itself
asserted, so the reason arm 2 exists cannot quietly stop being true.

### 10.5 ★★★ THE BOOT — and a NEW WALL, on the plane E6 exists to use

**Arm A, the default plane** (`KAYFABE_ISOLATES` unset). `[measured]` rev `147c069`, boot
`e6join1`, evidence `147c069_run_e6join1_{dmesg,qemu,probe,capture}.log`.
⊘ `git diff --name-only 147c069 5511cda` is `docs/…` plus **one test target that is not
linked into the archive** (`tests/tests/e6_hw_join.rs`), so the binary these boots ran is
this branch's content — the discipline §7 states. **The wall is
UNCHANGED**: `diff` of the dmesg against `ee3a8c3`'s `e5va1` with kernel timestamps stripped
is **empty**, 26 lines each, `RmInitAdapter failed! (0x25:0xffff:1249)`. The device reports
`doorbells: 0 arrived, 0 served, 0 REFUSED`.

⊘ **And it could not have moved**, which is worth saying plainly rather than implying: the
guest never submits at this wall (`kfifoRingChannelDoorBell_HAL` is reached *after* the
channel schedule that fails `0x56`), and nothing in `kayfabe-qemu-raw`/`kayfabe-shell` calls
`parse_pushbuffer` or `submit_ring`. E6 is **latent** on the product path for exactly E5's
reason. What the boot buys is the other half of `suspect_the_instrument_first`: the change
did not break it either.

**Arm B, the real plane** (`KAYFABE_SHIM_FEATURES=host-isolates`, `KAYFABE_ISOLATES=real`).
★★★ **QEMU ABORTS on a guest register write.** `[measured]` rev `147c069`, boot
`e6join2real`, evidence `147c069_run_e6join2real_qemu.log`:

```
thread '<unnamed>' panicked at crates/kayfabe-util/src/lockwitness.rs:125:5:
R1 no-blocking-under-lock violation (l1_concurrency.md §3.3): spawning a sandboxed child
process while holding rank(s) [0] …
thread caused non-unwinding panic. aborting.
  19: kayfabe_shim_regs_write
  20: nvkvm_trap_write
```

The lazy isolate spawn (E0b) is a **blocking call made under the device lock**, and the lock
witness aborts the hypervisor rather than let it convoy. ⇒ **The real isolate plane cannot
be used on a live boot at all today**, which is the plane E6 exists to reach.

★★ **ATTRIBUTED, not guessed.** The identical panic is present at the **base** revision:
`[measured]` boot `basereal` at rev **`f0b7efa`**, same box, same feature build,
`f0b7efa_run_basereal_qemu.log`, `grep -c "R1 no-blocking-under-lock"` = **1**. So this is
**not E6's regression** — it is a pre-existing wall E6's boot is the first to walk into,
because E6 is the first increment with a reason to run that plane. ⊘ It also means E0/E0b/E1's
live real-isolate measurements (revs `853a311`/`5c1f501`) no longer reproduce, and *when*
between those revisions it broke is **unmeasured**.

⇒ **This is the next increment, and it is a phase-shape change, not an edit**: `Gpu::refresh`
materializes isolates while the caller holds rank 0, so the spawn must move to a
plan/execute/commit shape of its own — decide *which* isolates are needed under the lock,
spawn with **zero** locks held, adopt under the lock with R5 re-validation. Inventing that
inside E6 is what this document exists to refuse.

### 10.6 ⊘ What E6 does NOT establish

1. **No live guest drove it.** The acceptance is a guest's *bytes* and a guest's
   *declarations* through the production core, on real silicon — not a booted guest. Three
   named things still separate them: (a) the boot wall above, unmoved; (b) the real isolate
   plane aborting QEMU (§10.5 arm B); ~~(c) `Ga10xArch::vchid_from_userd_flags` answering
   `VChid(0)` for every channel~~ — ★ **(c) CLOSED 2026-08-03, task #174.** The USERD decode
   landed, judged against a fifth compiled oracle that carries RM's own writer, reader,
   recombination and the `ownerGranularity` its own eheap holds; the seam now returns
   `Option<VChid>` and a word that names no channel is
   `ProjectionError::UnnamedVchid` rather than a substituted `VChid(0)`. ⊘ It is still not a
   live boot — `only_live_boots_are_proof` — so what moved is that a live guest's doorbell
   *can* now route to its own channel, not that one has.
2. **Nothing about the sandboxed isolate.** The hardware acceptance runs an **in-process**
   `HostRmBackend`, because a `CeWitness` and a CPU mapping both die at the process
   boundary. That the same verb chain survives the sandbox is R10/R11/R16's measurement.
3. **Nothing about guest RAM.** The method words live in a `MockVmm`; no hypervisor is
   running. The core reads those bytes and the GPU never does.
4. **Nothing about the completion plane.** `submit_ring` does not ring anything for the
   guest and raises no interrupt; the copy's completion is consumed by `ce_copy`'s own wait.

### 10.7 Suite, gates, ledger

- **Suite**, `[measured]` at `5511cda` on the **RTX 3060 bench** (the KVM box),
  `KAYFABE_NO_KVM` unset: `cargo test --workspace --no-fail-fast` → **1975 passed, 0
  failed, 1 ignored**; `KVM-GATE: RAN` **56**, `GPU-GATE: RAN` **1**.
- **Baseline** at `f0b7efa` on the 4-core dev box, same command: **1960 passed, 0 failed,
  1 ignored**, `KVM-GATE: RAN` **56**. ⇒ **+15**: 13 in `e6_join.rs`, 1 in `e6_hw_join.rs`,
  1 in `e2_doorbell.rs` (§10.3's behavioural witness).
- **Gates**, `[measured]` at `5511cda`: `./scripts/ci_gates.sh --all` →
  `ALL GATES CLEAN (22 steps, floor 22 for --all mode)`. No gated oracle family was added,
  so `GATE_STEPS_ALL_MIN` and `ci.yml`'s floor are untouched.
- **Claim ledger**: 382 unattributed / 66 conflated / 17 bare-hardware — the ceiling and
  both bars, unmoved. ⊘ It went red **twice** while §10 was written and was fixed by
  **attributing** both times: once at 384/382 (the two new sites got a run name, a revision
  and a date) and once at 67 conflated (an `ogkm` citation and the word *measured* sharing
  a sentence — the conflated bar's exact bite; the reading is now its own paragraph).
- ★ **Two gate steps went red on the way and neither was silenced.** `GPU-GATE: RAN` counted
  **0** over a full suite run in which the test passed against a real GA106 —
  `libtest_capture_swallows_thread_output`, fixed the way `kvm_gate::report` already was.
  And clippy's `too_many_arguments` (8/7) on the ring helper, fixed by grouping the copy's
  two ends rather than with an `#[allow]`.

---

## 11. E7 — §10.5's blocker, FIXED. The real isolate plane boots.

### 11.1 What was wrong, and it was one phase boundary

§10.5 arm B named it and refused to fix it inside E6: *"the spawn must move to a
plan/execute/commit shape of its own — decide which isolates are needed under the lock,
spawn with **zero** locks held, adopt under the lock with R5 re-validation."* That is
exactly what landed. The design record — the defect, why 1975 green tests could not see
it, the two R5 shapes, and the six doors the deferral had to be named at — is
`l1_concurrency.md` §12.47; it is not repeated here.

⊘ **Nothing about the increments E0–E6 changed.** E0b's property is preserved, and §11.2's
runs `r1real1`/`r1real2` at rev `e726844` re-measure it rather than assuming it: the spawn
still follows the guest's action. The deferral moves *where in the lock discipline* the
spawn happens, not *what causes* it.

### 11.2 ★★★ The acceptance — a real-plane boot that does NOT abort

`[measured]` rev **`e726844`**, vast **46529600**, RTX 3060 (GA106 `10de:2504`) at
`00:03.0` in the guest, host driver **580.159.04 Open Kernel Module**, guest Ubuntu 24.04
with a **stock unpatched** 580.159.04 open module. Archive built
`KAYFABE_SHIM_FEATURES=host-isolates`; the QEMU binary **and** `libkayfabe_qemu_raw.a` both
stamp `kayfabe-rev:e7268444bf77c68d0ae6e7590f4c9d4995b48162`. Three boots, one fresh QEMU
each, powered down between; harness `scripts/bench/e0_isolate_witness.sh`.

| run | `KAYFABE_ISOLATES` | rc | `R1 no-blocking-under-lock` in the QEMU log | isolate children | census |
|---|---|---|---|---|---|
| `r1real1` | `real` | **0** | **0** | 1, first sighting **t+42 s** | `1 materialized, 1 live, 0 refusing` |
| `r1real2` | `real` | **0** | **0** | 1, first sighting **t+43 s** | `1 materialized, 1 live, 0 refusing` |
| `r1ctl1` | *unset* | **0** | **0** | **0** | `1 materialized, 1 live, 1 refusing (1 no-plane)` |

Evidence: `bench_evidence/e726844_run_r1{real1,real2,ctl1}_{dmesg,qemu,probe,isolate}.log`.
Compare `bench_evidence/f0b7efa_run_basereal_qemu.log`, the same arm before the fix, where
the count is **1** and QEMU is gone.

#### 11.2.1 ★★ The pre-fix control, taken independently on a SECOND machine

`[measured]` rev **`dac9610`** (master immediately before this branch), vast **46494693** —
a *different* RTX 3060 from the acceptance runs above — host driver 580.159.04, same
harness, `KAYFABE_ISOLATES=real`. Evidence:
`bench_evidence/dac9610_run_masterreal_{qemu,isolate,probe}.log`.

The guest reached a login prompt at **t+28 s**, the module loaded cold, and QEMU **aborted
the moment the driver opened the device** — `crates/kayfabe-util/src/lockwitness.rs:125`,
`R1 no-blocking-under-lock violation … while holding rank(s) [0]`, then `panic in a function
that cannot unwind` and `thread caused non-unwinding panic. aborting.` The C frame below it
is `kayfabe_shim_regs_write ← nvkvm_trap_write ← memory_region_write_accessor`, i.e. an
ordinary **guest register write**.

Why this run exists at all: §11.2's before/after is otherwise one box's story, and
`f0b7efa` is a different revision from the branch's base. This pins the breach to **master
at `dac9610` on hardware the fix's author never touched**, so the pair is a genuine control
rather than a comparison across two variables at once.

⚠ **A harness trap the control exposed, and it is not fixed here.** `boot_capture.sh`'s
device-open step is a waiter with **no deadline**: when QEMU dies underneath it, the ssh
`open()` never returns and the script sits at *"opening the device"* indefinitely — it ran
~9 minutes before being killed by hand. The abort was in the QEMU log the whole time. Same
family as the `pgrep` trap in `bench_rebuild_notes.md`: **every waiter gets a deadline, and
a harness that hangs reports nothing while looking like it is still working.**

⊘ **The archive is `e726844`, the branch tip is later, and the difference is checked rather
than waved at** — §7's discipline. `git diff --name-only e726844 8b26763` outside `docs/` is
two files: `tests/tests/r1_spawn_outside_lock.rs` (a test target, **not linked into the
archive**) and `crates/kayfabe-fwd/src/lib.rs`, whose diff filtered of comment lines is
**zero lines**. So the binary these boots ran is this branch's content.

★ **E0b re-measured, not assumed:** the guest opened the device at **t+37 s** and the first
isolate child appeared at **t+42/43 s** — the spawn still *follows* the guest's action. The
deferral makes it strictly later, never earlier.

### 11.3 ★★★ The wall, and it is UNCHANGED — which is the honest headline

```text
NVRM: _memmgrMemUtilsScrubInitScheduleChannel: Unable to schedule channel, status: 56
NVRM: nvAssertFailedNoLog: Assertion failed: status == NV_OK @ ce_utils.c:304
NVRM: GPU 0000:00:03.0: RmInitAdapter failed! (0x25:0xffff:1249)
```

Three diffs, all `[measured]` at `e726844`, kernel timestamps stripped:

| pair | verdict |
|---|---|
| `r1real1` vs `r1ctl1` — guest dmesg | **IDENTICAL**, 26 lines / 22 `NVRM` / 3 adapter each |
| `r1real1` vs `r1real2` — guest dmesg | **IDENTICAL** |
| `r1ctl1` vs the committed control `147c069_run_e6join1_dmesg.log` | **IDENTICAL** |

⇒ **The control still behaves exactly as it does today**, and **the real plane is now
indistinguishable from it at this wall.** The device's own report differs between the arms
in **exactly one line** — the isolate census (`0 refusing` vs `1 refusing (1 no-plane)`)
plus the refusal sentence the stillborn plane prints. Every other counter is byte-identical:
`registers … 56309 writes`, `faults 0`, `interrupt requests dropped 91`, `framebuffer 122r
/55106w`, `BAR2 111r/21128w`, `commands: 92 decoded, 20 UNSERVICED, 17 distinct`,
`doorbells: 0 arrived`.

★ **Why unchanged is the expected answer and not a disappointment.** The isolate plane
serves *verbs*; the boot dies inside `RmInitAdapter`, long before a channel is scheduled, so
there is no guest submission for it to serve. E6 recorded the same shape (*"it could not
have moved"*). What this run buys is that **the plane can now be switched on at all** — the
prerequisite for every increment past this one — and the other half of
`suspect_the_instrument_first`: the change did not break the control either.

⊘ **What did NOT move and must not be read as having moved.** No `cuInit`, no doorbell, no
forwarded verb, no host RM ioctl caused by *guest intent* (the isolate's own bring-up is its
own, as `e0_isolate_witness.sh`'s docs already scope). `nvidia-smi` in the guest still finds
no device. The next wall is the same one §10.5 left: `status: 56` on the scrubber's CE
channel schedule.

⊘ **Against the committed control the READ counters move** (`3630`→`3646` register reads,
`gsp 341r`→`344r`, `timer 24`→`36`) while every write count is identical. That is the
`bench_rebuild_notes.md` §3 pattern — *diff the writes, tolerate the reads* — and it is
present between two runs of the **same** binary here too (`r1real1` 3630 vs `r1real2` 3638).

### 11.4 Suite, gates, ledger

- **Suite**, `[measured]` at `e726844` on the RTX 3060 bench (the KVM box), `KAYFABE_NO_KVM`
  unset: `cargo test --workspace --no-fail-fast` → **1986 passed, 0 failed, 1 ignored**;
  `KVM-GATE: RAN` **56**. ⇒ **+11 over `dac9610`'s 1975**, all of them
  `tests/tests/r1_spawn_outside_lock.rs`.
- **Gates**, `[measured]` at `e726844`: `./scripts/ci_gates.sh --all` →
  `ALL GATES CLEAN (22 steps, floor 22 for --all mode)`. No gated oracle family was added,
  so the floors are untouched.
- **Claim ledger**: **382 / 66 / 17** — unmoved. ⊘ It went red twice while this branch was
  written (384/382, then 383/382 when §11 was added) and was fixed by **attributing** every
  new site — each now names the run or the test that fails if it stops being true — never by
  moving the ceiling.
- **Bite-checks**: five, in `l1_concurrency.md` §12.47 §7. Four bite. ★★★ The fifth was a
  **non-biter on the first run and the finding was in the test, not the code** — the assert
  under test shares its panic message with `IsolateBox::drop`, so a `#[should_panic]` was
  being satisfied by the wrong door. Fixed and re-run: it bites.

## 12. E8 — BUILT. The PUBLISH phase §9.2 refused to invent, and what it closed

### 12.1 The gap, restated as the four links it broke

§9.2 measured that source 2 *"reaches a ROOT PAGE AND NO FURTHER"* and named the reason:
`classify_ce` produces a `PtWrite` only for a destination `Spine::pt_page_owner`
recognises, and that index held roots only. So a guest CE write into a **leaf** page table
was classified as ordinary data, forwarded, never witnessed, and every leaf under it stayed
`unwitnessed` — a MISS, a FAULT. The bytes decoded correctly and the metadata chain *was*
learned; only the binding was withheld.

§9.2 also named why it stopped there rather than patching it: *"`commit_pt_decode` holds
only `&mut Proc` (rank 1) and publishing into the spine (rank 0) from there inverts the lock
order. That is a phase-shape change of its own — an increment, not an edit."*

### 12.2 The shape — a FOURTH phase, and the ordering is the whole content

`SharedDevice::decode_pt_writes` was PLAN (rank 1) → EXECUTE (no lock) → COMMIT (rank 1).
It is now PLAN → EXECUTE → COMMIT → **PUBLISH (rank 0)**.

- `commit_pt_decode` *reports* what it learned (`PtDecodeOutcome::learned_pages`) instead of
  publishing it. It cannot publish: it holds rank 1.
- The shell takes the rank-0 write guard **after `with_proc_mut` has returned**, so its
  rank-1 guard is already dropped. That is an acquisition from nothing, not an inversion.
- **R5** lives in `Spine::publish_pt_pages`: every `(gpu, pdb)` is re-resolved against
  `by_pdb`, and a page whose address space died — or whose PDB value now belongs to a
  different proc — publishes nothing. `pages_published` is a separate counter from
  `meta_learned` precisely so that gap is visible rather than assumed away.

### 12.3 ★★★ The invariant that had to survive, and how

`Spine`'s routing maps are **derived from a projection, never accreted** — the comment sits
directly above `pt_roots.clear()`. A decode-learned page is not derivable from
`bounds.by_pdb`, so the naive move (accrete into `pt_roots`) would have quietly repealed
that rule for one map.

Instead `Spine::pt_learned` is a **projection of every live `Vas::pt_meta`**, recomputed
from scratch in `Spine::refresh` exactly as `pt_roots` is, and `publish_pt_pages` applies
**the same function early** — because a level learned in one pass must be recognisable
before the next RM graph event, and the guest does not wait for one.

⇒ publishing cannot disagree with the projection: it *is* the projection, run sooner. And
pruning stays automatic, which is the property the C artifact lacked — a `Vas` that dies
takes its pages with it, where the C's table was *"never pruned on handle free"*.

⚠ Bounded by `MAX_PT_LEARNED` (2^17), device-global, because the guest chooses both how
many page-table pages exist *and* how many address spaces to create — so the per-`Vas`
`MAX_PT_META` does not bound the device sum. Overflow **refuses and counts**; it never
evicts, since evicting a page the guest is still writing would silently return it to
"ordinary data" and unbind its leaves.

⊘ And a page already owned by a *different* `(proc, pdb)` is **refused, not re-homed**. Two
address spaces claiming one physical page-table page means either guest aliasing or a wrong
decode, and letting the last writer win is how the C's table came to attribute a page to
whoever touched it most recently.

### 12.4 Acceptance — the second write, and its non-vacuity

`tests/tests/e5_address_table_join.rs::a_ce_write_into_a_learned_leaf_table_is_witnessed_and_binds_its_leaf`:

1. Pass 1 — the guest writes the **root**; the subtree's metadata is learned; nothing binds.
2. ⊘ **Non-vacuity**: a ring naming the **leaf table** yields **zero** page-table writes.
3. PUBLISH — 4 pages, 0 refused.
4. The **identical ring, byte for byte**, now yields **one** page-table write, attributed to
   `A_PDB` — which is not the issuing channel's proc in general, and is why the index is
   device-global.
5. Pass 2 — the leaf is witnessed, `bound == 1`, and `leaf_va` resolves to the physical
   address the guest's own PTE names, at offset zero.

Wiring is asserted separately, in `pt_decode.rs`'s shell test, because "the function works"
and "the shell calls it" are different claims: `learned_pages == published == 4`, and
`SharedDevice::pt_page_owner` answers `(pid, A_PDB)` for each.

### 12.5 Bites — I broke both halves and both went red

★ Both halves of E8 are load-bearing, and that is `[measured]` rather than asserted:
2026-08-04 on branch `e8-pt-index` (off master `b63b5c5`), by editing
`Spine::publish_pt_pages` / `Spine::pt_page_owner` in place and re-running
`cargo test --no-fail-fast -p kayfabe-tests --test e5_address_table_join --test pt_decode`.
The two tests are
`a_ce_write_into_a_learned_leaf_table_is_witnessed_and_binds_its_leaf` and
`the_pass_runs_through_the_shell_in_both_lock_modes_with_the_blocking_phase_unlocked`;
whole-suite baseline for the same run was 2008 passed / 0 failed.

| mutation | e5 join | pt_decode shell |
|---|---|---|
| `publish_pt_pages` returns without publishing | **RED** | **RED** |
| `pt_page_owner` consults `pt_roots` only | **RED** | **RED** |

Run with `--no-fail-fast`: cargo stops after the first failing *target*, which manufactures
false non-biters (`bite_check_needs_no_fail_fast`).

### 12.6 ⊘ What E8 does NOT establish

- **No boot has been spent on it.** Everything above is the suite; `only_live_boots_are_proof`.
  The guest boot still stops at `mem_utils.c:2006` on `0xa06f0103`, which is the execution
  plane and is untouched by this increment.
- **The publish is driven only from `decode_pt_writes`.** Any future decode caller must add
  its own PUBLISH; nothing in the type system forces it, and that is a real residue.
- ★ **The absence test did not go red, and that was a finding about the test.**
  `the_ce_pt_write_source_can_witness_only_a_root_page_today` was written to fail the day
  this stage landed. It passed. Its assertion is about a *single* pass and remained true;
  the claim E8 falsified lived only in its prose. A test whose message is broader than its
  assertion cannot detect the thing its message is about — recorded in the test itself
  rather than tidied away.

### 12.7 ★★★ E8 v1 was REVIEWED AND FOUND WANTING — five mutations, none caught

`[measured]` 2026-08-05, branch `e8-refusals`. An adversarial review of `9f55716` planted
five mutations and the suite caught **zero**. I re-ran its headline mutation myself and
confirmed. §12.5 above claimed both halves of E8 were load-bearing and verified — that was
true of the *mechanism* and false of the *guards*.

**Why my own bite-check missed it.** My two mutations removed the FEATURE (`publish_pt_pages`
a no-op; `pt_page_owner` ignoring the map). The acceptance test exercises the feature, so
they bit. The review's five removed the REFUSALS — re-home on conflict, disable the R5
re-resolve, disable the ceiling, flip the projection to last-writer, delete the rebuild —
and every one of those is something §12.3 called load-bearing. The tests asserted
`(published, refused) == (4, 0)`: `refused` pinned at **zero**, so no test ever watched a
refusal happen. ⊘ *A guard nothing is ever seen to refuse is not evidence.*

#### The real defect it exposed: TWO conflict policies for one question

| path | rule | visible |
|---|---|---|
| `publish_pt_pages` (v1) | refuse the **second arrival**, keep the incumbent | counted |
| `refresh` (v1) | `entry().or_insert()` — keep the **lowest `ProcId`** | silent |

So `pt_page_owner(P)` answered one proc before a rebuild and another after, and the
"REFUSED, not re-homed" property — §12.3's stated fix for the C's last-writer-wins
attribution — was undone by the projection, silently and uncounted. The invariant
*"publishing cannot disagree with the projection"* was **false**, and the test §12.3 cited
for it (`pt_index_publish_equals_projection`) **did not exist**: the string occurred once
in the tree, in the comment claiming it.

#### The correction: both paths DECLINE

Neither "first" rule is implementable on both sides — publish cannot know a future
claimant, the projection has no arrival order. So a page claimed by two `(proc, pdb)` pairs
is indexed for **nobody** ([`Spine::pt_contested`]), sticky across publishes and re-derived
from scratch by `refresh`. `pt_page_owner` returns `None`, `classify_ce` forwards writes to
it as ordinary data, and its leaves do not bind — a loud miss instead of a wrong owner.

`tests/tests/pt_index_projection.rs` now drives every refusal: the equality itself (with a
genuinely contested page, and a non-vacuity assertion that the contest occurred), the
decline, stickiness, R5 against both a mis-routed and an unrouted PDB, the ceiling
refusing-and-counting without evicting, and the projection pruning a page whose metadata is
gone. **All five review mutations now bite** (`--no-fail-fast`; cargo stops after the first
failing target otherwise).

Also fixed: `refresh` no longer increments `pt_learned_refused`. It re-derives pages already
counted when first offered, so counting again made the diagnostic grow on every RM graph
event and stop meaning "publications turned away".

#### ⊘ What this is, and what it is NOT — ★ OWNER CORRECTION 2026-08-05

An earlier draft of this section called the residue a security question: *"a process able to
forge a PDE at another proc's page-table page"*. **That threat does not exist, and the owner
named why.**

Unprivileged guest userspace **cannot author a PDE**. GMMU page tables are built by
`nvidia.ko`, which already holds legitimate access to every address space in the guest — the
same reason a Linux process cannot edit its own PTEs without going through `mmap`. The same
answer disposes of the "authority" reframe: when an RM command carries a physical address,
that address was chosen by the guest kernel module, so validating it against cross-process
access is validating the guest kernel against itself.

★ **The threat-model error underneath it**: procs A and B are both processes *inside one
guest VM*. Guest-internal isolation is the guest kernel's job — which is equally true on
bare metal, where a real GPU does not protect one process from another; the driver does.
This port's boundary is **guest → host escape**, and no step of that scenario crosses it. A
compromised guest kernel loses A-vs-B on real hardware too, and that is not ours to prevent.

⊘ Checked, because it is the one thing that would change the answer: guest **userspace** does
author pushbuffer content, but those methods name **VAs**, not physical addresses —
`[measured]` 2026-08-02 at revs `81a1f45` (`#170`) and `49befb7` (`#171`), where VA ≠ GPA was
established by a RAM differential because a single boot read green — and a VA resolves only
against its own address
space, so B's VAs are *not found* in A's rather than *denied*. The physical-destination CE
path belongs to the kernel's own CeUtils channel.

⇒ **What the contested-page decline actually buys is ROBUSTNESS, not a boundary.** If our own
decode is wrong, or a buggy guest kernel does something unexpected, a contested page produces
a loud miss instead of a silent wrong binding. That is diagnosability and it is worth having.
It is **not** a security control, it does **not** need an owner ruling, and it is **not** the
same question as the chid namespace — pairing them was the same over-reach.
## 13. E9 — the execution-plane set: a JOIN across seams that already exist

### 13.1 The set, and why it is one requirement

`kayfabe_device::sweep`'s `0xa06f0104` row already names it: `0xa06f0103` (schedule),
`0xa06f0104` (bind), `0xc36f0108` (work-submit token) and the notifier-35 arming at
`mem_utils.c:1920` *"are not four rungs of a ladder; they are ONE requirement — put a
channel on a runlist, arm its completion, hand back its doorbell — asked four times."*
`[measured]` 2026-08-01, boot `evtprobe1` at rev `4e93f17` with a throwaway probe that
faked three of them: `mem_utils.c:2022` cleared and the boot reached the fourth.

### 13.2 ★ Most of it is already built, and that is worth stating plainly

The verbs the set needs exist on `kayfabe_isolate::RmBackend` and have run on real
hardware (`#113`, `3b2597c` — a real CE moved device memory through them):

| control | what it needs | state |
|---|---|---|
| `0xc36f0108` token | a real host channel's token | `alloc_channel` **returns** `(handle, host_work_submit_token)` |
| `0xa06f0103` schedule | an act on that channel | `RmBackend::schedule(chan)` exists |
| `0xa06f0104` bind | engine → runlist | `alloc_channel` takes `engine: EngineKind` (GR-1's fix: an engine-blind alloc is the C's wrong-runlist bug) |
| notifier 35 arming | a `SILENT_NOTIFIERS` row | argument written and cited in `sweep.rs`, deliberately **not taken** because taking it alone moves nothing |

⇒ E9 is a **join across existing seams**, not a new mechanism.

### 13.3 ★★★ SETTLED — the guest picks its own ChID, so translation is FORCED

⚠ **An earlier draft of this section was wrong and is struck.** It framed a decision:
*"does the guest get a token encoding its own vChid, or the host channel's verbatim?"* and
called it *"the one question with a silent wrong answer"*. There was never a choice. The
owner named the mechanism and the C confirms it in one line.

**`[src]` The guest kernel allocates the ChID, before anything reaches us.**
`kchannelAllocHwID_GM107` runs in the guest's own CPU-RM and calls
`kfifoChidMgrAllocChid` against an eheap the guest owns (`ogkm-580:
src/nvidia/src/kernel/gpu/fifo/arch/maxwell/kernel_channel_gm107.c:480`,
`kernel_fifo.c:569`). It then **smuggles the already-decided ChID to us through the alloc
flags**, because `NV_CHANNEL_ALLOC_PARAMS` has no chid field:

```
chid = flags[20:12] * 8 + flags[10:8]          (USERD_INDEX_PAGE_VALUE, USERD_INDEX_VALUE)
```

`C: src/qemu/nvkvm_gpu_emul.c:2914` — *"A GSP-client CPU-RM encodes its already-decided
ChID into USERD_INDEX so the physical RMAPI reuses it… doorbell `token[11:0]` == this
vChid; it's the demux key."* And `numChannelsPerUserd = 1 << DRF_SIZE(USERD_INDEX_VALUE)`
= 8, which is where the `* 8` comes from.

⇒ **We are told a chid; we do not choose one.** The host driver will independently pick its
own for the real channel. A guest vChid is meaningless on the host, so a guest token can
**never** be forwarded to the host directly — it always translates.

### 13.3.1 ★ And this is WHY the doorbell page is trapped at all

The trap is not a design preference, it is the only route. The guest writes
`token[11:0] = its own chid` into the doorbell register; the host channel carries a
different chid; nothing else in the flow sees both numbers. ⊘ Official vGPU does not have
this problem because a legacy vGPU capability let the GPU select from available vChids —
and that capability is **inaccessible** on the parts this project targets.

⇒ the shape is fixed: guest allocates a vChid and tells us → we ask the host driver for a
channel and it returns **its own** chid and token
(`RmBackend::alloc_channel` → `(HostHandle, u64)`) → we keep the pair → the doorbell trap
maps guest vChid to host token. `Ga10xArch::decode_doorbell` (E3) is what reads the guest's
written token back to a vChid, which is exactly the lookup this needs.

⊘ **The C did NOT do this, and that is not an argument against it.** The C demuxed by
walking every channel's pending GPFIFO on each doorbell (`C: nvkvm_gpu_emul.c:242-243`) —
correct, and O(n) per doorbell with n guest-driven. E3's decoder makes the same demux O(1).
The C's approach is a fallback that stays available, not the design.

### 13.4 ⊘ What is deliberately NOT built yet

The join itself: the guest-vChid → host-(handle, token) map, the token reply, the schedule
and bind acts, and the notifier-35 row. Nothing above is blocked on a decision any more —
what remains is the build. ⊘ No boot has been spent on any of it, so every claim this
section makes about the *join* is source-derived until one is
(`only_live_boots_are_proof`).

### 13.5 ★★★ The four are NOT homogeneous — two are replies, two are HOST ACTS

`[src]` 2026-08-05 at `ff59d23`. §13.1 calls the set *"one requirement asked four times"*,
which is right about the *requirement* and wrong about the *implementation*. Surveying before
building found the four split cleanly, and the split is what the remaining work is shaped by.

| control | kind | servable by today's path? |
|---|---|---|
| `0xc36f0108` work-submit token | **reply** — a pure function of `(runlist, guest vChid)` | ✔ **on the `ObjectPolicy` path** (not the init-table one) — see §13.5.2 |
| notifier-35 arming | **reply** — a `SILENT_NOTIFIERS` row; `sweep.rs:325-335` already wrote and cited the argument (*the arming is never read*) | ✔ (but see below: it must not be taken alone) |
| `0xa06f0103` `GPFIFO_SCHEDULE` | **host act** — must put a real host channel on a real runlist | ~~✘~~ ★ **SERVED since `#177`** — act deferred to the first doorbell, see §13.5.2 |
| `0xa06f0104` `BIND` | **host act** — engine → runlist on a real host channel | ✘ |

### 13.5.2 ⊘⊘ TWICE-CORRECTED — §13.5.1 IS STALE (`0xa06f0103` IS SERVED), and my first correction was ALSO WRONG

`[src]` 2026-08-06, and both errors are left standing rather than deleted because the pair is
the useful part.

**(1) `0xa06f0103` `GPFIFO_SCHEDULE` has been SERVED since `#177`.** §13.5's ✘ and §13.5.1's
closing *"⊘ Not started"* are stale. The tree contains
`OBJECT_CONTROLS = &[NVA06F_CTRL_CMD_GPFIFO_SCHEDULE]` (`policy.rs:851`),
`SharedDevice::schedule_channel` (`kayfabe-rt/src/device.rs:1736-1764`, its own rustdoc headed
*"★★★ #177"*), `ExecPlane::{requested, scheduled}` (`kayfabe-core/src/gpu.rs:340-424`), a
570-line `tests/tests/gpfifo_schedule.rs` and a 10-mutation `scripts/bite_gpfifo_schedule.py`.
`sweep.rs:725` says *"★★★ SERVED as of #177"* in as many words.

★ **And it landed in a DIFFERENT SHAPE than §13.5.1 proposed.** It does **not** put
PLAN→EXECUTE→COMMIT in the control path. The control records *intent* under rank 1 and replies
`NV_OK`; the real host act happens at the **first doorbell**, on the verb path that already
existed (`plan_doorbell` sets `schedule = !exec.scheduled.contains(&cid)`,
`kayfabe-fwd/src/lib.rs:1614-1623`; `Worker::execute`'s `Doorbell` arm runs `alloc_channel` +
`schedule` lock-free; `commit_doorbell` moves it to `scheduled`). The licence is written at
`sweep.rs:725-761`: between the control returning and the first doorbell, *"on the runlist now"*
and *"on the runlist by the first submission"* are **observationally indistinguishable to the
guest** — and the C, the only implementation a real driver ever accepted, does exactly this.

**(2) ⊘ My own correction, written earlier the same day, was wrong.** It said *"every reply
policy answers through `fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply>`… That
signature is the whole context: a command, and the policy's own fields. No device, no `Proc`,
no `Channel`, no `Worker."* The first half is true and the conclusion does not follow.
**Context arrives through `self`, not through the arguments.** `ObjectPolicy` holds
`gpu: Box<dyn ObjectModel>` (`policy.rs:813-819`) — a *port to the object model*, which in the
shell is `SharedObjectModel` over `SharedDevice`. That is precisely how `#177` reaches per-`Proc`
channel state from inside a `respond`.

⇒ **There is no "plumbing A".** The route from a control to a channel exists and is in use. What
made `InitTablePolicy` unable to serve the token is not the trait — it is that *that* policy
deliberately holds no handle-keyed state (`inittables.rs:173-191`), which remains correct and
must not be widened. The token belongs on the `ObjectPolicy` path, beside `#177`, not on the
init-table path.

★★ **What actually remains, then:**

| item | state |
|---|---|
| `0xa06f0103` `GPFIFO_SCHEDULE` | **SERVED** (`#177`), act deferred to first doorbell |
| `0xa06f0104` `BIND` | **not served** — one `OBJECT_CONTROLS` entry, one `respond_control` arm, one core route/apply pair, following `#177`'s landed idiom |
| `0xc36f0108` token | belongs on the `ObjectPolicy` path; the encoder exists (`ff59d23`) |
| notifier-35 | unchanged — `sweep.rs:325-335`'s cited reason to wait still holds |

⚠ **The mistake I made, named so it is not made again.** I read a trait *method signature*,
found no context in its arguments, and concluded the capability was absent — without reading
what the implementations hold in `self`. That is the same species as
`a_table_does_not_decide_behaviour`: the signature is not the dispatch, and it is the second
time in this campaign that **absence of the obvious carrier was read as absence of the fact**
(the first was inferring the guest could not tell us a ChID because `NV_CHANNEL_ALLOC_PARAMS`
has no chid field — it tells us through `USERD_INDEX`).

⚠ Still unmeasured: everything above is a reading of the tree. No boot has exercised `#177`.

### 13.5.1 ⊘ Why the two acts cannot use the path the other twenty-odd controls used

`InitTablePolicy` — the whole control-serving surface — **has no `Worker`**, and therefore no
route to the isolate. Every control this port has served so far is a reply computed from state
it already holds. The one previously described as *"an ACTION, not a description"*
(`0x20800a6c` `MEMSYS_L2_INVALIDATE_EVICT`, `#148`) is not a counter-example: its licence is
explicitly that it is *"a verb on hardware this device does not have"* — served **vacuously**,
by arguing nothing needed to happen. `0xa06f0103` and `0xa06f0104` have no such argument
available: something must happen on a real host channel or the guest's ring never runs.

⇒ serving them is a **phase-shape change**, the same species §9.2 named for E8 and E8 built:
the blocking host call cannot happen under a ranked lock, so the control path needs
PLAN (rank 1) → EXECUTE (no lock, via a `Worker`) → COMMIT (rank 1, R5 re-validate). That is
an increment with its own acceptance, not a table row.

★ Everything *beneath* it already exists and is wired: `Channel` carries `vchid`,
`host_channel: Option<HostHandle>` and `host_token: Option<u64>`; those are written at
`kayfabe-fwd/src/lib.rs:1743-1744` and `:2121-2122` and read at `:1602`, `:1748`, `:1757`.
`RmBackend::schedule(chan)` exists. The guest-vChid → host-channel map is **not** missing —
what is missing is a control that can reach it.

⊘ ~~**Not started.**~~ ★★ **STALE — read §13.5.2 first.** `0xa06f0103` shipped in `#177` and
deliberately did **not** take the phase-shape change this section proposes: the act moved to the
first doorbell, where the verb path and its `Worker` already are. This section is kept because
its *analysis* of why a control cannot block is correct and still governs — but its conclusion
that a control path needs PLAN/EXECUTE/COMMIT was overtaken by a cheaper shape.

### 13.6 `0xa06f0104` BIND — the ABI landed, and the refusal is BLOCKED on a routing fact

`[measured]` 2026-08-07, `1c47834` + `dac6484`. The ABI half of the bind is on master and
green: the command id, `BindParams`, the decode/encode, the status set, and
`nv2080_to_rm_engine_type`. What is **not** built is the policy arm, and the reason is a
finding rather than a shortage of time.

#### The refusal this rung is for

Per `ogkm-580: kernel_fifo_gm107.c:672-759`, a real GSP answers a bind by **linear-scanning
this GPU's own engine-info list** — the list built from the device-info table *we serve* — and
returns `NV_ERR_OBJECT_NOT_FOUND` (`:736`) when nothing matches. So the faithful refusal is
exactly *"an engine we never advertised"*. It is not invented, and it is not the
channel-shaped `NV_ERR_INVALID_STATE`.

⊘ And it must not be *"the engine disagrees with the channel's alloc engine"*. Nothing in the
bind path checks that (`kchannelCtrlCmdBind_IMPL` only calls
`gpuXlateClientEngineIdToEngDesc`, against a **static, chip-independent** table that `pGpu` is
passed to and never read from — `ogkm-580: gpu.c:5274-5295`). Adding it would be
`mock_fidelity_both_directions`' too-strict half.

#### ★★★ THE BLOCKER: the engine set is a per-CHIP fact with no route into the object plane

The check needs to ask *"is `rm_engine_type` in the set this device advertised?"*. Following
that question through the tree, `[measured]` at `dac6484`:

| holder | has the engine set? | is on the bind path? |
|---|---|---|
| `kayfabe_device`'s `ChipModel.engines` (`GA106_ENGINES`, `ga10x.rs:616`) | ★ **yes — the single description** | no |
| `kayfabe_arch::Arch` | no | yes (`Gpu` holds one) |
| `kayfabe_core::Gpu` | no | yes |
| `kayfabe_rt::SharedDevice` | no | yes (via `SharedObjectModel`) |
| `kayfabe_rmrpc::ObjectPolicy` | no | yes |

⇒ **Every type on the bind path lacks the fact, and the type that has it is not on the path.**

⚠ The tempting fix — put the engine set on `Arch` — is wrong twice over. It is a **per-chip**
fact, not a per-generation one (GA102 and GA106 differ in CE count while sharing `Ga10xArch`),
and it would be a *second* hand-written description of one silicon, which
`inittables.rs:862-865` forbids in as many words: the `InternalDeviceInfo` arm is *"the only
arm that is a **projection** rather than a statement … Two hand-written descriptions of one
silicon is the drift `kayfabe_abi::deviceinfo` exists to forbid."*

#### The three shapes, and the recommendation

1. **`ObjectModel` gains the query; the shell answers it, a bare `Gpu` declares nothing.**
   Both impls exist (`Gpu` in `policy.rs:310`, `SharedObjectModel` in `shim.rs:1325`). ⚠ But
   `SharedDevice` does **not** hold the chip either, so this needs the chip routed into
   `kayfabe-rt` first — the work is not one method, it is a new edge.
2. **`ObjectPolicy` takes the engine slice at construction.** `kayfabe-rmrpc` already depends
   on `kayfabe-abi`, so it can name `&'static [FifoDeviceEntry]` with no new dependency, and
   the shell already holds both halves. Cost: **24 construction sites** across 5 files,
   including `kayfabe-qemu-raw/src/shim.rs` (raw-crate, ratchet territory).
   ⊘ A defaulted `Option` is **not** available here: `None` would have to mean either "refuse
   every bind" (too strict, and it would break every mock-composed test silently) or "accept
   every bind" — and the second is the `sandbox_unsafe::last_capability` fail-open shape
   already recorded as undischarged residue. A gate whose default is open is not a gate.
3. **Serve BIND from a device-side policy that already holds `chip`.** Cheapest, and wrong:
   it splits the engine check from the channel routing across two policies, so neither can
   answer the whole control.

★ **Recommendation: (2).** It puts the check next to the only description of the silicon, it
is fail-closed by construction (the slice is required, so no composition can forget it), and
the 24 edits are mechanical. (1) is (2) plus an extra hop and a new crate edge for no gain.

⚠ **Held for the owner, not started.** The reason is the owner's own standing ruling that
*arch seams get built at the time the code is written rather than retrofitted*: whichever of
these lands is the shape every future per-chip capability question inherits (how many CEs,
which video engines, MIG partitioning), so it is worth one decision rather than one
precedent set at 02:30 by whoever was holding the keyboard.

### 13.7 ★★★ boot `m1` — the wall, and the instrument that cannot see it

`[measured]` boot `m1`, 2026-08-07, master `809b040` (revision read out of the QEMU binary
with `strings`), vast instance `47029542` (RTX 3060 / GA106), stock unpatched NVIDIA
580.159.04 open kernel module in the guest. Every line below is quoted from
`docs/reference/bench_evidence/m1_809b040_{dmesg,probe,qemu,serial}.log`, committed beside
this file because a boot log that lives only on a rented box is not evidence.

| | |
|---|---|
| box | vast `47029542`, RTX 3060 (**GA106**, the traces' own chip), 38 cores / 198 GB, `/dev/kvm` present |
| host driver | 580.159.04 **Open Kernel Module**, `Dual MIT/GPL`, all three nodes `open()` clean |
| guest | Ubuntu 24.04, kernel 6.8.0-136, **stock unpatched** 580.159.04 open module |
| revision | `809b040`, verified by `strings` on the QEMU binary → `kayfabe-rev:809b0400ab0bc…` |
| evidence | `docs/reference/bench_evidence/m1_809b040_{dmesg,probe,qemu,serial}.log` |
| blank box → captured boot | **17 minutes** |

**The fatal chain, in the guest's own words:**

```
mem_utils.c:2022   _memmgrMemUtilsScrubInitRegisterCallback: event notification control failed
                   → NV_ERR_GENERIC (0x0000FFFF)
mem_utils_gm107.c:1027 → ce_utils.c:304 → mem_scrub.c:181
mem_mgr_scrub_gp100.c:63 → mem_mgr.c:487 → kernel_fifo.c:3129
RmInitNvDevice: *** Cannot load state into the device
RmInitAdapter failed! (0x25:0xffff:1249)
```

★ **The boot HAS advanced since `419afe8`**: that revision died earlier, at
`osVerifySystemEnvironment` / `0x11:0x45:2134` (`NV_ERR_IRQ_NOT_FIRING`). That wall is gone.
⊘ **And it has not advanced since `ff5278d` (2026-08-03).** Four days, 32 commits, same line.

#### ★★★ THE LEDGER IS BLIND TO THE WALL, AND THIS BOOT PROVES IT

The device reports **19 unserviced commands, 16 distinct**, and here is the whole set:

```
0x2080017e 0x20800a2c 0x20800a2e 0x20800a30 0x20800a34 0x20800a38 0x20800a3f 0x20800a4b
0x20800a70 0x20800a80 0x20800a87 0x20800afe 0x20800aff 0x20800b03 0x20800b05 0x20802a0f
```

`0x20800301` — `NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION`, the control whose failure is printed
in the fatal line above — **appears 0 times.** It is *served-but-refused*, so
`InitTablePolicy::refuse()` returns `Some(Reply)` and the ledger never records it.

⇒ **The port's primary rung-picking instrument cannot see the class of wall the port is stuck
on.** Anyone choosing the next rung by diffing ledgers will pick from those 16 and none of them
is the blocker. That is not a metaphor for how four days went into `0xa06f0104`; it is the
mechanism.

#### What this settles

- ⊘ **E9's `0xa06f0104` is not the next rung.** 0 occurrences in 4 logs. §13.6's engine-set
  routing fork is **moot until the guest issues the control**, and is withdrawn from the
  owner's desk rather than answered.
- ★ **The next rung is `EVENT_SET_NOTIFICATION` index 35** — named by the guest, at the line
  that kills it.
- ★★ **Fix the ledger before using it again**: a served-but-refused reply must be recorded, or
  the ledger must lose its role in rung selection. A gate that is structurally blind to the
  failures you have is worse than no gate, because it answers.

## 14. E10 — the CPU branch of the CE decision tree. The `memmgrTestCeUtils` wall, dissected

★ Status 2026-08-07: **DESIGN + PREREQUISITE MEASURED.**
Written before any edit because the increment spans six crates and turns on a design fork
(where the CPU branch *executes*) that must be settled first. Every `[src]` below is a file
this survey read; nothing here is `[measured]` on hardware.

★★ Status 2026-08-08 — **E10a–E10d LANDED and tested (GPU-free); E10e + the shim data-path
wiring + the boot REMAIN.** The pure/shell layers of the CPU branch are built and pinned:
- **E10a** (`9c343d5`) — the phys-mode `PhysTarget` decode.
- **E10b** (`c2c3d28`) — the residency split: `PhysTarget`/aperture → `kayfabe_arch::CpuPlane`
  `{Fb, GuestRam}`, carried per-operand on `CeSpan`; `_PEERMEM` refused by name
  (`FwdFault::CePeerOperand`). This is why E10a exists — a raw physical address is ambiguous
  (an FB offset and a guest GPA collide numerically); the `_TARGET` disambiguates and now
  reaches the executor. Pinned in `ce_representability_split.rs` (numeric-collision,
  target-swap, Peer-refusal, fabricated-virtual-aperture), bite-checked.
- **E10c** (`749ef32`) — `kayfabe_rt::cpu_ce_unsafe::{execute_ours, execute_ours_spans}`, the
  shell CPU executor over `FbStore` (`SparseFb`) + `Vmm`. `memmgrTestCeUtils`' `sys ← vid`
  readback compare is reproduced GPU-free in `cpu_ce_executor.rs`. `FwdFault::CpuCeStraddle`
  refuses an `Ours` span whose needed plane is `None` by name.
- **E10d** (`c181d1d`) — `kayfabe_rt::cpu_ce::write_completion`: the finishPayload written to
  the resolved physical of the guest's own semaphore VA (the channel's own aperture — the #12
  where-mistake), one-word (4-byte), interrupt raised **only after** every write lands and
  **not at all** on a refusal.
- **E10c fix** (`33dac8f`) — the executor module `cpu_ce_unsafe.rs` was renamed to `cpu_ce.rs`:
  it accesses guest memory only through the `Vmm`/`FbStore` traits (bounded copies, the raw
  side re-validates), so it carries no unsound surface and no `_unsafe` suffix (§4.1 gate B).

★ **What E10e still owes for the boot** `[src]` (the doorbell for the CeUtils channel is still
refused `NoVas(ChanId(1))` at `bench_evidence/run_probe35_349924b_qemu.log`, so none of E10b–d
is on the live data path yet). Scope read off the tree by the 2026-08-08 E10e survey (§14.7):
(1) `plan_doorbell` / `read_pushbuffer` accept the physical-operand channel and translate its
still-virtual ring/semaphore — but the channel's `FERMI_VASPACE_A` page-directory root never
reaches the core (it arrives only via `0x90f10106`, unclaimed by `ObjectPolicy` and
`PageDirNotModelled` in the ABI), so no `AddressTable` materializes and the ring VA
`0x1_2006_4000` is untranslatable — a deep gap, not a link; (2) the shim must drive
`parse_pushbuffer → execute_ours_spans → write_completion` off the doorbell with a
`&mut dyn Vmm` and the `SparseFb` (today it rings with an empty working set and never parses).
No rung is claimed until a boot shows it.

### 14.1 The wall, in the guest's own source

`[measured]` boot `run_subdev_probe35_925b27b` at `0e2f3ae` dies at
`bench_evidence/run_subdev_probe35_925b27b_dmesg.log`:

```
NVRM: Assertion failed: Call timed out [NV_ERR_TIMEOUT] (0x65) returned from
      memmgrMemSet(pMemoryManager, &vidSurface, 0, sizeof vidmemData, TRANSFER_FLAGS_PREFER_CE) @ mem_mgr.c:463
NVRM: memmgrInitCeUtils(...) @ mem_mgr.c:526 → RmInitAdapter failed! (0x25:0x65:1249)
```

`[src]` `ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/mem_mgr.c:408-478` (`memmgrTestCeUtils`):
the guest allocates a **vidmem** (`ADDR_FBMEM`) surface and a **sysmem** surface, then:
`memmgrMemSet(vid, 0, CE)` → `memmgrMemWrite(vid, 0xAABBCCDD, CPU)` →
`memmgrMemWrite(sys, 0x11223345, CPU)` → `memmgrMemCopy(sys ← vid, CE)` →
`memmgrMemRead(sys, CPU)` → `NV_ASSERT_TRUE(sysmemData == vidmemData)`. The `MemSet` never
retires, so its finishPayload semaphore never advances and the guest times out at the first
CE op. **The guest's own readback compare is the acceptance oracle** — no forged completion
can satisfy it, because `memmgrMemRead` reads the real bytes the copy did or did not move.

### 14.2 The CPU branch CAN serve it — but only in the SHELL, not the isolate

Under the owner's CE decision tree, both operands are CPU-reachable:
- the `MemSet` destination is `ADDR_FBMEM` = our **emulated framebuffer**, which lives in the
  shell as `kayfabe_device::fbwin::SparseFb` (`plane.rs:1725` `fb_write`, installed
  `kayfabe-qemu-raw/src/shim.rs:1664`) — a sparse `BTreeMap` byte store, CPU-reachable;
- the `MemCopy` destination is `ADDR_SYSMEM` = **guest RAM**, reachable via
  `kayfabe_vmm::Vmm::gpa_read`/`gpa_write` (`kayfabe-vmm/src/lib.rs:754`/`:762`), which the
  shell already holds.

★★★ **The design fork this increment settles.** §12.4 states *"the executor is the isolate in
both cases."* That is **false for the CPU branch of the kernel-originated CE.** The isolate is
a separate sandboxed process; it has **neither** `SparseFb` **nor** `Vmm`. Its
`RmBackend::ce_copy(Ours)` therefore refuses `NOT_ON_THIS_RUNG`
(`kayfabe-isolate-host/src/rm.rs:2935-2937`) and *could not do otherwise* — the doc at
`rm.rs:2374-2398` already says the isolate's fabricated-aperture mapping "needs a GPU to run
it against." ⇒ **the CPU branch executes in the shell**, against the two memory planes the
shell owns. This does not contradict the owner's ruling — "whoever does the work releases the
semaphore" — it *locates* the "whoever" by where the bytes live, which is exactly the ruling's
own test. The isolate keeps the `HostCe` branch (operands in real device memory) unchanged.

### 14.3 The residency question is unanswerable today — the phys-mode target is not decoded

⚠ `Representability::executor` (`kayfabe-fwd/src/lib.rs:2939-2945`) keys on
`binding.host.is_some()` — a proxy for residency, not residency. A fabricated FB page (binding,
no host) falls to `Fabricated → Ours → NOT_ON_THIS_RUNG`; a sysmem binding with no host mapping
falls the same way. Neither answer is "this is CPU-reachable, serve it in the shell."

★ And for a **physical** CE operand — which is what CeUtils issues — the residency signal is
carried by a method the GA10x codec **does not decode**. `[src]`
`ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/channel_utils.c:629-642, 1069-1082`: the CeUtils
pushbuffer emits `SET_DST_PHYS_MODE`/`SET_SRC_PHYS_MODE` with
`_TARGET_{LOCAL_FB, COHERENT_SYSMEM, NONCOHERENT_SYSMEM}` from
`memdescGetAddressSpace(pMemDesc)`. `Ga10xPushbuffer::ce_launch`
(`kayfabe-chips/src/ga10x.rs:1088-1114`) reads only the `LAUNCH_{DST,SRC}_PHYSICAL` bit
(virtual vs physical) — it never latches the target, so FB-physical and sysmem-physical
operands are **indistinguishable** in-tree, and their addresses can even collide numerically
(the emulated FB aperture and guest GPAs are different number spaces). The target IS the
disambiguator, and it is the fact `classify_ce`/`Representability` needs.

`[src]` offsets/values, stable across `NVB0B5`/`NVC7B5` (`clc7b5.h:66-80`, `clb0b5.h:56-65`):
`SET_SRC_PHYS_MODE = 0x260`, `SET_DST_PHYS_MODE = 0x264`, `TARGET` = bits `1:0`,
`LOCAL_FB=0 / COHERENT_SYSMEM=1 / NONCOHERENT_SYSMEM=2 / PEERMEM=3`; register reset = 0 =
`LOCAL_FB`.

### 14.4 The three unwired stops (all `[src]`-confirmed) between here and the wall moving

1. **The guest-kernel ring is never parsed.** `SharedDevice::{parse_pushbuffer, forward_ce,
   submit_ring}` (`kayfabe-rt/src/device.rs:1437/1497/1535`) have **no production caller**; the
   shim's doorbell path rings the host doorbell with an **empty working set** and never parses
   (`kayfabe-qemu-raw/src/shim.rs:1490-1495`). The CeUtils channel (`Gpu::SYSTEM_PROC`, routed
   at `gpu.rs:3306`) reaches `plan_doorbell` and is refused `NoVas(ChanId(1))` — it is a
   **physical-mode** channel with `vas_pdb == None`, and `plan_doorbell` requires a VAS
   (`lib.rs:1593-1609`). A physical CE copy needs no VAS; this refusal is the first stop.
2. **`CeExecutor::Ours` / `CeSource::Constant` refuse on the real backend** (`rm.rs:2935-2940`),
   and cannot be served there at all (14.2). They must be diverted to the shell before the
   isolate.
3. **There is no completion write-back tail.** `PushbufferOutcome::sem_releases`
   (`lib.rs:2546`) is consumed by no crate; `SemRelease` is hashed into an opaque `OsEventRef`
   and the address is discarded (`lib.rs:3513-3518`); the delivery tail
   (`encode → gpa_write → IRQ`) is a comment, wired only in tests
   (`kayfabe-rt/src/executor.rs:227`). The finishPayload semaphore the guest polls is written
   **nowhere**. This is why the guest times out even when a copy could be performed.

### 14.5 The increment, as ordered edits

Following the owner's tree (translate → residency → executor → signal truthfully):

- **E10a — decode the phys-mode target.** `kayfabe-abi/src/submit::ce`: add
  `SET_SRC_PHYS_MODE`/`SET_DST_PHYS_MODE`/`PHYS_MODE_TARGET_*` constants. `kayfabe-arch`: add
  `PhysTarget { LocalFb, CoherentSysmem, NonCoherentSysmem, Peer }` and carry `dst_target`/
  `src_target` on `PushMethod::CeLaunchDma` (default `LocalFb` = reset, faithful to hardware).
  `kayfabe-chips/src/ga10x.rs`: latch slots 6/7 in `ce_slot`, read them in `ce_launch`. Mock +
  the 5 `CeLaunchDma` sites updated. **A pure decode test asserts LOCAL_FB vs SYSMEM off a
  built pushbuffer — GPU-free, and the first thing to land.**
- **E10b — residency, in the pure core.** Replace `Representability`'s host-proxy with a
  residency answer over `(aperture | phys-mode-target, host.is_some())`: real-device-memory →
  `HostCe`; emulated-FB (fabricated, or a `LOCAL_FB` physical operand) → CPU/**Fb**; guest-RAM
  (sysmem binding, or a `SYSMEM` physical operand) → CPU/**GuestRam**; untracked → forward.
  Carry the CPU plane so the executor knows which store to touch. `partition_ce` intersects as
  today; a sub-copy is shell-executable only if **both** ends are CPU-reachable.
- **E10c — the shell CPU executor**, `kayfabe-rt/src/cpu_ce_unsafe.rs` (named per
  `l1_os_shell.md` §4.2.1's third construct — raw arithmetic over guest memory bypassing
  lifetimes). Moves bytes between `SparseFb` and `Vmm` per the planned plane; memset/fill are
  destination-only. Diverted from `forward_ce` before the isolate; `HostCe` spans still go to
  the isolate. A fence between executors for overlapping regions (owner's ordering rule).
- **E10d — signal truthfully.** After the bytes land, write the finishPayload semaphore to the
  channel's own aperture where the guest polls it (the completion tail `executor.rs:227`
  marks), and raise the channel's interrupt. This is the `sem_releases` consumer that does not
  exist yet.
- **E10e — the VAS-less physical channel.** `plan_doorbell`/`parse_pushbuffer` must accept a
  `vas_pdb == None` channel whose CE operands are physical (no VAS needed); the `NoVas` refusal
  is only correct for a *virtual* submission.

### 14.6 Acceptance (unchanged from the owner's brief)

`memmgrTestCeUtils`' own readback compare passes and the boot advances past `mem_mgr.c:526`.
⊘ No rung is claimed until a boot the author produced shows it, per `only_live_boots_are_proof`.

### 14.7 E10e — the survey (2026-08-08), and why it is a deep gap not a link

`[src]` throughout — a reading of the tree at master `349924b`, plus one boot
(`bench_evidence/run_probe35_349924b_qemu.log`). E10a–E10d landed and are tested GPU-free;
the boot confirms they do **not** regress the wall and are **not yet** on the live data path.
The doorbell for the CeUtils channel (`Gpu::SYSTEM_PROC`, `ChanId(1)`) is still refused
`NoVas` before any ring is read. Three gaps stack, and relaxing `NoVas` alone buys nothing:

1. **No page-directory root reaches the core.** `[src]` `Channel::vas_pdb` is set from
   `RmGraph::pdb_of_resource` (`crates/kayfabe-core/src/rmgraph.rs:2304`), fed by
   `RmEvent::SetPageDir`, produced only in `translate_control`
   (`crates/kayfabe-rmrpc/src/lib.rs:1358`). But `ObjectPolicy`'s `OBJECT_CONTROLS`
   (`crates/kayfabe-rmrpc/src/policy.rs:891`) is `{GPFIFO_SCHEDULE, BIND}` only, so
   `GSP_RM_CONTROL` is unclaimed and `translate_control` is unreachable on a real boot; and
   the control that carries this VAS's root, `0x90f10106`
   (`NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES`), is `PageDirNotModelled`
   (`crates/kayfabe-abi/src/capability.rs:2731`, refused at `rmrpc/src/lib.rs:1329`). The
   `FERMI_VASPACE_A` alloc reaches the graph as an edge with `AllocParams::NoDeclaredFacts`
   (`crates/kayfabe-abi/src/versions.rs:1021`), i.e. carrying no PDB, ever. ⇒ no `Vas`/
   `AddressTable` materializes for this channel (`crates/kayfabe-core/src/gpu.rs:2365`).
2. **The ring is a GPU VA that nothing can translate.** `[src]` the census names it
   `0x1_2006_4000` GPU-virtual; `read_pushbuffer` hard-requires `vas_pdb`
   (`crates/kayfabe-fwd/src/lib.rs:2754`). ⚠ At 8 GiB that VA is *itself a legal GPA*, so an
   untranslated read **succeeds and returns wrong bytes** (§8.2.3's measured warning) — which
   is why relaxing `plan_doorbell:1650`/`read_pushbuffer:2754` before a resolver exists trades
   a loud refusal for silent corruption.
3. **The shim has no `Vmm`/`FbStore` at the doorbell and E10c/d have no caller.** `[src]`
   `SharedDoorbell::ring` rings with an empty working set (`crates/kayfabe-qemu-raw/src/shim.rs:1487`);
   `Regs` holds neither a `Vmm` nor a handle to the installed `SparseFb` (moved into `RegPlane`
   with no getter); `cpu_ce::{execute_ours_spans, write_completion}` have no production caller.

★ **What is already E10e-clean** `[src]`: the *pure-core* half. `partition_ce`/`operand_runs`
short-circuit before the table on a physical operand (`crates/kayfabe-fwd/src/lib.rs:3103`),
so E10a/E10b classify a physical `sys ← vid` copy with no VAS; `apply_pushbuffer` already
threads `chan_pdb: Option<Pdb>`. The gap is the **address resolution for the ring/semaphore**
and the **shim wiring**, not the operand classification.

**Ordering the next agent should read off the above** (not a committed plan): (a) decide the
ring-address resolver first — teach `PushRange` its address-kind and add a GMMU-walk resolver
reachable from the act phase (the §8.2.3 follow-up, the deep item), *or* find a `0x90f10106`
decoder path that yields the page-directory root as a `Pdb` (smaller — the reason
`PageDirNotModelled` exists is to make this visible); (b) only then relax the `NoVas` sites;
(c) wire the shim — a shared `FbStore` handle + a late-installed `QemuVmm` on `SharedDoorbell`
(mirroring the `attach_ram` lifecycle), drive `SharedDevice::parse_pushbuffer` (not the
`&mut Gpu` free function — `Gpu::procs` never contains `SYSTEM_PROC`), divert `Ours` spans to
`cpu_ce::execute_ours_spans` before `forward_ce`, then `write_completion`. ⚠ `write_completion`
resolves the semaphore VA through the channel table, so it needs the same resolver as (a) —
or a VAS-less variant if the finishPayload turns out to be physical too.

### 14.8 ★★★ E10e is COHESIVE — item (1) is NOT independently landable (2026-08-08, `[src]`)

`[src]`, a reading of the doorbell path at `c525a11`, not a boot — but a reading that changes
what "ordered work" is allowed to *commit*, so it is recorded before anyone edits.

**The finding: routing the VAS root into the CeUtils channel (item 1) WITHOUT the shim already
driving parse→execute→completion (item c) does not advance the wall — it turns a loud, correct
`NoVas` refusal into a SILENT NO-OP.** That is this repo's own forbidden shape
(`mode2_forwarding_model.md`: never signal work that did not happen; `only_live_boots` §"a
served-but-inert path is worse than a refusal"). The mechanism, each step `[src]`:

1. `plan_doorbell` checks the isolate **before** the VAS: `missing_isolate` at
   `crates/kayfabe-fwd/src/lib.rs:1629`, the `NoVas` refusal at `:1650`. The wall measured at
   boot `run_probe35_349924b`, rev `349924b` (`bench_evidence/run_probe35_349924b_qemu.log`) is
   `NoVas`, so SYSTEM_PROC's isolate already exists — the VAS is the *only* thing missing.
2. Give the channel a VAS (item 1) and `plan_doorbell` passes `:1650`. The `#14` ring-gate then
   runs over the shim's **empty** working set (`SharedDoorbell::ring` calls
   `self.0.doorbell(DOORBELL_TARGET_GPU, token, &[])`, `crates/kayfabe-qemu-raw/src/shim.rs:1490`);
   `VerbPlan::gated_doorbell` over an empty set produces no `UngatedVa`, so the gate is vacuous
   and the plan is built (`lib.rs:1679`).
3. `commit_doorbell` materializes + schedules + rings a host channel and returns `Ok`. For a
   **physical-mode** CeUtils MemSet there is no host-side CE work behind that ring, so the
   finishPayload never advances, the guest still times out at `mem_mgr.c:463` — but now with the
   doorbell reporting **Served**, not Refused. The one instrument that currently says "we cannot
   do this yet" goes quiet.

⇒ **Corrected ordering.** The design may still be *built* in the sequence (1)→(a)→(b)→(c), but
the first thing that may be **committed** is an increment in which the shim drives
`parse_pushbuffer → execute_ours_spans → write_completion` off the doorbell — because only then
does a channel-with-a-VAS produce a loud named refusal at the *next* stop (a ring-VA `Miss`, or
the resolver's fault) instead of a silent success. Equivalently: if item 1 is landed first in
isolation, `plan_doorbell` must be taught to keep refusing by name (e.g. a
`FwdFault::CeUtilsExecutorUnwired`) until the executor is reachable — a scaffold refusal, which
is a smell but still loud. The plan's own guard (⊘ do not relax `NoVas` before a real resolver)
has a twin here: **do not grant the VAS before the executor, either** — both directions of this
gap fail *silently* if opened alone, and silent is the one failure mode this project treats as
worse than a stalled boot.

**The two facts item 1 needs, now pinned so they are not re-derived** `[src]`: the client-arm
`0x90f10106` control is issued with `rmCtrlParams.hObject = hVASpace`
(`ogkm-580: gpu_vaspace.c:5174-5177`), so the VASpace it publishes is named by the RPC header's
`hObject` — which `translate_control` currently **drops** (`crates/kayfabe-rmrpc/src/lib.rs:1256`).
And `levels[0]` is the **root** page directory: `_gvaspacePopulatePDEentries` fills
`levels[i].physAddress` / `.aperture` from `pdeInfo.levels[i]` top-down with
`pageShift = virtAddrBitLo` (`ogkm-580: gpu_vaspace.c:5243-5251`), and on GA106 the levels are
`47, 38, 29, 21`, so `levels[0].physAddress` is the PDB and `levels[0].aperture`
(`GMMU_APERTURE_VIDEO` for GSP-managed page tables) says it lives in the emulated FB
(`SparseFb`) — which is exactly the byte source `kayfabe_mmu::walker::translate` (already built)
walks. ⚠ But the fact is discarded at the chain, not at the decoder: `0x90f10106` is served for
its *reply* by `InitTablePolicy` (the FIRST chain link, `served_chain` in
`crates/kayfabe-device/src/lib.rs:924`), which decodes + re-encodes and terminates the `find_map`
chain, so `ObjectPolicy` (which holds the object model) never sees it and no `SetPageDir`-shaped
event is produced. Item 1 is therefore *"observe the publication BEFORE the answering link and
apply its root to the graph, while leaving the reply plane untouched"* — an observer that
declines, seated ahead of `InitTablePolicy`, not a re-route of who answers (re-routing breaks
`cap1b_differential` / `gvaspace_pdes`, which pin `InitTablePolicy` as the answerer).

### 14.9 ★★★ MEASURED: `SET_PAGE_DIRECTORY` is issued **ZERO** times in the whole boot — the port models the one control the driver never sends (2026-08-08)

`[measured 2026-08-08]` from `traces/real_ga106/rpc_transcript_real_ga106.txt` — our own committed
transcript of a **real** 580.159.04 driver on a **real** GA106. A census of all 88
`KAYFABE-RPC` entries, by command word:

| control | occurrences in the boot | what the port does with it |
|---|---|---|
| `0x00801813` `NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY` | **0** | `ControlParams::SetPageDir` — ★ *the only control the port turns into a `Pdb`* |
| `0x00801814` (the paired unset) | **0** | `PageDirNotModelled` |
| `0x90f10106` `VASPACE_COPY_SERVER_RESERVED_PDES` | **4** (entries 57, 61, 66, 70) | `PageDirNotModelled` — refused |
| `0x20800a9f` | **1** | `PageDirNotModelled` — refused |

⇒ **`NoVas(ChanId(1))` is not a missing link-up. It is the designed consequence of building the
entire `Pdb` identity on a control that never arrives.** No amount of wiring downstream of
`RmEvent::SetPageDir` can populate `chan.vas_pdb` on the boot path, because nothing upstream of
it ever fires. §14.7 called this "a deep gap, not a link"; this is the measurement that says so
without inference.

★★ **This PROMOTES `translate_control`'s own rustdoc from `[src]` to `[measured]`, and
strengthens it.** That doc already warned — reading `gpu_vaspace.c:3109` — that `SET_PAGE_DIRECTORY`
reaches the wire *only* for a `SHARED_MANAGEMENT`/`IS_EXTERNALLY_OWNED` (i.e. UVM's) VASpace, and
concluded the arm is *"necessary and not sufficient"*. For the boot path it is not merely
insufficient: it is **entirely absent**, and the doc's own framing — *"a conclusion a live boot
can refute and a source read cannot"* — is what earned the check. ⇒ `0x90f10106` is not "one more
control to serve". It is **the only boot-path source of a page-directory root**, and E10e's item
(1) is therefore load-bearing rather than incremental.

#### The wire facts, each checked against the capture rather than assumed

- ★ **`psize=184` on all four occurrences, and 184 is the struct size derived field-by-field**:
  `hSubDevice`(4) + `subDeviceId`(4) + `pageSize`(8) + `virtAddrLo`(8) + `virtAddrHi`(8) +
  `numLevelsToCopy`(4) + 4 pad + `levels[6]`×24 = **184**
  (`ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl90f1.h:272-332`, `GMMU_FMT_MAX_LEVELS = 6` at
  `:37`; each level is `physAddress`(8) + `size`(8) + `aperture`(4) + `pageShift`(1) + 3 pad).
  The decoder's exact-size assertion (§4.3) therefore has a constant confirmed against the real
  GA106's own wire (`traces/real_ga106/rpc_transcript_real_ga106.txt`, census 2026-08-08), not a
  transcribed one. ★ Which matters here specifically: this is the failure mode
  `c_oracle_empty_rows_are_wrong` is about — a size trusted from a table rather than from a
  machine was a buffer overrun with a hardware writer behind it.
- ★ **The sender is positively identified, not inferred**: every occurrence logs
  `head=00 00 00 00 00 00 00 00`, i.e. `hSubDevice = 0` **and** `subDeviceId = 0`. That is exactly
  `_gvaspacePopulatePDEentries`, which sets `subDeviceId` from
  `gpumgrGetSubDeviceInstanceFromGpu` (0 on a single GPU) and **never** writes `hSubDevice`
  (`ogkm-580: gpu_vaspace.c:5251-5253`). The `hSubDevice`-populated path is not the one we see.
- ★ **`levels[0]` is the root, confirmed at the producer** — `gvaspaceGetPageLevelInfo` starts at
  `pLevelFmt = pGpuState->pFmt->pRoot` and descends via `mmuFmtGetNextLevel` at the *bottom* of
  the loop, filling `levels[level]` top-down (`ogkm-580: gpu_vaspace.c:3974-4031`). The receiver
  then consumes it **bottom-up** (`for (i = numLevelsToCopy - 1; i >= 0; i--)`, `:4492`), which
  is the corroborating half: root last, so root is index 0.
- ★ **The aperture is a real fork, not decoration** — the receiver switches
  `GMMU_APERTURE_VIDEO → ADDR_FBMEM` and `GMMU_APERTURE_SYS_{COH,NONCOH} → ADDR_SYSMEM`
  (`ogkm-580: gpu_vaspace.c:4503-4511`), and asserts on anything else.

#### ⊘ And this is the debt `translate_control` predicted coming due

Its rustdoc says `RmEvent::SetPageDir` has nowhere to put `aperture`, so *"a vidmem-rooted and a
sysmem-rooted page directory become the same event"*, safe **only** while `Pdb` is a bare key —
and names the moment that stops being free: *"the day a walker follows a PDB it must know whether
the address is a framebuffer offset or a guest-physical address."* **E10e is that day.** So the
event this increment adds must carry the aperture from the start; adding it later would mean
`kayfabe_arch::ids::Pdb` had already accreted the wrong assumption (its own doc currently says
*"a per-GPU FB address"*). ⊘ Do not reuse `RmEvent::SetPageDir` for `0x90f10106` — its shape is
the shape that drops the fork.

#### ⚠ Two bounds on the above, stated so nobody over-reads it

1. ✓ **The transcript's terminus — RESOLVED the same day, and it is COMPLETE.** `[measured
   2026-08-08]` the 55 distinct commands are a **strict subset** of the C oracle's 56-row table
   (`C: src/qemu/mode2_initctrl_ga106.h`): `rows − transcript = {0x20800a4c}` and
   `transcript − rows = ∅`. That one row is the one `traces/real_ga106/README.md` independently
   calls a *"client-driven query rather than an init control"*, reachable only by widening with
   `nvidia-smi -q`. A truncated capture drops an arbitrary tail; this one is missing exactly the
   single command already classified as post-init. ⇒ the count is over a **whole** init, so the
   zero is a zero over the whole init and not merely over a prefix. See
   `docs/reference/remaining_boot_surface.md` §1.
2. `[src]` **`0x90f10106` fires only for a SPLIT VAS** — `gvaspaceCopyServerRmReservedPdesToServerRm`
   returns early unless `IS_GSP_CLIENT(pGpu)` **and** `pGVAS->vaStartServerRMOwned != 0`
   (`ogkm-580: gpu_vaspace.c:4039-4051`). So it publishes the levels backing the *server-RM-owned
   sub-range*. ★ That does not weaken it: `levels[0]` is the root of the **whole** VAS regardless
   of which VA the walk was seeded with, because every walk in a VAS starts at the same root.

### 14.10 E10e item (1), BUILT — the publication is LATCHED and the report carries it (`d79b67f`)

`[built]` at `62a06af` + `d79b67f`, GPU-free; ⊘ **no boot of this revision had been taken when this
section was written** — §14.11 has since taken three, and where the two disagree §14.11 wins —
so everything below is a property of the tree and of its tests, not of a guest. The one number
this increment exists to produce — `levels[0]` per publication, with its `hObject` — is a
**boot output** and is deliberately not asserted here.

**What was built.** `kayfabe_device::gvaspub`: a `GvasPubLog` modelled on `bar2::BarPdeLog`, and
a `GvasPubRecorder` `CommandPolicy` that decodes `0x90f10106` / `0x20800a9f` and records, per
publication, the control id, the header's `hClient` **and `hObject`**, `virt_addr_lo`/`hi`,
`page_size`, `num_levels` and every `PdeLevel` (`phys_address`, `size`, `aperture`, `page_shift`).
Surfaced through `RegPlane::gvas_publications()`, into `PlaneResidue`, over the shim wire as
`KayfabeGvasPublication` / `KayfabePdeLevel` (ABI 16 → 17), and printed by `nvkvm.c` at teardown
with `levels[0]` labelled **ROOT**.

**Three decisions, each of which had a wrong alternative.**

1. **Seated FIRST in `served_chain`, and it declines everything.** `InitTablePolicy` *terminates*
   the `find_map` for these two ids, so a recorder at the tail — where the other two recorders
   live — could never see one. ⊘ Re-routing *who answers* was the alternative and it is
   forbidden: `gvaspace_pdes` and `cap1b_differential` pin `InitTablePolicy` as the answerer.
   An always-`None` observer ahead of it is what makes both facts hold at once, which is what
   §14.8 asked for. `sticky::POLICY_DISPOSITIONS` carries it as `NeverAnswers` (a claim about
   the **type**), not `Guarded` (a claim about a caller).
2. **`hObject` is threaded, not dropped.** `translate_control` reads `req.client` and `req.cmd`
   and never `req.object` (`crates/kayfabe-rmrpc/src/lib.rs:1256`); nothing here goes through
   `translate_control` at all, because `kayfabe_abi::view::RpcControlReq` already carries
   `object` and `InitTablePolicy` classifies off the same header. ★ Independently corroborated
   after the fact by `c_ceutils_ring_resolution.md`: the C artifact's PDB source was **also**
   `0x90f10106`, keyed `{hClient, hVASpace}`, root aperture taken from `levels[0]`.
3. **The whole `PdeLevel` is kept, aperture included, at every level.** ⚠ Do **not** read this
   record as an FB address: `c_ceutils_ring_resolution.md` measured a **sysmem-rooted** PDB as an
   executing channel's own root on a real GA106 (2026-07-25), and `bUseBar1` is **per-instance**
   (one CeUtils instance sysmem-backed and another vidmem-backed in one run). There is no single
   right answer to "which aperture", which is why the fork travels with every level and why this
   log stores rather than resolves.

**⊘⊘⊘ What it does NOT do, and this is the increment's shape.** It does not populate
`chan.vas_pdb`, does not create a `Vas` in `proc.vases`, and does not relax `NoVas`. §14.8's
measurement stands: granting the channel a VAS before the executor is reachable turns a loud,
correct `NoVas` refusal into a doorbell that reports **Served** over work that did not happen.
⇒ **The wall must be unchanged by this increment.** A boot of this revision that does *not* end
at `memmgrMemSet … NV_ERR_TIMEOUT (0x65) @ mem_mgr.c:463` with `doorbell: NoVas(ChanId(1))` is
evidence that an observer changed something it should not have, and is a defect rather than
progress.

**Bite-check** (`--no-fail-fast`, both mutations restored). Dropping `hObject` from the
de-duplication key killed `two_va_spaces_publishing_the_same_levels_are_two_rows_not_one` and
`the_distinct_count_keeps_counting_past_the_sample_cap`. Making the recorder answer its own ids
killed six tests across four files — including `cap1b_differential`'s two, i.e. **the C oracle's
own replay is what catches an observer that starts answering.**

**The join this sets up, and the instrument that will settle it.** Guest patch `0002` prints
`hVASpaceId` for the CeUtils channel from the guest's own `channelWaitForFinishPayload`. If that
handle equals an `hObject` in this log, the VAS→channel join becomes a fact the guest itself
stated rather than one we joined up. If it differs, that is the more valuable result and this record is what
makes the disagreement visible at all. ⊘ Patch `0002` is bring-up instrumentation, so any rung
claimed on a boot carrying it is a **diagnosis**, not the milestone.

### 14.11 ★★★ E10e item (1), BOOTED — the join HOLDS, and the semaphore is OUTSIDE every published range (`c89899a`)

`[measured 2026-08-08]`, vast GA106 bench (`vh`, RTX 3060 `10de:2504`, host driver **580.159.04
Open**, 38 cores), source revision **`c89899a`** verified by
`strings … | grep -o 'kayfabe-rev:[0-9a-f]*'` on **both** `target/release/libkayfabe_qemu_raw.a`
and `qemu-build/qemu-system-x86_64` → `kayfabe-rev:c89899a661f1382cabbd2b5cab197b43fd12af10` in
each. Three boots, one fresh QEMU each:

| tag | probe set | guest `nvidia.ko` | evidence |
|---|---|---|---|
| `p2_c89899a` | `[35]` | **PATCHED** with `0002-bringup-ceutils-finishpayload-wait.patch`, built in-guest from `ogkm-580.159.04` against `6.8.0-136-generic` | `bench_evidence/run_p2_c89899a_{dmesg,qemu,serial,probe}.log` |
| `stock_c89899a` | `[35]` | **STOCK** (the `.run`'s own module, `md5 b029cb74…`, `grep -c KAYFABE-BRINGUP` = 0) | `bench_evidence/run_stock_c89899a_{dmesg,qemu,serial,probe}.log` |
| `noprobe_c89899a` | none | **STOCK** | `bench_evidence/run_noprobe_c89899a_{dmesg,qemu,serial,probe}.log` |

★ **How the patched module was built, because the README's route had to be corrected once
already.** The vendored full-source oracle `ogkm-580.159.04` (⊘ *not* the `.run` payload, which
ships a 17 MB precompiled `nv-kernel.o_binary` and has no `channel_utils.c`) was copied **into the
guest**, patched, and built there with `make modules -j16` against the guest's own
`6.8.0-136-generic` headers — 0 errors, `strings kernel-open/nvidia.ko | grep -c KAYFABE-BRINGUP`
= **2**, `modinfo -F vermagic` = `6.8.0-136-generic SMP preempt mod_unload modversions`. The stock
module is preserved at `/root/stock-nvidia-ko/nvidia.ko` in the guest and the patched one at
`/root/patched-nvidia.ko`, so the A/B is one `cp` + `depmod -a` in either direction. **The guest
is left holding the STOCK module**, because the milestone is a stock guest and the bench's default
must be the milestone's configuration.

⚠ **`mem_mgr.c:463` is a PROBE-CONDITIONAL wall** — `boot_wall_may_be_probe_conditional` again, on
a new wall. `[measured 2026-08-08]`, rev `c89899a`, RTX 3060 / 580.159.04 Open: a **third** boot,
`noprobe_c89899a` (same revision, same stock module, no
`NVKVM_DEV_EXTRA`), dies **earlier**: `_memmgrMemUtilsScrubInitRegisterCallback: event
notification control failed` → `mem_utils.c:2022` (`NV_ERR_GENERIC`), `RmInitAdapter failed!
(0x25:0xffff:1249)`, with **2** publications instead of 3 and **0 doorbells arrived**. The
CE-copy wall this increment is about is only reachable with notifier 35 armed; a boot taken
without it never gets to the question, so it falsifies nothing here.
(`bench_evidence/run_noprobe_c89899a_*.log`.)

#### The falsifier §14.10 stated, CHECKED FIRST

`diff` of `run_probe35_349924b_dmesg.log` (no observer, stock module) against
`run_stock_c89899a_dmesg.log` (observer present, stock module), timestamps stripped:
**byte-identical, 22/22 lines.** The wall is `memmgrMemSet … NV_ERR_TIMEOUT (0x65) @
mem_mgr.c:463` and the doorbell is `first doorbell refusal [FwdFault::NoVas] NoVas(ChanId(1))`,
exactly as before. ⇒ **the observer changed nothing the guest can see.** §14.10's condition is met.

And `p2_c89899a` vs `stock_c89899a` (`[measured 2026-08-08]`, rev `c89899a`, RTX 3060 /
580.159.04 Open), timestamps stripped and the `KAYFABE-BRINGUP` lines removed,
differ in **one** line — the module's own build banner (`(ubuntu@ubuntu) Sat Aug 8 …` vs
`(dvs-builder@U22-I3-AF04-09-6) Wed Apr 29 …`). ⇒ **patch `0002` is observation-only, measured
rather than asserted.**

#### The guest's own lines, verbatim (`run_p2_c89899a_dmesg.log`)

```
[33.381991] NVRM: channelWaitForFinishPayload: KAYFABE-BRINGUP: waitFinishPayload ENTER: hClient=0xc1e00006 chId=0x2 hVASpaceId=0xa pbGpuVA=0x420000000 finishPayloadOffset=0x6c004 semaVA=0x42006c004 semaOffset=0x6c000 pbCpuVA=FFFFD171024C1000 bUseBar1=0 bUseVasForCeCopy=1 engineType=11 target=0x1 cur=0x0
[37.382285] NVRM: channelWaitForFinishPayload: KAYFABE-BRINGUP: waitFinishPayload TIMEOUT: hClient=0xc1e00006 chId=0x2 semaVA=0x42006c004 pbCpuVA=FFFFD171024C1000 bUseBar1=0 target=0x1 cur=0x0 pbSema=0x0 isChannelActive=0 workSubmitToken=0x10002
[37.382797] NVRM: … ENTER:  (identical to 33.381991 — the retry)
[41.383463] NVRM: … TIMEOUT: (identical to 37.382285 — the retry)
[42.326466] NVRM: channelWaitForFinishPayload: KAYFABE-BRINGUP: waitFinishPayload ENTER: hClient=0xc1e00005 chId=0x2 hVASpaceId=0x0 pbGpuVA=0x120000000 finishPayloadOffset=0x6c004 semaVA=0x12006c004 semaOffset=0x6c000 pbCpuVA=FFFFD17102453000 bUseBar1=1 bUseVasForCeCopy=0 engineType=11 target=0x0 cur=0x0
```

#### ★★★ The join, from two independent sides in ONE boot

| side | statement |
|---|---|
| device (`run_p2_c89899a_qemu.log`) | `gvas cmd 0x90f10106 hClient 0xc1e00006 hObject 0x0000000a` |
| guest (`run_p2_c89899a_dmesg.log`) | `hClient=0xc1e00006 … hVASpaceId=0xa` |

**EQUAL — on `hClient` *and* on the handle.** The VAS→channel join is now a fact the guest stated
about itself, not one we inferred from a resolver. ⊘ And `hVASpaceId != 0`: the scrubber **is** in
virtual mode (`bUseVasForCeCopy=1`), which is the branch §14.10 could not choose between.

The second CeUtils instance is the counter-example that proves the recorder is not just echoing:
the device recorded `hClient 0xc1e00005 hObject 0x0000000c`, while the guest printed
`hVASpaceId=0x0` for *that client's channel*. Not a contradiction — that channel is **not** in
virtual mode (`bUseVasForCeCopy=0`, `bUseBar1=1`), so it has no VAS handle; `0xc` is a VA space
the same client published for something else. ⇒ **the two CeUtils instances differ in aperture
*and* in mode within one run** (`[measured 2026-08-08]`, boot `p2_c89899a`, rev `c89899a`, RTX
3060 / 580.159.04 Open), reproducing on our device the per-instance `bUseBar1` split the C
artifact measured on 2026-07-25. The third publication, `cmd 0x20800a9f hClient 0 hObject 0`, is
the subdevice-scoped BAR2 one and carries no client.

★ **`workSubmitToken=0x10002` (guest) == `DOORBELL token 0x00010002` (device).** A third
independent join in the same boot: the doorbell our device refused is provably *this* channel's.

#### ★★★ THE FINDING — the semaphore is OUTSIDE every published page-directory range

All **3** publications, in both boots, cover exactly one range:

```
va [0x0000000100000000..0x000000011fffffff] pageSize 0x200000 levels 4
  level[0] ROOT phys 0x…000 size 0x20 aperture 1 pageShift 47   (aperture 1 on every level, all 3)
```

The channel that **walls** has `pbGpuVA=0x420000000`, `semaVA=0x42006c004` — **not in
`[0x1_0000_0000..0x1_1fff_ffff]`, nor in any published range.** Only the *other* instance
(`pbGpuVA=0x120000000`, `semaVA=0x12006c004`, the one that never waits: `target=0x0`) lands
inside it.

⇒ Two things follow, and they reorder the work:

1. **`NoVas(ChanId(1))` is not merely "we declined to bind a VAS" — there is no published VAS
   that could serve this channel's addresses even if we bound one.** Populating `chan.vas_pdb`
   from `gvaspub` as it stands would give the walling channel a page directory that does not map
   its own pushbuffer. That is precisely §14.8's "a doorbell that reports **Served** over work
   that did not happen", arrived at from a new direction.
2. Either RM publishes a *second* range per VAS that `0x90f10106` / `0x20800a9f` does not carry,
   or the `0x420000000` mapping arrives by a transport we are not recording at all. **Finding
   which is the next measurement**, and it is a different question from the one E10e item (2) was
   scoped to.

⚠ **This REFUTES the premise the task carried in**, that the walling channel would be the C
artifact's `0x120000000` / gpfifo `0x120064000` one. It is not. The C's channel is present and its
arithmetic reproduces **exactly** — `0x120064000 + 0x8004 = 0x12006c004`, and our device's own
`gpfifo rings: … first 0x0000000120064000` names it — but it is the instance that **succeeds
trivially** (`target=0x0`). The wall is a *different, second* CeUtils instance one VA aperture
higher. Any reasoning that assumed one CeUtils channel was reasoning about the wrong one.

#### At the timeout: `cur < target` at a plausible `semaVA`

`target=0x1`, `cur=0x0`, `pbSema=0x0`, `isChannelActive=0`. `cur` is **not** garbage, so this is
"the release never landed", not "it landed somewhere else" — and the device's own count
(`doorbells: 1 arrived, 0 served, 1 REFUSED`) says why: nothing executed at all. `isChannelActive=0`
is the guest agreeing that the channel never came up.

⊘ **Both boots are diagnostics, and one of them is patched.** No milestone is claimed. The
milestone remains a stock guest, and the stock arm of this pair ends at exactly the same wall.

### 14.12 ⊘ ADJUDICATION: "no published VAS could serve this channel" is REFUTED by arithmetic

`[measured 2026-08-08, boot run_p2_c89899a]` §14.11 concludes from the boot that the walling
channel's `semaVA=0x42006c004` is *"outside every published range"* and therefore *"there is no
published VAS that could serve this channel's addresses"*, and that only the non-walling instance
lands inside. **The first clause is true, the second does not follow, and the third is wrong.**

#### 1. BOTH channels are outside the published range — so it distinguishes nothing

```
published: [0x1_0000_0000 .. 0x1_1FFF_FFFF]      512 MiB, the server-RM split range
A (WALLS):  pbGpuVA 0x4_2000_0000   outside      (0x3_0000_0001 past the end)
B (succeeds): pbGpuVA 0x1_2000_0000 outside      (exactly ONE byte past the end)
```

B sits at `0x1_2000_0000`, and the range ends at `0x1_1FFF_FFFF`. ⇒ **B is outside too.** Being
outside the published range cannot be why A fails, because it is equally true of the instance that
succeeds. ★ B succeeds because it has **no pending work** (`target=0x0`) — its success says nothing
about addressing at all.

#### 2. The published RANGE is not the root's COVERAGE

The boot reports `level[0] … size 0x20 … pageShift 47`. That is `0x20 / 8 = 4` entries indexed at
bit 47, i.e. **4 × 2⁴⁷ = 512 TiB of coverage** — the whole GA106 virtual address space. Both
`0x4_2000_0000` and `0x1_2000_0000` resolve to **root entry index 0**.

⊘ So a root cannot fail to "cover" a VA in its own VAS; covering everything is what a root *is*.
What the publication's `virtAddrLo/Hi` scopes is which sub-range's **lower** levels RM reserved and
copied — `_gvaspacePopulatePDEentries` seeds `gvaspaceGetPageLevelInfo` with
`virtAddress = pGVAS->vaStartServerRMOwned` (`ogkm-580: gpu_vaspace.c:5236-5240`), and
`mmuWalkGetPageLevelInfo` returns the instance **on the path to that VA**. For level 0 there is only
one instance per VAS, so it is the root whatever VA seeded the walk (`:3974-3981`).

#### ⇒ The consequence for the increment: §14.11's plan stands, unchanged

`levels[0].physAddress` for `hObject 0x0a` — the VAS the join proved this channel belongs to — is a
usable PDB for `0x4_2000_0000`. The route is unchanged: **walk** from that root through `SparseFb`
with `kayfabe_mmu::walker::translate`, aperture fork at every level. The published `levels[1..3]`
are for a different path and ⊘ must not be reused — that part of §14.11 is correct and important.

#### ⊘⊘ AND THE "what if the walk misses" WORRY IS ALSO ALREADY ANSWERED — three times

I first wrote this section saying the open question was *"are the intermediate entries on the path
to `0x4_2000_0000` actually present in our emulated FB?"* ⊘ **That is the wrong worry, and §9.2
already settled it the other way** — `[measured 2026-08-02, rev 4e8960f]`, six days before I asked.
Recorded here rather than quietly deleted, because the next reader will reach for the same
hypothetical:

1. **A miss is not an open design question — it is built and loud.**
   `crates/kayfabe-mmu/src/walker.rs:215`: `PteDecode::Invalid => Err(TranslateFault::Unmapped {
   va, level })`, carrying the level it failed at; an un-enumerated leaf size is a separate named
   fault (`WalkFault::UnknownLeafSize`, #13's corollary made into a check). There is no fallback
   to design and no policy to decide.
2. ★★★ **A walk stopping at a root HAS been measured — and the cause was not missing bytes.**
   §9.2, `[measured]` 2026-08-02 at rev `4e8960f`, test
   `e5_address_table_join.rs::the_ce_pt_write_source_can_witness_only_a_root_page_today`. The
   cause was that **source 2** (the observed CE page-table write) witnesses *roots only* —
   `Spine::pt_roots` is seeded from roots, and a guest CE write into a leaf table is classified as
   ordinary data. §9.2's own words: *"The bytes decode correctly and the metadata chain **is**
   learned forward — only the **binding** is withheld."* ⇒ the constrained operation was never
   reading; it was **authorisation to bind**.
3. **E8 built the PUBLISH phase that closed exactly that** (§12).

⇒ **And tonight's measurement removes the constraint entirely for this channel.** The root arrives
on `0x90f10106` — an RPC binding, i.e. **source (1)** of the two co-equal populate sources named in
`mode2_address_table.md`, the sanctioned forward transport. Source 2's witness chain is not needed
for it at all. ★ And reading the intermediate levels *during a walk* is **reading page-table bytes,
not binding** — that is what a TLB does, and it was never the operation the doctrine restricts.

⊘ So: do not treat a walk from `levels[0]` as speculative, and do not go looking for "a second
range these two control ids don't carry". If a walk misses, it will say so **by name, at a level** —
and *that* named fault is the thing to investigate, not a transport hypothesis invented ahead of it.

⚠ **How the wrong inference was reachable, worth naming:** the report compared a VA against a
range and reasoned about *set membership*, where the operative property was *tree reachability*.
Both instances failing the membership test — and one of them succeeding anyway — is the check that
would have caught it, and it is one subtraction.

### 14.13 ★★★ E10e item (a), BOOTED — **THE WALK RESOLVES**, and every address of the walling channel lands in guest RAM (`84d857d`)

`[measured 2026-08-08]`, vast GA106 bench (`vh`, RTX 3060 `10de:2504`, host driver **580.159.04
Open**), source revision **`84d857d`** verified by `strings … | grep -o 'kayfabe-rev:[0-9a-f]*'` on
**both** `target/release/libkayfabe_qemu_raw.a` and `qemu-build/qemu-system-x86_64` →
`kayfabe-rev:84d857ded732cf1438f700dc261e39cd26ff1c5e` in each. Boot `p35_84d857d`, probe set
`[35]`, **stock** guest module. Evidence: `/workspace/bench/run_p35_84d857d_{qemu,dmesg,serial,probe}.log`.

The doorbell refusal now carries the walk, and this is the whole line:

```
first doorbell refusal [FwdFault::NoVas] NoVas(ChanId(1))
  | c=0xc1e00006 vas=0xa root=0x2efa9c000/ap1/sh47 ring=0x420064000
    rng=S:0x2f2c3000  fin=S:0x2f2cb004  gp0=0x420000064+0x60  pb=S:0x399d064
```

#### ★★★ §14.12's open question is ANSWERED, and the answer is yes

§14.12 closed with *"what genuinely remains open is the empirical question: are the intermediate
entries on the path to `0x4_2000_0000` actually present in our emulated FB? … ⊘ Do not go looking
for a second range these two control ids don't carry until a walk has actually missed."*
**The walk did not miss.** Starting from `levels[0]` of the publication the guest made for
`(hClient 0xc1e00006, hObject 0x0a)` — the pair §14.11's two-sided join proved is this channel's —
the GMMU descent through our emulated framebuffer resolves **every** address the submission names:

| address | what it is | resolves to |
|---|---|---|
| `0x4_2006_4000` | `gpFifoOffset`, the ring | **guest RAM** `0x2f2c_3000` |
| `0x4_2006_c004` | the finishPayload semaphore (`ring + 0x8004`) | **guest RAM** `0x2f2c_b004` |
| `0x4_2000_0064` | the first GPFIFO entry's target, **read out of the resolved ring and decoded**, 0x60 bytes | **guest RAM** `0x399_d064` |

⇒ **Three independent corroborations in one line.** (1) `0x420064000 + 0x8004 = 0x42006c004`, which
is the `semaVA` the guest itself printed (§14.11), so the ring address we derived and the semaphore
the guest polls are the same allocation. (2) `0x2f2cb004 − 0x2f2c3000 = 0x8004` — the *physical*
pages are contiguous across the same offset, which is the C artifact's cont.8 correction
(`c_ceutils_ring_resolution.md` §4: the finishPayload is contiguous at `+0x8004` within the 64 KiB
channel buffer) reproduced on our own device. (3) The ring's first entry decodes as a well-formed GP
entry naming `pbGpuVA + 0x64` — i.e. the pushbuffer inside the same buffer the channel declared —
rather than as garbage, which is what a wrong root would have produced.

★ **The chain, not one address of it.** The third row is the load-bearing one: it required resolving
the ring, *reading guest memory through that resolution*, decoding the bytes as a GPFIFO entry, and
resolving the address that entry named. A wrong root cannot pass that.

#### ⊘ The wall is UNCHANGED, and that is the increment's acceptance condition

`memmgrMemSet(… TRANSFER_FLAGS_PREFER_CE) @ mem_mgr.c:463`, `NV_ERR_TIMEOUT (0x65)`,
`RmInitAdapter failed! (0x25:0x65:1249)` — identical to `stock_c89899a`. §14.8 measured why that is
required rather than disappointing: this increment binds no `vas_pdb`, creates no `Vas` and relaxes
no `NoVas`, because granting the channel a VAS *before* the executor is reachable turns a loud,
correct refusal into a doorbell reporting **Served** over work that did not happen. The refusal is
still `NoVas(ChanId(1))`; what changed is that it now **states the resolution** instead of leaving it
to be inferred.

#### ★★★ THE APERTURE: all three addresses are **SYSMEM**, and that settles a fork the port had open

Every one resolves to guest RAM, not to the emulated framebuffer. That is the `bUseBar1=0` half of
the per-instance split: general CeUtils passes `_NO_BAR1_USE_TRUE` (`ogkm-580: mem_mgr.c:4134`) ⇒
sysmem, while the memory scrubber passes `_VIRTUAL_MODE_TRUE` with no `_NO_BAR1_USE`
(`ogkm-580: mem_scrub.c:154`) ⇒ vidmem (`c_ceutils_ring_resolution.md` §2). The guest's own
instrumentation agrees from the other side — §14.11's `bUseBar1=0 bUseVasForCeCopy=1` for exactly
this channel.

⇒ **Consequences for the executor, all three now MEASURED rather than open:**
1. The ring and the pushbuffer are read with `Vmm::gpa_read` / the plane's `GuestRam` port, **not**
   from `SparseFb`. A reader that assumed the framebuffer would have read an unrelated page.
2. The finishPayload must be written to **guest RAM** at `0x2f2c_b004`. This is `#12`'s
   where-mistake with the apertures the other way round from the C's scrubber instance, and it is
   the reason `cpu_ce::write_completion` resolves the aperture rather than picking one.
3. The **page directories** are in vidmem (`ap1` on all four published levels) while the **leaves**
   are sysmem — so the per-level aperture fork is not decoration on this channel, it is the
   difference between finding the tables and finding nothing.

#### ⚠⚠ How this was nearly missed, and it is `a_table_does_not_decide_behaviour` again

The **first** boot of this increment (`p35_a34025b`, same night, same box) refused
`rng=ROOTAP1 fin=ROOTAP1` — *"aperture 1 is not this device's framebuffer"* — with the join and the
plumbing already perfect. The cause was four constants: `GMMU_APERTURE` had been transcribed from
the **PDE field** encoding (`ogkm-580: kern_gmmu_fmt_gm10x.c:165-182`, `0=INVALID 1=VIDEO 2=SYS_COH
3=SYS_NONCOH`) instead of from the **enum** this control's `levels[].aperture` actually carries
(`ogkm-580: src/nvidia/inc/libraries/mmu/gmmu_fmt.h:280-325` — unnumbered, so declaration order is
the encoding: `INVALID=0 VIDEO=1 PEER=2 SYS_NONCOH=3 SYS_COH=4`). The two agree on `INVALID` and
`VIDEO` and disagree on everything else, and ⚠ `SYS_NONCOH` precedes `SYS_COH`, the reverse of every
other list in this port.

★ Two fields named `aperture`, in one subsystem, with different encodings, and the wrong one is a
plausible read of the right file. The boot is what told them apart: `[measured 2026-08-08]`
`run_p35_a34025b_qemu.log` says `rng=ROOTAP1 fin=ROOTAP1` and `run_p35_84d857d_qemu.log` says
`rng=S:0x2f2c3000 fin=S:0x2f2cb004`, on bit-identical inputs and four changed constants — while the
citation the wrong values carried (`ogkm-580: kern_gmmu_fmt_gm10x.c:165-182`) was, and remains,
a **correct** reading of a **different** field.

#### The discipline this walk runs under, stated because it is a safety rule

`gmmu_publication_discipline.md` §6.3: *"Walk-on-miss is safe if and only if 'miss' means 'the GPU
faulted on this VA'. Walk-ahead is not safe, and no ordering rule in this driver makes it safe."*
The trigger here is the **doorbell** — the guest's own submit fence, after it wrote the ring,
published the mappings and ran §3's flush — and it is the only commit point available, since §5
measured **both** invalidate transports at zero on this path. The permission is carried as a value
(`kayfabe_device::ceresolve::Demand::from_doorbell`) so a future prefetch cannot acquire it by
editing a comment. §7's eight rules are checked one by one in `ceresolve`'s module header, including
the two this port does **not** satisfy: rule 5 is enforced for vidmem leaves and **not** for sysmem
ones (no GPA bound is available; an out-of-range sysmem leaf refuses at *use*, not at decode), and
rule 7 is **vacuous** — with no invalidate to serialise against there is **no defence against
§6.2(3)**, a sub-level freed before any invalidate.

#### What remains for the executor, now that addressing is not the unknown

The `pb=S:0x399d064 +0x60` range is 96 bytes = 24 method words: the CeUtils `memset` pushbuffer.
⚠ `Ga10xPushbuffer::ce_launch` refuses `LAUNCH_REMAP_ENABLE` (`kayfabe-chips/src/ga10x.rs:1113`),
and a CE *fill* is exactly a remap-enabled launch — so decoding this submission needs
`SET_REMAP_CONST_A` / `SET_REMAP_COMPONENTS` and a `CeWork::Fill`, which E10a–E10d did not build.
That, plus driving `parse → execute_ours_spans → write_completion` off the doorbell with the
**sysmem** plane, is what E10e item (c) still owes. ⊘ None of it is an addressing question any more.

### 14.14 ★★★ E10e item (c), part 1 — the FILL DECODES, and four premises were refuted getting there (2026-08-08)

§14.13 closed with *"decoding this submission needs `SET_REMAP_CONST_A` /
`SET_REMAP_COMPONENTS` and a `CeWork::Fill`"*. That is built. What the building found is worth
more than the decode, and all four are recorded before the increment.

#### ⊘ REFUTED 1 — `CeWork::Fill` was produced by NO decoder, and its only "coverage" ran through the mock

`[src]`, a whole-tree read at `4c3348f`. `kayfabe_arch::CeWork::Fill` had exactly three
mentions outside its own definition:

| site | what it does | is a real `Ga10x` decode in its path? |
|---|---|---|
| `tests/tests/pushbuffer_parser.rs:197` | hand-builds `FILL` and hands it to `ce_executor_c` | **no** — `ce_executor_c` is a pure predicate; no decoder is involved |
| `tests/tests/ce_representability_split.rs:80` | hand-builds `FILL` and hands it to `partition_ce` / `cpu_ce` | **no** |
| `crates/kayfabe-fwd/src/lib.rs:3315` | the consumer, `Fill { pattern } => CeSource::Constant(pattern)` | **unreachable** — nothing produced a `CeLaunchDma` with `work: Fill` |

⇒ **The answer to "which Fill tests have a real `Ga10x` decode in their path" was: none, and it
could not have been otherwise** — `Ga10xPushbuffer::ce_launch` refused every remap-enabled
launch at `ga10x.rs:1113`. ★ And it is *worse* than mock-only coverage: `kayfabe_mocks`
round-trips `CeWork::Fill` (`lib.rs:518`, `:598`) but **`MockPushbuffer::ce_launch_dma_full` has
five callers and every one passes `Copy` or `Scrub`**. The mock's Fill arm had zero callers too.
So the whole path from a guest's bytes to `CeSource::Constant` was not merely green over a case
a real chip refuses — it was never exercised at all, in either direction. This is
`mock_fidelity_both_directions` with the mock not even being the culprit: the *capability* was
declared in three places and reachable from none.

#### ⊘ REFUTED 2 — the pattern is NOT a 4-byte word, and `LINE_LENGTH_IN` is NOT bytes

`[src]` `ogkm-580: kernel-open/nvidia-uvm/uvm_maxwell_ce.c:330-420`, the driver's own three
memset entry points on this class. `SET_REMAP_COMPONENTS` decides **two** things nothing else
carries:

1. **`LINE_LENGTH_IN` counts ELEMENTS.** `uvm_hal_maxwell_ce_memset_4` does `size /= 4`
   *before* `memset_common` pushes `LINE_LENGTH_IN`, and `memset_common` advances the
   destination by `memset_this_time * memset_element_size` (`:359`, `:371`, `:396`). An element
   is `COMPONENT_SIZE × NUM_DST_COMPONENTS` bytes.
2. **The pattern's PERIOD is the element.** `memset_1` puts an `NvU8` in `CONST_B` with
   `COMPONENT_SIZE_ONE`; `memset_8` spreads a 64-bit value across `CONST_A`+`CONST_B` with
   `NUM_DST_COMPONENTS_TWO`.

⚠ **And RM's memset path — the one behind `mem_mgr.c:463` — is the 1-BYTE map**
(`ogkm-580: channel_utils.c:1029-1033`: `DST_X = CONST_A | COMPONENT_SIZE_ONE |
NUM_DST_COMPONENTS_ONE`). So `memmgrMemSet(…, value, …)` writes `value & 0xFF` to **every
byte** — which is exactly what its own `TRANSFER_TYPE_PROCESSOR` arm produces with
`portMemSet(pDst, value, size)` (`ogkm-580: mem_utils.c:1122`). Two arms of one operation must
be observationally equal, and that is the corroboration this rests on.

⇒ The brief's rule *"use a pattern whose four bytes differ, the phase comes from the
destination address"* is **right for a 4-byte element and wrong for RM's**. The decoder's job
is to **normalise**: a 1-byte-element fill of `v` becomes `u32::from_le_bytes([v; 4])`, which
`cpu_ce`'s existing `pattern[a % 4]` phasing then reproduces byte for byte, unchanged.
★ Note the C artifact is *positively wrong* here rather than narrow: it writes `remapA` per
32-bit word unconditionally (`C: nvkvm_gpu_emul.c:6349`) and reads no component map at all, so
it disagrees with hardware on RM's own scrub map for every pattern above `0xFF`.

⚠ The two encodings that make this easy to get backwards: `COMPONENT_SIZE_ONE` and
`NUM_DST_COMPONENTS_ONE` are both the **literal zero** (`clc7b5.h:215`, `:225`) — the fields are
*size minus one* — so every `DRF_DEF(…, _ONE)` in RM and UVM contributes nothing to the word,
and a decoder reading the field as the size reports a **zero-byte element**.

#### ⊘ REFUTED 3 — a fill has no source operand, and requiring one refuses every memset

`[src]` `ogkm-580: channel_utils.c:1036-1067`: `channelPushMemoryProperties` pushes
`OFFSET_IN_UPPER/_LOWER` **only** on its `bCeMemcopy` arm. A memset's source registers are
never latched, so `ce_launch`'s `state.latched(subch, 0)?` would have refused every fill the
driver sends even after the remap gate opened. Reporting whatever a *previous* copy on that
subchannel left in them would report a stale address as the fill's source; the decode now
reports `src = 0` explicitly for the no-source work kinds.

#### What landed

`kayfabe_abi::submit::ce` gains the three method addresses and the map's field extents (with
`remap_component_bytes` / `remap_num_dst_components` carrying the minus-one encoding), and
`Ga10xPushbuffer` gains `remap_fill`. The two refusals that shared a line at `ga10x.rs:1113` are
now separate, and the fill has **five** of its own, each a distinct thing the codec cannot say:
no component map; a selected `CONST_*` never latched; a `DST_* = SRC_*` selector (a swizzle, not
a fill); `NO_WRITE`; and an element size that does not divide 4.

⊘ **The last one is UVM's `memset_8`, and it is refused rather than truncated.** `CeWork::Fill`
carries a `u32`, so an 8-byte period is not expressible; answering with its low four bytes would
write the wrong half of every other element, silently. Widening the variant is the fix, and the
named refusal is what keeps the gap from reading as support.

★ A **sixth** refusal is about the representation rather than the map: the engine phases an
element from the **start of the transfer** while `CeWork::Fill`'s downstream phases it from the
**absolute destination address**. Those agree for every byte iff `dst % element == 0`, so an
unaligned element-sized fill is refused. `[src]` both drivers align their memsets, which is
exactly why the condition must be checked rather than inherited from them.

Coverage: `tests/oracle/pushbuffer_abi_oracle.c` grows `emit_ce_fill_run`, which emits the
memset shape — remap registers first, **no** `OFFSET_IN_*`, `LINE_LENGTH_IN` alone, then
`LAUNCH_DMA` — with the component map read back through NVIDIA's own `DRF_VAL`. Eight accepted
cases (RM's scrub map, UVM's `memset_1`/`memset_4`, a 2-byte element, a two-constant element,
`memmgrTestCeUtils`' own zero-pattern 4-byte case, a physical destination, an aligned 4-byte
element) and eight refusals. `the_decoded_fill_writes_the_bytes_the_engine_writes` drives
`Ga10xPushbuffer` → `partition_ce` → `cpu_ce::execute_ours_spans` → a read-back of the store,
against a destination image modelled from NVIDIA's extraction of the map.

#### ⊘⊘ REFUTED 4 — and this one BLOCKS item (c): the two halves of the executor disagree about what a VA is

`[measured 2026-08-08, local test run of `the_decoded_fill_writes_the_bytes_the_engine_writes`
at this revision]`. Binding the fill's destination VA `0x22000` to `phys = 0x0400_0000` and
running the decoded work through `partition_ce` → `cpu_ce::execute_ours_spans`, the bytes landed
at **`0x22000` in the framebuffer store** — the *virtual* address — and `0x0400_0000` was
untouched. Reading back at the VA passes; reading back at the bound physical fails.

The mechanism, `[src]` and short:
- `kayfabe_fwd::operand_runs` returns `AddressTable::spans`' run start, which is a **VA**, and
  `partition_ce` puts it in `CeSubCopy::dst` unchanged. `Binding::phys` is consulted only for
  the *representability* answer.
- `cpu_ce::execute_ours` then does `write_plane(…, dst.wrapping_add(off), …)` — it writes the
  destination plane **at the VA**.
- `cpu_ce::write_completion`, in the same module, does `table.resolve(pdb, addr)` and writes at
  `binding.phys + off` — it **does** translate.

⇒ **The completion half translates and the data half does not.** No test in the tree could see
it: `cpu_ce_executor.rs` uses an `AddressTable` only for `write_completion` and every one of its
`execute_ours` cases is a *physical* operand (where the address genuinely is the plane address);
`ce_representability_split.rs` binds `phys ≠ va` but asserts only **split-vs-whole equality**, a
property both a translating and a non-translating executor satisfy identically.

⊘ This is `#12`'s where-mistake — *"landing the data where the guest cannot see it"* — in the
half the C paid weeks for, and it is a **blocker for item (c)**: `memmgrTestCeUtils`' memset has
a **virtual** destination (`bUseVasForCeCopy = 1` with `dstAddressSpace == ADDR_FBMEM` ⇒
`dstAddr + fbAliasVA - startFbOffset`, `ogkm-580: channel_utils.c:1090-1095`), so wiring the
doorbell executor today would fill the wrong framebuffer page and then release a truthful-looking
semaphore over it. The one absolute rule survives — the bytes *are* written — but rule 2 of
`ce_executor_tree.md`'s two prohibitions does not.

The fix is not a special case: `CeSpan` must carry the operand's **plane address** beside the
plane it already carries (`dst_plane` / `src_plane`), present exactly when the plane is, computed
in `operand_runs` where the binding and the run's offset into it are both in hand. `CeSubCopy::dst`
must stay the VA, because the `HostCe` arm submits it to a host VAS. ⊘ Two different addresses in
one field is the defect; one field each is the fix.

#### What item (c) still owes, after this

1. **The `CeSpan` plane-address fix above.** ⊘ Ordered first: it is the difference between
   forwarding and forgery on this channel's own destination.
2. Driving `parse_pushbuffer → execute_ours_spans → write_completion` off the doorbell. ⚠ Note
   the resolution route is `ceresolve`'s published-root walk, while `partition_ce` and
   `write_completion` both take an `AddressTable` — so the join needs the walk's answers to
   reach the table (a TLB fill at the demand, which is what the table *is*) or an operand
   resolver seam. That is a design decision, not a wiring detail, and §7 rule 6's *"never cache
   the walk"* has to be argued against `mode2_address_table.md`'s blessed staleness rather than
   assumed compatible.
3. ★ And `mem_mgr.c:463` is not the last stop: the very next line is
   `memmgrMemCopy(sys ← vid, 4 bytes, PREFER_CE)` followed by
   `NV_ASSERT_TRUE(sysmemData == vidmemData)` (`ogkm-580: mem_mgr.c:467-470`) — a real
   **virtual-source, physical-destination** CE copy whose bytes are read back and compared. The
   acceptance for `memmgrTestCeUtils` is that compare, not the memset.

### 14.15 ★★★ E10e item (c), parts 1b + 2 — the PLANE-ADDRESS newtype and the CE COMPLETION, both landed; and what the wiring still owes (2026-08-08)

Two commits, both GPU-free, both at `scripts/ci_gates.sh --all` **exit 0** (23 steps, ledger
`381/66/17`, all five oracle families `RAN` with `SKIPPED=0`): `97fe402` (the newtype) and
`e95c04a` (the completion decode). ⊘ **Neither wires the doorbell.** No boot was taken at
either revision, so nothing below is a claim about a guest.

#### 1. §14.14's REFUTED 4 is FIXED, and the fix is a type (`97fe402`)

`kayfabe_arch::PlaneAddr` — an address *inside* a [`CpuPlane`] — and
`kayfabe_arch::CpuOperand { residency, addr }`, which `CeSpan::{dst,src}_place` carries in
place of the old `{dst,src}_plane: Option<Residency>`. `cpu_ce`'s `read_plane`/`write_plane`
take a `PlaneAddr`, so writing at `CeSubCopy::dst` no longer compiles; `CeSubCopy::dst` stays
a VA because the `HostCe` arm submits it to a host VAS. Two `compile_fail` doctests, each
paired with a compiling twin one line different, hold the separation
(`error[E0308]: mismatched types … expected struct 'PlaneAddr', found struct 'GpuVa'`).

★ **Two corrections fell out of it, and both were latent defects rather than churn:**

1. `IntervalMap::spans` / `AddressTable::spans` now return each run's **offset into its
   range**. The doc claimed the offset was *"already inside that binding"*; it was not, and
   that sentence is why the physical address was unavailable at the one seam that needed it.
   `gpga::viewers_of`'s compensating second `lookup` is deleted in favour of it.
2. The span **merge** now requires the two places to be **contiguous**, not equal. Comparing
   `Residency` alone merged two spans lying in *different, non-adjacent* bindings that
   happened to share an aperture — harmless while the executor used the VA (the host MMU
   re-walks each page) and a **write past the end of the first backing** the moment it uses
   the plane address.

The missing test universe is closed with four cases in `tests/tests/cpu_ce_executor.rs`,
including one at a **non-zero offset into a binding** (an executor that resolved the binding
and then used its *base* passes the other three) and `mem_mgr.c:467-470`'s readback compare
over a virtual source. Bite-checked in both halves.

#### 2. ⊘⊘ THE REFUTATION — the completion word we could decode was the WRONG ONE (`e95c04a`)

`[src]` `ogkm-580: channel_utils.c` — **one** CeUtils pushbuffer block releases **two**
semaphores:

| word | transport | address | meaning |
|---|---|---|---|
| finishPayload | **engine class**: `SET_SEMAPHORE_A/B/PAYLOAD` + `LAUNCH_DMA.SEMAPHORE_TYPE` (`:645, 671-673`; `:832, 838-840`) | `pbGpuVA + finishPayloadOffset` | *the copy retired* — what `channelWaitForFinishPayload` spins on |
| host semaphore | **host FIFO**: `SEM_ADDR_LO…SEM_EXECUTE` (`:698-746`) | `pbGpuVA + semaOffset` = **4 bytes lower** (`:250`) | *HOST has read all the methods* |

`PushMethod::SemRelease` decoded **only the second**. `[measured 2026-08-08, boot
`run_p35_84d857d`]` those two words are guest RAM `0x2f2c_b000` and `0x2f2c_b004` on the
walling channel, and the guest's own probe printed `semaVA=0x42006c004 semaOffset=0x6c000`
(§14.11). ⇒ **an executor fed from the releases the port could already decode would have
advanced `…b000`, reported a completion, and left the guest spinning on `…b004`** — `#12`'s
where-mistake displaced by four bytes, with our own counters agreeing it was served. The
brief for this increment named `0x2f2cb004` correctly and the wiring as specified would
still have written the other word.

`kayfabe_arch::CeCompletion` is carried **inside** `PushMethod::CeLaunchDma`, not beside it:
the engine writes the payload *after* the copy retires, and a sibling `PushMethod` would be
an unordered fact an executor could serve first. Three refusals take the **whole launch**
down (four-word/with-timestamp, conditional-interrupt, and a release whose registers were
never latched) — reporting the copy and dropping its release moves bytes and leaves the
guest waiting forever on work we really did.

★ **And it found a defect in our own compiled oracle.** `pushbuffer_abi_oracle.c`'s `base`
`LAUNCH_DMA` word carried `_SEMAPHORE_TYPE, _RELEASE_ONE_WORD_SEMAPHORE` while **neither**
`emit_ce_run_parts` nor `emit_ce_fill_run` ever pushed `SET_SEMAPHORE_A/B/PAYLOAD` — it had
copied `rm::ce_pushbuffer`'s flags without its method runs, so every accepted case asked a
real engine for a release into whatever those three registers happened to hold. Invisible for
exactly one reason: nothing read the field. The moment `ce_completion` did, **every launch in
the corpus stopped firing** — the decoder being right about a harness that was wrong.
(`rm::ce_pushbuffer` itself is correct; it pushes the registers *and* sets the flag.)

#### ⊘ 3. What item (c) STILL owes — and the four obstacles below are each `[src]`, a read of this tree

The wiring was **not** attempted, because §14.14 item 2's *"a design decision, not a wiring
detail"* is understated. `[src]`, a read of the doorbell path at `97fe402`:

1. **Nothing in the adapter calls the parse/forward path at all.** `grep -rn
   'parse_pushbuffer|submit_ring|forward_ce|cpu_ce::' crates/kayfabe-qemu-raw/src/` returns
   **0**. `SharedDoorbell::ring` reaches `SharedDevice::doorbell` and nothing else, and it
   holds no `Vmm` — the executor's guest-RAM port does not exist on that path.
2. **The two consumers want an `AddressTable`; the walling channel has no `Vas`.**
   `partition_ce` takes `Option<&AddressTable>` and `write_completion` takes `&AddressTable`,
   while the only resolution route for this channel is `ceresolve`'s published-root walk
   (§14.13). Either the walk's answers reach a table (a TLB fill at the demand — which has to
   be argued against §7 rule 6's *"never cache the walk"*, not assumed compatible) or an
   **operand-resolver seam** is introduced that both consumers take. The second keeps rule 6
   intact and is the cheaper of the two to justify; neither is a wiring change.
3. **`RegPlane` holds BOTH stores under one lock** (`PlaneState::fb` + `PlaneState::ram`),
   which is the good news — but they are a `FbStore` and a `kayfabe_gsp::GuestRam`, while
   `cpu_ce` takes `&mut dyn Vmm`, and `write_completion`'s `raise_irq(COMPLETION_VECTOR)` has
   **no port on the plane at all**. So the executor cannot simply be called from where the
   bytes are; either the guest-RAM port is unified across the two traits, or the driver runs
   where the `Vmm` is and the plane hands out its stores.
4. **The pushbuffer decode needs a per-channel `MethodState`**, which lives in
   `Proc::channels` — unreachable for a channel that has no `Vas`. A submission-local
   accumulator is *probably* right for CeUtils (RM pushes the whole block every time) but it
   is a behavioural difference that must be stated, not assumed.

⊘ Landing any subset of these without the rest reproduces §14.8 exactly: a doorbell that
reports **Served** over work that did not happen. The wall is unchanged, and `NoVas(ChanId(1))`
is still the correct refusal until all four are closed together.

### 14.16 ★★★ E10e item (c), part 3 — the WIRING, all four obstacles closed together (2026-08-08)

⊘ **`[built]`, not `[measured]`.** Everything below is a property of the tree and of its
tests; no boot of this revision existed when it was written. §14.8's rule is why the four
had to close in one increment: any subset produces a doorbell reporting **Served** over work
that did not happen.

#### ⚠ Attribution correction, for §14.15's obstacle 2 and for commit `8cdde02`

`8cdde02`'s subject says the operand-resolver seam was taken *"as the owner ruled"*. **It was
not an owner ruling.** It is the **coordinator's** call, *derived* from
`gmmu_publication_discipline.md` §7 rule 6 (*"never cache the walk"*) plus the fact that rule
7 — the rule that would bound such a cache — is **vacuous on this path** (§5 measured both
invalidate transports at zero, so there is no event to invalidate a cache with). The owner's
standing rulings here are three and none of them is this one: residency must not become a
retrofit, reuse existing code, and ⊘ no research-artifact hacks.

★ The distinction is the claim ledger's whole purpose: **a judgement attributed to the owner
reads as settled and a future reader will not re-examine it.** This one is derived, and it
is re-openable if the seam ever turns out to cost more than the cache would. ⊘ `8cdde02`
itself is not rewritten — history-rewriting in this shared tree has already cost a rescue.

#### The four obstacles, and what closed each

| # | §14.15's obstacle | closed by |
|---|---|---|
| 1 | *nothing in the adapter calls the parse/forward path; `SharedDoorbell` holds no `Vmm`* | `SharedDoorbell::try_ce_submission` + `CeShellState`, the shell state installed at `attach_ram` beside `MachineRam` — **the same `QemuVmm` handle, cloned**, so a copy's bytes and its finishPayload travel by one description of guest memory |
| 2 | *the two consumers want an `AddressTable`; this channel has no `Vas`* | `kayfabe_fwd::OperandResolver` (`8cdde02`) with `kayfabe_rt::ceutils::WalkOperands` as its second implementation — the published-root walk, storing nothing |
| 3 | *`RegPlane` holds both stores under one lock; `cpu_ce` takes `&mut dyn Vmm`; `raise_irq` has no port* | the owner's **second** option: *"the driver runs where the `Vmm` is and the plane hands out its stores"*. `RegPlane::ce_session` → `CePlane { resolve, fb }`; `cpu_ce`'s signature is **unchanged**, so `raise_irq` goes through the real hypervisor port and no new one was invented |
| 4 | *the decode needs a per-channel `MethodState`* | a **submission-local** accumulator, stated as a claim about RM rather than assumed: `[src] channel_utils.c:806-990` builds each CeUtils block whole — both operands, both phys-mode targets, the remap registers, `LINE_LENGTH_IN`, the semaphore registers and `LAUNCH_DMA` — every time. ⚠ Not a general answer; a channel that latched once and fired many times would need the per-channel state, and the codec's own un-latched refusals are what would make that visible |

#### ★★★ The lock order is the reason the driver lives in the shell, and it is `[src]`

`apply_pushbuffer` runs under the device read lock **and** the issuing proc's mutex. The
resolution this channel needs comes from `RegPlane` — and the plane's command-policy chain
**already calls into the core under the plane's own mutex** (`kayfabe_rmrpc::policy::Bridge`
is boxed into `PlaneState::policy`). So **plane→core is the established order**, and
resolving from inside the core's act phase would be its inversion: a guest-buildable ABBA
(`l1_os_shell.md` §6.3), constructed on purpose.

⇒ The executor runs off `DoorbellPort::ring`, which `RegPlane::write` documents as being
called with **no plane lock held**, and takes the two in the sanctioned order — the core
first (`ce_channel_facts`, completed and released), the plane second (one `ce_session` for
the whole submission, with `SharedDevice::with_pushbuffer` at rank 0 inside it).
⊘ This is not a preference. It is why item (c) could not have been *"call `parse_pushbuffer`
from the doorbell"*, which is what §14.7's ordering note (c) had assumed.

#### ★ Three things the wiring had to decide, each with a wrong alternative

1. **Which entries a doorbell consumes.** The honest source is the channel's USERD `GPPut`
   (`[src] channel_utils.c:523`) and this port does not know where this channel's USERD
   lives. So a per-channel **cursor** answers *"which have we already run"* and the ring's
   own encoding answers *"is there more"* — an unwritten entry is zero and decodes to
   nothing, because RM zero-initialises the buffer (`TRANSFER_FLAGS_SHADOW_INIT_MEM`).
   ★ The arithmetic corroborates from the other side: RM writes the entry at
   `lastSubmittedEntry` (0 on a fresh channel) naming method block `putIndex =
   lastSubmittedEntry + 1`, i.e. `pbGpuVA + 1 × 0x64` — and §14.13's boot printed exactly
   `gp0=0x420000064+0x60`.
   ⊘ **The cursor is returned in the success value, never advanced through a `&mut`.** A
   cursor advanced by a submission that then refused would skip the entry it could not run —
   `#13`'s `CE-DROP` by another route — and making it structural means no caller can commit
   one by accident.
2. **A `HostCe` span on this path is a REFUSAL.** `execute_ours_spans` *skips* a `HostCe`
   sub-copy (it is the isolate's) and this driver has no isolate path, so a silent skip would
   move some bytes, release the semaphore and report a completion over a partly-done copy —
   §14.8's exact shape. It refuses by name instead.
3. **A doorbell that brought no readable entry is NOT served**, and neither is one that
   decoded no launch. The guest rang for work; finding none and saying "served" is the silent
   no-op the whole increment exists not to produce.

#### The evidence the wire now carries (ABI **18 → 19**)

`DoorbellReport::ServedLocally` — a **third** arm, not a `Served` with `host_token: 0`. A
zero in a field whose documentation says *"the HOST token that was rung"* is exactly the
*"counted as a doorbell, went nowhere, looked fine"* shape this crate's doorbell doctrine
forbids. On the wire it is `KAYFABE_DOORBELL_SERVED_LOCAL` plus
`KayfabeRegAudit::doorbell_local_serving`, which carries the **last** local serving's
sentence (last, where the refusal is first: a refusal is a diagnosis, a serving is progress,
and `memmgrTestCeUtils` issues a `MemSet` and *then* a `MemCopy`).

#### Coverage, and the two bite-checks

`tests/tests/e10e_ceutils_doorbell.rs` drives RM's own memset block — remap registers, no
`OFFSET_IN_*`, `LINE_LENGTH_IN` alone, the engine-class `SET_SEMAPHORE_A/B/PAYLOAD` run, then
`LAUNCH_DMA` — through a real `Ga10xPushbuffer`, a real `Ga10xGmmu` tree in a `SparseFb`, and
a real published root, at `phys != va` throughout.

- ★★★ **The completion word.** The host-FIFO semaphore four bytes below the finishPayload is
  **poisoned** and asserted untouched. `[bite]` changing the release to `c.addr - 4` reddens
  `a_ceutils_memset_on_a_vas_less_channel_fills_the_resolved_page_and_releases_the_finish_payload`
  and nothing else — restored.
- ★★★ **The doorbell join.** `[bite]` dropping `hObject` from `published_root`'s key reddens
  four tests across three files — `a_channel_with_no_published_root_opens_no_session`,
  `a_pair_that_published_nothing_gets_no_root_and_no_neighbours`,
  `the_root_is_keyed_on_hobject_as_well_as_hclient` — restored.

#### ⊘ What is still owed

A **boot**. `only_live_boots_are_proof`: none of the above is a claim about a guest, and the
acceptance is `memmgrTestCeUtils`' own readback compare at `ogkm-580: mem_mgr.c:467-470` —
which no forged completion can satisfy, because it reads the real bytes the copy did or did
not move.

### 14.17 ★★★★ E10e item (c), BOOTED — **THE CE WALL IS GONE.** `memmgrTestCeUtils` passes on a STOCK guest (`754e393`)

`[measured 2026-08-08]`, vast GA106 bench (`vh`, RTX 3060 `10de:2504`, host driver
**580.159.04 Open**, 38 cores), source revision **`754e393`** verified by
`strings … | grep -o 'kayfabe-rev:[0-9a-f]*'` on **both**
`target/release/libkayfabe_qemu_raw.a` and `qemu-build/qemu-system-x86_64` →
`kayfabe-rev:754e393828a1a61f3ea2b67d3d54744b1ea37693` in each. Boot `p35_754e393`, probe set
`[35]`, **STOCK** guest module (`(dvs-builder@U22-I3-AF04-09-6) Wed Apr 29`, the `.run`'s own
banner — ⊘ no `KAYFABE-BRINGUP` lines, no patch, this is the milestone's configuration).
Evidence: `docs/reference/bench_evidence/run_p35_754e393_{dmesg,qemu,probe,serial}.log`.

#### The A/B, one variable

| | `p35_84d857d` (§14.13) | `p35_754e393` |
|---|---|---|
| doorbells | 1 arrived, 0 served, **1 REFUSED** `NoVas(ChanId(1))` | **2 arrived, 2 SERVED, 0 refused** |
| guest wall | `memmgrMemSet(… PREFER_CE) NV_ERR_TIMEOUT (0x65) @ mem_mgr.c:463` | ⊘ **`mem_mgr.c` does not appear in the log at all** |
| `RmInitAdapter failed!` | `(0x25:0x65:1249)` | `(0x43:0x59:2239)` |
| guest dmesg | 22 lines | **39 lines** |

Same box, same guest image, same stock module, same probe set. The only variable is the
revision.

#### ★★★ The two servings, and the second one is the acceptance

```
DOORBELL token 0x00010002 at +0xbb0090 SERVED-LOCAL [CpuCe::ServedLocally]   ×2
last CPU-CE serving: cpu-ce: 1 gp, 9 methods, 1 launch, 1 span, 4 B, 1 sem
                     fin va=0x42006c004 -> S:0x2d68004
```

- **Token `0x00010002`** is the walling channel's own `workSubmitToken`, the one §14.11's
  guest probe printed — so these are provably that channel's rings, not another's.
- **Two** submissions: `memmgrMemSet(vid, 0, sizeof vidmemData)` at `mem_mgr.c:463`, then
  `memmgrMemCopy(sys ← vid, 4 bytes, PREFER_CE)` at `:467`. ★ The last serving moved
  **4 B** — `sizeof vidmemData` exactly — which is the *copy*, i.e. the memset had already
  retired and the guest had gone on.
- **`fin va=0x42006c004 -> S:0x2d68004`** — the finishPayload, at the VA the guest itself
  printed (§14.11: `semaVA=0x42006c004`), resolved through the published root into **guest
  RAM**. ⊘ Not the host-FIFO word four bytes lower. Had we advanced that one, the counters
  above would read exactly the same and the guest would still be at `mem_mgr.c:463`.

#### ★★★★ `memmgrTestCeUtils`' OWN READBACK COMPARE PASSED

This is the acceptance §14.6 stated and §14.14 sharpened, and it is met **by absence**:
`memmgrTestCeUtils` ends `memmgrMemRead(sys) → NV_ASSERT_TRUE(sysmemData == vidmemData)`
(`ogkm-580: mem_mgr.c:467-470`) and **no `mem_mgr.c` assertion appears in the boot at all** —
neither the `:463` timeout that walled every previous revision nor the `:470` compare. The
boot ran on for **thirteen more seconds** into `RmInitAdapter`'s graphics bring-up.

⊘ **No forged completion can produce this.** The compare reads back the bytes through the
guest's own CPU mapping; a semaphore advanced over a copy that did not happen leaves
`sysmemData` at its `0x11223345` seed and fails `:470` loudly. That the guest read what our
CPU executor wrote is what the silence means.

#### The NEW wall, and it is a different subsystem

```
NVRM: … kgrobjPromoteContext(...) @ kernel_graphics_object.c:224      NV_ERR_NOT_SUPPORTED
NVRM: … AllocWithHandle(..., classNum, NULL, 0) @ kernel_graphics.c:2519
NVRM: … kgraphicsCreateGoldenImageChannel(...) @ kernel_graphics.c:508
NVRM: rpcRmApiAlloc_GSP: GspRmAlloc failed: … hClass=0x0000c36f … status=0x00000056
NVRM: GPU 0000:00:03.0: RmInitAdapter failed! (0x43:0x59:2239)
```

Every one is `NV_ERR_NOT_SUPPORTED (0x56)` — an alloc or control **our RPC plane refused by
name**, not a timeout and not a wrong answer. The wall is now the **graphics golden-context
channel**, which is the boundary `mode2_fakeboot_complete.md` already names as *"GR golden-ctx
= silicon boundary → forward to host"*. ⊘ It is not this increment's, and nothing about the CE
plane is implicated in it.

#### `nvidia-smi`, verbatim, and why

```
Unable to determine the device handle for GPU0: 0000:00:03.0: Not Found
No devices were found
SMI_RC=6
```

`RmInitAdapter` failed, so the adapter never initialised and `nvidia-smi` finds no device —
⊘ **exactly as expected at a GR wall, and not a CE regression.** The device nodes *are*
created (`/dev/nvidia0`, `/dev/nvidiactl`, `/dev/nvidia-uvm`, `nvidia-caps/`), and both
modules are loaded (`nvidia`, `nvidia_uvm`); it is the adapter behind them that has no GR
context. `nvidia-smi` is the **next** milestone's oracle, not this one's.

#### What this does and does not establish

- ✔ The four obstacles closed together, on a real guest: a VAS-less channel's ring,
  pushbuffer, operands and finishPayload all resolved through the published-root walk, its
  bytes moved by the shell CPU executor, and the word the guest polls advanced.
- ✔ `[measured 2026-08-08, boots p35_754e393 and p35_754e393_b, rev 754e393]` the
  completion address is right, by the strongest oracle available: the
  guest's own readback compare and the fourteen seconds of boot after it.
- ⊘ It establishes **nothing** about the isolate's `HostCe` branch, which this path refuses
  by name and never took, and nothing about a multi-launch or multi-entry submission — the
  boot's two doorbells were one GPFIFO entry and one `LAUNCH_DMA` each.

#### ★★ REPRODUCED, and the difference between the two runs is the corroboration

`[measured 2026-08-08]` a second boot at the same revision, `p35_754e393_b`
(`docs/reference/bench_evidence/run_p35_754e393_b_*.log`): **byte-identical** guest dmesg,
39/39 lines with timestamps stripped, same `2 arrived, 2 served, 0 REFUSED`. ⊘ Recorded
because `#13` is the standing proof that *"it worked once"* is not a result on this bench.

★ And the one thing that **differs** between the two runs is the evidence that nothing here
is a constant: the finishPayload resolved to guest-physical `0x02d6_8004` in the first boot
and `0x22c5_9004` in the second, for the *same* GPU virtual address `0x4_2006_c004`. The
guest allocates a different page each boot and the walk followed it — a hard-coded address, a
cached translation or a lucky numerical coincidence would have produced the same number
twice.

#### ★ The `cuda.h` errand, done — and its trap, `[measured 2026-08-08, bench guest on vh, boot cudahdr, rev 754e393]`

`[measured 2026-08-08]` NVIDIA's own driver-API header is now at
**`/usr/local/include/cuda.h`** in the bench guest (`CUDA_VERSION 12090`, 1 168 299 bytes),
extracted host-side from `cuda-cudart-dev-12-9_12.9.79-1_amd64.deb` and `scp`'d in.
⊘ **No CUDA toolkit was installed** — one header, out of one `dpkg-deb -x`, into
`/usr/local/include` — and the guest's network was not reconfigured.

⚠ **The trap is real, and here is the guest's own `find`:**

```
/tmp/cuda.h
/usr/include/linux/cuda.h                                  ← the PowerMac ADB controller
/usr/src/linux-headers-6.8.0-136/include/linux/cuda.h       ← the same, twice
/usr/src/linux-headers-6.8.0-136/include/uapi/linux/cuda.h
/usr/local/include/cuda.h                                   ← NVIDIA's, and it is LAST
```

Three of the five hits are `linux/cuda.h`, the **Apple PowerMac ADB microcontroller** header,
and they sort *ahead* of the real one. An existence check passes on any of them and the
compile then fails on a missing `CUdevice`. ⇒ Check the **content**
(`grep -q CUDA_VERSION`), never the path. `gcc -fsyntax-only -I/usr/local/include` over a
translation unit whose only line is `#include <cuda.h>` succeeds, which is the property the
next rung needs and the only one this errand claims.

### 14.18 ★★★ MEASURED: the scrubber binds **COPY2**, and COPY2 **has** a non-stall vector (2026-08-08, `5a035e0`)

Two boots, both artifacts stamped `kayfabe-rev:5a035e0d22721d4b8fc974afa851e657dffbabbe`
(`libkayfabe_qemu_raw.a` and `qemu-system-x86_64`), bench serialised.

**The question.** §14.17 left index-35 arming refused because this device delivers no event.
A stronger ground was available *if* the guest's scrubber landed on CE0 or CE1: the captured
`GA106_INTR_TABLE` gives `MC_ENGINE_IDX` 15 (CE0) and 16 (CE1) `vectorNonStall = INVALID`,
so the refusal would rest on hardware's own authority that there is no vector to raise, and
any delivery path would have been built for a vector that does not exist.

**Why it could not be reasoned out.** `ceutilsGetFirstAsyncCe` takes the first CE that is
not a GRCE (`ogkm-580: ce_utils.c:66-81`). On GA106 `ceIsCeGrce` does **not** read a GRCE
mask register — `kceGetGrceMaskReg` is halified to the `NV_ERR_NOT_SUPPORTED` stub for
everything below GB202 (`ogkm-580: generated/g_kernel_ce_nvoc.c:847-858`) — so it falls
through to the partner-list walk (`kernel_ce_shared.c:76-135`) over **the device-info table
this port serves**. Deriving the pick from our own table is circular. It had to be read off
the wire, which is what `kayfabe_device::census::ChannelBind` (`5a035e0`) was built to do.

**The answer**, boot `cup2_p35`:

```text
nvkvm: channel binds (0xa06f0104): 2 total, 2 distinct
nvkvm:   bind engineType 11 (COPY2) client 0xc1e00006 object 0x00000002 result 0x0 x1
nvkvm:   bind engineType 11 (COPY2) client 0xc1e00011 object 0x00000002 result 0x0 x1
```

`NV2080_ENGINE_TYPE` 11 is `COPY2` (`COPY0 = 9`) ⇒ `ceId = 2` ⇒ `MC_ENGINE_IDX` 17 ⇒
**`vectorNonStall = 0x07`**. Replicated on boot `cebind_p35` (one bind, same engine) and by a
*second, different* client here — RM's `RmInitAdapter` scrubber and CUDA's own — so it is not
a property of one caller.

⇒ **A vector exists.** The refusal cannot be re-grounded on the table's silence. The honest
sentence is now *"the vector is published and this device does not yet raise it"*.

#### ⚠ The second measurement `[measured 2026-08-08, boot `cebind1` at `5a035e0`]` — it constrains the ORDER of the remaining work

Boot `cebind1`, the same revision in **shipping** configuration:

```text
nvkvm: probe-arm set: EMPTY (shipping configuration: every non-silent notifier arming refused)
nvkvm:   arming event 35 action 2 client 0xc1e00005 object 0x0000000b result 0x00000056 x1 REFUSED
nvkvm: channel binds (0xa06f0104): 0 total, 0 distinct
```

**The bind is downstream of the arming.** `NVA06F_CTRL_CMD_BIND` is sent at
`ogkm-580: mem_utils.c:1966`, *after* the `0x20800301` at `:1930` in the same function, so
refusing the arming bails `_memmgrMemUtilsScrubInitScheduleChannel` before the guest ever
names a copy engine. ⇒ The CE2 reading is observable **only under probe `[35]`** — not a
caveat on it, but the correct conditional: the question was always *"if we serve the arming,
is there a vector for the CE the guest then binds?"*

#### The remaining piece is smaller than it looks — one MSI-X vector, guest demuxes

⊘ `vectorNonStall = 0x07` is a **GPU interrupt-tree vector**, not an MSI-X index. This device
already delivers **one** message and lets the guest's ISR demultiplex through the TOP/LEAF
pending bits (`nvkvm.c`, `NVKVM_STALL_VECTOR`, and the C makes the same choice at
`C: src/qemu/nvkvm_gpu_emul.c:4386-4388`). So the wiring is:

1. `CpuIntrTree` gains a latch entry point that is not a guest `LEAF_TRIGGER` write —
   today `write(LeafTrigger, v)` is the only producer of a pending bit.
2. At the CE completion moment — `DoorbellReport::ServedLocally`, i.e. the copy really ran —
   resolve the ringing channel's bound engine to its `vector_non_stall` and latch it.
3. `PlaneState::ring_doorbell` returns a `WriteOutcome`, which **already carries
   `raise_cpu_intr`**, and a doorbell **is** a register write. ⇒ no ABI change, no new shell
   code, no second wire.
4. Only then may 35 be served — and it must be served because delivery happens, never as an
   entry in `SILENT_NOTIFIERS`.

The one fact step 2 still owes: the doorbell report must be able to name the engine the
ringing channel was bound to. `Gpu::bind_channel` records it; the doorbell path does not read
it yet.

#### ⊘ Where the ladder actually stops today `[measured 2026-08-08, boot `cup2_p35` at `5a035e0`, probe `[35]`]` — a measurement, NOT a rung

`cup2` builds clean in the guest (`gcc -O0 … -lcuda`, real header at `/usr/local/include/cuda.h`,
found by content) and stops on its **first** call:

```text
FAIL cuInit(0) -> no CUDA-capable device is detected (100)
```

The guest's own dmesg names the cause, and it is upstream of `GPU_PROMOTE_CTX` rather than at
it — the golden-image channel never gets built:

```text
NVRM: Assertion failed: … returned from pRmApi->AllocWithHandle(…, classNum, …) @ kernel_graphics.c:2519
NVRM: Check failed: … returned from kgraphicsCreateGoldenImageChannel(…) @ kernel_graphics.c:508
NVRM: rpcRmApiAlloc_GSP: GspRmAlloc failed: … hClass=0x00000070 … status=0x00000056
NVRM: rpcRmApiAlloc_GSP: GspRmAlloc failed: … hClass=0x0000c36f … status=0x00000056
NVRM: Check failed: … returned from kgrobjPromoteContext(…) @ kernel_graphics_object.c:224
NVRM: rpcRmApiAlloc_GSP: GspRmAlloc failed: … hClass=0x0000402c … status=0x00000056
NVRM: rpcRmApiAlloc_GSP: GspRmAlloc failed: … hClass=0x00002081 … status=0x00000056
```

Device side, same boot: `bridge refusals: 14 total, 3 distinct` —
`AllocClassNotPermitted::NotOnAllowlist ×4`, `UnmappedAllocClass ×3`, `FreeUnknown ×7` — plus
`60 UNSERVICED` commands over 25 distinct ids. ⇒ `GPU_PROMOTE_CTX` **does** bite (it is in the
list) but it is not the first thing to fix: four alloc classes are refused before it.

#### And the shipping-configuration boot, stated so it cannot be misread

Boot `cebind1`, `probe-arm set: EMPTY`, stock guest module:

```text
=== nvidia-smi (the device open) ===
No devices were found
SMI_RC=6
```

with `NVRM: RmInitAdapter failed! (0x25:0xffff:1249)`. ⊘ **`nvidia-smi` does NOT print a
device with the probe empty.** The milestone has not been reached; §14.17's wall is still the
index-35 arming, exactly where it was.

### 14.19 ★★★ §14.18 STEP 1 BUILT — the completion is **announced**, and index 35 is served from a second list

⊘ **`[built]`, not `[measured]`.** Everything below is a property of the tree and of its
tests; it is **unmeasured** on hardware — no boot of this revision existed when it was
written, and nothing here may be cited as a boot result. `only_live_boots_are_proof`.
(§14.20 is the boot, and it refutes part of the motivation below.)

#### What landed, in the four pieces §14.18 named

| piece | where |
|---|---|
| a latch entry point that is not a guest `LEAF_TRIGGER` write | `CpuIntrTree::latch` — the **same body** as the trigger, shared rather than duplicated, because the guest's ISR cannot tell the two apart and neither may the pending state |
| the engine the ringing channel was bound to | `CeChannelFacts::bound_engine`, read off the **same proc** the token routed in (`ExecPlane::bound` is keyed `(proc, chan)`) and carried on `DoorbellReport::ServedLocally { engine }` |
| engine → vector | `kayfabe_device::nonstall::non_stall_vector` — `RM_ENGINE_TYPE` → `MC_ENGINE_IDX` → the chip's captured `vectorNonStall` |
| raise | `RegPlane::announce_completion`, off `WriteOutcome::raise_cpu_intr`. ⊘ **No new shell code and no new wire**: a doorbell is a register write and the shell already delivers on that flag (`nvkvm.c:381`) |

#### ★★★ Index 35 is admitted by a **second list**, and the two lists must never merge

`SILENT_NOTIFIERS` accepts an arming because the event *cannot occur*. `DELIVERED_NOTIFIERS`
— new — accepts one because it *does* occur and this device raises it. ⊘ Moving 35 into the
first would keep a sentence that no longer supports it, which is why
`the_delivered_list_is_exactly_index_35_and_is_disjoint_from_the_silent_one` asserts the two
are disjoint and that every delivered row's argument names the vector it raises.

#### ⊘ The promise is AUDITABLE, not asserted — and one number must be zero

Every local serving lands in exactly one of `nonstall_raises` or `nonstall_unvectored`
(`every_local_serving_is_either_announced_or_counted_as_unannounced`). The second counts work
that happened and was never announced: **its healthy value is 0**, and every boot prints it.

★ `nonstall_masked` is the third, and it is the one that will pay for itself: the guest's own
non-stall scan is `intrReadRegLeaf(j) & intrReadRegLeafEnSet(j)`
(`ogkm-580: intr_nonstall_tu102.c:253-255`), so a vector latched into a leaf the guest has not
enabled is **invisible to its ISR even though the message was delivered** — and without the
counter that hang is byte-identical to never having raised. ⊘ This device still raises
regardless of the enables (`cpuintr`'s standing decision) and records the disagreement.

Wire ABI **20 → 21** for the three counters, plus the shell's own
`nvkvm: completions: …` line.

#### ⊘⊘ A DEFECT this increment found, and it was made dangerous by this increment

`RegPlane::device_reset` did **not** reset `PlaneState::cpu_intr`, and `RegPlane::residue`
recorded the reason as *"the tree's own arrays are transient — every bit the guest sets it
clears again in the same ISR"*. That was true while the only producer was
`_osVerifyInterrupts`' loopback, which clears its own bit before returning.

★ It is **false** of a completion vector: this device latches it, and only a guest that lives
long enough to run `_intrServiceNonStallLeaf_TU102` clears it. A guest that resets in between
hands the next one a `MC_ENGINE_IDX_CE2` bit pending for a copy that never happened in its
life — a **fabricated completion notification, across a device life**, from the one path this
whole increment exists to keep honest. Fixed (`device_reset` rebuilds the tree; it is also the
register block's own stated reset, `ampere/ga102/dev_vm.h:52,56,60`), and the residue comment
is corrected rather than deleted.

#### Coverage

`crates/kayfabe-device/tests/completion_notification.rs`, written against the three ways the
promise can break: announcing work that did not happen, not announcing work that did, and
announcing it where the guest cannot see it. The chip's own table is read independently
(`the_vector_under_test_is_the_one_this_chips_captured_table_publishes_for_ce2`) so a table
edit reddens before every other assertion silently changes meaning.

`[bite]` **7/7 bite**, `mutate_the_refusals_not_the_mechanism`-style — announce a forwarded
doorbell too; drop the unvectored counter; hard-code CE2's vector; let a pending bit survive a
reset; stop admitting the delivered list; decode an `INVALID` row to zero; stop recording
`masked`. Each reddened its own test and nothing else; all restored.

#### ⊘ What is still owed

A **boot**, in the shipping configuration (`probe-arm set: EMPTY`). ⊘ And note what a green
one does *not* establish: the guest's `LEAF_EN` for a CE non-stall vector is set by a path
this port has never observed, so `nonstall_masked` is the number to read first — a boot with
`raises > 0, unvectored = 0, masked > 0` is a delivered message the guest's own scan will
never attribute, and it looks exactly like success from every other angle.

### 14.20 ★★★★ BOOTED, SHIPPING CONFIGURATION — **`nvidia-smi` prints a device with the probe EMPTY** (`7a881a7`)

`[measured 2026-08-08]`, vast GA106 bench (`vh`, RTX 3060 `10de:2504`, host driver
**580.159.04 Open**), source revision **`7a881a7`** verified by
`strings … | grep -o 'kayfabe-rev:[0-9a-f]*'` on **both**
`target/release/libkayfabe_qemu_raw.a` and `qemu-build/qemu-system-x86_64` →
`kayfabe-rev:7a881a7e7ee7054f09a7965baad68414e7280ce3` in each. Boots `ship_7a881a7` and
`ship_7a881a7_b`, **`probe-arm set: EMPTY`**, **STOCK** guest module
(`(dvs-builder@U22-I3-AF04-09-6) Wed Apr 29`, the `.run`'s own banner — no patch, no
`KAYFABE-BRINGUP` lines). Evidence:
`docs/reference/bench_evidence/run_ship_7a881a7{,_b}_{dmesg,qemu,probe,serial}.log`.

#### `nvidia-smi`, verbatim

```text
Sat Aug  8 14:05:01 2026
+-----------------------------------------------------------------------------------------+
| NVIDIA-SMI 580.159.04             Driver Version: 580.159.04     CUDA Version: 13.0     |
+-----------------------------------------+------------------------+----------------------+
| GPU  Name                 Persistence-M | Bus-Id          Disp.A | Volatile Uncorr. ECC |
| Fan  Temp   Perf          Pwr:Usage/Cap |           Memory-Usage | GPU-Util  Compute M. |
|                                         |                        |               MIG M. |
|=========================================+========================+======================|
|   0  ERR!                           Off |   00000000:00:03.0 N/A |                  N/A |
| N/A   N/A  N/A             N/A  /  N/A  |       0MiB /  12288MiB |     N/A      Default |
|                                         |                        |                  N/A |
+-----------------------------------------+------------------------+----------------------+

+-----------------------------------------------------------------------------------------+
| Processes:                                                                              |
|  GPU   GI   CI              PID   Type   Process name                        GPU Memory |
|        ID   ID                                                               Usage      |
|=========================================================================================|
|  No running processes found                                                             |
+-----------------------------------------------------------------------------------------+
SMI_RC=6  →  SMI_RC=0
```

⊘ `RmInitAdapter failed!` **does not appear in the boot at all** — the string is absent from
both 38-line captures, where every previous shipping boot ended on it. The adapter
initialised.

#### ★★ REPRODUCED, and the two runs are indistinguishable

`ship_7a881a7_b` at the same revision: guest dmesg **byte-identical**, 38/38 lines with
timestamps stripped; `SMI_RC=0`; and the device's own report identical line for line
(`2 announced, 0 UNVECTORED, 2 masked`; `1 bind engineType 11 (COPY2)`; both index-35
armings `result 0x00000000`). ⊘ Recorded because `#13` is the standing proof that *"it
worked once"* is not a result on this bench.

#### The device's own report, and the three numbers that matter

```text
nvkvm: probe-arm set: EMPTY
nvkvm:   arming event 35 action 2 client 0xc1e00005 object 0x0000000b result 0x00000000 x1
nvkvm:   arming event 35 action 2 client 0xc1e00006 object 0x0000000c result 0x00000000 x1
nvkvm: channel binds (0xa06f0104): 1 total, 1 distinct
nvkvm:   bind engineType 11 (COPY2) client 0xc1e00006 object 0x00000002 result 0x00000000 x1
nvkvm: doorbells: 2 arrived, 2 served, 0 REFUSED by name; last token 0x00010002
nvkvm: completions: 2 announced (non-stall vector raised), 0 UNVECTORED, 2 would be masked
```

- ★★★ §14.18's ordering prediction **held**: serving the arming is what let the guest reach
  the bind, and the bind names **COPY2** in the shipping configuration — an engine this
  device had never been told about in a probe-empty boot before.
- ★★★ **`0 UNVECTORED`** — every copy this shell performed was announced. That is the
  counter whose non-zero value would be the promise of serving index 35 broken quietly.

#### ⊘⊘ AND IT REFUTES HALF OF WHY WE BUILT IT — `2 would be masked` `[measured 2026-08-08, boots `ship_7a881a7` and `ship_7a881a7_b`, rev `7a881a7`]`

★ **The guest never enabled the leaf.** Vector `0x07` is leaf 0, bit 7; both announcements
were latched with `LEAF_EN(0)` bit 7 clear, so the guest's own non-stall scan —
`intrReadRegLeaf(j) & intrReadRegLeafEnSet(j)`, `ogkm-580: intr_nonstall_tu102.c:253-255` —
**cannot see them**. The message was delivered and the ISR could not attribute it.

⇒ **The boot did not consume the notification, and it succeeded anyway.** So the scrubber
needed the arming to be *accepted*, not the event to be *received* — which is exactly what
§14.18's `[measured]` probe boot had already found (*"it registers and continues"*) and what
this shipping boot now confirms with the delivery in place.

⊘ **What that does and does not change.** It does **not** make the raise optional: accepting
a completion arming while delivering nothing is the promise this repository refuses to make,
and the honest ground for serving 35 is that we raise it. It **does** mean this boot is
**no evidence at all** that the guest receives it — and without `nonstall_masked`, a boot in
which the vector never arrived would have looked exactly like this one. ★ The counter §14.19
added as a hedge is the only reason this paragraph exists rather than an unearned
*"delivery works"*.

⚠ Where the enable comes from is now the open question: nothing in these two boots wrote
`LEAF_EN_SET(0)` bit 7, so either the guest enables CE non-stall leaves on a path
`RmInitAdapter` does not reach, or it enables them per-engine at a point this port has not
served yet. ⊘ Unmeasured; do not assume the first.

#### What this establishes, and what it does not

- ✔ A **stock** NVIDIA driver initialises its adapter against this emulated GPU and
  `nvidia-smi` enumerates the device, with **no probe, no guest patch** — reproduced.
- ⊘ `Name` reads `ERR!` and every telemetry field reads `N/A`: the device is enumerated, and
  the controls behind the name/clock/power queries are not served. That is a **description**
  of what is missing, not a failure of the enumeration.
- ⊘ **Nothing about CUDA.** `kgraphicsCreateGoldenImageChannel` still fails on four refused
  alloc classes (`0x0070`, `0xc36f`, `0x402c`, and `0x2081` in the probe boots), so the GR
  golden context does not exist and `cuInit` is unchanged. ★ Which is itself a correction:
  those four refusals do **not** block the adapter — they were assumed to be upstream of
  everything, and the adapter now initialises with all four still refused.

### 14.21 ⊘⊘ REVERTED: serving `GPU_PROMOTE_CTX` **kills the adapter**, because `NV_ERR_NOT_SUPPORTED` is the only status RM tolerates here (`[measured 2026-08-08, boot ship2_7c5d74d, rev 7c5d74d]`, against `ship_7a881a7`)

`[measured 2026-08-08]`, vast GA106 bench (`vh`, RTX 3060 `10de:2504`, host driver
**580.159.04 Open**), source revision **`7c5d74d`** verified by
`strings … | grep -o 'kayfabe-rev:[0-9a-f]*'` on **both**
`target/release/libkayfabe_qemu_raw.a` and `qemu-build/qemu-system-x86_64` →
`kayfabe-rev:7c5d74dc5c37520ad0a3447be8c18c3649632fed` in each. Boot `ship2_7c5d74d`,
**`probe-arm set: EMPTY`**, **STOCK** guest module. Compared against `ship_7a881a7` (§14.20).

#### What was built, and it worked exactly as designed

`NV2080_CTRL_CMD_GPU_PROMOTE_CTX` joined `OBJECT_CONTROLS` with an arm routing through
`Bridge::deliver` — the third `a_table_does_not_decide_behaviour`: `control_params` decoded
it, `translate_promote_ctx` translated it, `Bridge::deliver` had an apply arm and
`Gpu::promote_ctx` performed it, and `respond_control` gated on a list that did not name it.
Refusals answered `NV_ERR_INVALID_OBJECT_HANDLE` (`0x33`) or `NV_ERR_INVALID_STATE` (`0x40`),
per this repo's standing rule that `0x56` is the FSM's *"nobody claimed this"* signature and
must never be reused for a decision.

#### ★★★ The A/B, one variable, and the diff is FOUR LINES

Guest dmesg, `ship_7a881a7` vs `ship2_7c5d74d`, timestamps stripped — **identical** except:

| | `ship_7a881a7` | `ship2_7c5d74d` |
|---|---|---|
| `kgrobjPromoteContext` @ `kernel_graphics_object.c:224` | `NV_ERR_NOT_SUPPORTED (0x56)` | `NV_ERR_INVALID_STATE (0x40)` |
| `kgraphicsCreateGoldenImageChannel` @ `:508` | `0x56` | `0x40` |
| what follows | the RC watchdog runs, then **17 more lines**, no bail | `RmInitNvDevice: *** Cannot load state into the device` |
| `nvidia-smi` | full device table, `SMI_RC=0` | `SMI_RC=6`, `RmInitAdapter failed! (0x25:0x40:1249)` |

⊘ **We answered a better status and lost the milestone.** Nothing else changed.

#### ★★★ Why, in RM's own source — this is a READING that the boot sent us to find

`gpuStatePostLoad` (`ogkm-580: src/nvidia/src/kernel/gpu/gpu.c:3437-3439`):

```c
// RMCONFIG:  Bail on errors unless the feature/object/engine/class
//            is simply unsupported
if (rmStatus == NV_ERR_NOT_SUPPORTED)
    rmStatus = NV_OK;
if (rmStatus != NV_OK)
    goto gpuStatePostLoad_exit;
```

The golden-image channel is built from the FIFO engine's post-load callback
(`kfifoStateLoad_GM107` → `kfifoTriggerPostSchedulingEnableCallback`,
`ogkm-580: kernel_fifo_gm107.c:229-234`, which returns any error verbatim). So the whole
chain — promote-ctx → `_kgrAlloc` → `kgraphicsCreateGoldenImageChannel` → the FIFO engine's
`StatePostLoad` — lands on that one comparison. **`NV_ERR_NOT_SUPPORTED` is converted to
`NV_OK` and swallowed; every other status is fatal to `RmInitAdapter`.**

⇒ ★★ **The standing rule has a counter-instance — `[measured 2026-08-08, boots ship_7a881a7
and ship2_7c5d74d]` plus the `gpu.c:3437-3439` reading above — and it needs a scope rather
than a repeal.** *"⊘ Never `NV_ERR_NOT_SUPPORTED` for a decided refusal"* is right wherever it was
established — `GPFIFO_SCHEDULE`, `BIND` — because there the guest's error path *reads* the
status. It is **wrong for any control whose failure propagates into an engine's
`StatePostLoad`**, where `0x56` is the only status that keeps the adapter alive and a
"better" one silently converts an enumerating GPU into a dead one. Before choosing a refusal
status, ask **where the caller's error goes**, not only what the control's header documents.

#### The refusal itself was CORRECT, and it names the real gap

`nvkvm: bridge refusal PromoteFault::ContextVasUndeclared x1` — one refusal, the true one.
The golden-image channel's VASpace has declared no page-directory base, which is §14.9's
measurement restated from the other side: the boot issues `SET_PAGE_DIRECTORY` **zero**
times, the boot-path root arrives on `0x90f10106`, and that publication does not reach
`Vas::pdb` in the object model. ⇒ On the boot path **every** promote-ctx can only refuse, so
claiming the control buys nothing and costs the adapter.

#### ⊘ REVERTED, and what that decision is

The claim, the arm and the two status constants are **removed**; the `NV40_I2C` named
refusal and this section stay. The port must not claim a control it cannot perform when a
*decided* refusal is fatal where an *unserviced* one is tolerated — the difference is
`gpuStatePostLoad`'s one comparison, and it is not ours to argue with.

★ The re-enable condition is now exact and testable: **when the `0x90f10106` publication
reaches `Vas::pdb`, promote-ctx SUCCEEDS rather than refuses**, `NV_OK` replaces the refusal,
and the status question disappears entirely. Serving it before then is strictly negative.

#### ⚠ And a bench trap that invalidated the first boot of this section, silently

`scripts/build_qom_shim.sh <src> [<build-dir>]` defaults its build dir to
`<src>/build-nvkvm`; `scripts/bench/boot_nvkvm.sh` hard-codes
`Q=/workspace/bench/qemu-build/qemu-system-x86_64`. ⇒ The **obvious** invocation
(`build_qom_shim.sh /workspace/bench/qemu-10.2.4`) builds a binary the boot harness never
runs, and leaves the previous revision's binary in place — the `862c7c2` stale-artifact
failure, one level up. Boot `ship_7c5d74d` was produced that way and is **void**: its
`run_ship_7c5d74d_probe.log` stamps `kayfabe-rev:7a881a7…` while the tree said `7c5d74d`.
★ The only reason it was caught is that `boot_capture.sh` prints the revision **read out of
the binary it is about to run**, and prints it beside nothing else that agrees with it.
⇒ Always pass the build dir: `build_qom_shim.sh /workspace/bench/qemu-10.2.4 /workspace/bench/qemu-build`.

#### ★ `vh` is now a real gate host

Both vendored open-kernel-modules trees are installed at the default paths
`tests/build.rs` looks for (`/workspace/nvidia-gpu-passthrough/research_clones/{ogkm-580.159.04,ogkm}`,
130 MB + 143 MB). `scripts/ci_gates.sh --all` there now exits **0** with
`ALL GATES CLEAN (23 steps, floor 23 for --all mode)` and an oracle census of
**`SKIPPED=0` in all five families** — `GMMU RAN=15`, `PUSHBUFFER RAN=24`, `TOKEN RAN=3`,
`USERD-CHID RAN=5`, `VBIOS RAN=13`. Previously every family reported SKIPPED and the green
covered no compiled oracle at all (`skipped_oracle_kills_the_guard`).

⚠ **A gate run on `vh` measures `vh`'s tree.** The first `--all` run of this session reported
the claim ledger at 382/67 against a ceiling of 381/66 (`[measured 2026-08-08, gates3.log on
vh, rev 7c5d74d]`) — and the tree it measured was
`7a881a7` plus four hand-copied files, three doc commits behind master. On the real HEAD the
ledger is **381/66/17 with `MEASURED` 882 → 886**. Sync with a bundle before believing a
gate: it is the same species as the stale-binary trap above, and it costs a wrong ratchet.

### 14.22 ★★★ THE ALLOC CLASSES ARE NOT THE WALL — the golden image channel fails **upstream** of all of them (`[measured 2026-08-08, boot ship3_d5369b5, rev d5369b5]`)

`[measured 2026-08-08]`, vast GA106 bench (`vh`, RTX 3060 `10de:2504`, host driver
**580.159.04 Open**), source revision **`d5369b5`** verified by
`strings … | grep -o 'kayfabe-rev:[0-9a-f]*'` on **both**
`target/release/libkayfabe_qemu_raw.a` and `qemu-build/qemu-system-x86_64` →
`kayfabe-rev:d5369b58bda44de90c51c8baf055f889aeb7c71b` in each. Boot `ship3_d5369b5`,
**`probe-arm set: EMPTY`**, **STOCK** guest module. Evidence:
`docs/reference/bench_evidence/run_ship3_d5369b5_{dmesg,qemu,probe,serial}.log`.

#### ✔ First: the milestone REPRODUCES after the host reboot

`SMI_RC=0`, the full `nvidia-smi` device table, `probe-arm set: EMPTY`, stock module —
identical in kind to §14.20's `ship_7a881a7`, now at the reverted HEAD. The census matches
line for line: `completions: 2 announced, 0 UNVECTORED, 2 would be masked`,
`bind engineType 11 (COPY2)`, `doorbells: 2 arrived, 2 served`. ⇒ §14.21's revert restored
the adapter exactly as it predicted, and the recovered box behaves.

#### ★★★ The reordering: the four "blocking" alloc classes block **nothing upstream**

The standing framing was *"`kgraphicsCreateGoldenImageChannel` fails on refused alloc
classes `0x0070` / `0xc36f` / `0x402c` / `0x2081`."* **The guest's own dmesg timestamps say
the opposite**, and the order is unambiguous:

```text
[38.596585] kgrobjPromoteContext                @ kernel_graphics_object.c:224  -> 0x56
[38.639983] AllocWithHandle(… hObj3D, classNum) @ kernel_graphics.c:2519        -> 0x56
[38.683581] kgraphicsCreateGoldenImageChannel   @ kernel_graphics.c:508         -> 0x56
[38.690119] GspRmAlloc hClass=0x00000070  hObject=0x3141590f   <-- AFTER the failure
[39.002705] GspRmAlloc hClass=0x0000c36f  hObject=0x31415900   <-- AFTER the failure
[39.093908] Assertion failed: status == NV_OK @ kernel_rc_watchdog.c:1198
[39.125188] GspRmAlloc hClass=0x0000402c  hObject=0xcaf00002
```

★ **Every refused alloc class is logged AFTER `kgraphicsCreateGoldenImageChannel` has
already failed.** A refusal at `38.690` cannot cause a failure printed at `38.683`. The
golden image channel's own channel allocation **succeeded**; it died on the **3D object**,
whose `_kgrAlloc` runs `kgrobjPromoteContext`, and §14.21 measured that refusal by name:
`PromoteFault::ContextVasUndeclared`.

#### The handles name the real owner, and RM's source agrees

`WATCHDOG_PUSHBUFFER_CHANNEL_ID 0x31415900` (`ogkm-580:
src/nvidia/src/kernel/gpu/rc/kernel_rc_watchdog.c:64`) ⇒ `0x3141590f` and `0x31415900` are
the **RC watchdog's** objects, not the golden channel's. Confirmed against RM's source, which
also settles what the golden channel really allocates
(`ogkm-580: src/nvidia/src/kernel/gpu/gr/kernel_graphics.c:2136-2541`,
`kgraphicsCreateGoldenImageChannel_IMPL`):

| step | class | site |
|---|---|---|
| VA space | `FERMI_VASPACE_A` `0x90f1` | `:2293` |
| pushbuffer phys | `NV01_MEMORY_SYSTEM` `0x3e` | `:2315` |
| pushbuffer **virt** | **`NV50_MEMORY_VIRTUAL` `0x50a0`** — ⊘ *not* `0x0070` | `:2328` |
| USERD | `NV01_MEMORY_LOCAL_USER` `0x40` | `:2404` |
| **channel** | `kfifoGetChannelClassId` → **`AMPERE_CHANNEL_GPFIFO_A` `0xc56f`** | `:2411`, `:2454` |
| 3D object | `AMPERE_B` — **this is the one that fails** | `:2513` |

`kfifoGetChannelClassId_IMPL` takes the **numeric maximum** GPFIFO class
(`NV_MAX(class, pClassList[i])`, `ogkm-580: kernel_fifo.c:3448-3484`) over GA106's
`ENG_KERNEL_FIFO` set `{0xc36f, 0xc46f, 0xc56f}` ⇒ **`0xc56f`**, which this port already
admits and maps. ⚠ The `gpuIsClassSupported(VOLTA_CHANNEL_GPFIFO_A)` test at `:2337` is real
but only sizes the **USERD** buffer (`ctrlSize = sizeof(Nvc36fControl)`); it does not choose
the channel class. A reader who stops at that `if` concludes `0xc36f` — which is how the
wrong framing arose.

`0xc36f` on GA106 comes from `krcWatchdogInit`'s `gpfifoMapping[]`, which is scanned
**first-match-wins in ascending-arch order** and therefore stops at **VOLTA**, never reaching
TURING or AMPERE (`ogkm-580: kernel_rc_watchdog.c:617-652`).

#### ⇒ The corrected table

| class | who asks | refusal at `d5369b5` | verdict |
|---|---|---|---|
| `0x0070` `NV01_MEMORY_VIRTUAL` | **RC watchdog** virtual ctx handle (`kernel_rc_watchdog.c:673`) | `UnmappedAllocClass` | not upstream of anything; watchdog-only |
| `0xc36f` `VOLTA_CHANNEL_GPFIFO_A` | **RC watchdog** channel (`:1013`) | `NotOnAllowlist` | not upstream of anything; watchdog-only |
| `0x402c` `NV40_I2C` | i2c probe (`RS_UNIQUE_HANDLE_BASE`) | `AllocClassNotPermitted::**Refused**` | ★ already decided — refused BY NAME (`4088589`) |
| `0x2081` `NV2081_BINAPI` | — **nobody** — | *never requested* | ⊘⊘ **REFUTED — see §14.26.** It is **libcuda's**, measured at `amb1_ee1994b` |

⊘⊘ **THE PARAGRAPH BELOW IS WRONG AND IS KEPT SO THE REFUTATION HAS A SUBJECT** (§14.26,
`[measured 2026-08-08, boot amb1_ee1994b]`): `hClass=0x00002081 paramsSize=0x00000004` on
`hClient=0xc1d0000c / hParent=0x5c000003`, plus `unserviced fn 76 cmd 0x20810108`. The grep
was honest and its **universe** was too small — no boot in that directory had ever run a CUDA
process, so a class only libcuda allocates could not appear in it. ★ Worse: `:3645` of this
same file already printed the line, from a §14.19-era probe boot, and `:3844` names `0x2081`
as appearing *"in the probe boots"*.

> ⊘ **`0x2081` is a phantom.** `grep -l 'hClass=0x00002081'` over **every** captured boot in
> `docs/reference/bench_evidence/` returns nothing, and there is no `Alloc(… NV2081_BINAPI …)`
> call site anywhere in the open kernel tree — it is allocated only by closed userspace
> (NVML/`nvidia-smi`) under a Subdevice. It has been on `CLASSES_SHARED` the whole time. It
> entered the work list as a *name in a doc sentence* and was never checked against a boot.

★ The final sentence was the true one and it applied to the *ruling* as much as to the entry:
"never checked against a boot" was still true of the refutation itself until a boot ran CUDA.

★ And the census arithmetic proves the set is exactly three, with nothing hidden: three
alloc-class refusals in the log, three `GspRmAlloc` failures in dmesg, and the bridge census
reads `AllocClassNotPermitted::NotOnAllowlist x1` + `::Refused x1` + `UnmappedAllocClass x1`.

#### ⊘ What this means for the work — do NOT admit them

Serving `0x0070` + `0xc36f` would fix the **RC watchdog**, an engine whose failure §14.20 and
this boot both measure as **non-fatal** (the adapter initialises and `nvidia-smi` enumerates
with all three refused). It would buy **zero** progress toward `cuInit`, and §14.21's lesson
applies directly: a class admitted so a green appears somewhere is exactly the trade that
cost the adapter last time.

★★★ **The real wall is unchanged and already named**: `kgrobjPromoteContext` →
`PromoteFault::ContextVasUndeclared` → the golden channel's VASpace has no page-directory
base. §14.21's re-enable condition is the whole of the remaining work, and it is exact: **make
the `0x90f10106` publication reach `Vas::pdb`.**

⚠ And that publication is **not** refused — it is **SERVED**. `control 0x90f10106 result
0x00000000 x4` in both `ship_7a881a7` and `ship3_d5369b5`. `GvasPubRecorder`
(`kayfabe-device/src/lib.rs:970`) records it into a **census log** and declines;
`InitTablePolicy` answers it. The record carries everything needed — `client`, `object`
(= `hVASpace`), and `pdes.levels[0]` (= the root) — and **nothing forwards it into the object
model as an `RmEvent::SetPageDir`**. ⊘ Note the consequence for instruments: the
`ControlParams::PageDirNotModelled` arm in `translate_control` is **unreachable** for
`0x90f10106`, because `InitTablePolicy` terminates the chain first — `BridgeRefusal::
PageDirControlNotModelled` appears in **no** boot census. A refusal that cannot fire is not a
guard (`a_table_does_not_decide_behaviour`, fourth instance).

#### ⚠ Two bench-recovery facts the host reboot taught, both runtime-only state

- ★★★ **The `nvktap0` tap device does not survive a reboot.** `boot_nvkvm.sh` runs
  `-netdev tap,ifname=nvktap0,script=no,downscript=no`, i.e. QEMU requires the tap to
  **pre-exist**. After the reboot `ip -br addr` showed no tap at all, and `boot_capture.sh`
  failed with *"guest never answered ssh within 150s"* — while the serial log showed the guest
  reaching `graphical.target` and a **login prompt**. ⊘ A perfectly healthy guest reads as a
  dead one. Recover with:
  `ip tuntap add dev nvktap0 mode tap && ip addr add 192.168.77.1/24 dev nvktap0 && ip link set nvktap0 up`.
  ⚠ Note `gssh_nv` connects to `ubuntu@192.168.77.2` over this tap — **not** `localhost:2223` —
  so the remembered *"needs a `~/.ssh/config`"* trap is a **different** harness's, and chasing
  it here wastes the cycle. (There is no `~/.ssh/config` on `vh` and the boot is green without one.)

- ★★★ **A killed build leaves a ZERO-BYTE artifact that `cargo` then reports as FRESH.** The
  outage killed the `d5369b5` build mid-compile. Afterwards
  `cargo build --release -p kayfabe-qemu-raw` printed **`Finished in 0.82s`** — cargo trusts its
  fingerprint DB and never checksums its own outputs — while
  `target/release/deps/libkayfabe_rt-*.rlib` was **0 bytes**. `build_qom_shim.sh`'s guard is
  `[ -f "$ARCHIVE" ]` — **existence, not size** — so it laid an empty archive into the tree and
  the only symptom was ~200 undefined `kayfabe_shim_*` symbols at link. ⊘ Exactly one file was
  genuinely truncated (6 zero-length files, 5 of them lock/`stderr` files that are legitimately
  empty), so `cargo clean -p <the-one-crate>` was the whole fix.
  ⇒ Same family as `862c7c2` and `bench_rebuild_stub_gap`, with a new twist: here **both**
  the tool's own success signal **and** the script's guard were satisfied by a broken artifact.
  ★ The stamp check is what caught it — `strings … | grep kayfabe-rev` on the archive returned
  **nothing at all**, which is why that check must be run on the archive and not only on the
  binary.

### 14.23 ★★★ THE PUBLICATION REACHES THE OBJECT MODEL — recording is not forwarding (`[built]`)

⊘ **`[built]`, not `[measured]`** at the moment it was written; §14.24 is its boot, and
that boot **refutes this section's own closing claim**. Read both.

#### What §14.22 left, restated as a defect rather than as a gap

`[measured 2026-08-08, boots ship_7a881a7 / ship3_d5369b5]`: `control 0x90f10106 result
0x00000000 x4`. **Served.** `GvasPubRecorder` decoded all five publications (four client-arm
+ the global arm) and wrote them into a **census log** whose entire output was a number in a
report. Nothing turned one into an `RmEvent::SetPageDir`, so `Vas::pdb` stayed `None`, so
`kayfabe_core::promote::route_promote_ctx` could only ever answer `ContextVasUndeclared` —
which is exactly what both boots printed at `kernel_graphics_object.c:224`.

⊘ **A fact the guest states five times, answered `NV_OK`, reaching nothing.** That is the C
artifact's shape with better instrumentation on it.

#### What landed

| piece | where |
|---|---|
| `ControlParams::VaspacePublishedPdes` | `0x90f10106` + `0x20800a9f` leave `PageDirNotModelled`, which keeps only the **revocation** `0x00801814`. `params_size` = 184, checked **exactly** |
| `translate_published_pdes` → `RmEvent::SetPageDir` | the VA space is read off the RPC **header**'s `hObject` (`ogkm-580: gpu_vaspace.c:5174-5177`) — ⊘ **no params field names it at all** |
| three named refusals | `PublishedPdesUnnamedVaspace` (`hObject == 0`), `PublishedPdesMalformed` (the guest broke `ctrl90f1.h`'s own rules), `PublishedPdesRootAperture` (a root outside the framebuffer is **not** a `Pdb`) |
| `GMMU_APERTURE_*` + `decode_aperture` **move** to `kayfabe_abi::gvaspacepdes` | one declaration. This enum has already been transcribed wrong once — all four values — and a boot is what caught it (`two_encodings_agreeing_on_the_first_values`) |
| `kayfabe_gsp::CommandObserver` + `Observing` | ★★★ a chain link that **cannot answer, because it has nothing to return** |
| `served_chain`'s FRONT seat | takes a `CommandObserver`, ahead of `InitTablePolicy` — which still answers these two ids and must |
| `kayfabe_rmrpc::PublicationObserver` | declares into a **second `ObjectModel` handle on the same shell** (E2's port, used for its purpose), shares the refusal census, publishes `seen`/`applied` |
| wire ABI 21 → 22 | `gvas_pub_seen` / `gvas_pub_applied` / `gvas_pub_unexpected` |

★★ **Two counts of one event, deliberately.** `gvas_pub_total` is the *recorder's* (decode +
log); `gvas_pub_seen` is the *observer's* (decode + declare). Until this increment the first
read 5 while the object model held nothing, and one number could not have said that.

⊘ **`Bridge` is deliberately not reused here.** `Bridge::deliver` runs the `Reassembler`, and
there must be exactly **one** reassembler over a command stream: a second one seated ahead of
`ObjectPolicy` would consume the same continuation fragments into a second buffer — two
half-messages where the guest sent one.

#### ⊘⊘ Two INSTRUMENT defects found on the way, both this repository's own named traps

1. **`served_chain`'s object seat cited a test file that has never existed.** The comment
   read *"the obligation is stated here and `tests/served_chain_objects.rs` is where it is
   checked"*, and `kayfabe-crec` cited the same non-file a second time as the reason its
   replay chain is safe. `git ls-files` has never listed it. The obligation was load-bearing,
   precisely worded, cited twice, and unchecked for its whole life — the family of
   `should_panic_matches_the_wrong_site` and `gate_read_through_grep_cannot_fail`: a claim
   whose *reference* to a check was mistaken for the check. Discharged now by
   `tests/tests/served_chain_seats.rs`.
2. **`sticky::POLICY_DISPOSITIONS`' `ObjectPolicy` row claimed `NotAControl`, and `#177` had
   already made it false.** That policy answers `OBJECT_CONTROLS` (`0xa06f0103`,
   `0xa06f0104`) with `NV_OK` and a body. Its executing test sweeps `WantedTable::ALL` —
   **`InitTablePolicy`'s** 24 ids — so it quantified over a universe containing neither
   control and passed vacuously (`gates_quantified_over_a_list`). Row corrected to `Guarded`.

★ And a third, smaller, caught by the new test itself: the two publication ids are **also**
members of `WantedTable::ALL`, so a sweep posts each twice and `PUBLICATION_CONTROLS.len()`
is a wrong expected value that reads like the right one.

#### ⊘ The closing claim of this section, which was WRONG

> Nothing guest-visible changes: the front seat cannot answer, and every reply byte is
> identical with and without it.

Every reply byte **was** identical. §14.24 is what that missed.

### 14.24 ★★★★ THE MILESTONE HAD BEEN RESTING ON THE PORT'S IGNORANCE — measured, and fixed (`5849328`)

#### The boot that refuted it

`[measured 2026-08-08, boot pub1_3e43e9a, rev 3e43e9a]`, vast GA106 bench (`vh`, RTX 3060
`10de:2504`, host driver **580.159.04 Open**), stamp verified by
`strings … | grep -o 'kayfabe-rev:[0-9a-f]*'` on **both** `libkayfabe_qemu_raw.a` and
`qemu-build/qemu-system-x86_64`. Shipping configuration, `probe-arm set: EMPTY`, **STOCK**
guest module. Evidence: `docs/reference/bench_evidence/run_pub1_3e43e9a_*`.

```text
nvkvm:   of those, 3 reached the object model, 2 ACCEPTED
nvkvm: doorbells: 1 arrived, 0 served, 1 REFUSED by name
nvkvm:   first doorbell refusal [FwdFault::IsolateRetired] … c=0xc1e00006 vas=0xa root=0x2efa9c000/ap1/sh47
NVRM: Call timed out [NV_ERR_TIMEOUT] (0x00000065) returned from memmgrMemSet(…) @ mem_mgr.c:463
NVRM: Assertion failed: pCeUtils->lastCompletedPayload == lastSubmittedPayload @ ce_utils.c:349
NVRM: GPU 0000:00:03.0: RmInitAdapter failed! (0x25:0x65:1249)
```

against `2 arrived, 2 served [CpuCe::ServedLocally]` and `SMI_RC=0` one revision earlier.
**The publication forwarding worked exactly as designed and cost the milestone.**

#### ★★★ The mechanism, and it is one line of ours

`SharedDoorbell::try_ce_submission` opened with

```rust
if facts.vas_pdb.is_some() {
    return None; // the core can address this channel; it is not ours.
}
```

and its own precondition list said *"**`vas_pdb` must be `None`.** A channel the core can
address is the core's."* That is an **inference**: the core can *address* it, therefore the
core can *serve* it. It held only while the port did not know the channel's address space.
§14.23 made it know — so the CeUtils scrubber's channel became addressable, this executor
declined it as "not ours", the doorbell fell through to the real forwarding plane, and that
plane is `IsolatePlane::Stillborn` in **every shipping build** (`STILLBORN_WHY`: *"no host
verb can be issued"*).

⇒ ★★★ **`nvidia-smi` has been enumerating a device because the port did not know where the
scrubber's page tables were.** §14.20's green, §14.22's reproduction and §14.21's restoration
all rested on that. It is the same shape as §14.21 one plane over — *an accurate port state
is fatal when a fallback was keyed on the inaccuracy* — and it is the second time in two
days that **being more correct** broke the boot.

⚠ Note what §14.8 had already written, in the module this increment edited around
(`kayfabe_device::gvaspub`): *"granting the CeUtils channel a VAS **without the executor
being reachable** makes `plan_doorbell` pass … and makes `commit_doorbell` ring a host
channel with no CE behind it."* The warning named the right hazard and predicted the wrong
sign — it expected a doorbell reporting **Served** over work that did not happen; what
happened is a doorbell reporting **Refused** over work that used to happen. Reading it was
not enough; only the boot got the direction right.

#### The fix, and why it is not a fallback

The gate now asks the question it always meant — not *"can the core **address** this
channel?"* but **"is there any other executor?"** — answered from the composition root's own
`selected_isolate_plane()` reading, carried to the doorbell port at realize as
`SharedDoorbell::local_ce_is_the_only_executor`.

⊘ **Not a fallback-after-refusal.** The decision is made before any doorbell arrives, from a
choice the composition root declared; nothing retries a refused submission on a second path.
⊘ **One reading, not two**: the selector is read once in `object_policy` and carried out, for
the reason the probe set and the chip's engine slice are — two readings of one fact are two
facts that can disagree. A build that selects a real isolate plane keeps the old routing
exactly: a channel the core can address goes to the core.

#### `[measured 2026-08-08, boot pub2_5849328, rev 5849328]` — GREEN, with the fact in place

Same bench, same stamp discipline, `probe-arm set: EMPTY`, **STOCK** module. Evidence:
`docs/reference/bench_evidence/run_pub2_5849328_*`.

```text
SMI_RC=0                        (full nvidia-smi device table)
nvkvm: VA-space page-directory publications: 5 total, 5 distinct, 0 UNDECODABLE
nvkvm:   of those, 5 reached the object model, 4 ACCEPTED (Vas::pdb populated)
nvkvm: doorbells: 2 arrived, 2 served, 0 REFUSED by name
nvkvm: completions: 2 announced, 0 UNVECTORED, 2 would be masked
nvkvm:   bind engineType 11 (COPY2) client 0xc1e00006 object 0x00000002 result 0x00000000
```

★★ **The guest's dmesg is IDENTICAL to `ship3_d5369b5`** — 38 lines, timestamps stripped and
sorted, `diff` empty. So the milestone reproduces byte for byte **and** four VA spaces now
carry the guest's own page-directory base:

| publication | `hClient` / `hObject` | root | applied? |
|---|---|---|---|
| `0x20800a9f` (global arm) | `0x0` / `0x0` | `0x2efbae000` | ⊘ **no** — `BridgeRefusal::ReservedClient`, `NV01_NULL_OBJECT` is not a namespace |
| `0x90f10106` | `0xc1e00005` / `0xc` | `0x2efba5000` | ✔ |
| `0x90f10106` | `0xc1e00006` / `0xa` | `0x2efa9c000` | ✔ — the channel that binds COPY2 and rings the doorbell |
| `0x90f10106` | `0xc1e00007` / `0xbaba0042` | `0x2efa7c000` | ✔ |
| `0x90f10106` | `0xc1d00008` / `0xcaf00000` | `0x2efa7c000` | ✔ |

⊘ Zero `PublishedPdes*` refusals: every client-arm publication was well-formed and
**vidmem-rooted**, so the aperture fork never had to fire on this boot. It is still the right
refusal — `c_ceutils_ring_resolution.md` §2 measured a sysmem-rooted PDB on real GA106 — and
it is now a guard that *can* fire rather than one that could not.

#### ⊘ What this does NOT establish

`kgrobjPromoteContext` still returns `0x56` at `kernel_graphics_object.c:224`, and that is
**expected rather than disappointing**: §14.21 removed the claim, so `GPU_PROMOTE_CTX` is
unserviced and the envelope refusal is the FSM's. The fact it was waiting on now exists in
the object model; **claiming the control is the next increment**, and §14.21's re-enable
condition — *"when the `0x90f10106` publication reaches `Vas::pdb`, promote-ctx SUCCEEDS
rather than refuses"* — is now testable for the first time.

⚠ And when it is claimed, the `0x56` trap still governs: a refusal here propagates into
`gpuStatePostLoad`, where `NV_ERR_NOT_SUPPORTED` is the **only** status converted to `NV_OK`
(`ogkm-580: gpu.c:3437-3439`). ★ `BridgeRefusal::rpc_result` already answers `0x56` for every
variant, so routing promote-ctx's refusal through the **envelope** rather than through a
per-arm status constant is what §14.21 measured the need for — the C-shaped mistake was a
*better* status, chosen at the arm.

### 14.25 ★★★★ PROMOTE-CTX SUCCEEDS — §14.21's re-enable condition, met and measured (`423bf08`)

`[measured 2026-08-08, boot pro1_423bf08, rev 423bf08]`, vast GA106 bench (`vh`, RTX 3060
`10de:2504`, host driver **580.159.04 Open**), stamp verified on **both**
`target/release/libkayfabe_qemu_raw.a` and `qemu-build/qemu-system-x86_64` →
`kayfabe-rev:423bf080502168f46edb44d54d69d4a4d8f75f81` in each. **`probe-arm set: EMPTY`**,
**STOCK** guest module. Evidence: `docs/reference/bench_evidence/run_pro1_423bf08_*`.

#### The two lines this section exists for

```text
nvkvm:   control 0x2080012b result 0x00000000 x1
nvkvm:   control 0x2080012b result 0x00000056 x1 REFUSED
SMI_RC=0
```

★★★ **A `GPU_PROMOTE_CTX` was PERFORMED** — `Translation::CtxPromotion` applied through
`Gpu::promote_ctx`, `NV_OK` on the wire — and the adapter **survived**, which is the
combination §14.21 measured to be impossible before the fact existed.

⊘ **`PromoteFault::ContextVasUndeclared` does not appear in the census at all.** That is the
wall §14.21 named by measurement and §14.22 restated as *"the real wall is unchanged and
already named"*. It is gone, and it was removed by §14.23/§14.24 rather than argued away.

#### What was re-claimed, and the one thing that changed from §14.21's version

`NV2080_CTRL_CMD_GPU_PROMOTE_CTX` rejoins `OBJECT_CONTROLS` with a `respond_control` arm that
**routes** through `Bridge::deliver` (no second decoder for a 560-byte struct) and echoes the
request's own body on success — verbatim §14.21's design.

★★★ The single change is the **refusal status**, and it is the whole of §14.21's lesson made
executable: the arm answers `BridgeRefusal::rpc_result`, which is `NV_ERR_NOT_SUPPORTED`
(`0x56`) for **every** variant, instead of the per-arm `0x33`/`0x40` that killed the adapter.
`gpuStatePostLoad` converts only `0x56` to `NV_OK` (`ogkm-580: gpu.c:3437-3439`) and this
control's failure reaches it. ⊘ The consequence, said out loud: a refused promote-ctx is now
**wire-indistinguishable** from an unserviced one, and the difference lives only in this
port's own census — which is where it belongs. `bind_channel.rs`'s
`every_claimed_control_is_decided_even_when_malformed` still quantifies over the whole of
`OBJECT_CONTROLS` and splits the *expectation* per id with its reason; ⊘ the list was not
shortened.

#### ★★★ The wall MOVED, and here is the guest's own log saying so

`diff` of the guest's dmesg against `pub2_5849328` (timestamps stripped, sorted).
**Disappeared:**

```text
- kgraphicsCreateGoldenImageChannel(pGpu, pKernelGraphics) @ kernel_graphics.c:508      -> 0x56
- pRmApi->AllocWithHandle(…, hObj3D, classNum, NULL, 0)    @ kernel_graphics.c:2519     -> 0x56
- kgrobjPromoteContext(…)                                  @ kernel_graphics_object.c:224 (one of two)
- Assertion failed: 0                                      @ kernel_fifo.c:3129
- vaListDestroy: non-zero mapCount(pVaList): 0x1  ×4
```

**Appeared:**

```text
+ GspRmAlloc failed: hClient=0xc1e00007 hParent=0xbaba0045 hObject=0xbaba0046
                     hClass=0x0000c797 paramsSize=0x0 status=0x00000056
+ GspRmFree failed:  hClient=0xc1e00007 hObject=0xbaba0046
+ Assertion failed: (status == NV_OK) || (… FULLCHIP_RESET) @ rs_client.c:844, rs_server.c:{259,1375}
```

`0xc797` is **`AMPERE_B`**, and ★ it had **never been requested in any previous boot** —
`grep -l 'hClass=0x0000c797'` over every capture in `docs/reference/bench_evidence/` returns
nothing before this one. The `0xbaba00xx` handles are the golden-image channel's own
(`hClient 0xc1e00007` is the client whose VA space `0xbaba0042` published a root in §14.24's
table). ⇒ The chain that had been dying at `kgrobjPromoteContext` now runs past it and dies
at an **alloc class this port does not admit**.

⚠ **What is measured and what is inferred, kept apart.** Measured: those five lines left the
log, `0xc797` entered it, one promote-ctx returned `NV_OK`, one returned `0x56`, and the
adapter lived. Inferred (from handle prefixes and from the surviving
`kgrobjPromoteContext` line falling *between* the RC watchdog's `0x0070` at `33.675` and
`kernel_rc_watchdog.c:1198` at `34.136`): the promotion that **succeeded** is the golden
channel's and the one that **refused** — `PromoteFault::UnknownContextObject x1` — is the RC
watchdog's, an engine §14.20 and §14.22 both measured as **non-fatal**. ⊘ That attribution is
a reading of two timestamps and a handle prefix; it is not established by anything the device
itself printed, and the next increment should make the census carry the handles rather than
leave this to inference.

#### ⇒ The next wall, named

**`AMPERE_B` (`0xc797`) is refused as `BridgeRefusal::UnmappedAllocClass`.**

⊘ And note this is **not** §14.22's refused trio wearing a new number. That section's ruling
was *"do NOT admit `0x0070` / `0xc36f`"* because their refusals were logged **after** the
failure they were blamed for and belong to the non-fatal RC watchdog — the ordering is in
that section's own timestamps. `0xc797`'s refusal is logged at `33.559`, **before** every
watchdog line, on the golden channel's own client, at exactly the point the chain now
reaches. The two situations are opposites and both were settled by reading the guest's clock.

⚠ Past this sits the GR golden context itself, and the C artifact's answer to it was *"silicon
boundary, forward to the host"*. Admitting a class is not the same as serving what the class
does; whether `AMPERE_B` can be answered without real silicon is the question the next
increment has to put to hardware, not to a table.

### 14.26 ★★★ `AMPERE_B` ADMITTED — and §14.25's closing question, answered from RM's own source before it was put to hardware (`ee1994b`)

§14.25 ended: *"whether `AMPERE_B` can be answered without real silicon is the question the
next increment has to put to hardware, not to a table."* ★ **It was answerable from source,
and the answer is yes — because the function that allocates it never runs the engine.**

#### ⊘ First, the refutation of my own framing

I opened this increment expecting the hazard to be *"admitting the class lets the guest start
the golden channel, and a GR channel we cannot execute then times out"* — i.e. §14.24's shape
again, one plane over. **That fear is wrong, and one `sed` of RM's source settles it.**

`kgraphicsCreateGoldenImageChannel_IMPL` **ends at the 3D-object alloc**:

```c
2519    NV_ASSERT_OK_OR_GOTO(status,
2520        pRmApi->AllocWithHandle(pRmApi, hClientId, hChannelId, hObj3D, classNum, NULL, 0),
2521        cleanup);
2523 cleanup:
2533        pRmApi->Free(pRmApi, hClientId, hClientId);
```

(`ogkm-580: src/nvidia/src/kernel/gpu/gr/kernel_graphics.c:2519-2533`.) `:2519` is the **last
statement before `cleanup:`**, and `cleanup:` frees the whole internal client — 3D object,
channel, USERD, pushbuffer memory, VA space, subdevices, device. Between the two there is no
pushbuffer write, no `GPFIFO_SCHEDULE`, no doorbell, no semaphore and no FECS wait. RM says so
itself at `:2419-2424`: *"Set the gpFifoOffset to zero intentionally since we only need this
channel to be created, but will not submit any work to it."* ⇒ **`gpFifoOffset = 0`.**

★ And the one branch that *could* have run work is not on this chip:
`_kgraphicsPostSchedulingEnableHandler:509-511` calls `kgraphicsInitializeBug4208224WAR_HAL`
only when `kgraphicsIsBug4208224WARNeeded_HAL`, which `g_kernel_graphics_nvoc.c:464-480` wires
to `_TU102` for `TU102 | TU104 | TU106` and to `_3dd2c9` everywhere else — and `_3dd2c9`
is literally `static inline NvBool …(…) { return NV_FALSE; }`
(`ogkm-580: g_kernel_graphics_nvoc.h:928-930`; its sibling
`kgraphicsInitializeBug4208224WAR_56cd7a` is `{ return NV_OK; }` at `:922-924`). GA106 gets
the stub, so the WAR neither runs nor fails.

⇒ The kernel side of the golden image channel is **allocate and free**. The golden image
itself is GSP-RM's to produce, on the far side of the `GSP_RM_ALLOC` we answer.

#### What landed

| piece | where |
|---|---|
| `AMPERE_B` generated from `clc797.h:32` (both vendored tags agree, same line) | `kayfabe-abi/gen/src/main.rs`, `generated/classes.rs` |
| `alloc_params(0xc797) = NoDeclaredFacts` | `versions.rs` |
| `classify(0xc797) = EngineObject { engine: GrGraphics }` | `kayfabe-chips/src/ga10x.rs` **and** `kayfabe-mocks`' `WireClassArch` |
| the derivation ratchet `11 → 12` | `capability.rs::every_class_this_port_decodes_is_permitted` |
| the boot's own wire row, served, with its before/after pair; params-never-read over three shapes; the id pinned against ogkm; the classify arm pinned | `tests/tests/gsp_rm_alloc.rs`, `rmrpc_bridge.rs`, `oracle_layout.rs` |

⊘ **The capability boundary did not move by one class.** `0xc797` has been on `CLASSES_SHARED`
with `Origin::Empirical` since the port was written (`capability.rs:1099-1103`), and
`the_founding_rows_are_pinned` already asserted it (`cls(0x0000_c797, "AMPERE_B")`). The
refusal came from the statement **after** the capability gate — `alloc_params` returning
`None` (`kayfabe-rmrpc/src/lib.rs:1179`). This increment touches the decoder half only, which
is the split `translate_alloc:1167-1170` exists to make.

#### ★★ `NoDeclaredFacts` is the STRONG reading, and RM's own table says so

Not "we have no decoder" — *"there is nothing here to decode."* Four independent facts:

1. RM registers this class's params as **`RS_OPTIONAL(NV_GR_ALLOCATION_PARAMETERS)`**
   (`ogkm-580: src/nvidia/src/kernel/rmapi/resource_list.h:2010`), which expands to
   `{ sizeof(x), bParamRequired = NV_FALSE }` (`resource_desc.c:76`). A NULL is legal **by
   declaration**.
2. The struct is `{version, flags, size, caps}` — 4 × `NvU32`, 16 bytes, **no handle and no
   pointer** (`ogkm-580: nvos.h:2716-2721`); `caps` is an *output* the caller reads back.
3. ★ `grep -rn NV_GR_ALLOCATION_PARAMETERS src/nvidia/src/kernel/gpu/gr/` returns **nothing**.
   No GR code reads it on the alloc path at all. `kgrobjConstruct_IMPL` touches `pParams`
   only for `hResource` and `hParent` (`kernel_graphics_object.c:310, 339`).
4. The one allocator on this path supplies none of it — `NULL, 0` at `:2520` — which is why
   the wire said `paramsSize=0x00000000`.

#### ⊘ The standing hazard, checked in the direction that bit §14.21 and §14.24

The question is *"is there any other executor?"*, and before that, *"what is keyed on this
class being absent?"* Answers, each read rather than assumed:

- `SharedDoorbell::try_ce_submission`'s gate reads `facts.vas_pdb` and the realize-time
  `local_ce_is_the_only_executor`. It reads **no** `EngineKind` and **no** object-graph node.
- `kayfabe_fwd::plan_doorbell`'s gate reads `proc.exec.requested`, written only by the
  guest's own `0xa06f0103`. `chan.engine` is carried as a routing tag and gates nothing.
- `completion_arm` (`fwd:4159`) splits only `NvEnc` out; `GrCompute`, `GrGraphics` and `Ce`
  are all `SharedSema`.
- `route_engine_object` / `exec_engine_object` are reachable **only** through
  `SharedDevice::forward_engine_object`, which has no non-test caller. Declaring the object
  adds a graph node and nothing else.

★ And the refinement pass is a **no-op in kind**: `AMPERE_CHANNEL_GPFIFO_A` already classifies
`Channel { engine: GrCompute }` (`ga10x.rs:161`), and every consumer maps `GrCompute` and
`GrGraphics` to the same answer — `EngineRoute::for_engine` (`rc.rs:115`) and
`engine_type_for` (`kayfabe-isolate-host/src/rm.rs:1654`) both to
`NV2080_ENGINE_TYPE_GRAPHICS`. That is what makes the *truthful* label affordable:
`AMPERE_B` is `GR_OBJECT_TYPE_3D` (`kgrmgrGetGrObjectType_IMPL`,
`kernel_graphics_manager.c:130`), RM picks it over the compute class exactly when
`kgraphicsIsGFXSupported` (`kernel_graphics.c:2503-2510`), and GA106's class list carries
`{ AMPERE_B, ENG_GR(0) }` as its **only** 3D-typed entry
(`g_gpu_class_list.c:1112`, beside `{ AMPERE_COMPUTE_B, ENG_GR(0) }` at `:1114`).

⚠ **One hazard is real and is recorded rather than fixed**: `project.rs:737`'s
`engine_refine.entry(chan).or_insert(engine)` is *first-object-wins*. A channel carrying both
an `AMPERE_B` and an `AMPERE_DMA_COPY_B` could have its CE refinement stolen.
`[measured 2026-08-08, boots pro1_423bf08 and amb1_ee1994b]` it cannot happen on this
path — `0xc797` is `hClient=0xc1e00007 / hParent=0xbaba0045` (the golden
channel) while the CeUtils scrubber is `hClient=0xc1e00006` — but this is the failure mode to
watch if the golden channel is ever reused.

#### ⊘⊘ What this does NOT establish, said before the boot rather than after it

**Admitting a class is not serving what the class does.** The `GSP_RM_ALLOC` we answer `NV_OK`
is the point at which the *physical* RM, on GSP, constructs the GR object and builds a golden
context image on silicon. This port builds none. That is affordable **here and only here**,
for two reasons that are properties of this call site and not of the class:

- the guest frees the whole tree three lines later, and
- it never reads an image back through this port on the boot path.

A guest that later runs its **own** GR engine against a forged golden context is the case this
row does not cover, and the C artifact's standing answer for it is unchanged: *silicon
boundary, forward GR execution to the host* (`c_cuda_ladder.md` §3 — the host self-maps its
own ctx buffers at `st=0x51`, and faking them produced the `cuCtxCreate` stale-read crash).

★ The named next wall is therefore **not** in `kgraphicsCreateGoldenImageChannel`. It is
whatever the guest does with an adapter that finally completed `_kgraphicsPostSchedulingEnableHandler`.

#### ★★★★ `[measured 2026-08-08, boot amb1_ee1994b, rev ee1994b]` — THE GOLDEN-IMAGE CHANNEL COMPLETES

vast GA106 bench (`vh`, RTX 3060 `10de:2504`, host driver **580.159.04 Open**), stamp verified
on **both** `target/release/libkayfabe_qemu_raw.a` and `qemu-build/qemu-system-x86_64` →
`kayfabe-rev:ee1994b551fddf06b3964bef74e39f565148984d` in each, both non-empty (33.1 MB /
84.6 MB). **`probe-arm set: EMPTY`**, **STOCK** guest module, `SMI_RC=0`. Evidence:
`docs/reference/bench_evidence/run_amb1_ee1994b_*`.

`diff` of the guest's dmesg against `pro1_423bf08` (timestamps stripped, sorted) is **five
lines removed and ZERO added**:

```text
- rpcRmApiAlloc_GSP: GspRmAlloc failed: hClient=0xc1e00007; hParent=0xbaba0045;
                     hObject=0xbaba0046; hClass=0x0000c797; paramsSize=0x0; status=0x56
- rpcRmApiFree_GSP:  GspRmFree failed:  hClient=0xc1e00007; hObject=0xbaba0046; status=0x56
- Assertion failed: (status == NV_OK) || (… FULLCHIP_RESET) @ rs_client.c:844
- Assertion failed: (status == NV_OK) || (… FULLCHIP_RESET) @ rs_server.c:259
- Assertion failed: (status == NV_OK) || (… FULLCHIP_RESET) @ rs_server.c:1375
```

⇒ The whole 3D-object chain — `kgrobjConstruct` → ctx-buffer alloc/map → the 12-entry
`GPU_PROMOTE_CTX` → `GSP_RM_ALLOC(0xc797)` → `GSP_RM_FREE` — is now **silent**. Everything
still in the log is the RC watchdog's (`0x0070`, `0xc36f`, `kgrobjPromoteContext` at
`kernel_graphics_object.c:224`, `kernel_rc_watchdog.c:1198`) and the i2c probe's (`0x402c`),
all three of which §14.20/§14.22 measured to be non-fatal and this boot reproduces as such.

★ **Nothing was added**, which is the half a green diff usually cannot claim. The accuracy
improvement cost nothing this time — and per §14.21/§14.24 that was the thing to check, not
to assume.

#### ★★★★ AND THE LADDER RAN — `cuInit` reached this port for the FIRST time

The boot carried `POST_CAPTURE_HOOK=cup2_hook.sh`, so `cup2` was built and run **inside the
guest with the guest still up**. `cuda.h` located by content (`grep -q CUDA_VERSION`, not by
path) at `/usr/local/include/cuda.h`; `GCC_RC=0`.

```text
FAIL cuInit(0) -> no CUDA-capable device is detected (100)
CUP2_RC=1
```

⊘ **Rung 1 of the ladder does NOT pass, and that is the result.** But the run is not empty:
opening `/dev/nvidia0` a second time ran `RmInitAdapter` **again** (the device census doubles
almost everywhere — `2` promote-ctx `NV_OK`, `2` refused, `4` doorbells all served, `8`
accepted page-directory publications, `2` COPY2 binds on clients `0xc1e00006` and
`0xc1e00011`), and the second init is line-for-line the first. ★ So the adapter is
**re-initialisable**, which nothing had shown before.

#### ⊘⊘ THE NEXT WALL, MEASURED — and it REFUTES §14.22's own ruling

Exactly **one** control id is new in the unserviced ledger relative to `pro1` (23 → 24
distinct, `comm -13`), and exactly one new `GspRmAlloc` failure appears in the cup2 window:

```text
NVRM: GspRmAlloc failed: hClient=0xc1d0000c; hParent=0x5c000003; hObject=0x5c000004;
                         hClass=0x00002081; paramsSize=0x00000004; status=0x00000056
nvkvm:   unserviced fn 76 cmd 0x20810108
```

★★★ **`0x2081` is `NV2081_BINAPI`, and §14.22 ruled it a phantom.** That section's table says
*"who asks: — **nobody** — / never requested"* and its prose says *"⊘ `0x2081` is a phantom.
`grep -l 'hClass=0x00002081'` over **every** captured boot in `docs/reference/bench_evidence/`
returns nothing … It entered the work list as a *name in a doc sentence* and was never checked
against a boot."* ⊘ **That ruling is refuted by measurement**, and the refutation is not
subtle:

- it *is* requested — by **libcuda**, on the `0x5c0000xx` RM-internal chain, under a
  Subdevice, exactly as §14.22's own next sentence predicted it would be;
- ★ it had **already** been captured. `execution_plane_increments.md:3645` — 400 lines
  *above* §14.22, in this same file — prints `hClass=0x00002081 … status=0x00000056` from a
  §14.19-era probe boot, and `:3844` names `0x2081` explicitly as appearing *"in the probe
  boots"*. The grep was honest; its **universe** was `docs/reference/bench_evidence/`, and
  those probe boots' captures were not all committed there.

⇒ `gates_quantified_over_a_list`, in its purest form: *a smaller universe is a smaller true
statement*, and the sentence read as a fact about the world. ★ The reason it survived is worth
naming too — **no boot had ever run a CUDA process.** Every capture in the directory stopped
at `nvidia-smi`, so a grep over that directory could not have found a class only libcuda
allocates. The instrument was not wrong; it was pointed at a world where the event cannot
occur (`skipped_oracle_kills_the_guard`).

#### ⇒ The next increment, named — and ⊘ deliberately NOT bundled into this one

**`NV2081_BINAPI` (`0x2081`) and its control `0x20810108`.**

What is already known, from RM's source rather than from a table:

- `NV2081_ALLOC_PARAMETERS` is `{ NvU32 reserved; }` — **four bytes, and the field is
  literally named `reserved`** (`ogkm-580: src/common/sdk/nvidia/inc/class/cl2081.h:33-40`),
  which matches the measured `paramsSize=0x00000004` exactly. Registered
  `RS_OPTIONAL`, parent `RS_LIST(classId(Subdevice))`, internal class `BinaryApi`
  (`ogkm-580: resource_list.h:439-449`). ⇒ another `NoDeclaredFacts`, on the same strong
  reading as `AMPERE_B`.
- ⚠ **The control is the hard half, and it is opaque by construction.**
  ⊘⊘ **REFUTED — see §14.27.** It is opaque, and it is **not load-bearing**: refused alone on
  a real GA106, `cuInit` still returns `0`. The *alloc* — called "the easy half" — is the
  causal one, and `0x20800102` `GPU_GET_INFO_V2` is a co-equal second cause this section did
  not see. ★ The sentence below is kept intact so the refutation has a subject; *"has no
  oracle"* is a fact about our instruments and was silently read as *"is required"*.
  `binapiControl_IMPL` (`ogkm-580: src/nvidia/src/kernel/rmapi/binary_api.c:61-127`) does not
  interpret `pParams->cmd` at all — it forwards the whole command to GSP via
  `NV_RM_RPC_API_CONTROL`. So there is no kernel-side semantics to read for `0x20810108`;
  whatever it means lives in GSP-RM, which is not in any vendored tree. ⊘ Inventing a reply
  body here is `mock_fidelity_both_directions` with nothing to hold it to — the reply shape
  must come from a real GA106 (`../nvkvm-rs/traces/real_ga106/`) or the control must be
  refused **by name**.

#### ⊘⊘ AND `0x20810108` HAS NO ORACLE — checked in all three, before anyone reaches for one

The next increment's control cannot be answered from anything this project owns, and that is
worth establishing **now** rather than after a day of looking:

| source | has `0x20810108`? |
|---|---|
| the C's captured control table `mode2_initctrl_ga106.h` (56 rows) | ⊘ **no row.** `gsp_demand_list_cap1.md` §5.1 lists it among the *"demanded but absent from the table"* six |
| `cap1_coldboot_hermetic` | the **request** only — record 309 234, `UNRESOLVED` (`gsp_demand_list_cap1.tsv:72`). A demand is not a reply |
| `traces/real_ga106/rpc_{transcript,bodies}_real_ga106.txt` | ⊘ **no hit at all** (`grep 20810108` → nothing) |

★ And the three failures have **one** cause, which is the same cause as §14.22's phantom:
every oracle this project owns was produced by driving `RmInitAdapter` with **`nvidia-smi`**
(`traces/real_ga106/README.md`, "method"), and `0x20810108` is issued by **libcuda**. ⇒ A
world with no CUDA process in it cannot witness a control only CUDA asks for — the same
sentence that explains why the `0x2081` grep came back empty. ⊘ Two independent instruments
agreeing is not corroboration when they share the defect (`a_table_does_not_decide_behaviour`
— *"a correction from the same source is not an independent check"*).

⇒ **The instrument for the next increment is a new capture, not a lookup.** The obvious
candidate is `crates/kayfabe-isolate-host/src/bin/rmladder.rs` — already a *deterministic
cross-machine oracle* (`rm_ladder_is_a_deterministic_oracle`: two physical GA106s differed by
exactly one `hClient` line) — extended to allocate `NV2081_BINAPI` under a Subdevice and issue
`0x20810108` against the **host** GA106 that `vh` already has. ⚠ Whether an unprivileged
client may do so at all is itself unmeasured; `resource_list.h:445` says
`RS_FLAGS_ALLOC_NON_PRIVILEGED`, which is a statement about the *alloc* and not about the
control.

⊘ It is not bundled here for the reason §14.24 taught: this increment's boot is already
measured, and a second wall-moving change inside it would make a regression unattributable.

⚠ And the standing caution is unchanged: `cuInit` failing `100` **beside** a refused
`0x2081` is a correlation of two facts in one window, not a proof of causation. It is the
only new refusal in that window, which is why it is the next thing to try — not why it is
the answer.

#### ⊘ Gate state at `55d289b`, attributed rather than absorbed

`scripts/run_full_suite.sh --only ci-stable` on `vh`: **4 steps fail, and all four reproduce
at `origin/master` `8c40f00` in a clean worktree.** Measured by checking each one against a
`git worktree` at master rather than by assuming:

| failing step | at `8c40f00`? |
|---|---|
| Format check (main workspace) | ✔ yes — 8 sites: `kayfabe-device/src/lib.rs:969`, `kayfabe-rmrpc/src/policy.rs:1524`, `rmrpc_bridge.rs`, `served_chain_seats.rs` ×5 |
| Format check (fuzz workspace) | ✔ yes — 8 sites |
| Bridge-exclusivity (`kayfabe-device` names both `RpcCommand` and `RmEvent`) | ✔ yes |
| Claim-ledger (`UNATTRIBUTED 382 > 381`, `CONFLATED 67 > 66`) | ✔ yes |

⇒ *"★ AT LEAST ONE GATE FAILED — do not push"* was **already true at `8c40f00`**, and it is
recorded here so the next agent does not spend the cycle deciding whether the red is theirs.
⊘ Not fixed here, deliberately: reformatting and re-attributing another increment's work is
`path_scoped_add_does_not_scope_the_commit` waiting to happen.

★ What this increment DID own and fixed: one clippy `useless_vec`, two rustfmt sites, and
`+1 UNATTRIBUTED / +1 CONFLATED` in the claim ledger — **attributed, never ratcheted**, back
to master's own 382/67/17.

★★ And two gates that were failing are now GREEN and were **nobody's code**: the hexagonal
and VMM-vocabulary gates `grep -r` four crate directories *whole*, and
`crates/kayfabe-abi/gen/target/` — created by the suite's **own** `abi-gen-tests` phase, and
gitignored — is inside one of them. `[measured]` two runs of the same tree at the same
revision: first green, second *"binary file matches"*. ⇒ The suite **armed its own failure**,
those gates were green only on a tree that had never built the ABI generator (i.e. exactly a
fresh GitHub runner, which is why CI could not catch it), and the fix is
`--exclude-dir=target` on all four — cargo output, never a source file, pattern and crate
list untouched.

### 14.27 ★★★★ THE WALL WAS THE OTHER HALF — `0x20810108` is NOT load-bearing, and a SECOND cause was invisible to the instrument (`[measured 2026-08-08, real GA106]`)

#### ⊘⊘ First, the refutation of the framing I inherited and carried

§14.26 closed by naming the next increment and splitting it in two:

> *"**The alloc is the easy half** … ⚠ **The control `0x20810108` is the hard half and has NO
> ORACLE.**"*

⊘ **Both halves of that sentence are refuted by measurement.** On a real GA106, driving a
real libcuda, refusing `0x20810108` with `NV_ERR_NOT_SUPPORTED` and **nothing else**:

```text
INJECT CTRL cmd=0x20810108 real_status=0x00000000 forced=0x00000056
cuInit(0) -> 0
```

`cuInit` **still returns 0**. The control that had no oracle, that source could not answer
because `binapiControl_IMPL` forwards it whole to GSP, that §14.26 said *"must come from a
real GA106 or be refused by name"* — **is not load-bearing at all.** Refusing it by name was
always going to be enough, and the entire difficulty was located in the half that does not
matter.

The half §14.26 called *easy* is the one that decides:

```text
INJECT ALLOC hClass=0x00002081 real_status=0x00000000 forced=0x00000056
cuInit(0) -> 100
```

★ I inherited this framing, restated it, and went looking for the reply body first. The run
that overturned it (`[measured 2026-08-08, real GA106 on `vh`, rev `6c9e3d2bb`]`) took under
a minute once the instrument existed. ⊘ **"Has no
oracle" is a statement about our instruments; "is required" is a statement about the driver.
They are unrelated, and I had silently treated the first as evidence for the second.**

#### ★★★ And the second cause, which no diff we own could have surfaced

`0x20800102` — `NV2080_CTRL_CMD_GPU_GET_INFO_V2` — **also produces `cuInit=100` on its own**,
refused alone on real hardware. It is co-equal with the `0x2081` alloc: serving either one
without the other still yields `100`.

§14.26 reported *"exactly **one** control id is new in the unserviced ledger relative to
`pro1` (23 → 24 distinct, `comm -13`)"*. That sentence is **true and was the wrong
question**, for a reason that is a property of the instrument:

★★ **The unserviced ledger is a de-duplicated, un-timestamped, un-counted SET dump printed
once at end of run.** Check it against the served-controls ledger printed eleven lines below
it in the same log — that one carries multiplicities (`x2`, `x4`, `x8`); the unserviced one
carries none. So a `comm -13` between two boots' ledgers answers *"which ids were **never**
demanded in the earlier boot"*, which is strictly weaker than *"which ids the CUDA process
demands"*. `GPU_GET_INFO_V2` is demanded by `RmInitAdapter` **and** by `cuInit`; it is
therefore in both boots' ledgers; and it is therefore **invisible to newness** while being
every bit as fatal as the id that was visible.

⇒ `gates_quantified_over_a_list` again, and `refusal_invisible_in_the_ledger`'s sibling: the
ledger was our rung-picking instrument, and it cannot express *when* or *how often*. ⊘ A set
difference over sets that record neither is not a statement about a window.

★ The positive instrument that does answer it is in the table below.

#### The instruments, and why two were needed

| instrument | what it is for | file |
|---|---|---|
| `cuda_ioctl_trace.c` | `LD_PRELOAD` on `ioctl(2)`, gated on `_IOC_TYPE=='F'` (never the escape number — `NV_ESC_*` collide with UVM's, `ioctl_nr_collision_bug`). Decodes NVOS21/64/00/54 and snapshots control params **before and after** | `scripts/rpctrace/` |
| `cuinit_probe.c` | `dlopen`s `libcuda.so.1` — ⊘ no toolkit on the bench — and walks `cuInit → cuDeviceGetCount → cuDeviceGet → cuCtxCreate`, writing `MARK` lines into the same append-mode trace so a demand is attributable to the call that provoked it | `scripts/rpctrace/` |
| `rmladder --binapi-ctrl` (R20) | allocs `NV2081_BINAPI` under the Subdevice and issues a control with the buffer **seeded `0xCD`** | `rmladder.rs` |
| the same interposer, injection mode | forces one status to `0x56` and nothing else, so *"libcuda asks X"* separates from *"libcuda needs X"* | `NVFAULT_CTRL` / `NVFAULT_ALLOC` |

★★ **R20 exists because the interposer cannot answer its own question**, and this is the
whole methodological point of the increment. The interposed trace records `0x20810108` as
992 bytes in, 992 bytes out, `NV_OK`, **every byte zero on both sides** — because libcuda
hands RM a zeroed buffer. ⊘ That cannot distinguish *"GSP wrote 992 zeros"* from *"GSP
returned `NV_OK` and wrote nothing"*. Seeded to `0xCD`, two runs byte-identical:

```text
★ R20 0x20810108 = NV_OK, 992 bytes: 00 cd×131 00000000 cd×848 00 cd×7
  written: offset 0 (1 byte), offsets 132..135 (4 bytes), offset 984 (1 byte) — all zero
  untouched: 986 of 992
```

⇒ The reply is **6 bytes**, not 992. *"The reply is 992 zeros"* — the reading the trace alone
licenses — is a **986-byte over-claim**. This is `c_oracle_empty_rows_are_wrong` approached
from the opposite side: there an *empty* capture was decoded as zeros; here a *zero* capture
would have been decoded as a written body. ★ An observer must not modify what it measures; a
ladder is free to, and that is why the project needs both.

#### `[measured]` The whole of `cuInit` on a real GA106 — the first capture past `nvidia-smi`

`traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt`: 9 allocs, ~60 controls, `cuInit → 0`.
Two facts in it bound what any port must serve:

- ★ **`0x2080012f` (`GPU_QUERY_ECC_STATUS`) returns `0x56` on real hardware and `cuInit`
  still succeeds.** A refusal mid-`cuInit` is survivable; refusals are not automatically
  walls, and this is the first direct evidence of that on the CUDA path.
- ⚠ **`GPU_GET_INFO_V2` is input-dependent.** libcuda sends `gpuInfoListSize=11` and eleven
  `(index, data)` pairs with `data=0`; RM fills `data` per index. ⊘ **No fixed-body table row
  can answer it** — a captured row would be right only for the exact index list that was
  captured, which is `a_table_does_not_decide_behaviour` waiting in a new place.

The eleven indices and the real part's answers, all capability booleans and all correct for a
consumer GA106 rather than magic numbers (`derive_what_you_cannot_query_then_oracle_it`):

| index | name (`ogkm-580: ctrl2080gpu.h`) | GA106 |
|---|---|---|
| `0x11` | — not named in this header — | 0 |
| `0x22` | `GEMINI_BOARD` | 0 |
| `0x27` | `GLOBAL_POISON_FUSE_ENABLED` | 0 |
| `0x2a` | `GPU_SMC_MODE` | 0 |
| `0x2d` | `GPU_FLA_CAPABILITY` | 0 |
| `0x37` | `GPU_DEBUGGING_CAPABILITY` | **1** |
| `0x3a` | `GPU_LOCAL_EGM_CAPABILITY` | 0 |
| `0x3b` | `GPU_SELF_HOSTED_CAPABILITY` | 0 |
| `0x3c` | `CMP_SKU` | 0 |
| `0x3d` | `DMABUF_CAPABILITY` | **1** |
| `0x44` | `COHERENT_GPU_MEMORY_MODE` | 0 |

#### ⊘ What this increment did NOT do, said plainly

**No port change landed and no guest boot was taken.** This increment is the measurement, and
`only_live_boots_are_proof` cuts both ways: a port change I could not boot would be `[built]`
wearing a `[measured]` section's clothes. ⊘ `cup2` still fails `cuInit(0) -> 100` at
`amb1_ee1994b`, unchanged, because nothing in the port changed.

★ What it *does* do is make the next increment a specification rather than a search, and it
subtracts a wall rather than climbing one.

#### ⇒ The next increment, specified by the injection matrix above (`[measured 2026-08-08, real GA106 on `vh`]`)

**Both of these, together — neither alone changes `cuInit`'s answer:**

1. **Admit `NV2081_BINAPI` (`0x2081`).** `alloc_params(0x2081) = NoDeclaredFacts` on §14.26's
   strong reading (`RS_OPTIONAL(NV2081_ALLOC_PARAMETERS)`, `{NvU32 reserved}`, no handle and
   no pointer, `resource_list.h:439-450`), `classify` under a Subdevice, the derivation
   ratchet `12 → 13`. Structurally identical to §14.26's `AMPERE_B` landing, and the same
   hazard check applies first: **what is keyed on this class being absent?**
2. **Serve `0x20800102` `GPU_GET_INFO_V2`** as a `WantedTable` arm that **reads the request**
   — echo each `(index, data)` pair the guest sent and fill `data` from the table above,
   exactly as `WantedTable::DeviceInfo` already reads its `baseIndex` cursor. ⚠ `params_size`
   is `4 + 8 × NV2080_CTRL_GPU_INFO_MAX_LIST_SIZE = 564`, and the guest's own
   `gpuInfoListSize` must be **bounded against that**, never trusted — it is a guest-supplied
   count that indexes a buffer.
3. **`0x20810108` — refuse by name, deliberately.** Measured non-load-bearing. If it is ever
   served, the truthful body is the 6 bytes above and **not** 992 zeros; recording that here
   is what stops the over-claim from being re-derived.

⚠ And the standing caution, now discharged rather than repeated: §14.26 said *"`cuInit`
failing 100 beside a refused `0x2081` is a correlation of two facts in one window, not a
proof of causation."* It was right to say so, and the correlation was **half right** — the
alloc is causal, the control beside it is not, and a third fact outside that window is
equally causal. ★ A correlation flagged honestly is still a correlation; the only thing that
retires one is the experiment.

### 14.28 ★★★★ BOTH HALVES LANDED — and §14.27's eleven-row table was at the WRONG BOUNDARY BY ONE LAYER

#### ⊘⊘ First, the refutation of the specification I was handed

§14.27 closed with a specification rather than a search, and it was right about the two
causes and right that they are co-equal. It was wrong about one thing, and the wrongness is
not conservative:

> *"Serve `0x20800102` `GPU_GET_INFO_V2` as a `WantedTable` arm that **reads the request** —
> echo each `(index, data)` pair the guest sent and **fill `data` from the table above**."*

★★★ **Ten of those eleven indices never reach a GSP.** `getGpuInfos`
(`ogkm-580: src/nvidia/src/kernel/gpu/subdevice/subdevice_ctrl_gpu_kernel.c:88-580`) is a
thirty-two-arm `switch` that answers `GEMINI_BOARD`, `GPU_SMC_MODE`,
`GPU_DEBUGGING_CAPABILITY`, `DMABUF_CAPABILITY` and twenty-eight more **from kernel state**,
writing each answer into `pParams->gpuInfoList[i].data` at `:566`. Only the `default:` arm is
forwarded, and it is marked: `pParams->gpuInfoList[i].index |= INDEX_FORWARD_TO_PHYSICAL`
(`:548`), a constant `0x8000_0000` `ct_assert`ed at `:84` to equal the SDK's own
`NV2080_CTRL_GPU_INFO_INDEX_RESERVED` bit. Only then does one `NV_RM_RPC_CONTROL` carry the
whole struct across (`:570-577`).

Of libcuda's eleven, exactly **one** — index `0x11`, unnamed in both vendored open headers —
hits `default:`.

⇒ A port built to §14.27's sentence would **overwrite ten values the guest's own kernel had
just computed**, with numbers that agree today only because both readings came off the same
machine. The table was *correct*; it is a reading of the **ioctl** boundary, and this port
lives one layer below it. `a_table_does_not_decide_behaviour`, in a new place — and note the
shape of how it survived: §14.27 measured `cuInit`, `cuInit` is an ioctl-boundary observer,
and *"the instrument that found the wall"* was silently promoted to *"the instrument that
specifies the fix."*

⚠ And a second thing the sentence would have got wrong on its own: a port keyed on `0x11`
would have matched **nothing**, because what arrives on the wire is `0x80000011`.

#### `[measured 2026-08-03, real GA106, task #178]` The oracle that settles it was already committed

`traces/rpctrace_ga106_boot1.bin` is a **GSP-level** RPC capture from a real GA106 boot
(2026-08-03, driver 580.159.04, task #178). It carries **three** `0x20800102` calls, all `status=0x0 psize=564`,
and they are the whole specification:

```text
seq303  REQ 01000000 11000080 00000000                    (listSize 1; 0x11 | FORWARD)
        REP 01000000 11000000 00000000                    (bit 31 CLEARED, data 0)
seq780  identical to seq303
seq806  REQ 02000000 23000080 00000000 24000080 00000000
        REP 02000000 23000000 58e0ec19 24000000 32251eb9
```

⇒ The forward bit is set on the request and **cleared in the reply**; the untouched tail
comes back verbatim; and two further indices — `0x23`, `0x24` — are demanded by the guest
kernel that libcuda never asks for.

#### ★★★ `0x23` / `0x24` are PER-CHIP IDENTITY VALUES, and that is why this port REFUSES them

A new rung settles it. `rmladder --gpu-info-sweep` (**R21**) asks all seventy indices **one
call each** — because `getGpuInfos` breaks its loop on the first non-`NV_OK` status
(`:566-569`), so a seventy-index request measures only *"the first index that fails"* — with
the tail seeded `0xCD`.

| source | GPU | `0x23` | `0x24` |
|---|---|---|---|
| `rpctrace_ga106_boot1.bin` seq806 | `GPU-e28d7776-…` | `0x19ece058` | `0xb91e2532` |
| R21, 2026-08-08, run 1 | `GPU-d0913685-…` | `0x4324d4e9` | `0x8708a4a8` |
| R21, run 2, same box | same | `0x4324d4e9` | `0x8708a4a8` |

**Stable across runs on one part, different between two parts.** Unnamed in both vendored
headers (`0x23`, `0x24` and `0x26` are blank between `GEMINI_BOARD` `0x22` and
`SURPRISE_REMOVAL_POSSIBLE` `0x25`); the handler is GSP firmware.

⊘ `derive_what_you_cannot_query_then_oracle_it` says *never a per-chip table*, and this is
precisely that shape. So `kayfabe_abi::gpuinfo` ships **one** row (`0x11 → 0`, which has
three independent readings across two parts) and refuses the rest by name
(`GpuInfoError::UnmeasuredForwardedIndex`).

⚠ **And this is NOT the `dlen = 0` mistake in reverse.** Answering `0x23`/`0x24` zero is not
decoding an absence to zeros — it is **contradicting four positive measurements**. The C
artifact did answer `0` (its map has no row, default zero, `C: nvkvm_gpu_emul.c:3226-3231`)
and still reached `bad=0 maxerr=0`, so zero is *probably* survivable — and *probably* is not
a reason to write a fabricated 32-bit identity into a reply the guest is free to cache
forever (`RMCTRL_FLAGS_CACHEABLE_BY_INPUT`, flags `0x30118`, `g_subdevice_nvoc.c:151`).

⊘ **And the refusal cannot regress anything**, which is the argument that makes it cheap
rather than brave: the control is *entirely* unserved today — seven committed bench boots log
`unserviced fn 76 cmd 0x20800102` — so serving two of the three recorded calls and leaving
the third exactly as it is is strictly more than the status quo on every call.

#### The hazard check, run first in the direction that has bitten three times

*"What is keyed on `0x2081` being absent?"* — **nothing.** `ObjectKind::Unknown` is
constructed in two places and **matched in zero**; `ObjectKind::Other` likewise. The graph
branches only on `Device`/`Client`/`Memory`/`Event` (`rmgraph.rs:1346, 1429, 2372, 2399`),
the projection only on `VaSpace`/`Tsg`/`CtxShare`/`Channel`/`EngineObject`
(`project.rs:726, 758`), and `origin_of_kind` is a discriminant compare no caller can aim
here. `0x2081` was **already permitted** by the capability table (`capability.rs:919-923`,
`Origin::Nvproxy`); the refusal came from `alloc_params` returning `None` one statement
later. ⚠ ⊘ NOT `EngineObject` — that is the one variant that rewrites a *sibling* node's
routing.

#### What landed

| piece | where |
|---|---|
| `NV2081_BINAPI` generated from `cl2081.h:33` (both tags agree) | `kayfabe-abi/gen/src/main.rs`, `generated/classes.rs` |
| `alloc_params(0x2081) = NoDeclaredFacts`; ratchet `12 → 13` | `versions.rs`, `capability.rs` |
| `classify(0x2081) = ObjectKind::Other` — its **first** constructor | `kayfabe-chips/src/ga10x.rs`, `kayfabe-mocks` |
| ★ `WireClassArch` also gained `NV20_SUBDEVICE_0` and `NV01_EVENT_KERNEL_CALLBACK_EX`, missing since it was written — the silent-`Unknown` trap its own comment warns about | `kayfabe-mocks/src/lib.rs` |
| `kayfabe_abi::gpuinfo` — the forward-bit ABI, the one-row table, five named refusals, 9 unit tests | `crates/kayfabe-abi/src/gpuinfo.rs` |
| `WantedTable::GpuInfoV2`, the **request-editing** arm; `ChipProfile::forwarded_gpu_info` | `kayfabe-device/src/inittables.rs`, `lib.rs`, `ga10x.rs` |
| R21 `--gpu-info-sweep` | `rmladder.rs` |
| served universe `24 → 25`; branch-(a) cacheable `4 → 5` (the first via `CACHEABLE_BY_INPUT`) | `init_tables.rs`, `sticky.rs` |
| `CLAIMED_BUT_REFUSED` gains `0x20800102` with its argument — **2 served / 1 refused** of the three recorded calls, which is exactly the argument-keyed shape that row demands | `replay_conformance.rs` |

★ `replay_conformance.rs` is the strongest half: `n_claimed` moves `84 → 87` and
`size_checked` `24 → 25` against a real-GA106 GSP capture, so the reply's `paramsSize` is
hardware-evidenced rather than declared.

#### ⊘⊘⊘ THE BOOT: both halves landed on the wire, and `cuInit` STILL RETURNS 100

`[measured 2026-08-08, boot `gi1_e6ed6bc`, real GA106 on `vh`, shipping config,
`probe-arm set: EMPTY`, **STOCK** guest module, both stamps
`kayfabe-rev:e6ed6bcf8647a0df459c80c9f11a922be3da1936`]`:

```text
SMI_RC=0
FAIL cuInit(0) -> no CUDA-capable device is detected (100)
```

⊘ **Not "the change did not take".** The wire says both halves landed, three ways:

| evidence | before (`amb1_ee1994b`) | after (`gi1_e6ed6bc`) |
|---|---|---|
| `0x20800102` in the **unserviced** ledger | present | ★ **gone** — `control 0x20800102 result 0x00000000 x1` |
| `0x2081` among `GspRmAlloc failed` | refused | ★ **absent** — the refused classes are now `0x70`, `0xc36f`, `0x402c`, `0x208f` |
| controls demanded | 24 unserviced | 32, incl. **nine never demanded before** |
| `0x20810110` | never seen | ★ present — a **second** control on the BinApi handle, which libcuda can only issue on a handle it now owns |

★ The boot **advanced**. `RmInitAdapter` succeeds, `nvidia-smi` returns 0, and nine control
ids reach this port for the first time. What did not move is `cuInit`'s answer.

#### ⊘⊘ THE REAL REFUTATION — an injection matrix measures NECESSITY and can NEVER measure SUFFICIENCY

§14.27 forced one status to `0x56` **at a time, on a working system** — real GA106, real
libcuda, real GSP firmware answering everything else correctly. That experiment removes X and
observes failure, which proves X is **required**. It says nothing whatever about what else is
required, because nothing else was ever removed. §14.27 wrote it up as *"⇒ Two co-equal
causes … Land both, then boot"*, and I carried that sentence as a **complete** specification.

⊘ It was a pair of *necessary* conditions produced by a method that is structurally incapable
of enumerating the rest. Two necessary conditions do not make a sufficient one, and the boot
above is the falsifier.

★★★ **And the instrument is now exhausted, which is itself the measurement.** Every id this
port still refuses on the `cuInit` path was put through the same injection matrix on real
hardware, one at a time (`[measured 2026-08-08]`, `inj_1428.sh`):

```text
BASELINE (nothing forced)          cuInit(0) -> 0
refuse CTRL 0x20810110/0x208f1105/0x20809009/0x2080014b/0x20800157/0x20801357
           /0x2080a612/0x2080a618/0x2080012b/0x00800294/0x20810108   -> 0  (all)
refuse ALLOC 0x70/0xc36f/0x402c/0x208f                               -> 0  (all)
```

**Sixteen for sixteen non-load-bearing.** So the next cause is **not a single refused id**,
and single-fault injection cannot find it. Two reasons, both structural:

1. ⊘ **It runs on hardware that WORKS.** Every id it clears is cleared in a world where
   everything else is answered by real firmware. Our port answers many of those differently.
2. ⊘ **It can only turn `NV_OK` into `0x56`.** It has no way to turn a *right* answer into a
   *wrong* one — so a control this port SERVES with a wrong body is invisible to it, by
   construction. That is `mock_fidelity_both_directions` and
   `refusal_invisible_in_the_ledger` meeting in one instrument.

⇒ The next increment needs a **comparison, not a subtraction**: the same interposer run
**inside the guest**, diffed positionally against
`traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt`. That names a served-but-wrong answer,
which is the only remaining class this project has an oracle for.

#### ★★★★ THE WALL, NAMED — and it is ONE LINE of the guest's own `cuInit` trace

The comparison instrument was built and run the same night: the §14.27 `LD_PRELOAD`
interposer, pushed **into the guest** and run over `cuInit`. Boot `gt1_e6ed6bc`, same
shipping config, same stamps. The whole trace is committed as
`traces/real_ga106/cuinit_ioctl_trace_guest_gt1_e6ed6bc.txt` — **44 ioctls**, and the last
one before teardown is this:

```text
CTRL cmd=0x20800102 hObject=0x5c000003 size=564 status=0x00000056 rc=0
  in = 0b000000 11000000 00000000 22000000 00000000 …   (libcuda's eleven indices)
  out= 0b000000 11000000 00000000 22000000 00000000 …   ⊘ BYTE-IDENTICAL TO `in`
FREE ×3
== stage1 cuInit END
cuInit(0) -> 100
```

Beside it, the real part on the same 564 bytes
(`traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:42`): `status=0x00000000`, and
`0x37 → 1`, `0x3d → 1`.

⇒ **`out == in`. Not one of the eleven `data` fields was written.** The guest kernel bailed
before it filled anything, and `cuInit` gives up on the next line.

#### ⊘⊘ AND THIS REFUTES MY OWN INCREMENT'S GREEN ROW

The port's served ledger for that very boot says:

```text
control 0x20800102 result 0x00000000 x1
```

★ One call, served, `NV_OK`. **And libcuda's call is not it.** libcuda's `GPU_GET_INFO_V2`
produced **no ledger row of any kind** — not served, not refused, not unserviced. It never
reached this port. The `0x56` was manufactured **inside the guest kernel, without an RPC**.

⊘ So *"the control is served"* and *"the guest's ioctl succeeds"* are two different facts, and
this increment established the first while reporting it as though it settled the second. The
served ledger is a record of what **crossed the RPC boundary**; a control the guest answers
locally is invisible to it in exactly the way `refusal_invisible_in_the_ledger` describes —
one boundary further out. ⇒ **Every instrument this port owns lives at the GSP boundary, and
`cuInit` is decided at the ioctl boundary.** The guest-side interposer is the first
instrument we have that spans them, which is why it found in one boot what sixteen injections
could not.

⚠ `[unmeasured]` **Why** the kernel answers `0x56` without an RPC is the next increment's
question, and it must not be guessed. Two candidates, both testable from the same trace:
`getGpuInfos`'s switch bailing on an arm before it reaches `default:` (its loop `break`s on
the first non-`NV_OK` and returns it for the whole call, `:566-569`), or the RM control
**cache** — this id is `CACHEABLE_BY_INPUT` and is now row five of
`kayfabe_device::sticky::BRANCH_A_CACHEABLE`, whose whole thesis is that *no reply of ours can
influence it*. ⊘ Do not pick between them from source; the guest will say which, and the
instrument that asks is already built.

### 14.29 ★★★★ THE WALL FELL — one index of eleven, and it was `0x20800a4c`

#### ⊘⊘ First, the refutation of the framing I was handed — and it is my predecessor's own

§14.28 handed me two candidates and called them co-equal: *"`getGpuInfos`'s switch bailing
on an arm before it reaches `default:` … or the RM control **cache**."* ★ **They were never
co-equal**, and one measurement separates them completely. The cache hypothesis makes a
prediction — *"the answer does not depend on which index is asked"* — and the bisect below
falsifies it in eleven lines. ⇒ A pair of candidates is only co-equal until someone writes
down what each predicts; §14.28 named them and stopped one step short of that.

⚠ And the second thing that framing got wrong: it called the two §14.28 halves *"both
necessary"* and treated `GpuInfoV2` as the half that might still be wrong. `GpuInfoV2` was
**already correct** — all ten of its kernel-answered indices match a real GA106 exactly. The
half that was missing was a control **nobody had classified as part of `cuInit` at all**.

#### `[measured 2026-08-08, boot `gis1_e6ed6bc`, real GA106 on `vh`, shipping config, `probe-arm set: EMPTY`, STOCK module]`

The instrument is `scripts/rpctrace/cuda_ioctl_trace.c` with a new `NVSWEEP_GPUINFO` mode,
driven by `scripts/bench/guest_gpuinfo_sweep.sh`. ★ It reuses **libcuda's own
`hClient`/`hObject`**, on the same fd, in the same process, at the same instant — so it needs
no client-allocation path of its own and a disagreement with `rmladder` R21 is a fact about
the machine rather than about a second unverified harness. It fires once, AFTER the observed
call has completed and been logged.

```text
SWEEP-BEGIN hClient=0xc1d0000c hObject=0x5c000003 observed_n=11
SWEEPIDX pos=0  idx=0x11 status=0x00000000        SWEEPPFX len=1 status=0x00000000
SWEEPIDX pos=1  idx=0x22 status=0x00000000        SWEEPPFX len=2 status=0x00000000
SWEEPIDX pos=2  idx=0x27 status=0x00000000        SWEEPPFX len=3 status=0x00000000
SWEEPIDX pos=3  idx=0x2a status=0x00000056  ★     SWEEPPFX len=4 status=0x00000056  ★
SWEEPIDX pos=4..10 (0x37,0x3b,0x3c,0x3d,0x2d,0x3a,0x44)  all status=0x00000000
                                                  SWEEPPFX len=5..11 status=0x00000056
```

⇒ **Exactly one** of libcuda's eleven indices poisons the call, and the prefix sweep puts the
break at exactly its position. ⊘ The cache is refuted: ten of eleven answer `NV_OK` on the
same handle at the same instant.

`GPU_SMC_MODE` (`0x2a`) is **not** answered from kernel state on a bare-metal GSP client. Its
arm issues `NV2080_CTRL_CMD_INTERNAL_GPU_GET_SMC_MODE` (`0x20800a4c`) on the **physical**
RMAPI and assigns *its* status to the loop (`ogkm-580: subdevice_ctrl_gpu_kernel.c:232-266`);
the loop `break`s on the first non-`NV_OK` and returns it **for the whole call**
(`:566-569`). Corroboration at the other boundary, same boot: `unserviced fn 76 cmd
0x20800a4c` — and in seven earlier committed bench boots.

★ And the ten that already worked matched a real GA106 **exactly** (`0x11=0 0x22=0 0x27=0
0x37=1 0x3b=0 0x3c=0 0x3d=1 0x2d=0 0x3a=0 0x44=0`). §14.28's arm was right all along.

#### ⊘⊘ THE REASON IT SURVIVED: a correct set-difference and a wrong inference

`docs/reference/remaining_boot_surface.md` §1 computed `rows − transcript = {0x20800a4c}`
over two committed artefacts and concluded: *"The transcript covers the init set exactly,
missing precisely the one command that is **by definition not part of init**."* ★ Every word
of the derivation is right. The inference — that the leftover was therefore uninteresting —
is what carried the wall for four rungs.

⇒ *"Not reached during `RmInitAdapter`"* and *"not needed"* are different statements, and
**every oracle this project owns was `nvidia-smi`-driven**, so none of them could have
distinguished them (`traces/real_ga106/README.md`, "the method row is also this directory's
blind spot"). The single unexplained row in a set-difference is not residue; it is the only
place the blind spot could show through.

#### The value, and where it must NOT come from

⊘ **Not from the C oracle.** `C: mode2_initctrl_ga106.h:6243` is `0x20800a4c`'s row, `psize 4,
dlen 0` — one of the eleven **empty** rows, nine of which hardware contradicts, and
`traces/real_ga106/README.md` already marks it *"⚠ coincides"*: nothing **about the row**
distinguishes it from the nine that are wrong. The value comes from two positive measurements
on two different physical parts (`rpc_bodies_real_ga106.txt:617-628` = `00 00 00 00`;
`rmladder_r21 … 0x2a NV_OK data=0`), read by two different instruments.

★ `kayfabe_abi::smcmode` encodes an **enum**, never a `u32`. On this control the correct
answer *is* four zero bytes, so — unlike `ce_fault_method_buffer_size`, where zero is the
sentinel for *unstated* and realize refuses it — **a numeric sentinel is unavailable by
construction**. The type is the only place *"UNSUPPORTED, measured"* survives a refactor
apart from *"nothing was written"*. `tests/internal_gpu_get_smc_mode.rs` poison-fills the
request with `0xAA` for exactly that reason: no assertion on the *value* could tell a served
answer from an unwritten buffer here.

#### The hazard check, run first, in the direction that has bitten three times

*"What is keyed on `0x20800a4c` being absent?"* — **nothing behavioural.** It has no
`sweep::TRIAGE` row (it was never in the sweep universe: not an init control), no capability
entry, and no `sticky::BRANCH_A_CACHEABLE` row — its own export flags are `0xc0`
(`ogkm-580: g_subdevice_nvoc.c:2530-2540`), which carries no `RMCTRL_FLAGS_CACHEABLE_*` bit.
The only mentions are `kayfabe_abi::oracle`'s empty-row table — a fact about the *C table*,
unaffected by what we serve — and two reference docs.

#### `[measured 2026-08-08, boot `w1429_49b182a`, both stamps `49b182a…`]` THE WALL IS DOWN

```text
SWEEPIDX pos=0..10   ALL status=0x00000000   (pos=3 idx=0x2a was 0x56 at e6ed6bc)
SWEEPPFX len=1..11   ALL status=0x00000000
CTRL cmd=0x20800102 … size=564 status=0x00000000     ← the real eleven-index call
nvkvm: control 0x20800a4c result 0x00000000 x5       ← served, and GONE from unserviced
```

#### ⊘⊘⊘ AND `cuInit` STILL RETURNS 100 — the wall MOVED, five ioctls further

`cup2`, verbatim, boot `v1429_49b182a`, shipping config, stock module:

```text
SMI_RC=0
=== cup2: run ===
FAIL cuInit(0) -> no CUDA-capable device is detected (100)
CUP2_RC=1
```

The guest-side trace says where it now stops:

```text
0x20800102  status=0x00000000   ← the §14.28/§14.29 wall, now served
0x20801701  status=0x00000000
0x20801823  status=0x00000000   ← BUS_GET_INFO_V2, 3 entries (0x00,0x02,0x0b)
0x20801801  status=0x00000000
0x20801823  status=0x00000056   ★ THE NEW WALL — the SECOND call, 6 entries
FREE ×3 ; cuInit(0) -> 100
```

★★ **The oracle for it is already committed and the request bytes are identical.**
`traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:46` is the same six-entry request,
answered `status=0x00000000` by a real part; line 44 is the three-entry call, and our guest
already reproduces **that** one byte for byte.

```text
in : 06000000 |0f000000 03000000|10000000 00000000|2c000000 05000000|
               2d000000 00000000|03000000 00000000|06000000 00000000
out: 06000000 |0f000000 00000000|10000000 07000000|2c000000 00000000|
               2d000000 20300003|03000000 033d4500|06000000 00000000
```

⚠⚠ **And §14.28's trap is present again, in the same shape — read this before building a
table from those bytes.** Of the six indices, exactly **one** is RPC-forwarded on a GSP
client: `0x2d` `PCIE_GEN_INFO`, the first case label of `getBusInfos`'s
`bSendRpc = IS_VIRTUAL(pGpu) || IS_GSP_CLIENT(pGpu)` group (`ogkm-580:
kern_bus_ctrl.c:283-296`). The other five — `0x0f` `BUS_NUMBER`, `0x10` `DEVICE_NUMBER`,
`0x2c` `DOMAIN_NUMBER`, `0x03` `PCIE_GPU_LINK_CAPS`, `0x06` `PCIE_DOWNSTREAM_LINK_CAPS` — are
computed by the guest's own kernel and **must not be written by this port**. And
`kbusSendBusInfo` forwards **one entry at a time** under `NV_CHECK_OK_OR_RETURN`
(`:333`), so a single refused entry returns for the whole call — the same failure mode as
`getGpuInfos`, reached by a different mechanism.

⊘ `[unmeasured]` **What `0x2d` must carry.** `0x03003020` is what one part answered.
`PCIE_GEN_INFO` describes the **link**, not the die, so it is the `0x23`/`0x24` shape: no
chip-family row may state it. ⇒ The next rung is an `rmladder` sweep of `BUS_GET_INFO_V2` on
the host — unprivileged, exactly the way R21 obtains `GPU_INFO` — before any value is written
down. ⚠ A single-part reading pasted into a chip row here would be the `0x20802a08` mistake
with a different id.

#### What landed

| piece | where |
|---|---|
| `NVSWEEP_GPUINFO` — the in-guest bisect, on the caller's own handles | `scripts/rpctrace/cuda_ioctl_trace.c` |
| `guest_gpuinfo_sweep.sh` — the `POST_CAPTURE_HOOK` that drives it | `scripts/bench/` |
| ★ `guest_cuinit_trace.sh`: `NVTRACE_FILE` → `NVTRACE_OUT`. The interposer reads `NVTRACE_OUT`, so that line wrote **no file**; §14.28's trace survives only because ssh carried stderr, and the script's own `wc -l`/`cat` were reading a path that never existed | `scripts/bench/` |
| `kayfabe_abi::smcmode` — enum ABI, two-part provenance, 5 unit tests | `crates/kayfabe-abi/src/smcmode.rs` |
| `ChipProfile::smc_mode`, typed; `WantedTable::InternalGpuGetSmcMode`; served universe **25 → 26**, attributed | `kayfabe-device` |
| 5 integration tests over a `0xAA`-poisoned request | `tests/internal_gpu_get_smc_mode.rs` |
| the two bisect captures, annotated with their boot tags and stamps | `traces/real_ga106/gpuinfo_bisect_guest_gis1_e6ed6bc.txt`, `cuinit_bisect_guest_w1429_49b182a.txt` |

### 14.30 ★★★★ `BUS_GET_INFO_V2` SERVED — and its value is the first this port DERIVES, because the oracle MOVES

#### ⊘⊘ First, the refutation of the framing I was handed — and it is mine as much as my predecessor's

§14.29 closed with a specification that was right to stop and wrong in two of its
particulars, and I carried both for the first hour:

> *"`[unmeasured]` **What `0x2d` must carry.** `0x03003020` is what one part answered."*

★ **`0x03003020` is not what any part answered.** The reply bytes in
`traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:46` are `2d000000 00203000`, which is
index `0x2d` and data **`0x00302000`**. The published word was the same bytes re-grouped one
boundary off — and it **decodes plausibly** (`GEN=gen4` instead of `gen3`), which is exactly
why reading it never caught it. `[measured]` R22 replays that six-entry request on a real
GA106 and reproduces the whole reply, all six entries byte for byte.

⚠ Worth naming as a class: **a mis-transcription that still decodes is not a typo, it is a
second oracle.** Nothing about `0x03003020` looks wrong; it looks like a Gen4 part. The only
thing that separates it from the truth is asking the hardware again.

⊘ And the second particular: §14.29 said the next rung was *"an `rmladder` sweep … before any
value is written down."* Correct, and **a sweep alone could not have answered it.** The idle
sweep returned **sixteen identical words** — which reads as *"constant, safe to tabulate"* and
is worth nothing, because an idle PCIe link is a constant link. The measurement below only
exists because the link was made to move.

#### `[measured 2026-08-08, vh, RTX 3060 (GA106) `GPU-d0913685`, driver 580.159.04 Open, rev `4e79a14`]`

R22 (`rmladder --bus-info-sweep`) asks all 52 `BUS_GET_INFO_V2` indices one call each, replays
libcuda's own two requests byte for byte, and then reads `0x2d` sixteen times. Run twice: once
with the link idle, once with `scripts/rpctrace/pcie_link_load.c` moving 11.2 GiB/s across it.

| link | `current_link_speed` | `nvidia-smi gen.current` | `0x2d` | `0x03` |
|---|---|---|---|---|
| idle | 2.5 GT/s | 1 | `0x00302000` (×16, 1 distinct) | `0x00453d03` |
| loaded | 8.0 GT/s | 3 | ★ **`0x00322000`** (×16, 1 distinct) | `0x00454d03` |

The delta is `0x0002_0000` — bits 19:16, `CURR_LEVEL`, `GEN1 → GEN3`. **The same physical
part, minutes apart, answers two different words.** Evidence:
`traces/real_ga106/rmladder_r22_businfo_{sweep,loaded}_real_ga106.txt`.

#### ★★★ THE ANSWER TO §14.29's QUESTION: the word holds THREE generations and only one is the die's

Decoded with `NV2080_CTRL_BUS_INFO_PCIE_LINK_CAP_*` (`ogkm-580: ctrl2080bus.h:355-390`) and
checked against `nvidia-smi` on the same box in the same second — three fields, three
different numbers, three different owners:

| field | bits | value | `nvidia-smi` | owner |
|---|---|---|---|---|
| `GPU_GEN` | 23:20 | `gen4` | `pcie.link.gen.gpumax = 4` | ★ **the die** |
| `GEN` | 15:12 | `gen3` | `pcie.link.gen.max = 3` | **the slot** (this box's root port is 8 GT/s) |
| `CURR_LEVEL` | 19:16 | `gen1`→`gen3` | `pcie.link.gen.current = 1`→`3` | **the live link** |

⇒ **No chip-family row may state this word**, and that is now measured rather than argued.
`0x00302000` in a `GA106` row would claim every GA106 sits in a Gen3 slot idling at 2.5 GT/s —
wrong on a Gen4 board, and wrong on **the same box thirty seconds later**. ★ Note the two
"static-looking" fields already disagree *with each other* on the one box we own: the die is
gen4 and the slot is gen3. A single reading could never have shown that.

#### What this port serves, and the residual named rather than elided

The chip row states **one enum** — `ChipProfile::pcie_max_gen`, the die's own maximum
generation, `[measured]` `Gen4` on GA106 by two instruments — and the word is **derived** by
`PcieGenInfo::fully_trained`: the emulated link is presented as trained at the die's own
generation, so `GPU_GEN == GEN == CURR_LEVEL` and the served word is `0x00333000`.

⊘ Deliberately **not** either measured word. This is a statement about the link *this port
presents*, identical on every host by construction, derived for every architecture from one
enum rather than tabulated per chip — `derive_what_you_cannot_query_then_oracle_it`.

⚠ **The residual:** the guest's DMA really does traverse the *host's* link, which this reply
does not describe. The truthful upgrade is `current_link_speed` / `max_link_speed` from sysfs
— world-readable, no privilege, no RM ioctl — folded into `GEN`/`CURR_LEVEL`. ⊘ Not done here
because **the shipping archive has no host GPU binding at all**: `host-isolates` is off by
default (`kayfabe-qemu-raw/Cargo.toml:87`) and `InitTablePolicy` holds a `&'static ChipProfile`
and nothing else. `PcieGenInfo`'s three independent fields ARE that seam.

#### ⚠ §14.28's trap, present for the third time — and this time there is no bit to key on

Of the six indices in the failing request, exactly one is RPC-forwarded on a GSP client:
`0x2d`. `0x0f` `BUS_NUMBER`, `0x10` `DEVICE_NUMBER`, `0x2c` `DOMAIN_NUMBER`, `0x03`
`PCIE_GPU_LINK_CAPS` and `0x06` `PCIE_DOWNSTREAM_LINK_CAPS` are the guest kernel's own
(`ogkm-580: kern_bus_ctrl.c:283-470`).

★★★ **And unlike `GPU_GET_INFO_V2` there is no forward bit — because none is needed.**
`kbusSendBusInfo_IMPL` forwards one entry at a time in a **fresh** params struct with
`busInfoListSize = 1` (`ogkm-580: kern_bus.c:1065-1101`). The six-entry struct is the
**ioctl**; what reaches a GSP is a **one-entry RPC per forwarded index**, and the boot's
ledger agrees — `unserviced fn 76 cmd 0x20801823` appeared exactly **once** at `49b182a`.
⇒ Arriving here IS the marker, so every declared entry is filled and an index with no
derivation is refused by name. ⊘ Never zero-filled: `PCIE_LINK_CAP_GEN_GEN1 == 0`, so a zero
entry is the positive claim *"Gen 1"* rather than an absence.

#### The hazard check, run first, in the direction that has bitten three times

*"What is keyed on `0x20801823` being absent?"* — **nothing.** It is already permitted by the
capability table (`capability.rs:749`, `Origin::Nvproxy`); its flags are `0x10118`
(`ogkm-580: g_subdevice_nvoc.c:6800-6812`), carrying neither `RMCTRL_FLAGS_CACHEABLE`
(`0x400`) nor `_CACHEABLE_BY_INPUT` (`0x20000`), so it is **not** a
`sticky::BRANCH_A_CACHEABLE` row — which matters here more than usual, because a value that
moves with the link must never be cached for the life of a boot.

#### `[measured 2026-08-08, boot `bus1430_0dbbabc`, both stamps `0dbbabc…`, shipping config, `probe-arm set: EMPTY`, STOCK module]` THE WALL IS DOWN

```text
nvkvm: control 0x20801823 result 0x00000000 x1     ← served, and GONE from unserviced
```

`cup2`, verbatim:

```text
SMI_RC=0
=== cup2: run ===
FAIL cuInit(0) -> no CUDA-capable device is detected (100)
CUP2_RC=1
```

#### ⊘⊘⊘ AND `cuInit` STILL RETURNS 100 — the wall moved ONE control further

Boot `gt1430_0dbbabc`, the in-guest interposer, 52 lines on disk
(`traces/real_ga106/cuinit_trace_guest_gt1430_0dbbabc.txt`):

```text
0x20800102  status=0x00000000     ← §14.29's wall
0x20801701  status=0x00000000
0x20801823  status=0x00000000     ← 3 entries
0x20801801  status=0x00000000
0x20801823  status=0x00000000     ★ §14.29's wall, now SERVED — 6 entries
0x20801803  status=0x00000000
0x2080182a  status=0x00000056     ★★ THE NEW WALL
FREE ×3 ; cuInit(0) -> 100
```

`0x2080182a` is `NV2080_CTRL_CMD_BUS_GET_PCIE_SUPPORTED_GPU_ATOMICS`
(`ogkm-580: ctrl2080bus.h:1273`), flags `0x40048`, params 112 bytes, and it is the **very next
line of the real-hardware trace** (`cuinit_ioctl_trace_real_ga106.txt:48`), where a real GA106
answers `NV_OK`.

#### ★★★ THE NEXT RUNG IS NOT "COPY THE TRACE" — the oracle is BLIND here, in the `dlen = 0` shape

Two facts, both measured today, that the next increment must not skip past:

1. ⊘ **The committed trace's body for `0x2080182a` is ambiguous by construction.** Its `in=`
   and `out=` are both 112 zero bytes, and `traces/real_ga106/README.md` already warns why
   that decides nothing: *"libcuda hands RM zeroed buffers, so an all-zero pair is ambiguous."*
   This is `c_oracle_empty_rows_are_wrong` reached by a different route — **an all-zero body
   read out of a zero-filled request is evidence of NOTHING, not evidence of zeros.**
2. ⊘ **And the `0xCD`-seed instrument that would resolve it CANNOT reach the control.**
   `[measured]` `rmladder --probe-ctrl 0x2080182a:112`, twice, on the real GA106:
   `refused Other(86)` — `NV_ERR_NOT_SUPPORTED`, both times. So on the same physical part, in
   the same hour, **libcuda gets `NV_OK` and a bare Subdevice gets `0x56`.** The handler is a
   `_DISPATCH` (`g_subdevice_nvoc.c:6809`), so the answer depends on caller state that
   `rmladder` does not reproduce, and *"the control is unsupported"* is a reading the evidence
   does not support.

⇒ The rung is **not** a value to transcribe. It is: find what makes the answer differ between
the two callers (`rmladder` allocates a Subdevice and stops; libcuda has a Device, an
`NV2081_BINAPI`, and eleven earlier controls behind it), and only then decide what this port
says. ★ A single-caller reading pasted into a table here would be `0x20802a08` for the third
time.

⚠ And one instrument note, since it cost nothing only because it was checked: R22's own idle
run would have "confirmed" a constant sixteen times over. **The perturbation is the
instrument** — `scripts/rpctrace/pcie_link_load.c` exists so that a value suspected of
describing the link can be caught moving, on one box, with no second part to rent.

#### What landed

| piece | where |
|---|---|
| R22 `--bus-info-sweep` — 52 indices one call each, libcuda's two requests replayed byte for byte, `0x2d` ×16 decoded, both hypotheses written down before the run | `crates/kayfabe-isolate-host/src/bin/rmladder.rs` |
| ★ `pcie_link_load.c` — the perturbation that turns a decode into a measurement | `scripts/rpctrace/` |
| `kayfabe_abi::businfo` — the ABI, `PcieGen`/`PcieGenInfo`, the request-editing answer, four named refusals, 8 unit tests | `crates/kayfabe-abi/src/businfo.rs` |
| `ChipProfile::pcie_max_gen` — an **enum**, one field, not the `u32` the control returns | `kayfabe-device/src/lib.rs`, `ga10x.rs` |
| `WantedTable::BusGetInfoV2`; served universe **26 → 27**, attributed | `kayfabe-device/src/inittables.rs` |
| the two R22 captures and the guest trace | `traces/real_ga106/rmladder_r22_businfo_{sweep,loaded}_real_ga106.txt`, `cuinit_trace_guest_gt1430_0dbbabc.txt` |
| a citation `truncated_row_reads` refused — §14.29's `ga106.h:6243` lines now name `0x20800a4c` | `smcmode.rs`, this file |

### 14.31 ★★★★ `0x2080182a` SERVED — the refusal that "proved caller-dependence" was the INSTRUMENT'S OWN SEED, and the new wall has NO LEDGER ENTRY

#### ⊘⊘ First, the refutation of the framing I was handed — and it is §14.30's, which is mine as much as my predecessor's

§14.30 closed with a finding stated in two clauses, one measured and one inferred:

> *"`[measured]` `rmladder --probe-ctrl 0x2080182a:112`, twice, on the real GA106: `refused
> Other(86)`, both times. So on the same physical part, in the same hour, **libcuda gets
> `NV_OK` and a bare Subdevice gets `0x56`.** The handler is a `_DISPATCH`
> (`g_subdevice_nvoc.c:6809`), so the answer depends on caller state that `rmladder` does not
> reproduce."*

★ **The two callers never issued the same call.** `capType` is an **`[IN]`** field
(`ogkm-580: ctrl2080bus.h:1256-1258`, struct at `:1311-1315`), and `probe_ctrl` seeds *every*
byte with `0xCD` — so R18 asked for `capType = 0xCDCDCDCD`, which is none of
`_CAPTYPE_SYSMEM(0)` / `_GPU(1)` / `_P2P(2)` (`:1226-1228`). libcuda hands RM a **zeroed**
buffer, so it asks for `_CAPTYPE_SYSMEM`. Nothing about the caller was ever in play.

⇒ ★★ **The `0xCD` sentinel is sound only on a pure-`[OUT]` struct.** On a struct with an
`[IN]` field it is an input **mutation**, and the instrument perturbs the thing it measures —
`probe_ctrl`'s own doc-comment reasons entirely about *"whether RM **touched** the buffer"*
and never once about what the buffer *says*. Same family as §14.30's own lesson (the
perturbation is the instrument), with the sign flipped: there a perturbation was needed to see
a value move; here an unintended perturbation manufactured a refusal.

⚠ And the `_DISPATCH` was never the decider. This control's flags are `0x40048` =
`NON_PRIVILEGED | ROUTE_TO_PHYSICAL | PHYSICAL_IMPLEMENTED_ON_VGPU_GUEST`
(`ogkm-580: g_subdevice_nvoc.c:6806-6819`, `rmapi/control.h:202-308`), so on a bare-metal GSP
client the kernel RM **never runs the local arm at all** — it RPCs the whole struct to GSP-RM,
which is exactly why the boot ledger carries `unserviced fn 76 cmd 0x2080182a` once. The
`_92bfc3` arm NVOC installs for every non-VF variant is a bare `return NV_ERR_NOT_SUPPORTED`
(`g_subdevice_nvoc.h:6999-7002`) that exists **because it should never run**. ⇒ Reading a HAL
suffix and inferring caller-dependence skipped the two flags that say the HAL is bypassed.
★ A `_DISPATCH` suffix is a fact about NVOC codegen, not about who is calling.

#### `[measured 2026-08-08, real GA106 `GPU-d0913685`, driver 580.159.04 Open, rev `1d5704dd9`]` R23

`rmladder --atomics-probe` — eight arms, on the **same bare Subdevice R18 used**, allocating
nothing. Hypotheses written down before the run: H1 = caller state (every arm refuses),
H2 = the seed (the `0/1/2` arms answer). Evidence:
`traces/real_ga106/rmladder_r23_atomics_real_ga106.txt`.

| arm | `capType` | tail seed | result |
|---|---|---|---|
| R18 replay | `0xCDCDCDCD` | `0xCD` | refused `0x56` — §14.30 reproduced exactly |
| captype poisoned only | `0xCDCDCDCD` | `0x00` | refused `0x56` |
| ★ **SYSMEM, tail poisoned** | `0` | `0xCD` | **`NV_OK`, body WRITTEN** |
| libcuda replay | `0` | `0x00` | `NV_OK`, body indistinguishable by construction |
| GPU | `1` | `0xCD` | refused `0x56` |
| P2P | `2` | `0xCD` | refused `0x56` |
| undeclared | `3` | `0xCD` | refused `0x56` |
| SYSMEM, `dbdf` poisoned | `0` | `0xCD` | `NV_OK`, `dbdf` echoed back `0xCDCDCDCD` |

**H1 is dead and H2 holds.** The 2x2 is what separates *"the captype is invalid"* from *"a
seeded byte anywhere is refused"*: poisoning only the tail still answers, poisoning only
`capType` still refuses.

★★★ **Three further facts the seeded arm bought, none of which the committed trace could
give.** (1) The reply is thirteen ops at `bSupported = 0x00, attributes = 0x00000000`, **read
out of a `0xCD` buffer** — a positive reading, where the trace's all-zero `out=` decides
nothing (`traces/real_ga106/README.md`). (2) RM writes **five bytes of every eight-byte
entry**: the three padding bytes after `bSupported` come back `0xCD`. (3) `dbdf` is `[IN]`
and untouched, as the header says (*"Used only for the `_CAPTYPE_P2P`"*).

★ And a second, independent source agrees on the value: RM's **own** vGPU-guest arm writes
exactly this — `subdeviceCtrlCmdBusGetPcieSupportedGpuAtomics_VF` loops all thirteen ops to
`NV_FALSE / 0x0` under the comment *"Atomics not supported in VF. See bug 3497203."*
(`ogkm-580: kern_bus_ctrl.c:693-707`). NVIDIA's answer for a virtualized GPU is this port's
answer, arrived at independently.

⊘ **Why this zero is not the `0x20802a08` zero.** That one was decoded out of an *unmeasured*
row and became a buffer size with a hardware DMA writer downstream. This one is measured,
corroborated, and its failure direction is conservative: `bSupported = FALSE` denies a
capability, so the driver takes a fallback path. Nothing here can produce a wrong `TRUE`.

#### ⊘ What is REFUSED, and why the refusal is the measurement rather than a gap

`_CAPTYPE_GPU` and `_CAPTYPE_P2P` are **declared in the header and refused by the hardware**.
So this port refuses every captype but `SYSMEM`, **by name**. ⊘ Answering them "all thirteen
unsupported" would be a *stronger* claim than a real GA106 makes, and the difference is
observable: `NV_OK` where hardware says `0x56`.

⊘ **No chip row.** Whether a GPU atomic completes to coherent sysmem depends on the **root
complex** being a PCIe AtomicOp completer, so this is `PCIE_GEN_INFO`'s species, not
`GPU_GEN`'s. `GpuAtomicOp::none_supported()` takes **no chip argument** — a compile-time
statement of that, in the shape §14.30 established.

#### The hazard check, run first, in the direction that has bitten three times

*"What is keyed on `0x2080182a` being absent?"* — **nothing.** Its only mention in the tree
was `capability.rs:750` (already permitted, `Origin::Nvproxy`), and its flags carry neither
`RMCTRL_FLAGS_CACHEABLE` (`0x400`) nor `_CACHEABLE_BY_INPUT` (`0x20000`), so it is not a
`sticky::BRANCH_A_CACHEABLE` row.

#### `[measured 2026-08-08, boots `atom1431_ff7a0ea` and `gt1431_ff7a0ea`, both stamps `ff7a0eae9…`, shipping config, `probe-arm set: EMPTY`, STOCK module]` THE WALL IS DOWN

```text
nvkvm: control 0x2080182a result 0x00000000 x1     ← served, and GONE from unserviced
```

`cup2`, verbatim:

```text
SMI_RC=0
=== cup2: run ===
FAIL cuInit(0) -> no CUDA-capable device is detected (100)
CUP2_RC=1
```

#### ⊘⊘⊘ AND `cuInit` STILL RETURNS 100 — but this time the ledger POINTS AT THE WRONG CONTROL

`traces/real_ga106/cuinit_trace_guest_gt1431_ff7a0ea.txt`, the tail:

```text
0x20801803  status=0x00000000
0x2080182a  status=0x00000000     ★ §14.30's wall, now SERVED
0x2080012f  status=0x00000056     ← GPU_QUERY_ECC_STATUS, 1464 bytes, IN the ledger
0x20801303  status=0x00000056     ★★ FB_GET_INFO_V2, 7 entries — NOT in the ledger at all
FREE ×3 ; cuInit(0) -> 100
```

★★★★ **The committed real-hardware trace clears one and convicts the other, and it is the
opposite way round from what the ledger suggests.**

- `0x2080012f` `NV2080_CTRL_CMD_GPU_QUERY_ECC_STATUS` (`ogkm-580: ctrl2080gpu.h:1148`) is
  `unserviced fn 76` in our boot — and **a real GA106 answers it `status=0x00000056` too**
  (`cuinit_ioctl_trace_real_ga106.txt:49`). Our refusal is **correct**, libcuda tolerates it,
  and it is not the wall. ★ This is the first time the real trace has let us *clear* a
  refusal rather than demand a serve: an entry in the unserviced ledger is not automatically
  a gap.
- `0x20801303` `NV2080_CTRL_CMD_FB_GET_INFO_V2` (`ctrl2080fb.h:459`) with the seven indices
  `{0x0b, 0x19, 0x1b, 0x18, 0x0d, 0x17, 0x08}` is answered **`NV_OK`** by a real GA106 on the
  **byte-identical request** (`:50`). **That is the wall.**

★★★ **AND IT HAS NO LEDGER ENTRY IN EITHER DIRECTION.** `[measured]`
`grep -c "unserviced fn 76 cmd 0x20801303"` = **0**, and there is no
`control 0x20801303 result …` line either: the command **never reaches the emulated GSP**.
The guest's own kernel refuses it out of its own state — `kfbGetInfo` resolves most indices
locally and forwards only its `default:` arm, the same one-of-N shape as `GPU_GET_INFO_V2`'s
ten-of-eleven and `BUS_GET_INFO_V2`'s five-of-six. Three earlier `0x20801303` calls in the
**same trace** are answered `NV_OK` (indices `0x08`, `0x3b`, `{0x16,0x09,0x10}`) — so the
control is **argument-keyed** and the port has never been asked about it.

⇒ ⊘⊘ **The unserviced ledger — this project's primary rung-picking instrument — is
structurally blind to this wall**, and worse, it offers a plausible decoy one line above it.
`refusal_invisible_in_the_ledger` recorded *"a served-but-refused command never appears in the
unserviced ledger"*; this is one layer further out — **never asked at all**. ★ The only
instrument that found it is the in-guest interposer diffed against the committed
real-hardware trace, which is exactly what that pair is for.

#### The next rung, fully specified

`FB_GET_INFO_V2`'s six unanswered indices, with the real GA106's own reply
(`cuinit_ioctl_trace_real_ga106.txt:50`, re-derived from the raw bytes and **not** from this
paragraph):

| index | real GA106 `data` |
|---|---|
| `0x0b` | `0x000000c0` |
| `0x19` | `0x00000003` |
| `0x1b` | `0x00240000` |
| `0x18` | `0x00000000` |
| `0x0d` | `0x00000011` |
| `0x17` | `0x00000000` |
| `0x08` | `0x0000c000` — ★ already agreed: our guest's first call answers this exactly |

⚠ **Do not transcribe these into a chip row before asking which of them the guest kernel
answers itself.** §14.30's trap has now fired three times (`GPU_GET_INFO_V2` ten-of-eleven,
`BUS_GET_INFO_V2` five-of-six) and this control has the same shape. The first move is
`ogkm-580: kern_fb_ctrl.c`'s switch, not the table — and then the `rmladder` equivalent of
R21/R22 for `FB_GET_INFO_V2`, because two of these words (`0x1b` = `0x00240000`, `0x0b` =
192) look like memory-geometry facts that a **rented board's FB size** would poison exactly
the way `PCIE_GEN_INFO` poisons a chip row.

#### What landed

| piece | where |
|---|---|
| R23 `--atomics-probe` — the 2x2 that separates the seed from the caller, plus the three declared captypes and one undeclared, both hypotheses written before the run | `crates/kayfabe-isolate-host/src/bin/rmladder.rs` |
| `kayfabe_abi::gpuatomics` — the ABI, `GpuAtomicOp::none_supported()` (no chip argument), the request-editing answer, two named refusals, 8 unit tests | `crates/kayfabe-abi/src/gpuatomics.rs` |
| `WantedTable::BusGetPcieSupportedGpuAtomics`; served universe **27 → 28**, attributed | `kayfabe-device/src/inittables.rs` |
| 7 integration tests over a `0xAA`-poisoned request, incl. the padding assertions | `kayfabe-device/tests/bus_get_pcie_supported_gpu_atomics.rs` |
| ★ §14.30's `BusGetInfoV2` had **no reply-plane test at all** — 6 written here so its differential exemption is true rather than quiet | `kayfabe-device/tests/bus_get_info_v2.rs` |
| ⊘ **three inherited RED gates repaired, each attributed** (`[measured]` all three fail at `78bee9e`): the `cap1b` closure set + both non-vacuity counts; the claimed-call pin `87 → 95` where the **whole +8 is §14.29's `0x20800a4c`** and this rung contributes **zero**; and the size-evidence gate, exempted **with the reason it demands** rather than deleted | `cap1b_differential.rs`, `replay_conformance.rs` |
| ★ the "which copy runs" trap found again — on the **payload**: the hook pushed the box's 445-line interposer where the repo's is 666, so §14.29's `NVSWEEP_GPUINFO` was never reachable through it. Repo copy now wins and prints its md5; a missing `gtrace.txt` is now shouted rather than exited-0 over | `scripts/bench/guest_cuinit_trace.sh` |
| the R23 capture and the new guest trace | `traces/real_ga106/rmladder_r23_atomics_real_ga106.txt`, `cuinit_trace_guest_gt1431_ff7a0ea.txt` |

### 14.32 ★★★★ `FB_GET_INFO_V2` SERVED — and the ledger that "proved" it never arrived was **SATURATED** (`[measured 2026-08-08, boots fb1432_20e319b / gt1432_20e319b, rev 20e319b]`)

`[measured 2026-08-08, vast GA106 bench (`vh`, RTX 3060 `10de:2504`, `GPU-d0913685`, host
driver 580.159.04 Open), boots `fb1432_20e319b` and `gt1432_20e319b`, both
`target/release/libkayfabe_qemu_raw.a` and `qemu-build/qemu-system-x86_64` stamped
`kayfabe-rev:20e319bc7a37545f0ba5fabb98eb40122475f962`, shipping config,
`probe-arm set: EMPTY`, STOCK guest module]`. Evidence:
`docs/reference/bench_evidence/run_{fb,gt}1432_20e319b_{qemu,probe}.log` and
`traces/real_ga106/cuinit_trace_guest_gt1432_20e319b.txt`.

#### ⊘⊘ First, the refutation of the framing I was handed — and it is §14.31's, whose author
#### was told to expect exactly this and still could not have found it by reading

§14.31 closed with its sharpest finding stated as a measurement:

> *"★★★ **AND IT HAS NO LEDGER ENTRY IN EITHER DIRECTION.** `[measured 2026-08-08]`
> `grep -c "unserviced fn 76 cmd 0x20801303"` = **0**, and there is no
> `control 0x20801303 result …` line either: the command **never reaches the emulated
> GSP**. The guest's own kernel refuses it out of its own state."*

★ **The grep is right and the conclusion is wrong. Both ledgers were FULL.** `[measured
2026-08-09, re-reading that same boot's `/workspace/bench/run_gt1431_ff7a0ea_qemu.log`]` —
the two summary lines directly above the rows §14.31 read:

```text
nvkvm: commands: 362 decoded, 67 UNSERVICED (…), 32 distinct
nvkvm: controls: 101 answered, 32 distinct cmd/result rows (…)
```

Both `32`s are the **caps** — `UNSERVICED_SAMPLE_MAX` and `SERVED_CONTROL_SLOTS`, each `= 32`
— and `UnservicedLog::note` was `if s.len() < MAX && !s.contains(&entry) { push }`. The list
was saturated; every command first seen after the thirty-second was dropped with no line
anywhere. `0x2080012f` is the thirty-second and last row printed, and `0x20801303` is asked
*after* it.

⇒ ★★ **An absence from a saturated list is not evidence of absence.** Third species of the
same defect, after `pgrep_comm_truncation_trap` (a check that cannot fail) and
`gate_read_through_grep_cannot_fail` (a verdict a grep cannot deliver): here an **instrument
that silently stops recording** at a bound.

★★★ And the reason it survived is worth more than the bug. The `UNSERVICED_SLOTS` doc
asserted, in as many words:

> *"`unserviced_len` reports the truth even when it exceeds this, so a full array is never
> mistaken for a complete list."*

`unserviced_len` was `sample().len()` — clamped by that very cap and **structurally unable to
exceed it**. `safety_comment_is_not_the_check`, on the project's primary rung-picking
instrument: the prose stated the exact property whose absence caused the error, and stating
it is what stopped anyone checking. ⊘ `ControlCensusLog` had kept a separate `served_distinct`
counter all along, so the served list's count *was* truthful — one module got it right and
the sentence in the other one covered for it.

⚠ ★ **The cap was documented.** `unserviced.rs` says *"The distinct set is capped"* in its own
header. A documented bound is not a bound anybody checks; only a **printed** one is.

#### ⚠ And §14.31's reply table mis-transcribed a word, one byte-boundary off — for the second time in three rungs

It published *"`0x08` | `0x0000c000` — ★ already agreed: our guest's first call answers this
exactly"*. `[measured]` the raw bytes at `traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:50`
are `08000000 0000c000`, i.e. index `0x08`, data **`0x00c00000`** = 12 582 912 KiB = **12
GiB**, which is the RTX 3060 12 GB's whole framebuffer. `0x0000c000` would be 48 MiB.

⇒ §14.30's `0x03003020`-for-`0x00302000` was the first; this is the second, and **both decode
plausibly**. ★ It cost nothing only because `0x08` turns out to be answered by the guest's own
kernel and never forwarded — luck, not process. **Re-derive from the hex, never from the
paragraph**, including when the paragraph says it was re-derived from the hex.

#### ★★★ Three of the seven indices are the guest kernel's own, and the RPC is NOT `BUS_GET_INFO_V2`'s shape

`_kmemsysGetFbInfos` (`ogkm-580: src/nvidia/src/kernel/gpu/mem_sys/kern_mem_sys_ctrl.c:137-996`)
answers what it can locally and tracks the rest in `fbInfoListIndicesUnset`. Every index with
a `case` arm in its second `switch` is kernel-answered; the `default:` arm is a bare
`continue`. For libcuda's failing seven-index request:

| index | name | answered by |
|---|---|---|
| `0x08` | `TOTAL_RAM_SIZE` | **guest kernel** (`:335`) |
| `0x17` | `RAM_LOCATION` | **guest kernel** (`:711`) |
| `0x18` | `FB_IS_BROKEN` | **guest kernel** (`:716`) |
| `0x0b` | `BUS_WIDTH` | ★ forwarded |
| `0x19` | `FBP_COUNT` | ★ forwarded |
| `0x1b` | `L2CACHE_SIZE` | ★ forwarded |
| `0x0d` | `RAM_TYPE` | ★ forwarded |

⚠ **And the forward is ONE COMPACTED RPC, not one per index.** `kbusSendBusInfo_IMPL` sends a
fresh one-entry struct per forwarded index; `_kmemsysGetFbInfos` allocates **one** fresh
`NV2080_CTRL_FB_GET_INFO_V2_PARAMS`, copies the unset indices into it **compacted from slot
zero**, and sends a single `NV_RM_RPC_CONTROL` of `sizeof(*pRpcParams)` (`:952-990`). So the
request this port answers is a **four**-entry struct, never the guest's seven-entry ioctl
buffer. ★ The property that matters is unchanged: **arriving here is the marker**, so every
declared entry is filled and an index with no derivation refuses the whole call.

#### ★★★ What is served — and it is the first rung that states NO NEW NUMBER

All four are **projections of `ChipProfile::memory_system`**, the row already served to
`NV2080_CTRL_CMD_INTERNAL_MEMSYS_GET_STATIC_CONFIG` (`0x20800a1c`):

| index | served | from | real GA106 |
|---|---|---|---|
| `0x1b` `L2CACHE_SIZE` | `0x0024_0000` | `memory_system.l2_cache_size`, verbatim | `0x00240000` ✓ |
| `0x0d` `RAM_TYPE` | `0x11` `GDDR6` | `memory_system.ram_type`, verbatim | `0x00000011` ✓ |
| `0x0b` `BUS_WIDTH` | `192` | `ltc_count × 32` (one 32-bit FBPA per LTC) | `0x000000c0` ✓ |
| `0x19` `FBP_COUNT` | `3` | `ltc_count ÷ 2` (two FBPAs per FBP on GA10x) | `0x00000003` ✓ |

★★ **The projection is the design.** `l2CacheSize` and `ramType` are *the same two silicon
facts* under two control ids; a second table of measured words is precisely what would let
this device tell RM its L2 is 2.25 MiB under one id and something else under the next.
`kayfabe-device/tests/fb_get_info_v2.rs` drives **both** controls through one policy and
compares the bytes, so the agreement is executed rather than asserted. The two relations hold
for every Ampere `ltcCount` RM's own PLC arms enumerate (`kmemsysIsPagePLCable_GA102`:
`{48, 40, 4×8, 3×8}` ⇒ `ltcCount ∈ {12, 10, 8}` ⇒ 384/320/256-bit) and are named `GA10X_*` so
the Hopper seam is a named line rather than a retrofit.

#### ⊘⊘ The OBVIOUS next step is contradicted by hardware — `LTS_COUNT` is not `ltc × ltsPerLtc`

`[measured 2026-08-08, real GA106 `GPU-d0913685`, `traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:66`]`:

The very next `FB_GET_INFO_V2` in the same real trace (`:66`) asks `{0x1a, 0x22, 0x23}` and is
answered `{0x07, 6, 18}`. `0x22` `LTC_COUNT` is `memory_system.ltc_count` exactly. `0x23`
`LTS_COUNT` is **not** the product: `6 × 4 = 24`, and hardware says **18**.

Both readings are real hardware and neither is wrong. `ltsPerLtcCount = 4` is a captured GSP
reply (`C: src/qemu/mode2_initctrl_ga106.h:5391`, `dlen 40`, a row **with** a body);
`FB_INFO_INDEX_LTS_COUNT` is documented as *"the **active** LTS count across all active LTCs"*
(`ogkm-580: ctrl2080fb.h:251-254`), and `18 × 128 KiB = 2304 KiB` is this part's L2 — the same
slice size GA102's `== 48` arm implies.

★★★ ⚠ **And `ga10x.rs`'s own comment on that field is arithmetic that self-justifies the wrong
reading:** *"2.25 MiB = 24 slices x 96 KiB … The capture agrees with itself."* 24 × 96 KiB is
2304 KiB, so it **checks out** — and 96 KiB is not an Ampere L2 slice.
`two_encodings_agreeing_on_the_first_values`, in a doc comment. ⊘ The **field is correct and
was not touched**; only the justification is, and `0x1a`/`0x22`/`0x23` are refused by name
with the contradiction pinned in a test so nobody "simplifies" `0x23` into the product later.

#### `[measured 2026-08-08, boots `fb1432_20e319b` and `gt1432_20e319b`, both artifacts stamped `kayfabe-rev:20e319bc7a37545f0ba5fabb98eb40122475f962`, shipping config, `probe-arm set: EMPTY`, STOCK module]` THE WALL IS DOWN

```text
nvkvm: control 0x20801303 result 0x00000000 x1     ← served
```

`cup2`, verbatim:

```text
SMI_RC=0
=== cup2: run ===
FAIL cuInit(0) -> no CUDA-capable device is detected (100)
CUP2_RC=1
```

★★ **And the instrument repair paid for itself in the same boot, measurably.** Unserviced went
`32 distinct` → **34**, and the two rows the saturated list could not have shown either way
are `0x20802a12` and `0x20802a0b`. Served rows went `32` → **33**, with
`control 0x20801303 result 0x00000000` among them — ⊘ at the old `SERVED_CONTROL_SLOTS = 32`
that row would have been **counted and not printed**, and this rung's own success would have
been unobservable in the ledger.

#### ★★★★ `cuInit` GOT NINE CONTROLS FURTHER, and the traces now agree LINE FOR LINE to the divergence

`traces/real_ga106/cuinit_trace_guest_gt1432_20e319b.txt` — 52 lines → **67**:

```text
0x20801303  status=0x00000000     ★ §14.31's wall, now SERVED (all four calls)
0x20800170  status=0x00000000
0x20800119  status=0x00000000
0x0000027b  status=0x00000000
ESC 0xc9 ; ESC 0xce
0x20801201  status=0x00000000
0x2080122a  status=0x00000000
0x2080122b  status=0x00000000  ×3
0x20801227  status=0x00000000
0x20802a0a  status=0x00000056     ★★ THE NEW WALL
FREE ×3 ; cuInit(0) -> 100
```

★★★ **Our trace and the real GA106's are now identical from line 50 to line 61** — twelve
consecutive calls, same ids, same order, all `NV_OK` — and diverge at **line 62**:
`0x20802a0a` `NV2080_CTRL_CMD_CE_GET_ALL_CAPS` (`ogkm-580: ctrl2080ce.h:325`, 136 bytes),
which a real GA106 answers `NV_OK` and this port answers `0x56`. ⊘ Unlike `0x2080012f`, the
real trace **convicts** rather than clears it.

#### The next rung, fully specified — and the ID TO SERVE IS NOT THE ID THAT FAILS

★★★ `0x20802a0a` is **not** in this boot's unserviced ledger either — and this time that
silence is *trustworthy*, because the ledger is no longer saturated (34 of 64, no truncation
line). `subdeviceCtrlCmdCeGetAllCaps_IMPL` (`ogkm-580: kernel_ce_shared.c:283-336`) explains
it exactly:

1. `portMemSet(pCeCapsParams, 0, sizeof(*pCeCapsParams))`;
2. `NV_ASSERT_OK_OR_RETURN(pRmApi->Control(…, NV2080_CTRL_CMD_CE_GET_ALL_PHYSICAL_CAPS, …))`
   on the **physical** RMAPI — i.e. **`0x20802a0b`**, straight to our GSP;
3. then it **overwrites** `present` and `capsTbl[kceInst]` locally for every non-stubbed
   `KernelCE` via `kceAssignCeCaps_HAL`.

⇒ ★★ **Serve `0x20802a0b`, not `0x20802a0a`.** And `0x20802a0b` is one of the two rows the cap
raise made visible — the repaired instrument produced the next rung's target directly, out of
the same boot, having spent two rungs hiding it.

⇒ ★ A refinement of §14.31's rule rather than a reversal: *"do not pick your rung from the
ledger"* becomes **the interposer names the failing IOCTL and the ledger names the RPC to
serve, and they are different ids.** You need both instruments, and the ledger only after it
was made able to tell you the truth.

The real GA106's own reply (`cuinit_ioctl_trace_real_ga106.txt:62`, re-derived from the raw
`out=` bytes and **not** from this paragraph — `NV2080_CTRL_CE_GET_ALL_CAPS_PARAMS` is
`NvU8 capsTbl[64][2]` then a 8-aligned `NvU64 present`, 136 bytes):

| field | real GA106 |
|---|---|
| `capsTbl[0]` | `e3 03` |
| `capsTbl[1]` | `e3 03` |
| `capsTbl[2]` | `e2 03` |
| `capsTbl[3]` | `e2 03` |
| `capsTbl[4..63]` | all zero |
| `present` @128 | `0x0f` — CE0..CE3 |

⚠ **This is a body with real content, not another `dlen = 0` row** — the `0x20802a08` shape is
absent here, and `present = 0x0f` agrees with `GA106_ENGINES`' four copy engines independently.
⊘ But the request is `[OUT]`-only and libcuda hands RM a zeroed buffer, so **do not read the
zero tail as measured**: `rmladder --probe-ctrl 0x20802a0b:136` is sound on this struct
(§14.31's `[IN]`-field trap does **not** apply — there is no `[IN]` field) and is the cheap
instrument that turns the `capsTbl[4..63]` zeros from ambiguous into positive. ★ Decode the
caps bits against `NV2080_CTRL_CE_CAPS_*` before writing any of them down; `0x03e3` vs `0x03e2`
differs in exactly one bit between CE0/1 and CE2/3, and which bit that is decides whether this
is a per-CE fact or a copy-paste.

#### What landed

| piece | where |
|---|---|
| `kayfabe_abi::fbinfo` — the ABI, `FbGeometry`'s four projections, seven named refusals, 12 unit tests incl. the `LTS_COUNT ≠ product` pin | `crates/kayfabe-abi/src/fbinfo.rs` |
| `WantedTable::FbGetInfoV2`; served universe **28 → 29**, attributed | `kayfabe-device/src/inittables.rs` |
| 7 reply-plane tests, one of which drives `0x20801303` **and** `0x20800a1c` through one policy and compares the bytes | `kayfabe-device/tests/fb_get_info_v2.rs` |
| ★★★ **the saturated-ledger repair**: `UnservicedLog::distinct()` / `::truncated()` counting before the capacity test, `unserviced_len` finally truthful, both caps 32 → 64, an explicit `TRUNCATED … absence here is NOT evidence of absence` line in the C printer, ABI **22 → 23**, and a hand-written `Default` because `[T; N]` only derives it to `N == 32` | `unserviced.rs`, `census.rs`, `plane.rs`, `shim.rs`, `kayfabe_shim.h`, `nvkvm.c` |
| the test the old bounded-set test could not be: a saturated sample must *say so* | `kayfabe-device/tests/unserviced_ledger.rs` |
| two gate exemptions extended **with their reasons** rather than deleted — and `0x20801303`'s size is the best-evidenced of the three (`size=1028` on five real-GA106 ioctls and four of ours) | `cap1b_differential.rs`, `replay_conformance.rs` |
| the new guest trace and both boots' device reports | `traces/real_ga106/cuinit_trace_guest_gt1432_20e319b.txt`, `docs/reference/bench_evidence/run_{fb,gt}1432_20e319b_{qemu,probe}.log` |

#### ⊘⊘ The gate ledger for this rung, measured at the PARENT revision rather than assumed

★ `scripts/ci_gates.sh --all` is **red at `82e5354`**, the revision this rung started from —
`[measured 2026-08-09]` in a clean worktree of that exact commit, three steps fail:

| gate | at `82e5354` | at this rung | whose |
|---|---|---|---|
| Hexagonal boundary | **FAILED**, 39 hits | ★ **GREEN** | see below — it was never anybody's |
| Bridge-exclusivity | **FAILED** | FAILED, same lines | inherited; `staticinfo.rs:189`, `lib.rs:1075` and the `use kayfabe_gsp::{…}` imports all pre-date this rung (`git blame`: `61fb1f4`, `02aa11e`) |
| Claim ledger | unattributed **403**/381, conflated **68**/66, bare-HW **18**/17 | 406 / 68 / 18 | ⊘ this rung's own delta is **0 on all three**; the `+3` is `docs/PRODUCT_POSITIONING.md` (+1) and `docs/design/gpu_compartmentalisation.md` (+2), which are the owner's concurrent doc commit |

⊘ Recorded rather than ratcheted, and rather than absorbed: no bar is raised here.

#### ★★★★ And the boundary gate had been red on a SUBSTRING — `libc` is inside `libcuda`

`[measured 2026-08-09]` the gate's pattern is
`eventfd|epoll|timerfd|rawfd|libc|O_NONBLOCK`, case-insensitive, over fourteen pure crates.
It returns **39 hits, and filtering out the word `libcuda` leaves ZERO**. With `libc` the
count is **0**. There was never a real breach among them — the gate has been failing on the
name of a userspace library this port necessarily talks *about*, since libcuda entered the
vocabulary at §14.27.

⊘⊘ **A permanently-red gate is not a strict gate, it is an absent one.** Nobody could have
distinguished a genuine `libc::` from thirty-nine `libcuda`s, and §14.29, §14.30 and §14.31
each ran this suite and shipped past it — `skipped_oracle_kills_the_guard` reached from the
other side: there, a guard covered no compiled oracle; here, a guard's output was pure noise.
★ The fix is a **correction, not the weakening the gate's own instruction forbids**, and it
was bite-checked rather than argued: `libc` still fires on a planted
`// bite probe: libc::c_int` in `kayfabe-util` (`:` is a word boundary) and does not fire on
`libcuda`.

#### ★ `run_full_suite.sh` on `vh`, and the two things it found that are NOT this rung's

`[measured 2026-08-09, `vh` (38 cores, 198 GB), rev `4aad26b`, `/workspace/bench/suite_4aad26b.log`]`
`RAN 8 / FAILED 3 / SKIPPED 6`, with the **compiled oracles all live** —
`GMMU RAN=15, PUSHBUFFER RAN=24, TOKEN RAN=3, USERD-CHID RAN=5, VBIOS RAN=13, SKIPPED=0` in
every family. ★ So the local-only rule these rungs were run under is obsolete; `vh` is the
better host for the suite as well as the bench.

- **`fuzz-build`** failed in 5 s with `can't find crate for std … x86_64-unknown-linux-musl`.
  ⊘ Not a code failure: the box's **nightly** toolchain had no musl std, while stable did, so
  the main workspace's isolate built and the fuzz workspace's did not.
  `rustup target add x86_64-unknown-linux-musl --toolchain nightly` and the step builds
  clean. ⚠ A suite step that fails on a missing toolchain component **guards nothing while it
  is red**, which is `skipped_oracle_kills_the_guard` in its third costume this session.
- **`test-hardware`**: `stress_multi_vcpu_interleaved_ops` panicked with
  `doorbell routes: NotScheduled { chan: ChanId(0), vchid: VChid(258) }`
  (`tests/tests/concurrency_stress.rs:421`) followed by seven `PoisonError`s — the poison is
  the consequence, not the fault. ⊘ **Not this rung's, and that is checked rather than
  asserted**: `NotScheduled` is `kayfabe-core`/`kayfabe-fwd`'s and the failing path is
  `handle_doorbell` over a `MockArch`; this rung touched `kayfabe-abi`, `kayfabe-device`,
  `kayfabe-qemu-raw`, `qemu/` and docs, none of which that path reads.
  `[measured]` **8/8 green** re-running it alone (5×) and the whole binary (3×, ~4.6 s each);
  it failed once, at 13.4 s, **under full-suite contention**. That is
  `flake_rate_depends_on_core_count` pointed the other way — a big box does not only hide
  races, a *loaded* one exposes them — and it is a real handoff, not a dismissal.

### 14.33 ★★★★ `CE_GET_ALL_PHYSICAL_CAPS` SERVED — and §14.32's probe plan could never have run, on a control §14.32 read at the wrong boundary

`[measured 2026-08-09, vast GA106 bench (`vh`, RTX 3060 `10de:2504`, `GPU-d0913685`, host
driver 580.159.04 Open), boots `ce1433_0de5ddb` and `gt1433_0de5ddb`, both
`target/release/libkayfabe_qemu_raw.a` and `qemu-build/qemu-system-x86_64` stamped
`kayfabe-rev:0de5ddb70a1509ac00ab66e4b833043f604ae524`, shipping config, STOCK guest module]`.

#### ⊘⊘ First, the refutation of the framing I was handed — and it is TWO refutations of one paragraph

§14.32 closed by specifying this rung completely: the id to serve, the reply bytes, and the
instrument. It got the **id** right, and then got both of the other two wrong in ways that
would each have cost a rung.

##### 1. ★★★ The probe it specified is sound on the struct and CANNOT RUN

> *"`rmladder --probe-ctrl 0x20802a0b:136` is sound on this struct (§14.31's `[IN]`-field
> trap does **not** apply — there is no `[IN]` field) and is the cheap instrument that turns
> the `capsTbl[4..63]` zeros from ambiguous into positive."*

Every clause is true, and §14.31's trap really is disarmed. `[measured 2026-08-09, real GA106
`GPU-d0913685`, `traces/real_ga106/rmladder_r18_cecaps_real_ga106.txt`]`:

```text
info  R18 0x20802a0b    = refused Other(86) (no value measured)
★     R18 0x20802a0a    = NV_OK, 136 bytes: e303e303e203e203 00…00 0f00000000000000
```

`0x20802a0b`'s export flags are `0x101d0` (`ogkm-580: g_subdevice_nvoc.c:7705-7718`) —
`GPU_LOCK_DEVICE_ONLY(0x10) | ROUTE_TO_PHYSICAL(0x40) | INTERNAL(0x80) |
API_LOCK_READONLY(0x100) | GSP_PLUGIN_FOR_VGPU_GSP(0x10000)` — and they carry **neither**
`PRIVILEGED(0x4)` **nor** `NON_PRIVILEGED(0x8)`, which is `RMCTRL_FLAGS_KERNEL_PRIVILEGED`
(`ogkm-580: control.h:170-247`): refused to every usermode client including root.

⊘ **That precondition was already written down**, in `probe_ctrl`'s own doc comment, by the
rung that first hit it on `0x20802a08`. Nobody had to discover it.

⇒ ★★ **Checking that an instrument is sound on the DATA is not checking that it can reach
the SUBJECT.** Two preconditions; clearing the one that bit you last says nothing about the
other. For an RM control the second is one grep of the export row's flags.

⊘⊘ And the reachable sibling cannot answer it either. `0x20802a0a` carries `NON_PRIVILEGED`
and does answer `NV_OK` — but `subdeviceCtrlCmdCeGetAllCaps_IMPL` opens with
`portMemSet(pCeCapsParams, 0, sizeof(*pCeCapsParams))`
(`ogkm-580: src/nvidia/src/kernel/gpu/ce/kernel_ce_shared.c:312`) **before** it forwards. The
`0xCD` seed — the entire mechanism that separates *"wrote zeros"* from *"wrote nothing"* — is
destroyed by the callee, and `[measured]` not one `0xCD` byte came back. ★ The seed
instrument is **blind by construction on this control**, and `capsTbl[4..63]`'s zeros are
still unmeasured at the physical boundary. No usermode instrument can measure them.

##### 2. ★★★ The reply table it published is the reply of the control it correctly said NOT to serve

§14.32's table is headed *"The real GA106's own reply (`cuinit_ioctl_trace_real_ga106.txt:62`)"*
and line 62 is **`0x20802a0a`** — the guest kernel's composed answer, one boundary above the
id being served. That is the very trap the same section named in bold two paragraphs earlier
(*"a table read at one boundary does not describe the boundary below"*), applied to the id
and then not applied to the bytes.

★ The bytes turn out to be right, and having **proved** that is worth more than having
inherited it. `subdeviceCtrlCmdCeGetAllCaps_IMPL` (`kernel_ce_shared.c:282-336`) post-processes
the physical reply with exactly two operations and **both are monotone ORs**:

- `pCeCapsParams->present |= BIT64(kceInst)` for each non-stubbed `KernelCE` (`:329`);
- `kceAssignCeCaps_HAL(…, capsTbl[kceInst])` (`:331`), which for every Turing/Ampere/Ada part
  resolves to `kceAssignCeCaps_GP100` (`g_kernel_ce_nvoc.c:413-427`) → a bare
  `if (pKernelNvlink != NULL) kceGetNvlinkCaps(…)` (`kernel_ce_gp100.c:311-323`) → at most
  three `RMCTRL_SET_CAP`s, and `RMCTRL_SET_CAP` is `|=` (`control.h:99`).

There is **no `portMemSet` after the RPC and no `RMCTRL_CLEAR_CAP` on this path**, so the
physical reply survives in full. And the three bits the kernel *could* add — `_CE_SYSMEM_READ`,
`_CE_SYSMEM_WRITE`, `_CE_NVLINK_P2P` — are **exactly the three that measure clear** in both
observed entries, because `GPU_GET_KERNEL_NVLINK` is `NULL` on a GeForce GA106. ⇒ The kernel
added nothing; the caller-visible bytes **are** the physical reply. And the construction is
idempotent even if that argument were wrong: the guest runs the same kernel code, so serving
`V` = the observed value yields `V | X = V`.

#### ★★★ What is served, and every bit's source

`NV2080_CTRL_CE_GET_ALL_CAPS_PARAMS` (`ogkm-580: ctrl2080ce.h:331-334`) is
`NvU8 capsTbl[64][2]` then an 8-aligned `NvU64 present` — 136 bytes, `present` at 128, no
padding. `0x20802a0b` shares the type by `typedef` (`:340`).

| field | served | from |
|---|---|---|
| `present` | `0x0f` | the `DEV_TYPE_ENUM_LCE` rows of `ChipProfile::engines` — **the same slice** `FIFO_GET_DEVICE_INFO_TABLE` and `INTERNAL_DEVICE_INFO` already serve |
| `capsTbl[0..1]` | `0x03e3` | `GA10X_LCE_BASE_CAPS \| GRCE` |
| `capsTbl[2..3]` | `0x03e2` | `GA10X_LCE_BASE_CAPS` |
| `capsTbl[4..63]` | `0x0000` | absent from `present`; the header says an absent CE's caps *"should be ignored"*, and a table whose ignored rows still claim `SYSMEM \| P2P` lies to anything that stops honouring the qualifier |

The single per-CE bit is `_CE_GRCE` (`0:0x01`, *"Set if the CE is synchronous with GR"*).
★ Principled, not copy-paste: `NV_CE_GRCE_ALLOWED_LCE_MASK = 0x03` (`kernel_ce_ga102.c:34`,
returned by `kceGetGrceSupportedLceMask_GA102` at `:188-196` for GA102/103/104/**106**/107)
names exactly LCE0 and LCE1, backed by `NV_CE_GRCE_CONFIG__SIZE_1 = 2` (`dev_ce.h:32`) and
`NV_CE_MAX_GRCE = 2`. ⊘ Open source gives the *allowed* mask; the measurement gives that this
part realises it. Both are recorded.

#### ⊘⊘ FOURTH sighting of `a_table_does_not_decide_behaviour` — and this one would have shipped

The same HAL file declares `NV_CE_MAX_LCE_MASK = 0x1F` (`kernel_ce_ga102.c:37`) — five GA10x
LCEs, `{0,1}` GRCE, `{2,3}` sysmem, `{4}` even-async — and reading it as the exposed set
predicts `present = 0x1f` with a fifth caps entry. `[measured]` a real GA106 answers
**`present = 0x0f`** and `capsTbl[4] = {0x00, 0x00}`, from **two independent callers**.

⇒ The mask is the **permitted universe**; what the part exposes is the dispatch's, and they
differ by one engine. Kept as `GA10X_EXPOSED_LCE_MASK_IS_NOT_A_SOURCE` with the contradiction
pinned in two tests, so nobody "simplifies" the projection into the constant later.

#### ⚠ One projection considered and REFUSED, recorded so it is not re-invented

`_CE_CC_SECURE` is *"Set if the CE is capable of encryption/decryption"* (`ctrl2080ce.h:137-138`)
— a property of the **silicon**, not of whether Confidential Computing is switched on.
Deriving it from `ChipProfile::conf_compute` (which this port serves as both-bits-clear) would
agree on GA106 **by coincidence** and be wrong on any CC-capable part with CC disabled. It is
an arch fact and stays one.

#### ★ THIRD hand-regrouped-hex defect in four rungs — and the first a TEST caught

The byte-for-byte pin started life as a 272-character hex literal in `kayfabe_abi::cecaps`.
It was **sixteen bytes short**, and the length assertion caught it before a human read it.
§14.30's `0x03003020` and §14.32's `0x0000c000` were each caught by a *reader*, one rung late.

⇒ The literal is gone. The abi-side expectation is built structurally, and the authority is
`kayfabe-device/tests/ce_get_all_physical_caps.rs`, which parses the reply out of the
committed artifacts themselves. ★ That test then found two more things nobody would have
predicted: the two traces are in **different formats**, and **the provenance header I wrote
into my own trace file is a decoy** — it names the id and the status and contains no hex, so
a `contains(id) && contains("NV_OK")` filter matched it. A trace annotated for a human reader
is a trace with decoys in it.

#### What landed

| piece | where |
|---|---|
| `kayfabe_abi::cecaps` — the ABI, all twelve named cap bits as `(byte, mask)` pairs, `CeGeometry::from_engines`, two refusals, 12 unit tests | `crates/kayfabe-abi/src/cecaps.rs` |
| `WantedTable::CeGetAllPhysicalCaps`; served universe **29 → 30**, attributed; the first arm whose reply is **constructed** rather than the request edited | `kayfabe-device/src/inittables.rs` |
| 9 reply-plane tests, incl. the two-independent-captures agreement, the no-surviving-seed pin, and the `present`-is-the-engine-slice projection | `kayfabe-device/tests/ce_get_all_physical_caps.rs` |
| the R18 CE-caps probe: `0x20802a0b` refused, `0x20802a0a` served, both hypotheses written before the run | `traces/real_ga106/rmladder_r18_cecaps_real_ga106.txt` |
| the two closure sets, each entry with its reason; the `cuInit`-driven-capture flag promoted from a paragraph to a queue item at **five rungs overdue** | `cap1b_differential.rs`, `replay_conformance.rs` |

#### `[measured 2026-08-09, boot `gt1433_0de5ddb`, artifact stamped `kayfabe-rev:0de5ddb70a1509ac00ab66e4b833043f604ae524`, STOCK module]` THE WALL IS DOWN

```text
nvkvm: control 0x20802a0b result 0x00000000 x1     ← served
```

and in the guest's own interposed trace, the control that was `0x56`:

```text
CTRL cmd=0x20802a0a size=136 status=0x00000000 out=e303e303e203e203 00…00 0f00000000000000
```

⊘ **Byte-identical to the real GA106's**, which is the assertion `ce_get_all_physical_caps.rs`
already makes — now confirmed through a whole guest kernel rather than through a policy call.

★★ And note which ledger it came out of. The plain boot `ce1433_0de5ddb` is `SMI_RC=0` with
**191 commands decoded** and no `0x20802a0b` anywhere: `nvidia-smi`'s `RmInitAdapter` never
asks for it. `gt1433_0de5ddb` decodes **368**. ⇒ The cap1b exemption this rung adds is not an
excuse, it is the same fact measured from the other side — a capture driven by `nvidia-smi`
**cannot** contain this control.

#### ★★★★ `cuInit` went TWO CONTROLS FURTHER, and the agreement with hardware nearly DOUBLED

`traces/real_ga106/cuinit_trace_guest_gt1433_0de5ddb.txt`:

```text
0x20801227  status=0x00000000
0x20802a0a  status=0x00000000     ★ §14.32's wall, now NV_OK and byte-identical
0x2080121b  status=0x00000000     ← new, 9240 bytes
0x20803801  status=0x00000056     ★★ THE NEW WALL
ESC 0xcf ; FREE ×3 ; cuInit(0) -> 100
```

★★★ Compared id-by-id and status-by-status against `cuinit_ioctl_trace_real_ga106.txt`, the
two traces are now **identical for the first 62 calls with exactly one exception**, and they
diverge at **call 63**. §14.32's agreement ran from 50 to 61 — twelve consecutive calls; it
now runs from 1 to 62 with one hole, and the hole is **`0x20810108`** (call 39), the
`NV2081_BINAPI` control §14.26 measured as having **no oracle in any instrument**.

⊘⊘ **That is a result in itself, and it is a good one:** `0x20810108` is answered `NV_OK` by
real hardware and `0x56` by this port, and `cuInit` carries on for **twenty-three more calls**
regardless. Its refusal is therefore **not fatal**, which no amount of reasoning about an
uncapturable control could have established — only running past it could.

Call 63 is `0x20803801` `NV2080_CTRL_CMD_GRMGR_GET_GR_FS_INFO` (`ogkm-580: ctrl2080grmgr.h:57`),
1928 bytes, which a real GA106 answers `NV_OK` and this port answers `0x56`. ⊘ Like
`0x20802a0a` and unlike `0x2080012f`, the real trace **convicts** rather than clears it.

#### The next rung — and this time the reachability question is asked FIRST

★ `0x20803801`'s export row is the first thing to read, not the last: this rung's whole
opening lesson is that a probe plan written without it costs a cycle. Ask, in this order:
(1) does the id that fails route to a *different* id at the physical boundary, as
`0x20802a0a` → `0x20802a0b` did; (2) do the failing control's flags permit a usermode probe
at all; (3) only then, what does the reply have to contain.

⚠ And `GET_GR_FS_INFO` is a **query-list** control like `FB_GET_INFO_V2`, not a flat struct —
`NV2080_CTRL_GRMGR_GET_GR_FS_INFO_PARAMS` carries a `numQueries` and an array of tagged
queries, so the `[IN]`-field trap §14.31 was bitten by **does** apply here, unlike on
`0x20802a0b`. A `0xCD`-seeded probe of it would ask an undeclared query type. ⊘ Read the
declared query tags before seeding anything.

### 14.34 ★★★★ `GR_FS_INFO` + the three FB indices §14.32 deferred — and the deferred one's answer was inside the sentence that deferred it

`[measured 2026-08-09, vast GA106 bench (`vh`, RTX 3060 `10de:2504`, `GPU-d0913685`, host
driver 580.159.04 Open), rev `373c145`, shipping config, STOCK guest module]`.

#### ⊘⊘ First, the refutation of my own framing — and it is §14.32's sentence, read properly

§14.32 refused `0x1a`/`0x22`/`0x23` **by name** on a stated ground:

> *"`0x23` has exactly one supporting reading and a plausible-looking derivation that
> contradicts it, which is the configuration that has produced this project's silent wrong
> answers. It is the next rung, with the evidence written down."*

★★★ The evidence written down **contains the answer**, four lines above, in the same module
header: *"`18 × 128 KiB = 2304 KiB`, which is `l2_cache_size`"*. That is not a coincidence
worth noting. It is a **derivation**:

```text
LTS_COUNT = l2_cache_size / GA10X_L2_SLICE_BYTES = 0x0024_0000 / 131 072 = 18
```

⇒ `0x23` is a projection of the row this port **already serves** to `0x1b` and to
`0x20800a1c`, so the FB half of this rung states **no new number at all** — the same shape
§14.32 was proud of for its own four indices, available for its next three the whole time.

⊘ And the alternative was worse than "one new number": a literal `18` in the GA106 chip row
is precisely the **per-chip table** the owner's `derive_what_you_cannot_query_then_oracle_it`
directive forbids. The derivation cannot drift from what `0x1b` answers; a literal could.

★ `a_flag_is_not_progress`, in its sharpest form yet. The flag was **well written** — it
named the index, the contradiction, the trace line and the file — and that is exactly why it
survived two rungs unread. A repeat flag is evidence the answer is nearby; this one had the
answer *inside it*.

⚠ **What the derivation rests on, stated rather than assumed.** `GA10X_L2_SLICE_BYTES` has
**two** supporting points and **no source line anywhere**: this part (`18 × 128 KiB =
2304 KiB = l2_cache_size`, both sides measured on real hardware) and GA102
(`kmemsysIsPagePLCable_GA102`'s `== 48` arm, on a part whose L2 is 6 MiB). A Hopper profile
must re-establish it rather than inherit it, which is what the `GA10X_` in the name is for,
and an L2 that is not a whole number of slices **refuses rather than rounding**.

⚠ And `0x1a` `FBP_MASK` carries an assumption this part **cannot settle**:
`(1 << fbp_count) − 1` is right only for FBPs contiguous from zero, and GA106's `0x07` is
consistent with that *and* with a captured literal. Named, not resolved.

#### ★★★ `0x20803801` — the first control whose errors are PER-ITEM, not per-call

`ogkm-580: ctrl2080grmgr.h:42-50`, in as many words:

> *"If there is any error in `NV2080_CTRL_GRMGR_GET_GR_FS_INFO_PARAMS`, we will immediately
> fail the call. However, if there is an error in the query-specific calls, we will **log the
> error and march on**."*

⊘ [`fbinfo`]'s rule — *"one refused index fails all of them"*, correct there and stated in
bold — is **exactly wrong here**, and carrying it across would have refused a call a real
GA106 answers `NV_OK`.

⚠⚠ **And the inverse is the trap.** A per-query `NV_ERR_NOT_SUPPORTED` rides inside an
`NV_OK` reply, so it reaches **neither ledger this port keeps**: the command was served, and
the served list's result column says `0`. A query type we merely had not modelled would
become a **silent wrong answer** — `refusal_invisible_in_the_ledger` with a new carrier, and
the ledger is still the primary rung-picking instrument.

⇒ `kayfabe_abi::grfsinfo` has **three** outcomes, not two:

| outcome | when | why |
|---|---|---|
| answer | `GPC_COUNT`, `CHIPLET_GPC_MAP`, `CHIPLET_SYSPIPE_MASK`, `CHIPLET_GRAPHICS_SYSPIPE_MASK` | derivable from rows already served |
| **per-query** `0x56` | types 5, 7, 8, 9, 12 | ★ RM itself refuses these on a non-MIG part — each carries its own *"does not support … legacy case"* contract, and type 5 is refused on **every** part |
| **whole-call** refusal | `TPC_MASK`, `PPC_MASK`, `ROP_MASK`, unknown | ⊘ real hardware answers these and we do not; a per-query refusal would be invisible, a call refusal costs one boot and cannot be missed |

#### ⊘ NO id translation — and the reachability question was asked FIRST this time

§14.33's whole opening lesson. `0x20803801`'s flags are `0x10248`
(`ogkm-580: g_subdevice_nvoc.c:9520-9534`) = `NON_PRIVILEGED(0x8) | ROUTE_TO_PHYSICAL(0x40) |
ROUTE_TO_VGPU_HOST(0x200) | GSP_PLUGIN_FOR_VGPU_GSP(0x10000)`. `ROUTE_TO_PHYSICAL` makes
`NVOC_EXPORTED_METHOD_DISABLED_BY_FLAG` true (`control.h:159-161`), so
`subdeviceCtrlCmdGrmgrGetGrFsInfo_IMPL`'s pointer compiles to `NULL` and **its body is not in
the open tree at all**; `rmresControl_Prologue_IMPL` RPCs `pParams->cmd` **unmodified**
(`resource.c:255-291`). ⇒ The id that fails **is** the id to serve, unlike §14.33's.

★ It *does* carry `NON_PRIVILEGED`, so unlike `0x20802a0b` it is probeable from usermode —
but it is a query list with `[IN]` fields, so a `0xCD`-seeded `--probe-ctrl` would ask query
type `0xCDCD`. §14.31's trap applies here with full force. Any sweep must **set**
`numQueries`, `queryType` and the per-type input words.

#### ★ The stride, established from the wire BEFORE the header was opened — and they agree

`traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:64` is a **complete** 1928-byte record
(the interposer's `TRUNC` marker is absent), and byte-diffing its `in=` against its `out=`
gives **exactly two changed bytes in the whole struct**, at offsets **40** and **60**.

⊘ Size arithmetic does **not** discriminate: `1920` divides evenly by 16, 20 *and* 24, and
picking the divisor that "works" is `two_encodings_agreeing_on_the_first_values` in its
purest form. What discriminates is **coherence of the repeated record** — only a 20-byte
stride based at 8 reads `queryType` at the same slot in all three elements, with a monotone
index at `+8` and the single changed word at `+12`. The header then confirms
`8 + 96 × 20 = 1928`.

⚠ `NV2080_CTRL_GRMGR_GR_FS_INFO_QUERY_MAX_SIZE = 32` (`ctrl2080grmgr.h:66`) is a **bound**,
not the element size. A reader who takes the named constant gets `8 + 96 × 32 = 3080` and a
struct that does not exist — the one offset most likely to bite.

#### ⚠ `CHIPLET_GPC_MAP` is NOT implemented as the identity the capture shows

The capture shows `gpc 0→0, 1→1, 2→2`. This module implements *"the `n`-th set bit of
`gpcMask`"*, which is what the query means, and on GA106's contiguous `0b111` the two are
**indistinguishable**. Third sighting this session of a relation fitted to a part that cannot
falsify it (after `FBP_MASK` above and `LTS_COUNT`'s slice constant), and the only one where
the two readings separate on hardware that exists — a floorswept part.

⊘ Sourced from `ChipProfile::gr_static.gpc_mask()`, which derives from the same `gpcs` slice
`GrFloorsweepingMasks` encodes — **not** from `GA106_GPC_MASK`, which would be a second
statement of one silicon.

#### `[measured 2026-08-09, boot `gt1434_373c145`, artifact stamped `kayfabe-rev:373c1454476dc9fb2f5d2ae0373959038e56d703`, STOCK module]` BOTH WALLS DOWN

```text
nvkvm: control 0x20801303 result 0x00000000 x2     ← now TWO calls, not one
nvkvm: control 0x20803801 result 0x00000000 x1
```

★★★★ **`cuInit` went NINE calls further** — `traces/real_ga106/cuinit_trace_guest_gt1434_373c145.txt`,
69 → 82 lines. Compared id-by-id, **size**-by-size and status-by-status against
`cuinit_ioctl_trace_real_ga106.txt`, our trace is now **identical for the first 71 calls with
exactly one exception** and diverges at call **72**.

⊘ The exception is still call 39, `0x20810108` — the `NV2081_BINAPI` control §14.26 measured
as having **no oracle in any instrument**. It is answered `NV_OK` by hardware and `0x56` here,
and `cuInit` has now run **thirty-two further calls** past it. §14.33 established its refusal
was not fatal over twenty-three calls; this run doubles the evidence.

★★ And nine of the eleven calls this rung unlocked are ones this port **must not touch**:
`TURING_USERMODE_A`'s alloc, two `CLIENT_GET_ADDR_SPACE_TYPE`s, two `NV_ESC_RM_MAP_MEMORY`
escapes and `RM_USER_SHARED_DATA` all passed with **no new code at all** — they are the guest
kernel's own, and the correct action on them was nothing. Two controls were served; nine
calls came free.

The new wall is call 72: `0x20803601` `NV2080_CTRL_CMD_GSP_GET_FEATURES`, 72 bytes,
`NV_OK` on a real GA106 and `0x56` here.

#### The next rung — and its value is NOT a chip fact

`GSP_GET_FEATURES` answers `{gspFeatures = 1 (UVM_ENABLED), bValid = 1, bDefaultGspRmGpu = 1,
firmwareVersion = "580.159.04"}`. ⚠ **That string is the HOST driver's version, not the
guest's and not the chip's** — it must be routed through
[`kayfabe_abi::host_driver::HostDriverVersion`], which exists for exactly this reason and
which nothing in `kayfabe-device` reads yet. ⊘ Putting `"580.159.04"` in a GA106 chip row
would make a fact about *this machine's installed driver* into a fact about *a die*, which is
the `PCIE_GEN_INFO` species of error and the one §14.31 already recorded once.

⚠ It also carries `RMCTRL_FLAGS_CACHEABLE` (flags `0x40549`), so the guest may cache the
answer **permanently** — the first row this port would serve that
[`kayfabe_device::sticky::BRANCH_A_CACHEABLE`] actually covers, and the guard at the serve
site stops being unreachable.

#### ★★★ §14.35's rung, RESEARCHED — and the routing question answered before the code, twice

⊘⊘ **Do not take the ranked plan's word for either half of this one.** Both were checked and
one is wrong.

**1. Routing — it IS ours, but not for the reason the flags first suggest.**
`0x20803601`'s flags are `0x40549` (`ogkm-580: g_subdevice_nvoc.c:9466`) =
`NO_GPUS_LOCK(0x1) | NON_PRIVILEGED(0x8) | ROUTE_TO_PHYSICAL(0x40) | API_LOCK_READONLY(0x100)
| CACHEABLE(0x400) | PHYSICAL_IMPLEMENTED_ON_VGPU_GUEST(0x40000)`.

⚠ Unlike `0x20803801`, that last bit makes `NVOC_EXPORTED_METHOD_DISABLED_BY_FLAG` **false**
(`control.h:159-161` requires `ROUTE_TO_PHYSICAL && !PHYSICAL_IMPLEMENTED_ON_VGPU_GUEST`), so
a CPU-RM body **is** compiled in — and reading only that far would conclude the guest answers
it locally. It does not: `rmresControl_Prologue_IMPL` still RPCs on
`IS_FW_CLIENT && ROUTE_TO_PHYSICAL` and returns `NV_WARN_NOTHING_TO_DO`, which skips the
handler (`resource.c:255-291`, `rs_resource.c:191-200`).

★ And the body that exists **proves** it, because it cannot be what a real GA106 does:

```c
subdeviceCtrlCmdGspGetFeatures_KERNEL(…) { pGspFeaturesParams->bValid = NV_FALSE; return NV_OK; }
```

— `subdevice_ctrl_gpu_kernel.c:3569-3578`, the whole function. It sets `bValid = FALSE`, and
hardware answers `bValid = 1`. ⇒ The body is the vGPU-guest arm; on a GSP client the RPC
wins. `[measured, boot `gt1434_373c145`]` `unserviced fn 76 cmd 0x20803601` confirms it
arrives. **Serve the same id.**

**2. ⊘ The firmware version is the GUEST driver's, NOT the host's — and this bench cannot
tell.** The ranked plan says *"route `"580.159.04"` through the host-driver-version pin, not
a chip row"*. Half right: it is certainly not a chip row. But `firmwareVersion` is the
**GSP-RM firmware build**, and GSP firmware ships **inside the driver package** — a guest
running `580.159.04` loads `580.159.04` GSP firmware, and this port *is* that firmware. The
host's driver version is a different machine's fact, and `kayfabe_abi`'s own doc already says
so in as many words: *"The host's driver version is a **different** number about a different
machine, and the product property is that the two need not agree"*.

⚠ **On `vh` both are `580.159.04`**, so no boot on this bench can distinguish the two
choices. Third time in two rungs that a relation is fitted to a part that cannot falsify it,
after `FBP_MASK` and `CHIPLET_GPC_MAP`. ⇒ Serve it from the **guest** `DriverVersion` the
device already detects to select its ABI table, which makes it a projection of a value this
port already holds rather than a new one, and say plainly that the host/guest question is
undecided by measurement here.

★ Nothing in RM reads the field: a repo-wide grep for `firmwareVersion` in `src/nvidia/src/`
finds **one** hit and it is an unrelated HWBC struct (`client_resource.c:3426`). It is
report-only, consumed by libcuda and `nvidia-smi`. So it gates nothing and a wrong choice
here is a **fidelity** defect, not a boot failure — which is exactly the kind that survives.

**3. ★★ It is the first served row `sticky::BRANCH_A_CACHEABLE` actually covers.**
`CACHEABLE (0x400)` is set, so `rmapiControlCacheSetUnchecked` may let the guest cache our
answer **permanently** and later asks never reach the wire. Every id this port serves today
is outside that mask, which is why the guard at the serve site has been unreachable. Serving
`0x20803601` makes it live, and the decision `BRANCH_A_CACHEABLE` exists to force has to be
made rather than inherited.

⇒ Values: `gspFeatures = 1` (`UVM_ENABLED`, `ctrl2080gsp.h:78-80`), `bValid = 1` (truthful —
this port *is* a GSP client with the GPU offloaded to firmware, `:45-49`),
`bDefaultGspRmGpu = 1`, `firmwareVersion = <guest driver version>`. Struct is
`NvU32 + NvBool + NvBool + NvU8[64]` = 70, padded to **72** (`ctrl2080gsp.h:66, 70-75`), and
`[measured]` the wire agrees: the reply's only written bytes are offsets 0, 4, 5 and the
ASCII run at 6.

#### ★★★ §14.35 BOOTED — the wall is down, and the value came off the GUEST rather than a table

`[measured 2026-08-09, boot `gf1435` at `d24ad77`, real GA106 on `vh`, probe flags: **NONE**]`
Both artifacts stamped `kayfabe-rev:d24ad776…` (archive **and** `qemu-system-x86_64`), so this
is not the `862c7c2` shape.

**The rung landed byte-for-byte.** The guest's own trace:

```text
CTRL cmd=0x20803601 hClient=0xc1d0000c hObject=0x5c000003 size=72 status=0x00000000 rc=0
  out=0100000001013538302e3135392e3034 0000…
```

— identical to the real GA106's `out=` at `cuinit_ioctl_trace_real_ga106.txt:73`, and
`0x20803601` appears **zero** times in that boot's unserviced ledger. ★ The ASCII run decodes
`"580.159.04"`, and it got there by being **latched from the guest's own fn 1**, not from any
constant this port holds — the two constants that look right are refuted in
`kayfabe_abi::gspfeatures`.

##### Where `cuInit` now stops, positionally

Our trace is **89 lines**, the same length as the real GA106's, and a positional diff on
`(kind, id, size, status)` gives **77 of 87 rows identical**. ⊘ But the raw count understates
it, because rows 81-88 are not disagreements — they are *our teardown* (`ESC 0x4f`, `FREE`)
where hardware continues. There are exactly **TWO** substantive divergences:

| ln | id | real | ours | verdict |
|---|---|---|---|---|
| 40 | `0x20810108` | `NV_OK` | `0x56` | **benign, re-confirmed** — `cuInit` runs 40 further rows past it |
| 80 | `0x20808159` | `NV_OK` | `0x56` | ★★★ **THE NEW WALL, and it is FATAL** — every row after it is teardown |

⊘⊘ **And one row that looks like a divergence and is not:** `0x2080012f` answers `0x56` at row
49 — **on real hardware too** (`cuinit_ioctl_trace_real_ga106.txt:49`). Our refusal *matches*
there, so it must not be counted as a wall or "fixed". That is the third time on this ladder a
`0x56` has been read as ours when it was the driver's.

⇒ The wall moved from row 73 to row **80**: seven further calls served, `cuInit` still `100`.

##### ★★ What this boot settles about `0x20810108`, and what it does not

Row 40's refusal is now measured non-fatal **twice**, at two different wall positions. ⊘ That is
still only evidence about *reachability*, never about correctness: `injection_measures_necessity_
never_sufficiency`. A served-but-wrong answer there would look exactly like this.

#### ★★★ §14.36's rung, RESEARCHED — `0x20808159`, and the wall map's caveat is REFUTED

The new wall is a **GSS-legacy** control (`0x8159 & 0x8000`), 332 bytes, which the real GA106
answers `NV_OK` with `in == out` (`cuinit_ioctl_trace_real_ga106.txt:80`).

##### 1. ⊘ The "`in == out` might be `SKIP_COPYOUT`" caveat does NOT apply here

The ranked plan flags line 80 as *"`⊘ unmeasured`, not 'the reply is zeros'"*, on the grounds
that `in == out` is *"also the exact signature of `RMAPI_PARAM_COPY_FLAGS_SKIP_COPYOUT`"*. That
is a sound worry for lines 84 and 87 and **cannot apply to this one**, by the plan's own other
finding: a GSS-legacy call **bypasses resserv entirely**, so it never reaches
`rmapiParamsCopyOut` and no `RMAPI_PARAM_COPY` flag is ever consulted. Its copy-out is a bare,
unconditional `portMemExCopyToUser` on `status == NV_OK`
(`ogkm-580: src/nvidia/interface/deprecated/rmapi_gss_legacy_control.c:145-151`), paired with
an unconditional `portMemExCopyFromUser` on the way in (`:72-75`).

⇒ For **this** id, `in == out` is a `[measured 2026-08-09, real GA106 on 580.159.04]` fact:
physical RM was handed the buffer and gave it back byte-unchanged. Preserving the guest's own 332 bytes under `NV_OK` is therefore an
identity this port can stand behind, not a fabricated body.

##### 2. ★★★ The doctrine conflict is real, and the resolution is the ID, not the rule

⚠ This port **deliberately refuses** rule-permitted GSS-legacy controls
(`BridgeRefusal::GspRuleControlUnserviced`), and `kayfabe-rmrpc/tests/gss_legacy_answer.rs`
exists to keep that refusal red-if-removed — because the C's *default* echo of an all-zero
`[OUT]` body is what made cudart fail with `cudaErrorInitializationError(3)` and **no log line**
(`C: nvkvm_gpu_emul.c:3335-3360`).

⊘ That doctrine is about the **default**, and it must not be relaxed. What §14.36 needs is one
**named id** with a measured end-state — the same shape as every rung on this ladder. The
`GraphPolicy` default stays a refusal; `the_gss_legacy_rule_passes_half_the_command_space` and
the echo tests stay exactly as they are.

##### 3. ★★ The sticky question is already answered, and by a guard that is not a comment

The branch-(b) cache condition reads **the reply's** `rmctrlFlags`, i.e. a word *we* write
(`ogkm-580: rpc.c:11098-11103`):

```c
else if (IsGssLegacyCall(cmd) && !FINN_SERIALIZED &&
         rmapiControlIsCacheable(rpc_params->rmctrlFlags, rpc_params->rmctrlAccessRight, NV_TRUE) &&
         !(rpc_params->rmctrlFlags & RMCTRL_FLAGS_CACHEABLE_BY_INPUT))
    rmapiControlCacheSetUnchecked(…);
```

and `StickyAnswerGuard::respond` **already zeroes both words unconditionally** on every accepted
control reply, with `rmapi_control_is_cacheable(0, …)` returning `false` on its first conjunct.
⇒ A served GSS-legacy id cannot become sticky while that link is in the chain.

⚠ Which makes `InitTablePolicy`'s own `if is_gss_legacy(req.cmd) { return refuse(); }` the thing
that must change — and its own comment already says what it is: a tripwire written when *"every
id this port serves is outside that mask today"*. That stopped being true the moment this rung
lands, so the tripwire has to become the narrower statement it always meant, rather than being
deleted.

⊘ **Not measured, and it is the reason to boot rather than reason further** (boot `gf1435` at
`d24ad77` could not see it): whether `0x20808159`'s refusal is the *last* wall. Rows 81-88 are unobserved on our side — every one of
them is currently our teardown — so the eight calls after it have never been exercised against
this port at all.

#### ★★★★ §14.37 BOOTED — `cuInit`'s CONTROL PLANE IS COMPLETE, and the wall is no longer a control

`[measured 2026-08-09, boot `gf1438` at `20126b5`, real GA106 on `vh`, probe flags: **NONE**]`
Both artifacts stamped `kayfabe-rev:20126b54…`.

A positional diff of the guest's `cuInit` trace against
`traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt` on `(kind, id, status)` now leaves **one**
control divergence in the whole of rows 2-87:

| row | id | real | ours | verdict |
|---|---|---|---|---|
| 40 | `0x20810108` | `NV_OK` | `0x56` | benign — `[measured]` three boots running, `cuInit` proceeds 47 rows past it |

Rows 85, 86 and 87 (`0x20808162`, `0x2080182b`, `0x20803002`) all match. ⇒ **every control
`cuInit` issues is now either served correctly or refused exactly where a real GA106 also
refuses** (`0x2080012f`, row 49).

##### ★★★ Where it stops now, and why that is a change of KIND

Real hardware's row 88 is `ALLOC hClass=0x000050a0` — `NV50_MEMORY_VIRTUAL`, the last row before
`MARK stage1 cuInit END`. Our trace never reaches it: after row 87 libcuda goes straight to
teardown (`ESC 0x4f`, `FREE …`). `cuInit` returns **3**.

⊘⊘ **So the remaining gap is no longer a wall in the sense the last five rungs used the word.**
Every previous rung had a named refusal at a known row, visible in the unserviced ledger and
diffable by id. This one has neither: nothing is refused at row 88 because **the guest never asks**.
That makes the ledger, the census and the id-diff — the three instruments this ladder has run on —
all structurally blind to it, which is exactly the condition
`a_saturated_instrument_looks_exactly_like_absence` warns about.

⚠ And it is the predicted place: the original wall map's rung 4 called the alloc/RPC plane *"the
real remaining work, and it is not values — it is new verbs"*, and said it was the one item whose
**shape** is new rather than whose value is new. That prediction has held.

##### ⊘ What `cuInit → 3` does NOT license

`3` is `CUDA_ERROR_NOT_INITIALIZED`, and it is also the number the C artifact's echo defect
produced at a completely different layer (`cudaErrorInitializationError`). ⊘ Do not read the two
as the same finding. What is `[measured]` is positional: the trace grew from 89 lines at
`d24ad77` to 96 at `20126b5`, and rows 81-87 moved from *"our teardown"* to *"matches hardware"*.
That is the sound progress claim; the error code is not evidence of anything on its own.

### 14.40 ★★★★ THE WALL WAS ONE REGISTER — `BAR0 + 0x88084`, and it is not on any plane this port was watching (`[measured 2026-08-09, boots us1445/lc1446 @ 69f8817, pu1448 @ ef20ccc]`)

#### ⊘⊘ First, the three things REFUTED — including the whole of this rung's brief

The brief said: `cuInit`'s isolated dmesg names three classes we refuse, so serve them and boot.
**All three refusals are correct and none of them is the wall.**

| class | who allocates it | proof, and it is a HANDLE, not an inference |
|---|---|---|
| `0x0070` `NV01_MEMORY_VIRTUAL` | the RC watchdog | `hParent=0x31415903` = `WATCHDOG_DEVICE_ID` (`ogkm-580: kernel_rc_watchdog.c:63,65` — `WATCHDOG_PUSHBUFFER_CHANNEL_ID 0x31415900` + 3); the alloc site is `:669-672` |
| `0xc36f` `VOLTA_CHANNEL_GPFIFO_A` | the RC watchdog | same `hParent`; alloc site `:1013-1017` |
| `0x402c` `NV40_I2C` | `RmInitAdapter`'s own client | `hParent=0xcaf00001` is an RM-server unique handle (`rs_client.h:37 RS_UNIQUE_HANDLE_BASE 0xcaf00000`) ⇒ `osinit.c:1767`, whose own comment says *"expected to fail"* |

★★ **`0xc36f` is a FINGERPRINT, not a class choice.** The watchdog's `gpfifoMapping[]` breaks on the
**first** supported entry in Kepler→Blackwell order (`kernel_rc_watchdog.c:619-651`); GA106's class
list carries VOLTA, TURING **and** AMPERE GPFIFO (`g_gpu_class_list.c:1108-1171`), so it picks
**VOLTA**. Every other consumer takes `NV_MAX` (`nv_gpu_ops.c:8679-8690`) and gets `0xc56f`, which
this port already serves. ⇒ on a GA106 a `0xc36f` alloc can only be the watchdog's.

⊘ **UVM allocates neither.** `NV01_MEMORY_VIRTUAL` has exactly three RM callers
(`kernel_rc_watchdog.c:673`, `rmapi_deprecated_misc.c:115`, nvkms) and appears **nowhere** in
`nv_gpu_ops.c`, which uses `NV50_MEMORY_VIRTUAL`. So `uvm_channel_manager_create` being on
`cuInit`'s path — which §14.39 established and which is true — does **not** implicate these classes.

★★★ **And the refusals are LOAD-BEARING in the forgiven direction.** `krcWatchdogInit` failing
aborts `RmInitAdapter` (`RM_INIT_WATCHDOG_FAILED`, `goto shutdown`) for every status **except**
`NV_ERR_NOT_SUPPORTED`, which `osinit.c:2167-2172` logs at `LEVEL_INFO` and forgives. Our `0x56` is
the only reason the device opens at all. ⇒ the question was never *"serve or refuse"*; it was
*"refuse with WHICH status"*, and that was already right. `not_supported_is_the_forgiven_status`
again, from the other side.

#### ★★★ The measurement that relocated everything: `params.rmStatus` (`[measured 2026-08-09, boot us1445, rev 69f8817]`, evidence `docs/reference/bench_evidence/uvm-rmstatus-us1445-69f8817.log`)

`UVM_REGISTER_GPU` returns **0** at the syscall boundary and carries its verdict in
`params.rmStatus` (`ogkm-580: uvm_ioctl.h:534-543`). `strace` prints `= 0`; UVM prints **nothing**
to `dmesg` on this path (`grep -ai uvm` ⇒ *no line*). Two instruments, both silent, both by design.

`scripts/bench/uvm_ioctl_trace.c` (new — an `LD_PRELOAD` that reads the struct after the call)
on boot **`us1445` @ `69f8817`**:

    ★ UVM_REGISTER_GPU rmStatus = 0x00000040   <- NV_ERR_INVALID_STATE
      IN: rmCtrlFd = -1, hClient = 0, hSmcPartRef = 0   (the non-SMC path)

⚠ Every ioctl instrument this project owns gates on `_IOC_TYPE == 'F'` and UVM's magic is **0**
(`scripts/rpctrace/cuda_ioctl_trace.c:493`). The plane holding the answer was filtered out of every
trace on disk. Evidence: `docs/reference/bench_evidence/uvm-rmstatus-us1445-69f8817.log`.

#### The chain, every link from `ogkm-580`, with no branch left in it

    UVM_REGISTER_GPU.rmStatus = 0x40
      <- nvGpuOpsGetGpuInfo            nv_gpu_ops.c:7220   (returns the first failure)
      <- getPCIELinkRateMBps           nv_gpu_ops.c:2118   [BOTH its prints are in that dmesg]
      <- calculatePCIELinkRateMBps     nv_gpu_ops.c:2077-2079  default arm, "Unknown PCIe speed":
         MAX_SPEED (3:0) was not one of the six legal encodings (ctrl2080bus.h:357-363)
      <- NV2080_CTRL_BUS_INFO_INDEX_PCIE_GPU_LINK_CAPS (0x03)
      <- ⊘ NOT an RPC. getBusInfos's bSendRpc switch (kern_bus_ctrl.c:296-330) forwards THIRTEEN
         indices and 0x03 is not one of them; the guest answers it itself at kernel_bif.c:1072
      <- kbifGetGpuLinkCapabilities    kernel_bif.c:879-903
      <- GPU_BUS_CFG_RD32(NV_XVE_LINK_CAPABILITIES)
      <- GPU_REG_RD32(DEVICE_BASE(NV_PCFG) + 0x84) = **BAR0 + 0x88084**  kern_gpu_gm107.c:176-190

⊘ The early-out `gpuGetBusIntfType != PCI_EXPRESS` cannot fire: on a discrete GA106 that HAL is the
compile-time constant `3` (`g_gpu_nvoc.h:4633`, dispatch `g_gpu_nvoc.c:1112-1126`).

Boot **`lc1446` @ `69f8817`** read the word from inside the guest through `resource0`
(`scripts/bench/guest_linkcap_probe.sh`, evidence `linkcap-before-lc1446-69f8817.log`):

    BAR0+0x88084 = 0x00000000   MAX_SPEED(3:0) = 0  ★ ILLEGAL
    BAR0+0x88000 = 0x00000000   NV_XVE_ID — the WHOLE NV_PCFG window is unclaimed
    setpci CAP_EXP: "no capabilities with that id"

★ The last line is worth keeping: this device presents **no PCI Express capability at all**, and it
does not matter — RM never reads configuration space for this. ⊘ A fix applied there would have
changed nothing and would have looked plausible.

#### ⊘⊘ CORRECTED IN PLACE: `businfo.rs` said "or from its own config space"

That module's own docs described `0x03` as *"answered from the guest's own kernel state **or from
its own config space**"*. The second half is false, and it is **why nobody looked**: it made `0x03`
read as a plane QEMU already emulates. ★ A comment naming the plane a value comes from is a
**claim**, and this one was never checked against the macro it was describing.

#### The fix: one `BootReg`, DERIVED, and the trap it created

`ga10x.rs`: `XVE_LINK_CAPABILITIES = 0x0008_8084`, value
`PcieLinkCaps::fully_trained(GA106_PCIE_MAX_GEN).encode()`.

⊘ **Not the measured word.** A real GA106 answers `0x00454d03`, whose `MAX_SPEED` is `3` = 8 GT/s —
one generation *below* the `gen4` die, because an NVIDIA endpoint advertises what it trained to in
the slot it is in. Transcribing it would be `0x20802a08` again. The committed oracle
(`traces/real_ga106/rmladder_r22_businfo_loaded_real_ga106.txt`) is used as an oracle **for the
field layout** and nothing else.

★ `GA106_PCIE_MAX_GEN` is now stated **once** and consumed by both planes — the chip row's
`pcie_max_gen` and this register — because two independent statements of one die's generation are
two statements free to disagree.

⚠⚠ **One word, two encodings of "which PCIe generation", off by one.** `GEN`/`CURR_LEVEL`/`GPU_GEN`
are zero-based (`GEN1 == 0`); `MAX_SPEED` is one-based (`_2500MBPS == 1`). Reaching for
`PcieGen::field()` when filling `MAX_SPEED` sends `Gen4` out as `3` — a **legal** value, so nothing
refuses it and the link is understated forever — and sends `Gen1` out as **0**, the exact value that
stopped `cuInit`. Hence `PcieGen::max_speed_field()` as a separate method, and
`every_generation_encodes_to_a_speed_calculate_pcie_link_rate_accepts` asserting against the six
legal encodings **by name**.

#### ★★★★ BOOTED — the wall moved, and its successor is NAMED

Boot **`pu1448` @ `ef20ccc`**, identical harness and hook to `us1445` (evidence
`uvm-rmstatus-pu1448-ef20ccc.log`):

    BAR0+0x88084 = 0x00000104   MAX_SPEED = 4 (16 GT/s)  MAX_WIDTH = 16   [pc1447]
    ★ UVM_REGISTER_GPU rmStatus = 0x00000056     (was 0x00000040)

and one line appears in `cuInit`'s dmesg that was **not there before**:

    NVRM: faultbufConstruct_IMPL: Failed to setup Replayable Fault buffer (status=0x00000056).

with exactly one new id in the unserviced ledger: **`0x20800a9b`
`NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_FAULT_BUFFER`** (`ogkm-580: ctrl2080internal.h:1810`).
`faultbufConstruct_IMPL` (`mmu_fault_buffer.c:34-70`) calls
`kgmmuFaultBufferReplayableAllocate`, which is what issues it.

⇒ **The next rung is `0x20800a9b`**, and unlike this one it *is* visible in the ledger. UVM's
replayable fault buffer is the same machinery `MMU_FAULT_QUEUED (0x1005)` is listed against in the
alloc/RPC map's §5 — the GSP→guest event direction, which that map already named as the genuinely
unmapped territory.

#### ⊘ What this does NOT license

- **Not** that `cuInit` is close. `0x56` is one step further than `0x40`, and that is all.
- **Not** that the completion plane moved. `scrubberDestruct: Timed out` /
  `ce_utils.c:349 lastCompletedPayload != lastSubmittedPayload` and
  `first doorbell refusal [FwdFault::NoVas] NoVas(ChanId(3))` are unchanged, on a build whose
  isolates still report `no-plane`. A different failure, untouched by this.

##### ⊘⊘ CORRECTED — and it removes a whole line of enquiry rather than opening one

`§14.39` filed the `NoVas` doorbell beside the `cuInit` failure and this section first repeated
that framing as *"the completion plane"*. Both readings are **wrong about whose channel it is**,
and the correction is decisive:

- **`ChanId(3)` is not UVM's channel.** The discriminator we print is `c=0xc1e00010`, and
  `0xC1E00000` is `RS_CLIENT_INTERNAL_HANDLE_BASE` (`ogkm-580: resserv.h:135,138`;
  `serverIsClientInternal`, `rs_server.c:2624`) — an **internal** client, reachable only under
  `RMAPI_GPU_LOCK_INTERNAL`, which `ceutilsConstruct_IMPL` takes (`ce_utils.c:188`). UVM's module
  uses `RMAPI_EXTERNAL_KERNEL` and lands on the `0xC1D0…` base. ⇒ it is **RM's own CE-scrubber**.
- ⚠ **`ChanId` could not have told you that.** It is a per-`Proc` mint counter
  (`gpu.rs:2386-2391`) — not the chid, not stable across boots. `hClient` is the only
  discriminator in that line that means anything, and reading `ChanId(3)` as an identity is the
  `two_encodings_agreeing_on_the_first_values` shape with a counter instead of an enum.
- **`scrubberDestruct` is TEARDOWN.** It fires ~4 s and ~8 s *after* the last real error, on the
  unwind path (`memmgrPreSchedulingDisableHandler` → … → `scrubberDestruct`), a destruct handler
  with no init-path caller — and at the *same relative position* in `us1445` and `pu1448`.
  ⊘ **Not on `cuInit`'s critical path**, in either boot.

⊘⊘ **And do NOT "fix" `NoVas` by binding a VA space.** `isolate refusal [no-plane]` is
`StillbornIsolates` — a **deployment** fact (the default when `KAYFABE_ISOLATES` is unset,
`shim.rs:3111-3114`), meaning nothing was even attempted — and it **dominates**: with a `Vas`
bound, `plan_doorbell` would reach a zero-width retired isolate and still execute nothing, while
the doorbell reported **Served**. ★ That is §14.8's measured lesson exactly: granting a channel a
VAS before an executor exists converts a *correct refusal* into a false success. The fix is the
executor (E5 + a real isolate plane), which is an increment, not an edit.

##### ★ Two instrument repairs this rung paid for on the way past

1. **`shim.rs`'s doorbell probe collapsed two different facts into one string.** `vas=none
   ring=none` was printed whenever *either* was absent, so it read as *"the channel declared
   neither"* — a claim. `[measured 2026-08-09, boots us1445 @ 69f8817 and pu1448 @ ef20ccc,
   evidence docs/reference/bench_evidence/uvm-rmstatus-{us1445-69f8817,pu1448-ef20ccc}.log]` both
   boots print the identical `c=0xc1e00010 vas=none ring=none` for the refused doorbell; the
   **source** then says which half was missing, and that half is the VA space, because
   `AllocParams::Channel` sets `gp_fifo_ring: Some(..)` unconditionally
   (`kayfabe-rmrpc/src/lib.rs:1269-1272`) while `h_vaspace` goes through `declared_handle` — so on
   that path the two cannot be absent together. ⚠ Note the split honestly: the *string* is
   measured, the *attribution of which half* is a reading, and it is only because the two were
   conflated in one word that a reading was needed at all. It now names each half separately. ⚠ The old string cost an auditor three
   source files to disambiguate — a diagnostic that conflates two facts sends its reader elsewhere.
2. **The CE wall's arming state is one INFO line and nobody was reading it.**
   `Initializing global CeUtils instance` vs `Skipping global CeUtils creation` decides whether
   `memmgrTestCeUtils`'s `memmgrMemSet(.. PREFER_CE)` (`ogkm-580: mem_mgr.c:463`) runs at all —
   and that memset is where boots `stock_c89899a` / `p35_84d857d` died. `guest_uvm_status.sh` now
   greps for it every boot. ⊘ A boot showing `Skipping` has the CE wall **dormant** and cannot be
   cited as evidence the CE path works: the absence of the failure is the absence of the attempt.

##### ⚠ Process, recorded because it nearly cost this adjudication

`§14.39`'s commit message pastes RM's `rpcRmApiAlloc_GSP` output with the `hObject=` /
`paramsSize=` fields **stripped** and `->` annotation lines interleaved. Ordering cannot be read
off it, and the handles that settle *whose* allocations they are — `hObject=0x3141590f`
`WATCHDOG_VIRTUAL_CTX_ID`, `hObject=0x31415900` `WATCHDOG_PUSHBUFFER_CHANNEL_ID` — are exactly what
was removed. ⇒ **paste evidence verbatim or mark the edit.** The raw logs persisted beside it are
what made the adjudication possible, which is the argument for persisting them.
- ⚠ **A harness note, measured here**: boot `pc1447` ran a second device-opening probe before
  `cup2` and `cup2`'s `RmInitAdapter` then died on `WPR2 already up` / `Bad sequence number.
  Expected 0 got 190`, giving `cuInit → 999`. That is the stale-queue chain, not a regression:
  the emulated GSP's WPR2 only resets on a full QEMU restart. ⇒ **one device-opening consumer per
  boot**, and `pu1448` is the A/B that matters because its hook is byte-identical to `us1445`'s.

---

## §14.41 — `0x20800a9b` SERVED: the wall moves off the replayable fault buffer

`[measured 2026-08-09, boot `fb1503` at `3afa896`]`, one device-opening consumer, probe set
EMPTY, evidence verbatim under `docs/reference/bench_evidence/run_fb1503_3afa896_*.log`.

### The A/B, one control apart

```
  pu1448 @ ef20ccc :  NVRM: faultbufConstruct_IMPL: Failed to setup Replayable Fault buffer
                            (status=0x00000056).
                      unserviced fn 76 cmd 0x20800a9b
  fb1503 @ 3afa896 :  ⊘ that line is GONE.
                      control 0x20800a9b result 0x00000000 x1
                      NVRM: faultbufCtrlCmdMmuFaultBufferRegisterNonReplayBuf_IMPL: Error
                            allocating client shadow fault buffer for non-replayable faults
                      unserviced fn 76 cmd 0x20800a9d   ← THE NEXT RUNG
                      unserviced fn 76 cmd 0x20800a9c   ← its UNREGISTER partner
```

`UVM_REGISTER_GPU rmStatus` is **still `0x56`** and `cup2` still returns 1. ⊘ The rung moved;
`cuInit` did not pass. Stated first so no line below can read as more than it is.

### ★★★ Three predictions CONFIRMED — `[measured 2026-08-09, boot `fb1503` at `3afa896`]`

…and one of the three was a design decision, not an observation waiting to happen.

1. **The two controls agree on the geometry, measured.** The device's own report:
   `replayable fault buffer: 1 registration(s) SERVED NV_OK; first 0x31000 B = 49 pages, 0
   malformed`. `0x31000` is exactly the `replayableFaultBufferSize` this port answers to
   `0x20800a59` (`ga10x.rs:1515-1520`), and 49 = `0x31000 / 4096` — comfortably under the
   vendor's 256-entry `faultBufferPteArray` bound. The guest registered back what we
   advertised. ⊘ `register_fault_buffer.rs::the_size_the_guest_registers_is_the_size_this_port_advertised`
   asserted this against the policy; the boot is the second, independent source.
2. ★★★ **Leaving `0x20800a9c` UNREGISTER unserved was the right call, and it is now MEASURED
   rather than inferred.** The commit predicted, from `ogkm-580: kern_gmmu.c:1325-1333`, that
   CPU-RM logs the unregister's failure and proceeds. The guest said exactly that:
   `NVRM: kgmmuFaultBufferReplayableDestroy_IMPL: Unregistering Replayable Fault buffer failed
   (status=0x00000056), proceeding...` ⇒ the register/unregister pair could have been a latch
   that only closes; refusing to model half of it costs nothing, and the guest says so in its
   own words.
3. **Exactly ONE registration.** The `> 1` warning path did not fire, so the receiver's
   double-register rule (`ogkm-580: kern_gmmu.c:3117`) is still unexercised — and correctly
   still unmodelled, on evidence rather than on a paragraph.

### ★★ And the unbuilt half printed itself

```
nvkvm: replayable fault buffer: 1 registration(s) SERVED NV_OK; first 0x31000 B = 49 pages, 0 malformed
nvkvm:   ⊘ fault DELIVERY is UNBUILT: this port raises no replayable fault and never advances
         MMU_FAULT_BUFFER_PUT(1), so a fault the guest should have been told about becomes a HANG
         inside UVM's replayable-fault service loop, not an error
         (docs/design/resume_from_fault.md §7 steps 5b-5d)
```

⇒ The boot that first served the control is also the first boot that states what serving it did
not buy. That is the whole of why the marker had to land in the same commit as the answer: a
census row reading `control 0x20800a9b result 0x00000000` is indistinguishable from a built
feature, and the failure it hides has no message at all.

### The next rung

`0x20800a9d` — the **client shadow fault buffer for NON-replayable faults**. ⚠ Do not assume it
is the replayable case again: `simulated_gpu_fault.md` §3.3 records that on a GSP client the
**GSP** is what copies non-replayable faults into the client shadow buffer
(`nvGpuOpsReportNonReplayableFault`, `ogkm-580: src/nvidia/src/kernel/rmapi/nv_gpu_ops.c:11154-11185`),
i.e. **we** would be the writer. If that holds, answering `NV_OK` there claims a capability in a
strictly stronger sense than it did here, and the honesty question must be re-asked from scratch
rather than inherited from this rung.

### Harness notes

- The guest auto-loads `nvidia` at ~17 s; `dmesg` at that point carries the module banner and
  **no** RM init. So a boot's first device open is genuinely the probe's, and this one had one
  device-opening consumer. ⊘ `run_fb1503_3afa896_dmesg.log` is persisted and asserted to contain
  `NVRM` before being cited.
- `is the CE wall ARMED or DORMANT?` → **`NEITHER LINE PRESENT`**. ⊘ So this boot says nothing
  about the CE wall in either direction; do not read the absence of `memmgrTestCeUtils`'s failure
  as evidence the CE path works.
- The end-of-run census is emitted by `nvkvm_exit_notify`, i.e. **only on QEMU exit**. A boot left
  running has no census, and grepping its `qemu.log` for a control id finds nothing whether or not
  the control arrived. ⇒ quit the monitor before citing any census line.

---

## §14.41 rung 2 — `0x20800a9d` SERVED: the wall leaves the fault plane entirely

`[measured 2026-08-09, boot `sh1605` at `075395f`]`, one device-opening consumer, probe set
EMPTY, evidence verbatim under `docs/reference/bench_evidence/run_sh1605_075395f_*.log`.

### The A/B, one control apart — and the status CHANGED CLASS

```
  fb1503 @ 3afa896 :  NVRM: faultbufCtrlCmdMmuFaultBufferRegisterNonReplayBuf_IMPL: Error
                            allocating client shadow fault buffer for non-replayable faults
                      UVM_REGISTER_GPU rmStatus = 0x00000056   NV_ERR_NOT_SUPPORTED
  sh1605 @ 075395f :  ⊘ that line is GONE. Both fault buffers register.
                      NVRM: uvmInitializeAccessCntrBuffer(pGpu, pUvm, pAccessCounterBuffer)
                            @ access_cntr_buffer.c:72  →  NV_ERR_INVALID_ARGUMENT
                      UVM_REGISTER_GPU rmStatus = 0x0000001f   NV_ERR_INVALID_ARGUMENT
```

`cup2` still returns 1 and `cuInit` still fails. ⊘ Stated first.

### ★★★ The rung changed CLASS, and that is the finding

`0x56` → `0x1f` is not "one more control down". **No new id entered the unserviced ledger** —
the ledger's 24 distinct entries are the same set as `fb1503`'s plus `0x20800a9e`, the shadow
buffer's own unregister. ⇒ Nothing is missing. Something we **already answer** carries a value
UVM rejects, which is the same shape as §14.40's `BAR0+0x88084` and **not** the shape of the
two rungs before it. ⚠ A rung of this class cannot be found by reading the unserviced list, and
an agent that only greps that list will report "nothing to do".

★ Note also where the assertion is: `access_cntr_buffer.c:72` is **UVM's access counter
notification buffer**, which is a third fault-adjacent object after the replayable HW buffer
and the non-replayable client shadow queue. `resume_from_fault.md` §4.3 `[meas]` already
recorded the C's guest reading `ACCESS_COUNTER_NOTIFY_BUFFER_SIZE` at BAR0 `0xB83110` → `0x100`,
and §7 step 1 already flags that value as *"keep it (it is load-bearing for `cuInit`) but write
it down as a deliberate lie with its reason"*. That is the first place to look.

### ★★★ Both markers printed, and the geometry matched on BOTH controls

```
nvkvm: replayable fault buffer: 1 registration(s) SERVED NV_OK; first 0x31000 B = 49 pages, 0 malformed
nvkvm:   ⊘ fault DELIVERY is UNBUILT: … a HANG inside UVM's replayable-fault service loop, not an error
nvkvm: client shadow fault buffer: 1 registration(s) SERVED NV_OK; first 0x120c20 B = 289 pages, type 0 (0=non-replayable), 0 malformed
nvkvm:   ⊘ shadow-queue PUSH is UNBUILT: on a GSP client the GSP is the WRITER of this queue …
```

`0x120c20` is exactly the `nonReplayableFaultBufferSize` this port answers to `0x20800a59`, and
289 = `align_up(0x120c20)/4096`. **Both** controls now round-trip their own advertised geometry
`[measured 2026-08-09, boot `sh1605` at `075395f`]`, and `type 0` confirms only the non-replayable shadow buffer is registered with
Confidential Compute off — the CC-gated replayable shadow never arrived, exactly as
`mmu_fault_buffer_ctrl.c:148` says it cannot.

### ★★★ The unserved-UNREGISTER decision confirmed TWICE — `[measured 2026-08-09, boot `sh1605` at `075395f`]`

```
NVRM: kgmmuClientShadowFaultBufferUnregister_IMPL: Unregistering non-replayable fault buffer
      failed (status=0x00000056), proceeding...
NVRM: kgmmuFaultBufferReplayableDestroy_IMPL: Unregistering Replayable Fault buffer
      failed (status=0x00000056), proceeding...
```

Both `0x20800a9c` and `0x20800a9e` are refused, both are logged-and-proceeded, and neither
costs anything. ⇒ Refusing to model half of a register/unregister pair is safe on **both**
pairs `[measured 2026-08-09, boots `fb1503` at `3afa896` and `sh1605` at `075395f`]`, not
extrapolated from one.

### ⊘ What this boot still does not say

- `is the CE wall ARMED or DORMANT?` → **`NEITHER LINE PRESENT`** again. Two boots in a row have
  said nothing about the CE wall; the probe's grep is not finding the line at this loglevel and
  that is an instrument gap, not a result.
- `scrubberDestruct` / `ce_utils.c:349` fire ~4 s and ~8 s after the last real error, on the
  unwind path, at the same relative position as in every previous boot. ⊘ Teardown, not cause.

---

## §14.41 rungs 3+4 — BOOTED `ac1710`: the fault plane is BEHIND us

`[measured 2026-08-09, boot `ac1710` at `0abca34`]`, one device-opening consumer, probe set
EMPTY, evidence under `docs/reference/bench_evidence/run_ac1710_0abca34_*.log`.

```
  sh1605 @ 075395f :  uvmInitializeAccessCntrBuffer(...) @ access_cntr_buffer.c:72
                      UVM_REGISTER_GPU rmStatus = 0x0000001f   NV_ERR_INVALID_ARGUMENT
  ac1710 @ 0abca34 :  ⊘ that line is GONE. The access counter buffer constructs.
                      NVRM: ... NV2080_CTRL_CMD_CE_GET_PHYSICAL_CAPS @ kernel_ce.c:550
                      NVRM: queryCopyEngines: queryCopyEngines:8511: Call not supported
                      UVM_REGISTER_GPU rmStatus = 0x00000056
                      unserviced fn 76 cmd 0x20802a07   ← THE NEXT RUNG
```

`cup2` still returns 1. ⊘ Stated first.

### ★★★ The `[predicted]` rung, now MEASURED — `[measured 2026-08-09, boot `ac1710` at `0abca34`]`

```
nvkvm: access counter buffer: 1 registration(s) SERVED NV_OK; first 0x2000 B = 2 pages, 0 malformed
```

`0x2000` = 8192 = **256 advertised entries × 32 bytes**, and 2 pages is `2 *
ACCESS_COUNTER_ENTRIES_PER_PAGE`'s own arithmetic coming back to us through the guest. ⇒ Two
things established at once `[measured 2026-08-09, boot `ac1710` at `0abca34`]`: `0x20800a1d` really is sent (it was served in the same commit on a
prediction, and the prediction held), and the guest sized its buffer from **exactly** the
fiction we advertised at BAR0 `0xB83110`. ★ That closes the loop on the fiction: it is not
merely accepted, it is *propagated* — which is the strongest possible reason for the
`[ADVERTISED FICTION]` label to be on the register row where a dump will meet it.

### ★★★ The UNREGISTER ruling now holds on THREE pairs

```
NVRM: kgmmuClientShadowFaultBufferUnregister_IMPL: ... failed (status=0x00000056), proceeding...
NVRM: kgmmuFaultBufferReplayableDestroy_IMPL:      ... failed (status=0x00000056), proceeding...
NVRM: uvmTerminateAccessCntrBuffer_IMPL: Unloading UVM Access counters failed (status=0x00000056), proceeding...
```

`0x20800a9c`, `0x20800a9e` and now `0x20800a1e` — all three refused, all three
logged-and-proceeded. ⇒ *"Do not model half of a register/unregister pair"* now holds on three
independent pairs `[measured 2026-08-09, boots `fb1503`, `sh1605` and `ac1710`]`, and the cost
is still zero.

### Where `cuInit` is now

The wall has left the **fault plane entirely**. Four rungs ago it was
`faultbufConstruct_IMPL`; it is now `queryCopyEngines` — copy-engine capability discovery
inside the same `UVM_REGISTER_GPU`. The next rung is `0x20802a07`
`NV2080_CTRL_CMD_CE_GET_PHYSICAL_CAPS`, the per-instance sibling of `0x20802a0b`
`CE_GET_ALL_PHYSICAL_CAPS` which this port **already serves** — so the answer is likely to
be a projection of a table already in the tree rather than a new fact.
⚠ Verify that rather than assume it: `ce_get_all_physical_caps.rs` derives its bytes from two
independent real-GA106 captures, and a sibling control that *looks* like a slice of the same
table is exactly the shape that invites an unchecked reuse.

### The three markers, all printing

All three unbuilt-half sentences appear in this boot's own report, each naming its own gap. ⊘
None of them has become true; they are louder now precisely because more of the plane is
served around them.

### ⧗ The next rung, researched to the struct so the next agent starts there

`0x20802a07` `NV2080_CTRL_CMD_CE_GET_PHYSICAL_CAPS`. ⊘ **It is NOT a pure-`[IN]` echo** like
the three before it — it has a real `[OUT]` body, so the previous three rungs' argument does
**not** carry over and the value has to come from somewhere.

```c
/* ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080ce.h:283-287 */
#define NV2080_CTRL_CMD_CE_GET_PHYSICAL_CAPS (0x20802a07)
typedef NV2080_CTRL_CE_GET_CAPS_V2_PARAMS NV2080_CTRL_CE_GET_PHYSICAL_CAPS_PARAMS;
/* :82-85 */
typedef struct NV2080_CTRL_CE_GET_CAPS_V2_PARAMS {
    NvU32 ceEngineType;                              /* [IN]  */
    NvU8  capsTbl[NV2080_CTRL_CE_CAPS_TBL_SIZE];     /* [OUT], 2 bytes */
} NV2080_CTRL_CE_GET_CAPS_V2_PARAMS;
```

Caller: `kceGetDeviceCaps` / `queryCopyEngines` sends it **once per LCE** with
`ceEngineType = NV2080_ENGINE_TYPE_COPY(pKCe->publicID)` and `portMemCopy`s the 2-byte
`capsTbl` straight out (`ogkm-580: src/nvidia/src/kernel/gpu/ce/kernel_ce.c:556-576`). The
macro is `COPY(i) = (i < 10) ? COPY0 + i : COPY10 + i - 10`, `COPY0 = 0x09`
(`ogkm-580: src/common/sdk/nvidia/inc/class/cl2080_notification.h:293-295, 398`) — ⚠ a
**two-branch** mapping, and the second branch is exactly the sort of off-by-ten that a
`0x09 + i` shortcut would get right on this part and wrong on a bigger one.

★★ **This is a derivation opportunity, not a fabrication.** [`kayfabe_abi::cecaps`] already
serves `0x20802a0b` `CE_GET_ALL_PHYSICAL_CAPS`, whose `capsTbl[64][2]` is double-sourced from
**two independent real-GA106 captures** (`present = 0x0f`, `[measured 2026-08-09]`). The
per-engine answer is `capsTbl[publicID]` from that same table. ⇒ Project it; do not tabulate a
second, separate statement of the same silicon fact.
⚠ And check the boundary the module already documents: only engines with a `present` bit have a
`capsTbl` slot, so a `ceEngineType` outside it must be a **named refusal**, not a zero row.

---

## 15. §14.43 — `KAYFABE_ISOLATES=real` ROUTES THE SCRUBBER AWAY FROM THE ONLY EXECUTOR THAT COMPLETES

`[measured 2026-08-09]`, two boots, **one revision** (`319d29a`), **one environment
variable** apart. Both artifacts stamped
`kayfabe-rev:319d29a3cb0f988dc2c85f92c1b2676bae4c17bd`; host GPU `GPU-d0913685` (RTX 3060),
host driver 580.159.04; guest 580.159.04 open, **stock**; probe-arm set EMPTY.
Evidence: `traces/guest_boots/fmb1_319d29a_*` and `traces/guest_boots/iso1_319d29a_real_*`.

### 15.1 The two arms, and they invert

| arm | `KAYFABE_ISOLATES` | doorbell lines (verbatim, from the committed QEMU logs) | `RmInitAdapter` | `nvidia-smi` | `cuInit` |
|---|---|---|---|---|---|
| `fmb1` | *unset* | `SERVED-LOCAL [CpuCe::ServedLocally]` ×4, `REFUSED [FwdFault::IsolateRetired]` ×1 | never fails | `SMI_RC=0` | **hangs** in `uvm_push_end_and_wait` |
| `iso1` | `real` | `SERVED` ×3 | **`failed! (0x25:0x65:1249)`** | `SMI_RC=6` | `999` |

`iso1`'s census is everything the increments promised: `isolates: 1 materialized, 1 live, 0
refusing`, `doorbells: 3 arrived, 3 served, 0 REFUSED by name`, `bind engineType 11 (COPY2)`
×3 all `result 0`. **The plane works.** And the boot is strictly worse.

### 15.2 ★★★ The cause, and the code already predicted it in writing

`SharedDoorbell::local_ce_is_the_only_executor` is set at realize from
`isolate_plane == IsolatePlane::Stillborn` (`crates/kayfabe-qemu-raw/src/shim.rs:2523`), and
`try_ce_submission` gates on it (`:2156-2159`):

```rust
if facts.vas_pdb.is_some() && !self.local_ce_is_the_only_executor {
    return None; // the core can address AND serve this channel; it is not ours.
}
```

With the real plane installed the flag is `false`, so the CeUtils scrubber's channel — whose
`vas_pdb` **is** `Some` since §14.23 — is declined by the shell's CPU executor and falls
through to the forwarding plane.

★★★ **And only the declined path writes the completion.** The finishPayload release lives in
`kayfabe_rt::ceutils::run_submission`, which calls `cpu_ce::resolve_releases` and
`cpu_ce::write_resolved_completion` (`crates/kayfabe-rt/src/ceutils.rs:567,577`).
`try_ce_submission` is its **only** caller (`shim.rs:2193`). The forwarding path answers the
doorbell and advances no payload — which is why `iso1`'s line reads `SERVED` and never
`SERVED-LOCAL`. The guest then polls a word that never moves:

```text
memmgrMemSet(…, TRANSFER_FLAGS_PREFER_CE) @ mem_mgr.c:463   -> NV_ERR_TIMEOUT (0x65)
Assertion failed: pCeUtils->lastCompletedPayload == lastSubmittedPayload @ ce_utils.c:349
memmgrInitCeUtils(…) @ mem_mgr.c:526                        -> NV_ERR_TIMEOUT (0x65)
RmInitNvDevice: *** Cannot load state into the device
```

⊘ **This is §14.24's defect, reached by a second route.** §14.24 records the identical
sequence at boot `pub1_3e43e9a`, caused by `vas_pdb` becoming `Some`; the fix widened the
gate from *"can the core address it?"* to *"is there another executor?"*. `iso1` shows the
widened gate has the same hole through the **other** conjunct: turning the real plane on
makes the answer *"yes, another executor"* — and that other executor does not complete.
★ §14.24's own closing sentence is where it hides: *"A build that selects a real isolate
plane keeps the old routing exactly — a channel the core can address goes to the core."*
True, and **measured to be the wrong thing to want**: the core's route does not advance the
payload, so "keeps the old routing exactly" preserves a route that had never been exercised.

### 15.3 ⊘ What this refutes, in the project's own words

`execution_plane_increments.md` §14.8's rule was *"granting a VAS before an executor exists
converts a correct refusal into a doorbell reporting **Served** over work that never
happened."* With the executor now present, one word has to change: it is not the VAS and not
the executor, it is the **completion**. `iso1` is a doorbell reporting `Served`, with an
isolate live and a channel bound, over work whose result the guest never sees.
⇒ **Serving is worse than refusing until the payload advances**, and the refusal that
`fmb1` prints (`FwdFault::IsolateRetired`) was load-bearing without anyone having said so.

★ It also converges the two arms on one cause. `fmb1` hangs in `uvm_push_end_and_wait` →
`uvm_tracker_wait_for_entry` → `uvm_spin_loop` (sampled on
`uvm_gpu_tracking_semaphore_update_completed_value`, five samples in
`fmb1_319d29a_uvm_stack.log`); `iso1` times out on `pCeUtils->lastCompletedPayload`. Two
consumers, two failure shapes — an unkillable busy spin and a 4000 ms timeout — **one
missing thing**. And it is the plane `c_rust_trace_differential.md` already flagged as
having **no C oracle**, because the C *forges* completions: the forgery is why the C got past
this and this port does not.

### 15.4 The next increment, with its acceptance already written

Give the forwarding/isolate path the same completion tail the local path has — resolve the
`sem_releases` in the channel's own table and write them where the guest polls, then and only
then signal (`cpu_ce::write_completion`'s ordering discipline, which exists precisely to
avoid the C's `#12`).

Two oracles, both live boots, neither satisfiable by a test:
1. `KAYFABE_ISOLATES=real` reaches `nvidia-smi SMI_RC=0` — i.e. `RmInitAdapter` stops failing;
2. with the default plane, `cup2` **returns** instead of spinning in `uvm_spin_loop`.

⚠ And a third that must not regress: `fmb1`'s arm must keep its four `SERVED-LOCAL` lines.
A change that makes both paths complete but stops the local one running would be invisible to
oracle 1 and fatal to the milestone.

⊘ **One thing neither boot shows, stated rather than glossed:** whether the CE work
*executed on the host* under the real plane and only the release is missing, or whether
nothing ran at all. `served` is a statement about the RING, not about the copy, and no
`CeEvidence` is printed on this path. Establish that first — the fix differs.

### 15.5 ★★★ CORRECTION to §15.4 — the question it left open is ANSWERED, and the answer changes the increment

§15.4 closed with *"whether the CE work executed on the host under the real plane and only
the release is missing, or whether nothing ran at all … Establish that first — the fix
differs."* Established, from the call graph, and **nothing ran at all**. The fix does differ.

Three checks, each read at the site rather than inferred:

1. **The forwarding path is never given the ring.** `SharedDoorbell::ring` calls
   `self.device.doorbell(DOORBELL_TARGET_GPU, token, &[])`
   (`crates/kayfabe-qemu-raw/src/shim.rs:2071`) — an **empty** working set. Recovering which
   VAs a submission touches means parsing the ring, and that was deferred to E4.
2. **The isolate's doorbell verb is one store.** `HostRmBackend::ring_doorbell`
   (`crates/kayfabe-isolate-host/src/rm.rs:2315-2318`) is two statements: narrow the token to
   `u32`, `self.conn.doorbell(token)`. No pushbuffer parse, no method decode, no CE
   submission, no semaphore, no interrupt.
3. **The whole parse → execute → complete chain has NO production caller.** Both
   `submit_ring`s — the free function at `crates/kayfabe-fwd/src/lib.rs:4026` (`parse_pushbuffer`
   at `:4033`, `exec_ce` at `:4035`) and the method at
   `crates/kayfabe-rt/src/device.rs:1631` (`parse_pushbuffer` at `:1638`, `forward_ce` at
   `:1639`) — are called **only** from `tests/tests/e6_join.rs` and
   `tests/tests/e6_hw_join.rs`. `forward_ce` has exactly one caller and it is `submit_ring`.
   ⇒ `ce_copy` / `ce_copy_outcome` / `CeEvidence` are transitively unreachable from a
   doorbell, and `PushbufferOutcome::sem_releases` (`kayfabe-fwd/src/lib.rs:2644`) has no
   production consumer at all.

⇒ **`SERVED` on the real plane means: we rang a doorbell on a host channel into which the
guest's methods were never copied.** The host engine had nothing to run. E6's join is real
and is measured — but only through its tests; it is not wired to the doorbell.

★★★ **So the increment §15.4 named is the WRONG ONE, and dangerously so.** Adding the
completion tail to the forwarding path would advance the payload for work that never
happened — a forged completion, precisely what `mode2_forwarding_model.md` forbids and
exactly what the C did (`C: nvkvm_gpu_emul.c:4295-4340` forges the finishPayload at
`gpfifo_va + 0x8004`, plus the `0xFFF500/0xFFF508` backdoor at `:3778-3826` where a patched
guest hands QEMU the GPA and the payload to DMA in). ⊘ We would have re-implemented the
forgery, passed both of §15.4's oracles, and called it compute.

★ The real order is: **wire the ring first, complete second.** `try_ce_submission` is the
only doorbell path that reads the guest's ring, and it is also the only one that writes a
payload — that is not a coincidence, it is the same correctness argument twice. The
forwarding path needs `parse_pushbuffer` → `forward_ce` on the doorbell before a completion
tail is even meaningful.

⊘ And this is the **third** sighting of the pattern `a_declared_capability_reachable_from_nowhere`
records: a capability that is built, tested, and reachable from nothing. The E6 row's
acceptance (`CeEvidence::copied()`, §10) is satisfied by a test harness calling `submit_ring`
directly; nothing in that acceptance quantified over *"and a guest doorbell reaches it."*
⇒ Any future increment row whose acceptance is a predicate on a function must also state the
**caller** that a guest action reaches it through.

---

## 16. §15.6–§15.10 — THE UVM CHANNEL: three stacked walls removed, four boots, and the wall moved twice

`[measured 2026-08-09]`, five boots on the GA106 bench (`vh`, RTX 3060 `GPU-d0913685`, host
driver 580.159.04, guest 580.159.04 open and **stock**, probe set EMPTY, one device-opening
consumer per boot). Every claim below names the boot and the **binary's own** `kayfabe-rev`
stamp.

| boot | rev (stamped in the binary) | first doorbell refusal |
|---|---|---|
| `msr1` | `319d29a3…` | ⊘ **no census at all** — see §15.6 |
| `msr2` | `319d29a3…` | `[FwdFault::IsolateRetired]` … `vas=NONE-DECLARED ring=0x121010000` |
| `uvm1` | `b731e3c3…` | `[CeResolve::NoPublication]` `(hClient 0xc1d0000a, hVASpace 0xcaf00005)` |
| `uvm2` | `d0fbac0e…` | `[FwdFault::PushTooFragmented]` … `rng=V:0x20000 gp0=0x0…0 NOT-A-GP-ENTRY` |
| `scan1` | `00865a75…` | as `uvm2`, plus `scan=64/1024 declared, unread=0, nonzero=NONE` |

⚠ In all five, the CeUtils channel's **four `SERVED-LOCAL` lines on token 0x00010002 are
unchanged**. That was §15.4's third oracle and it is the one that must not regress.

### 16.1 ⊘ WHAT THIS REFUTES, INCLUDING THE BRIEF I WAS GIVEN

The rung I was handed was: *"make a guest doorbell reach the code that reads the guest's
ring … so `ce_copy` / `CeEvidence` become reachable **from a doorbell**"*.

**The submission `cuInit` walls on contains no copy at all.** `[src] ogkm-580`:
`channel_init` (`uvm_channel.c:2518-2572`) → `uvm_channel_end_push` (`:1492, 1512-1513`) →
`uvm_channel_tracking_semaphore_release` (`:1053-1063`) → `do_semaphore_release`
(`:1043-1051`) → `uvm_hal_volta_ce_semaphore_release` (`uvm_volta_ce.c:50-70`), which pushes
`SET_SEMAPHORE_A/_B/_PAYLOAD` and one `LAUNCH_DMA` carrying
`DATA_TRANSFER_TYPE_NONE | SEMAPHORE_TYPE_RELEASE_ONE_WORD_SEMAPHORE`.

⇒ There is nothing for a copy engine to copy. Reaching `ce_copy` from that doorbell would
have meant **manufacturing a transfer the guest never asked for** — the same class of
fabrication as the forged completion §15.5 stopped, one field over.

★ And the honest execution of a release-only launch is **not** a forged completion. A
forgery advances a payload for work that did not run; here the guest's own encoding says the
release **is** the work. The distinction is carried in the type
(`PushMethod::CeRelease`), not in a comment.

### 16.2 The three walls in front of it — wall 1 from `[measured 2026-08-09, boot `msr2_319d29a`]`, walls 2 and 3 from `[src] ogkm-580`

1. **ROUTING** (`msr2`). `ce_channel_facts` reported the VA space as the channel's *own*
   declared `hVASpace`. A UVM channel declares none — it inherits through CtxShare/TSG — so
   `try_ce_submission` returned `None` **before reading a byte of the ring**, while
   `Channel::vas_pdb`, derived from the *resolved* node, was `Some`. Two projections of one
   fact, disagreeing, with the weaker one load-bearing. ⊘ The next line in that very
   function already said *"two projections of one fact can disagree."*
2. **SUBCHANNEL** (`[src]`). UVM binds the CE class on subchannel **0** and issues every CE
   method on subchannel **4** (`uvm_maxwell_ce.c:29-37` + `uvm_push_macros.h:85, 101, 109` +
   `cla06fsubch.h:30`). Requiring the class on the firing subchannel refused **every method
   UVM emits**. RM's `channel_utils.c` binds and fires on one subchannel, which is why the
   CeUtils path worked and hid this completely.
3. **TRANSFER TYPE** (`[src]`). `DATA_TRANSFER_TYPE == NONE` decoded to `Opaque` under the
   sentence *"there is no copy to report"* — every clause true, conclusion false. There is
   no *copy*; there is still a **release**, and it is the entire content of the push.

### 16.3 ★★★★ A SATURATED LIST WAS THE DATA PATH (`uvm1` → `uvm2`)

`uvm1`'s own census refutes `uvm1`'s own refusal:

```text
VA-space page-directory publications: 12 total, 11 distinct, 0 UNDECODABLE
first doorbell refusal [CeResolve::NoPublication] no page-directory root was published
  for (hClient 0xc1d0000a, hVASpace 0xcaf00005)
```

`ceresolve::published_root` searched `GvasPubSnapshot::sample`, **capped at eight rows**, in
a boot that published **eleven**. ⇒ three VA spaces were unresolvable and the refusal's own
sentence — *"the guest published no page-directory root"* — was **false about the guest**.
`uvm2` proves it: with a separate uncapped-in-practice table the root resolves
(`0x4000/ap1/sh47`) and so do the ring and the finishPayload.

⊘ The type already said so: `sample` is documented *"capped"*, `distinct` *"the truth even
past the cap"*, and the one consumer that decides whether a channel can address anything
read `sample`. ★ Fifth sighting of `a_saturated_instrument_looks_exactly_like_absence` and
the **first where the saturated list is not an instrument**. A report that clips is a
report; a lookup that clips is a wrong answer.

### 16.4 ⧗ THE WALL AS IT STANDS, and the two candidates the next rung must separate

```text
[FwdFault::PushTooFragmented] { va: GpuVa(0x121010000), len: 0 }
  | c=0xc1d0000a vas=0xcaf00005 root=0x4000/ap1/sh47 ring=0x121010000
    rng=V:0x20000 fin=V:0x28004 gp0=0x0000000000000000 NOT-A-GP-ENTRY
    scan=64/1024 declared, unread=0, nonzero=NONE — every scanned entry is ZERO
```

The whole address chain resolves. Every one of the first 64 GPFIFO entries **reads
successfully** and every one is **zero**, so `run_submission` finds no work and refuses —
⊘ correctly: a doorbell that brought no readable entry is not served.

★ `unread=0` **refutes** the first candidate for indices 0..63: the guest did not submit at
an index we failed to read. UVM's first push is at `cpu_put = 0`.

⇒ **We are reading a store the guest never wrote.** And the shape of it is new: this ring
resolves to `V:` — **this device's emulated framebuffer** — while the CeUtils ring resolves
to `S:` (guest RAM, `[measured 2026-08-08, boot run_p35_84d857d]` `rng=S:0x2f2c3000`). UVM's
is the **first channel whose ring lives in video memory**, and the address plane's `Fb`
store is fed only by the BAR0 moving window and the GMMU-translated BAR2 window.

⧗ The unresolved question, stated so the next agent starts at it rather than at the top:
**which path did the guest write that ring through, and does it reach the `Fb` store?**
BAR1 is a `NVKVM_KIND_RESERVATION` whose fallback handler **discards the value**
(`nvkvm.c:494-501`) and whose shadow arm is a plain RAM memslot with no connection to the
framebuffer — so bytes written there are in neither store the address plane reads. ⊘ That is
a **candidate, not a finding**: `reservation_touches` has been counted since the memory plane
existed and reached **no report**, so no boot so far can say whether the guest touched BAR1
at all. §15.11 prints it; one boot then decides.

### 16.5 ★★★★ BAR1 REFUTED, and the ROOT is the anomaly — `[measured 2026-08-09, boots `bar1_6ba1bd5` / `vaspan_994bbdc`]`

```text
nvkvm: BAR1 (flat aperture): 3 accesses reached the DISCARDING fallback
nvkvm: framebuffer: 1040 reads / 337854 writes served through the BAR0 moving window
nvkvm: BAR2 (translated): 1018 reads / 286352 writes resolved through the GMMU
```

**Three.** §16.4 named BAR1 as a candidate and said to measure it before acting. It is
three, against 624 206 accesses through the two paths that *do* reach the framebuffer store.
⇒ **BAR1 is innocent**; the guest did not write its ring there.

`vaspan_994bbdc` then printed the pair `VasRoot` had carried and never shown:

```text
root=0x4000/ap1/sh47/va[0x100000000..0x11fffffff] ring=0x121010000
```

⊘ **The ring is outside the published range** — and that alone proves nothing, because the
CeUtils ring (`0x420064000`) is outside it too and its walk demonstrably moves real bytes.
`virtAddrLo..Hi` bounds which PDEs were copied to the server; `levels[0]` is the whole VA
space's root, so walking a VA beyond the copied range is legitimate.

★ What IS anomalous is the **root itself**:

| VA space | published root | walk lands in | works? |
|---|---|---|---|
| CeUtils (`0xc1e00006` / `0xa`) | `0x2efa9c000` | `S:` guest RAM | ✅ bytes moved |
| the global arm (`0x0` / `0x0`) | `0x2efbae000` | — | — |
| **UVM (`0xc1d0000a` / `0xcaf00005`)** | **`0x4000`** | `V:0x20000` | ❌ all zeros |

Every root this boot prints sits around `0x2efa_xxxx`; the refusing channel's is **16 KiB**
into the framebuffer. A root at that address, walked, produces plausible-looking leaves that
read back as zeros — which decodes as *"the ring is empty"* rather than faulting. That is
`two_encodings_agreeing_on_the_first_values` in its purest form.

### 16.6 ⧗ THE NEXT RUNG, and the instrument gap §16.3 opened in front of it

**Print the publication row(s) for `(0xc1d0000a, 0xcaf00005)` — the whole 184-byte body, all
four levels — and compare them with a row that works.** Three outcomes, three different
fixes: the row is a real root RM had not yet backed; the row is a *stale* publication that
last-write-wins picked over a later real one; or the body was decoded from the wrong arm.

⊘ **That row prints nowhere today.** §16.3 made the *lookup* complete
(`GvasPubSnapshot::roots`, 256 entries) and left the *report* clipped at eight
(`GVAS_PUBLICATION_SAMPLE_MAX`), and this pair is past the cap — so the one row that decides
the wall is structurally invisible in every boot log. Widening what the census shows, or
printing the resolved row inside the addressing probe, is owed **before** anything is changed
about the walk.

⚠ Do not skip to widening the `Fb` store. If `reservation_touches` is **zero**, BAR1 is
innocent and the bytes went somewhere else entirely — and a fix aimed at the wrong store
would be an invented answer that happens to make a refusal disappear.

### 16.7 §16.6's INSTRUMENT, built — and three report caps found on the way, one of them a false diagnosis

The rung §16.6 left was *"print the publication row for `(0xc1d0000a, 0xcaf00005)` — whole
body, all four levels — beside a working row"*, and it said the report had to be widened
**before** anything is changed about the walk. Nothing about the walk, the address plane or
the executor is touched here.

**What now prints, and where.**

1. **The row the LOOKUP chose, on the refusal line itself.** `publication_row`
   (`crates/kayfabe-qemu-raw/src/shim.rs`) reads
   `GvasPubSnapshot::roots` — the *same* map `ceresolve::published_root` reads, not a second
   projection — and formats `arm`, `count`, `num_levels`, `pageSize`, the subdevice pair,
   `virtAddrLo..Hi` and **every declared level** as `Ln=<phys>/sz<size>/ap<aperture>/sh<shift>`.
   Each field separates one of §16.6's three outcomes: the **arm** decides *decoded from the
   wrong arm*, the **count** decides *a stale publication last-write-wins picked*, and
   `L0.size` plus `L1..L3` decide *a real root RM had not yet backed*. An **absent** row
   prints `ABSENT-FROM-ROOT-TABLE(n rows, m REFUSED-BY-CAP)`, because §16.3 is the boot where
   *"the guest published none"* and *"our table dropped it"* were the entire bug.
2. **The census sample, 8 → 32 rows.** `GVAS_PUBLICATION_SAMPLE_MAX` and
   `KAYFABE_GVAS_PUBLICATION_SLOTS`. `[measured 2026-08-09, boots `uvm1_b731e3c` …
   `vaspan_994bbdc`]` every one published **12 total, 11 distinct** and printed the first
   **eight**; the refusing pair sat past the cap in all six. ★ De-duplication keys on the
   **whole body**, so a VA space re-published with different levels takes two rows here while
   `roots` keeps only the later one — the table says which root won, the sample says what it
   beat, and that pair is what makes a stale publication visible at all.

**⊘ And two more bounded collections were found by auditing the instrument I was about to
load up** — standing rule (b), *audit every bounded collection for which side of the boundary
it sits on*:

3. **The doorbell sentence buffer was SATURATING SILENTLY.** `DOORBELL_REFUSAL_LEN` was
   **448**; `[measured 2026-08-09, boot `vaspan_994bbdc`, rev `994bbdc10`]` the refusal was
   **292 bytes** (⊘ this said 262; re-measured from the sentence after the refusal kind in
   `traces/guest_boots/run_vaspan_994bbdc_qemu.log` — the conclusion sharpens, since
   292 + ~180 is ~472 into a 448-byte array), and the four
   `PdeLevel`s this rung appends are ~180 more — landing the sentence *on* the cap. The copy
   was a bare `s.len().min(LEN)`, so a clipped sentence and a complete one produce **the same
   log line**, and the levels are at the END. Now 1024, and `copy_sentence` stamps a literal
   `[CLIPPED, sentence was N bytes]` tail on every one of the three sentence buffers, so
   saturation is a statement instead of an absence.

4. ★★★★ **THE WALL HAS BEEN REPORTED UNDER THE WRONG NAME FOR FOUR BOOTS.** `[measured
   2026-08-09, boots `uvm2_d0fbac0`, `scan1_00865a7`, `vaspan_994bbdc`]` the UVM channel's
   refusal reads:

   ```text
   [FwdFault::PushTooFragmented] PushTooFragmented { va: GpuVa(4848680960), len: 0 }
   ```

   `PushTooFragmented` means *"one GPFIFO range cut into more address-table spans than
   `MAX_PUSH_SPANS`"* — **a bound of ours**, raised in `kayfabe_fwd::pushbuffer_ranges`. The
   `[src]` fact is the opposite: `ranges.is_empty()` at `kayfabe-rt/src/ceutils.rs`, i.e.
   **no range existed to fragment**, because the ring read back as zeros (`scan=64/1024
   declared, unread=0, nonzero=NONE`). Four boot logs therefore named a limit that was never
   reached, and the only hint was `len: 0`.

   ⇒ New variant `FwdFault::RingBroughtNoEntry { ring_va, index, entries }`, carrying the
   index so *"the cursor ran past the end"* and *"index 0 of a ring the guest never wrote"*
   are two readings of it rather than one. ⊘ *Refuse by name* is a claim that the name is
   **TRUE**, not that there is one — a wrong name is a specific, actionable, **false**
   diagnosis, which is worse than an unnamed refusal. The test that pinned the old name
   asserted, in its own words, *"the empty ring is named"*; it was named, and named something
   else.

**⊘ What this rung deliberately does not do.** No walk, no aperture, no `Fb` store, no
executor, no completion. Nothing is emitted that the guest did not ask for. The doorbell is
still refused, and after §16.7 it is refused **by its own name** with the deciding row
attached.

### 16.8 ★★★★★ BOOTED `row1_44b7d69` — the row PRINTS, and the eleven publications split into TWO FAMILIES

`[measured 2026-08-09, boot `row1_44b7d69`, binary stamped `kayfabe-rev:44b7d69e3…`]`, default
plane, probe set EMPTY, one device-opening consumer. §16.7's instrument works: all **11**
publications print (the refusing pair is row **11 of 11** — it really was past the eight), the
refusal now carries its own row, and it is named `RingBroughtNoEntry` rather than
`PushTooFragmented`.

```text
[FwdFault::RingBroughtNoEntry] { ring_va: GpuVa(0x121010000), index: 0, entries: 1024 }
  | c=0xc1d0000a vas=0xcaf00005 root=0x4000/ap1/sh47/va[0x100000000..0x11fffffff]
    ring=0x121010000 rng=V:0x20000 fin=V:0x28004 gp0=0x0…0 NOT-A-GP-ENTRY
    scan=64/1024 declared, unread=0, nonzero=NONE — every scanned entry is ZERO
    row=arm0x90f10106 x1 lv4/6 pgsz0x200000 sd0x0/0 va[0x100000000..0x11fffffff]
        L0=0x4000/sz0x20/ap1/sh47 L1=0x5000/sz0x1000/ap1/sh38
        L2=0x6000/sz0x1000/ap1/sh29 L3=0x7000/sz0x1000/ap1/sh21
```

**Two of §16.6's three outcomes are REFUTED by that line alone**: `arm0x90f10106` is the
client VA-space arm, so it was **not decoded from the wrong arm**; `x1` means the pair was
published **once**, so it is **not a stale publication last-write-wins picked over a later
one**. The row is well-formed and singular — `lv4/6`, `pgsz0x200000`, root `sz0x20`, shifts
47/38/29/21 — **identical in every field to a working VA space's except the addresses**.

#### The split, from the same log

| `(hClient, hObject)` | L1 | L2 | L3 | walk |
|---|---|---|---|---|
| `0xc1e00006 / 0xa` (CeUtils) | `0x2efa9b000` | `0x2efa9a000` | `0x2efa99000` | ✅ `S:` guest RAM, bytes moved |
| `0xc1d00008 / 0xcaf00000` | `0x2efa7b000` | `0x2efa7a000` | `0x2efa79000` | — |
| `0xc1d0000c / 0x5c000008` | `0x1000` | `0x2000` | `0x3000` | — |
| **`0xc1d0000a / 0xcaf00005`** | **`0x5000`** | **`0x6000`** | **`0x7000`** | ❌ `V:0x20000`, all zeros |

★★★ **Two families, and the difference is in the KIND of address.** The working rows carry
real framebuffer physical addresses (`~0x2efa_xxxx` ≈ 11.7 GiB, consistent with this GA106's
FB) and **descend**. The failing rows carry tiny **ascending** consecutive 4 KiB pages
starting near zero — and the two of them are **contiguous with each other**: `0x0000–0x3fff`
then `0x4000–0x7fff`, four pages each. That is the signature of **offsets into one shared
buffer handed out by a bump allocator**, not of physical pages.

⇒ **Our walk reads them as framebuffer physical addresses.** It therefore descends
successfully — the numbers are page-aligned and in range — lands at `V:0x20000`, and reads an
unwritten page, which decodes as *"the ring is empty"* rather than faulting. That is
`two_encodings_agreeing_on_the_first_values` in its sharpest form yet: these rows agree with
the working ones on **`levels`, `pageSize`, `aperture`, every `pageShift` and the whole
`virtAddr` range** — every field except the one that means something different.

⊘ **What the base is, is NOT established, and must not be guessed.** Named candidates, in the
order they should be checked: offsets into a **reserved page-table pool** RM allocated for
these VA spaces; offsets relative to a **WPR/heap base** the guest knows and we do not; or
guest-**physical** addresses of a sysmem pool, in which case `aperture 1` is the field that
lies. ⚠ Note the aperture is `ap1` (VIDEO) on **all eleven** rows, so it does not separate the
families and cannot be used as the discriminator.

⧗ **The next rung**, and it needs no new plumbing: dump the 32 bytes our framebuffer
holds at `0x4000` and the 4 KiB at `0x5000`, and compare them with `0x2efa9b000`. If the low
addresses hold plausible PDEs, they are a real pool we have the bytes for and only the base is
missing; if they hold zeros or unrelated data, the walk has been descending noise and every
address downstream of it — including `V:0x20000` — is a coincidence.

### 16.9 ★★★★★ BOOTED `fbd1_f760a4b` — the tables at `0x4000` ARE REAL, and "the base we lack" is REFUTED

`[measured 2026-08-09, boot `fbd1_f760a4b`, both artifacts stamped
`kayfabe-rev:f760a4b3b8395abb…` (archive **and** QEMU binary), GA106 bench `vh`, stock
580.159.04 guest, probe set EMPTY, one device-opening consumer]`. §16.8's rung, answered on
its first boot:

```text
fbL0@0x4000       = 0205000000000000 00000000…  nz2/4096
fbL1@0x5000       = 0206000000000000 00000000…  nz2/4096
ctl=0x0/0x0
ctlL0@0x2efbae000 = 02adfb2e00000000 00000000…  nz4/4096
ctlL1@0x2efbad000 = 02acfb2e00000000 00000000…  nz4/4096
```

Decoded little-endian, entry 0 of each:

| dumped page | entry 0 | flag byte | `(entry >> 8) << 12` | the publication's next level |
|---|---|---|---|---|
| `0x4000` (failing L0) | `0x0000_0000_0000_0502` | `0x02` | **`0x5000`** | `L1 = 0x5000` ✅ |
| `0x5000` (failing L1) | `0x0000_0000_0000_0602` | `0x02` | **`0x6000`** | `L2 = 0x6000` ✅ |
| `0x2efbae000` (control L0) | `0x0000_0000_2efb_ad02` | `0x02` | **`0x2efbad000`** | `L1 = 0x2efbad000` ✅ |
| `0x2efbad000` (control L1) | `0x0000_0000_2efb_ac02` | `0x02` | **`0x2efbac000`** | `L2 = 0x2efbac000` ✅ |

★★★★★ **Both families are the SAME ENCODING and both are SELF-CONSISTENT with their own
publication.** Identical flag byte `0x02`, the page-frame number in the bits above it, and
each entry points at exactly the next level the publication declared. The `nz` census says
the same thing from the other side: `2` non-zero bytes in the failing pages and `4` in the
control's — one entry written per page in **both**, the difference being only how many bytes
the frame number needs.

⇒ ⊘⊘ **BOTH ARMS OF §16.8's RUNG ARE WRONG, and that is the finding.** It offered *plausible
PDEs ⇒ a real pool whose **base we lack*** versus *zeros ⇒ the walk has been descending
noise*. The measurement is a **third** answer:

- **NOT noise.** These are real, well-formed page-directory entries.
- **NOT a base we lack.** They are at the base we are already using — the guest wrote them
  into our framebuffer at exactly the offsets it published, and we read them back, chained
  correctly, with no translation missing. A base that were wrong could not produce a chain
  that agrees with the publication at every level.

⇒ **The two families are not two kinds of address after all.** `~0x2efa_xxxx` and `0x4000`
are both framebuffer physical addresses, both written by the guest, both read by us. The
difference between them is **where the guest's allocator put the tables**, which is not a
defect and not a discriminator. ⊘ §16.8's *"the signature of offsets into one shared buffer
from a bump allocator"* is refuted as a claim about the address's KIND; it remains an
accurate description of their *layout*, which is now known to be irrelevant.

★ Corollary, and it is what makes the wall smaller: the address plane, the aperture decode,
the root attribution and the descent are all **exonerated for the first two levels**. The
refusal is unchanged and correctly named:

```text
[FwdFault::RingBroughtNoEntry] { ring_va: GpuVa(0x121010000), index: 0, entries: 1024 }
  … rng=V:0x20000 fin=V:0x28004 scan=64/1024 declared, unread=0, nonzero=NONE
```

⧗ **WHERE THIS PUTS THE NEXT RUNG, stated so it is decidable and needs no new plumbing.**
The dump above is **entry 0** of each page, and entry 0 is *not* the entry the ring's walk
consumes. For `va = 0x121010000` the level-2 index is `(0x121010000 >> 29) & 0x1ff = 9`, not
0 — and `nz2/4096` proves the level-1 page has **exactly one** non-zero entry, which is entry
0. ⇒ Dump **the entry the walk actually indexes at each of the four levels**, for the ring VA
and for a working channel's ring VA, in both families. That separates *"the leaf genuinely
names `V:0x20000` and the guest never wrote the ring there"* from *"we index the wrong slot
and `V:0x20000` is what the wrong slot happens to hold"*.

⚠ And note the fact §16.5 raised, dismissed, and which is now the only structural asymmetry
left: the ring VA `0x121010000` is **outside** the publication's own
`virtAddrLo..Hi = 0x100000000..0x11fffffff`. That range is what
`COPY_SERVER_RESERVED_PDES` reserves; the PDEs covering anything outside it are written by
the guest through a different path. ⊘ **A candidate, not a finding** — the CeUtils ring is
outside its range too and demonstrably moves bytes — and it is a candidate the next rung's
per-level dump measures rather than assumes.

⊘ **This rung changed nothing the guest can see.** `fb_peek` reads our own store; no walk, no
`Demand`, no allocation (`SparseFb::read` does not fault a page in, so the residency counter
this boot reports — `368640 bytes` — is unperturbed by the dump). No completion, no payload,
nothing emitted the guest did not ask for.

### 16.10 ★★★★★ BOOTED `wlk1_dcd096c` — the walk is COMPLETE AND CORRECT, and it lands on an EMPTY page

`[measured 2026-08-09, boot `wlk1_dcd096c`, archive **and** QEMU binary both stamped
`kayfabe-rev:dcd096c62afd5c2e…`, GA106 bench `vh`, stock 580.159.04 guest, probe set EMPTY,
one device-opening consumer]`:

```text
walk: L0@0x4000[ch1 lf0 sp0 inv3]    =PDE@0x0         ->0x5000/Vidmem
      L1@0x5000[ch1 lf0 sp0 inv511]  =PDE@0x0         ->0x6000/Vidmem
      L2@0x6000[ch2 lf0 sp0 inv510]  =PDE@0x120000000 ->0x8000/Vidmem
      L3@0x8000[ch2 lf8 sp0 inv247]  =PDE@0x121000000 ->0xa000/Vidmem
      L4@0xa000[ch0 lf2 sp0 inv30]   =LEAF@0x121010000->0x20000/Vidmem/sz0x10000
```

★★★ **The leaf's own VA base is `0x121010000` — the ring address exactly — and it maps to
`V:0x20000` with a 64 KiB page.** That is byte-identical to `resolve`'s independent answer
printed in the same sentence (`rng=V:0x20000`). ⇒ The two projections **agree**, which is
what §16.2 wall 1 taught us to check rather than assume.

⊘⊘ **THIS REFUTES THIS RUNG'S OWN PREMISE, WHICH WAS MINE.** §16.9 argued *"entry 0 is not
the entry the walk consumes — for `va = 0x121010000` the level-2 index is
`(va >> 29) & 0x1ff = 9`"*. Both halves are wrong:

- **The tree is FIVE levels, not four.** The publication declares `lv4` (shifts 47/38/29/21)
  and the descent runs `L0…L4`. The published levels are only the **top four** of the
  format's five, so any index arithmetic done against the publication's shifts describes a
  different tree. ⊘ The trace does no arithmetic — it selects on the `vabase`
  `decode_page` stamps — which is the only reason it is right where my reasoning was not.
- **The pages beyond the published range ARE populated and ARE found.** `L2` carries two
  children, `L3` two children and eight leaves, `L4` two leaves. The guest wrote them, we
  read them, the descent follows them.

⇒ ★★★★ **The address plane is now exonerated END TO END for this channel.** Root
attribution, aperture decode, the five-level descent, the leaf's page size and the final
translation are all correct. §16.5's *"a root at `0x4000`, walked, produces plausible-looking
leaves"* is refuted: they are not *plausible-looking*, they are **right**.

⇒ The wall is one step further out and is now a single sentence: **the guest's own page
tables say its GPFIFO ring lives at framebuffer offset `0x20000`, and our framebuffer has
never had a byte written there.** The page tables reached our store (§16.9 read them back);
the ring contents did not.

### 16.11 ⊘⊘⊘ §16.5's "BAR1 IS INNOCENT" DOES NOT FOLLOW FROM THE NUMBER IT CITES

`[src]` `qemu/hw/misc/nvkvm/nvkvm.c:71-73`, the enum's own comment:

> `NVKVM_KIND_RESERVATION` — *"A pure-MMIO reservation the archive **shadows with its own
> slots**. Its callbacks are **not reached in normal operation**; if one fires, the shadow is
> missing."*

and `:485-490`, the counter's own comment: *"Reached only where the archive's slot does not
cover the range … **Counted so 'the shadow is missing' is a number** rather than a
suspicion."*

⇒ `reservation_touches` counts **how often the shadow was ABSENT**. It does **not** count
guest accesses to BAR1 — by the code's own statement, an access the shadow serves never
reaches the callback and is therefore never counted. §16.5 read the number the other way:

> *"**Three.** … It is three, against 624 206 accesses through the two paths that do reach
> the framebuffer store. ⇒ **BAR1 is innocent**; the guest did not write its ring there."*

⊘ **That inference is unsound.** `3` is consistent with the guest never touching BAR1 **and
equally consistent with the guest writing gigabytes through it into a shadow memslot** — the
two are indistinguishable in that counter, which is the exact shape
`a_saturated_instrument_looks_exactly_like_absence` names, inverted: not a full list read as
complete, but a **miss counter read as a hit counter**.

⚠ **This is a CANDIDATE, not a finding, and it is stated as one.** Nothing here measures that
a BAR1 shadow slot exists in this build, nor that it holds the ring bytes. What is measured
is that the *evidence which cleared BAR1 cannot bear that weight*. It matters because
`:1431` already records what the shadow arm is — *"a plain RAM slot with **no connection to
the framebuffer**"* — so bytes written there are in **neither store the address plane
reads**, which is precisely the shape §16.10 now requires: correct page tables naming
`V:0x20000`, and nothing ever written to `V:0x20000`.

⇒ ⧗ **The next measurement, and it is decidable:** report whether a BAR1 shadow slot is
installed, its extent, and how many of its bytes are non-zero — beside the existing
`reservation_touches`. A non-zero shadow is guest data in a store nothing reads; an absent
shadow restores §16.5's conclusion by a route that actually supports it.

★ **The general lesson, and it is new in kind for this project.** Every instrument failure so
far has been a *report* that clipped, a *name* that was wrong, or a *list* that saturated.
This one is a counter that is **correct, well-documented, and measures a different thing than
the sentence built on it**. Reading its own comment was enough to refute a committed
conclusion — no boot required. ⇒ Before citing a counter, read what its **increment site**
says it counts, not what its **name** suggests.

### 16.12 ★★★★★ BOOTED `bar1_03a679f` — BAR1 is innocent AFTER ALL, and the ring page is EMPTY TO THE BYTE

`[measured 2026-08-09, boot `bar1_03a679f`, archive **and** QEMU binary both stamped
`kayfabe-rev:03a679fd566e7032…`, GA106 bench `vh`, stock 580.159.04 guest, probe set EMPTY]`.

**Answer 1 — ⊘ my own §16.11 candidate is REFUTED, and by the route that supports it:**

```text
nvkvm: BAR1 (flat aperture): 3 accesses reached the DISCARDING fallback, and NO shadow is
installed (window-size=0), so this IS a complete census of BAR1 traffic.
```

`[src]` the chain, which is what made this decidable without guessing: the shadow is
installed at exactly one site (`nvkvm.c:1204`, gated `if (s->window_size != 0)`);
`window_size` is the `window-size` property and `:2294` defaults it to **0**;
`scripts/bench/boot_nvkvm.sh:24` sets `bar1-size` and `bar2-size` and **never** `window-size`;
and the install's own `reservation of 0x… bytes installed` line appears in **zero** captured
boots. ⇒ Every BAR1 access reaches the counted fallback, `3` is a **complete census**, and
§16.5's *"BAR1 is innocent"* is **correct**.

★★★★ **But it was correct by luck of configuration and said so nowhere.** The identical line
would have read *"the guest barely touched BAR1"* about a boot in which it wrote gigabytes,
the moment anyone set `window-size`. ⇒ **A number whose meaning depends on a condition must
print that condition.** The report now branches, and the shadowed arm says outright that the
count *"is NOT a census of BAR1 traffic and must not be cited as one"*. That is the general
fix for the class §16.11 named — not *"read the increment site"* as advice, but a counter
that carries its own precondition into the log.

**Answer 2 — the wall, pinned to the byte:**

```text
fbL0@0x4000   =0205000000000000…  nz2/4096      ← the guest's page tables ARE in our store
fbL1@0x5000   =0206000000000000…  nz2/4096
fbRING@0x20000=0000000000000000000000000000000000000000000000000000000000000000  nz0/4096
```

⇒ ★★★★★ **`nz0/4096`. Not one non-zero byte in the ring's whole page.** The address the
guest's own five-level walk names for its GPFIFO ring has **never been written** in this
device's framebuffer.

★ And the pair is the finding, not either half. **Both** addresses are vidmem, **both** are
reached by the same two write paths (BAR0 moving window: 337 854 writes; BAR2 translated:
286 352 writes), and **the page-table writes landed while the ring writes did not.** So this
is not *"our framebuffer is not written"* — it demonstrably is, 337 854 times — it is *"these
particular bytes never arrived"*.

⇒ ⧗ **THE NEXT MEASUREMENT, and it needs no new plumbing beyond a census.** `[src]`
`SparseFb` holds its written pages in a `HashMap` and reports only a total
(`[measured, this boot]` `resident 368640 bytes` = 90 pages).
**Report WHICH framebuffer frames are resident** — the count, the extent, and whether the
ring's frame is among them. Three outcomes, three different fixes: the ring's frame is absent
entirely (nothing ever addressed it); it is present but zero (something addressed it and
wrote zeros); or the resident set clusters somewhere that says which *path* the ring writes
took instead.

⊘ **Do not let "the guest wrote it somewhere else" become the default hypothesis.** Three
things are settled by named boots — BAR1's innocence by `bar1_03a679f` above, the walk's
correctness by `wlk1_dcd096c` (§16.10), the page tables' arrival by `fbd1_f760a4b` (§16.9) —
so the remaining candidates are about *ordering and timing* as much as about *routing*, and
nothing here separates them yet.

### 16.13 ★★★★★ BOOTED `res1_fc21926` — the ring's frame was NEVER WRITTEN, and two of the three outcomes are refuted

`[measured 2026-08-09, boot `res1_fc21926`, archive **and** QEMU binary both stamped
`kayfabe-rev:fc21926c9a8e405e…`, GA106 bench `vh`, stock 580.159.04 guest, probe set EMPTY]`:

```text
framebuffer residency: 90 page(s) spanning [0x0..0x2efbd5000] — 3079126 page(s) of extent,
  so the resident set is SPARSE.
fbL0@0x4000    …  nz2/4096  resY
fbL1@0x5000    …  nz2/4096  resY
ctlL0@0x2efbae000 … nz4/4096 resY
fbRING@0x20000 =0000…0000  nz0/4096  resN-NEVER-WRITTEN
```

⇒ ★★★★★ **Outcome 1.** The ring's frame is **not resident**: nothing ever aimed a write at
framebuffer offset `0x20000`. It is not *"written with zeros"* — **outcome 2 is refuted**,
and that distinction was invisible to every boot before this one because
[`FbStore::read`] returns *zero and `Ok`* for a page nobody wrote.

⊘ **Outcome 3 — "written and then erased by `device_reset`" — is refuted by the same line.**
`device_reset` clears the *whole* store, and the page-table pages at `0x4000`/`0x5000` are
**resident**. A reset that erased the ring would have erased them too. ⚠ It is refuted only
up to ordering: a reset between the ring write and a *later* page-table write would survive
this argument, and nothing here excludes that. A first-writer sequence number would; the
byte census and the residency bit both cannot.

★ And the resident set is **90 pages spread across the whole 11.7 GiB aperture** — sparse,
not clustered. So this is not a store that only ever received one region's worth of traffic.

⇒ **The wall, restated as narrowly as the evidence now allows:** every reachable vidmem write
path lands in the one store the walker reads (`ring_write_path_map.md`), that store received
337 854 BAR0-window writes and 286 352 BAR2 writes with **zero refusals**, the guest's own
five-level walk names `0x20000` for its ring, and **no write was ever aimed there**.

### 16.14 ⊘⊘ `translated-window drops 0r/0w` IS A VACUOUS ZERO — `[src]`, re-derived here

`[src]`, checked rather than taken on report: `fb_window_reads` increments at
`plane.rs:2005` and `fb_window_writes` at `:2242`; both sit in the
`WindowRefusal::NoAddressModel` arm; that refusal is returned **only** for
`FbWindow::FbAperture` (`:2110`), i.e. BAR1; and BAR1 registers with
`nvkvm_reservation_ops` and never crosses the shim seam — `kayfabe_shim.h` has **no BAR1
spelling at all** (`KAYFABE_BUS_BAR_REGS 0u`, `KAYFABE_BUS_BAR_INST 2u`).

⇒ The pair **cannot move in this build**, and the report's own sentence claimed it counted
*"the two GMMU-translated windows"* — wrong twice: it counts **one** (BAR2's refusals go to
`bar2_faults`), and that one is unreachable. ⊘ Its zero was **true, unfalsifiable, and read
as evidence** that no translated window ever dropped a byte. The `pgrep -x
qemu-system-x86_64` shape exactly: a check that cannot fail always passes.

★ Fixed by the rule §16.12 established rather than by deleting the counter: the report now
**says the zero is vacuous and why**, and a non-zero value prints a warning that the
counter's own precondition has changed and every BAR1 conclusion in that boot needs
re-reading. **A counter must carry its own precondition into the log.**

---

## §16.16 / §16.17 BOOTED `s16_5fcd259`, `s17_e8fde62` — ★★★★★ THE RING WRITE ARRIVES ON **BAR1** AND WE DESTROY IT

`[measured 2026-08-09, boots `s16_5fcd259` and `s17_e8fde62`, archive AND QEMU binary both
stamped with the boot's own revision and asserted EQUAL before booting, GA106 bench `vh`,
stock 580.159.04 guest]`

### The three writes, verbatim from `run_s17_e8fde62_qemu.log`

```
BAR1 access log: 3 of 3 access(es) recorded in full — complete
  BAR1[0] WRITE off=0x90000 size=4 val=0x20000000
  BAR1[1] WRITE off=0x90004 size=4 val=0x2801
  BAR1[2] WRITE off=0xa008c size=4 val=0x1
```

Decoded — arithmetic shown so it can be checked rather than believed:

| | value | decode |
|---|---|---|
| `BAR1[0]`+`BAR1[1]` | qword `0x0000_2801_2000_0000` | ★ **a valid GPFIFO entry**: `gpu_va = 0x1_2000_0000`, `len = 10 dwords = 40 bytes`, `subroutine=0`, `sync_wait=0` |
| `BAR1[2]` | `off = 0xa008c`, `val = 1` | ★ **`GP_PUT = 1`** — `0x8c` is `USERD_GP_PUT` (`35*4`, `kayfabe-abi/src/submit.rs:1231`), USERD base `0xa0000` |

⇒ This is `internal_channel_submit_work` **exactly as `ogkm-580:
kernel-open/nvidia-uvm/uvm_channel.c:984-1015` writes it**: `set_gpfifo_entry` through a
dereferenced CPU pointer, `mb()`, then `write_gpu_put`. The 8-byte entry arrives as two 4-byte
stores because that is how the guest's CPU issues it.

⊘⊘ **`nvkvm_reservation_write` (`qemu/hw/misc/nvkvm/nvkvm.c`) does `(void)val;`. All three
writes are destroyed.** The bytes cease to exist; no store in the address plane ever sees them.

### ★ This explains every number the campaign has collected, with nothing left over

- the ring page is **`resN-NEVER-WRITTEN`** — of course: the write never reached a store, so
  `SparseFb` never created the frame;
- **BAR2: 286 352 writes, 0 refusals** — consistent, because the ring never went near BAR2;
- **BAR1: 3 accesses, complete census** — and 3 is *exactly* one entry (two halves) plus one
  `GP_PUT`. The number that looked like "the guest barely touched BAR1" was the whole
  submission handshake;
- the page tables at `0x4000`/`0x5000` **are** resident, `byBAR2#83`/`#84` — they arrive on an
  aperture we do service;
- **`GP_PUT` DID advance, 0 → 1.** ⊘ So "nothing was ever submitted" is **refuted**: work was
  submitted, and we threw the submission away.

### ⊘ What this REFUTES, including two of my own instrument's bounds

- ⊘ **The memslot / missed-mapping hypothesis is dead.** The trap-status table asks the
  *hypervisor*: all four regions resolve to **our own** region and every one is **`IO —
  TRAPPED`**. No RAM row, no shadow (`window-size=0`), nothing overlaying a BAR except QEMU's
  own `msix-table` inside the MSI-X container. ⇒ We saw the writes. We discarded them.
- ⊘ **The scan cap was NOT the cause, and lifting it proved so.** `scan=1024/1024 declared
  (COMPLETE: every declared entry was read), nonzero=NONE` — the *whole* ring is zero, not the
  first 6.25 %. ★ The cap was still a real blindness and still had to go; it simply was not
  this bug. Both statements are worth keeping.
- ⊘ **The one-page dump was NOT the cause either.** All three pages of the allocation —
  `fbRING[p0]@0x20000`, `fbRING[p1]@0x21000` and `fbFIN@0x28004` — read
  `resN-NEVER-WRITTEN`. ★ This is the owner's **structure test** answered: three addresses in
  one allocation, all pure zero, none ever created.
- ⊘ **`userdOffset[0] = 0`**, so the documented silent-stall mechanism (a non-zero offset making
  hardware see `GP_PUT == GP_GET` forever) is **not** in play.
- ★★ **`dec=NONE`** — the channel declares **no `hVASpace` of its own**. The VA space the walk
  uses is entirely **derived** through CtxShare/TSG. That remains true and remains unaudited;
  it is simply no longer the leading candidate.

### ★★★ Where the fix goes — and where it must NOT go

⊘ **Do not "stop discarding" by giving BAR1 a store of its own.** BAR1 is an *aperture onto
framebuffer memory*: a write at BAR1 offset X must land in **the same `SparseFb`, at the same
framebuffer address, that the GMMU walk reads**, and a read-back through *either* aperture must
agree. Anything else recreates the self-consistent-wrong-store defect one aperture over.

★ The shape of the correct fix is already half-present and was never connected:

- `crates/kayfabe-device/src/bar2.rs:131-136` — `BarPdes` **already carries a `bar1` field**,
  documented *"The framebuffer aperture's root, if the guest has published one."* The boot
  reports **`roots published 3`**;
- `crates/kayfabe-device/src/plane.rs` — `window_phys` answers `FbWindow::FbAperture` with
  `Err(WindowRefusal::NoAddressModel)`, which is the *only* reason BAR1 registers as a
  discarding reservation at all;
- `qemu/hw/misc/nvkvm/nvkvm.c` — BAR1's row is `NVKVM_KIND_RESERVATION`; BAR2's was changed to
  `NVKVM_KIND_TRAP` at `#149` for **precisely this reason**, and its comment already states the
  general rule: *"this window is GMMU-translated … so it cannot be shadowed by a memslot the
  way a flat reservation can."*

⇒ BAR1 needs the treatment BAR2 got at `#149`: a real address model, translating through the
BAR1 root the guest publishes, writing into the one `SparseFb`. ⚠ And `Bar0Window::target()` —
decoded and never consulted — is the standing warning about doing half of this.

### ★ The instruments, and what each one was worth

- **first-writer census**: `PRAMIN 21 / BAR1 0 / BAR2 68 / EXEC 1 / UNATTRIBUTED 0`. ★
  `UNATTRIBUTED 0` is the line that makes the rest readable — full instrumentation coverage, so
  the other four are a census of the guest rather than of our own gaps. ⊘ At tree `e394b69` this
  same census would have read `UNATTRIBUTED 90`, because nothing called `write_tagged`.
- **GPFIFO forward search**: 29 of 90 resident pages carry entry-shaped qwords; best
  `0x2efa81000`, 256 shaped entries, `byBAR2`. ⚠ **Not yet a finding** — a page of PTEs scores
  on this sieve too, and the sieve was built to exclude noise, not to identify rings. It is
  reported as a score and must not be read as "the ring is at `0x2efa81000`".
- **trap-status table**: the one instrument that asked something other than ourselves, and the
  one that closed the owner's hypothesis.
- **BAR1 access log**: the decisive one, and it exists only because the *count* was consistent
  with two opposite readings. ★ A number that cannot discriminate is not evidence; the
  addresses and values were.

---

## 16.23 ⚠ OPEN — the completion interrupt is raised BEFORE the fact that explains it

**Status: measured by audit (`026374c`), NOT yet fixed, and deliberately not folded into
§16.21.** Recorded here so it is actionable rather than re-derived.

### The two raises, and their order

A served CPU-CE doorbell produces **two** signals, in this order:

1. `kayfabe-rt/src/cpu_ce.rs:336` — `vmm.raise_irq(COMPLETION_VECTOR)`, deep inside
   `write_resolved_completion`, i.e. **inside** `run_submission`, i.e. **inside**
   `DoorbellPort::ring`.
2. `kayfabe-device/src/plane.rs:2845` — `announce_completion(engine)` latches the engine's
   non-stall vector into the CPU interrupt tree, and its `raise_cpu_intr` is delivered by
   the C shim. This runs **after** `port.ring(token)` has returned.

⇒ the guest is told *something finished* before we have recorded **what** finished.

### ⊘ What this is NOT

**Not a forged completion.** The payload is written before the raise, and `write_resolved_
completion` returns before raising on every error path, so no interrupt is ever raised over
work that did not happen. Q5 is untouched. It is an **unattributable** interrupt, not a false
one.

### ⚠ Why it is invisible, and why that is the reason to fix it

The guest **polls** (`uvm_gpu_tracking_semaphore_update_completed_value`,
`channelWaitForFinishPayload`), so today nothing reads the vector's attribution. That is
precisely the *"works until the guest sleeps"* shape: the first blocking waiter turns a
latent ordering bug into a wakeup that cannot be attributed to an engine.

### Why it was not fixed alongside §16.21

The raise sits **three frames below** the latch and on the far side of the
`DoorbellPort::ring` seam, so no reordering inside `ceutils` can reach it. The honest fixes
all change §14.18's latch/deliver split:

- **(a)** `write_resolved_completion` stops raising and returns "an interrupt is owed";
  `CeUtilsRun::completions` already carries the count. The shim's `Ok(run)` arm then raises
  — but that is *still* before the plane's `announce_completion`, so (a) alone does not fix
  the order.
- **(b)** the completion MSI-X rides `WriteOutcome` the way `raise_cpu_intr` already does,
  and the C shim delivers both after `plane.write()` returns, in the plane's chosen order.
  This is the shape that actually fixes it, and it changes the C header.
- **(c)** the shim latches the engine itself before the copy runs. ⊘ Rejected on sight:
  `announce_completion`'s documented precondition is that it is reached **only** from a
  `ServedLocally`, i.e. only after the bytes moved. Latching first would make the
  attribution a prediction.

⇒ **(b)**, and it needs its own boot against the `s22_f4f3865_cup2` baseline. ⊘ Landing an
interrupt-delivery change unbooted, on the path that had just started working, is the
`an instrument that COMPILES is not one that RUNS` trap with the guest on the other end.

### ★ Related, same audit

`lockwitness::assert_lock_free` masks **ranked** locks only, so the CPU copy runs beneath
three locks it cannot see. Two of them are now at least *declared*
(`tests/tests/unranked_locks.rs`, §16.22) — `Mutex<Option<QemuVmm>>` is held across the whole
submission — but declaring is not witnessing, and the witness still cannot see them.

---

## 16.24 ★★★★★ BOOTED `s23_10a769c_cup2` — `GP100_UVM_SW` ADMITTED, four fatal refusals removed, and the wall DID NOT MOVE

**Status: BOOTED.** vast GA106 bench (`vh`, RTX 3060 `10de:2504`, host driver **580.159.04
Open**), stamp verified on **both** `target/release/libkayfabe_qemu_raw.a` (33.5 MB) and
`qemu-build/qemu-system-x86_64` (84.8 MB) →
`kayfabe-rev:10a769c6bd0c7c54eb09f9c670069a0e6827baf8` in each, matching a clean `HEAD`.
**STOCK** guest module, `MODPROBE_RC=0`, `SMI_RC=0`, `probe-arm set: EMPTY`, hook
`cup2_hook_deadline.sh` — the same hook s20/s21/s22 carried. Evidence:
`traces/guest_boots/run_s23_10a769c_cup2_{dmesg,probe}.log`.

### 16.24.1 ⊘⊘ WHAT THIS REFUTES FIRST — the rung I was handed, and it was the WRONG CALLER

The brief named the rung as **GR context promotion**, citing

```text
NVRM: kgrobjPromoteContext(...) @ kernel_graphics_object.c:224 -> NV_ERR_NOT_SUPPORTED
```

⊘ **That line is the RC WATCHDOG's, not the golden-image channel's, and the handles say so
outright.** `s22`'s own dmesg puts it between `hObject=0x31415900` / `0x3141590f` allocs and
`kernel_rc_watchdog.c:1198`, and

```text
ogkm-580: src/nvidia/src/kernel/gpu/rc/kernel_rc_watchdog.c:64
  #define WATCHDOG_PUSHBUFFER_CHANNEL_ID 0x31415900
```

names the owner exactly. §14.20/§14.22/§14.26 each measured that engine's refusals
**non-fatal**, and §14.26 already closed the question the brief was re-asking: the
golden-image channel **completes**, its `0xc797`/`0xbaba00xx` lines having left the log for
good. `s22`'s device census agrees independently — `control 0x2080012b result 0x00000000
x2` **alongside** `result 0x00000056 x2`, i.e. two promotions served and two refused, the
refused pair being `PromoteFault::UnknownContextObject x2` on the watchdog.

⇒ **§5 Q0 is CLOSED.** It was answered on 2026-08-08 and re-queued for a day afterwards
because a `file:line` was read without its caller. Third instance of
`read_the_caller_not_the_id`, and the first where the wrong caller was in a *brief* rather
than in a ledger.

### 16.24.2 ★ The wall the same log actually named — and it had never been read

`s22`'s `cuInit` window ends like this, four lines apart, immediately before teardown:

```text
GspRmAlloc failed: hClient=0xc1d0000a; hParent=0xcaf00012; hObject=0xcaf00015;
                   hClass=0x0000c076; paramsSize=0x00000000; status=0x00000056
   … and three more, hParent=0xcaf0001d / 0xcaf00028 / 0xcaf00033
Assertion failed: NULL != pGpuState->pRootInternal @ gpu_vaspace.c:3332
```

`0xc076` is **`GP100_UVM_SW`** (`ogkm-580: clc076.h:33`), and it is the **last call of every
UVM channel allocation**:

```c
// Allocate the SW method class for fault cancel
if (isDevicePascalPlus(device) && (channel->tsg->engineType != ..._SEC2))
{
    status = pRmApi->Alloc(pRmApi, session->handle, channel->channelHandle,
                           &channel->hFaultCancelSwMethodClass, GP100_UVM_SW, NULL, 0);
    if (status != NV_OK)
        goto cleanup_free_controlpage;
}
```

(`ogkm-580: src/nvidia/src/kernel/rmapi/nv_gpu_ops.c:6110-6122`, in `channelAllocate`
`:5730`.) A GA106 is Pascal-plus and UVM's CE channels are not SEC2, so the branch always
runs, and there is **no forgiving caller** above it — `nvGpuOpsChannelAllocate` fails, and
with it `uvm_channel_manager_create` and `UVM_REGISTER_GPU`. Four parents, eleven handles
apart: four UVM channels, each destroyed at its final step. The `pRootInternal` assert at
`gvaspaceExternalRootDirRevoke_IMPL` (`gpu_vaspace.c:3277`, asserting at `:3332`) is the
teardown that follows.

⊘ `0xc076` was permitted by **neither** table — absent from `capability.rs` (hence
`AllocClassNotPermitted::NotOnAllowlist`) and absent from `alloc_params`.

### 16.24.3 ★★ `NoDeclaredFacts`, and why this row's version of it is the STRONGEST on the table

`AMPERE_B` and `NV2081_BINAPI` are both `RS_OPTIONAL`: a params struct exists and a NULL is
merely *legal by declaration*. `GP100_UVM_SW` is registered **`RS_NONE`** —

```text
ogkm-580: src/nvidia/src/kernel/rmapi/resource_list.h:1535-1544
  /* External Class */ GP100_UVM_SW,  /* Alloc Param Info */ RS_NONE,
  /* Flags */ RS_FLAGS_ALLOC_PRIVILEGED | … | RS_FLAGS_CHANNEL_DESCENDANT_COMMON,
  /* Parents */ RS_LIST(classId(KernelChannel)),
```

— **no alloc-params struct is declared for the class anywhere**, and its one allocator
passes `NULL, 0`, measured on the wire as `paramsSize=0x00000000`. *"Its params are never
read"* is a property of the ABI here, not a choice this port made.

★ `Origin::Mode2Rpc` is exact rather than a fallback: `grep -rn 0xc076 gvisor/` is **empty**
(checked with `$?`, not through a pipe). A privileged leaf the guest's own **kernel** RM
allocates inside `nvGpuOpsChannelAllocate` never crosses the ioctl boundary nvproxy gates;
it reaches us only because in Mode 2 the transport is GSP RPC and we are the GSP.

### 16.24.4 ★★★★ THE MEASUREMENT — exactly −4 and −4, and NOTHING ADDED

Device census, `s22_f4f3865` → `s23_10a769c`:

| row | s22 | s23 |
|---|---|---|
| `AllocClassNotPermitted::NotOnAllowlist` | **x6** | **x2** |
| `RmGraphError::FreeUnknown` | **x11** | **x7** |
| bridge refusals, total | 26 | 18 |
| `AllocClassNotPermitted::Refused` | x2 | x2 |
| `ReservedClient` / `UnmappedAllocClass` / `PromoteFault::UnknownContextObject` | x2 / x3 / x2 | x2 / x3 / x2 |
| `commands` | 454 decoded, 84 UNSERVICED, 38 distinct | **identical** |
| `controls` | 130 answered, 45 distinct rows | **identical** |
| `doorbells` | 24 arrived, 9 served, 15 REFUSED | **identical** |
| `gpfifo rings` | 10 declared, 8 non-zero | **identical** |

★ **The four `0xc076` refusals are gone — `grep -c c076` over `s23`'s dmesg and probe logs
is `0`** — and so are the four `GspRmFree failed … 0xcaf000{15,1e,2b,34}` lines, which were
the guest tearing down the four dead channels. The `−4` and the `−4` are the same four
objects, seen on the allocate side and the free side. **Six distinct refusal kinds before,
the same six after: nothing was added.** The accuracy improvement cost nothing, which per
§14.21/§14.24 was the thing to check rather than assume.

### 16.24.5 ⊘⊘ AND THE WALL DID NOT MOVE — my own hypothesis, refuted by the boot

```text
FAIL cuInit(0) -> initialization error (3)     CUP2_RC=1
doorbells: 24 arrived, 9 served, 15 REFUSED by name
first doorbell refusal [FwdFault::NoVas] NoVas(ChanId(3))
    | c=0xc1e00010 vas=NONE-DECLARED dec=NONE userd=h0x9/off0x0 ring=0x120064000
```

**Byte-for-byte s22's verdict.** I opened this increment expecting `0xc076` to be *the*
wall: it is a refusal that RM's own source shows is fatal to every UVM channel, it sits
immediately before the teardown, and removing it removed exactly what it should have. ⊘
**And the guest's outcome did not change at all.**

★ That is worth more than a moved wall, because it settles an ORDERING that no timestamp
could. I had tried to place `0xc076` relative to the doorbell refusals by anchoring guest
uptime against the host's wall clock, and concluded it came first. ⊘ **That reading is
refuted**: had `0xc076` been upstream of the doorbells, removing it would have changed the
doorbell census, and the census is identical to the entry. `0xc076` was a *downstream
casualty* on a path already lost, and the live wall is the one the census has been naming
all along.

⇒ **The named next wall is `FwdFault::NoVas` — `NoVas(ChanId(3))`, 15 of 24 doorbells, on
client `0xc1e00010` with `vas=NONE-DECLARED dec=NONE`**: the channel declares no VA space of
its own and none resolves through its CtxShare or its TSG. ⊘ Note this is the same string
s22 reported; what §16.24 buys is that it is now the *only* candidate rather than one of two.

### 16.24.6 ★★ The hedge in §16.24's own comment, made EXECUTABLE rather than prose

The admission carried a scope: *"the object's only in-band use is to hold a subchannel for
`FAULT_CANCEL_A`, and this port raises no fault for UVM to cancel — a guest whose faults we
DO deliver is the case this row does not cover."*

⊘ **That is exactly the shape that cost six boots one rung ago** (§16.21: a comment naming
its own exception, and the code taking the rule). So it is compiled instead of narrated:
`kayfabe_abi::submit::uvm_sw::is_fault_method`, `PushMethod::UvmSwFaultMethod`, and
`FwdFault::UvmFaultMethodWithoutFaultDelivery`, which **refuses the whole submission before
the execute loop** — so a cancel can never be walked past while the copies beside it run.

⊘⊘ **And the obvious trigger is REFUTED by source.** *"Fire when `SET_OBJECT
GP100_UVM_SW` appears"* is wrong: `uvm_hal_pascal_host_init` is the host HAL's **per-push
init hook** —

```c
void uvm_hal_pascal_host_init(uvm_push_t *push)
{
    if (uvm_channel_is_ce(push->channel))
        NV_PUSH_1U(C076, SET_OBJECT, GP100_UVM_SW);
}
```

(`ogkm-580: kernel-open/nvidia-uvm/uvm_pascal_host.c:314-318`) — so the bind heads **every**
UVM CE pushbuffer, and `[measured, boot s23_10a769c]` nine served doorbells carried it. A
tripwire there fires on every healthy submission and means nothing. `NO_OPERATION`
(`clc076.h:36`) is excluded for the same reason. What expires the assumption is a **cancel**:
`FAULT_CANCEL_A/B/C` and `CLEAR_FAULTED_A/B`, `0x104..=0x114`, reachable only once something
has told UVM a fault occurred — which this port never does.

★ Bite-checked, not merely written: with the predicate disabled the test fails
`FAULT_CANCEL_A (0x104) must trip … left: None, right: Some(260)`; restored, 14/14 green;
and `git diff --stat` confirmed the file actually changed on both passes
(`the_bite_check_that_could_not_bite`).

### 16.24.7 ⊘ Two things the suite says that are NOT this increment's

- ★★ **`stress_multi_vcpu_interleaved_ops` is RED at the parent commit, 5/5, deterministic.**
  `[measured 2026-08-09]` in an isolated `git worktree add --detach 11b1377`, with
  `KAYFABE_SLOW=1`, it panics identically every time —
  `doorbell routes: NotScheduled { chan: ChanId(0), vchid: VChid(258) } @
  tests/tests/concurrency_stress.rs:421`, in 0.09 s. ⊘ **So "the suite is green at
  `11b1377`" is refuted.** The test is `KAYFABE_SLOW`-gated, so a plain `cargo test
  --workspace` **skips** it and reports green — `skipped_oracle_kills_the_guard`, and the
  most likely origin of the green claim. It is a real, deterministic, pre-existing defect
  and it is owed; it is not this increment's and was not bundled with it.
- The claim-ledger ratchet is red at **69 (bar 66)** conflated and **18 (bar 17)** bare —
  the residue already recorded as debt owed. ⊘ Unchanged by this increment, and ⊘ no bar
  was raised.

★ Otherwise **210 test targets `ok`**, and the three capability ratchets that fired were
this increment's to move and were moved with their reasons: class counts +1 at **all four**
boundaries (75→76, 83→84, 89→90, 91→92 — moving *together* is the evidence the row went
into the shared base) and decoded classes 13→14.

## 16.25 ★★★★ The `NoVas` null MADE TO DISCRIMINATE — and it named the wall in one boot

`[measured 2026-08-09, boot `s24_cf18883_cup2`, bench `vh`, GA106/RTX 3060, host
580.159.04 Open, STOCK guest, hook `cup2_hook_deadline.sh` — the same hook s20–s23
carried. Rev `cf188835a8f4…` stamped in BOTH the archive and the QEMU binary, matching a
clean `HEAD`.]`

### 16.25.1 What was wrong with the instrument

s23 refused **15 of 24 doorbells** `FwdFault::NoVas(ChanId(3))` and printed

```text
c=0xc1e00010 vas=NONE-DECLARED dec=NONE userd=h0x9/off0x0 ring=0x120064000
```

`project::resolve_channel_vas` has **three** routes — the channel's own `hVASpace`, its
CtxShare's, its parent TSG's — and all three returned the identical `None`. `dec=NONE`
reports only that **the channel** declared none. `shim.rs:2414` had already written the bug
report as a comment: *"a `NoVas` refusal names the absence and nothing else."* Four
consecutive rungs were framed on guesses about that absence.

The fix was report-only: `resolve_declared_handle` now returns `Result<&RmNode,
HandleMiss>` (so the *reason* is produced by the code that makes the *decision*, never by a
parallel diagnoser), `resolve_channel_vas` returns `(Option<&RmNode>, VasRoutes)`, and the
refusal carries both the routes and a **census of every live channel grouped by outcome**,
because nine doorbells were served and the served channels are the control.

### 16.25.2 ★★★★ THE MEASUREMENT — the discriminator is exact

```text
doorbells: 24 arrived, 9 served, 15 REFUSED by name       (byte-identical to s23)
NoVas(ChanId(3)) | c=0xc1e00010 … route[own=not-declared cs=not-declared
                                        tsg=mid-miss(h0xa,wrong-kind(Device))]
census[6 chans, 3 outcomes]
  {1x pdb=N own=not-declared cs=not-declared tsg=mid-miss(h0xa,wrong-kind(Device))
       p0/c3*:vc1 Ce c0xc1e00010/0x2}                     ← THE WALL, and it is ONE channel
  {1x pdb=Y own=ok(h0xa=>c0xc1e00011/0xa) cs=not-attempted tsg=not-attempted
       p0/c4:vc2  Ce c0xc1e00011/0x2}
  {4x pdb=Y own=not-declared cs=not-declared tsg=ok(h0xcaf00005=>c0xc1d0000a/0xcaf00005)
       p0/c6:vc3 c7:vc4 c8:vc5 +1 more  Ce c0xc1d0000a/…}
```

Three facts fall out that no previous boot could state:

1. **The wall is ONE channel.** Six live channels, and exactly one has `pdb=N`. All 15
   refusals are that one channel rung 15 times — not a class of channels.
2. **The other five resolve, by two different routes**, so the machinery works: one on its
   own `hVASpace`, four (the UVM channels, on `0xc1d0000a` — the same client prefix the
   page-directory publications arrive on) through their parent TSG.
3. ★★★ **The refused channel declares NOTHING and its parent is a `Device`.** Route 3
   looked up its parent handle `0xa` and found a **Device**, not a TSG. ⊘ All three routes
   are *correct*: there genuinely is nothing declared to inherit from.

⊘ **The wall did not move**, and it was never going to: this increment adds no behaviour.
`cup2` is still `FAIL cuInit(0) -> initialization error (3)` (`CUP2_RC=1`). The doorbell
counts are byte-identical to s23. That invariance is the evidence the change is
observational.

### 16.25.3 ★★★★ WHY THERE IS NO FOURTH ROUTE — and why no instrument could have found it

RM source, not inference (`ogkm` 580.159.04):

- `kernel_channel.c:350-375` — when a channel's parent is a **Device** rather than a TSG,
  RM **internally allocates a TSG to wrap it**, forwarding `tsgParams.hVASpace =
  pChannelGpfifoParams->hVASpace` (here `NV01_NULL_OBJECT`). Its own comment: *"Internally
  allocate a TSG to wrap this channel. **There is no point in mirroring this allocation in
  the host**, as the channel is already mirrored."*
- `kernel_ctxshare.c:127` → `vaspaceGetByHandleOrDeviceDefault(pClient, hDevice, hVASpace,
  &pVAS)`.
- `vaspace.c:178` — `vaspaceGetByHandleOrDeviceDefault_IMPL`: when `hVASpace ==
  NV01_NULL_OBJECT` it resolves **the DEVICE** and returns **the device's default VA
  space**.

⇒ The missing route is: **a channel that declares neither `hVASpace` nor `hCtxShare`, and
whose parent is a Device, inherits that DEVICE's default VA space.**

★★★ And note the shape of the trap, because it is the reusable lesson: the intermediate we
were looking for **is deliberately never sent to us**. RM allocates that wrapper TSG on the
CPU side and explicitly declines to mirror it. So this is not a fact we failed to observe
and could have caught by observing harder — it is a fact that is *unobservable by
construction*, and must therefore be **derived and then oracled**
(`derive_what_you_cannot_query_then_oracle_it`). Every rung that assumed better
instrumentation would eventually surface the missing parent was chasing an object that does
not exist on the wire.

⊘ **Deliberately NOT fixed here.** Adding the fourth route is a behaviour change and needs
its own boot to be attributable — and it carries a real open question this boot does not
answer: whether the Device's default VA space is a VASpace the guest allocated (findable in
the graph) or one RM created implicitly with the Device (which we would have to mint). The
`0xc1e00010` namespace shows no VASpace in this capture, but the capture has no object
census, so that is **unmeasured, not empty** — `c_oracle_empty_rows_are_wrong`.

## 16.26 ⊘ The owed red test: the GATE was right, the SETUP was stale

`stress_multi_vcpu_interleaved_ops` failed 5/5 at `11b1377` (§16.24.7) and reproduces at
`bc53173` on the bench: `NotScheduled { chan: ChanId(0), vchid: VChid(258) }`.

**Root cause.** `3ab1305` (#177, 2026-08-03) made `plan_doorbell` gate on
`proc.exec.requested`, which only `Gpu::schedule_channel` — the guest's own `0xa06f0103` —
writes. That gate is #177's entire point: it is what makes serving the control a *performed
transition* rather than a fabricated promise. `stress_gpu` builds its device from
`Scenario::compute_process`, which emits `Alloc`/`SetPageDir` and nothing else, so from
`3ab1305` onward the test asserted a doorbell would be served on a channel nobody had asked
to schedule. Every other test needing a live doorbell already calls `schedule_channel`.

★ It **looked** like a race and was not one: `VChid(258)` = `gr_vchid(2)` every run, because
the per-thread RNG is deterministic. A deterministic "concurrency" failure is a tell that
the concurrency is not the cause — `suspect_the_instrument_first`.

★★★ It survived six days because `skip_slow!` means `cargo test --workspace` never runs it
and reports **green**. Fixed by having the setup do what a real guest does. **3/3 green on a
4-core box** (the flake-prone configuration), full ~23 s soak each.

★ And the wider suite is better than recorded: `KAYFABE_SLOW=1 cargo test --workspace
--no-fail-fast` at `bc53173` on the bench had **exactly one failing target** — this one. The
"6 tests red" of §16.22 no longer holds.

### 16.27.1 ★★★★ BOOTED `s25_01d12e6_cup2` — the fork is settled: **there is no VASpace to find**

`[measured 2026-08-09, boot `s25_01d12e6_cup2`, bench `vh`, GA106/RTX 3060, host
580.159.04 Open, STOCK guest, hook `cup2_hook_deadline.sh`. Rev `01d12e6b078a…` stamped in
BOTH archive and QEMU binary, matching a clean `HEAD`.]`

```text
ns[c0xc1e00010 6 objs 1xChannel { engine: GrCompute } 1xEngineObject { engine: Ce }
   1xEvent 1xDevice 1xSubdevice 1xClient | NO-VASPACE-IN-NAMESPACE]
```

⇒ **Fork (b).** The walling channel's namespace holds **six objects and not one VASpace**.
So the Device's default VA space was created implicitly by RM and **was never an `RM_ALLOC`
on the wire**. The missing fourth route cannot be a lookup — there is nothing to look up.
It must **MINT** a VA space for the Device.

★ Note this is stated **positively** (`NO-VASPACE-IN-NAMESPACE` is printed by an
enumeration that ran), not inferred from a report that failed to mention one. That
distinction is the whole of §16.27.

### 16.27.2 ★★ The observation-only self-check PASSED

`doorbells: 24 arrived, 9 served, 15 REFUSED` — **byte-identical to s23, s24 and s25**, and
`cup2` is still `FAIL cuInit(0) -> initialization error (3)` (`CUP2_RC=1`). §16.27's commit
demanded exactly this: *"the next boot's doorbell counts must again be byte-identical …
if they are not, this 'observation-only' change was not."* They are.

### 16.27.3 ★ What the namespace's SHAPE says, and one candidate it makes concrete

Six objects — `Client → Device → Subdevice → Channel(+ `EngineObject{Ce}`) + Event` — with
no VASpace and no TSG. That is the **GSP-managed CeUtils / scrubber** shape
`kayfabe-fwd/src/lib.rs:3357` already names (*"GSP-managed CeUtils channel walls
`NoVas(ChanId(1))`"*).

⚠ Note also that the channel's declared **class** is `Channel { engine: GrCompute }` while
the exec-plane census reports it as `Ce`: the `EngineObject { engine: Ce }` allocated on it
refines it, which is `ChannelFacts::engine`'s documented job. ⊘ Not a contradiction — the
two fields are the class default and the refinement, and they are behaving as specified.

★ **A CANDIDATE, offered as such and not as a finding.** §14.24 records that
`try_ce_submission`'s precondition 2 used to read *"`vas_pdb` must be `None` — a channel the
core can address is the core's"*, and that it was replaced by a build-time decision from
`selected_isolate_plane`. This channel has `vas_pdb: None` and is exactly the CE-scrubber
family that the shell's CPU CE executor exists to serve (the standing rule *"⊘ do not flip
`KAYFABE_ISOLATES=real` — it takes the CE scrubber from the only executor that serves
it"*). So *"the scrubber's doorbell reaches `plan_doorbell` and takes `NoVas` before the
shell executor ever gets the chance"* is a **checkable hypothesis** about the dispatch, and
it is the natural next rung.

⊘ It is NOT established here, and this campaign's last four rung framings were refuted, so
it is written as a question with a named place to look — `a_queue_item_is_a_hypothesis`, and
`a_table_does_not_decide_behaviour — the DISPATCH does`. **Read the dispatch before
believing it.**

## 16.28 ★★★★ THE FOURTH ROUTE — and §16.27's "it must MINT" is **REFUTED BY SOURCE**

### 16.28.1 ⊘⊘ What this increment refutes, starting with its own brief

§16.27 settled a fork positively — `NO-VASPACE-IN-NAMESPACE`, printed by an enumeration
that ran — and concluded: *"the Device's default VA space was created implicitly by RM and
**was never an `RM_ALLOC` on the wire**. The missing fourth route cannot be a lookup —
there is nothing to look up. It must **MINT** a VA space."*

⊘ **The enumeration is right and the conclusion drawn from it is wrong**, and this
increment's first job is to say so plainly. The Device's default VA space **is** an
`RM_ALLOC` on the wire. It is simply an alloc that is **freed again three RPCs later**, so
by the time any doorbell rings, the namespace census truthfully reports no VASpace in it.
An enumeration taken at time *T* cannot see an object that lived from *T-3* to *T-1*, and
nothing in §16.27's capture distinguished *"never existed"* from *"existed and was freed"*.

★ Note the shape, because it is a *new* member of a family this campaign keeps meeting:
`c_oracle_empty_rows_are_wrong` says an empty capture is evidence of nothing. This is the
next one along — **a capture that is full, positive, and correct can still be evidence for
the wrong conclusion, when the question is about a LIFETIME and the instrument samples one
instant.** §16.27 was careful to state its fork positively and still landed on the wrong
side of it, because "positively stated" and "quantified over time" are different properties.

⊘ It also refutes §16.25's own generalisation, one clause of it: *"the intermediate is
unobservable by construction … no better instrument would ever have surfaced it."* That is
**true of the wrapper TSG** (`kernel_channel.c:350-375` really does decline to mirror it)
and **false of the VA space**, which announces itself explicitly. Two different objects
were folded into one sentence.

### 16.28.2 ★★★★ THE MECHANISM, read from RM rather than inferred

`ogkm-580: src/nvidia/src/kernel/mem_mgr/gpu_vaspace.c:4066-4136`,
`gvaspaceCopyServerRmReservedPdesToServerRm_IMPL`. When the calling resource is not a
`VaSpaceApi` — i.e. this is the **device default** VA space, which has no client handle
because RM creates it with the Device (`device_share.c:324-347`, `pDevice->pVASpace`) —
the local `hVASpace` is **zero** (`:4070-4075`), and on a GSP client RM then:

| step | site | what reaches the wire |
|---|---|---|
| 1 | `:4101` `serverutilGenResourceHandle(hClient, &hVASpace)` | nothing — a fresh handle is minted |
| 2 | `:4103-4113` `NV_RM_RPC_ALLOC_OBJECT(… hDevice, hVASpace, FERMI_VASPACE_A, &vaParams)` with `vaParams.index = NV_VASPACE_ALLOCATION_INDEX_GPU_DEVICE` | ★ **a VASpace alloc** |
| 3 | `:4128` → `:5175` `rmCtrlParams.hObject = hVASpace` | ★ **the page-directory publication**, `0x90f10106` |
| 4 | `:4135` `NV_RM_RPC_FREE(pGpu, hClient, hDevice, hVASpace)` | ★★★ **the free** |

RM's own comment at step 2: *"VAS handle is 0 for the device vaspace. Trigger an allocation
on server RM so that the plugin has a valid handle to the device VAS under this client.
This handle will be required by the plugin when we make the RPC later."*

⇒ ★★★ **The free frees the NAME, not the ADDRESS SPACE.** `pDevice->pVASpace` is untouched
by step 4; the only site that destroys it is `deviceRemoveFromClientShare_IMPL`
(`device_share.c:307-320`), which runs when the **Device** goes away. Reading step 4 as
*"the guest destroyed its address space"* is what discarded the only statement of that
address space the wire ever carries.

★ And the discriminator is a single wire field: `NV_VASPACE_ALLOCATION_INDEX_GPU_DEVICE`
`= 0x03`, *"Acquire reference to device vaspace"* (`ogkm-580: nvos.h:3187`), against
`…INDEX_GPU_NEW = 0x00`, *"Create new VASpace, by default"* (`:3184`).

### 16.28.3 ⊘ A WRONG COMMENT is why nobody looked — and it was ours

`versions.rs`'s alloc-params table carried `FERMI_VASPACE_A` under `NoDeclaredFacts` with
this justification:

> *"A VASpace's params are geometry (`index`, `vaSize`, `vaBase`, `pasid`) … the protocol
> content of all three is the EDGE — parent, handle, class — which the RPC header already
> carries."*

`index` is **not** geometry; it is the field that says whether the alloc creates anything.
So the one wire fact that identifies the walling channel's address space was sitting behind
a comment asserting there was nothing there to read — `a_wrong_comment_is_why_nobody_looked`,
and it cost four rungs (§16.24 → §16.27).

⚠ **Acceptance is deliberately unchanged.** `decode_vaspace_index` returns an `Option` and
**cannot fail**: params shorter than four bytes yield `None` (*"unread"*), never a refusal.
A class this port accepts today must not become one it rejects because somebody wrote a
reader for it — `accuracy_is_fatal_when_a_fallback_was_keyed_on_ignorance`, twice measured.

### 16.28.4 What was built

- `kayfabe_arch::VaSpaceRole { Own, DeviceDefault }` — the vocabulary type, because
  `kayfabe-core` is quarantined from NVIDIA numbers (decision #2). The bridge decides the
  meaning; the core acts on the meaning.
- `RmGraph::device_default_vas: BTreeMap<NodeKey /*(client, DEVICE)*/, HObject>` — latched
  at the index-3 `Alloc`, keyed on the **parent Device**, and cleared **only** when the
  Device's own handle is freed. ⊘ The transient VASpace handle's `Free` prunes nothing,
  because it is not a key — which is the whole asymmetry, and it mirrors RM's exactly.
- ⚠ **No lifetime is extended.** The VASpace resource still dies with its handle, `refs`
  keeps its *"never empty for a live resource"* invariant, and nothing is kept alive. What
  survives is one `HObject` — a **name** — and a name is enough because
  `kayfabe_device::gvaspub` files the guest's own publication under `(hClient, hObject)`
  and never prunes it.
- `project::resolve_channel_vas` **route 4**, running only when route 3 positively resolved
  the parent and found a **`Device`** (`HandleMiss::WrongKind(ObjectKind::Device)`) — the
  same fork RM branches on. Reported as `VasHop::DeviceDefault{device,vas}` or, when the
  Device has named none, `VasHop::DeviceDefaultUndeclared{device}` — a **DEFER**, not a
  verdict.
- ⊘ Route 4 populates **no** `vas_origin` and **no** `vas_pdb`. The object really is dead;
  minting a `Vas` out of a freed handle is the forgery this project forbids. `by_pdb`,
  `plan_doorbell` and the #14 ring gate are untouched.
- The dispatch reads it at `ce_channel_facts` as an `or` — a channel that resolved a live
  VASpace keeps that answer, and the two can never disagree because
  `resolve_channel_vas` returns at most one of them.

### 16.28.5 ★★ Why this is expected to MOVE the wall, stated before the boot

`SharedDoorbell::ring` tries the CPU copy-engine executor **first**, unconditionally. For
the walling channel it declined itself at `facts.vaspace?` — ⊘ **so the predecessor's
hypothesis that "the scrubber's doorbell takes `NoVas` before the shell executor gets it"
is refuted by the dispatch: the executor gets it first and returns `None`, and the `NoVas`
is downstream of that decline.** The remaining preconditions are already met:

| precondition | walling channel, `[measured, s25]` |
|---|---|
| `facts.vaspace` | ⊘ **`None` — the only one missing** |
| `facts.ring_va` | `Some(0x120064000)` |
| `vas_pdb.is_some() && !local_ce_is_the_only_executor` | `false` (`isolates: 2 … 2 refusing (2 no-plane)`), so it does not decline here |
| a publication for `(hClient, hVASpace)` | ★ `gvas cmd 0x90f10106 hClient 0xc1e00010 hObject 0xc` — **already recorded and ACCEPTED** |

⇒ **The prediction is falsifiable and specific.** The doorbell census must stop being
byte-identical: the one `pdb=N` channel should join the served group, `devdef=0xc` should
appear in place of `devdef=NONE`, and the split `24/9/15` must move. ⊘ If it is
byte-identical again, route 4 fired on nothing and this increment changed nothing — and
that must be said plainly rather than explained.

⊘ **This increment is a BEHAVIOUR change and is deliberately not bundled with anything**
(§16.23's interrupt-ordering defect stays owed and separate), so whatever moves is
attributable to it.

### 16.28.6 Bite-checked, not merely written

Three mutations, each verified to have **actually changed the file** (`diff` against a
pristine copy, not `git diff --stat` — `the_bite_check_that_could_not_bite`):

| mutation | expected | result |
|---|---|---|
| route 4 never fires | the outlives test fails | ✔ 1 failed |
| the latch ignores the declared role (`!= Own` instead of `== DeviceDefault`) | the `Own` test fails | ✔ 2 failed |
| the free prunes the latch by the **VASpace** handle too (the defect this fixes) | the outlives test fails | ✔ 1 failed |

★ The first test asserts the freed handle really resolves to nothing **before** asserting
route 4 resolves it anyway — without that, the test could pass because the free did nothing
and every claim in it would be vacuous.

### 16.28.7 ⊘⊘ THE EVIDENCE OF s23/s24/s25 WAS NEVER IN THE REPOSITORY — recovered, and gated

`[measured 2026-08-09]` `git show --stat 08ef29b 18b75e8 a5ede26` — three consecutive
`BOOTED` commits, each touching **exactly one file**, `docs/design/execution_plane_increments.md`.
`s23`'s `_qemu.log` and the whole of `s24` and `s25` were **not in the tree**, and
`git status --porcelain` showed nothing untracked either: they had never been copied in.

⇒ Every number §16.25 and §16.27 rest on — *"the wall is ONE channel"*, the six-row census,
`NO-VASPACE-IN-NAMESPACE`, and above all the **`24/9/15` byte-identical invariance that is
the entire proof those two increments were observational** — described boots that nothing in
the repository could re-verify.

★ **Recovered.** The raw logs were still on `vh` and are now committed under
`traces/guest_boots/`, and the claims reproduce from them: all three of s23/s24/s25 grep
`doorbells: 24 arrived, 9 served, 15 REFUSED`, all three carry 31 `NVRM` dmesg lines, and
`run_s25_01d12e6_cup2_probe.log` carries
`kayfabe-rev:01d12e6b078a7bbe34fea1da480b292e5abff8be` — so the boots are attributable to
their commits. ⊘ Nothing in §16.25/§16.27 needed correcting; what was missing was the
ability for anybody else to check it.

**Why it happened, which is the valuable half.** `boot_capture.sh` did *not* fail to run,
and it is not lax — it asserts the dmesg is non-empty, that `RmInitAdapter` actually appears
(not merely `NVRM`, for a measured reason), and that the exit-notifier census reached disk.
It asserted all of that about **`/workspace/bench/`**, a directory on a rented box. Its own
final line names four bench paths and the repository is not one of them.
⇒ **The gap was the harness's SCOPE, not its rigour.**

★★★ And it is the **third sighting of one shape**, the second in a night:

| | the operation that succeeded anyway |
|---|---|
| C-era | the serial log exists, is fresh, is named after the boot — and contains no `NVRM` |
| earlier tonight | `git commit -- <path>` **does not add untracked files**, and exits 0 |
| this | `boot_capture.sh` stores the evidence somewhere nobody reading the repo can reach, and exits 0 |

⇒ The common cause is not carelessness: **the operation succeeds either way, so only an
explicit assertion separates the two outcomes.** Two were added:

- `boot_capture.sh` **phase 6** now carries `_qemu`/`_dmesg`/`_probe` into
  `traces/guest_boots/` and **dies** if fewer than three arrive. ⊘ `_serial.log` is
  deliberately not carried — 70 KB per boot, and `grep -ci nvrm` over every one returns 0.
- `scripts/bench/assert_boot_evidence.sh` — the gate a `BOOTED` commit must pass: the three
  files exist, are non-empty, are **tracked by git**, and *say what a boot says* (a census
  line, `RmInitAdapter`, and a `kayfabe-rev:` stamp). Run with no tag it sweeps the whole
  directory for untracked or empty files, which catches the other half — evidence copied in
  and then left out of the commit.

★ **Bite-checked by construction**: run against the tree before the recovery commit it
printed `FAIL … NOT TRACKED` and listed exactly the seven real files. A gate whose first run
is green over a known defect is not a gate.

★ **And the gate's own first tagged run was WRONG, in the way this project keeps measuring.**
It failed `s25` on *"no `RmInitAdapter` output"*. The log is fine: `RmInitAdapter failed!`
prints **only on failure**, and `s25`'s adapter came up — `SMI_RC=0`, 31 `NVRM` lines of real
driver work. `boot_capture.sh`'s own check is a **disjunction** (`n_adapter == 0` *and* no
`SMI_RC=0`); the gate was copied from it and **lost the clause that made it correct**, which
is `a_defect_in_the_argument_is_invisible` reproduced inside the instrument written to
prevent a different one. Fixed, and now green on s23/s24/s25 and red on a fabricated tag.

### 16.28.8 ★★★★ BOOTED `s26_0484a3b_cup2` — **the doorbell wall is GONE: `24/9/15` → `24/24/0`**

`[measured 2026-08-09, boot `s26_0484a3b_cup2`, bench `vh`, GA106/RTX 3060, host
580.159.04 Open, STOCK guest, hook `cup2_hook_deadline.sh` — the same hook s20–s25 carried.
Rev `0484a3b9987ab476b6026e4260f533c511fcbbb0` stamped in the archive AND in the QEMU binary
AND equal to the bench's `HEAD`, all three checked before booting.]`

```text
doorbells: 24 arrived, 24 served, 0 REFUSED by name          (s23/s24/s25: 24 / 9 / 15)
  last CPU-CE serving: cpu-ce: 1 gp, 9 methods, 1 launch (0 release-only), 1 span,
                       65536 B, 1 sem fin va=0x12006c004 -> S:0x4d09004
```

★★★ **The prediction §16.28.5 wrote down before the boot is met exactly.** Token
`0x00010001` — the one refused fifteen times in three consecutive boots — is now
`SERVED-LOCAL [CpuCe::ServedLocally]`, seven times in this log. The refused channel joined
the served group; `9 + 15 = 24`.

★★★★ **And the attribution is airtight, by INVARIANCE.** Every other census line is
**byte-identical** to s25:

| line | s25 | s26 |
|---|---|---|
| `commands:` | 454 decoded, 84 UNSERVICED, 38 distinct | **identical** |
| `bridge refusals:` | 18 total, 6 distinct | **identical** |
| `controls:` | 130 answered, 45 distinct | **identical** |
| `isolates:` | 2 materialized, 2 live, 2 refusing | **identical** |
| `gpfifo rings:` | 10 declared, 8 non-zero, first `0x120064000` | **identical** |
| `doorbells:` | 24 / **9** / **15** | 24 / **24** / **0** |

⇒ One line moved, and it is the one line this increment aims at. That is the same
invariance argument s24/s25 used to prove they were observational, run in the other
direction to prove this one is not.

★★ **Positive evidence that it is ROUTE 4 that fired**, rather than merely that the wall
went away: `va=0x12006c004 -> S:0x4d09004` is a **resolved** address on the walling
channel's own ring (`0x120064000 + FINISH_PAYLOAD_FROM_RING 0x8004` — the CeUtils
finishPayload semaphore, `ce_utils.c:349`'s subject). Resolving it requires
`ce_session(hClient, hVASpace)` to have found a publication, which requires
`facts.vaspace == Some(0xc)`, and route 4 is the only thing in the tree that can produce
that value for a channel whose three declared routes all miss. ⇒ **The name route 4 handed
over is demonstrably the name the walk resolved through**, and the guest's own
publication `(hClient 0xc1e00010, hObject 0xc)` is what it resolved *to*.

★ The channel is now doing real work: **65536 B** in one span, where s25's last serving was
the 32 B UVM push. ⊘ No completion was forged — the semaphore write is the one the guest's
own pushbuffer asked for, at the address its own methods named.

### 16.28.9 ⊘ WHAT DID NOT MOVE — and it must be said as plainly as what did

- **`cup2` is still `FAIL cuInit(0) -> initialization error (3)` (`CUP2_RC=1`)**, exactly as
  in s20–s25.
- ⊘⊘ **The guest's dmesg is BYTE-IDENTICAL to s25.** `diff` of the two, timestamps
  stripped, is **empty**. Not "similar" — empty.

⇒ **The doorbell plane's last refusal is gone and the guest cannot yet tell.** That is a
real result and a bounded one, and the two halves must not be blurred: this increment
removed a wall in *our* port and made a previously unaddressable channel execute; it did
**not** advance `cuInit`. Whatever `cuInit` is failing on is upstream or downstream of the
CE plane, and the identical dmesg says the guest's kernel-side story is unchanged.

★ Note what this rules out, which is worth more than the disappointment: *"the CeUtils
scrubber's refused doorbell is what fails `cuInit`"* — a live hypothesis since §14.24 — is
**refuted**. The doorbell is served, the copy runs, the semaphore is written, and `cuInit`
returns the same error at the same place.

### 16.28.10 ⚠ THE NEXT WALL — offered as a HYPOTHESIS with a named place to look

⊘ Written as a question because this campaign's last five rung framings were refuted
(`a_queue_item_is_a_hypothesis`), and because §16.28.9 has just demonstrated that *"the
loud refusal nearest the failure"* need not be the cause.

`run_s26_0484a3b_cup2_probe.log`'s `cup2` dmesg delta — ⊘ **identical to s25's**, so none of
this is new and none of it is caused by route 4 — shows one chain that ends where `cuInit`
does, all on the RC-watchdog client `0xc1d00013`, inside one second:

```text
[59.96] GspRmAlloc failed: hParent=0x31415903 hObject=0x3141590f hClass=0x00000070 → 0x56
[60.20] GspRmAlloc failed: hParent=0x31415903 hObject=0x31415900 hClass=0x0000c36f → 0x56
[60.34] Check failed: NOT_SUPPORTED from kgrobjPromoteContext @ kernel_graphics_object.c:224
[60.38] Assertion failed: status == NV_OK @ kernel_rc_watchdog.c:1198
```

The two classes are named, and one of them is a surprise worth checking before anything is
built on it:

- `0x0070` = `NV01_MEMORY_VIRTUAL` (`ogkm-580: cl0070.h:32`).
- `0xc36f` = ★ **`VOLTA_CHANNEL_GPFIFO_A`** (`ogkm-580: clc36f.h:43`) — **not**
  `AMPERE_CHANNEL_GPFIFO_A`, which is `0xc56f` (`clc56f.h:43`) and is the class this port
  maps. So the RC watchdog is asking for a channel class we do not admit at all.
- `kernel_rc_watchdog.c:1198` is the `NV_ASSERT(status == NV_OK)` at the function's `error:`
  label, i.e. the *report* of a failure that happened earlier in `krcWatchdogInit`.

⚠ **And the obvious inference is exactly the one to distrust.** `nvidia-smi` returns
`SMI_RC=0` in this same boot, so the adapter is up; the RC watchdog is a 1 Hz diagnostic
channel (`osSchedule1HzCallback`, `:1189-1193`) and its `error:` path frees its own client
and returns — it is not obviously on `cuInit`'s critical path at all. ⇒ **Loudness is not
causality** (`scrubber_teardown_is_not_the_wall`, measured once already in this campaign).

★ So the first move is **not** to admit `0xc36f`. It is to establish *which* call `cuInit`
actually fails on, from the guest side, with `scripts/bench/guest_cuinit_trace.sh` — the
instrument that answers "where", rather than reasoning from the loudest line in a log that
also contains a successful `nvidia-smi`.

### 16.28.11 ★★★★ THE OWNER'S PHYSICAL QUESTION — *"how is a real GPU supposed to know?"*

> *"if the vaspace is never allocated, how is a real GPU supposed to know, with a host driver?"*

★★★ **The question is right and it cuts through the abstraction.** Hardware has never heard of
a `VaSpace` object. The whole `Client → Device → TSG → CtxShare → VASpace` hierarchy is **RM's
bookkeeping** — handles in a namespace. What the engine consumes is the channel's **instance
block**, one field of which is the **page-directory base**; the host fetches from the GPFIFO,
hits a virtual address, and walks from *that* number. No objects, no handles. ⇒ The wrapper TSG
exists to satisfy **RM's own invariant** (every channel belongs to a scheduling group), not the
hardware's.

⇒ **Record this as a first-class distinction: the object model is bookkeeping; the instance
block and the page tables are the machine.** Anywhere this port reasons about objects, the
question to ask is what hardware actually reads. It is the same
*measure-at-the-boundary-not-inside* move that produced §16.28 in the first place — reading RM's
emitter rather than reasoning about our own census.

### 16.28.12 ⊘ AND IT ALREADY HOLDS HERE — because §16.28 MINTED NOTHING

The concern the question raises is about a **mint**: a minted VA space must still name an
address a real GPU could have walked. ⊘ **Route 4 mints nothing and invents no address.** Both
halves of what it produces are the guest's own:

| what route 4 hands over | where it comes from |
|---|---|
| the **name** `hVASpace = 0xc` | the handle **RM itself** minted, allocated `FERMI_VASPACE_A` at, published under, and freed (`gpu_vaspace.c:4101-4135`) |
| the **address** the walk roots on | the guest's own `0x90f10106` publication for `(hClient 0xc1e00010, hObject 0xc)`, recorded and ACCEPTED in every boot since `uvm1_b731e3c` |

⇒ **The physical acceptance criterion is not merely met, it is measured.** *"What would the
instance block have contained?"* — the page-directory root of the Device's default VA space, and
that is exactly the number the guest's own driver published under that name. The proof it is a
real address and not a plausible one is that the walk **succeeded**: `va=0x12006c004 ->
S:0x4d09004` on the walling channel's own ring, feeding a 65536 B copy (§16.28.8).

### 16.28.13 ★★ THE INSTANCE-BLOCK ROUTE — checked, named, and NOT needed to unblock

⊘ Recorded so nobody re-asks, and because it remains the **strongest available oracle**: two
independent derivations of one number.

- **This port does not capture the instance block at all.** `kayfabe_abi::submit` names the
  field's offset in a comment (*"+144 instanceMem"*) and nothing reads it. ⇒ The check the
  question proposes cannot be run against s26's logs; it needs an instrument first.
- ★ **The C oracle DID capture it, and measured it EMPTY.** `nvkvm_gpu_emul.c:2877` reads
  `instanceMem.base` from `cmd + 256` into `chan_inst_block`, and the field's own declaration
  says *"unused: GSP-managed, empty"* (`:225`), with `:369` stating the consequence outright:
  *"This is the channel PDB source (the GSP-managed instblk is empty …)"* — which is **why the C
  rooted its walk from the `0x90f10106` publication instead**, i.e. from exactly the source
  route 4 uses.
- ⚠ **But that negative is scoped to GSP-managed channels, and nobody has asked it of this
  Device-parented one.** ⊘ An empty read is evidence of emptiness only if the read is sound
  (`c_oracle_empty_rows_are_wrong`), and the C's reading was never aimed at this channel.

⇒ **Owed, as an oracle rather than as a fix**: capture `instanceMem.base` /
`instanceMem.addressSpace` off the channel alloc the way `gpFifoOffset` is already captured,
print it beside the resolved PDB for **all six** channels, and check whether the five that
resolve through the object model carry the *same* number in their instance blocks. ★ If they
do, the object model is confirmed as a convenience over the machine's own record — and route 4
gains a second, independent derivation. ⊘ It is not needed to unblock: the wall is already down
and the walk already resolves.

### 16.28.14 ⊘ THE SUITE CAUGHT §16.28 ONE COMMIT LATE — and the failing test held the refuted comment

`[measured 2026-08-09, `KAYFABE_SLOW=1 cargo test --workspace --no-fail-fast` on `vh` at
`0484a3b`]` — **210 targets `ok`, exactly one FAILED**:
`every_class_in_the_table_decodes_its_declared_facts_and_only_those`
(`tests/tests/rmrpc_bridge.rs:3168`), and it is §16.28's own.

⊘ **The honest part first: this was committed before the workspace suite was run.** `b0aeae7`
cites `cargo clippy --workspace --all-targets` clean and `miss_taxonomy` 25/25, and both were
true — but the guard that actually covered the change was in a different target, and it was not
run until after the commit and after the boot. ⇒ **Clippy plus the tests you thought of is not
the suite**; the whole point of a workspace suite is the target you did not think of.

★ And what it was guarding is the same wrong comment §16.28.3 records, transcribed into an
assertion:

> `xlate(FERMI_VASPACE_A, [0xff; 56]) == AllocFacts::default()`
> *"★ VASpace: 56 bytes of 0xff declare NOTHING. Its params are geometry, and a decoder that
> invented a fact from them would be inventing it from garbage"*

⇒ The refuted claim existed in **two** places — a rustdoc table and a test — and the test made
it look verified. ★ `a_wrong_citation_is_more_durable_than_none`, with the extra twist that a
green test is a much stronger endorsement than a comment.

**Fixed by strengthening, never by weakening.** One row became three, so the discriminator is
pinned in both directions and the unread case is its own third answer:

| params | expected | what it forbids |
|---|---|---|
| `index = 0xffff_ffff` | `Some(Own)` | ⊘ garbage claiming a Device's address space |
| `index = 3` | `Some(DeviceDefault)` | the whole fourth route rests on this one comparison |
| **empty** | `None` (unread), and **still accepted** | ⊘ a class the port admits becoming one it rejects because a reader was written for it |

⊘ The row's underlying concern — *a decoder must not invent facts* — is preserved verbatim; only
its false premise about `index` is gone. `rmrpc_bridge` 114/114, `miss_taxonomy` 25/25, clippy
clean.

⚠ ⊘ **This does not touch s26.** The failing assertion is a test-side expectation; the shipped
decoder is unchanged by the fix, so the binary that booted is the binary this describes.

### 16.28.15 ★ One interaction worth recording before somebody flips `KAYFABE_ISOLATES`

Route 4 sets `CeChannelFacts::vaspace` and leaves `vas_pdb` at `None` (§16.28.4). That means
`try_ce_submission`'s precondition 2 —

```rust
if facts.vas_pdb.is_some() && !self.local_ce_is_the_only_executor { return None; }
```

— cannot fire for a route-4 channel **whatever plane is installed**, so the CeUtils scrubber
stays with the shell's CPU copy-engine executor even on a build that selects a real isolate
plane.

★ That is the standing rule's own content, now held by construction rather than by a warning:
*"⊘ do not flip `KAYFABE_ISOLATES=real` — it takes the CE scrubber from the only executor that
serves it."* §14.24 measured the cost of the opposite (`pub1_3e43e9a`: an accurate `vas_pdb`
handed the scrubber to a `Stillborn` plane and cost the adapter). ⊘ It is recorded here as an
*observed consequence*, not as a designed one — nothing was arranged for it, and a later change
that gives route-4 channels a `Pdb` would silently undo it.

---

## §16.29 ★★★★ BOOTED `s27_c73d3ab_uvm` — the RC watchdog is REFUTED, and the wall has a NAME

`[measured 2026-08-09, boot `s27_c73d3ab_uvm` at `c73d3ab`]`, binary and archive both stamped
`kayfabe-rev:c73d3abfea3314b0073d989fdabcae7ee94f5e23`, evidence
`traces/guest_boots/run_s27_c73d3ab_uvm_{qemu,dmesg,probe}.log`.

⊘ **Stated first: `cup2` is still `FAIL cuInit(0) -> initialization error (3)` (`CUP2_RC=1`).**
Nothing was made to work in this increment. What changed is that the failure now has a source-
level chain instead of a nomination.

### 16.29.1 ⊘⊘ What this increment REFUTES, starting with its own brief

**1. The nominated instrument could not have answered the question.** §16.28.10 said *"the first
move is `scripts/bench/guest_cuinit_trace.sh`"*. That script preloads
`scripts/rpctrace/cuda_ioctl_trace.c` and nothing else, and that file gates every decode on
`_IOC_TYPE(request) != NV_IOCTL_MAGIC` (`'F'`, `:493`). **UVM's ioctl magic is 0**
(`UVM_IOCTL_BASE(i)` is literally `i`, `ogkm-580: uvm_ioctl.h:41`), so `/dev/nvidia-uvm` traffic
passes it untouched. ⊘ The nominated instrument is **structurally blind to the only plane that
fails** — and the brief said so one paragraph earlier and still named it. ★ **A warning written
next to a wiring defect does not fix the wiring**; it makes the defect look considered.

**2. "Guest-side instrumentation has been owed for six rungs" is false.** It was built and run at
§14.40: `scripts/bench/uvm_ioctl_trace.c` + `guest_uvm_status.sh` printed `UVM_REGISTER_GPU
rmStatus = 0x56` (`fb1503`), `0x1f` (`sh1605`) and `0x56` again (`ac1710`). What was owed was
that instrument's **value at the current revision** — a re-read, not a build. ⇒ A capability
recorded as *missing* when it is merely *stale* buys a rung of invented work.

**3. ★★★★ THE RC WATCHDOG IS NOT THE WALL — refuted twice over, and the second proof was
already in the tree.** §16.28.10 was right to distrust it; the reasons available were stronger
than the one offered.

- **From source.** `krcWatchdogInit` is called from `RmInitAdapter`
  (`ogkm-580: osinit.c:2161`), and the very next branch reads:
  ```c
  else if (status.rmStatus == NV_ERR_NOT_SUPPORTED) {
      NV_PRINTF(LEVEL_INFO, "krcWatchdogInit returned _NOT_SUPPORTED. … this is normal\n");
  }
  else { RM_SET_ERROR(status, RM_INIT_WATCHDOG_FAILED); … goto shutdown; }
  ```
  The whole chain (`0x70`, `0xc36f`, `kgrobjPromoteContext`, `kernel_rc_watchdog.c:1198`)
  carries `0x56` = `NV_ERR_NOT_SUPPORTED` — the **explicitly forgiven** branch, printed at
  `LEVEL_INFO` and continued past. ⇒ `not_supported_is_the_forgiven_status`, exactly.
  ⊘ **Admitting `0xc36f` would have bought nothing.**
- **From measurement, and it needed no new boot.** The identical four-line chain appears in
  **`nvidia-smi`'s own adapter init** — `run_s26_0484a3b_cup2_dmesg.log:33.4–33.9`, on
  `hClient=0xc1d00008` — in the boot where `SMI_RC=0`. ★ **A signature present in both a
  success and a failure cannot be the discriminator.** That evidence was committed two rungs
  ago and was read as *"a log that also contains a successful `nvidia-smi`"*; the stronger
  reading is that the chain **is** `nvidia-smi`'s as well.

**4. ⊘ And one refutation of my own, mid-increment.** I first filed
`NULL != pGpuState->pRootInternal @ gpu_vaspace.c:3332` under *"teardown, so
`scrubber_teardown_is_not_the_wall` applies"*. **Reading the callers refuted that** (§16.29.3).
★ **A line that LOOKS like teardown can be the rollback arm of the failing call itself** — the
position of a log line in time does not tell you which function emitted it.

### 16.29.2 ★★★ THE MEASUREMENT — both userspace planes, one process, one clock

`scripts/bench/guest_cuinit_wall.sh` (new) chains **both** interposers. Chaining is sound rather
than a coin flip: each resolves its next hop with `dlsym(RTLD_NEXT, "ioctl")` and `RTLD_NEXT`
searches the load order *after* the calling object, so `app → rm → uvm → libc`, each passing
through what it does not claim. ⊘ Both trace files are asserted non-empty and `cup2.err` is
printed, because that is where `ld.so` reports a preload it could not load.

```text
UVM ioctl fd=12 cmd=0x30000001 (UVM_INITIALIZE)          rmStatus = 0x00000000 (NV_OK)
UVM ioctl fd=13 cmd=0x0000004b nr=75 (UNKNOWN — not in the table)
UVM ioctl fd=12 cmd=0x00000027 (UVM_PAGEABLE_MEM_ACCESS) rmStatus = 0x00000000 (NV_OK)
UVM ioctl fd=12 cmd=0x00000025 (UVM_REGISTER_GPU)        rmStatus = 0x00000056  ← THE FAILURE
UVM ioctl fd=12 cmd=0x30000002 (UVM_DEINITIALIZE)
```

`UVM_REGISTER_GPU`'s IN block decodes cleanly and unremarkably — `rmCtrlFd = 0xffffffff` (-1,
i.e. no SMC partition), `hClient = 0`, `hSmcPartRef = 0`, `numaEnabled = 0` — so **nothing about
the request is malformed**; only the answer is.

★ **The RM ioctl plane is CLEAN at the point of failure.** Of 96 traced RM calls, exactly **two**
carry a non-zero status — `0x20810108` and `0x2080012f`, both `0x56`, both early (trace lines 39
and 48), and **both already on the unserviced ledger**. Every one of the last 25 calls returns
`status=0x00000000`, ending in ordinary `FREE`s. ⇒ **`cuInit` does not fail on anything libcuda
asks RM for.** It fails inside `nvidia-uvm`'s kernel path, which reaches RM through in-kernel
internal clients — invisible to any userspace interposer, which is why the *dmesg* is the
instrument for the rest of it.

★★ **Invariance control: the instrument changed nothing observable.** All five census lines are
**byte-identical** to `s26`'s — commands 454/84/38, bridge refusals 18/6, controls 130/45,
isolates 2/2/2, doorbells 24/24/0 last token `0x00010001`. A double `LD_PRELOAD` in the guest is
therefore observational on the device plane.

### 16.29.3 ★★★★ THE DISCRIMINATING DIFF — four lines, and only one of them is new

`dmesg` was cleared immediately before `cup2`, so the hook's capture is `cuInit`'s **alone**;
`run_s27_c73d3ab_uvm_dmesg.log` is `nvidia-smi`'s window, and it returned `SMI_RC=0`. Timestamps
and handles normalised, the two windows differ by exactly four lines, all on `cup2`'s side:

```text
kgmmuClientShadowFaultBufferUnregister_IMPL: … failed (status=0x56), proceeding...   ┐ the three
kgmmuFaultBufferReplayableDestroy_IMPL:      … failed (status=0x56), proceeding...   ├ logged-and-
uvmTerminateAccessCntrBuffer_IMPL:           … failed (status=0x56), proceeding...   ┘ proceeded pairs (§14.41)
nvAssertFailedNoLog: Assertion failed: NULL != pGpuState->pRootInternal @ gpu_vaspace.c:3332
```

Three are the register/unregister pairs §14.41 already ruled non-fatal at zero cost. **The fourth
is the only line in this boot that is neither shared with a successful device open nor already
known to be harmless.**

### 16.29.4 ★★★★★ WHAT IT NAMES — `0x00801813`, and the chain closes from RM's own source

`gpu_vaspace.c:3332` sits inside **`gvaspaceExternalRootDirRevoke_IMPL`**. That function has
exactly **three** call sites, and two are eliminated by measurement:

| site | what it is | verdict |
|---|---|---|
| `gpu_vaspace.c:1251` (`_gvaspaceGpuStateDestruct`) | VA-space teardown | ⊘ **impossible** — guarded by `if (NULL != pGpuState->pRootInternal)`, i.e. by the very condition the assert checks |
| `dma.c:629` | the handler for `NV0080_CTRL_CMD_DMA_UNSET_PAGE_DIRECTORY` (`0x801814`) | ⊘ **did not run** — `0x801814` appears **nowhere** in this boot's census, neither served nor unserviced |
| `dma.c:539` | the **rollback arm of `NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY`** (`0x801813`) | ★ the only one left |

And `dma.c:508-530` is the mechanism, verbatim:

```c
if (IS_VIRTUAL_WITH_SRIOV(pGpu) || IS_GSP_CLIENT(pGpu)) {
    NV_RM_RPC_CONTROL(pGpu, hClient, hDevice,
                      NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY, …, status);
    if (status != NV_OK) SLI_LOOP_BREAK;          // ← the local commit is SKIPPED
}
status = gvaspaceExternalRootDirCommit(pGVAS, hClient, pGpu, pParams);
…
if (status != NV_OK) { gvaspaceExternalRootDirRevoke(pGVAS, pGpu, &params); … }
```

⇒ **The chain, end to end, every link cited:**

1. `0x00801813` = `NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY` (`ctrl0080dma.h:828`) is **demanded
   and REFUSED by this port** — `nvkvm: unserviced fn 76 cmd 0x00801813`, in both `s26` and
   `s27`.
2. Because the RPC fails, RM `SLI_LOOP_BREAK`s **before** `gvaspaceExternalRootDirCommit`, so
   `pGpuState->pRootInternal` is never populated.
3. The failure's own rollback calls `gvaspaceExternalRootDirRevoke`, which asserts
   `NULL != pGpuState->pRootInternal` and returns `NV_ERR_INVALID_STATE` — **the one line unique
   to `cuInit`'s window.**
4. The guest's Device VA space therefore **never gets its page-directory root installed**.

★★★ This is the same object §16.28's fourth route is about — the publication of a
page-directory root under the Device's default VA space. Route 4 taught this port to *resolve*
such a publication; `0x801813` is the guest **asking us to install one**, and we refuse it.

### 16.29.5 ⚠ TWO THINGS THIS DOES **NOT** ESTABLISH — named, so the next rung does not assume them

- ⊘ **Who issues the SET is not settled.** `nvUvmInterfaceSetPageDirectory` is called from
  `uvm_gpu.c:1305` (`configure_address_space`, on `UVM_REGISTER_GPU`'s path) **and** from
  `uvm_va_space.c:1394`. ⚠ The `UVM_ERR_PRINT("nvUvmInterfaceSetPageDirectory() failed…")` at
  `uvm_gpu.c:1312` **did not appear** in this boot's dmesg — all 28 lines are `NVRM:`. So
  attributing this particular SET to that call site would be an inference, not a measurement.
  ★ `read_the_caller_not_the_id`: the ledger names an id; only the caller names a function.
- ⊘ **`0x801814` is missing from the wire and RM's source says it should be there.**
  `dma.c:541-548` sends the `UNSET` RPC unconditionally inside the same rollback block, and our
  census shows it neither served nor unserviced. ⇒ Either the census has a **blind spot** for
  it, or the branch differs in the shipped build. ⚠ **"The census does not show it" is not
  "it did not happen"** (`a_saturated_instrument_looks_exactly_like_absence`). Settle which,
  before anything is built on the count.

### 16.29.5b ★★★ WHAT THE REFUSED CONTROL ACTUALLY CARRIES — and why it is route 4's own object

`NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_PARAMS` (`ogkm-580: ctrl0080dma.h:790-828`) is a **page-directory
publication stated outright**, not one that has to be inferred:

- `physAddress` — physical address of the new page directory, in the aperture named by `flags`
- `numEntries` — its size in entries
- `hVASpace` — ★★★ *"handle for the allocated VA space that this control call should operate on.
  **If it's 0, it assumes to use the implicit allocated VA space associated with the
  client/device pair.**"*
- `chId`, `pasid`, and an `ALL_CHANNELS` flag meaning *"update the instance blocks for all
  channels using the VAS"*

⇒ ★★★ **This is exactly the object §16.28 spent a rung recovering by inference.** Route 4 resolves
a publication for the **Device's default VA space**; `hVASpace = 0` in this control *is* that VA
space, named by the guest itself. And `ALL_CHANNELS` names the instance-block update that
§16.28.13 filed as *"checked and named, not done"*.

★ So serving `0x801813` is not answering a question we cannot answer — it is accepting a fact the
guest is **handing** us, in the vocabulary the address table already speaks
(`no_real_phys_only_gpga_or_gpa`: `physAddress` here is a guest-physical address, which is what
the table stores). ⊘ That is the opposite of forging a completion: nothing is invented, and the
guest's own `gvaspaceExternalRootDirCommit` still does the local half.

⚠ ⊘ **But it is NOT free and must not be waved through.** Answering `NV_OK` while recording
nothing would be a refusal wearing an acceptance's clothes — the guest would proceed believing
its root is installed on the GSP side. The rung must record the publication and be able to show
it, or refuse by a name that is true.

### 16.29.6 ⇒ THE NEXT RUNG, stated as a falsifiable prediction

**Serve `0x00801813`.** If §16.29.4 is right, the boot after it must show **both** of:

1. `NULL != pGpuState->pRootInternal @ gpu_vaspace.c:3332` **gone** from `cuInit`'s dmesg window;
2. `UVM_REGISTER_GPU rmStatus` **≠ `0x56`** — moved, not necessarily to `0`.

⊘ If `cuInit` still fails with the assert gone, the chain held and the wall moved. If the assert
survives, §16.29.4 is refuted and the elimination in its table is where to look first.

### 16.29.7 ⊘ THE HOOK'S OWN FIRST RUN LOST AN ARTIFACT

`guest_cuinit_wall.sh` printed **excerpts** and left `/tmp/rmtrace.txt` and `/tmp/uvm.txt` inside
the guest; `boot_capture.sh` phase 4 then powered the guest down, and a later `scp` produced two
**zero-byte** files. The full UVM plane survives (it was printed in full), but the middle ~50
lines of the RM trace are **unrecoverable for this boot** — which is why §16.29.2 quantifies the
RM plane by its two non-zero rows and its last 25 calls rather than by a diff.

★ Same shape as every trap this repository already records: *the operation succeeds either way,
and only an explicit copy separates "observed" from "kept where anybody will find it later."*
⊘ **A new instrument's first run is itself unverified** — this is the second time in three rungs
that a freshly written harness was wrong on its first tagged use (§16.28.7's gate dropped a
disjunct and failed a good boot). Fixed in the same commit: the full traces are now emitted into
the probe log **and** copied to the bench directory before anything powers anything off.

## §16.30 ★★★★ SERVE `0x00801813` — with the falsifier written BEFORE the boot, and sharpened

`[built 2026-08-09]`, not yet booted. This section is written **before** the boot so the
boot can refute it. ⊘ Nothing here is a measurement of the guest; §16.30.5 is the only
part a boot can settle, and it is stated as a prediction with three readings, exactly one
of which confirms §16.29.4.

### 16.30.1 ⊘⊘ WHAT THIS INCREMENT REFUTES — including two claims in my own brief

**1. ★★★★ `3686b8b`'s subject line over-claims, and the commit body contains both halves.**
`3686b8b` is titled *"the refused `0x801813` CARRIES a page-directory publication — **route
4's own object, named**"*. Two propositions are folded together there and only one is
sourced:

| claim | status |
|---|---|
| *"`hVASpace = 0` means the client/device pair's implicit VA space"* | ★ **CORRECT AND CITED** — `ogkm-580: ctrl0080dma.h:812-815` says it in words. This is the header speaking, not us. |
| *"the SET names route 4's object"* | ⊘ **NOT MEASURED.** The header says what a `0` *would* mean. It does not say the guest **sent** a `0`. |

⇒ The gap is exactly **one `u32` read off the wire**, and it has never been read. ★ The
counter-hypothesis is live and specific: the C artifact's own notes make `0x801813` UVM's
transport for **user** VA spaces — the ones a kernel-internal VAS never takes. If the guest
sends a non-zero handle, the SET and route 4 are about **different objects**, the
convergence between §16.28 and §16.29 is coincidence, and every doc asserting it is
fiction. ⚠ **A non-zero handle is a finding worth its own rung, not a nuisance.**

**2. ★★★★ §16.29.6's falsifier is TOO STRONG, and this increment refuses to inherit it.**
It reads: *"if the assert survives, §16.29.4 is refuted."* That conflates *"the RPC was the
blocker"* with *"the RPC was the **only** blocker."* Answering `NV_OK` gets RM past
`dma.c:508-520` and no further: `gvaspaceExternalRootDirCommit` then runs **locally** and
can still fail on any of eight of its own checks (`ogkm-580: gpu_vaspace.c:3057, 3067,
3085, 3088, 3093, 3094, 3097, 3109`), and a failure there takes the **same**
`SLI_LOOP_BREAK` into the **same** rollback and fires the **same** assert at `:3332`.
⇒ §16.30.5 restates the falsifier three-valued. ★ It is still falsifiable — sharper, not
weaker: the discriminating reading is now *narrower* than "the assert survived."

**3. ⊘ §16.29.5's second open item offered two options and the true one was neither.** It
said `0x801814`'s absence meant *"either the census has a blind spot for it, or the branch
differs in the shipped build."* See §16.30.3: there is a third, it is RM's own macro, and
it converts the absence from a hole into **corroboration**.

**4. ⊘ And one of mine, before it reached a doc.** I first read
`gvaspaceExternalRootDirCommit`'s `:3109` assert — `SHARED_MANAGEMENT || externallyOwned` —
as *refuting* the chain outright, since the `:3332` assert is only reachable on a VAS that
is **not** externally owned (`gpu_vaspace.c:3320-3328` returns early for one). It does not:
`SHARED_MANAGEMENT` **without** externally-owned is a legal combination, so the chain
survives. ★ An assert that constrains a path is not an assert that closes it.

### 16.30.2 ★★★ THE ELIMINATION TABLE, RE-DERIVED RATHER THAN INHERITED

§16.29.4 eliminated two of `gvaspaceExternalRootDirRevoke_IMPL`'s three call sites. Both
eliminations **survive** a first-hand re-read, and one is now stronger than recorded:

| site | verdict | why, re-derived |
|---|---|---|
| `gpu_vaspace.c:1251` | ⊘ impossible | ★ **confirmed verbatim**: the call sits inside `if (NULL != pGpuState->pRootInternal)` (`:1246-1253`), which is exactly the condition `:3332` asserts. Inside the guard the assert cannot fire. |
| `dma.c:629` | ⊘ did not run | ★ **stronger than "the census lacks `0x801814`"**: that handler initialises `status = NV_OK` (`dma.c:582`) and RPCs at `dma.c:606-615` **before** it revokes at `:629`. Had it run, its own `0x801814` would have been on the wire *ahead of* the assert. |
| `dma.c:539` | ★ the only survivor | the rollback arm of `0x801813` |

★ `read_the_caller_not_the_id`, applied to an inherited conclusion: the predecessor was
right, and the reason it is now safe to build on is that it was checked, not that it was
repeated.

### 16.30.3 ★★★★ WHY `0x00801814` IS ABSENT — the third option, and it CORROBORATES

`ogkm-580: src/nvidia/inc/kernel/vgpu/rpc.h:223-242`:

```c
#define NV_RM_RPC_CONTROL(pGpu, hClient, hObject, cmd, pParams, paramSize, status)  \
    do {                                                                            \
        OBJRPC *pRpc = GPU_GET_RPC(pGpu);                                           \
        NV_ASSERT(pRpc != NULL);                                                    \
        if ((status == NV_OK) && (pRpc != NULL))          /* ← the guard */         \
        …
```

**`NV_RM_RPC_CONTROL` is a no-op when `status != NV_OK` on entry.** The rollback block at
`dma.c:531-551` runs **precisely because `status != NV_OK`**, and `status` is *not*
reassigned before the `UNSET` macro is reached — `gvaspaceExternalRootDirRevoke`'s return
is **discarded** at `:539`. ⇒ ★★★ **The `UNSET` RPC is structurally unsendable from the
rollback arm.** Its absence is not a blind spot, not a different build; it is what RM
guarantees.

★★ **And it discriminates.** `dma.c:629`'s RPC is issued with `status` freshly `NV_OK`, so
*that* path would have shown `0x801814`. The absence therefore argues **for** `dma.c:539`
and **against** `dma.c:629` — it is evidence, and it points the same way §16.29.4 did.

⊘ **The instrument was checked before the absence was trusted**
(`a_saturated_instrument_looks_exactly_like_absence`). `s27` is **not** saturated on any of
the three lists a control can land on: **38 distinct** unserviced rows printed against a
sample cap of **64**; all **45** served-control rows printed, of which exactly **one**
carries a non-zero result (`0x2080012b`, `x2 REFUSED`); and 6 named bridge-refusal kinds,
none page-directory. `0x801814` is on none of them.

### 16.30.4 ★ WHAT WAS BUILT

`crates/kayfabe-device/src/setpagedir.rs` — a chain link that answers `0x00801813` `NV_OK`
and **latches what it accepted**: both header handles and all seven params fields. Seated
among the answering links beside `bar2::BarPdePolicy`.

- ⊘ **It records, because answering while recording nothing is §16.29.5b's refusal wearing
  an acceptance's clothes.** The record crosses the shim (ABI **32 → 33**, nine words) and
  is printed unconditionally by `qemu/hw/misc/nvkvm/nvkvm.c`.
- ★★★ **`set_page_dir_valid` is a separate word and it is load-bearing.** `hVASpace == 0`
  is a *real handle value*, so a reported `0` without a latched bit beside it cannot be
  told from *"no SET ever arrived"* — and **a zero nobody watched arrive is a non-claim,
  not a measurement.** The `else` branch of the printer says so in words rather than
  printing zeros. This is the C oracle's `dlen=0` shape (`c_oracle_empty_rows_are_wrong`)
  and it is refused the same way.
- ⊘⊘ **It does NOT** create a `Vas`, populate `Channel::vas_pdb`, or relax any downstream
  refusal — `gvaspub`'s reason exactly: a served-but-inert data path converts a *loud*
  refusal into a *silent* timeout.
- ★ **Targeting is proved, not asserted.** `tests/set_page_directory.rs::seating_the_link_changes_no_other_reply_byte`
  drives the whole production chain with and without the link over six ids and compares
  reply bytes. The link claims **one id**; `WantedTable::from_cmd` has no arm for it and
  `kayfabe_rmrpc::OBJECT_CONTROLS` (`policy.rs:891-904`) lists three ids and this is not
  one of them.
- ★ **The tests were bite-checked**, because a green test I wrote myself is unverified
  (`the_bite_check_that_could_not_bite`): forcing `respond` to decline turned **8 of 11**
  red, and the 3 that stayed green are the decline/empty-log cases, which *should* be
  unaffected. The shim's own size gate also bit on the way in — **16800 vs 16728**, a
  72-byte delta that is 9 × 8 exactly.

### 16.30.5 ⇒ THE FALSIFIER, in two parts, both written BEFORE the boot

**(a) The chain.** Three readings, and only one confirms §16.29.4:

| next boot's `cuInit` dmesg window | reading |
|---|---|
| `:3332` **gone** and `UVM_REGISTER_GPU rmStatus ≠ 0x56` | ★ **§16.29.4 CONFIRMED**; the wall moved. |
| `:3332` **survives**, and a **new** assert appears from `gpu_vaspace.c:3057-3109` | ★ **§16.29.4 CONFIRMED** — RM got *past* the RPC and failed in the LOCAL `commit`. The wall moved *inside* `gvaspaceExternalRootDirCommit`. ⊘ This is the reading §16.29.6 would have mis-scored as a refutation. |
| `:3332` **survives, alone and unaccompanied** | ⊘ **§16.29.4 REFUTED.** The revoke came from somewhere the elimination table did not reach, and §16.30.2 is where to look first. |

★ The middle row is readable **only because every one of those eight is an `NV_ASSERT*`
and therefore logs its own `file:line`.** That is what makes the falsifier three-valued
rather than a coin flip.

**(b) The handle.** The boot log must print the **observed** `hVASpace`, and the boot's
commit must state which branch it took:

- `set_page_dir_valid = 0` ⇒ ⊘ **the rung was not exercised at all** — say so, and read
  nothing else on the line. A `0` from an unwritten field is reading back our own silence.
- `set_page_dir_valid = 1, hVASpace = 0` ⇒ the guest named the client/device pair's
  implicit VA space, and *only then* is §16.28's convergence a measured claim.
- `set_page_dir_valid = 1, hVASpace ≠ 0` ⇒ ★★★ **a finding.** The SET is about a VA space
  the guest named explicitly, `3686b8b`'s subject line is wrong, and route 4 is about a
  different object. That earns its own rung.

⊘ (b) is a **print, not a gate**: the control is served either way. ⊘ And no doc, refusal
name or log string in this increment says *"route 4's object"* — the wire has not been
asked yet.

## §16.31 ★★★★★ BOOTED `s28_933a709_spd` — §16.29.4 CONFIRMED, and the wall moved INSIDE `commit`

`[measured 2026-08-09, boot `s28_933a709_spd` at `933a709`]`, binary **and** archive both
stamped `kayfabe-rev:933a709dbc49c107d8f6ab60bf70183bfe69c2c9`, evidence
`traces/guest_boots/run_s28_933a709_spd_{qemu,dmesg,probe}.log` +
`_{rmtrace,uvm}.txt`, all five non-empty and checked non-empty **before** being read.

⊘ **Stated first: `cup2` is still `FAIL cuInit(0) -> initialization error (3)` (`CUP2_RC=1`).**
Nothing was made to work. A refused control became a served one and the failure moved
**two source lines**, from RM's RPC gate into RM's own local commit.

### 16.31.1 ★★★★ THE FALSIFIER, SCORED — and it landed on the row §16.29.6 could not see

`0x00801813` is **served**: `control 0x00801813 result 0x00000000 x1`, and it is **gone**
from the unserviced list. Then, 102 microseconds apart:

```text
[53.111956] NVRM: nvAssertFailedNoLog: Assertion failed: vaLimitNew <= pGVAS->vaLimitMax @ gpu_vaspace.c:3094
[53.112058] NVRM: nvAssertFailedNoLog: Assertion failed: NULL != pGpuState->pRootInternal @ gpu_vaspace.c:3332
```

That is **exactly** §16.30.5's middle row: `:3332` survives, accompanied by a **new** assert
from `gvaspaceExternalRootDirCommit`'s range.

⇒ ★★★★ **§16.29.4 is CONFIRMED.** RM got **past** the RPC gate at `dma.c:508-520` — which
is the entire content of the claim — ran `gvaspaceExternalRootDirCommit`, and failed one of
that function's **own local** checks. `:3332` is then its rollback firing exactly as
§16.29.4 said it would.

⊘⊘ **§16.29.6's falsifier as written would have scored this a REFUTATION.** *"If the assert
survives, §16.29.4 is refuted"* — the assert survived, and §16.29.4 is right. ★★★★ The
sharpening was not pedantry: it was load-bearing on the **first** boot that used it, and it
was only possible because RM's checks are `NV_ASSERT*` and therefore log their own
`file:line`. **A falsifier that cannot tell "the blocker" from "the only blocker" scores a
confirmation as a refutation.**

★★ **Corroborated from the other end of the stack, by an independent instrument.**
`UVM_REGISTER_GPU rmStatus` moved **`0x56` → `0x1f`** — falsifier (a)'s second half, which
asked only for *"moved, not necessarily to 0"*. And `0x1f` is `NV_ERR_INVALID_ARGUMENT`,
which is precisely what `NV_ASSERT_OR_RETURN(vaLimitNew <= pGVAS->vaLimitMax,
NV_ERR_INVALID_ARGUMENT)` at `:3094` returns. ⇒ The status the guest's *userspace* sees and
the assert in the guest's *kernel* name the same failure, measured separately.

### 16.31.2 ★★★★★ THE HANDLE — read off the wire, and `3686b8b`'s premise is REFUTED

```text
nvkvm: SET_PAGE_DIRECTORY (0x00801813): 1 ACCEPTED, 0 refused; latest hClient 0xc1d0000a
       hObject 0xcaf00000 hVASpace 0xcaf00005 physAddress 0x200000 numEntries 4 flags 0x8 (aperture 0)
```

⊘⊘ **`hVASpace = 0xcaf00005`, NOT `0`.** `3686b8b`'s subject line — *"route 4's own object,
**named**"* — rested on the guest sending `0` so that the header's *"if it's 0, it assumes
the implicit VA space associated with the client/device pair"* would apply. **The guest
sends an explicit handle.** The premise is refuted; the header citation was always sound and
was always about a case that does not occur here.

★★★★★ **And the conclusion survives anyway — by a different, now-MEASURED route.** The same
boot's route-4 census carries:

```text
gvas cmd 0x90f10106 hClient 0xc1d0000a hObject 0xcaf00005 va [0x100000000..0x11fffffff] pageSize 0x200000 levels 4
```

`(hClient 0xc1d0000a, hVASpace 0xcaf00005)` **≡** `(hClient 0xc1d0000a, hObject 0xcaf00005)`.
⇒ **Two page-directory transports, two independent decodes, one boot, and they name the SAME
VA space** — the very pair `gvaspub.rs` records as *"the one `cuInit` walls on"*. This is the
"two independent derivations" §16.29.5b asked for, and they **agree on identity**.

★★★ **The finding is the shape of the agreement, and it is worth more than the agreement.**
A true conclusion was being carried by a false premise. Had the guest sent `0`, the port
would have had to *infer* the VA space from the client/device pair; instead the guest names
it, and the name matches independently. ⇒ ⚠ **A claim that happens to be right is not a
claim that was measured** — and a doc asserting the `hVASpace = 0` mechanism would have been
fiction that survived every review precisely because its conclusion checked out.

⊘ **Also settled: neither branch the brief predicted was correct.** My coordinator's third
branch read *"non-zero ⇒ route 4 is about a **different** object"*. It is the **same**
object, reached by an explicit handle rather than by the implicit-VAS rule.

### 16.31.3 ⊘ ONE INFERENCE OF MINE, MADE AND RETRACTED WITHIN THE SAME READING

Seeing `va [0x100000000..0x11fffffff]` beside the failing `vaLimitMax` assert, I first read
it as *"the VA space covers 512 MiB, so `vaLimitMax ≈ 0x11fffffff`"* — which closes the
arithmetic beautifully. ⊘ **It is wrong.** **All eleven** publication rows in this boot carry
the **identical** `va [0x100000000..0x11fffffff] pageSize 0x200000 levels 4`, across four
different clients and six different objects. A range that is identical for every VA space in
the system is the **server-reserved PDE window** RM copies for itself, not any VAS's limit.

★★ A number that fits the hypothesis and is **constant across every row** is describing the
instrument, not the subject. `observed_error_plus_plausible_mechanism`: the mechanism was
plausible, the error was real, and the join between them was invented.

### 16.31.4 ★★★ THE INVARIANCE CONTROL — the change is targeted, and the census proves it

| census line | `s27` | `s28` |
|---|---|---|
| commands decoded | 454 | **454** |
| commands UNSERVICED | 84 total, 38 distinct | **83 total, 37 distinct** |
| controls answered | 130, 45 distinct | **131, 46 distinct** |
| bridge refusals | 18 total, 6 distinct | **identical** |
| isolates | 2/2/2 | **identical** |
| doorbells | 24/24/0, last `0x00010001` | **identical** |
| VA-space publications | 12 total, 11 distinct, 0 UNDECODABLE | **identical** |

⇒ **Exactly one command moved from the unserviced list to the served list and nothing else
changed.** −1/−1 unserviced, +1/+1 served, every other line byte-identical. That is what
makes the two `dmesg` windows comparable at all.

### 16.31.5 ⇒ THE NEXT WALL, stated with its arithmetic and with what is NOT known

`gpu_vaspace.c:3091-3094`:
```c
vaLimitNew = mmuFmtEntryIndexVirtAddrHi(pGpuState->pFmt->pRoot, 0, pParams->numEntries - 1);
NV_ASSERT_OR_RETURN(vaLimitNew >= pGVAS->vaLimitInternal, NV_ERR_INVALID_ARGUMENT);  // :3093 PASSED
NV_ASSERT_OR_RETURN(vaLimitNew <= pGVAS->vaLimitMax,      NV_ERR_INVALID_ARGUMENT);  // :3094 FAILED
```

**What is measured:** `numEntries = 4` (off the wire, this port's own record); `:3093`
passed; `:3094` failed. ⇒ `vaLimitInternal <= vaLimitNew` and **`vaLimitNew > vaLimitMax`**:
four entries of `pGpuState->pFmt->pRoot` cover **more VA than the VA space's own maximum**.

**What is NOT measured, and must not be assumed:** the value of `pFmt->pRoot->virtAddrBitLo`
on the guest side, and the value of `pGVAS->vaLimitMax`. ⚠ Neither appears in any log this
boot produced. The arithmetic *"4 entries at shift 47 covers 2⁴⁹, so `vaLimitMax < 2⁴⁹−1`"*
is a **hypothesis**, and the shift it names is this port's belief about GA106, not a reading
of the guest's format.

★ Two candidate rungs, and the first is cheap:
1. **Make the guest print both numbers.** They are two `NvU64`s in a struct the guest owns;
   an `UVM_ERR_PRINT`-shaped probe or a `gvaspaceGetVaLimit` read settles `vaLimitMax`
   directly, and `pFmt->pRoot` is reachable from the same `pGpuState`. ⊘ Until then any
   claim about *which side is wrong* is unsourced.
2. **Suspect what THIS port advertises about the GMMU.** `vaLimitMax` is fixed at VA-space
   construct and `pFmt` is chosen from the GMMU static info this device serves, so a
   mismatch between the format we advertise and the root size UVM sizes against would
   produce exactly this assert. ★ `two_encodings_agreeing_on_the_first_values` is the
   cautionary precedent: two encodings can agree on every value anyone has checked and
   diverge on the one that matters.

⊘ **`0x801814` still does not appear**, in either boot — as §16.30.3 predicted from
`NV_RM_RPC_CONTROL`'s `(status == NV_OK)` guard. The rollback ran (its assert is in the log)
and the RPC was suppressed. ★ A prediction made from source before the boot and confirmed by
it.

## §16.40 ★★★★ The instrument was ALREADY BUILT — and gated behind a plane that started succeeding

### 16.40.1 ⊘⊘ WHAT THIS INCREMENT REFUTES, starting with the brief that commissioned it

- ⊘ **"`ContextVasUndeclared` names a VA space and nothing logs WHICH ONE. Bump the shim ABI,
  log the handles, get a same-boot identity."** — **REFUTED as to the diagnosis.** The
  per-channel VA-space census that names exactly this (`own=`/`cs=`/`tsg=`/`dev=` route strings
  plus `pdb=Y|N` per channel) has existed since §15, lives at
  `crates/kayfabe-qemu-raw/src/shim.rs`'s `vas_census_line`, and **already crossed the ABI**
  inside the doorbell-refusal sentence. Nothing needed inventing.
  ★ `[measured 2026-08-09]` `grep -l 'census\[' traces/guest_boots/*.log` returns **two** files —
  `run_s24_cf18883_cup2_qemu.log` and `run_s25_01d12e6_cup2_qemu.log` — and **none** of the
  fifteen boots since. The reason is not decay: the census was reachable **only from inside a
  doorbell refusal**, and `s35_03a7e10_dup` reports `doorbells: 124 arrived, 124 served, 0
  REFUSED by name`. ⇒ **a diagnostic for the ADDRESS plane was gated on the EXECUTION plane
  failing.** Fixing the execution plane silenced the address plane's only instrument, and the
  boot report has no line that says so — there is only an absence.
  ★★★ The ABI bump in this increment is therefore real but its justification is inverted: it
  carries an **existing** instrument out from behind a **stale gate**, rather than adding a new
  measurement.

- ⊘ **"`docs/design/execution_plane_increments.md` §16.38–§16.39b"** — **those sections are not in
  that file.** The document ends at §16.31; §16.32 onward exist **only as commit messages**. A
  brief that cites a doc section by number, and a reader who opens the file, disagree silently.

- ⊘ **My own first design, refuted before it was booted.** The latch was written against
  `ObjectModel::as_gpu()`. `SharedObjectModel::as_gpu` returns **`None` by design**
  (`shim.rs:2482`) because the shipped composition root is a sharded shell — so the diagnosis
  would have printed *"no whole `Gpu`"* **on every real boot** while passing every test that
  composes a bare `Gpu`. That is `skipped_oracle_kills_the_guard`: green in the harness, blind
  on the bench. The census is a **trait method** now, so `rustc` requires the shell to answer.

- ⊘ **"`fuzz-run`/`fuzz-corpus-replay` were still running and unread."** They were finished and
  gone. What was **still running** was **two orphaned `cargo-mutants` processes** (1 h 05 m and
  57 m old) holding **11.8 GB** of `/tmp` and still growing — the same orphan class as the
  fuzzers, from the same `pkill`ed suite, and not named in any hand-off. They were the reason
  disk sat at the 6144 MB floor. Killed by exact pid; **free space went 6.4 G → 19 G.**
  ★ `pgrep -x cargo-mutants` finds them; a sweep that greps only for `qemu` or `fuzz` does not.

### 16.40.2 ★★★★ THE DUAL-SOURCED REFUSAL — one tag stood for two opposite diagnoses

`route_promote_ctx` returned `PromoteFault::ContextVasUndeclared` from **two** places with
**identical payloads** (`promote.rs`, hops 2 and 3):

| hop | the lookup | a miss means | whose defect |
|---|---|---|---|
| 2 | `Spine::ctx_vas` | no `(gpu, pdb)` was ever derived for this channel/TSG — **its VA space declared no page-directory base** | the guest has not published a root, or we did not route the publication |
| 3 | `Spine::by_pdb` | a `(gpu, pdb)` **was** derived and **no proc owns it** | our own projection disagreeing with itself |

⇒ *"the root never arrived"* versus *"the root arrived and the owner index lost it"* — opposite
diagnoses, one tag, and a census that counts tags could not tell a reader which had happened.
`s35` printed `PromoteFault::ContextVasUndeclared x1` and three rungs read it as hop 2 because
that is the reading the variant's doc comment invited; **nothing in the capture could have
refuted hop 3.** Hop 3 is now `PromoteFault::ContextVasNoOwner`, carrying the `Pdb` it resolved.

★ It is guest-observationally neutral, checked at the source rather than assumed:
`BridgeRefusal::rpc_result` is a `const fn` returning `NV_ERR_NOT_SUPPORTED` **for every
variant** (`lib.rs:861-863`), so the split changes a report and cannot change a boot.

★★ And the split immediately caught a **live disagreement in our own test suite**:
`tests/tests/promote_ctx.rs:1315` asserted `ContextVasUndeclared` under a comment reading *"a
promotion naming the **dead proc's** address space"*. The comment named hop 3; the assertion
took hop 2's name. `a_comment_that_names_an_exception_is_a_bug_report`, and the compiler found
it the moment the two names existed.

### 16.40.3 ★★★ WHAT `s35`'s OWN CAPTURE ALREADY SETTLES — at zero boot cost

Verified against the raw files rather than inherited (the brief's duty 1):

- **The unserviced ledger is complete and consistent.** `... | grep -o 'fn [0-9]*' | sort -n |
  uniq -c` over `run_s35_03a7e10_dup_qemu.log` returns exactly **39 × `fn 76`**, matching the
  summary's own `39 distinct`. `DUP_OBJECT` (fn 21) is absent because §16.38 **served** it.
- **The dup is fully named, and by the GUEST.** `run_s31_675af4a_echofix_probe.log:307`:
  `GspRmDupObject failed: hClient=0xc1d0000a; hParent=0xcaf00000; hObject=0xcaf00036;
  hClientSrc=0xc1d00015; hObjectSrc=0x5c000007`. ⇒ UVM dups **VASPACE #1** into `0xcaf00036`,
  and `0x00801813` publishes a root for `0xcaf00036`. Meanwhile `0x90f10106` publishes for
  **`0x5c000008`** — VASPACE #2 (`s35` gvas rows). **Both** libcuda VA spaces get a root, by
  **different transports**; only the second's transport is routed into the object model
  (`PUBLICATION_CONTROLS` holds exactly `0x90f10106` and `0x20800a9f`; `0x00801813` is not in it).
- **`cup2`'s object tree**, from its own `rmtrace`: client `0xc1d0000c`; two `0x90f1` at
  `0x5c000007`/`0x5c000008` under Device `0x5c000002`; TSG `0xa06c` at `0x5c000012`; channel
  `0xc56f` at `0x5c000019` **parented to the TSG**; `0xc7c0` at `0x5c00001a` under the channel,
  `status=0x56`. ⇒ the channel's VAS resolves through **route 3 (the TSG)**, so the handle that
  decides this rung is the **TSG's** declared `hVASpace`, which no capture reads.
- ⚠ **Two promotions already SUCCEED** (`control 0x2080012b result 0x00000000 x2`) beside the
  three refused. So `ctx_vas` resolves for *some* context objects; the wall is not "the index is
  empty".

### 16.40.4 ★★★ A MEASURED INSTRUMENT DEFECT, recorded and NOT yet fixed

`s35` prints `of those, 12 reached the object model, 10 ACCEPTED (Vas::pdb populated from the
guest's own publication)`. **That parenthesis is not something the counter can support.**
`PublicationObserver::observe` increments `applied` on `Ok(())` from `gpu.apply(ev)`, and
`RmGraph`'s `SetPageDir` arm returns `Ok(())` on **both** arms — the resolved one (`res.pdb =
Some(pdb)`) and the **parked** one (`pending_pdbs.insert(target, pdb)`, for a VA space whose
handle does not resolve yet). ⇒ a publication for a VA space that **does not exist** is counted
as ACCEPTED and printed with the words *"`Vas::pdb` populated"*.

⊘ Not fixed in this increment, deliberately: it needs `apply` to report which arm it took, and
this boot must change instruments only. It is named here so the next reader does not take
`10 ACCEPTED` as ten populated PDBs. ★ Same family as §16.40.1's gating accident — a number that
is true of the code and false of the sentence printed beside it.

### 16.40.5 ⇒ THE FALSIFIER, THREE-VALUED, WRITTEN AND COMMITTED BEFORE THE BOOT

The boot changes **no behaviour** (§16.40.2's neutrality check). It adds one line to the report:
`nvkvm: promote-ctx FIRST REFUSAL: <tag> <fault Debug>` + ` census[N chans, M outcomes] {...}`.
Enumerated from source, these are the outcomes and what each **means**, so that a confirmation
cannot be scored as a refutation:

| # | what the line says | reading | what it makes the NEXT rung |
|---|---|---|---|
| **A** | `PromoteFault::ContextVasUndeclared` + a `pdb=N` group whose route is `tsg=ok(h0x5c00000X=>…)` | **hop 2.** The TSG named a VA space, it resolved, and it has **no PDB**. `X` is the answer to leg 1 — read it directly. | route a page-directory base to *that* VA space. If `X=7`, it is the UVM-dup'd one and `0x00801813` is its only transport ⇒ `PUBLICATION_CONTROLS` gains `0x00801813`. If `X=8`, the transport is already routed and the defect is downstream of `translate_published_pdes`. |
| **B** | `PromoteFault::ContextVasNoOwner` + a `pdb=Y` group | **hop 3.** A `(gpu,pdb)` exists and `by_pdb` names no owner — an internal disagreement, **not** a missing publication. | ⊘ **This outcome refutes the whole `b4f00f3` hypothesis**, which is entirely about getting a PDB onto the wire. Routing `0x00801813` would change nothing. Go to `project.rs`'s `by_pdb` arm: the `if let Some(gpu)` guard at `:1177` drops a VA space whose Device target has not resolved. |
| **C** | `census[NO-LIVE-CHANNELS]` on either fault | the promotion was refused at a moment when the model held **no channel at all** | the wall is earlier than believed — the promote precedes the channel reaching the graph. Neither A nor B's fix applies; instrument the ordering. |
| **D** | `promote-ctx: NO REFUSAL LATCHED` | no `GPU_PROMOTE_CTX` was refused. ⊘ Cross-check `control 0x2080012b result 0x00000056` in the same report: **present** ⇒ the refusal bypassed `Bridge::deliver` and the latch is in the wrong seat (an instrument defect, not a finding); **absent** ⇒ the wall genuinely moved. | fix the seat, or climb. |

⚠ **The three-valued discipline, restated for this rung:** outcomes A and B are *both*
confirmations that the instrument works and the wall is where §16.39 said — they disagree only
about **which hop**, and that disagreement is the entire point of the boot. Only C and D say the
instrument or the model is wrong. A two-valued falsifier ("does the promote still fail?") would
score A, B and C identically and learn nothing.

★ **Prediction, from source, before the boot:** **A**, with `tsg=ok(...)` and `pdb=N`. The
channel is parented to the TSG (`rmtrace`), so route 1 declines and route 3 commits; and of the
two libcuda VA spaces only `0x5c000008`'s transport reaches the object model. ⊘ Recorded so the
boot can refute it — the last five rungs each refuted part of their own brief.

## §16.41 ★★★★ BOOTED `s36_3a0146c_vascensus` — the instrument FIRED, and it measured the WRONG EVENT

`[measured 2026-08-09, rev `3a0146c`, RTX 3060 / GA106, binary stamped
`kayfabe-rev:3a0146cd…` and verified before the boot]`. Evidence:
`traces/guest_boots/run_s36_3a0146c_vascensus_{qemu,dmesg,probe}.log`.

### 16.41.1 ★★★ THE INVARIANCE CONTROL — s36 reproduces s35 refusal for refusal

The commit changed instruments only, and the boot proves it rather than asserting it. Every
refusal row is identical to `s35_03a7e10_dup`'s:

```
bridge refusals: 19 total, 7 distinct
  AllocClassNotPermitted::NotOnAllowlist x2   AllocClassNotPermitted::Refused x2
  ReservedClient x2                           UnmappedAllocClass x3
  PromoteFault::ContextVasUndeclared x1       PromoteFault::UnknownContextObject x2
  RmGraphError::FreeUnknown x7
control 0x2080012b result 0x00000000 x2 · result 0x00000056 x3 REFUSED
doorbells: 124 arrived, 124 served, 0 REFUSED by name
```

⇒ the fault-variant split is observationally neutral to the guest, as §16.40.2 predicted from
`rpc_result`'s source.

### 16.41.2 ★★★★ FALSIFIER OUTCOME **B IS REFUTED** — the wall is hop 2, measured

`PromoteFault::ContextVasNoOwner` **does not appear in this boot**, and
`ContextVasUndeclared x1` does. Since §16.40.2 split the two hops apart, that is now a
*measurement* rather than a reading:

⇒ ★★★ **The refusal is `Spine::ctx_vas` (hop 2): no `(gpu, pdb)` was ever derived for the
failing context object.** It is **not** the owner index losing a root it had. `b4f00f3`'s
hypothesis — that a page-directory base is missing for the VA space the channel names —
**survives its first test that could have killed it**, and the alternative that would have
made routing `0x00801813` pointless is eliminated.

⊘ This is exactly the ambiguity three previous rungs could not resolve: `s35` printed the same
`ContextVasUndeclared x1` and it stood for *either* hop. One name per hop settled it in one
boot, at zero extra cost.

### 16.41.3 ⊘⊘ AND THE DIAGNOSIS LATCHED THE WRONG REFUSAL — my own instrument, refuted

The new line printed, and what it printed was:

```
promote-ctx FIRST REFUSAL: PromoteFault::UnknownContextObject
  UnknownContextObject { client: HClient(3251634184), object: HObject(826366208) }
  census[2 chans, 2 outcomes]
    {1x pdb=N own=not-declared cs=not-declared tsg=mid-miss(h0xa,wrong-kind(Device))
         dev=dev-default(dev0xa=>h0xc) p0/c0:vc1 Ce c0xc1e00005/0x2}
    {1x pdb=Y own=ok(h0xa=>c0xc1e00006/0xa) cs=not-attempted tsg=not-attempted
         dev=not-attempted p0/c1:vc2 Ce c0xc1e00006/0x2}
```

`3251634184` = `0xc1d00008`, `826366208` = `0x31415900` — **kernel RM's** promotion, refused
long before `cup2` ran, with a census of the **two CE channels** alive at that instant. The
refusal this rung is about was never latched, **because it was not first.**

★★★ **The defect is in the word "first", and I imported it from a precedent that does not
transfer.** `KayfabeDoorbellRefusal` latches the first because its flood is *identical rings
from one guest*, so first is representative. A boot's promote refusals are **several distinct
refusals from different callers** — kernel RM, UVM, libcuda — and "first" selects the earliest,
which is the one nobody asked about. ⊘ *"Bounded"* was the requirement; *"first"* was one
implementation of it, and I copied the implementation instead of re-deriving the requirement.

★ It is `a_correct_capture_can_answer_the_wrong_question` a second time in one increment, and
note how well it hides: the line is **present**, **well-formed**, **internally consistent**, and
its census is **true** — two channels really were live at that instant. Nothing about the output
says it is answering a different question. Only knowing that `0x31415900` is not a libcuda handle
separates it from the answer.

⇒ **FIXED at §16.41.4**: the latch is keyed on the [`FaultTag`], so each *kind* of refusal
carries its own first. Still bounded and still guest-independent — `PromoteFault` has ten
variants, a fixed finite set, so a guest drives the counts and never the number of rows.

### 16.41.4 ⚠ THE BUILD TRAP FIRED, and the standing warning caught it BEFORE the boot

The first build produced a `qemu-system-x86_64` stamped **`kayfabe-rev:03a7e10`** — the
*previous* rung's revision — while `libkayfabe_qemu_raw.a` was correctly stamped `3a0146c`.

Cause: I exported `CARGO_TARGET_DIR=/workspace/bench/cargo-target`, so `cargo` built the archive
there, while `scripts/build_qom_shim.sh` copies `$REPO/target/release/libkayfabe_qemu_raw.a` —
a **stale file from 20:08 that still existed**, so the script's `[ -f "$ARCHIVE" ]` guard passed
and it linked the old archive against the new C shell.

⊘ The build exited **0** and the binary contained my new `promote-ctx` strings (they live in
`nvkvm.c`, which *is* copied fresh), so every cheap signal said the build was current. Only the
revision stamp disagreed. ⇒ ★ `CLAUDE.md`'s *"any bench claim must carry the SOURCE REVISION it
was measured at"* is not bookkeeping; it is the only check that fires here. **Do not set
`CARGO_TARGET_DIR` when running `build_qom_shim.sh`** — the script's archive path is not derived
from it.

★ Note what would have happened otherwise: ABI 34's C header against the ABI-33 archive, caught
at realize by `kayfabe_shim_abi_version() != KAYFABE_SHIM_ABI`. The version check would have
turned a silent wrong-binary boot into a named refusal — which is what it is for — but the boot
would have been spent.

### 16.41.5 ⇒ WHAT `s36` STILL DOES NOT SAY

⊘ **Which VA space the failing channel names is STILL unread.** Outcome A's discriminator — the
route string and `pdb=Y|N` for `cup2`'s own channel — needs the `ContextVasUndeclared` row, and
that row was not latched. The next boot is the same instrument with §16.41.3's fix, and the
falsifier of §16.40.5 stands unchanged: **A** (with `tsg=ok(h0x5c00000X…)`, `pdb=N`) versus
**C**/**D**. **B is already eliminated.**

## §16.42 ★★★★★ BOOTED `s37_0dfe7f7_pertag` — leg 1 MEASURED, the chain CLOSES, and my prediction was wrong about the ROUTE

`[measured 2026-08-09, rev `0dfe7f7`, binary stamped `kayfabe-rev:0dfe7f7add…` and verified
before the boot]`. Evidence: `traces/guest_boots/run_s37_0dfe7f7_pertag_{qemu,dmesg,probe}.log`.

### 16.42.1 ★★★★★ THE LINE THAT ANSWERS THE RUNG

```
promote-ctx refusals: 2 distinct kind(s), each with the VA-space census AS IT STOOD AT ITS FIRST refusal
  promote-ctx PromoteFault::ContextVasUndeclared:
    ContextVasUndeclared { client: HClient(3251634188), object: HObject(1543503897) }
    census[7 chans, 4 outcomes]
      …
      {1x pdb=N own=not-declared cs=ok(h0x5c000007=>c0xc1d0000c/0x5c000007)
           tsg=not-attempted dev=not-attempted p2/c0:vc7 GrCompute c0xc1d0000c/0x5c000019}
```

`3251634188` = **`0xc1d0000c`**, `1543503897` = **`0x5c000019`**. Both are `cup2`'s own, and
both are corroborated by `cup2`'s `rmtrace` **from the same boot**:
`ALLOC hClass=0x0000c56f hRoot=0xc1d0000c hParent=0x5c000012 hObject=0x5c000019`.

⇒ **the refused promotion is `cup2`'s `AMPERE_CHANNEL_GPFIFO_A`**, its VA space resolves to
**`0x5c000007`**, and that VA space has **`pdb=N`**. Leg 1 of `b4f00f3`'s three is measured.

### 16.42.2 ⊘ MY PREDICTION WAS WRONG — right VA space, WRONG ROUTE, and the difference matters

§16.40.5 predicted `tsg=ok(...)`: the channel is parented to the TSG `0x5c000012`, so I
reasoned route 3 would commit. The boot says **`cs=ok(h0x5c000007=>c0xc1d0000c/0x5c000007)`**
and **`tsg=not-attempted`**.

★ Route **2** — the `FERMI_CONTEXT_SHARE_A` — commits first and `resolve_channel_vas` never
reaches the TSG (each route *commits*; there is no fall-through). ⊘ A parent handle is not a
routing answer: the channel's `hParent` really is the TSG, and the VA space still comes from
its `hCtxShare`. Reading the alloc tree told me the right VA space **for the wrong reason**,
and a fix written against the TSG's declared `hVASpace` would have been aimed at a hop that
never runs.

★★ This is why the census prints all four routes and not just the winner: `own=not-declared
cs=ok(…) tsg=not-attempted dev=not-attempted` is a complete account of *why* this VA space and
not another. A one-line "vas=0x5c000007" would have been true and would not have caught me.

### 16.42.3 ★★★★★ THE CHAIN, END TO END, ALL FROM MEASURED FACTS

| # | fact | source |
|---|---|---|
| 1 | `cup2`'s channel `0x5c000019` declares no `hVASpace`; its **CtxShare** names `0x5c000007` | `s37` census, this boot |
| 2 | `0x5c000007` is libcuda's **FIRST** `FERMI_VASPACE_A` (of two: `…07`, `…08`) | `s35`/`s37` `rmtrace` |
| 3 | UVM **dups `0x5c000007`** into `0xcaf00036` | `GspRmDupObject … hObject=0xcaf00036; hObjectSrc=0x5c000007`, `run_s31_675af4a_echofix_probe.log:307` |
| 4 | `0x00801813` publishes a root **for `0xcaf00036`** | `SET_PAGE_DIRECTORY … hVASpace 0xcaf00036 physAddress 0x201000`, every boot since `s35` |
| 5 | `0x00801813` is **not** in `PUBLICATION_CONTROLS`, so that root never reaches the object model | `policy.rs`, source |
| 6 | libcuda's *second* VA space `0x5c000008` publishes through `0x90f10106`, which **is** routed | `s35`/`s37` gvas rows |
| 7 | ⇒ VASPACE #1 has no `pdb` ⇒ `Spine::ctx_vas` misses ⇒ `ContextVasUndeclared` (hop 2, per §16.41.2) | this boot |

★★★ **Two VA spaces, two publication transports, and only one of them routed.** The port has
been answering `NV_OK` to the other since §16.30 and writing it into a report.
⊘ *"Recording is not forwarding"* — the sentence `PublicationObserver`'s own docs already make
about `0x90f10106`, unnoticed one control over.

### 16.42.4 ⇒ THE FIX, and why it is an OBSERVER entry

`PUBLICATION_CONTROLS` gains `NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY`. **No new mechanism**:
`translate_control` already produces `RmEvent::SetPageDir` for it, `RmEvent::SetPageDir` sets
`pdb` on the **resource**, and a `Dup` binds the alias to the source's resource id — so a root
published under `0xcaf00036` lands on `0x5c000007`'s resource, which is what `ctx_vas` resolves
through. This list was the entire missing route.

⚠ §14.21 measured this control being *claimed*, killing the adapter, and being reverted. That
risk is **answering** it with a status the guest's error path reads. `SetPageDirPolicy` keeps
answering it, byte for byte; `PublicationObserver` is a `CommandObserver` whose `observe`
returns nothing to return and so **cannot change a reply**. Identical shape to `0x90f10106`'s.

### 16.42.5 ⇒ THE FALSIFIER FOR THE FIX BOOT, three-valued, committed BEFORE it

| # | what the report says | reading |
|---|---|---|
| **P** | `promote-ctx refusals` loses the `ContextVasUndeclared` row (or that census row turns **`pdb=Y`**), **and** `control 0x2080012b result 0x56` drops from `x3` | the route was **sufficient**. Leg 3 answered. Next wall is whatever `AMPERE_COMPUTE_B` hits after `kgrobjPromoteContext`. |
| **Q** | the `cs=ok(h0x5c000007…)` row turns **`pdb=Y`** but the promotion is refused under a **different** name (`ContextVasNoOwner`, `UnknownVas`, `ForeignContextObject`) | the route was **necessary and not sufficient** — `injection_measures_necessity_never_sufficiency`. ★ This is a CONFIRMATION of §16.42.3, not a refutation: the PDB arrived; a *later* hop refuses. The new name says which. |
| **R** | the row is still **`pdb=N`** | the route did not land. ⊘ Then check `VA-space page-directory publications` for a **new refusal**, and `PdbCollision` in particular: `0x5c000008` already claims a root, and two VA spaces claiming one `(gpu, pdb)` is a loud projection error. |
| **S** | a *new* `RmGraphError` appears, or publications-accepted falls | the observer's apply is refused where the recorder's was not. Read the tag; the graph refuses by name. |

⊘ **Q is the outcome I expect to be scored wrongly**, so it is written first: a two-valued
"did cup2 get further?" would call it a failure. It is the hypothesis being *confirmed* and the
wall moving one hop deeper, which is what six of the last eight rungs have actually produced.

⚠ And §16.40.4's recorded defect now matters directly: `10 ACCEPTED (Vas::pdb populated)`
counts **parked** publications as accepted. If this fix lands and the row stays `pdb=N`, that
counter cannot tell whether the root was applied or parked for a handle that never resolved —
read the `pdb=Y|N` in the promote census, which is a fact about the channel, not about our
bookkeeping.

## §16.43 ★★★★★ BOOTED `s38_411d280_route` — **OUTCOME Q**, the route LANDED, and the wall moved one hop deeper

`[measured 2026-08-09, rev `411d280`, binary stamped `kayfabe-rev:411d2803e…` and verified
before the boot]`. Evidence: `traces/guest_boots/run_s38_411d280_route_{qemu,dmesg,probe}.log`.

### 16.43.1 ★★★★★ THE ROUTE LANDED — five independent numbers moved, all in the same direction

| fact | `s37` | `s38` |
|---|---|---|
| `cup2`'s channel `0x5c000019`, `cs=ok(h0x5c000007…)` | **`pdb=N`** | ★ **`pdb=Y`** |
| `cup2`'s live GrCompute channels | 1 | **8** |
| channels in the census | 7 | **14** |
| publications reaching the object model / accepted | 12 / 10 | **14 / 12** |
| doorbells | 124 | **163** |
| `control 0x2080012b result 0x00000000` | x2 | **x3** |
| `PromoteFault::ContextVasUndeclared` | **x1** | ⊘ **gone** |

⇒ §16.42's chain is **confirmed end to end**. The page-directory root UVM published under the
dup alias `0xcaf00036` now reaches `0x5c000007`'s **resource**, which is what `Spine::ctx_vas`
resolves through, and the VA space `cup2`'s channel names has a PDB for the first time.

### 16.43.2 ★★★★ THE NEW WALL, NAMED — and it is `PromoteFault::ForeignContextObject`

```
promote-ctx PromoteFault::ForeignContextObject:
  ForeignContextObject { client: HClient(3251634186), object: HObject(1543503897), owner: ProcId(2) }
```

`3251634186` = **`0xc1d0000a`** — **UVM's** client. `1543503897` = **`0x5c000019`** —
**`cup2`'s** channel. `ProcId(2)` = `cup2`'s proc.

⇒ **UVM issues `GPU_PROMOTE_CTX` naming a channel that belongs to `cup2`'s address space**, and
the cross-namespace guard refuses it: the envelope's `hClient` is not in the component that owns
the address space being promoted into. `cuCtxCreate` still ends at
`kgrobjPromoteContext … NV_ERR_NOT_SUPPORTED @ kernel_graphics_object.c` — **same symptom, a
different and deeper mechanism.**

★★★ ⚠ **AND THE CHECK REFUSES A CASE RM's OWN SOURCE DOCUMENTS AS LEGAL.** This is not a
guess; `translate_promote_ctx`'s own rustdoc already states it, from `ogkm-580:
kernel_graphics_object.c:130-135`:

> RM sets `params.hChanClient = RES_GET_CLIENT_HANDLE(pChannelDescendant)` and then issues the
> control with `RES_GET_CLIENT_HANDLE(pSubdevice)` as the envelope client; the two are usually
> equal and **are not required** to be.

`s38` is the boot where they are **not** equal: envelope `0xc1d0000a` (UVM's subdevice client),
`hChanClient` `0xc1d0000c` (`cup2`'s). ⊘ So the port has a correct citation, in the right file,
describing exactly this case — and the check written beside it treats the inequality as hostile.
★ `a_correct_citation_narrowed_by_the_reading`, and this one was **written down before the boot
that needed it**: the doc says "usually equal, not required", the code took "not equal ⇒ foreign".

⊘ **Do not simply delete the check.** Its docstring records what it is for: *"without this, a
client may declare bindings in a **victim's** address space by naming the victim's client and
channel — the params-field injection the C could not even detect."* The question the next rung
owns is **what makes UVM's kernel client part of a CUDA process's component**, which is the
`proc_is_not_a_set_of_rm_clients` question, not a licence to open the guard.

### 16.43.3 ⊘ WHAT ELSE MOVED, recorded so it is not read as noise

`AllocClassNotPermitted::NotOnAllowlist` x2 → **x3** and `RmGraphError::FreeUnknown` x7 → **x8**.
Both are consistent with `cup2` getting **further** (eight channels instead of one, so more
allocs and more frees), but neither is *measured* to be — they are recorded here as unexplained
deltas rather than absorbed into the win. ⚠ `NotOnAllowlist` in particular is a class the guest
asked for and we refused; the next rung should name which class before assuming it is benign.

### 16.43.4 ★ THE FALSIFIER, SCORED

**Q**, and it was written **first** precisely because a two-valued reading scores it as failure:
`cup2` still fails `cuCtxCreate` with the same `801`. But the route was the hypothesis, the route
landed, and seven numbers moved. ⊘ **P** (sufficient) is refuted — the route was **necessary and
not sufficient**, exactly as `injection_measures_necessity_never_sufficiency` warns and as
`b4f00f3`'s third unmeasured leg asked. **R** and **S** are refuted: no publication was refused,
no new `RmGraphError` tag appeared, and accepted publications went **up**.

⇒ All three of `b4f00f3`'s named legs are now measured: **(1)** the VA space is `0x5c000007`, via
the **CtxShare**; **(2)** the refusal belonged to `cup2`'s own channel `0x5c000019`; **(3)** the
route is **necessary and not sufficient**.

## §16.44 ★★★★★ THE MEMBERSHIP RULE — and the question it was posed as is the WRONG QUESTION

`[rev to be stamped at the boot]`. This section is written **before** `s39` and its falsifier is
committed with it.

### 16.44.1 ⊘⊘ REFUTED — "what makes UVM's kernel client part of a CUDA process's component?"

§16.43 handed the next rung that question. **Nothing makes it part of that component, and
nothing may.** §12.27 assigns every declared `ClientKind::Kernel` client to the ONE reserved
system component *by rule and never by dup*, and a dup **into** a kernel client is defined
there as a **reference, not a merge** — precisely so that UVM's single global session client
cannot fuse every CUDA process on the box into one blast radius. Widening `Proc::clients` to
admit it would have deleted the isolation this port exists to provide, in the name of fixing it.

★ And `s38`'s own census says so in numbers, which is what settles it rather than the argument
(`traces/guest_boots/run_s38_411d280_route_qemu.log:162`):

```
{4x pdb=Y … tsg=ok(h0xcaf00005=>c0xc1d0000a/0xcaf00005)
     p0/c6:vc3 Ce c0xc1d0000a/0xcaf00012  p0/c7:vc4 Ce c0xc1d0000a/0xcaf0001d …}
{8x pdb=Y … cs=ok(h0x5c000007=>c0xc1d0000c/0x5c000007)
     p2/c0:vc7 GrCompute c0xc1d0000c/0x5c000019 …}
```

⊘ The four channels UVM's client holds are **`p0`** — the system proc — carrying UVM's *own*
`0xcaf…` handles in UVM's *own* TSG. `cup2`'s eight are `p2`. The briefing's reading that "UVM's
client is already inside this component by another path" is **refuted by the proc column**: it is
inside the *system* component by another path, which is a different component.

### 16.44.2 ★★★★ THE CITATION WAS TO A SITE THAT ESTABLISHES THE OPPOSITE

`promote.rs`'s module doc and `translate_promote_ctx`'s rustdoc both cite
`ogkm-580: src/nvidia/src/kernel/gpu/gr/kernel_graphics_object.c:130-135` for *"the two are
usually equal and are **not required** to be"*. ★ The lines are real and were verified this rung
(`:131` sets `params.hChanClient = RES_GET_CLIENT_HANDLE(pChannelDescendant)`; `:136` issues the
control with `RES_GET_CLIENT_HANDLE(pSubdevice)`) — ⚠ **but on that path they are ALWAYS equal**,
because `:74-79` obtains the subdevice with
`subdeviceGetByDeviceAndGpu(RES_GET_CLIENT(pKernelGraphicsObject), pDevice, pGpu, &pSubdevice)`
— *in the graphics object's own client*. The cited site is the counter-example to its own claim.

★★★ The site where they genuinely differ is **`nvGpuOpsBindChannelResources`**
(`ogkm-580: src/nvidia/src/kernel/rmapi/nv_gpu_ops.c:10870`, `:10891-10893`):

```c
pParams->hChanClient = RES_GET_CLIENT_HANDLE(pKernelChannel);   // the USER's client
pParams->hObject     = RES_GET_HANDLE(pKernelChannel);
status = pRmApi->Control(pRmApi,
                         retainedChannel->session->handle,      // UVM's session client
                         retainedChannel->rmSubDevice->subDeviceHandle,
                         NV2080_CTRL_CMD_GPU_PROMOTE_CTX, …);
```

That is `s38`'s envelope `0xc1d0000a` / `hChanClient` `0xc1d0000c` exactly. ⇒ **The claim was
true, the citation was to the wrong site, and the check written beside it took the SITE's
behaviour rather than the CLAIM's.** ⊘ This is not `a_comment_that_names_an_exception`, which is
what §16.43 filed it as: the comment named the exception *and cited a source that does not
contain it*, so anyone who followed the citation to check would have found the code correct.
★ **Opening the cited file is what catches this; reading the sentence above the citation is what
does not** — the inverse of `a_correct_citation_narrowed_by_the_reading`.

### 16.44.3 ★★★★★ THE RULE, STATED — and it is RM's, not ours

> A promotion's **acting** (envelope) client may write into an address space it is not a
> component member of **iff it is a declared `ClientKind::Kernel` client.**

This is not a licence invented to get past a wall; it is the gate RM itself applies on the only
path that produces the shape. Reaching `nvGpuOpsBindChannelResources` requires a live
`UVM_CHANNEL_RETAINER` (`0xc574`), which RM registers

```
/* Internal Class */ UvmChannelRetainer,
/* Parents        */ RS_LIST(classId(Device), classId(KernelChannelGroupApi)),
/* Alloc Param    */ RS_REQUIRED(NV_UVM_CHANNEL_RETAINER_ALLOC_PARAMS),
/* Flags          */ RS_FLAGS_ALLOC_KERNEL_PRIVILEGED | …
```

(`ogkm-580: src/nvidia/src/kernel/rmapi/resource_list.h:394-400`), and whose constructor resolves
the *named* client with `serverGetClientUnderLock` and applies **no ownership test at all beyond
that privilege gate** (`ogkm-580: src/nvidia/src/kernel/gpu/fifo/uvm_channel_retainer.c`). ⇒ we
permit exactly what RM permits, and no more — `refuse_by_name_means_the_name_is_true`.

⊘ **The injection still refuses, and the reason is a DECLARED field, not a convention.**
`ClientKind::Kernel` comes from `NV0000_ALLOC_PARAMETERS.processID == 0xFFFF_FFFF`
(`kayfabe_abi::GuestOs::client_kind_from_process_id`), written by the guest's **kernel** RM. A
hostile CUDA process is a user client, is outside the owning component, and lands on
`ForeignContextObject` exactly as before. ⚠ Note what this does **not** defend against: a
compromised guest *kernel*. It never did — that is the host boundary's job, not this guard's.
Both halves are pinned against the same handles and the same ranges by
`a_kernel_client_may_promote_into_a_user_procs_vas_and_a_foreign_user_client_may_not`.

### 16.44.4 ★★★ THE INSTRUMENT — and it already existed, twice

★★★★★ Per the standing rule *"before building any instrument, search for it — and search for it
DISABLED"*, three of the four numbers this rung wanted were **already being printed**:

| wanted | where it already is |
|---|---|
| how many promotions, how many accepted | `control 0x2080012b result 0x00000000 x3` / `result 0x00000056 x3` — **6 promotions, 3 `NV_OK`, 3 refused** in `s38` (`…_qemu.log:192-193`); `s37` was `x2`/`x3` |
| which class the `NotOnAllowlist` delta is | the **guest's own** `GspRmAlloc failed: … hClass=…` lines in `run_<tag>_probe.log` |
| which channels are in which proc | the VA-space census's `p<proc>/c<chan>` prefix |

⊘ **No new instrument was built for any of them.** The one thing genuinely missing was that
`ForeignContextObject` — the fault that is *about two clients disagreeing* — printed only one of
them, so §16.43 had to **infer** `hChanClient` from a census exemplar and from the previous
boot's differently-shaped fault, and wrote the inference into this document looking like a quoted
field. It now carries `chan_client`. That is a struct field on an existing variant: the shim
prints the fault's `Debug`, so it costs **zero ABI change**.

### 16.44.5 ★★★★ THE `NotOnAllowlist` DELTA IS NAMED, AND IT IS NOT BENIGN

§16.43.3 asked the next rung to name the class behind `NotOnAllowlist x2 → x3` before assuming it
was benign. Diffing `s37`'s and `s38`'s guest-side `GspRmAlloc failed` lines names it with **no
code and no boot** (`run_s38_411d280_route_probe.log:97`):

```
NVRM: rpcRmApiAlloc_GSP: GspRmAlloc failed: hClient=0xc1d0000a; hParent=0xcaf0003e;
      hObject=0xcaf00041; hClass=0x0000c574; paramsSize=0x00000008; status=0x00000056
```

`0xc574` = **`UVM_CHANNEL_RETAINER`**; `paramsSize=8` = exactly its two `NvHandle`s. It is the
**only** class `s38` asks for that `s37` did not, it appears in no other boot in the tree, and it
is absent from `CLASSES_SHARED`. ⇒ **UVM tried to legitimise the very cross-namespace channel
reference that then faults as `ForeignContextObject`, and we refused it** — followed at `:98` by
the free of the same handle and at `:101` by
`nvAssertFailedNoLog: Assertion failed: status == NV_OK @ nv_gpu_ops.c:10328`.

★★ ⚠ **AND THAT LEAVES A DISAGREEMENT THIS RUNG DOES NOT RESOLVE — recorded, not absorbed.**
`nvGpuOpsRetainChannel` does `goto error` on that alloc failure (`nv_gpu_ops.c:10225-10232`), and
the `error:` path only frees (`_nvGpuOpsReleaseChannel`, `:10304-10340`). So a failed retain
should mean `nvGpuOpsBindChannelResources` **never runs** — yet its promotion is what we
observe. Either the promote precedes the retainer alloc, or an emitter not in
{`kernel_graphics_object.c`, `nv_gpu_ops.c`, `kernel_falcon.c`} produced it. ⊘ Do **not** read
§16.44.2's attribution as settled on this point: the *client shape* is measured, the *emitting
call site* is inferred. The new `chan_client` field plus ordering in `s39` is what decides it.

⇒ ⊘ **`0xc574` is deliberately NOT admitted in this rung.** Its params declare a real fact — "kernel
client K retains channel `(hClient, hChannel)`" — which is precisely the retention edge a *tightened*
guard would key on instead of the blanket kernel arm. Admitting it as `NoDeclaredFacts` would record
that fact as nothing and burn the evidence, which is the direction `admitting_the_class_is_not_serving_it`
warns about. It is the next rung, with an `AllocParams` shape and a graph edge.

### 16.44.6 ★ THE FALSIFIER FOR `s39`, THREE-VALUED, COMMITTED BEFORE THE BOOT

⚠ Three-valued because several of these checks share one error path, and a two-valued reading
scores a confirmation as a refutation — twice proven in this campaign.

| | outcome | reading |
|---|---|---|
| **P** | `ForeignContextObject` **x0**, `control 0x2080012b` accepted count **3 → 4+**, and `cup2` prints `rv == 0xabcd1234` | sufficient. Climb to `cupctx2_min`. |
| **Q** | `ForeignContextObject` **x0** and the accepted count rises, but `cuCtxCreate` still returns `801` under a **different named wall** | ★ **CONFIRMATION of §16.44.3, not refutation** — the guard was necessary and not sufficient. The new name says which hop. Expect `0xc574` to be it. |
| **R** | `ForeignContextObject` still x1, **or** a *new* promote fault appears (`Malformed`, `Collides`, `TooManyRanges`), **or** a green counter regresses (accepted publications < 12, doorbells < 163, `cup2` GrCompute channels < 8) | refuted — the predicate is wrong or the arm is mis-wired. |
| **S** | no census printed / `cup2` never runs / revision stamp disagrees | **not a result.** Re-run; do not score. |

★ Two sub-predictions, recorded separately so they can fail on their own:
- **(a)** the promotion binds **ZERO** ranges. `nvGpuOpsBindChannelResources` writes only
  `bufferId` and `gpuVirtAddr` — never `gpuPhysAddr`/`size` — so every entry should decode as
  `PromoteEntry::PromoteOnly` and land in `declined.promote_only`. If `bound > 0`, §16.44.2's
  emitter attribution is **wrong** and §16.44.5's disagreement resolves against it.
- **(b)** `NotOnAllowlist` stays **x3** (`0xc574` is untouched this rung) and `FreeUnknown` moves
  only if `cup2` gets further. Neither is a scoring input; they are named so a change is not
  read as noise.

## §16.45 ★★★★★ BOOTED `s39_fd92017_kernelarm` — **OUTCOME Q**, and the next wall was named IN the falsifier

`[measured 2026-08-09, rev `fd92017`, binary stamped `kayfabe-rev:fd9201723d09…` on **both**
`target/release/libkayfabe_qemu_raw.a` and `qemu-build/qemu-system-x86_64`, verified before the
boot]`. Evidence: `traces/guest_boots/run_s39_fd92017_kernelarm_{qemu,dmesg,probe}.log`.

### 16.45.1 ★★★★★ THE GUARD OPENED, AND THE PROMOTE PLANE WENT FROM 3 TO 11

| fact | `s38` | `s39` |
|---|---|---|
| `PromoteFault::ForeignContextObject` | x1 | ⊘ **gone** |
| `control 0x2080012b result 0x00000000` (promotions **accepted**) | x3 | ★ **x11** |
| `control 0x2080012b result 0x00000056` (refused) | x3 | **x2** (both the RC watchdog's) |
| doorbells | 163 | **170** |
| controls answered | 136 | **143** |
| publications total / accepted | 12 / 11 | 12 / 11 (unchanged) |
| `cup2` | fails at `cuCtxCreate` | **fails at `cuCtxCreate`** |

⇒ **Outcome Q**, and it was written first precisely because a two-valued reading scores it as
failure: `cup2` still stops after `cuDeviceTotalMem` with `CUP2_RC=1`. But the guard was the
hypothesis, the guard opened, the accepted-promotion count nearly **quadrupled**, and no green
counter moved backwards. ⊘ **R is refuted** — no new promote fault, no publication regression,
doorbells up. §16.44.3's rule is confirmed: a kernel-privileged client acting on a user client's
channel is a stream the guest's own driver emits, and refusing it was refusing RM.

### 16.45.2 ★★★★★ THE NEW WALL — and the falsifier's own Q row NAMED IT IN ADVANCE

§16.44.6 wrote *"Expect `0xc574` to be it."* It is, and the mechanism is measured — **six**
occurrences of one assert in `run_s39_fd92017_kernelarm_probe.log`:

```
NVRM: nvAssertFailedNoLog: Assertion failed: status == NV_OK @ nv_gpu_ops.c:10328
```

`:10328` is `nvGpuOpsRetainChannel`'s failure return. The lines bracketing it say what failed and
what it cost:

```
NVRM: rpcRmApiFree_GSP: GspRmFree failed: hClient=0xc1d0000a; hObject=0xcaf00059; status=0x00000056
NVRM: … returned from pRmApi->Control(… NVA06F_CTRL_CMD_STOP_CHANNEL …) @ nv_gpu_ops.c:10957
NVRM: … returned from pRmApi->Control(… NV2080_CTRL_CMD_GPU_EVICT_CTX …) @ nv_gpu_ops.c:10977
NVRM: kgmmuClientShadowFaultBufferUnregister_IMPL: … failed (status=0x00000056), proceeding...
NVRM: uvmTerminateAccessCntrBuffer_IMPL: Unloading UVM Access counters failed …
```

⇒ **UVM now gets all the way to retaining `cup2`'s channels — six of them — and every retain dies
on the `UVM_CHANNEL_RETAINER` (`0xc574`) alloc we refuse**, then tears the channel back down
through two more controls we do not serve. The promote plane is no longer the blocker; the
**class allowlist** is. That is `0xc574` promoted from "an unexplained `NotOnAllowlist` delta"
(§16.43.3) to "the named wall", in two rungs, without ever being guessed at.

### 16.45.3 ★★★★ SUB-PREDICTION (b) REFUTED — and the two "unexplained deltas" are ONE EVENT

§16.44.6(b) predicted `NotOnAllowlist` would stay **x3**. It went to **x10**. ⊘ Refuted, mine.

But the miss is informative, because the *other* flagged delta moved by the identical amount:

| | `s38` | `s39` | Δ |
|---|---|---|---|
| `AllocClassNotPermitted::NotOnAllowlist` | 3 | **10** | **+7** |
| `RmGraphError::FreeUnknown` | 8 | **15** | **+7** |

★★★ **They are the same seven events, counted twice.** A refused alloc is never entered in the
object model, so the guest's subsequent `Free` of that very handle cannot resolve — which is
exactly the `GspRmFree failed: hClient=0xc1d0000a; hObject=0xcaf00059`/`0xcaf0005d` pair in the
dmesg, one per dead retainer. §16.43.3 recorded both as "consistent with `cup2` getting further,
but not measured to be"; they are now measured, and the shared cause is `0xc574`.

### 16.45.4 ⊘⊘ SUB-PREDICTION (a) IS **UNSCORED**, NOT CONFIRMED — the readout does not exist

§16.44.6(a) predicted the promotion binds **zero** ranges. ⊘ **Nothing in the boot report can
say.** The promote diagnosis latches only *refusals*; an accepted promotion contributes one
`control 0x2080012b result 0x0` tick and nothing else — no `bound`, no `already`, no
`declined.promote_only`. So the eleven acceptances are eleven opaque successes.

⚠ **Recording this as "unscored" rather than quietly dropping it is the point.** A prediction with
no instrument behind it is not a prediction that passed; `injection_measures_necessity_never_sufficiency`
has a sibling here — **a claim nothing could have contradicted was never a test.** The `PromoteJoin`
already carries all three numbers; only the report throws them away.

### 16.45.5 ★★★★★ FIXING THE PROMOTE PLANE SWITCHED THE CENSUS OFF — the SAME class as last rung

`s38`'s report carried `census[14 chans, 4 outcomes]`: every live channel, its proc, its VA-space
route, its `pdb=Y/N`. `s39`'s carries only `census[2 chans, 2 outcomes]`.

Nothing about the census broke. It is latched **only from inside a promote refusal**, and the
14-channel snapshot was the one attached to `ForeignContextObject` — *the refusal this rung
deleted*. ⊘ So the instrument that measured the win is only reachable while the bug is present,
and closing the bug blinded it.

★★★★★ **This is `a_small_count_is_not_a_small_event`'s cousin and it is now TWICE in two rungs.**
The previous rung's headline finding was that the per-channel VA census *already existed and
already crossed the shim ABI* but was reachable only from inside a **doorbell** refusal, so fixing
the doorbell plane switched it off and it survived in 2 of 17 boot logs. The same structural
mistake was then re-made one plane over, by the same hands, with the lesson already written down.
⇒ **The rule is not "look for disabled instruments"; it is *an instrument hung off a refusal
path has its own deletion scheduled by the fix it exists to guide*.** The census must be emitted
unconditionally at the end-of-run report, not as a rider on a fault.

### 16.45.6 ⚠ AND THE §16.44.5 METHOD DID NOT SURVIVE ITS OWN SECOND USE

§16.44.5 named `0xc574` from the guest's `GspRmAlloc failed: … hClass=…` lines in the probe log,
with no code and no boot. ⊘ In `s39` that method returns **nothing**: `grep -c "GspRmAlloc failed"`
is **0**, because the probe log's dmesg is a *tail* and `s39`'s first retained line is
`[   61.949060]` — the allocs happened earlier and fell out of the window that `cup2`'s assert
flood pushed them from. The class evidence was not absent from the run; it was absent from the
capture, and the capture gave no sign of it.
⇒ The device knows the class — `BridgeRefusal::AllocClassNotPermitted { class, denial }` carries
it — and only `FaultTag`'s `&'static str` collapse drops it before the report. **Name the class in
the refusal row**, and stop depending on a guest-side tail of unbounded depth.

### 16.45.7 ⇒ THE NEXT RUNG

1. **Admit `0xc574` with a real `AllocParams` shape**, not `NoDeclaredFacts`: its two `NvHandle`s
   are the retention edge `(kernel client K) retains (hChanClient, hObject)` — the fact a
   *tightened* `ForeignContextObject` should key on instead of the blanket kernel arm §16.44.3
   installed. That converts the rule from *"a kernel client may"* to *"a kernel client that has
   observably retained this channel may"*, which is strictly narrower and still RM's.
2. `NVA06F_CTRL_CMD_STOP_CHANNEL` and `NV2080_CTRL_CMD_GPU_EVICT_CTX` (`nv_gpu_ops.c:10957`,
   `:10977`) are the next two refusals on the same path — but they are on the **teardown** leg, so
   they may simply stop being asked once (1) lands. ⊘ Do not pre-emptively serve them.
3. Emit the VA-space census **unconditionally** (§16.45.5) and **name the class** in the
   `AllocClassNotPermitted` row (§16.45.6), and report `PromoteJoin`'s three counters
   (§16.45.4). All three are report-side; none needs a new measurement.

## §16.46 ★★★★ HANG THE CENSUS OFF THE **SUCCESS** PATH — an instrument that survives its own fix

⊘ **§16.45.7 item 3 was WRONG as written, and the code said so before the boot did.** It
prescribed *"emit the VA-space census unconditionally at the end-of-run report"*.
`Gpu::vas_census_string`'s own doc already records why that cannot work: *"by the time the
device's exit notifier runs, the CUDA process has exited and its channels are freed, so a
teardown-time call returns `NO-LIVE-CHANNELS` — a true sentence about the wrong instant."*
★ `a_correct_capture_can_answer_the_wrong_question`, and I re-proposed the exact capture that
memory names. The prescription is corrected here rather than quietly replaced.

**What lands instead:** the census is latched on the **accepted** promotion, LAST-wins
(`SharedPromoteDiag::latch_last`). Promotions arrive in the order `cuCtxCreate` builds the
context, so the last one is taken at the deepest point the guest reached — which is the instant
§16.45.5 lost. ⇒ the instrument now rides the path that *survives* the fix instead of the path
the fix deletes.

★ The same latch closes §16.45.4: the row carries `PromoteJoin`'s `bound` / `already` and the
promotion's `declined.promote_only` / `declined.initialize_only` / `entries`, plus the three
handles. `s39`'s eleven acceptances were eleven anonymous ticks; `PromoteJoin` has carried those
numbers all along and only the report threw them away.

⊘ Zero ABI change: it rides `PROMOTE_DIAG_SLOTS` (4, of which `s39` used 1) and prints through
the same shim path as the refusal rows.

### 16.46.1 THE FALSIFIER FOR `s40`, THREE-VALUED, COMMITTED BEFORE THE BOOT

⚠ This is a **pure instrument** rung: nothing the guest can observe changes, so `cup2` must
fail exactly as it did in `s39`. A rung whose success criterion is "the numbers move" would be
scored backwards here.

| | outcome | reading |
|---|---|---|
| **P** | the `ACCEPTED` row appears, carries a census with **more than 2 channels**, and every `s39` guest-facing number is **unchanged** (`ForeignContextObject` x0, `0x2080012b` accepted x11 / refused x2, doorbells 170, `NotOnAllowlist` x10, `FreeUnknown` x15, `CUP2_RC=1`) | the instrument works and is observationally neutral. Sub-prediction §16.44.6(a) becomes scoreable. |
| **Q** | the row appears but its census is still small, or it is `[CLIPPED …]` | ★ partial: the latch fires and the *sampling instant* or the 2048-byte budget is wrong. A real result about the instrument, not about the port. |
| **R** | the row is absent, **or** any `s39` guest-facing number moved | refuted — either the latch never runs on the accepted path, or a "pure instrument" change was not one. ⚠ The second is the serious one. |
| **S** | boot does not reach `cup2` / stamp disagrees | not a result. |

★ **And the number this rung exists to read:** `bound=`. §16.44.6(a) predicted **zero** —
`nvGpuOpsBindChannelResources` writes only `bufferId` and `gpuVirtAddr`, never
`gpuPhysAddr`/`size`, so every entry should decode as promote-only. If `bound > 0`, §16.44.2's
attribution of the emitting call site is **wrong**, and §16.45's open disagreement (a failed
retainer alloc should make that function unreachable, yet its promotion is what we see)
resolves against it.

## §16.47 ★★★★★ BOOTED `s40_4733730_acceptcensus` — **OUTCOME P**, and `bound=0` EXPOSES A TWO-PHASE PROMOTE

`[measured 2026-08-09, rev `4733730`, binary stamped `kayfabe-rev:4733730ea688…` on **both**
artifacts, verified before the boot]`. Evidence:
`traces/guest_boots/run_s40_4733730_acceptcensus_{qemu,dmesg,probe}.log`.

### 16.47.1 THE INSTRUMENT WORKS, AND IT IS OBSERVATIONALLY NEUTRAL

The row `s39` could not produce:

```
promote-ctx ACCEPTED (last, with the census AT it): bound=0 already=0
  declined.promote_only=10 declined.initialize_only=0 entries=0
  client=0xc1d0000a chan_client=0xc1d0000c object=0x5c000037 proc=ProcId(2)
  census[14 chans, 4 outcomes] {…}
```

★ `census[14 chans, 4 outcomes]` — **back**, from `s39`'s `census[2 chans]`, without the bug it
used to ride on. And **every** guest-facing number is byte-identical to `s39`: `0x2080012b`
accepted **x11** / refused **x2**, `NotOnAllowlist` **x10**, `FreeUnknown` **x15**,
`UnmappedAllocClass` x3, `Refused` x2, `ReservedClient` x2, doorbells **170**, `CUP2_RC=1`, no
`ForeignContextObject`. ⇒ **P**: the instrument fires and changes nothing.

### 16.47.2 ★★★★★ `client` AND `chan_client` ARE NOW MEASURED FIELDS, NOT AN INFERENCE

`client=0xc1d0000a chan_client=0xc1d0000c` — printed, from the wire. §16.43.2 asserted exactly
this pair and had to derive it from a census exemplar plus the previous boot's differently-shaped
fault; §16.44.2 built the whole membership argument on it. ⊘ The inference was **correct**, and
it is now not an inference. That is the difference the `chan_client` field was added for.

### 16.47.3 ★★★★★ SUB-PREDICTION (a) **CONFIRMED** — and it is much worse news than it looks

§16.44.6(a) predicted the promotion binds **zero** ranges. `bound=0`, `entries=0`,
`declined.promote_only=10`. ⇒ confirmed, and the entry *shape* independently re-confirms the
emitter: ten entries each declaring a VA and no `gpuPhysAddr`/`size` is precisely what
`nvGpuOpsBindChannelResources` writes — `bufferId` and `gpuVirtAddr` only
(`ogkm-580: nv_gpu_ops.c:10886-10888`). ⊘ **§16.45's open disagreement is resolved in favour of
that attribution**, by the parameter shape rather than by the handles.

★★★★★ **But `bound=0` on ELEVEN acceptances means the promote plane is currently a NO-OP for the
address table.** Eleven promotions were answered `NV_OK`, and the number of VA→backing bindings
they produced is **zero**. `a_wall_that_can_carry_no_name`, inverted: we are answering `NV_OK` and
performing nothing, which is the C's behaviour that this port's crate docs name as the
anti-pattern.

The cause is structural and RM states it in a comment, in the file we already read
(`ogkm-580: src/nvidia/src/kernel/gpu/falcon/kernel_falcon.c:266`):

> `// Promote physical address only. VA will be promoted later as part of nvgpuBindChannelResources`

⇒ **For an externally-owned (UVM) VA space, RM promotes in TWO PHASES**: phase 1 carries
`gpuPhysAddr`/`size` with `bNonmapped = NV_TRUE` and **no VA** (our `initialize_only`); phase 2
carries the VA and **no physical** (our `promote_only`). `PromotedRange` requires *both in one
entry* — `promote.rs`'s "only the both-preparers-ran state reaches here" — so under this shape
**neither phase can ever produce a bindable range**, and the join never happens.

⊘ This is not a bug in the refusal: binding `va → phys 0` really would be manufacturing an
address. It is a **missing join**. And the field to join on is already there and already
documented as the thing the C threw away — `PromotedRange::buffer_id`
(`NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_*`), whose rustdoc says *"Carried rather than dropped.
The C artifact never stored it, so its table could not tell one context buffer from another."*
★ It was carried for this, one rung before anyone knew.

### 16.47.4 ⊘ A LIMIT OF `latch_last`, NAMED IMMEDIATELY

Last-wins shows the **final** accepted promotion and nothing about the other ten. The phase-1
(`initialize_only`) promotions are therefore **invisible** in this row — `declined.initialize_only=0`
describes the last promotion only, and must not be read as "no promotion ever declared a physical
buffer". ⚠ That reading would refute §16.47.3's two-phase account using evidence that cannot
address it. The next rung needs a **per-`buffer_id` tally across all promotions**, not a
prettier single row.

### 16.47.5 ⇒ THE NEXT RUNG, RE-ORDERED BY THIS MEASUREMENT

★ §16.45.7's item 1 (admit `0xc574`) is **demoted**. It is a real refusal on a real path, but
`bound=0` says the address plane would still bind nothing even if every retain succeeded.

1. **★★★★★ Join the two promote phases on `(chan_client, object, buffer_id)`** — phase 1 supplies
   `phys`/`size`/`aperture`, phase 2 supplies `va`; a `PromotedRange` is complete when both have
   arrived, and is refused only when a `buffer_id` is still half-declared at use. This is the
   whole of why eleven `NV_OK`s bind nothing.
2. Instrument it first (§16.47.4): a per-`buffer_id` accumulation census, so the join can be
   scored rather than asserted.
3. Then `0xc574`, with the retention edge (§16.45.7 item 1 unchanged in content, only in order).

## §16.48 ★★★★★ THE TWO-PHASE PROMOTE JOIN — and THREE claims refuted before a line was written

### 16.48.1 ⊘⊘ REFUTED — `:10328` IS NOT `nvGpuOpsRetainChannel`'S FAILURE RETURN

§16.45.2 read the six `Assertion failed: status == NV_OK @ nv_gpu_ops.c:10328` as
*"`:10328` is `nvGpuOpsRetainChannel`'s failure return"* and concluded *"every retain dies on
the `UVM_CHANNEL_RETAINER` (`0xc574`) alloc we refuse"*. ⊘ **Both halves are wrong, and the
file says so.** `nvGpuOpsRetainChannel` ends at `:10309`; `:10325-10329` is inside
`_nvGpuOpsReleaseChannel`:

```c
10325:     if (retainedChannel->hChannelRetainer)
10326:     {
10327:         status = pRmApi->Free(pRmApi, session->handle, retainedChannel->hChannelRetainer);
10328:         NV_ASSERT(status == NV_OK);
```

⇒ it is the **Free** of the retainer, on the **teardown** leg — the `FreeUnknown` half of
§16.45.3's one-event pair, not a retain failing.

★★★★ **And the same probe log proves the retain SUCCEEDED.** Every assert at `:10328` is
followed within 2 ms by `:10957` and `:10977`, and both of those are inside
`void nvGpuOpsStopChannel(gpuRetainedChannel *retainedChannel, …)` (`ogkm-580: nv_gpu_ops.c:10916-10988`)
— a **separate exported function** whose only caller is `uvm_user_channel_stop`, which
returns early unless **both**

```c
769:     if (!user_channel->rm_retained_channel) return;
776:     if (!atomic_read(&user_channel->is_bound))  return;
```

hold (`ogkm-580: kernel-open/nvidia-uvm/uvm_user_channel.c:765-793`). `is_bound` is set only
after `bind_channel_resources()` succeeds (`:686`, `:467`). ⇒ **UVM retained the channel AND
`nvGpuOpsBindChannelResources` returned `NV_OK`.** Phase 2 is not merely reachable — it ran.

⊘ **So `0xc574` is not the wall the last two rungs promoted it to.** It is a teardown-only
symptom. The brief's instruction to spend this rung on the join rather than on `0xc574`
survives — for a *stronger* reason than the brief gave, and the "unless the join proves
blocked by it" escape hatch is **not** taken.

⚠ One thing this does NOT explain and this rung does not claim to: `run_s38_411d280_route_probe.log:97`
shows that alloc answered `status=0x00000056`, yet the retain that depends on it succeeded.
That is a real open disagreement, it is **recorded rather than resolved**, and it is not on
this rung's path.

### 16.48.2 ⊘ REFUTED (MINE) — "phase 2 cannot arrive while `0xc574` is refused"

I derived exactly that from `nv_gpu_ops.c:10231`'s `goto error` before reading the probe log,
and it is **wrong** — §16.48.1 is the measurement that refutes it. Recording it because the
derivation was sound, cited, and beaten by evidence that already existed on disk:
`measure_before_reasoning_is_the_order`.

### 16.48.3 ★★★ CORRECTED — the join key is NOT `(chan_client, object, buffer_id)`

§16.47.5 item 1 prescribed joining on `(chan_client, object, buffer_id)`. ⊘ Handles are
recyclable and `route_promote_ctx` already resolves them to a stable identity at rank 0
before anything keys on them. Both emitters name the **same channel in the same namespace**
— phase 1 sends `RES_GET_PARENT_HANDLE(pChannelDescendant)`
(`ogkm-580: kernel_graphics_object.c:131-133`), phase 2 sends `RES_GET_HANDLE(pKernelChannel)`
(`:10869-10870`) — so both route to the same `(gpu, pdb)`, and the key that lands is
**`buffer_id` within the `Vas`**. ⚠ Where two channels do *not* share a VA space this
orphans the halves instead of joining them; that limit is deliberately **visible**
(`Vas::promote_orphans`) rather than pre-empted by a guess.

★ And `bufferId` is the join field only because RM writes it on **both** legs — but note
`nvGpuOpsBindChannelResources` writes it **only for GR**:
`if (RM_ENGINE_TYPE_IS_GR(rmEngineType)) pParams->promoteEntry[i].bufferId = …` (`:10885-10886`).
Off the GR path every phase-2 entry carries `bufferId = 0`; the falcon leg sets
`bufferId = 0; // unused for flcn` (`kernel_falcon.c:261`) but sends `entryCount = 1`, so the
key stays unique there by accident rather than by design. ⊘ Not a problem today, and named
so it is not discovered as one.

### 16.48.4 ★★★★★ THE MECHANISM, STATED — RM SPLITS ONE PROMOTION ACROSS TWO CONTROLS

| phase | emitter | writes | our decode |
|---|---|---|---|
| 1 — physical | `kgrctxPrepareInitializeCtxBuffer` (`kernel_graphics_context.c:1843-1849`) | `gpuPhysAddr`, `size`, `physAttr`, `bufferId`, `bInitialize=1`, `bNonmapped=1` | `InitializeOnly` |
| 2 — virtual | `nvGpuOpsBindChannelResources` (`nv_gpu_ops.c:10869`, `:10885-10888`) | `bufferId`, `gpuVirtAddr` — the struct is `portMemSet` to 0 and **nothing else is written** | `PromoteOnly` |

The GR path states the split as code: `kgrctxPreparePromoteCtxBuffer_IMPL` opens with
*"RM is not responsible for promoting the buffers when UVM is enabled"* and returns
`*pbAddEntry = NV_FALSE` for an externally-owned VAS (`kernel_graphics_context.c:1883-1885`).
The falcon path states it as prose, in the comment the brief cited and which checks out
(`kernel_falcon.c:217`). ⇒ for a UVM-owned VA space phase 1 **cannot** carry a VA and phase 2
**cannot** carry a physical, so a `PromotedRange` demanding both in one entry could never be
built — eleven `NV_OK`s, zero bindings.

### 16.48.5 WHAT LANDED

- `PromoteHalf::{Physical,Virtual}` — the two phases, carried instead of counted-and-dropped.
- `Vas::promote_halves: BTreeMap<u16, ParkedHalf>` — `ParkedHalf::{AwaitingVa, AwaitingPhysical}`,
  ★ named for what is **missing**, so the two orphan numbers are distinguishable at the call site.
- `apply_promote_ctx` stages the join over a **scratch** copy and commits only if every half
  validates, so it stays all-or-nothing and so two halves inside one control can join each other.
- A completion faces the **ordinary** laws — wrap/zero, self-overlap, table collision — no
  weaker check for having arrived in two pieces.
- `PromoteFault::HalfConflict` — a differing re-declaration of a parked `buffer_id` refuses
  by name; an identical one is `half_already`.
- ⊘ **A zero-length physical half is counted (`half_unusable`) and dropped, never refused.**
  Refusing it was the first draft and would have made a "pure join" change refuse traffic the
  guest already sends (the ABI classifier's last rule routes any all-zero entry here). Parking
  it would have inflated the orphan count with an orphan *we* created.
- `PromoteJoin` gains `joined`, `parked`, `half_already`, `half_unusable`, `orphans`.
  ★ `joined` is counted **apart** from `bound`: "two controls were stitched" and "one entry
  carried both" are different mechanisms, and a rung that summed them could not tell the join
  working from the guest simply having sent a complete entry.
- `SharedPromoteTally` — the **cumulative per-`buffer_id`** row (§16.47.4), on the SUCCESS
  path, riding the existing `PROMOTE_DIAG_SLOTS` (⊘ no ABI change).
- 8 new tests (`tests/tests/promote_ctx.rs`), including both orders, non-joining across ids,
  non-joining across address spaces, and the two negative controls that pin *"a parked half
  must not resolve"*.

### 16.48.6 ★ THE FALSIFIER FOR `s41`, THREE-VALUED, COMMITTED BEFORE THE BOOT

⚠ **Each row must produce a DISTINGUISHABLE line** — §16.45.4's lesson. Enumerated from the
source between the fix and the symptom, the new `ACCEPTED` row and the new `TALLY` row are
what discriminate them:

| | outcome | the line it prints | reading |
|---|---|---|---|
| **P** | the join fires: `TALLY` shows at least one `bid` with **both** `phys>0` and `va>0`, and the `ACCEPTED` row carries `joined>0` | `{bid=0x… phys=N va=M …}` + `joined=K` | ★★★★★ the address plane binds for the first time. §16.48.4's account is confirmed end-to-end. |
| **Q** | halves park but never pair: `TALLY` shows `va>0` on ids whose `phys` is **0**, `joined=0`, `orphans(awaiting_phys>0)` | `joined=0 parked=N orphans(awaiting_va=0,awaiting_phys=N)` | ★ partial and INFORMATIVE — the join is correct and **phase 1 never arrives**. That is a fact about the guest, not the port, and it names the next rung precisely (why does `kgrobjPromoteContext` not run?). ⊘ Not a refutation of the join. |
| **R** | `joined=0` **and** `TALLY` shows some `bid` with both `phys>0` and `va>0` | a paired id beside `joined=0` | ⊘⊘ refuted — the key is wrong (§16.48.3's cross-VAS limit is real and load-bearing), or the parking map is being reset between promotions. `orphans` says which. |
| **R′** | `PromoteFault::HalfConflict` appears | `promote-ctx PromoteFault::HalfConflict` in the refusal census | ★ **INFORMATIVE, not merely bad** — it is `R` stated as a refusal instead of as a silence: the same `buffer_id` in the same VAS was declared with two different values, which is exactly the cross-VAS limit §16.48.3 named. ⚠ It is also a **guest-facing regression** (a control answered `NV_OK` at `s40` is now refused), so it must be reported as both. ⊘ Refusing was chosen over keeping-the-first precisely so this cannot express itself as a *wrong table*. |
| **R″** | any OTHER `s40` guest-facing number moves (`0x2080012b` accepted x11 / refused x2, `NotOnAllowlist` x10, `FreeUnknown` x15, doorbells 170), or `Malformed` appears | a changed refusal census with no `HalfConflict` | ⊘⊘ **the serious one**: a change advertised as a pure join changed the guest's stream through a path nothing predicted. `Malformed` in particular would mean the zero-length arm is not doing what §16.48.5 says. |
| **S** | boot does not reach `cup2`, or the two artefacts' revision stamps disagree | — | not a result. |

⚠ **Amended once, BEFORE the boot and after re-reading the join**: `R′` originally bundled
`HalfConflict` together with "any number moved backwards" under one reading. They are not one
reading — `HalfConflict` is a *diagnosis of the join key* and the other is *an unexplained
regression* — and a single row could not have told the two apart in the report. Recorded as an
amendment rather than silently rewritten.

★ **Predicted, and it is a prediction that can lose:** `cup2` still fails. This rung fills the
GR context-buffer gap under MISS = FAULT; `promote.rs`'s own module doc says that is
*"necessary, narrow, and nowhere near sufficient"*, and the compute working set arrives through
the CE page-table writes, not here. ⇒ **`CUP2_RC=1` is expected in P, Q and R alike**, and a
rung scored on `cup2` alone would read all three as the same failure. The scoreable quantity is
`joined`.

## §16.49 ★★★★★ BOOTED `s41b_62e757f_twophase` — **OUTCOME R**, and R is the ANSWER, not a setback

`[measured 2026-08-09, rev `62e757f`, binary stamped `kayfabe-rev:62e757f1f6f7…` on **both**
`target/release/libkayfabe_qemu_raw.a` and `qemu-build/qemu-system-x86_64`, verified before the
boot]`. Evidence: `traces/guest_boots/run_s41b_62e757f_twophase_{qemu,dmesg,probe}.log`, and a
companion driver-init-only boot `run_s41_62e757f_twophase_*` (no `cup2`).

### 16.49.1 THE CHANGE IS OBSERVATIONALLY NEUTRAL — `R″` REFUTED

Every `s40` guest-facing number is **byte-identical**: `0x2080012b` accepted **x11** / refused
**x2**, `NotOnAllowlist` **x10**, `FreeUnknown` **x15**, `UnmappedAllocClass` x3, `Refused` x2,
`ReservedClient` x2, doorbells **170**, `CUP2_RC=1`, `cuCtxCreate → 801`. No `HalfConflict`, no
`Malformed`, no new `PromoteFault` of any kind. ⇒ a change advertised as a pure join **was** one.

★ `R′` did not fire either, and its absence is a *positive* result: `half_already=10` on the last
promotion means eight channels re-declared the same ten VA halves **byte-identically**. So the
per-VAS key's one live worry — that channels sharing a VA space might carry different VAs for one
`buffer_id` — is **refuted by measurement**, not merely unobserved.

### 16.49.2 ★★★★★ `joined=0` — AND THE TALLY SAYS EXACTLY WHY

```
promote-ctx ACCEPTED (last): bound=0 joined=0 already=0 parked=0 half_already=10
  half_unusable=0 orphans(awaiting_va=0,awaiting_phys=10)
  declined.promote_only=10 declined.initialize_only=0 entries=0 halves=10
  client=0xc1d0000a chan_client=0xc1d0000c object=0x5c000037 proc=ProcId(2)

promote-ctx TALLY (cumulative, all promotions):
  {bid=0x0 phys=1 va=8 complete=2} {bid=0x1 phys=1 va=8 complete=0}
  {bid=0x2 phys=1 va=8 complete=2} {bid=0x3 phys=0 va=10 complete=0}
  {bid=0x4 phys=0 va=10 complete=0} {bid=0x5 phys=0 va=10 complete=0}
  {bid=0x6 phys=0 va=10 complete=0} {bid=0x9 phys=0 va=8 complete=2}
  {bid=0xa phys=2 va=8 complete=0} {bid=0xb phys=0 va=8 complete=2}
```

Named from `ogkm-580: ctrl2080gpu.h:932-944`: `0x0` MAIN, `0x1` PM, `0x2` PATCH, `0x3`
BUFFER_BUNDLE_CB, `0x4` PAGEPOOL, `0x5` ATTRIBUTE_CB, `0x6` RTV_CB_GLOBAL, `0x9` FECS_EVENT,
`0xa` PRIV_ACCESS_MAP, `0xb` UNRESTRICTED_PRIV_ACCESS_MAP.

⇒ **Outcome R.** Four ids — MAIN, PM, PATCH, PRIV_ACCESS_MAP — carried **both** a physical half
and VA halves across the run, and `joined` is still **0**. Per the falsifier's own R row that
means *"the key is wrong"*, and the `orphans` field says which way:
**`awaiting_va=0, awaiting_phys=10`** — cup2's address space holds **ten VA halves and not one
physical half**. The physicals arrived; they arrived **somewhere else**.

★★★★ ⇒ **§16.48.3's cross-VAS limit is not a hypothetical. It is THE mechanism.** That paragraph
was written as a caveat — *"⚠ where two channels do not share a VA space this orphans the halves
… `Vas::promote_orphans` is the number that will say whether it happens"* — and the number said
**yes**, on the first boot, in the shape the caveat predicted. ⊘ Recording this as a design note
that turned out to be the finding, rather than re-writing it as though it had been the plan.

The companion `s41` boot (driver init only, no `cup2`) is the control: `proc=ProcId(0)`,
`bound=4 joined=0 parked=5 orphans(awaiting_va=1,awaiting_phys=4)`, tally
`{bid=0xa phys=1 va=0}` — a physical half parked under **proc 0** for the very id that cup2's
address space later waits on. Two address spaces, one buffer, neither holding both halves.

### 16.49.3 ★★★ AND FOUR IDS CAN NEVER PAIR, BY RM'S OWN CODE

`0x5` ATTRIBUTE_CB and `0x6` RTV_CB_GLOBAL (and `0x7` GFXP_POOL) fall through to a single arm in
`kgrctxPrepareInitializeCtxBuffer_IMPL`:

```c
1752:  case …_ATTRIBUTE_CB:      // fall-through
1754:  case …_RTV_CB_GLOBAL:     // fall-through
1756:  case …_GFXP_POOL:
1757:      // No initialization from kernel RM
1758:      return NV_OK;
```

(`ogkm-580: kernel_graphics_context.c:1752-1758`, `*pbAddEntry` left `NV_FALSE`.) ⇒ for those ids
a phase-1 entry is **never emitted by anyone**, which the tally shows as `phys=0` and which no
join can ever fix. `0x3`/`0x4` are gated by `pCtxBuffers->bInitialized[internalId]` (`:1777`,
`:1796`) — promoted once per GPU, not once per context.

⊘ **So "join the two phases" was never going to be sufficient on its own, and the instrument is
what makes that visible in one boot instead of three.** The physical side of a *global* context
buffer is not a per-VAS fact at all; it is a per-GPU one that RM declares once and then never
re-states.

### 16.49.4 ⇒ THE NEXT RUNG

1. **★★★★★ The physical half of a global context buffer is GPU-scoped, not VAS-scoped.** Park
   phase-1 physicals for the global ids (`0x3`–`0x9`, `0xa`, `0xb`) at the **GPU** level and let
   any VAS's phase 2 join against them; keep MAIN/PM/PATCH per-VAS, where they belong. ⚠ The
   scoping must be **derived from RM's own arms** (`kgrctxGetGlobalContextBufferInternalId` names
   exactly which ids are global) and not from the ids that happened to orphan in this boot.
2. `0x5`/`0x6`/`0x7` will still not pair — nothing emits their physical. Their backing has to come
   from the same place RM gets it (`kgraphicsGetGlobalCtxBuffers`), i.e. it is an
   **allocation-time** fact this port has not yet recovered, not a promotion-time one. ⊘ Do not
   let step 1's success hide that step 2 is untouched.
3. ⊘ `0xc574` stays where §16.48.1 put it: not the wall, and not this ladder's business.

★ And the prediction that held: `cup2` still fails at `cuCtxCreate`, exactly as §16.48.6 said it
would in P, Q **and** R alike. A rung scored on `cup2` would have read this boot as identical to
`s39` and `s40`. The scoreable quantity was `joined`, and it is `joined` plus `orphans` that
turned "the join does not fire" into "the halves are in two different address spaces, and here is
the RM arm that puts them there".

## §16.50 ★★★★★ THE GLOBAL PHYSICAL IS GPU-SCOPED — and §16.49.3's OWN count was wrong THREE ways

### 16.50.1 ⊘⊘ REFUTED (MINE, §16.49.3) — "`0x3`/`0x4` are gated by `bInitialized` (`:1777`, `:1796`)"

§16.49.3 excluded `0x3` BUFFER_BUNDLE_CB and `0x4` PAGEPOOL from the never-pair set with a
stated mechanism: *"`0x3`/`0x4` are gated by `pCtxBuffers->bInitialized[internalId]`
(`:1777`, `:1796`) — promoted once per GPU, not once per context."* ⊘ **Both citations name
arms that `0x3`/`0x4` never reach.** Read at `ogkm-580: kernel_graphics_context.c`:

| line | the arm that owns it | the ids that reach it |
|---|---|---|
| `:1748-1758` | `// No initialization from kernel RM` → `return NV_OK` | `0x3`, `0x4`, `0x5`, `0x6`, `0x7` |
| `:1777` | `if (pCtxBuffers->bInitialized[internalId]) return NV_OK;` | **`0x8` GFXP_CTRL_BLK only** (`:1759`) |
| `:1796` | idem | **`0x9`/`0xa`/`0xb` only** (`:1783-1785`) |

`0x3` and `0x4` are `case` labels at `:1748` and `:1750`, falling straight through to
`:1757`. They **never reach any `bInitialized` check**. The mechanism §16.49.3 gave them
belongs to four other ids.

⚠ And the *reason* this matters is not pedantry: §16.49.4 item 1 turned that misreading
into an implementation instruction — *"park phase-1 physicals for the global ids
(`0x3`–`0x9`, `0xa`, `0xb`) at the GPU level"* — a list that is **wrong in both
directions**. It includes six ids that publish no physical at all (a no-op dressed as a
fix, which would have read as "the scoping did nothing" on the next boot) and it includes
`0x8`, whose physical **may be a private per-context buffer** (`:1768-1771` reads
`localCtxBuffer` first when `bAllocated`). Publishing `0x8` GPU-wide would let one
context's VA bind to another context's private physical — a wrong table, which is worse
than the orphan it cures.

### 16.50.2 ⊘ THE COUNT, SETTLED — SIX, and the brief's "five" was right as far as it went

The brief asked whether §16.49.3's *"four ids can never pair"* (naming three) was a
miscount or a scoping choice. It is **both, in different places, and neither survives**:

- the heading's **four** matches nothing — not the source, not its own body's three. A
  miscount.
- the body's **three** is a *scoping choice*, and the choice rests on §16.50.1's
  misattributed citation. It is a decision made from a wrong reading, not a defensible
  narrowing.
- the brief's **five** for the fall-through group at `:1748-1758` is **confirmed exactly**
  — `BUFFER_BUNDLE_CB`, `PAGEPOOL`, `ATTRIBUTE_CB`, `RTV_CB_GLOBAL`, `GFXP_POOL`.
- ★ but the true never-pair count is **six**: `0xc` GLOBAL_PRIV_ACCESS_MAP reaches the same
  *"No initialization from kernel RM"* by its **own separate arm** at `:1803-1805`. A grep
  shaped like the fall-through group cannot match it — *enumerate, then filter*, and this
  is the minority row that shape misses.

⇒ Three readings of the same fifteen lines produced 3, 4, 5 and 6. **This is exactly why
membership must come from the enum and be pinned in code**, and it now is:
`the_phys_half_scope_is_derived_from_rms_arms_and_covers_the_whole_id_space` walks all
thirteen wire ids and asserts the count is six. Prose cannot drift back.

### 16.50.3 ★★★★★ THE CLASSIFICATION, FROM RM'S ARMS — and membership is NOT the scope

`kgrctxGetGlobalContextBufferInternalId_IMPL` (`:201-250`) is RM's **membership** oracle:
it refuses `0x0`/`0x1`/`0x2` with `NV_ERR_INVALID_ARGUMENT` (`:214-219`) and maps
`0x3`–`0xc` onto the ten-entry `GR_GLOBALCTX_BUFFER` enum
(`kernel_graphics_context_buffers.h:186-196`). So **global = `0x3..=0xc`, exactly.**

⊘ **But "is it global?" is not "where does its physical live?", and using the enum alone
as the scoping predicate — which is what the brief prescribed — would have been wrong.**
Seven of the ten global ids are not GPU-scopable: six publish nothing, and `0x8` may
publish a private buffer. The *scope* has to come from the arms of the only function that
can ever emit a phase-1 entry, `kgrctxPrepareInitializeCtxBuffer_IMPL` (`:1710-1807`):

| scope | ids | the arm, and what it reads |
|---|---|---|
| `PerContext` | `0x0` MAIN, `0x1` PM, `0x2` PATCH | `:1713-1747` — three per-context memory descriptors |
| `PerContext` ⚠ | `0x8` GFXP_CTRL_BLK | `:1759-1782` — `localCtxBuffer` **if `bAllocated`**, else the GPU pool. Ambiguous ⇒ narrowest scope |
| **`PerGpu`** | `0x9` FECS_EVENT, `0xa` PRIV_ACCESS_MAP, `0xb` UNRESTRICTED_PRIV_ACCESS_MAP | `:1783-1801` — `kgraphicsGetGlobalCtxBuffers(pGpu, …, gfid)`, **unconditionally** |
| `Never` | `0x3`–`0x7`, `0xc` | `:1748-1758`, `:1803-1805` — `// No initialization from kernel RM` |

★ `Never` is a **named third value**, not folded into `PerContext`. A `PerContext` id whose
physical never shows up is a bug in our routing; a `Never` id whose physical never shows up
is RM behaving exactly as written. A two-valued classifier reports both as "orphaned" and
sends the next rung hunting a phase 1 that cannot exist —
`falsifier_blocker_vs_only_blocker`, in the shape that has already cost this campaign.

⊘ An id past `0xc` hits RM's `default:` and is refused (`:1806`). We cannot refuse (the
entry may be a complete range), so it is classified `PerContext` — the **narrowest** scope.
Nothing gains reach by being unrecognised.

★ And the tally corroborates the classification without having produced it: `s41b` shows
`phys=0` for `0x3`, `0x4`, `0x5`, `0x6` — every observed `Never` id — and `phys>0` for
`0xa`, the one observed `PerGpu` id. `0x7`, `0x8` and `0xc` never appear at all, which is
why the classification could **not** have been read off the boot.

### 16.50.4 ★★★ THE `RM_ENGINE_TYPE_IS_GR` GATE, READ AT `ogkm-580: nv_gpu_ops.c:10885` — and ⊘ MY OWN ESCALATION OF IT REFUTED

⊘ **A source reading, said as one.** Everything here is `ogkm-580: nv_gpu_ops.c`, read this
rung. No boot exercised it.

The gate itself reads exactly as §16.48.3 stated:

```c
10883:  for (i = 0; i < retainedChannel->resourceCount; i++)
10885:      if (RM_ENGINE_TYPE_IS_GR(rmEngineType))
10886:          pParams->promoteEntry[i].bufferId = channelResourceBindParams[i].resourceId;
10888:      pParams->promoteEntry[i].gpuVirtAddr = channelResourceBindParams[i].resourceVa;
```

⇒ off the GR path every phase-2 entry carries `bufferId = 0` while `gpuVirtAddr` still
varies per entry.

★★★★ ⊘⊘ **AND THE HAZARD I DERIVED FROM THAT IS REFUTED — BY MY OWN NEXT READ.** The first
draft of this section escalated §16.48.3's *"the key stays unique by accident"* into a named
defect: `:10872` sets `entryCount = retainedChannel->resourceCount`, not `1`, so — I argued —
a non-GR channel with two or more bound resources would emit N entries all keyed
`buffer_id = 0` with different VAs, and our per-VAS join would answer the second with
`HalfConflict` and refuse the control. **It cannot happen.** `resourceCount` is set on
exactly three mutually exclusive paths, and `nvGpuOpsGetChannelInstanceInfo` asserts there
are no others (`:10578-10580`):

| engine type | where `resourceCount` comes from | value |
|---|---|---|
| `UVM_GPU_CHANNEL_ENGINE_TYPE_CE` | `:10582-10586` — *"CE channels have 0 resources, so they skip this step"*, `goto done` before any assignment | **0** ⇒ `:10855`'s `if (resourceCount != 0)` is false and **no promote control is emitted at all** |
| `UVM_GPU_CHANNEL_ENGINE_TYPE_SEC2` | `:10596-10650`, the falcon arm, commented *"single buffer"* | **exactly 1** |
| `UVM_GPU_CHANNEL_ENGINE_TYPE_GR` | `:10708`, `pParams->bufferCount` from `NV2080_CTRL_CMD_GR_GET_CTX_BUFFER_INFO` | **may exceed 1** — and this is the GR path, where `RM_ENGINE_TYPE_IS_GR` is **true** and `bufferId` **is** written |

⇒ the only arm that can produce a multi-entry promotion is the **GR** arm, which is the only
arm that writes `bufferId`. The two conditions coincide, so `entryCount > 1` and
`bufferId = 0` are **mutually exclusive at this site**. §16.48.3's reassurance holds; my
escalation of it does not.

★ **The consequence of the gate is therefore nil today**, and that is the honest statement:
`RM_ENGINE_TYPE_IS_GR` is a *tautology* at `:10885` — every promotion that reaches it with
more than one entry already satisfies it. The one real residue is that SEC2's single entry
carries `bufferId = 0`, colliding in key space with GR's MAIN; that is `entryCount = 1` so
it cannot self-conflict inside a control, but a SEC2 and a GR channel **sharing a VA space**
would collide across controls. ⊘ Recorded as **unmeasured**, not as a hazard: no SEC2
channel has been reached on this ladder, and nothing establishes that they share a VAS.

⚠ **Why this correction is in the log rather than silently edited out.** The brief asked me
to *"verify [the gate] and state its consequence"*, and the first consequence I derived was
wrong in the specific way this campaign keeps paying for: I read one line (`:10872`), found
it disagreed with a reassurance, and wrote the disagreement up as a finding **without
reading what feeds it**. Two screens up the same file, `:10578-10586` closes it. This is
`measure_before_reasoning_is_the_order` at source-reading scale — and it is the fourth
consecutive rung on which the *central claim of a briefing* was refuted by checking. The
difference is that this one was **mine**, and it was caught before the boot rather than by it.

### 16.50.5 WHAT LANDED

- `PhysHalfScope::{PerContext, PerGpu, Never}` + `phys_half_scope(u16)` — the classifier,
  every arm carrying its `ogkm` line. `is_global_ctx_buffer(u16)` exposed **separately**,
  because membership and scope are different questions and stating them apart is what
  stops them being conflated again.
- `Spine::global_ctx_phys: BTreeMap<GpuId, GlobalCtxPhys>` — the GPU-scoped publications,
  at rank 0. ⊘ **Not `refresh`-derived**: it records what RM *declared*, and rebuilding it
  from the resource graph would erase a publication whose emitting client has since been
  freed — the normal case, since the driver-init client does not outlive boot.
- ⊘ **A publication is never consumed by a join.** One global buffer is mapped by every
  context that needs it; removing it on first join would re-orphan every later address
  space — this rung's own failure, reintroduced one layer down.
- **PASS 1a, the drain**: a VAS's already-parked `AwaitingPhysical` halves for `PerGpu` ids
  are completed against the map at the start of every promotion. ★ Without it the fix would
  only help halves arriving *after* it, and `s41b`'s ten orphans were parked before —
  the boot would have reported `joined_global=0` for a reason unrelated to the scoping.
- The scope test is repeated **at the point of use**, not trusted from the point of
  insertion, so a future caller building the map cannot silently gain cross-context joins.
- A differing re-publication refuses by name (`HalfConflict`); an identical one is free.
- Lock order preserved as **0 → 1 → 0, never nested**: the shell reads the snapshot at
  rank 0, joins under the owning proc's lock alone, merges back after releasing it. ⚠ The
  visibility window (a publication is joinable by the *next* promotion, not by one racing
  it) is named in the code, and its worst case is one extra orphaned round — never a wrong
  binding. Merge happens **only on success**, so a refusal cannot half-apply.
- 8 new tests, including three negative controls: a `PerContext` half still refuses to
  cross address spaces, a `Never` id stays orphaned, and a publication survives its first
  joiner. ⚠ The existing cross-VAS test's doc claimed *"a half never leaks across address
  spaces"* — a universal this rung deliberately breaks — so it was **renamed and rewritten
  as the negative control it now is**, rather than left passing on an incidental fixture.

### 16.50.6 ★★★★★ THE COUNTERS — built for the case where the fix DOES NOTHING

★ An instrument hung off a refusal path has its deletion scheduled by the fix it guides;
that has cost two consecutive rungs. All three new counters ride the **success** path and
are emitted unconditionally in the `ACCEPTED` row.

- `joined_global` — of `joined`, how many drew on the GPU-wide map. ⊘ A **strict subset**,
  counted at bind time and not where the completion was staged: a completion that turns out
  to be an identical re-promote is skipped, and counting it at staging would have reported
  a bind that never happened.
- `globals_known` — **the one that matters if nothing else moves.** It does not ride any
  join, so `joined_global=0` stays legible:

| `globals_known` | `joined_global` | reading |
|---|---|---|
| `0` | `0` | ⊘ no `PerGpu` physical was **ever** published. The scoping is irrelevant; phase 1 is not arriving for those ids at all, and the next question is `kgraphicsGetGlobalCtxBuffers` at allocation time — **not** the join. |
| `>0` | `0` | the map filled and nothing drew on it: the VA halves are for `Never` ids, or the drain is not reaching them. |
| `>0` | `>0` | ★ the cross-address-space bridge fired. |

- `globals_added` — new publications *this* control. Separates "the map is filling" from
  "the map was already full", which `globals_known` alone cannot, and a steady
  `globals_known>0` with `globals_added=0` on every row is a fact about **when** phase 1 runs.

### 16.50.7 ★ THE FALSIFIER FOR `s42`, THREE-VALUED, COMMITTED BEFORE THE BOOT

⚠ Every row must print a **distinguishable** line. Checked against the format string in
`policy.rs`: all three counters are in the `ACCEPTED` row, so each row below is scoreable
from that one line plus the `TALLY`.

| | outcome | the line it prints | reading |
|---|---|---|---|
| **P** | `globals_known>0` **and** `joined_global>0`, and `orphans(awaiting_phys)` falls below 10 | `joined=K joined_global=J globals_known=N` with `J>0` | ★★★★★ the physical and virtual halves bind **across address spaces** for the first time. §16.50.3's classification is confirmed end-to-end. |
| **Q** | `globals_known>0`, `joined_global=0` | `joined_global=0 globals_known=N` with `N>0` | ★ partial and INFORMATIVE: publication works, the draw does not. Either the ten waiting VA halves are all `Never` ids (check the `TALLY` — `0x3`–`0x6` were four of them), or the drain is not reaching them. ⊘ Not a refutation of the scoping; it names which of the two. |
| **R** | `globals_known=0` | `globals_known=0 globals_added=0` | ⊘⊘ **no `PerGpu` physical is ever published.** The whole rung is a no-op and the wall is at *allocation* time (`kgraphicsGetGlobalCtxBuffers`), exactly where §16.49.4 item 2 said the untouched work is. ★ This is a REAL possible outcome: `s41b`'s tally shows `0xa phys=2`, but nothing proves those two arrived through a path this port routes to `apply_promote_ctx`. |
| **R′** | `PromoteFault::HalfConflict` appears in the refusal census | `promote-ctx PromoteFault::HalfConflict` | ★ INFORMATIVE **and** a guest-facing regression, reported as both: two address spaces declared **different** physicals for one global `buffer_id`, which would mean the buffer is not GPU-scoped after all and §16.50.3's `PerGpu` row is wrong. ⊘ Refusing was chosen over first-wins precisely so this cannot express itself as a wrong table. |
| **R″** | any other `s41b` guest-facing number moves (`0x2080012b` accepted x11 / refused x2, `NotOnAllowlist` x10, `FreeUnknown` x15, doorbells **170**), or `Malformed` appears | a changed refusal census with no `HalfConflict` | ⊘⊘ **the serious one**: a change advertised as a scoping fix changed the guest's stream through a path nothing predicted. |
| **S** | boot does not reach `cup2`, or the two artefacts' revision stamps disagree | — | not a result. ⚠ `CARGO_TARGET_DIR` makes the QOM shim link a **stale** archive at `rc=0` with only the stamp disagreeing. Verify both stamps before the boot. |

★ **Predicted, and it can lose:** `CUP2_RC=1` still. This fills the GR context-buffer gap
under MISS = FAULT and nothing more — `promote.rs`'s own module doc calls that *"necessary,
narrow, and nowhere near sufficient"*, and the compute working set arrives through the CE
page-table writes. ⇒ `CUP2_RC=1` is expected in **P, Q and R alike**, and a rung scored on
`cup2` reads all three as one failure. ★★ It is also now known that `cup2` launches **no
kernel at all** — it is `cuCtxCreate` + a 4-byte CE round-trip — so even a green `cup2`
would be a control-plane result and not evidence about compute. The scoreable quantities
are `joined_global` and `globals_known`.

## §16.51 ★★★★★ BOOTED `s42_21f967b_gpuscope` — **OUTCOME P**, and the counter that proved it read ZERO

`[measured 2026-08-09, rev 21f967b, boot s42_21f967b_gpuscope]`. Binary stamped
`kayfabe-rev:21f967b0de2b…` on **both** `qemu-build/qemu-system-x86_64` and the linked
`libkayfabe_qemu_raw.a`, verified against `git rev-parse HEAD` before the boot. Evidence:
`traces/guest_boots/run_s42_21f967b_gpuscope_{qemu,dmesg,probe}.log`, tracked and passing
`scripts/bench/assert_boot_evidence.sh`. ⚠ A third, older archive exists under
`/workspace/bench/cargo-target/` (the `CARGO_TARGET_DIR` trap); the linked one was the
fresh one and the stamps agree.

### 16.51.1 THE CHANGE IS OBSERVATIONALLY NEUTRAL — `R″` REFUTED

Every `s41b` guest-facing number is **byte-identical**: `0x2080012b` accepted **x11** /
refused **x2**, `NotOnAllowlist` **x10**, `FreeUnknown` **x15**, `UnmappedAllocClass` x3,
`ReservedClient` x2, doorbells **170**, `CUP2_RC=1`, `cuCtxCreate → 801`. No `HalfConflict`,
no `Malformed`, no new `PromoteFault`. ⇒ the scoping did not perturb the guest's stream.

### 16.51.2 ★★★★★ THE JOIN FIRED — and the row that names it says `joined_global=0`

```
promote-ctx ACCEPTED (last): bound=0 joined=0 joined_global=0 globals_known=1
  globals_added=0 already=1 parked=0 half_already=9 half_unusable=0
  orphans(awaiting_va=0,awaiting_phys=9)
  declined.promote_only=10 declined.initialize_only=0 entries=0 halves=10
  client=0xc1d0000a chan_client=0xc1d0000c object=0x5c000037 proc=ProcId(2)
```

Against `s41b`, three numbers moved and **only** three:

| | `s41b` | `s42` | |
|---|---|---|---|
| `orphans(awaiting_phys)` | **10** | **9** | one VA half is no longer parked |
| `half_already` | **10** | **9** | so only nine remain to be re-declared |
| `already` | **0** | **1** | …because the range it produced **is in the table** |

⇒ **one of cup2's ten orphaned VA halves bound, across address spaces.** `already` counts
ranges already bound byte-identically by a previous promotion; it was `0` for every
promotion of `s41b` and is `1` here. A range exists now that did not exist then, and the
only mechanism that could have created it is the GPU-scoped join.

★★★ **And it is the one id the classification said could pair, and no other.**
`globals_known=1`: exactly **one** `PhysHalfScope::PerGpu` physical was ever published, and
the tally names it — `{bid=0xa phys=2}`, PRIV_ACCESS_MAP. Of the ten VA halves cup2
declares, the other nine cannot pair, each for a reason §16.50.3 gives **from source**:

| ids | why they stay orphaned |
|---|---|
| `0x3`, `0x4`, `0x5`, `0x6` | `PhysHalfScope::Never` — tally `phys=0`, nothing emits their physical. §16.49.4 step 2, untouched exactly as predicted. |
| `0x9`, `0xb` | `PerGpu` but tally `phys=0` — RM did not emit their physical this boot (`bInitialized` already set, or a NULL memdesc) |
| `0x0`, `0x1`, `0x2` | `PerContext` — their physicals (tally `phys=1` each) belong to the proc that declared them, and MUST NOT cross. The negative control holding in production. |

⇒ **1 of 1 possible joins fired.** The nine that did not are nine the source says cannot,
and the instrument distinguishes the two reasons without any further boot.

### 16.51.3 ★★★★★ ⊘ A NEW INSTRUMENT FAILURE CLASS — an unconditional SUCCESS-path counter, destroyed by a LAST-WINS LATCH

`joined_global` was built to exactly the standard the last two rungs paid for: on the
**success** path, emitted **unconditionally**, counted at bind time so it is a strict subset
of `joined`. It reads **`0`** on the boot where it fired.

★ The reason is the **latch**, not the counter. The `ACCEPTED` row is `latch_last` — it
keeps the *last* accepted promotion — and the cross-address-space join is a **one-shot event
that happens early**: once `0xa` binds, every later promotion re-declares it identically and
it scores as `already`, forever. By the last promotion there is nothing left to join.

⊘ **So §16.50.7's outcome-P row was UNSCOREABLE FROM THE LINE IT NAMED.** It said
*"`globals_known>0` **and** `joined_global>0`"*, and the true outcome printed
`globals_known=1 joined_global=0`. Had I scored the falsifier mechanically I would have read
a **P as a Q** — "publication works, the draw does not" — and spent the next rung debugging
a drain that was working. It was `orphans` and `already`, neither of them the counter this
rung was built around, that carried the result.

★ The class, stated so it is not re-derived: **"on the success path" and "unconditional" are
not sufficient — the counter must also survive the AGGREGATION it is read through. A
last-wins latch measures only the last occurrence, so a one-shot event is invisible at the
end of the run.** This is `a_prediction_with_no_readout_was_never_a_test` one level in: I
checked that each falsifier branch named a *line*, and did not check that the line's
*latching discipline* could still hold the value when the branch came true.

⇒ **Fixed in the same rung, not deferred**: `SharedPromoteTally` now accumulates the join
outcome across every accepted promotion and renders
`|| CUMULATIVE bound=N joined=N joined_global=N already=N globals_added=N` beside the per-id
row. ⊘ It is emitted even when nothing was ever declared, because the absence of the row and
a row of zeros must not look the same.

### 16.51.4 ⇒ THE NEXT RUNG

1. ★ **`0x9` and `0xb` are `PerGpu` with `phys=0`** — they *can* publish and did not. That is
   a different question from `0x3`–`0x6`'s `Never`, and the instrument now separates them.
   Why does RM not emit their phase 1? `bInitialized` already set on a prior context
   (`:1796`) is the first hypothesis and it is checkable from the trace.
2. `0x3`–`0x6` remain §16.49.4 step 2: their backing is an **allocation-time** fact
   (`kgraphicsGetGlobalCtxBuffers`) this port has not recovered. ⊘ Step 1's success does not
   touch it, and `globals_known` is what keeps that visible.
3. ⊘ `0xc574` stays where §16.48.1 put it: not the wall.

## §16.52 ★ THE FALSIFIER FOR `s43`, COMMITTED BEFORE THE BOOT — a pure INSTRUMENT boot

⊘ **This rung changes no behaviour.** §16.51.3's cumulative accumulator is report-side
only; the join, the scoping and the classifier are byte-for-byte what `s42` ran. So the
question is exactly one: **does the instrument now show what `s42` had to be inferred from?**

★ It is worth a boot precisely because of what §16.51.3 says. Shipping a fix *for an
instrument that could not be read* without reading it would repeat the same pattern one
level up — `a_flag_is_not_progress`, applied to my own correction.

| | outcome | the line it prints | reading |
|---|---|---|---|
| **P** | the `TALLY` row carries `\|\| CUMULATIVE …` with **`joined_global` ≥ 1**, and every `s42` number is unchanged (`orphans(0,9)`, `already=1`, `half_already=9`, `globals_known=1`, doorbells **170**, `CUP2_RC=1`) | `\|\| CUMULATIVE bound=B joined=J joined_global=1 already=A globals_added=1` | ★★★★★ §16.51.2's inference is confirmed **directly** instead of through `orphans` and `already`, and outcome P is scoreable from one line for the first time. |
| **Q** | the `CUMULATIVE` row appears and `joined_global` is **0** | `joined_global=0` beside `already=1` | ⊘⊘ **the serious one, and it indicts ME**: §16.51.2 inferred the join from a `10 → 9` orphan drop and an `already` of 1. If no promotion ever recorded a global join, something *else* put that range in the table and the §16.50 account is wrong. ★ Note this row exists only because the accumulator can contradict the inference it was built to confirm — an instrument that could only agree would be worthless. |
| **R** | no `\|\| CUMULATIVE` substring in the log at all | — | the render never ran, or the `PROMOTE_DIAG_SLOTS` sentence clipped it. Not a result about the join; a result about the transport. ⚠ The row is emitted even for an empty tally precisely so this is distinguishable from "nothing to report". |
| **R′** | any `s42` guest-facing number moves | a changed refusal census | ⊘⊘ a **report-side** change altered the guest's stream — which would mean the accumulator is not report-side. |
| **S** | boot does not reach `cup2`, or the stamps disagree | — | not a result. |

★ **Predicted:** `CUP2_RC=1`, unchanged, and `cuCtxCreate → 801`. ⊘ Nine of cup2's ten
context-buffer VA halves still cannot bind and §16.51.4's items 1 and 2 are both untouched,
so nothing here can move the wall. ★★ And `cup2` gates on `cuCtxCreate` alone — it launches
**no kernel at all** (`cuMemAlloc` + a 4-byte CE round-trip), so even a green `cup2` would be
a control-plane result, never evidence about compute. The scoreable quantity is
`joined_global` in the cumulative row, and nothing else.

## §16.53 ★★★★★ BOOTED `s43_b17381c_cumjoin` — **OUTCOME P**, scored from ONE LINE

`[measured 2026-08-09, rev b17381c, boot s43_b17381c_cumjoin]`. Stamped
`kayfabe-rev:b17381c70416…` on **both** artefacts and checked against `git rev-parse HEAD`
before the boot — ⚠ and the guard **fired for real this rung**: the first attempt shipped a
bundle predating the commit, the checkout aborted on untracked `s42` traces, and all three
stamps still read `21f967b`. The build reported `BUILD_RC=1`; the stamps are what would have
caught it had it reported `0`. Evidence:
`traces/guest_boots/run_s43_b17381c_cumjoin_{qemu,dmesg,probe}.log`, tracked, passing
`assert_boot_evidence.sh`.

### 16.53.1 ★★★★★ THE CUMULATIVE ROW — and it says what `s42` could only be inferred to say

```
promote-ctx TALLY (cumulative, all promotions): {bid=0x0 …} … 
  || CUMULATIVE bound=8 joined=4 joined_global=1 already=7 globals_added=1
```

⇒ **`joined_global=1`.** The cross-address-space join fired **exactly once**, `globals_added=1`
published **exactly one** `PerGpu` physical, and §16.51.2's inference — drawn from an orphan
count falling `10 → 9` and an `already` rising `0 → 1` — is now **confirmed directly**.
⊘ Row `Q` of §16.52 (the row that would have indicted §16.51.2) did not fire.

★★★★ **And the demonstration of §16.51.3 could not be cleaner.** The last-wins row is
**byte-identical between `s42` and `s43`**:

```
ACCEPTED (last): bound=0 joined=0 joined_global=0 globals_known=1 globals_added=0
  already=1 parked=0 half_already=9 half_unusable=0 orphans(awaiting_va=0,awaiting_phys=9)
```

Two boots, the same visible row, and one of them carries `joined=4 joined_global=1` in a
number the row cannot show. ⊘ **The last-wins latch was hiding four joins and one global
join behind a row of zeros** — not because the counters were wrong, on a refusal path, or
conditional, but purely because of the *aggregation they were read through*. `bound=8` and
`already=7` were likewise invisible: the deepest promotion binds nothing, so the row that
survives is the row with the least to report.

### 16.53.2 `R′` REFUTED — a report-side change stayed report-side

Byte-identical to `s42` and therefore to `s41b`: `0x2080012b` accepted **x11** / refused
**x2**, `NotOnAllowlist` **x10**, `FreeUnknown` **x15**, `UnmappedAllocClass` x3,
`ReservedClient` x2, doorbells **170 arrived, 170 served, 0 REFUSED**, `CUP2_RC=1`,
`cuCtxCreate → 801`, `cuDeviceTotalMem` → 11959 MiB. No `HalfConflict`, no `Malformed`.

★ **Predicted and held:** `cup2` still fails at `cuCtxCreate`. Nine of ten context-buffer VA
halves still cannot bind, so nothing this rung did could move that wall — and `cup2` gates on
`cuCtxCreate` alone, launching **no kernel**, so it was never the instrument for compute
anyway.

### 16.53.3 ⇒ WHAT THE NEW NUMBERS OPEN

`joined=4` against `joined_global=1` says **three two-phase joins completed inside a single
address space** and one across. Those three were invisible at `s41b` too — its row also read
`joined=0` — so the §16.48 join has been working for two rungs longer than its own report
could show. ⊘ Recording that rather than re-attributing it: it does not change what `s42`
proved, and it does change how much of the join was ever unmeasured.

Next, unchanged from §16.51.4: `0x9`/`0xb` are `PerGpu` with `phys=0` — they *can* publish
and did not; and `0x3`–`0x6` are `Never`, an allocation-time gap no join can close.

## §16.54 ★★★★★ NAME THE REFUSAL — three of the brief's premises REFUTED before a line was written, and the falsifier for `s44`

`[source-read 2026-08-10, rev 4706b9f]` ⊘ **This rung changes no port behaviour whatsoever.**
`b17381c..4706b9f` is `git diff --stat`-verified as **four files, all documentation and
traces, zero lines under `crates/` or `qemu/`** — so the bench binary stamped
`kayfabe-rev:b17381c…` *is* HEAD behaviourally, and this boot needs **no rebuild**. ★ That
retires the stamp trap for this rung by construction rather than by vigilance: there is
nothing to re-link, so there is no stale archive to link.

### 16.54.1 ⊘⊘ REFUTED — "the log format is not what I assumed". The DATUM IS NOT IN THE LOG.

The brief records that greps for refused alloc-class ids came back empty and reads that as a
format mismatch, prescribing *"enumerate, then filter"*. Enumerating settles it the other
way, and the correction matters because the prescription would never have terminated:

`crates/kayfabe-rmrpc/src/lib.rs:406-411` constructs the refusal **carrying the class**:

```rust
AllocClassNotPermitted { class: u32, denial: Denial },
```

and `lib.rs:898-905` maps it to the tag that gets counted:

```rust
BridgeRefusal::AllocClassNotPermitted { denial: Denial::NotOnAllowlist, .. }
    => FaultTag("BridgeRefusal::AllocClassNotPermitted::NotOnAllowlist"),
```

The `..` **drops `class`**, and `policy.rs:377` then counts by tag alone. ⇒ `NotOnAllowlist
x10` is ten refusals *whose class ids were computed, held in the value, and thrown away one
function call later*. ⊘ **No grep over that log can ever return a class id.** A search that
comes back empty against a log which structurally cannot contain the datum is not a failed
search — and "enumerate, then filter" applied to it yields a perfect enumeration of the wrong
population, indefinitely.

★ The general form, and it is the one worth keeping: **an absent datum and an unmatched
pattern produce the identical observation — an empty result — and only reading the EMITTER
tells them apart.** Enumerating the log is the right move for the second and useless for the
first; the discriminator is upstream of the log.

### 16.54.2 ★★★ ALL THREE censuses are AGGREGATIONS that destroy the question — §16.51.3, one level out

The brief's lesson 1 is that a counter must survive the aggregation it is read through. The
same audit applied to the three lists the previous rung was read from, from source:

| census | structure | what it destroys |
|---|---|---|
| unserviced | `seen: Vec<UnservicedCommand>` deduplicated by `contains`, element `{function, cmd}`, **no count, no timestamp, no seq** (`kayfabe-device/src/unserviced.rs:206-283`) | **multiplicity and order.** `s43`: **104 events → 42 rows.** 62 occurrences left no mark. Nothing says whether an id fired in `cuInit`, in `cuCtxCreate`, or in teardown |
| bridge refusals | `BTreeMap<FaultTag, usize>` (`policy.rs:86,361,377`) | **every payload.** `class`, `hClient`, `NodeKey` — all projected away by `fault_tag()` |
| controls | `Vec<ServedControl{cmd, rpc_result, count}>`, cap 64 (`census.rs:101-111,249-268`) | **order.** Keeps counts (better than the other two) but cannot place a row in time, and sees only controls that were *answered* |

⇒ **The device's end-of-run report cannot name what refuses `cuCtxCreate`, for any reader.**
The previous rung's failure to find it was not a reading failure; the report is the wrong
instrument, and no amount of care with it would have produced the name.

### 16.54.3 ⊘ REFUTED — `0xc36f` is **VOLTA**, not Ampere; and the guest's refused allocs are all KERNEL RM's

`crates/kayfabe-chips/tests/host_classes.rs:89-91` and `kayfabe-abi/src/capability.rs:1080`:
**`0xc36f` = `VOLTA_CHANNEL_GPFIFO_A`; `AMPERE_CHANNEL_GPFIFO_A` is `0xc56f`.** `0xc56f` **is**
on the alloc allowlist, as is `0xa06c` (`KEPLER_CHANNEL_GROUP_A`); `0xc36f` is **not**.

`s43`'s boot dmesg carries four `GspRmAlloc failed … status=0x00000056`, and — the point —
**dmesg carries the `hClass` that the device census discards:**

| hClient | hClass | resolved (`ogkm-580`) |
|---|---|---|
| `0xc1d00008` | `0x00000070` | `NV01_MEMORY_SYSTEM_DYNAMIC` |
| `0xc1d00008` | `0x0000c36f` | `VOLTA_CHANNEL_GPFIFO_A` — the RC watchdog's channel |
| `0xc1d00009` | `0x0000402c` | `NV40_I2C` |
| `0xc1d00001` | `0x0000208f` | `NV20_SUBDEVICE_DIAG` |

★★ **Every one is a `0xc1d0xxxx` KERNEL RM client, and every one is in the *boot* dmesg, not
in `cup2`'s delta.** `cup2`'s delta (`run_s43_…_probe.log:71-110`) contains **zero**
`GspRmAlloc` failures — only `GspRmFree` ones. ⇒ **No allocation was refused during
`cuCtxCreate`**, which retires the whole alloc-class family as the wall and confirms the
brief's `PromoteFault`-is-the-watchdog finding from the independent direction.

★ Note the cross-instrument shape: the datum the device census projects away (`hClass`) is
present in dmesg, because RM prints its own arguments. **Two instruments blind in different
places are worth more than one instrument trusted everywhere.**

### 16.54.4 ★★★ WHAT IS LEFT, and the candidate the enumeration names

`s43`'s 42 distinct unserviced ids resolved against `ogkm-580`'s `ctrl/` headers. Two matter:

- **`0xa06c0101` = `NVA06C_CTRL_CMD_GPFIFO_SCHEDULE`** — the TSG schedule. It is
  **allowlisted** (`capability.rs:777`, so it raises no *bridge* refusal) but it is **not in
  `OBJECT_CONTROLS`** (`kayfabe-rmrpc/src/policy.rs:1283-1296`, which claims exactly three ids:
  `0xa06f0103`, `0xa06f0104`, `0x2080012b`). ⇒ nothing in the chain answers it, it falls to the
  `UnservicedLedger`, and it is answered `NV_ERR_NOT_SUPPORTED`. **Admitted, then unserved.**
- **`0xa06f0112` = `NVA06F_CTRL_CMD_STOP_CHANNEL`** — **absent from the entire repo**. This one
  is *explained*, not a candidate: it is exactly what `nv_gpu_ops.c:10957` reports failing, and
  `:10957` is inside `nvGpuOpsStopChannel`, i.e. teardown. Confirms the brief's teardown
  ruling by naming its mechanism.

★★ And the silence is itself evidence. `cup2`'s dmesg delta shows **nothing at the moment of
the refusal** — the first line is already `_nvGpuOpsReleaseChannel`'s
`NV_ASSERT(status == NV_OK)` at `nv_gpu_ops.c:10328`, freeing `hChannelRetainer`. A control
issued by **libcuda from userspace** that fails prints to no ring buffer at all. ⇒ a *userspace*
control refusal is precisely the wall shape that is invisible to every instrument this rung
inherited. `0xa06c0101` is issued by libcuda.

⊘ **This is a candidate, not a finding.** It is coherent with six independent facts and that
is exactly the state in which this campaign has been wrong before. It gets measured — see
§16.55, `[measured 2026-08-10, boot s44_b17381c_rmtrace]`.

### 16.54.5 THE INSTRUMENT — and it was ALREADY BUILT (§16.40's class, a third time)

No new tracer. `scripts/rpctrace/cuda_ioctl_trace.c` (666 lines) already decodes
`NV_ESC_RM_CONTROL` / `_ALLOC` / `_FREE` with `cmd`, `hClass`, and **the `status` word the
caller reads back**; `scripts/bench/uvm_ioctl_trace.c` covers the UVM plane, whose verdict is
in `params.rmStatus` and which every `_IOC_TYPE=='F'` filter misses structurally. Both were
written to run on the **host** against real firmware. Pointing them at the **guest**, against
our emulated GPU, is the new measurement. `scripts/bench/cup2_hook_rmtrace.sh` is the wiring
(result: §16.55, `[measured 2026-08-10, boot s44_b17381c_rmtrace]`).

★ **Trace one increment to the character in the log** (the brief's lesson 1, discharged): one
`write(2)` per ioctl to an `O_APPEND` fd. No latch, no max, no per-key overwrite, no sampling,
**no dedup** — the increment *is* the line, and order is the file's order. The only bound is
`NVTRACE_MAX`, which caps the hex width of a payload *inside* a line and so cannot hide a
record; it is passed explicitly rather than defaulted.

⚠ Two traps found while wiring it, both of the kind that make a real result read as a null one:
- The UVM interposer prints `rmStatus = 0x%08x` **with spaces**; the RM one prints
  `status=0x%08x`. A grep shaped like the RM row matches **zero** UVM rows, and the zero reads
  as "no UVM failure". Both patterns are written against the `fprintf` they must match.
- `cuda_ioctl_trace.c` doubles as a **fault injector** (`NVFAULT_CTRL`/`_ALLOC`/`_STATUS`,
  `NVSWEEP_GPUINFO`). A trace with any of them set is not a capture. They are `env -u`'d, and
  the trace is asserted to contain no `INJECT` line.

### 16.54.6 THE FALSIFIER FOR `s44`, committed BEFORE the boot

The scoreable quantity is **the ordered list of RM/UVM records with a non-zero status**, and
the question is *which record, with which argument, sits between the last `cuDeviceTotalMem`
control and the teardown burst.*

| | outcome | the line it prints | reading |
|---|---|---|---|
| **P** | a `CTRL` record with `cmd=0xa06c0101` and a non-zero status, before the `FREE` burst | `CTRL cmd=0xa06c0101 … status=0x00000056` | ★★★★★ the wall is **named**: the TSG schedule, admitted by the allowlist and unserved by the chain. §16.54.4's candidate confirmed at the boundary |
| **Q** | the failing record is a `CTRL` with **some other** `cmd` | `CTRL cmd=0x???????? … status=0x……` | ★★★★ **also a win — the deliverable is a NAME, not a specific name.** §16.54.4's candidate refuted and replaced in the same measurement |
| **R** | the failing record is an `ALLOC` | `ALLOC hClass=0x???????? … status=0x……` | ⊘⊘ indicts §16.54.3: `cup2`'s dmesg delta shows no `GspRmAlloc` failure, so an alloc failing here would mean the delta is not the whole story. The trace names the class the census discards |
| **S** | **every** RM and UVM record has status 0, and `cuCtxCreate` still returns 801 | the non-zero lists are both EMPTY *and* the totals beside them are non-zero | ★★★★★ **the most informative branch**: the refusal is **not at the ioctl boundary at all**. It is libcuda rejecting a value we answered *successfully but wrongly* — a `_wall_that_can_carry_no_name` — and the next instrument is a value diff against the C oracle, not another refusal hunt |
| **T** | trace missing/empty, `INJECT` present, `PROMOTE_CTX_SEEN=0`, or any `s43` guest-facing number moves (`0x2080012b` x11/x2, `NotOnAllowlist x10`, `FreeUnknown x15`, doorbells **170**, `CUP2_RC=1`) | the VERIFY block | ⊘ not a result about the wall — the interposer perturbed the run or never loaded. `PROMOTE_CTX_SEEN` is the positive control: a control we independently KNOW this boot issues |

★ **Predicted:** `CUP2_RC=1`, `cuCtxCreate → 801`, and every `s43` census number unchanged —
the port is byte-identical, so anything that moves is the instrument, not the port.
⊘ **And a green `cup2` would still not be a compute result**: it launches no kernel
(`cuMemAlloc` + a 4-byte CE round-trip), so the first real compute rung remains `cup3`.

## §16.55 ★★★★★ BOOTED `s44_b17381c_rmtrace` — **OUTCOME P**. THE REFUSAL HAS A NAME: `NVA06C_CTRL_CMD_GPFIFO_SCHEDULE`

`[measured 2026-08-10, boot s44_b17381c_rmtrace]`. Binary stamped
`kayfabe-rev:b17381c70416a371cfdded98c2b35dff60c6cb27` on **both** `qemu-build/qemu-system-x86_64`
and the linked `qemu-10.2.4/hw/misc/nvkvm/libkayfabe_qemu_raw.a`. ★ The stamp is `b17381c` and
that is **correct, not stale**: `git diff --stat b17381c 4706b9f` is four files, all docs and
traces, **zero lines under `crates/` or `qemu/`**, so the binary IS HEAD behaviourally and this
rung required **no rebuild at all**. Box repo synced to `f4fa6e3` before the boot, hook sha256
verified identical on both sides. Evidence:
`traces/guest_boots/run_s44_b17381c_rmtrace_{qemu,dmesg,probe}.log`, tracked, passing
`assert_boot_evidence.sh`.

### 16.55.1 ★★★★★ THE ANSWER — four refusals reach libcuda, and the LAST one is followed by teardown

The complete ordered list of RM ioctls returning a non-zero status to libcuda, all 249 records
of `cup2` in scope, from `run_s44_b17381c_rmtrace_probe.log`:

```
 39: CTRL cmd=0x20810108 hClient=0xc1d0000c hObject=0x5c000004 size=992  status=0x00000056
 48: CTRL cmd=0x2080012f hClient=0xc1d0000c hObject=0x5c000003 size=1464 status=0x00000056
 90: CTRL cmd=0x2080200a hClient=0xc1d0000c hObject=0x5c000003 size=8    status=0x00000056  in=12000000ffffffff out=12000000ffffffff
196: CTRL cmd=0xa06c0101 hClient=0xc1d0000c hObject=0x5c000012 size=3    status=0x00000056  in=010000 out=010000
```

⇒ **`0xa06c0101` = `NVA06C_CTRL_CMD_GPFIFO_SCHEDULE`, on `hObject=0x5c000012`, record 196 of 249.**
**The very next record is a `FREE`, and every record after it is `FREE` or `ESC nr=0x4f` (unmap)
— pure teardown.** §16.54.4's candidate, confirmed at the boundary.

★ And `0x5c000012` is not any handle: the immediately preceding records show it is the **parent
of every channel `cuCtxCreate` just built**:

```
ALLOC hClass=0x0000c56f hParent=0x5c000012 hObject=0x5c000037   ← AMPERE_CHANNEL_GPFIFO_A
ALLOC hClass=0x0000c7c0 hParent=0x5c000037 hObject=0x5c000038   ← AMPERE_COMPUTE_B
ALLOC hClass=0x0000c7b5 hParent=0x5c000037 hObject=0x5c000039   ← the copy object
CTRL  cmd=0x906f0101 (GET_CLASS_ENGINEID)     status=0
CTRL  cmd=0xc36f0108 (GET_WORK_SUBMIT_TOKEN)  status=0  out=0e000000
CTRL  cmd=0x20801218                          status=0
CTRL  cmd=0xa06c0101 hObject=0x5c000012 size=3 status=0x00000056   ← THE WALL
FREE  …
```

`0x5c000012` is the **TSG** (`hClass=0x0000a06c`, `KEPLER_CHANNEL_GROUP_A`, allocated once).
libcuda builds the whole context — TSG, eight channels, eight compute objects, eight copy
objects, all `status=0` — and then asks RM to **schedule the group**. We answer
`NV_ERR_NOT_SUPPORTED`, and it unwinds.

★★ `in=010000` is `{bEnable=1, bSkipSubmit=0, bSkipEnable=0}` — **byte-for-byte the three-byte
payload the C oracle sends to the real host driver** at `nvkvm_gpu_emul.c:8044`. `out=010000`,
unchanged, because nothing serviced it.

### 16.55.2 ★★★★ THE MECHANISM, END TO END, MEASURED AT BOTH ENDS `[boot s44_b17381c_rmtrace, 2026-08-10]`

`[measured 2026-08-10, boot s44_b17381c_rmtrace]`, cross-read against `cap3` (`traces/mode2_c_reference/`) and
`ogkm-580.159.04`.

Neither end alone names this. Together they close it:

| plane | evidence | fact |
|---|---|---|
| guest userspace | `s44` trace record 196 | libcuda issues `0xa06c0101` and reads back `0x56` |
| our device | `s43`/`s44` qemu log, `unserviced fn 76 cmd 0xa06c0101` | the kernel forwarded it as `GSP_RM_CONTROL` and **nothing in our chain answered** |
| our source | `capability.rs:777` vs `policy.rs:1283-1296` | it is **allowlisted** (so no *bridge* refusal) but absent from `OBJECT_CONTROLS`, whose entire membership is `0xa06f0103`, `0xa06f0104`, `0x2080012b` |
| the C oracle | `cap3` fn=76 histogram: `0xa06c0101 ×3` | the guest issues it on the **successful** path too — it is not an artefact of our failure |
| the C oracle | `nvkvm_gpu_emul.c:3057` | the C answers unrecognised controls **`NV_OK` by default**, and separately schedules the *host* TSG itself (`:8044`, `:4052`, `:4191`, `:9131`, `:9577`) |

★★★ **`admitted` and `served` are two different gates, and passing the first is what made this
invisible.** An id on the alloc/control allowlist raises no bridge refusal, produces no
`FaultTag`, and appears in no refusal census — it falls silently to the `UnservicedLedger` and
is answered `NOT_SUPPORTED`. ⇒ **The allowlist is a statement about what we permit, never about
what we implement**, and a census built on refusals cannot see the gap between them.

### 16.55.3 ★★★★★ THREE FORGIVEN, ONE FATAL — `not_supported_is_the_forgiven_status`, MEASURED `[boot s44_b17381c_rmtrace, 2026-08-10]`

`[measured 2026-08-10, boot s44_b17381c_rmtrace]`, from the same 249-record stream.

The other three `0x56`s are the control: `cuInit`, `cuDeviceGetCount`, `cuDeviceGetName`,
`cuDeviceGetAttribute` ×2 and `cuDeviceTotalMem` **all succeed after them**.

| cmd | resolved | verdict |
|---|---|---|
| `0x20810108` | `NV2081_BINAPI` | forgiven — the §14.26 "phantom" |
| `0x2080012f` | `NV2080_CTRL_CMD_GPU_QUERY_ECC_STATUS` | forgiven — ★ and **`0x56` is the CORRECT answer**: the C returns it deliberately (`nvkvm_gpu_emul.c:3111`) because a GeForce GA106 has no ECC and real hardware returns `0x56` |
| `0x2080200a` | `NV2080_CTRL_CMD_PERF_BOOST` | forgiven |
| `0xa06c0101` | `NVA06C_CTRL_CMD_GPFIFO_SCHEDULE` | **FATAL** |

⇒ four identical statuses, three absorbed and one terminal, in one 249-record stream. ★ The
discriminator is not the status, not the caller and not the log level — it is **what the caller
does next**. Only an ORDERED trace can show that, which is precisely what all three device
censuses (set-union, tag-projection, order-free counting map) destroy.

### 16.55.4 ⊘ REFUTED — the "ten refusals / zero failures" asymmetry does not exist

The coordinator's reframe rested on `s43` showing **zero** `GspRmAlloc failed … hClass=` against
our **ten** `NotOnAllowlist`. `[measured 2026-08-10, boot s44_b17381c_rmtrace]` and re-read against
`run_s43_b17381c_cumjoin_dmesg.log`, it is refuted twice over:

- `s43` and `s44` **each carry four** `GspRmAlloc failed … hClass=` lines, **naming their
  classes**: `0x70` (`NV01_MEMORY_SYSTEM_DYNAMIC`), `0xc36f` (`VOLTA_CHANNEL_GPFIFO_A`, the RC
  watchdog's channel), `0x402c` (`NV40_I2C`), `0x208f` (`NV20_SUBDEVICE_DIAG`). They are in
  **`run_s4*_dmesg.log`** — a *different file* from the probe log that was enumerated.
- `0x402c` is present in **every** boot `s38`→`s44` (dmesg + qemu log), not "gone since `s38`".
  It looks absent only because the probe log holds `dmesg | tail -40`, a window that has since
  moved past it.

★★★★ **The enumeration was correct and it ran over the wrong population.** "I enumerated
`status=0x…` across the whole probe log" is true, methodical, and answers a question about a
different time window — `a_correct_capture_can_answer_the_wrong_question`, with `tail -40` as
the reduction that did it. ⊘ Same shape as CLAUDE.md's serial-log trap: the file exists, is
freshly timestamped, is named after the boot, and is **not where the datum lives**.

⇒ And `s44` settles the underlying question directly, which no amount of dmesg could: **all 52
of libcuda's allocations return `status=0`.** Every `NotOnAllowlist` refusal in this boot is
kernel-RM's, none is on `cuCtxCreate`'s path, and the alloc-class family is retired as the wall.
⊘ `0xc574` **is** `s38`-only, as claimed — that one holds.

### 16.55.5 ⚠ MY OWN POSITIVE CONTROL WAS WRONG, and the instrument survived it

`PROMOTE_CTX_SEEN=0`. I chose `0x2080012b` as the "a control we independently KNOW this boot
issues" check — and it is **structurally invisible** to this instrument: `kgrobjPromoteContext`
is called by kernel RM, so it never crosses the *userspace* ioctl boundary. The device census
counts it (x11/x2) precisely because the device sees the kernel's RPC.

★ The class: **a positive control must be drawn from the population the instrument can see, not
from the population the question is about.** A control chosen from another plane fails on a
working instrument and would have scored a good boot as outcome `T` `[measured 2026-08-10, boot s44_b17381c_rmtrace]`.
What actually proved the
instrument live was the payload — 249 RM records, 316 UVM, `hClass=0xc7c0 ×8`, `0xc56f ×8`,
`0xa06c ×1`: the entire context-build sequence, which nothing but a loaded interposer could
produce. ⇒ replace it with `ALLOC hClass=0x0000a06c` next rung.

### 16.55.6 `T` REFUTED — the interposer is observationally neutral

Every guest-facing number **byte-identical to `s43`**: `commands: 589 decoded, 104 UNSERVICED`,
`bridge refusals: 34 total, 6 distinct`, `controls: 143 answered, 46 distinct`,
`doorbells: 170 arrived, 170 served, 0 REFUSED`, `0x2080012b` **x11** accepted / **x2** refused,
`NotOnAllowlist x10`, `Refused x2`, `ReservedClient x2`, `UnmappedAllocClass x3`,
`UnknownContextObject x2`, `FreeUnknown x15`, `isolates: 2/2/2`. `CUP2_RC=1`,
`cuCtxCreate → 801`, `cuDeviceTotalMem → 11959 MiB`. **UVM plane: 28 `rmStatus` rows, ZERO
non-zero** — outcome `S` is excluded, and the empty list is distinguishable from a missing one
because the total is printed beside it.

### 16.55.7 ⇒ WHAT THIS OPENS, and what it does NOT

⊘ **Do not read this as "implement `0xa06c0101` and `cup2` goes green."** What is measured is
that it is the *first* thing `cuCtxCreate` cannot get past, and `nothing after record 196 is
anything but teardown` — so the port has **never executed** whatever libcuda does after a
successful schedule. There may be further walls; this rung names the first.

★ The C oracle also tells us what "servicing" it must mean, and it is not an ack:
`nvkvm_gpu_emul.c:8038` — *"The guest's GPFIFO_SCHEDULE is a control (not forwarded by
shadow_fwd), so the host TSG is idle until we schedule it."* The C **answers the guest `NV_OK`
and separately issues `0xa06c0101` to the real host TSG**, having first issued `0xa06c0102`
(`BIND`). ⇒ a bare `NV_OK` would move the wall without scheduling anything, and would be
`a_flag_is_not_progress` in its purest form.

⊘ **And `0xa06c0102` (`NVA06C_CTRL_CMD_BIND`) is NOT a second wall** — `[measured 2026-08-10, boot s44_b17381c_rmtrace]`,
the same capture. I
flagged it as worth checking because the C issues BIND before SCHEDULE (`:4048`, `:9574`); the
data already answered it. `s44`'s userspace census contains **exactly one `a06c` control**,
`0xa06c0101`, and `grep -c a06c0102` over both `s43`'s and `s44`'s device logs returns **0** —
so the guest never issues BIND from userspace *or* from the kernel. ⇒ the C's BIND is the C
**originating** a host-side setup call, not replaying one of the guest's, and there is no
guest BIND for us to serve. ★ Recorded as closed rather than carried forward: a hypothesis I
could answer from a capture already in the tree had no business becoming next rung's question.

---

## §16.56 ★★★★★ SERVE THE TSG SCHEDULE — and the refutation that came with it: the wall was never invisible

`0xa06c0101` (`NVA06C_CTRL_CMD_GPFIFO_SCHEDULE`) is now claimed by `ObjectPolicy` and
**performed** rather than acked. This section carries the code, the argument, three
refutations, and the falsifier for the boot — committed before it.

### 16.56.1 ⊘⊘⊘ REFUTED — "why nothing saw it". THE LEDGER SAW IT, SIX TIMES, ON DISK

The brief for this rung said:

> **Why nothing saw it:** `0xa06c0101` is allowlisted but absent from `OBJECT_CONTROLS`.
> Clearing the first gate means **no `FaultTag` is ever built**, so no refusal-census row,
> no counter, **silent** fall to the unserviced ledger.

Every clause of the mechanism holds. The conclusion does not.
`[measured 2026-08-10, over traces/guest_boots/*_qemu.log]`:

```
run_s44_b17381c_rmtrace_qemu.log:149
  nvkvm:   unserviced fn 76 cmd 0xa06c0101
```

**Six committed boot logs carry that line** — `s39_fd92017_kernelarm`,
`s40_4733730_acceptcensus`, `s41b_62e757f_twophase`, `s42_21f967b_gpuscope`,
`s43_b17381c_cumjoin`, `s44_b17381c_rmtrace` — by command id, in full, from the first boot
that reached the point. §16.55.2's own evidence table cites that very line. ⇒ The datum was
recorded correctly by a working instrument and committed to this repository for six rungs.

★★★★ **The defect is RANK, not visibility.** `s44`'s ledger prints **42 distinct**
unserviced ids in one undifferentiated block. One ended `cuCtxCreate`; forty-one were
survivable. The ledger records membership and deliberately nothing else — so nothing in it
said which, and nobody was ever *required* to say. ⇒ A gate that made the id **more
visible** would have closed nothing. What was missing is an **argument attached to each
entry**, and that is what `tests/tests/admitted_is_served.rs` now demands.

⚠ Note the shape of my own near-miss: the brief's mechanism is correct and its conclusion is
not, and the difference is one `grep` over files the brief itself pointed at. *"No counter
was incremented"* and *"no record exists"* are different facts, and the first does not imply
the second when a terminal-shaped recorder sits at the end of the chain — which is exactly
what `unserviced::UnservicedLedger` is, by design, documented in `served_chain`.

### 16.56.2 ⊘⊘ REFUTED — a GREEN, EXECUTING TEST held the wall in place

`tests/tests/gpfifo_schedule.rs::the_control_claim_is_exactly_these_ids` asserted, and had
asserted since #177:

```rust
assert!(!OBJECT_CONTROLS.contains(&NVA06C_CTRL_CMD_GPFIFO_SCHEDULE),
    "★ the TSG form is what we send the HOST, never what the guest asks us —
     ogkm-580: mem_utils.c:1973-1989 issues the a06f form on a TSG-less channel");
```

It ran on every CI run. It was green. It was **wrong**, and it named its source.

★★★★ **A correct citation, a false universal.** `mem_utils.c:1973-1989` really does issue
the `a06f` form on a TSG-less channel. Its scope is `RmInitAdapter`'s **scrubber** — one
channel, allocated by kernel RM. The assertion generalised it to *"never what the guest asks
us"*, a quantifier that ranges over **libcuda**, about which the cited line says nothing.
`docs/design/gpfifo_schedule.md` §1's table carries the same narrowing (`0xa06c0101` → *"on
this path? no"*), and the table is right: on **that** path, it is no.

⇒ A citation establishes what it says about the path it is on. The quantifier is the
reader's, and it is the reader's to get wrong. (`a_correct_citation_narrowed_by_the_reading`
— and this is the second instance in this campaign.)

### 16.56.3 ⊘ REFUTED — `admitted ⊆ served` is not the invariant: 142 of 163

`[measured 2026-08-10, rev 1f38160, tests/tests/admitted_is_served.rs]`

The brief prescribed a compile- or test-time assertion that *every command the allowlist
admits must have an implementation*. `[measured 2026-08-10, rev 1f38160]`:

| | |
|---|---|
| controls the bench boundary admits **by name** (`CapabilityTable::all_controls`) | **163** |
| of those, ids the production chain has an arm for | **21** |
| **admitted and served by nothing** | **142** |

★ That is not 142 bugs. The two sets are about **different planes**: `capability::CONTROLS_*`
is ported from gVisor `nvproxy` and gates the guest's **userspace ioctl** surface; the served
chain answers the **GSP RPC** surface. Most of the 142 never reach our GSP at all — the
guest's own kernel RM answers them locally out of state it already has. Building answers for
them would be building answers for traffic that does not exist, and a gate demanding it would
be 142 rows of noise that every future reader learns to skip.

⇒ The universe with force is not the allowlist. It is **what a boot recorded the guest
actually sending us**, which is what the gate quantifies over. `[measured 2026-08-10]` that
universe is **43** ids across every committed boot log; `0xa06c0101` has left it (served),
`0x00801813` left it at §16.30, and the remaining **41** are now listed with the two
directions machine-checked.

⊘ And the gate is **scoped in its own docs**: `ControlPermit` has two *rule-based*
admissions (`GssLegacyRule`, bit 15; `BinApiRule`, class `0x2081`) that admit an unbounded id
space no table enumerates. They cannot be swept. A reader who takes the gate for the whole
admission surface has inherited the same false universal §16.56.2 is about.

### 16.56.4 THE CODE — and why the `NV_OK` is not forged

The C's standing warning is `nvkvm_gpu_emul.c:8038` — *"The guest's GPFIFO_SCHEDULE is a
control (not forwarded by shadow_fwd), so the host TSG is idle until we schedule it."* An ack
alone would move the wall and schedule nothing.

| layer | what landed |
|---|---|
| `kayfabe-core/src/gpu.rs` | `route_schedule_group` / `apply_schedule_group` / `Gpu::schedule_group`, `ScheduleGroupRoute` / `Ack` / `Fault` |
| `kayfabe-rmrpc/src/policy.rs` | `OBJECT_CONTROLS` += `0xa06c0101`; `respond_gpfifo_schedule_group`; `ObjectModel::schedule_group` |
| `kayfabe-rt/src/device.rs` | `SharedDevice::schedule_group` — ROUTE at rank 0, ACT under one proc lock |
| `kayfabe-qemu-raw/src/shim.rs` | the shell's seat |
| `tests/tests/gpfifo_schedule.rs` | the doorbell transition, per member; the refusal vocabulary; the whole RPC |
| `tests/tests/admitted_is_served.rs` | the ledger ratchet (§16.56.3) |

★★ **The fan-out is the driver's own semantics, not our invention.**
`kchangrpapiCtrlCmdGpFifoSchedule_IMPL` walks `pKernelChannelGroup->pChanList` **twice**
before it RPCs — once asserting every member is schedulable (`NV_ERR_INVALID_STATE` if not),
once forcing every member onto one runlist (`ogkm-580:
src/nvidia/src/kernel/gpu/fifo/kernel_channel_group_api.c:1102-1170`). The params are a
**typedef**, not a look-alike: `typedef NVA06F_CTRL_GPFIFO_SCHEDULE_PARAMS
NVA06C_CTRL_GPFIFO_SCHEDULE_PARAMS` (`ogkm-580: ctrl/ctrla06c.h:101`), and the guest's vGPU
dispatcher sends both ids down one arm (`ogkm-580: src/nvidia/src/kernel/vgpu/rpc.c:4557-4559`).

★★★ **What makes the ack falsifiable** is identical to #177's and inherits its argument
(`docs/design/gpfifo_schedule.md` §2): the members land in `ExecPlane::requested`, which
`kayfabe_fwd::plan_doorbell` **gates** on (`FwdFault::NotScheduled`), and the host-side
`0xa06c0101` is issued by `kayfabe_isolate::RmBackend::schedule` against the **host** group at
the member's first doorbell. `one_tsg_control_lets_every_member_channel_past_the_doorbell_gate`
runs the transition on **every** member, before and after — a port that recorded the intent
against the group handle, or against one member, acks the guest and then refuses its very next
doorbell, which is the #12 shape.

⊘ **Refusals are by name and never `0x56`.** `ScheduleGroupFault` has five variants
(`UnknownGroup`, `NotAGroup`, `GroupHasNoChannels`, `NoMemberMaterialized`,
`GroupSpansProcs`) answered `NV_ERR_INVALID_STATE` (`0x40`) — which is in the command's own
vocabulary (`kernel_channel_group_api.c:1106-1109`), and is *not* `NV_ERR_NOT_SUPPORTED`,
because `0x56` is the signature this port wore on this exact id for six boots.

⊘ **What is still false** is unchanged and is not re-argued here: `gpfifo_schedule.md` §3.
`GroupHasNoChannels` and `NoMemberMaterialized` are split deliberately — one means the guest
scheduled an empty group, the other means **our** projection lost channels the guest built,
and only the second is our defect.

### 16.56.5 ★★★ THE FALSIFIER FOR `s45`, committed BEFORE the boot

Instrument: `scripts/bench/boot_capture.sh s45_<rev>_tsgsched` with
`POST_CAPTURE_HOOK=scripts/bench/cup2_hook_rmtrace.sh` — the same LD_PRELOAD RM/UVM
interposer that produced `s44`'s 249 ordered records, read **in guest order**.

⚠ **Positive control, chosen from the population the instrument can see.** §16.55.5's own
correction: `0x2080012b` was structurally invisible to this instrument (kernel-originated,
never crosses the userspace boundary) and would have scored a good boot as a failure. The
control here is **`ALLOC hClass=0x0000a06c` ×1** — measured present in `s44`, on the
userspace path, immediately before the record under test. ⊘ If it is **absent**, nothing
below is scored: the capture did not reach the context build.

| # | outcome | the distinguishable line | verdict |
|---|---|---|---|
| **A** | `CTRL cmd=0xa06c0101 … status=0x00000000`, **and** records appear after it that are not `FREE`/`ESC nr=0x4f` | `196: CTRL cmd=0xa06c0101 … status=0x00000000` followed by a non-teardown record | ★★★★★ the wall moved. **Report the id and status of the next non-zero record** — that is the next wall, and §16.55.7 says to expect one |
| **B** | `CTRL cmd=0xa06c0101 … status=0x00000040` | the same record with `0x40`, and `nvkvm: control 0xa06c0101 result 0x00000040` in the qemu log | ★★★★ **a win of a different kind**: the control is CLAIMED and the group route DECIDED against it. The device log names which `ScheduleGroupFault`. Almost certainly `NoMemberMaterialized` — the projection did not place libcuda's eight channels — which is a defect of ours the port could not previously express |
| **C** | `CTRL cmd=0xa06c0101 … status=0x00000056` still | unchanged from `s44` | ⊘⊘ **instrument/plumbing failure, not a port result.** `0x56` is *"nobody claimed it"*, and this rung claimed it. Check the binary's `kayfabe-rev` on BOTH `qemu-build/qemu-system-x86_64` and the linked `libkayfabe_qemu_raw.a` before reading anything else — the bench served a stale binary for weeks once |
| **D** | no `0xa06c0101` record at all, control present | `ALLOC hClass=0x0000a06c` present, no `a06c0101` | the guest changed behaviour between boots. Report; do not score A/B/C |

⊘ The reply-status alone does not decide **A vs B** by itself in one direction: `A` requires
*both* `0x00000000` **and** a following non-teardown record, because an `NV_OK` followed by
nothing but `FREE` would mean the ack was accepted and the next thing failed silently — which
is a fourth state, and it reads as `A` to anyone who only checks the status.

Also captured every boot, and asserted non-empty: `run_s45_*_dmesg.log` containing
`RmInitAdapter`, `CUP2_RC`, the qemu log's `commands:` / `unserviced` / `controls:` census
lines, and `nvkvm: control 0xa06c0101 result …`.

---

## §16.57 ★★★★★ BOOTED `s45_748a207_tsgsched` — **OUTCOME A**. The wall moved, and it moved 207 records

`[measured 2026-08-10, boot s45_748a207_tsgsched]`. Binary stamped
`kayfabe-rev:748a207…` — ⚠ **verified on BOTH** `qemu-build/qemu-system-x86_64` and
`target/release/libkayfabe_qemu_raw.a`, and the first attempt at this rebuild **failed that
check**: see §16.57.4. Evidence:
`traces/guest_boots/run_s45_748a207_tsgsched_{qemu,dmesg,probe}.log`, tracked, passing
`assert_boot_evidence.sh`.

### 16.57.1 ★★★★★ RECORD 196 IS NOW `status=0x00000000`

The exact record `cup2` died on in `s44`, byte for byte, one rung later:

```
s44:   196  CTRL cmd=0xa06c0101 hClient=0xc1d0000c hObject=0x5c000012 size=3 status=0x00000056  in=010000
s45:   196  CTRL cmd=0xa06c0101 hClient=0xc1d0000c hObject=0x5c000012 size=3 status=0x00000000  in=010000 out=010000
```

Same client, same TSG handle, same three bytes, same record index. And it is not the only
one: libcuda goes on to build **two more** channel groups and schedule both —

```
   233  CTRL cmd=0xa06c0101 … hObject=0x5c00003b … status=0x00000000
   270  CTRL cmd=0xa06c0101 … hObject=0x5c000049 … status=0x00000000
```

— matched on the device side by `nvkvm: control 0xa06c0101 result 0x00000000 x3`. ★ The
positive control fired on the plane it was chosen for: `TSG_ALLOC_SEEN=3`,
`TSG_SCHED_SEEN=3`. (`PROMOTE_CTX_SEEN=0` again, and it is *still* the right value —
§16.55.5's correction holds; it is printed unpromoted precisely so its zero is legible.)

### 16.57.2 ★★★★ HOW FAR IT MOVED — every plane, `s44` → `s45`

| | `s44` | `s45` |
|---|---|---|
| RM ioctl records captured | 249 | **456** |
| `ALLOC hClass=0xa06c` (TSGs) | 1 | **3** |
| `0xc56f` channels | 8 | **16** |
| `0xc7b5` copy objects | 8 | **16** |
| ★ **allocations that failed** | 0 of 52 | **0 of 96** |
| commands decoded (device) | 589 | **717** |
| unserviced | 104, 42 distinct | 135, 46 distinct |
| controls answered | 143, 46 distinct | 150, 47 distinct |
| bridge refusals | 34 total, 6 distinct | 66 total, 6 distinct |
| ★★★ **doorbells** | 170 arrived, 170 served, **0 REFUSED** | **448 arrived, 261 served, 187 REFUSED** |
| `cuCtxCreate` | 801 | **801** |

⊘ **The rung did not pass, and it was never going to** — §16.55.7 said so before the boot:
*"do not read this as 'implement `0xa06c0101` and `cup2` goes green'."* What moved is the
**distance**: 207 more RM records, twice the context built, and the execution plane went
from a place no doorbell was ever refused (because none reached it) to one carrying 448.

★ Read the doorbell row carefully, because its two halves point opposite ways: `0 REFUSED`
in `s44` was **not** health — the channels were never scheduled, so the 278 doorbells that
now exist were never rung. `187 REFUSED` is new traffic meeting a named obstacle, not a
regression. A counter that improves by falling is one to check the denominator of.

### 16.57.3 ⇒ THE NEXT WALL, named at both ends — and there are TWO, on different planes

**Guest plane** — `[measured]` the same reading method §16.55.1 used, in guest order: the
last non-zero record before the `FREE` burst begins.

```
   327  ALLOC hClass=0x00000079 … status=0x00000000
   328  CTRL cmd=0x20801702 … status=0x00000056        ← MC_SERVICE_INTERRUPTS (×20+, forgiven)
   329  CTRL cmd=0x83de0309 hObject=0x5c000072 … status=0x00000056   ← NV83DE_CTRL_CMD_DEBUG_SET_EXCEPTION_MASK
   330  CTRL cmd=0x20801702 … status=0x00000056
★  331  CTRL cmd=0x20801210 hClient=0xc1d0000c hObject=0x5c000003 size=32 status=0x00000056
             in=01000000 1200005c 0000…      ← NV2080_CTRL_CMD_GR_SET_CTXSW_PREEMPTION_MODE
   332  FREE …                                ← teardown starts here
```

⇒ **`0x20801210` = `NV2080_CTRL_CMD_GR_SET_CTXSW_PREEMPTION_MODE`**, and its params name
the TSG: the second word is `0x5c000012`, the very group record 196 just scheduled. It is
**on the allowlist and in no claim list** — the same admitted-and-unserved shape.

★★★★★ **And this is where the new gate paid for itself, on its first outing.** The moment
`s45`'s log entered the tree, `every_unserviced_id_a_boot_recorded_is_classified` went **red**
and named five ids no earlier boot had ever reached, because `cup2` had never got this far:

```
★★★ 5 control id(s) reached the unserviced ledger in a committed boot and this port
    has no recorded position on them:
  0x20801210  (recorded by 1 boot(s): s45_748a207_tsgsched)   ← the new wall
  0x20801702  (recorded by 1 boot(s): s45_748a207_tsgsched)   ← MC_SERVICE_INTERRUPTS, x20+
  0x83de0309  (recorded by 1 boot(s): s45_748a207_tsgsched)
  0xa06c0103  (recorded by 1 boot(s): s45_748a207_tsgsched)   ← SET_TIMESLICE, in teardown
  0xa06c0105  (recorded by 1 boot(s): s45_748a207_tsgsched)   ← PREEMPT, in teardown
```

⊘ Note what the gate did **not** do: it did not rank them, and it cannot. Two of the five
arrive *inside* the `FREE` burst and are RM tearing the group down — that ordering came from
reading the stream, as §16.55.3 says it must. What the gate did is make it **impossible for
the five to enter the tree unremarked**, which is the whole of what was missing when
`0xa06c0101` sat unargued for six boots. The rung's own new ids forced their own rows, in
the same commit as the boot that produced them.

**Device plane** — `nvkvm: first doorbell refusal [CeResolve::NoPublication] no
page-directory root was published for (hClient 0xc1d0000c, hVASpace 0x5c000007)`, with a
`scan=1024/1024 declared (COMPLETE: every declared entry was read),
unread=1024, nonzero=NONE` beside it. `0x5c000007` is libcuda's own `FERMI_VASPACE_A` — the
handle §16.38 already identified as the one UVM dups.

⚠ ★★★★ **CORRECTION (§16.63) — that scan clause is a FABRICATION, and calling it a
"complete walk" here is the error it licensed.** `unread=1024` means **every** read failed,
because `read_published_va` answers `NoPublication` before touching any store; `nonzero=NONE`
is an empty vector nothing was appended to, and `COMPLETE` came from the loop bound alone.
Nothing was walked. Fixed at `ring_scan_sentence`.

⊘ **Do not assume these are one wall.** They are on different planes and no run has yet
established whether either causes the other `[not measured — stated as an open question,
not as an inference]`; `0x20801210` is what the *guest* gives up on, and
`CeResolve::NoPublication` is what *we* refuse. Establishing the relation is a measurement,
not an inference, and it is the next rung's question.

### 16.57.4 ⚠ THE REV-STAMP TRAP FIRED — on the first rebuild of this very rung

The first `build_qom_shim.sh` run of this rung was invoked with `CARGO_TARGET_DIR=/workspace/bench/cargo-target`
(the bench's shared cache). It exited **0**, relinked `qemu-system-x86_64`, and produced:

```
qemu-system-x86_64          → kayfabe-rev:b17381c…   ← the PREVIOUS rung's binary
cargo-target/…_raw.a        → kayfabe-rev:ce59601…   ← this rung's archive
```

`build_qom_shim.sh:38` reads `ARCHIVE="$REPO/target/release/libkayfabe_qemu_raw.a"` — a
**hard-coded path** that `CARGO_TARGET_DIR` silently redirects cargo away from. So the
script copied a stale archive, meson relinked, and everything downstream said *success*.

★ This is CLAUDE.md's *"the bench silently served a binary built from `862c7c2` for weeks"*,
reproduced in one command, and the only thing that caught it was reading the stamp off
**both** artefacts before booting. ⇒ Read the stamp off the **hypervisor**, not off the
archive and not off the build's exit code: the archive is what you built, the hypervisor is
what runs.

### 16.57.5 ⚠ ALSO LANDED — the refused **id** now crosses the seam (ABI 34 → 35)

`[measured 2026-08-10, over traces/guest_boots/*_qemu.log]` **`grep -c hClass` over every
committed device log returns `0`.** This port has never once named a class it refused:
`NotOnAllowlist x10` was the whole report, and §16.55.4 could only answer *which ten* by
reading the **guest's** dmesg — a plane we neither own nor always capture.

The cause is structural: `FaultTag` is a `&'static str`, `SharedRefusalCensus` keys on it,
and `BridgeRefusal::AllocClassNotPermitted { class, denial }` **captures the class** and then
drops it at the `FaultTag` boundary. ⇒ A method prescribed on that evidence — *"enumerate the
refused classes, then filter"* — could not have terminated.

Landed: `BridgeRefusal::fault_id`, `RefusalCensus::ids`, `KayfabeBridgeRefusal::{ids, ids_len}`,
`KAYFABE_REFUSAL_IDS_PER_TAG = 8`, and the C printer appends ` id=0x…,0x…` to the same line
so a grep for the tag returns them.

⊘ **Capped at eight per tag, and the cap is the security property**: the tag set is closed
and cannot grow with traffic, but an `hClass` is a **guest-supplied value** and an uncapped
set of them is an unbounded allocation a hostile guest drives directly (the
`GpuError::SpineCapacity` rule). ★ The cap is safe only because `count` is **not** capped —
`n` ids beside a larger count reads as a visible truncation, never as a complete list.

⚠ This is **not yet measured on a boot**: `s45` ran at `748a207`, ABI **34**, before this
landed. The next boot is where the first refused class id appears in a device log.

### 16.57.6 ★★★ THE FALSIFIER FOR `s46`, committed BEFORE the boot

§16.57.5 landed an **ABI bump** (34 → 35) and a claim that goes with it: *the change is
report-only and observationally neutral on the guest-facing plane.* ⊘ An ABI bump that has
never booted is a landmine — the C header and the Rust struct were edited in one sitting and
have never been linked into a running hypervisor together — and *"it is only a report"* is
exactly the sentence that precedes a perturbation.

`s45` is the reference, and it is a **strong** one: 456 RM records, record 196 `status=0`,
`0xa06c0101 ×3` all `NV_OK`, `448/261/187` doorbells, `717` commands decoded, `0` of `96`
allocations failed, wall at record 331.

⚠ **The positive control is the realize itself.** If the header and the archive disagree on
`sizeof(KayfabeBridgeRefusal)`, the shim's version handshake refuses and the device never
realizes — so a boot that reaches a login prompt with `/dev/nvidia0` open has already proved
the handshake. ⊘ That is not a control I chose; it is one the ABI design provides, which is
why `ABI_VERSION` exists at all.

| # | outcome | the distinguishable line | verdict |
|---|---|---|---|
| **E** | `s45`'s numbers reproduce **and** `grep -c "id=0x" run_s46_*_qemu.log` **> 0** | `nvkvm:   bridge refusal BridgeRefusal::AllocClassNotPermitted::NotOnAllowlist x10 id=0x…,0x…` | ★★★★ the reporting gap is closed and the first refused class ids appear in **our own** log — the thing `grep -c hClass` returned 0 for across every prior boot |
| **F** | `s45`'s numbers reproduce, `bridge refusal` lines present, **no `id=`** | `grep -c "bridge refusal" > 0` **and** `grep -c "id=0x" == 0` | ⊘ the ids are collected and not reported — a wiring gap, and `a_flag_is_not_progress` if it were shipped unread |
| **G** | the device does not realize | an ABI/`struct_size` message, or QEMU exiting before the guest boots | ⊘⊘ the header and the archive disagree. **Not** a port result; fix the two constants and re-boot |
| **H** | the boot runs and `s45`'s numbers do **NOT** reproduce | any of: record 196 back to `0x56`; RM records ≠ ~456; doorbells back near `170/170/0` | ★★★★ **the important one.** A report-only change that moves the guest-facing plane is a perturbation, and the claim in §16.57.5 is refuted. Report the delta before anything else |

⊘ **E requires BOTH halves.** `id=` lines with `s45`'s numbers not reproducing is `H`, not `E` —
a new instrument that also changed what it measures has answered a different question
(`a_correct_capture_can_answer_the_wrong_question`).

## §16.58 ★★★★ BOOTED `s46_1a9e93c_abi35` — **OUTCOME E**, both halves. The first refused class ids this port has ever printed

`[measured 2026-08-10, boot s46_1a9e93c_abi35]`. Stamp `kayfabe-rev:1a9e93c…` on **both**
the hypervisor and the archive, read before the boot. Evidence:
`traces/guest_boots/run_s46_1a9e93c_abi35_{qemu,dmesg,probe}.log`.

### 16.58.1 ★★★★ HALF ONE — the ids are in **our** log

```
nvkvm:   bridge refusal BridgeRefusal::AllocClassNotPermitted::NotOnAllowlist x18 id=0x0000c36f,0x0000c574
nvkvm:   bridge refusal BridgeRefusal::AllocClassNotPermitted::Refused        x3  id=0x0000402c,0x000083de
nvkvm:   bridge refusal BridgeRefusal::UnmappedAllocClass                     x10 id=0x00000070,0x00000079,0x0000208f
nvkvm:   bridge refusal BridgeRefusal::ReservedClient                         x2
nvkvm:   bridge refusal PromoteFault::UnknownContextObject                    x2
nvkvm:   bridge refusal RmGraphError::FreeUnknown                             x31
```

Before this rung, that block read `NotOnAllowlist x18` and nothing else, and
`grep -c hClass` over **every** committed device log returned `0`. Three rows now name what
they refused; three name nothing, correctly, because they are not about an id.

★★ **And the first reading is already worth the rung.** `0x000083de` is `GT200_DEBUGGER`
(`ogkm-580: class/cl83de.h:33`), refused under `Refused` — i.e. **denied by name in our own
`DENIED_CLASSES`** — and record 329 of the RM stream is
`0x83de0309 NV83DE_CTRL_CMD_DEBUG_SET_EXCEPTION_MASK`, a control on that class, two records
before the wall. ⊘ Whether the two are connected is **not measured** and must not be assumed
from adjacency; what is new is that the question is *askable from our own evidence*.

⚠ `0x0000c574` also appears here. §16.55.4 recorded it as *"`s38`-only, as claimed"* on the
strength of the **guest's** dmesg. ⊘ That is **not** contradicted and **not** confirmed:
this instrument did not exist for any earlier boot, so "absent from earlier device logs" and
"invisible to earlier device logs" are the same observation from here. One boot cannot
separate them; a second one, diffed against this, can.

### 16.58.2 ★★★★ HALF TWO — outcome `H` EXCLUDED. Every guest-facing number is byte-identical to `s45`

| | `s45` (ABI 34) | `s46` (ABI 35) |
|---|---|---|
| commands decoded / unserviced / distinct | 717 / 135 / 46 | **717 / 135 / 46** |
| bridge refusals | 66 total, 6 distinct | **66 total, 6 distinct** |
| controls answered | 150, 47 distinct | **150, 47 distinct** |
| isolates | 2 / 2 / 2 (2 no-plane) | **2 / 2 / 2 (2 no-plane)** |
| doorbells | 448 / 261 / 187 | **448 / 261 / 187** |
| RM records | 456 | **456** |
| record 196 | `0xa06c0101 … status=0x00000000` | **identical** |
| records 233, 270 | `0xa06c0101 … status=0x00000000` | **identical** |
| record 331 / 332 | `0x20801210 → 0x56` / `FREE` | **identical** |
| `CUP2_RC` | 1 | 1 |

⇒ The ABI 34 → 35 change is **report-only in fact and not merely in intent**, and `s45`'s
result reproduces on a second, independent boot at a different revision. ★ That second
property was not the falsifier's subject and is the more valuable one: `s45`'s numbers are
not a one-off — the whole of §16.57 replays.

⊘ **`G` was excluded before any of this was read**: the guest reached a login prompt and
`RmInitAdapter` ran, which a size disagreement between `kayfabe_shim.h` and the archive would
have prevented at realize. That is the ABI design being its own positive control.

### 16.58.3 ⊘ TWO CORRECTIONS FROM THE COORDINATOR, CHECKED AGAINST THIS TREE — one confirmed with a number, one already closed `[measured 2026-08-10, rev 2fa5d84 + boots s45/s46]`

`[measured 2026-08-10, rev 2fa5d84 + boots s45/s46]`

Both arrived after §16.58 landed. Each was **measured against this tree's own evidence**
rather than adopted.

**(1) `admitted ⊆ served` is also INVERTED — CONFIRMED, and quantified.**
`[measured 2026-08-10, rev 2fa5d84]` there are **29** controls the chain **serves** that
`capability.rs` does **not admit**, including both ids named to me (`0x20802a08`,
`0xa06c010a`). The other 27 are the `NV2080_CTRL_CMD_INTERNAL_*` family — kernel RM's own
GSP traffic, which by definition never crosses a userspace ioctl boundary and therefore has
no business on an `nvproxy`-derived allowlist.

⇒ The two surfaces are **not nested in either direction**: 142 admitted-and-unserved, 29
served-and-unadmitted. A gate built over `capability.rs` alone would report that this port
refuses commands it answers. `tests/tests/admitted_is_served.rs::the_chain_serves_controls_the_allowlist_does_not_admit`
pins the 29 as **membership**, so the inversion cannot drift silently in either direction.

**(2) "The class id is dropped into the tag; no grep can name what was refused" — ALREADY
CLOSED, and boot-measured, in §16.57.5 / §16.58.1.** `[measured 2026-08-10, boot
s46_1a9e93c_abi35]`:

```
nvkvm:   bridge refusal BridgeRefusal::AllocClassNotPermitted::NotOnAllowlist x18 id=0x0000c36f,0x0000c574
```

★ `0xc574` — the exact id described to me as *"invisible behind that aggregation"* — is one
of the two the line now names. It is `UVM_CHANNEL_RETAINER`
(`ogkm-580: class/clc574.h:33`), sourced rather than recalled.

⚠ **But only its MEMBERSHIP, not its count.** The row reports `x18` for the whole *tag*, and
`ids` is a **set**. So *"refused 8× per TSG"* is not something this instrument can confirm
or deny — 3 TSGs × 8 would be 24 and the tag's total is 18, but the tag also covers
`0xc36f`, so the arithmetic settles nothing. ⇒ Per-id **counts** remain unmeasured; the gap
narrowed from *"which ids"* to *"how many of each"*, and pretending otherwise would be the
saturated-instrument reading of my own new instrument.

**⊘ `0x20801702` is not a wall — agreed, and it was never claimed to be here.** §16.57.3
lists it under *"forgiven"* and the `LEDGER` row says *"×20+ and forgiven every time"*.
`[measured 2026-08-10, boots s45/s46]` it occurs **41** times in each boot's RM stream and
the guest continues past every one of them, reaching record 331. ★ The `cap3` datum offered
alongside — zero occurrences in 1122 elements, 28× only during a hang — is **cited, not
measured here**: this rung has not decoded `cap3`.

**★ Wall 1's proposed fix (an echo for `0x20801210`, `0x83de0309`, `0xa06c0103`) is NEXT
rung's, and it carries a hazard that must be named in advance.** All three params structs
have no `[OUT]` field, so an echo is *structurally* honest. Whether it is *semantically*
honest is the standing `NV_OK`-to-an-unperformed-action question — this port does not program
a preemption mode — and the only evidence either way is that the C echoed and reached
`bad=0 maxerr=0`. ⇒ Whoever lands it must state which of the two they are claiming, and
must give it a falsifier that is not the reply (there is nothing in the reply to get right),
exactly as `gpfifo_schedule.md` §2 had to for `0xa06f0103`.

---

## §16.59 ★★★★★ SERVE `0x20801210` — and the brief's central premise REFUTED: our request is not the C's

`NV2080_CTRL_CMD_GR_SET_CTXSW_PREEMPTION_MODE` is now claimed by `ObjectPolicy` and
**classified** rather than echoed. This section carries the refutations, the argument, and
the falsifier for `s47` — committed before the boot.

### 16.59.1 ⊘⊘⊘ REFUTED — "our request bytes match the C's byte-for-byte". THEY DIFFER IN THE ONE WORD THAT MATTERS

The brief for this rung said, of record 331:

> ★ The C's replies to all three are **byte-identical echoes with `NV_OK`** (`cap3`
> #453701/2, #453716/7, #453731/2), and **our request bytes match the C's byte-for-byte**:
> `01 00 00 00 | 12 00 00 5c | 00 00 00 00 | 02 00 00 00`.

The first clause is true and now independently verified. The second is **false**, and the
quoted bytes are **the C's, not ours**. `[measured 2026-08-10, cap3_matmul_forwarding
#453716 decoded from the committed capture, against boot s46_1a9e93c_abi35 record 331]`:

| | `flags` | `hChannel` | `gfxpPreemptMode` | `cilpPreemptMode` |
|---|---|---|---|---|
| C, `cap3` #453716 | `1` | `0x5c000012` | `0` | ★ **`2`** = `COMPUTE_CILP` |
| ours, `s46` record 331 | `1` | `0x5c000012` | `0` | ★ **`0`** = `COMPUTE_WFI` |

One byte, at offset 12. And it is the **only** word in the struct that decides whether an
`NV_OK` is a true sentence:

- `COMPUTE_CILP` = *"preempt the compute engine at the instruction level"*
  (`ogkm-580: ctrl2080gr.h:859`). The C answered `NV_OK` and had no such machinery — a
  promise, not an answer. It still reached `bad=0 maxerr=0`, because a short matmul never
  preempts and **nothing ever read the promise**.
- `COMPUTE_WFI` = *"the normal wait-for-idle context switch mode"* (`:857`). That is the
  state this port's execution plane is unconditionally in.

★★★★★ **A green oracle is evidence about the ORACLE'S payload, never about ours.** The
`c_oracle_empty_rows_are_wrong` lesson was *"an empty capture is evidence of nothing"*; this
is its live sibling and it is worse, because the capture here is **full, dense, correct and
verified** `[measured 2026-08-10, cap3_matmul_forwarding: n_errors=0, decompressed md5
`6cadc8e3cb2b5ce04c3059235b88e1e6` matching `MD5SUMS`, record index == seq]`. Diffing the **reply** shows a
perfect match and teaches you to ship the unconditional echo. Only diffing the **request**
catches it. ⇒ Before porting a C behaviour, diff the C's *input*, not its output.

⊘ **Why the two guests ask differently is `[not measured]`.** Both are `cup2`; both name the
same TSG handle `0x5c000012`; the C's `hClient` is `0xc1d00003` against our `0xc1d0000c`.
Stated as an open question, not inferred. (Cross-capture: the same trio appears in `cap2`
byte-identically and in **zero** of `cap1`, `cap1b`, `cap2b` — consistent with `cuCtxCreate`
emitting it, not driver load.)

⚠ Two smaller corrections from the same verification: the brief's index→id mapping is
**swapped** for the first two (`0x20801210` is #453716/7, `0x83de0309` is #453701/2), and
"byte-identical echo" is true of the **control body** only — the C rewrites `checkSum`,
`seqNum`, `rpc_result` and `rpc_result_private`, which is the only place `NV_OK` is actually
asserted. The `status` field *inside* `rpc_gsp_rm_control` is `0` in the **request** too, so
a checker reading `status` would score a green on a reply the C never touched.

### 16.59.2 ⊘⊘ REFUTED — "three controls, and the fix is an ECHO". There is ONE, and the other two are already decided AGAINST in this tree

The brief grouped `0x20801210`, `0x83de0309` and `0xa06c0103` as *"the same shape and the
same fix"*. `[measured 2026-08-10, boots s45/s46 + this tree]` they are three different
things, and two of the three were already answered **in this repository**:

| id | what it actually is | source |
|---|---|---|
| `0x20801210` | ★ the wall: record **331**, last non-teardown record | `s45`/`s46` probe logs |
| `0x83de0309` | **refused by name, deliberately**, in `capability.rs`'s `DENIED_CONTROLS` under `DeniedBecause::SmDebuggerTrapping` — *moved off the allowlist* by an earlier rung because this port does not implement SM debugger trapping at all | `crates/kayfabe-abi/src/capability.rs:1338-1348` |
| `0xa06c0103` | record **344**, i.e. **inside the `FREE` burst that begins at 332** — RM tearing the group down | `run_s46_*_probe.log:111` |

⊘ `0x83de0309` is not an unserved gap; it is a **kept decision**. Echoing `NV_OK` to
`SET_EXCEPTION_MASK` (`exceptionMask = 0x3a` = TRAP|INT|CILP|PREEMPTION_STARTED) would claim
we armed SM exception trapping we have not armed, and would reverse a narrowing this repo
made on purpose and wrote down. ⊘ `0xa06c0103` is in teardown: serving it cannot move any
wall, and `tests/tests/admitted_is_served.rs:256` already said so in the commit *before* the
brief was written.

★ And by the brief's **own** discriminator — *"the discriminator is what the caller does
next"* — `0x83de0309` is not a wall either: records 330 and 331 follow it. That is the same
argument the brief used, correctly, to retract `0x20801702`.

### 16.59.3 ⊘ REFUTED — "we would be answering `NV_OK` to an action we did not perform"

The brief offered *"structurally honest, semantically unverified"* as an acceptable position.
It is available, and this rung does not take it, because a stronger one is.

The whole answer to this control is **ours by the driver's own routing**: its dispatch row is
`flags=0x10348` = `NON_PRIVILEGED | ROUTE_TO_PHYSICAL | API_LOCK_READONLY |
ROUTE_TO_VGPU_HOST | GSP_PLUGIN_FOR_VGPU_GSP`
(`ogkm-580: g_subdevice_nvoc.c:9361-9374`, bits at `control.h:205,230,244,250,287`), and
`subdeviceCtrlCmdKGrSetCtxswPreemptionMode` has **no `_IMPL` body anywhere in the open tree**
— only the generated dispatch row (`compute_limiting_and_priority.md` §3.3). On a GSP client
the CPU half does nothing at all; the mode is programmed inside signed firmware. **We are
that firmware.** There is no upstream semantics to be faithful to — only our own execution
plane to tell the truth about.

So decompose it the way `gpfifo_schedule.md` §2 decomposes its control:

| | claim | ours? |
|---|---|---|
| **P1** the context switches at **wait-for-idle** after this call | ★ **yes, and verifiable** — this port has no preemption machinery of any kind, so WFI is not a mode it fails to program, it is the only mode it has |
| **P2** the context switches at CTA / CILP / GfxP | ⊘ **no**, and `[unknown]` even for the silicon: no `_IMPL`, no `bCilpSupported` symbol in the tree |
| **P3** the mode is written to a hardware register | ⊘ not modelled, and not observable to the guest through any path this port serves |

⇒ The arm **classifies the request** and answers `NV_OK` only for P1. The claim in the commit
is **"classified, then answered"** `[verified by mutation 2026-08-10: deleting the classifier
turns 3 of the 8 tests in tests/tests/ctxsw_preemption_mode.rs red]` — a request for CILP,
CTA or GfxP is refused by name
(`CtxswPreemptionFault::PreemptionNotImplemented`). ⊘ Note what that costs: on the C's own
payload this port would **refuse** where the C said `NV_OK`. That is deliberate, and it is
the difference between the two implementations, not an oversight.

★ **The refusal status is `NV_ERR_NOT_SUPPORTED`, and for once that is not a bent rule.**
`ctrl2080gr.h:791-795`, of this exact command: *"A value of `NV_ERR_NOT_SUPPORTED` is
returned if the target channel does not support preemption context switch mode changes."*
The standing rule forbids **borrowing** a status whose meaning is *absent*; here the header
supplies it for the meaning we intend. `bind_channel.rs`'s per-id split now carries three
arms, and the third's reason is different in kind from `GPU_PROMOTE_CTX`'s: that one uses
`0x56` for its *effect* (any other status kills the adapter), this one uses it because it is
*true*.

### 16.59.4 THE CODE

| layer | what landed |
|---|---|
| `kayfabe-abi/src/submit.rs` | `CtxswPreemptionRequest` (32 B, compile-time offsets), `CtxswPreemptionAsk` + `asks_for`, `decode/encode_ctxsw_preemption_mode`, `CTXSW_PREEMPTION_REFUSED_STATUS`, the mode/flag constants |
| `kayfabe-core/src/gpu.rs` | `route_ctxsw_preemption`, `CtxswPreemptionAck` / `CtxswPreemptionFault`, `Gpu::set_ctxsw_preemption_mode` (`&self` — nothing is recorded, because there is nothing to record) |
| `kayfabe-rmrpc/src/policy.rs` | `OBJECT_CONTROLS` += `0x20801210`; `respond_ctxsw_preemption_mode`; `ObjectModel::set_ctxsw_preemption_mode` |
| `kayfabe-rt/src/device.rs` | `SharedDevice::set_ctxsw_preemption_mode` — rank 0 only, no proc lock |
| `kayfabe-qemu-raw/src/shim.rs` | the shell's seat |
| `tests/tests/ctxsw_preemption_mode.rs` | the classifier, both measured payloads as fixtures, the non-vacuity |

⚠ **The `as_gpu` trap was live and was avoided by having been written down.** The first
draft of the policy arm read `self.gpu.as_gpu()`. The shipped composition root installs a
sharded shell whose `as_gpu` returns `None` **by design**, so that arm would have refused
`0x20801210` on **every real boot** while passing every test in `ctxsw_preemption_mode.rs`
(which composes a bare `Gpu`). `ObjectModel::vas_census` was made a trait method for this
exact reason and its own doc carries the case (`skipped_oracle_kills_the_guard`); the trap
is therefore not a new finding, only a re-encounter. It is a trait method now.

★ **The mode classification is in `kayfabe-abi`, not `kayfabe-core`** — `kayfabe-core` does
not depend on `kayfabe-abi`, and which modes a request names is a pure question about a wire
struct. Same split as `respond_bind`'s engine-space conversion.

### 16.59.5 ★★★ THE FALSIFIER FOR `s47`, committed BEFORE the boot

⊘ **The reply is not the falsifier and cannot be.** Every field of the params struct is
`[IN]`; the reply is the request's own bytes by construction, so checking it tests a
`copy_from_slice`. Two things discriminate, and both are named here in advance:

- **at unit level** — hold all 32 bytes fixed and move `cilpPreemptMode` from `0` to `2`; the
  answer must change. `[verified by mutation]` deleting the classifier from
  `respond_ctxsw_preemption_mode` turns **3 of the 8** tests in
  `tests/tests/ctxsw_preemption_mode.rs` red;
- **at boot level** — **what the guest does next.** Record 332 currently begins the `FREE`
  burst. Whether it still does, with record 331 at `status=0`, is the whole result.

Instrument: `scripts/bench/boot_capture.sh s47_<rev>_ctxsw` with
`POST_CAPTURE_HOOK=scripts/bench/cup2_hook_rmtrace.sh`, read **in guest order**.

⚠ **Positive control, chosen from the population the instrument can see**, and measured
present in *both* prior boots: `CTRL cmd=0xa06c0101 … status=0x00000000` **×3**
(`TSG_ALLOC_SEEN=3`, `TSG_SCHED_SEEN=3`). ⊘ If it is absent, nothing below is scored — the
capture did not reach the context build. ⚠ And read `kayfabe-rev` off the **hypervisor**
before anything else (§16.57.4).

| # | outcome | the distinguishable line | verdict |
|---|---|---|---|
| **I** | record 331 `status=0x00000000` **and** records after 332 that are not `FREE`/`ESC nr=0x4f` | `331: CTRL cmd=0x20801210 … status=0x00000000` followed by a non-teardown record | ★★★★★ the wall moved. **Report the id and status of the next non-zero record** |
| **J** | record 331 `status=0x00000000` **and** record 332 still begins the `FREE` burst | `331 … status=0x00000000` / `332 FREE` | ★★★★★ **the most valuable outcome, and the one I predict.** The control was *served* and the guest still gives up ⇒ record 331 was **never** the blocker, and the "last non-zero record before the `FREE` burst" reading method — which named the walls at §16.55, §16.57 and this one — has produced a **false positive**. Wall 2 becomes the only live candidate and the two walls are **not** one wall |
| **K** | record 331 still `0x56` **and** `unserviced fn 76 cmd 0x20801210` still in the qemu log | the ledger line survives | ⊘⊘ **instrument/plumbing failure, not a port result.** `0x56` here is *"nobody claimed it"* and this rung claimed it. Check the stamp on the **hypervisor** |
| **L** | record 331 `0x56`, **and** the ledger line is GONE, **and** `control 0x20801210 result 0x00000056` appears in the control census | ledger absent + census present | ★★★★ **we claimed it and refused it** — either the classifier refused (⇒ **report the `in=` bytes**: this boot's guest asked for something other than WFI, which would itself refute §16.59.1's stability) or `hChannel` did not resolve in our graph. A real result, not a failure |
| **M** | `s45`/`s46`'s numbers do **not** reproduce | doorbells ≠ `448/261/187`; RM records ≠ 456; record 196 not `status=0` | ★★★★ a claim added to the GSP-RPC plane perturbed the rest. **Report the delta before anything else** |

★ **The prediction, stated so it can be wrong: `J`.** Three grounds, two
`[measured 2026-08-10, boots s45_748a207_tsgsched and s46_1a9e93c_abi35]` and one sourced:
(1) `0x56` is this control's **own documented** answer for a target that does not support
mode changes, so a guest treating it as fatal would be treating a legitimate status as fatal;
(2) the guest in this very window tolerates `0x56` **41 times** from `0x20801702` and once
from `0x83de0309` and keeps going; (3) `s45` measured **187 refused doorbells** with
`CeResolve::NoPublication` — the guest's work never ran — and `cuCtxCreate` returning 801 is
a statement about the execution plane, not about a preemption knob.

⊘ Against the prediction, and it is not weak: record 331 **is** the last non-zero record
before teardown, which is the strongest signal the guest plane offers, and it is the signal
that named the last two walls correctly. If `I` lands, the prediction is simply wrong and the
reading method is vindicated a third time.

⊘ **`I` and `J` are both wins and `J` is the bigger one**, because it retires a *method*
rather than a control. Do not read `J` as "the rung failed": the control is served either
way, honestly, and one of this campaign's most-used instruments would be shown to have a
false-positive mode.

## §16.60 ★★★★★ BOOTED `s47_81582e3_ctxsw` — **OUTCOME J**, the predicted one. The control serves, and record 331 was NEVER the wall

`[measured 2026-08-10, boot s47_81582e3_ctxsw]`. Stamp `kayfabe-rev:81582e3f76cc…` read off
**both** the hypervisor and the archive **before** the boot. Evidence:
`traces/guest_boots/run_s47_81582e3_ctxsw_{qemu,dmesg,probe}.log`, tracked, passing
`assert_boot_evidence.sh` (148 files).

### 16.60.1 THE ONE-LINE RESULT

```
s46:   331  CTRL cmd=0x20801210 … size=32 status=0x00000056   in=01000000 1200005c 00000000 00000000 …
s47:   331  CTRL cmd=0x20801210 … size=32 status=0x00000000   in=… out=… (echoed)
       332  FREE hRoot=0xc1d0000c hParent=0x5c00001a hObject=0x5c00007a status=0x00000000
```

The control is **served** — `nvkvm: control 0x20801210 result 0x00000000 x1`, and
`grep -c "unserviced fn 76 cmd 0x20801210"` on this boot's device log returns **0**, where
`s45` and `s46` both carried the line. And **record 332 still begins the `FREE` burst**, from
the same object, at the same index, as it did when 331 answered `0x56`.

⊘ Note the request bytes: `cilpPreemptMode = 0`, exactly as `s46`. §16.59.1's divergence from
the C is **stable across boots**, not a one-off — which also excludes the `L` reading (the
classifier did not refuse; it served, because the guest asked for wait-for-idle again).

### 16.60.2 ★★★★★ THE FINDING — a reading METHOD produced a false positive, and it named three walls

`0x20801210` was identified as the wall by one rule, applied in §16.55.1 and §16.57.3 and
inherited by this rung's brief: **"the last non-zero record before the `FREE` burst."** The
rule is what named `0xa06c0101` at `s44` (correctly — record 196 went `0x56` → `0` and the
guest built **two more** channel groups and issued 207 more ioctls). Applied to `s45` it named
`0x20801210`. `[measured]` that one is **wrong**: the record is now `status=0` and nothing
about the guest's behaviour changed.

★★★★ **And the method has now run out of candidates entirely**, which is the cleanest possible
demonstration that it was the wrong instrument. The non-zero records still standing before 332
are records 328/330 (`0x20801702`, `0x56`) and record 329 (`0x83de0309`, `0x56`) — **both
already retracted as walls**, by the coordinator and by §16.59.2 respectively, on the grounds
that the guest continues past them. The rule's next answer is an id its own users have already
ruled out.

⇒ **"Last thing before teardown" is a statement about ORDER, and a blocker is a statement about
CAUSE.** They coincide when the failing call is the one the caller checks; they come apart the
moment a caller batches its checks, or gives up for a reason decided earlier and surfaced at
the next syncpoint. The discriminator that survives is the coordinator's — *what does the
caller do next* — and its honest reading here is that the caller does **the same thing either
way**, i.e. record 331 carries no information about why `cuCtxCreate` fails.

⊘ **This does not retract §16.56/§16.57.** `0xa06c0101` was a real wall and the evidence is
independent of the rule that found it: serving it took RM records 249 → 456, TSGs 1 → 3,
channels 8 → 16, and doorbells 170 → 448. A method that is right once and wrong once is not
discredited, it is **scoped** — and its scope is *"generates candidates"*, never *"identifies
blockers"*.

### 16.60.3 EVERY OTHER NUMBER IS `s46`'s, TO THE COMMAND — `M` EXCLUDED

| | `s46` | `s47` | |
|---|---|---|---|
| RM records captured | 456 | **456** | ✓ |
| `0xa06c0101` at `status=0` | ×3 | **×3** | ✓ positive control (`TSG_ALLOC_SEEN=3`, `TSG_SCHED_SEEN=3`) |
| commands decoded | 717 | **717** | ✓ |
| unserviced / distinct | 135 / 46 | **134 / 45** | ★ **exactly one fewer, and it is `0x20801210`** |
| controls answered / distinct | 150 / 47 | **151 / 48** | ★ **exactly one more, and it is `0x20801210`** |
| bridge refusals | 66 total, 6 distinct | **66 total, 6 distinct** | ✓ |
| doorbells | 448 / 261 / 187 | **448 / 261 / 187** | ✓ |
| isolates | 2 / 2 / 2 (2 no-plane) | **2 / 2 / 2 (2 no-plane)** | ✓ |
| `CUP2_RC` | 1 | **1** | ✓ |

★ The two rows that moved moved by **exactly one, in opposite directions, on the same id**.
That is the arithmetic signature of a command changing seats and nothing else happening, and
it is a stronger exclusion of `M` than any single equality in the table.

⚠ **A miscount of my own, recorded because it nearly became a reported perturbation.** My first
pass grepped record lines over the **whole** probe log and got **538**, an apparent +82. The
probe log prints the complete stream *and then reprints* `the LAST 80 RM records`; the
duplicated region is the difference.
`[measured 2026-08-10, boot s47_81582e3_ctxsw]` the stream between its own delimiters is
**456**. ⇒ Count within the section delimiters, not over the file — an instrument that prints
one region twice makes a count over the file a count of the instrument.

### 16.60.4 ⇒ WALL 2 IS THE ONLY LIVE CANDIDATE, AND IT IS BYTE-IDENTICAL

```
nvkvm: first doorbell refusal [CeResolve::NoPublication] no page-directory root was published
  for (hClient 0xc1d0000c, hVASpace 0x5c000007) … scan=1024/1024 declared (COMPLETE: every
  declared entry was read), unread=1024, nonzero=NONE … walk=NO-PUBLICATION
```

Unchanged from `s45` and `s46`, down to the handle. **187 of 448 doorbells refused**, and
`0x5c000007` is libcuda's own `FERMI_VASPACE_A`.

⚠ ★★★★ **CORRECTION (§16.63) — the `scan=…` half of that line is a FABRICATION and I cited
it here.** `read_published_va` answers `NoPublication` *before it touches any store*, so all
1024 reads failed: `unread=1024` means **nothing was scanned**, `nonzero=NONE` is an empty
vector that was never appended to, and `COMPLETE` was computed from the loop bound with no
reference to `unread`. The clause said the ring's entries were zero when **no entry was
read**. ⇒ Read this paragraph as *"the refusal is identical across boots"*, which is true and
is all it ever measured; the scan clause is `NoPublication` restated. Fixed at
`ring_scan_sentence`, which now says `⊘ NOTHING WAS READ`.

⊘ **And the question the brief posed is now answered.** *"Nothing has measured whether Wall 1
and Wall 2 are one wall"* — `[measured 2026-08-10, boot s47]` they are **not**: Wall 1 was
removed and Wall 2 did not move by a single count. More than that, Wall 1 was not a wall at
all, so there was never a pair. The next rung's question is `CeResolve::NoPublication` and
nothing else on the control plane.

★ The control stays served. It is honest, it is classified rather than echoed, it costs
nothing, and it removes an id from the unserviced ledger that would otherwise have been
re-nominated as the wall by the next reader applying the same rule.

## §16.61 ★★★ THE FALSIFIER FOR `s48` (`w201-completion-observer`), committed BEFORE the boot

`w201` wires the guest doorbell to the only function in this tree that observes a real host
GPU completion — arm **(b)** of `completion_wait_architecture.md` §4, no completion tail. It
had slipped five rungs. Re-verified on this base rather than trusted:
`tests/tests/doorbell_reaches_the_completion_observer.rs` is **0 passed / 2 failed** at
`a12456e` and **3 passed / 0 failed** at `e7bed44`, and the two commits are separate so each
half is falsifiable on its own.

### 16.61.1 ⚠ WHAT MAKES THIS A LIVE-PATH RUNG AND NOT A DARK ONE

`SharedDevice::doorbell` now calls `forward_ring` **whenever the caller passes a VMM port**,
and `kayfabe-qemu-raw`'s `SharedDoorbell::ring` passes its attached port whenever it has one
(`device.rs:1607-1631`). ⊘ That is **not** gated on `KAYFABE_ISOLATES` — the isolate plane
selector gates what happens *after* `forward_ce`, not whether the ring is read. So on a
**default** boot, every doorbell that `plan_doorbell` accepted now attempts a ring read where
it previously did nothing at all. `[measured 2026-08-10, boot s47_81582e3_ctxsw]` that is
**261** doorbells.

★★★ And `forward_ring` propagates every failure with `?`. This is residual **(4)** of the
wiring commit's own list, stated there as `[NOT MEASURED]`:

> a doorbell whose ring now faults (MISS, wrong aperture, an unretired copy) **refuses**
> where it previously reported `Served`.

⇒ **The doorbells row is the falsifier.** `s47` is the reference: `448 arrived, 261 served,
187 REFUSED`.

⊘ **A limit of this instrument, named in advance so it is not discovered as a result.** A
ring read that runs and legitimately finds nothing (`read_gpfifo_ring` answers `Ok(None)` for
the three shapes that are the *guest* saying something) is **indistinguishable in the device
report** from a ring read that never ran. So outcome `N` below cannot prove the new code
executed; it can only prove it did no harm. ⊘ *"The port is passed"* is **unmeasured on a
boot**: it is read from `kayfabe-qemu-raw`'s `SharedDoorbell::ring`, and a source read is not
a run.

### 16.61.2 THE TABLE

Instrument: `scripts/bench/boot_capture.sh s48_<rev>_cwait` with the same
`POST_CAPTURE_HOOK=scripts/bench/cup2_hook_rmtrace.sh`. ⚠ Read `kayfabe-rev` off the
**hypervisor** first (§16.57.4), and run the liveness check this rung's own boots ran —
`pgrep -x qemu-system-x86` **and** `ss -tln | grep 2223` — rather than inheriting anyone's
"bench is clean".

⚠ **Positive control**, `[measured 2026-08-10, boots s45_748a207_tsgsched,
s46_1a9e93c_abi35 and s47_81582e3_ctxsw]`: `CTRL cmd=0xa06c0101 … status=0x00000000` **×3**,
`TSG_ALLOC_SEEN=3`, `TSG_SCHED_SEEN=3`. ⊘ If absent, nothing below
is scored.

| # | outcome | the distinguishable line | verdict |
|---|---|---|---|
| **N** | `doorbells: 448 arrived, 261 served, 187 REFUSED` **and** the guest plane reproduces `s47` | every row of §16.60.3 unchanged | ★★★ the wiring is **observationally neutral on the shipping path**, and residual (4) did not bite on this workload. ⊘ Does **not** show the observer was reached — see §16.61.1 |
| **O** | `served` **< 261** (and `REFUSED` up by the same amount) | the doorbells row moves | ★★★★ **residual (4) BIT** — the most informative outcome. **Report the fault tag**: the refusal carries a name (`Faulted::fault_tag`), which is exactly why the wiring was written to refuse rather than to swallow. This is a real behaviour change on live traffic and it is what a boot is for |
| **P** | the guest plane moves: RM records ≠ 456, record 331 ≠ `status=0`, `0xa06c0101` not ×3 at `status=0`, or `CUP2_RC` ≠ 1 | any row of §16.60.3 | ★★★★ the ring read perturbed the **control** plane, which it has no business touching. Report the delta before anything else |
| **Q** | the guest never reaches a login prompt, or `cup2` never returns inside the hook's 180 s deadline | `boot_capture.sh` times out; no `CUP2_RC` line | ⚠ **residual (5) is the first suspect**, not the only one: `CeShellState::vmm` is held across a call that can park in `PoolGate::wait_for_return`, and a hang is the shape a lock inversion takes. ⊘ But **a slow boot is not a crash** — the guest needs ~20–25 s and `-serial file:` lags. Confirm with `ss -tln \| grep 2223` before calling it a hang |
| **R** | the device does not realize | an ABI/`struct_size` line, or QEMU exiting before the guest boots | ⊘⊘ `PushbufferAbi` gained a **required** method (`gpfifo_entry_stride`) this rung; a shim/archive disagreement would surface here. Not a port result |

⊘ **`N` is the expected outcome and it is the least interesting one.** `O` is the one worth
having: it converts a `[NOT MEASURED]` residual into a measurement, and the whole reason the
wiring propagates the fault instead of swallowing it is so that `O` is *legible* rather than
silent. ⇒ Do not read `O` as a regression to be reverted without reading the tag first — a
doorbell that reports `Served` on a copy that never retired is the defect §14.8 names, and
refusing it is the correction.

## §16.62 ★★★★★ BOOTED `s48_4f5b357_cwait` — **OUTCOME N**, and N is WEAKER than §16.61 wrote it. My own premise is refuted

`[measured 2026-08-10, boot s48_4f5b357_cwait]`. Stamp `kayfabe-rev:4f5b357d0594…` read off
**both** artefacts before the boot; bench liveness checked with `pgrep -x qemu-system-x86`
**and** `ss -tln | grep 2223` rather than inherited. Evidence:
`traces/guest_boots/run_s48_4f5b357_cwait_{qemu,dmesg,probe}.log`.

### 16.62.1 THE RESULT — every plane identical, and the device logs are identical *as logs*

| | `s47` | `s48` |
|---|---|---|
| ★ **doorbells** | 448 / 261 / 187 | **448 / 261 / 187** |
| commands decoded / unserviced / distinct | 717 / 134 / 45 | **717 / 134 / 45** |
| controls answered / distinct | 151 / 48 | **151 / 48** |
| bridge refusals | 66 total, 6 distinct | **66 total, 6 distinct** |
| isolates | 2 / 2 / 2 (2 no-plane) | **2 / 2 / 2 (2 no-plane)** |
| RM records / record 331 / record 332 | 456 / `status=0` / `FREE` | **456 / `status=0` / `FREE`** |
| `0xa06c0101` at `status=0` | ×3 | **×3** (`TSG_ALLOC_SEEN=3`, `TSG_SCHED_SEEN=3`) |
| `CUP2_RC` | 1 | **1** |

★ Stronger than a table: normalise both device logs (strip timestamps, fold every hex and
decimal literal) and `diff` the sorted line-kind census — **they are identical, exit 0**. Not
one line kind appeared, disappeared, or changed count. `O`, `P`, `Q` and `R` are all excluded.

⊘ `Q` deserves a word because it looked live for ~20 s: the guest reached
`ubuntu login:` and then the device log went quiet while `cup2` ran. That is not a hang and
CLAUDE.md says so — *a slow boot is not a crash*. `CUP2_RC=1` arrived, same as every boot
since `s45`.

### 16.62.2 ⊘⊘⊘ AND HERE IS THE REFUTATION, AND IT IS OF §16.61.1 — MINE, WRITTEN HOURS EARLIER

§16.61.1 opened *"WHAT MAKES THIS A LIVE-PATH RUNG AND NOT A DARK ONE"* and asserted:

> on a **default** boot, every doorbell that `plan_doorbell` accepted now attempts a ring
> read where it previously did nothing at all. […] that is **261** doorbells.

**That is wrong, and the identical logs are what sent me back to check it.** `SharedDoorbell::ring`
tries `try_ce_submission` **first** and returns its report if it is `Some`
(`shim.rs:2732-2739`). That function declines — falls through to `SharedDevice::doorbell`,
which is the only caller of `forward_ring` — under exactly one condition:

```rust
if facts.vas_pdb.is_some() && !self.local_ce_is_the_only_executor {
    return None; // the core can address AND serve this channel; it is not ours.
}
```

and `local_ce_is_the_only_executor` is set to `isolate_plane == IsolatePlane::Stillborn`
(`shim.rs:3849`). `[measured 2026-08-10, boot s48]` the isolate census reads
`2 refusing (2 no-plane)` — **Stillborn**. ⇒ On a shipping-default boot that flag is `true`,
`!true` is `false`, and the guard **never** declines on those grounds.

★★★★ **So the only way to reach `forward_ring` on a default boot is through
`try_ce_submission`'s three earlier `?`s** — `facts.vaspace?`, `facts.ring_va?`,
`self.plane.upgrade()?`. Two of those three are *"this channel has no VA space"* and *"this
channel declared no ring"*, which are precisely the shapes `read_gpfifo_ring` answers
`Ok(None)` for. ⇒ **On the shipping build the wiring can only be reached by a channel that
has no ring to read.** It is inert by construction, not by accident, and `s48` could not have
shown it working whatever the guest did. ⊘ This paragraph is a **source reading**
(`shim.rs:2732-2739, 2874-2877, 3849`) with one boot fact in it (the Stillborn census line);
it is not itself a measurement, and §16.62.3 says what it would take to make it one.

⊘ This is the brief's own horizon note arriving one layer lower than expected: *"the isolate
plane is `Stillborn` unless a non-default feature is on, and flipping it regresses the CE
path that works today"* (§14.24). The completion observer is behind the **same** gate as GR
compute, and `w201` does not move that gate — correctly, because moving it is the three-way
client-kind routing key, which is design and is not this rung.

### 16.62.3 ⚠ AND THE INSTRUMENT CANNOT SETTLE IT EITHER — a bounded log read as a census

I reached for the doorbell log to check how many of the 261 served doorbells were local:

```
doorbells: 448 arrived, 261 served, 187 REFUSED by name; last token 0x00010001 (16 logged)
```

All **16** logged are `SERVED-LOCAL [CpuCe::ServedLocally]`, in `s47` and in `s48`. ⊘ **16 of
448 is a bounded sample, not a census** (`a_small_count_is_not_a_small_event`, and the
coordinator's own warning about membership-versus-distribution this same night). The census
line counts `served` **without splitting it by `DoorbellReport` kind**, so *"how many of the
261 reached `SharedDevice::doorbell`"* is `[NOT MEASURED]` and cannot be recovered from any
committed log.

⇒ **The next step here is an instrument, not a conclusion**: split the `served` counter by
report kind (`Served` vs `ServedLocally`). That is a census change, it costs nothing, and it
converts *"is the forwarding path reached at all on a default boot?"* from a source reading
into a number a boot prints. ⊘ Until then, §16.62.2's answer stands on `ogkm`-style code
reading alone, and is tagged as such.

### 16.62.4 WHAT `s48` DID ESTABLISH, stated at its real strength

- ★★★ **The wiring is SAFE to carry on the shipping path**: byte-identical device report,
  identical guest plane, `cup2` unchanged. Residual **(4)** — *"a doorbell whose ring now
  faults refuses where it previously reported `Served`"* — **did not bite**, and residual
  **(5)** — `CeShellState::vmm` held across a possible `PoolGate::wait_for_return` park — did
  not deadlock. Both remain `[NOT MEASURED]` **in the sense that matters**, because §16.62.2
  says the code they describe was almost certainly not executed.
- ★★★ **The severance is closed in the tree**, which is what it was for:
  `doorbell_reaches_the_completion_observer.rs` is **0/2 at `a12456e`, 3/3 at `e7bed44`**,
  re-verified on this base rather than inherited. `HostRmBackend::await_semaphore` now has a
  production caller; whether a *default* build can reach it is §16.62.2's separate answer.
- ⊘ **It did NOT establish that a guest doorbell reaches a real host completion on the bench.**
  Saying otherwise would be the five-rung-old claim this boot was run to test, restated.

## §16.63 ★★★★★ AN INSTRUMENT THAT ASSERTED A FALSEHOOD ON FOUR BOOTS — and I cited it twice before checking it

`[measured 2026-08-10, census over every committed `traces/guest_boots/*_qemu.log`]`

### 16.63.1 THE CENSUS THAT SETTLES IT — and it splits cleanly

```
   9  scan=64/1024    unread=0     nonzero=NONE     ← honest: bounded, and it says so
   1  scan=1024/1024  unread=0     nonzero=NONE     ← honest: a real complete scan
   4  scan=1024/1024  unread=1024  nonzero=NONE     ← ★ FABRICATED
```

The four are `s45_748a207_tsgsched`, `s46_1a9e93c_abi35`, `s47_81582e3_ctxsw` and
`s48_4f5b357_cwait` — every boot since the wall reached `CeResolve::NoPublication`. The lone
honest complete scan is `s17`/`s19`/`s20`/`s21`'s.

`MemoryPlane::read_published_va` answers `Err(Unresolved(NoPublication))` **before it touches
any store**. So under a `NoPublication` all `n` reads fail:

- `unread == n` — nothing was scanned;
- `nonzero` is empty because **nothing was ever appended to it**, not because the entries
  were zero — yet the sentence read `nonzero=NONE — every scanned entry is ZERO`;
- `COMPLETE: every declared entry was read` was computed from the **loop bound alone**, with
  no reference to `unread`.

⇒ The line restated `CeResolve::NoPublication` as if it were independent evidence about the
ring's *contents*. Two of your own computations agreeing is not corroboration
(`measure_at_the_boundary_not_inside`) — and here they were not even two: it was one fact
printed twice, the second time wearing a different instrument's clothes.

### 16.63.2 ⚠ AND I CITED IT — twice, in this document, in the same session

- §16.57.3 called it *"with a complete walk beside it"*. Nothing was walked.
- §16.60.4 quoted it whole as the evidence that Wall 2 was unmoved.

⊘ **The load-bearing half of both citations survives**: the refusal line *is* byte-identical
across `s45`–`s48`, and that identity is what both paragraphs actually needed. The scan clause
added nothing and asserted something false. Both are corrected in place rather than deleted,
because a citation that was wrong is worth more visible than absent.

★ **The older citation is exonerated by the same census, and that matters.** §12's *"the scan
cap was NOT the cause, and lifting it proved so — the whole ring is zero, not the first
6.25 %"* rests on the **`unread=0`** complete scan. It read all 1024 entries and they were
zero. ⇒ This is not "the instrument was always lying"; it is *"the instrument became a liar
the moment its subject started failing to resolve"*, which is a narrower and more useful
statement — and it is why the fix is a **guard**, not a deletion.

### 16.63.3 THE FIX

`ring_scan_sentence` is now a free function (testable without a plane) with three states
instead of two:

| condition | sentence |
|---|---|
| `unread == n` | `⊘ NOTHING WAS READ: all n of N declared entries failed to resolve, so this scan says NOTHING about the ring's contents — it is the resolution failure above, restated` |
| `0 < unread < n` | `nonzero=NONE among the {n - unread} entries that RESOLVED` |
| `unread == 0` | unchanged — `COMPLETE` / `every scanned entry is ZERO` |

⊘ The guard is `unread == n`, **not** `unread > 0`: a partial read really did scan
`n - unread` entries and those are legitimately reported, with the denominator said out loud.
Four tests in `kayfabe-qemu-raw`, one per state plus the non-vacuity that a real complete scan
still reports completeness.

★★★ **The transferable rule, and it is a sharper form of one this campaign already has.**
`RING_SCAN_ENTRIES`' own doc says *"when the bound and the declared size differ, the sentence
itself must change — a reader should not have to do the division."* That rule was applied to
the **numerator** and not to the **failure count**. ⇒ **Every field a diagnostic prints must
be derivable from what was actually observed, and a summary clause must be computed from the
same variables it describes.** `COMPLETE` was computed from `n` and `entries` while claiming
something about `unread`; that is the whole defect, and it is checkable by inspection.

### 16.63.4 ⇒ WHAT THIS DOES *NOT* CHANGE, and the next rung's question

⊘ The refusal itself is untouched: `CeResolve::NoPublication` for `(0xc1d0000c, 0x5c000007)`
is real, reproduces across four boots, and is the live wall. What the correction removes is a
**false corroboration** of it — one that made "the ring is empty" look like an independent
finding when it was the same refusal wearing a scan's clothes.

★★★★★ And the coordinator's parallel read answers the next question, verified here against
`s45`'s own log rather than adopted: line 169 of `run_s45_748a207_tsgsched_qemu.log` carries
`{8x pdb=Y own=not-declared cs=ok(h0x5c000007=>c0xc1d0000c/0x5c000007) … GrCompute …}` —
**eight channels, `pdb=Y`, for the exact `(hClient, hVASpace)` pair line 234 of the same boot
says has no published root.** ⇒ Our object model already holds that root; `CeResolve` reads a
different, weaker projection keyed by **raw handle**. The refusal's sentence — *"the guest
published no page-directory root"* — is **false about the guest**. That is the next rung, and
it is wiring rather than design.

## §16.64 — Wall 2: the root we already held, and the aperture we were dropping

### 16.64.1 What I REFUTED before writing a line

★★★ The brief for this rung was right about the wall and wrong in three places I had to
measure to find. Each is recorded because each would have shipped.

**R1 — `gpu_vaspace.c:523` does NOT say what it was cited for.** The brief cited it for
*"an externally-owned VAS publishes through `0x00801813 SET_PAGE_DIRECTORY` instead"*. Line
523-524 is a **comment inside the `VASPACE_FLAGS_ENABLE_ATS` arm** saying PASID is programmed
via that control. It is not a publication path and not about externally-owned VA spaces. ⊘
The *conclusion* holds, on a citation nobody had produced: `nvGpuOpsSetPageDirectory`
(`ogkm-580: nv_gpu_ops.c:8778-8871`) builds the params and issues
`NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY` at `:8870`, reached from UVM at `uvm_gpu.c:1305` and
`uvm_va_space.c:1394`.

★ And the flag chain the brief asserted without one is real, in three hops, all verified
here: `NV_VASPACE_ALLOCATION_FLAGS_IS_EXTERNALLY_OWNED` sets **`VASPACE_FLAGS_DISABLE_SPLIT_VAS`**
(`vaspace_api.c:617-621`) → that flag is exactly what excludes the call to
`gvaspaceReserveSplitVaSpace` (`gpu_vaspace.c:598-612`) → which is the **only** writer of
`vaStartServerRMOwned` (`:394-421`) → so it stays `0` and
`gvaspaceCopyServerRmReservedPdesToServerRm_IMPL` returns `NV_OK` having published nothing
(`:4046-4051`).

**R2 — "a raw-handle key with no alias resolution" is the wrong diagnosis.** Alias
resolution *does* run: `ChannelFacts::vas_origin` is documented and implemented as the VASpace
**resource** with dup-aliases resolved, and `CeChannelFacts::vas_pdb` comes off it via
`RmGraph::pdb_of_resource`. The actual fault is sharper and worse for any handle-keyed fix:
the publication is filed under UVM's **dup** (`hClient 0xc1d0000a hVASpace 0xcaf00036`) while
the channel resolves to the **origin** (`c0xc1d0000c/0x5c000007`). ⇒ **Both sides resolve
correctly, to different handles of one resource.** No handle-keyed table can join them, and
"normalize the key" — the obvious fix — would have failed whichever handle it normalized to.

**R3 — the brief's s45 line numbers are for a file it did not name.** Lines 169/234 are in
`run_s45_748a207_tsgsched_qemu.log`, not the `_probe.log`. Both lines are verbatim correct
there. ⊘ Recorded because two of the three logs in that boot have >169 lines, so the citation
silently resolves to the wrong file and reads as a refutation.

★★ **Confirmed unchanged** `[measured 2026-08-10, boot `s45_748a207_tsgsched`, re-read here
from this repo's own committed `traces/guest_boots/run_s45_748a207_tsgsched_{qemu,probe}.log`
rather than inherited from the brief]`: doorbells `448 arrived, 261 served, 187 REFUSED`;
records 73/74 are the only two `hClass=0x90f1` allocs in the boot, same `hRoot=0xc1d0000c`,
same `hParent=0x5c000002`, `hObject` `0x5c000007` / `0x5c000008`; and the aperture trap is
real — `_APERTURE_VIDMEM == 0` (`ctrl0080dma.h:842-845`) against `GMMU_APERTURE_INVALID == 0`
/ `GMMU_APERTURE_VIDEO == 1` (`gmmu_fmt.h:277-285`).

### 16.64.2 The defect underneath the wall — and it was NOT the one I was sent for

★★★★ Chasing the transport turned up something the brief did not name and that outranks it.
`RmEvent::SetPageDir` is minted at **two** sites. `translate_published_pdes` (`0x90f10106`)
forks on the aperture and refuses a non-framebuffer root — its refusal's own doc calls this
*"that refusal at the point the fact is born instead of at the point it is used."*
`translate_control`'s `0x00801813` arm — **UVM's** transport, the one this whole rung is
about — built `Pdb(p.phys_address)` with **no aperture fork at all**.

So a `_SYSMEM_COH` root became a `Pdb`, which `kayfabe_arch::ids::Pdb` documents as *"a
per-GPU FB address"*. Silent, and with the sibling's refusal twenty lines away.

★★★★ **A comment that names an exception is a bug report, and this one named its own expiry
and still shipped.** `translate_control`'s rustdoc said the drop *"is safe exactly as long as
`Pdb` is only ever a key … The day a walker follows a PDB it must know whether the address is
a framebuffer offset or a guest-physical address."* **This rung is that day** — its entire
purpose is to let the CE resolver walk that root. The paragraph was correct, precise, and had
no reader. ⊘ And it was never true globally anyway: it described a property only one of the
two arms had.

★★★★★ **A GREEN TEST PINNED THE DEFECT AS A PROPERTY.** `rmrpc_bridge.rs` carried
`the_aperture_is_dropped_so_a_vidmem_and_a_sysmem_root_are_the_same_event`, asserting all four
`flags[1:0]` encodings produce one identical event. Its doc even wrote the expiry down —
*"The day a walker follows a PDB, this test is the one that has to change, and it will say so
by failing."* It did exactly that, which is the good half. The bad half is the mechanism: a
**passing** test is read by nobody, so a correctly-written expiry condition is only ever
delivered to someone who has already broken it. This is the same shape as
`gvas_publication.rs:436-438`'s wrong quantifier — a green assertion holding a wall in place.

★★★★ **And its fixture encoded a MISREAD of a correct citation.** `HEX_SPD_FLAGS` was
`SYSMEM_COH`, justified as *"the shape UVM actually sends (`nv_gpu_ops.c:8857-8862`)"*. That
citation is real and correctly located, and it is a **ternary**: UVM sends `_VIDMEM` *or*
`_SYSMEM_COH` depending on `bVidMemAperture`. Reading one arm of a conditional as a constant
produced a fixture our own boot contradicts — `[measured 2026-08-10, boot
`s45_748a207_tsgsched`]` the live control is `flags 0x8 (aperture 0)`, **VIDMEM**. ⇒ The
transferable rule: *a citation to a conditional must carry the condition.* Checking that a
claim is **sourced** never checks that the source says it — the same failure class as the C
oracle's empty `dlen=0` rows satisfying a `C:` citation gate.

### 16.64.3 What landed

1. **The fork, at the second birth site.** `BridgeRefusal::SetPageDirRootAperture` carries the
   **decoded** `PdbAperture`, never a bare word, so the two encodings can never be matched
   against the wrong table. ⇒ `Pdb`'s documented meaning is now true **by construction on
   every path that can mint one**, which is what makes step 2 sound rather than convenient.
2. **`ceresolve::root_from_declared_pdb`** — a walkable root from a base the object model
   resolved **by resource identity**. `page_shift` is derived from `GmmuFmt::level_shift(0)`
   (the control has no such field); `aperture_raw` is stamped `GMMU_APERTURE_VIDEO` — the
   value meaning vidmem *in that field's own encoding* — and the control's `0` is **not**
   copied across.
3. **A second root source at the doorbell**, tried only after the publication table misses, so
   nothing that works today is routed differently. Its own failure reports by its own name
   (`CeResolve::DeclaredRootUnusable`), never as `NoPublication`.
4. **The `NoPublication` sentence narrowed** to what it can still truthfully claim: neither
   the device's table nor the object model knows of a root.

### 16.64.4 ⊘ THE FALSIFIER — committed BEFORE the boot

⊘ The instruction I am holding myself to: **the falsifier must not be the thing the fix
produces.** "`published_root` now resolves" is unusable — it is the change restated. Every
value below is read off the **guest's** subsequent behaviour.

- **A — the route landed.** `448/261/187` moves to `448/>261/<187` **and** the
  `CeResolve::NoPublication` count for `(0xc1d0000c, 0x5c000007)` goes to 0.
- **B — the wall moves one hop deeper (EXPECTED).** Doorbells still refuse, but the refusal
  **changes name** — `Fault(Unmapped)`, `Fault(Sparse)`, `AddressOutOfRange`, or a CeUtils
  refusal. Necessity is not sufficiency; a root that resolves is not a ring that decodes.
  ★ B is a **confirmation**, and I am predicting it rather than A.
- **C — refuted at the doorbell surface.** Still `NoPublication` at 187. Then the question is
  whether `facts.vas_pdb` was `None` for those channels — i.e. the graph never held the root
  for the *channels that ring*, only for the ones the promote census printed.
- **D — ⚠ the NEW regression arm, and it exists because step 1 can take something away.**
  `pdb=Y ×8` becomes `pdb=N`, or the four CeUtils `SERVED-LOCAL` lines on token `0x00010002`
  disappear. That would mean a `SET_PAGE_DIRECTORY` this boot *needs* is being refused by the
  new aperture fork. The measured aperture is `0` (VIDMEM) so this should not fire — which is
  exactly why it must be checked rather than assumed.

⊘ **Positive control that must not regress:** `pdb=Y ×8` on `cs=ok(h0x5c000007…)`, and the
four CeUtils `SERVED-LOCAL` lines on token `0x00010002`.

⚠ Counting discipline for the read-back: the probe log **reprints its last 80 records** — the
record count is ~456, not ~538. And the rev must be read off the **hypervisor**, not from
`build_qom_shim.sh`'s exit code (`CARGO_TARGET_DIR` redirects cargo away from its hard-coded
`$REPO/target/release` and it copies a stale archive at `rc=0`).

### 16.64.5 ★★★ THE OUTCOME — read against §16.64.4, which was committed before the boot

Two boots, both at revisions **read off the hypervisor** (`strings … | grep kayfabe-rev`), not
off a build script's exit code:

| | `s45_748a207_tsgsched` (before) | `s49_57bd756_declroot2` | `s50_9a446e9_probefix` |
|---|---|---|---|
| doorbells | **448 / 261 / 187** | **448 / 354 / 94** | 448 / 354 / 94 |
| first refusal | `[CeResolve::NoPublication]` | `[FwdFault::SubmissionHasNoLaunch]` | same |
| `CeResolve::NoPublication` | the wall | **0** | **0** |
| `rng=` / `fin=` | `NOPUB` / `NOPUB` | (probe stale — see §16.64b) | `V:0x1024000` / `V:0x102c004` |
| ring scan | *all 1024 failed to resolve* | — | `1024/1024 … unread=0` |
| walk | `walk=NO-PUBLICATION` | — | **four levels to a `LEAF`** |

⇒ **Falsifier arms A and B BOTH, which is a stronger result than either alone.**

- **A landed.** `NoPublication` went to **0** and 93 doorbells moved from *refused* to
  **served** (`261 → 354`, `187 → 94`).
- **B landed too.** The residue refuses under a **different name**,
  `FwdFault::SubmissionHasNoLaunch { methods: 3, opaque: 2, set_object: ClassId(0xc7b5) }` —
  a refusal that is **true**, replacing one this rung established was **false about the guest**.
- **C refuted.** Not still `NoPublication`.
- **D did not fire.** `SET_PAGE_DIRECTORY (0x00801813): 2 ACCEPTED, 0 refused` — the new
  aperture fork refused nothing on the path `cup2` walks, exactly as the measured `flags 0x8`
  predicted. Positive control intact: `pdb=Y ×8` on `cs=ok(h0x5c000007…)` unchanged, and
  `SERVED-LOCAL` is **16 in both** boots. `DeclaredRootUnusable`: **0** — the derivation never
  failed.

★★★★ **What the walk proves, and it is not the same claim as "the count moved."** `s50`'s probe
descends from the declared root through every level to a real leaf:

```text
root=0x201000/ap1/sh47 rootsrc=declared(object-model)
L0@0x201000=PDE@0x0->0x202000/Vidmem   L1@0x202000=PDE@0x0->0x203000/Vidmem
L2@0x203000=PDE@0x200000000->0x204000/Vidmem
L3@0x204000[ch29 lf7]=LEAF@0x200200000->0x1000000/Vidmem/sz0x200000
pbm[8w of 32B]: [0]sub4/m0x0/…=0xc7b5 [1]sub4/m0x240/…n3 [2]sub4/m0x300/…=0x14
```

⊘ Four levels of the **guest's own** page tables decoding to well-formed PDEs and a 2 MiB leaf,
then real pushbuffer methods behind it, is not something a wrong root produces — a wrong root
lands on `Invalid` or on bytes that do not decode. The `page_shift 47` printed there was
**derived from the installed format**, never a literal, and it agrees with what a real GA106
publishes.

⊘ **`row=ABSENT-FROM-ROOT-TABLE(11 rows)` is still printed and is still CORRECT** — the
publication table genuinely has no row for this pair, which is the whole finding. It now sits
beside `rootsrc=declared(object-model)`, so it reads as *"the other source answered"* instead
of as a contradiction.

### 16.64.6 ⊘ What this rung did NOT establish, and the coordinator's prediction it refuted

★★★ **A parallel read predicted the count would stay at ~187 with a new tag** — the reasoning
being that the 187 are `GrCompute` channels claimed by the CE executor (true, and its
mechanism is confirmed: see the corrected comment in §16.64b), whose rings decode to `Opaque`
and would simply refuse for a different reason. ⊘ **Measurement refutes the prediction while
confirming the mechanism**: a rename preserves the count, and **93 doorbells moved into
`served`**. So a majority of that population was real CE work blocked behind a false refusal;
only the remaining 94 are the engine-partition residue that story describes.

⚠ **The per-`EngineKind` doorbell histogram was asked for and is NOT in this rung.** It is the
right instrument and the reason is not that it is unnecessary — it is that the doorbell census
is formatted **in the C shim** (`qemu/hw/misc/nvkvm/nvkvm.c:2360`) from a fixed ABI struct, so
a histogram is a cross-ABI change, and adding one at the end of a rung whose falsifier had
already returned is how an instrument ships unvalidated. ⊘ It is also not load-bearing for
*this* verdict: the count **moved by 93**, which already discriminates "the fix helped" from
"the fix renamed the refusal" — the exact ambiguity it was proposed to resolve. It is the
natural first step of `w202`, where the partition it measures is the subject.

⊘ **`cup3` is not reached and was never in scope.** GR compute remains structurally
unreachable in every shipping build; this rung moves `cup2`'s CE path and says nothing about
the completion plane, which has no C oracle at all.

## §16.65 ★★★★★ BOOTED `s51_d502ac6_engroute` (`w202`) — **AN OUTCOME THE FALSIFIER HAD NO CELL FOR**, because its own evidence was misattributed

`[measured 2026-08-10, boot `s51_d502ac6_engroute`, both artifacts stamped
`kayfabe-rev:d502ac658b7fa11c02190de74d587a869aa03c91`]`

### 16.65.1 The change

`CeChannelFacts` gained `engine: EngineKind`, copied off the **same** resolved `Channel` that
already yields `vas_pdb`; `route_of_engine` turns it into a `DoorbellRoute`
(`CpuCe` / `HostGr` / `Unserved`); `try_ce_submission` refuses a non-`CpuCe` route by name
(`Route::NotACopyEngineChannel`) **before** it reads a byte of a ring. ⊘
`local_ce_is_the_only_executor` and `KAYFABE_ISOLATES` untouched; the plane stayed `Stillborn`.

The instrument shipped with it and is the half that mattered: ABI 35 → 36, `KayfabeRegAudit`
gaining `doorbells_by_engine[6]` + `doorbells_engine_unrouted` and the served split
`_locally` / `_forwarded`. §16.64.6 deferred exactly this and named `w202` as its home.

### 16.65.2 The measurement, `[measured 2026-08-10, boot `s51_d502ac6_engroute`]`

```
doorbells: 448 arrived, 354 served, 94 REFUSED by name
  of the served: 354 local (CPU CE, end witnessed), 0 forwarded (host channel rung)
  by engine: GrCompute=86 GrGraphics=0 Ce=362 NvEnc=0 NvDec=0 Other=0 unrouted=0
```

Every other number is **bit-identical to `s49`/`s50`**: 16 `SERVED-LOCAL` lines (4 on token
`0x00010002`), `pdb=Y` ×4, `CUP2_RC=1`, `census[14 chans, 4 outcomes]`. ⊘ **No regression** —
falsifier D did not fire, and the §15 regression the design claimed to structurally exclude
did not appear.

★ `86 + 362 = 448` and `362 − 354 = 8`, so the partition closes exactly: **86 `GrCompute`
doorbells, every one of them now refused by the routing fact** (a code certainty — the gate
returns `Some(refused(..))` for every non-`CpuCe` route, unconditionally, before any other
check), plus **8 refused `Ce` doorbells**, totalling the 94. The routing defect §16.65 was
built for is **real and is 86 doorbells wide**, and nothing but this histogram could say so:
before the change those 86 were *already* refused, under a name that described bytes.

### 16.65.3 ★★★★★ THE FALSIFIER HAD NO CELL FOR THIS, and that is the finding

⊘ **Scored honestly first, against the three values committed before the boot.**

| clause of **A** (the predicted landing) | on `s51_d502ac6_engroute` |
|---|---|
| `448/354/94` unchanged in count | ✔ |
| `CUP2_RC=1`, `pdb=Y`, `SERVED-LOCAL` on `0x00010002` unchanged | ✔ |
| the 94's tag becomes `Route::NotACopyEngineChannel` | **partly** — 86 of 94 |
| `SubmissionHasNoLaunch` → 0 | ✘ **it is still 1, and it is still FIRST** |

**B** required *"the refused count moves **off** 94"*. It did not. **C** required a refusal
from an `EngineKind::Ce` channel — one *did* occur, but not for C's stated reason (the engine
refinement reached UVM's channels correctly; `Ce=362` is the proof). **D** required `served`
to drop; it did not.

⇒ The boot landed in **no cell**. A three-valued falsifier is only as good as the premise its
cells are cut from, and this one was cut from *"the first refusal is a misrouted GR
pushbuffer"* — which is false. ★ Same family as `falsifier_blocker_vs_only_blocker`: the
values were fine and the **partition** was wrong, so `[measured 2026-08-10, boot
`s51_d502ac6_engroute`]` had nowhere to land. ⊘ The instrument that rescued the rung was the histogram, which was not a falsifier
value at all.

### 16.65.3b The refutation is of my own brief's EVIDENCE, not its fix

The brief's motivating sentence was: *"the `methods: 3, opaque: 2` above is a GR pushbuffer
being decoded by the CE codec and correctly declining to find a CE launch."*

⊘ **Measured false**, `[measured 2026-08-10, boots `s49_57bd756_declroot2`, `s50_9a446e9_probefix`, `s51_d502ac6_engroute`]`. The first refusal is **unchanged** across all three —
`FwdFault::SubmissionHasNoLaunch` — and its own printed pushbuffer is:

```
[0] sub4/m0x0   /Incrementing/n1 = 0xc7b5     ← SET_OBJECT = AMPERE_DMA_COPY_B (the COPY ENGINE)
[1] sub4/m0x240 /Incrementing/n3 = 0x2        ← SET_SEMAPHORE_A/B/PAYLOAD
[2] sub4/m0x300 /Incrementing/n1 = 0x14       ← LAUNCH_DMA
```

That is a **CE** pushbuffer, on a **CE**-labelled channel, routed to the **CE** executor. It
is exactly where it belongs, and `w202` could not move it and did not. ⇒ The rung's fix is
right and its cited evidence was a different doorbell's.

★ **This is why the histogram had to be the rung's first step.** With only `448/354/94` and an unchanged first-refusal tag, `s51` reads as *"nothing
happened"*. The partition says 86 doorbells changed executor-verdict while the count held —
the same shape §16.64.6 recorded as a prediction refuted by a rename that preserved a count,
now with the instrument that separates the two.

### 16.65.4 ★★★★★ THE ACTUAL WALL AFTER `w202`, named from our own tables

`LAUNCH_DMA` flags `0x14` decode against `kayfabe_abi::submit::ce` as
`LAUNCH_SEMAPHORE_TYPE_RELEASE_FOUR_WORD (2<<3 = 0x10) | LAUNCH_FLUSH_ENABLE (1<<2 = 0x4)`,
with `DATA_TRANSFER_TYPE` (field `1:0`) = **0 = NONE**.

⇒ a **zero-byte, flush-enabled, four-word (timestamped) semaphore release** — the
`finishPayload` the guest then waits on. And `Ga10xPushbuffer`'s own doc already names the
refusal, correctly, at the codec:

> **`DATA_TRANSFER_TYPE == NONE`.** The engine moves no bytes; the launch exists to release a
> semaphore. There is no copy to report.

So the codec returns `None` → `PushMethod::Opaque` → the submission carries no `CeLaunchDma` →
`FwdFault::SubmissionHasNoLaunch`.

⊘ **The fault's NAME is false and that is what hid this.** The submission **does** have a
`LAUNCH_DMA`; it has no **copy**. *"Submission has no launch"* sends a reader to look for a
missing method, and the method is right there in the line the fault itself prints. ★ Same
family as §16.64b: a projection that is true of the thing it measured and wrong about the
question being asked.

⚠ And `LAUNCH_SEMAPHORE_TYPE_RELEASE_FOUR_WORD`'s own constant records the cost:
*"Sixteen bytes: the payload plus a **hardware timestamp** this port has no source for."* The
next rung is that release, not a routing change.

### 16.65.5 ★★★★ A CHANNEL'S `EngineKind` IS A FACT WITH A LIFETIME — found by a test, not by the bench

`Ga10xArch::classify` labels **every** `AMPERE_CHANNEL_GPFIFO_A` `GrCompute` (there is one
GPFIFO class per architecture; the engine type is an `NV_CHANNEL_ALLOC_PARAMS` field
`RmEvent::Alloc` has nowhere to carry), and a channel becomes `Ce` only when its
`AMPERE_DMA_COPY_B` engine object lands and `project.rs`'s refinement pass rewrites it.

⇒ **a channel is `GrCompute` from its alloc until its engine object arrives.** The `s48`
census that corroborated this rung's design is an **end-of-boot snapshot**: it says what the
14 channels *are*, and cannot say what they *were* when their first doorbell rang. A CE
channel ringing before its engine object would be routed away by this gate.

★ It did not happen on `s51` — `unrouted=0`, `GrGraphics=0`, and every count held — so the
hazard is **latent, not live**. It was found because `e2_doorbell.rs`'s fixture declares a
channel and **no** engine object, so the test tree reproduced it before the bench could. The
fixture now declares the engine object, and says in place why.

⊘ Related and **not** dissolved: `project.rs`'s refinement is `or_insert` over ascending
origin-key order, i.e. **first-wins**, justified by a comment that says *"the real protocol
allocates one engine object per channel context"*. That comment is unverified against a boot.

### 16.65.6 What this rung did NOT establish

⊘ `try_ce_submission` had **zero** test coverage of any kind before `w202`; it now has the
pure decision (`route_of_engine`, quantified over the whole enum) and one end-to-end pair
differing by a single event. That is not coverage of the executor.

⊘ **`cup3` is not reached.** `CUP2_RC=1` is unchanged, `0 forwarded` is unchanged, and the
completion plane still has no C oracle. GR now refuses by a name that is true; it still needs
a host channel that **shadows** the guest's and the `OS_DESCRIPTOR` primitive, neither built.

## §16.66 ★★★★★ `w203` — THE ZERO-BYTE SEMAPHORE RELEASE, and four claims of the brief refuted before a line was written

### 16.66.1 ⊘ WHAT I REFUTED FIRST, because three of the four would have aimed the rung wrong

**(1) ⊘ The refusal is NOT `DATA_TRANSFER_TYPE == NONE`, and the doc that says so is STALE.**
The `w203` brief quoted `Ga10xPushbuffer`'s own text — *"`DATA_TRANSFER_TYPE == NONE`. The
engine moves no bytes; the launch exists to release a semaphore. There is no copy to
report."* — as the live explanation of `s51`'s first refusal. That arm **stopped returning
`None` at `b731e3c` (§15.7)**, four commits before `s51` booted; it returns
`PushMethod::CeRelease`, and `ceutils` executes it. The sentence survived as a **doc-comment
bullet on `ce_launch`** and answered the question anyway. ★ A stale doc does not go quiet, it
**answers**, and it is the answer a reader trusts most because the file is the source.

The single reachable refusal for flags `0x14` is `ce_completion`'s `_ => None`, reached via
`ce_launch`'s `Self::ce_completion(state, subch, flags)??`. That is a **code certainty**, not
a reading: the four-word arm did not exist, and no other arm can return `None` for a launch
whose `SET_SEMAPHORE_*` registers are all latched.

**(2) ⊘ The emitter is NOT UVM and NOT kernel RM — and the pushbuffer says so in one field.**
`uvm_hal_maxwell_ce_init` binds the CE class on **subchannel 0**, and says why in its own
comment (*"Notably this sends SET_OBJECT with the CE class on subchannel 0 instead of the
recommended by HW subchannel 4"*, `ogkm-580: uvm_maxwell_ce.c:29-38`). Kernel RM's
`RM_SUBCHANNEL` is **`0x0`** (`ogkm-580: inc/kernel/gpu/mem_mgr/channel_utils.h:61`). The
refused push binds on **subchannel 4** (`[0]sub4/m0x0/…=0xc7b5`). ⇒ neither. Two further
confirmations: UVM's Ampere HAL ORs `plc_mode` = `DISABLE_PLC` (bit 26) into **every**
`LAUNCH_DMA` (`uvm_hal.c:143-149` → `uvm_ampere_ce.c:113-116`), and `0x14` has bit 26 clear;
and RM's only four-word emitter is none — `channel_utils.c:645,832` push
`_RELEASE_ONE_WORD_SEMAPHORE` and nothing else. The guest is the **open** 580.159.04 module
(`run_s51…_dmesg.log:4`), so this is the whole kernel-side universe. ⇒ the emitter is
**userspace `libcuda`**, and `cup2` dies at `cuCtxCreate` (801) with `cuInit` green
(`run_s51…_probe.log:64,75`), which is where it would be.
⚠ **Consequence for this rung's crux**: *"what does the guest do with the timestamp?"* cannot
be answered from `ogkm`, because the reader is closed source. It is answered structurally
below instead, and the difference is stated rather than smoothed over.

**(3) ⊘ `SEMAPHORE_PAYLOAD_SIZE` is a SEPARATE FIELD, and it dissolves the "four words = 8-byte
payload" problem the brief inherited.** `NVC7B5_LAUNCH_DMA_SEMAPHORE_PAYLOAD_SIZE` is bit
**`27:27`** (`clc7b5.h:157-159`); `SEMAPHORE_TYPE` is `4:3`. `RELEASE_FOUR_WORD` names the
**sixteen-byte structure**, `PAYLOAD_SIZE` names how much of it is payload. `0x14` leaves bit
27 clear ⇒ a **one-word (32-bit) payload** inside a four-word record, and
`SET_SEMAPHORE_PAYLOAD_UPPER` (`0x24C`) is **not consulted**. ★ `CeCompletion::payload`'s own
doc had said the upper half *"can never be silently needed"* **because** the four-word release
*"is refused rather than decoded"* — a comment naming an exception, whose exception this rung
makes false. It is now a decoded field with its own refusal when unlatched, not a sentence.

**(4) ⊘ The brief's owed item is half right.** `SetPageDirPolicy` (no aperture fork, answers
`NV_OK`) and `kayfabe_rmrpc::translate_control` (refuses
`BridgeRefusal::SetPageDirRootAperture` by name) **do** disagree on a non-vidmem root — that
part holds. But *"`mean_wire.rs:1448` carries a stale comment your own boot contradicts"* does
not: those bytes are a **hand-built decoder fixture**, and the boot neither corroborates nor
contradicts them. What the boot actually says is stronger and different — `SET_PAGE_DIRECTORY
… flags 0x8 (aperture 0)`, i.e. **every root this port has ever seen is vidmem**, so the
sysmem arm of *both* layers is unexercised by every boot in `traces/guest_boots/` and the
disagreement cannot fire. The comment is corrected to say that, which is a smaller claim than the brief's and the
true one. ⚠ Naming a fixture after a guest behaviour is how a fixture becomes a citation.

### 16.66.2 The change

- `ce_completion` **decodes** `RELEASE_FOUR_WORD` into `CeCompletion { structure:
  CeSemStructure::FourWord, payload_bytes }`, gating the payload width on bit 27 and
  requiring `_PAYLOAD_UPPER` to be latched when it is set. `RELEASE_CONDITIONAL_INTR` stays
  refused, and its refusal is now narrow enough to be true.
- `ResolvedRelease` resolves **every four-byte word of the record separately**, so a
  sixteen-byte release that straddles a page refuses instead of writing into whatever page
  follows. `write_resolved_completion` checks every word's backing **before** writing any.
- **The timestamp source is `CePlane::now_ns` — `RegPlane`'s own `NanoClock`, the same
  counter `ptimer_read` answers the guest's `NV_PTIMER_TIME_0/1` from.** ⊘ Not
  `Instant::now`, not a counter of this executor's own: being the *same* clock is the whole
  property, so it is taken through the same object rather than passed by a caller who could
  pass another. A guest that stamps, reads `PTIMER`, and subtracts gets a consistent answer.
  ⚠ **Stated, not hidden**: that counter is a synthetic CPU-side clock (`NanoClock`'s own docs
  call it a boot-only stopgap), so it is in a different timebase from any *real host GPU*
  timestamp. That matters the moment compute is actually forwarded and it is `#128`'s
  read-native memslot, not this rung's.
- ⊘ **A four-word release with no clock is `FwdFault::CeReleaseNoClock`, not zeros.** `0` is a
  legal `PTIMER` reading; `[measured 2026-08-10, boot `s51_d502ac6_engroute`]` that semaphore
  page reads `fbFIN@0x102c004=0000…0000 nz0/4096 resN-NEVER-WRITTEN`, so a zeroed timestamp is
  **byte-identical to never having run**. Writing zeros is the oracle's fifth limit rebuilt
  one plane over. The third option the brief listed — fabricate — is refused by name.
- `FwdFault::SubmissionHasNoLaunch` → **`SubmissionDecodedNoWork`**. The old name was
  **false**: the submission *had* a `LAUNCH_DMA` (`[2]sub4/m0x300/…=0x14`); it had no *copy*.
- Four in-code comments carrying §16.65's refuted *"a GR pushbuffer decoded by the CE codec"*
  attribution are corrected in place (`shim.rs`, `device.rs`, `e2_doorbell.rs`,
  `engine_context.rs`). The correction was committed to this doc at `1f4adaa` and **not** to
  the code, where four readers would have met it first.
- The **C oracle** (`tests/oracle/pushbuffer_abi_oracle.c`, compiled against NVIDIA's own
  headers) gains `sem_fourword`, `sem_fourword_wide` and `refusesem_wide_nolatch`, and now
  emits `paysize`/`payupper` from NVIDIA's own `DRF_VAL`. ★ So the two-fields-eight-bits-apart
  claim is checked against the header, not against my reading of it. Both gates print `RAN`.

### 16.66.3 ★★★ THE FALSIFIER, committed BEFORE the boot

Baseline is `s51_d502ac6_engroute`: `448 arrived / 354 served / 94 REFUSED`, `GrCompute=86
Ce=362 unrouted=0`, first refusal `FwdFault::SubmissionHasNoLaunch`, `CUP2_RC=1` with
`cuCtxCreate → operation not supported (801)`, `sem fin va=0x12006c004 -> S:0x3089c004`.

⊘ **The falsifier is about what the GUEST does, never about what my fix produces.** A
counter of four-word releases would go up by construction; none of these arms reads one.

| arm | the line that distinguishes it |
|---|---|
| **A — the release is served and the guest moves** | `REFUSED` **< 94** *and* `served` **> 354** (they must move together and by the same amount), `by engine` still `GrCompute=86 Ce=362`, first-refusal tag now `Route::NotACopyEngineChannel`, **and** `CUP2_RC` changes or `cuCtxCreate`'s error changes off `801` |
| **B — the release is served and the guest does NOT move** | the same counter movement and the same tag change, but `CUP2_RC=1` with `cuCtxCreate → … (801)` **unchanged**. ⇒ the wall was real and was not the last one |
| **C — the release decodes and then refuses somewhere new** | `REFUSED` still `94` (or moves by less than the `Ce` deficit) **and** the first-refusal tag is one of `FwdFault::Address`, `FwdFault::CeUnstableBacking`, `FwdFault::CeReleaseNoClock`. ★ This is the `#12` cell: the release resolved nowhere, or somewhere the guest does not read |
| **D — regression** | `served` **< 354**, or `by engine` off `86`/`362`, or `unrouted > 0`, or `arrived ≠ 448` |
| **E — NONE OF THE ABOVE** | ⊘ **Reserved, and it will be used rather than rounded to the nearest arm.** §16.65's falsifier had four arms cut from a false premise and the boot landed in none of them; the correct response was to add the cell, not to score it as `B`. If the numbers land outside every row above, this row is the answer and the reason goes here |

★ **And one thing that is NOT an arm but must be printed either way** — the `#12` check the
brief demanded: the **aperture** the four-word release resolves to. `completion_at` carries
`(va, plane, plane_addr)` and the report prints it as `sem fin va=… -> S:…` / `V:…`. A release
written to `V:` while the guest polls a sysmem `pbCpuVA` is `#12` rebuilt, and it would be
invisible in every count in the table above.

### 16.66.4 ★★★★★ THE OUTCOME — **B**, scored against §16.66.3 as committed, and confirmed twice

`[measured 2026-08-10, boots `s53_af255fa_fourword2` and `s54_af255fa_wallrepeat`, both
artifacts stamped `kayfabe-rev:af255fa089259c5dcb367296b7f3a4d46cb481af` read off the
hypervisor binary]`

★ **`s54` is the exact comparison** — it arrived at the *same* 448 doorbells as `s51`, so
every number is directly subtractable. (`arrived` is not constant across boots: `s53` saw
450. A comparison that does not pin it is comparing two different guests.)

| | `s51` (`d502ac6`) | `s54` (`af255fa`) | `s53` (`af255fa`) |
|---|---|---|---|
| arrived | 448 | **448** | 450 |
| served | 354 | **362** | 364 |
| REFUSED | 94 | **86** | 86 |
| by engine | `GrCompute=86 Ce=362` | **`GrCompute=86 Ce=362`** | `GrCompute=86 Ce=364` |
| first refusal | `SubmissionHasNoLaunch` | **`Route::NotACopyEngineChannel`** | same |
| `CUP2_RC` | 1, `cuCtxCreate → 801` | **1, `cuCtxCreate → 801`** | same |

**`Ce=362` and `served=362`: every copy-engine doorbell the guest rang is now served.** The
94 was `86 GrCompute + 8 Ce`; the 8 were the four-word releases, and all 8 moved. Nothing
else moved at all — `GrCompute=86` is bit-identical, `unrouted=0`, `0 forwarded`.

⇒ **Row B**: *"the release is served and the guest does NOT move."* The wall was real, the
fix is right, and it was **not the last wall**. `cuCtxCreate` returns `801` for a reason this
rung did not touch and did not claim to.

### 16.66.5 ⊘ WHAT THE BOOT DID **NOT** ESTABLISH — and the instrument gap it found

⊘ **The `#12` aperture question is UNANSWERED for the four-word release, and §16.66.3
promised to print it either way, so here is the absence.** The only per-serving aperture line
the report carries is `last CPU-CE serving: … 1 sem fin va=… -> S:…`, and on all three boots
the *last* serving was a 65 536-byte copy (`0 release-only`). A **last** cannot answer a
question about a **class** of events — the same shape as §16.65's *"the counts alone read as
nothing happened"*, and the same fix: a per-kind census, not a last-one exemplar.

What IS guaranteed is structural rather than witnessed, and the difference matters: the
executor resolves the guest's own semaphore VA **through the guest's own page tables** and
refuses the whole record on any miss (`FwdFault::Address`), so it cannot have written to a
scratch framebuffer page the guest never reads — which is exactly `#12`'s failure. But *which
plane* those eight releases landed in is not in the log, and I am not going to infer it from
the `S:` of a different serving.
★ **Owed**: split `doorbell_local_serving` into a cumulative per-kind census (copies,
one-word releases, four-word releases, each with the plane) — an ABI bump this rung
deliberately did not take, because the counters it already had were enough to *score the
falsifier* and adding one would have meant shipping an instrument in the same boot as the
change it measures.

⊘ **And the timestamp is written but UNREAD-BY-ANYONE-WE-CAN-SEE.** §16.66.1(2) established
the emitter is `libcuda`, which is closed. So *"the guest gets a consistent answer if it
correlates against `PTIMER`"* is an argument from the clock being the same object, **not** a
measurement of a guest reading the field. The eight doorbells moving from refused to served
is the measurement; the timestamp's *value* has no oracle here.

### 16.66.6 ★★★★ TWO INSTRUMENT FAILURES, one of them mine

`[measured 2026-08-10, boots `s52_af255fa_fourword` and `s54_af255fa_wallrepeat`]`

**(1) ⊘ I CHANGED THE INSTRUMENT BETWEEN THE BASELINE AND THE MEASUREMENT.** The first boot
at `af255fa` (`s52_af255fa_fourword`) ran `POST_CAPTURE_HOOK=guest_cuinit_wall.sh`, while the
`s51` baseline it was being compared against had run `cup2_hook_rmtrace.sh`. `s52` reported
`cuInit(0) → unknown error (999)` and `2 arrived, 2 served`, and for several minutes I read
that as **falsifier row D, a regression**. It was not a comparison at all: the two boots
differ in the code *and* in the thing doing the looking. ★ The guest's `dmesg` was
**byte-identical to `s51`** across that whole scare, which is what said the driver path was
untouched and the difference was above it.

**(2) ⊘ `s52` IS STILL UNEXPLAINED, AND IS RECORDED AS OPEN RATHER THAN RESOLVED.** The
obvious story — *"that hook is not observationally neutral"* — is **not supported**:
`s35_03a7e10_dup` ran the same hook and got `ok cuInit(0)`, and so did `s54`, at this very
revision, with this very hook. So `s52` is a **flake**, and by
`deterministic_failure_indicts_the_test` a flake indicts the *system*, not the test. Its
signature is on disk (`run_s52_af255fa_fourword_probe.log`): `cuInit` gives up after **19** RM
calls at `CTRL cmd=0xcb330101 hClient=0xc1d0000c hObject=0x5c000001 status=0x0000002f`,
against **456** trace lines on a good run. ⚠ One boot each way is not a rate; do not quote it
as one.

★ Both `s52` and `s54` are carried into `traces/guest_boots/` **including the failure**. A
boot that refuted my own reading is evidence, and dropping it would leave the tree saying this
rung went straight from a hypothesis to a green.

---

## §16.67 ★★★★★ `R25` — OS_DESCRIPTOR is a **PORT**, measured at both privilege levels, with a negative control

`[measured 2026-08-10, RTX 3060 GA106 / 580.159.04, binary stamped REV_UNDER_TEST=40d44db84,`
`traces/real_ga106/rmladder_r25_osdescriptor_real_ga106.txt]`

### 16.67.1 ⊘ WHAT I REFUTED FIRST — four citations of the brief, and its central premise

**(1) ⊘ `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` is NOT "in the allowlist with no alloc path".**
The brief cited `crates/kayfabe-abi/src/capability.rs:1435` for that. Line 1435 is inside
**`DENIED_CLASSES`**. The class is refused **by name**, with
`DeniedBecause::CallerMemoryDescriptor`, whose own text reads: *"Pins a descriptor over the
**caller's own** address range. In Mode 2 the caller is the guest kernel and the range is
guest RAM, so honouring it would hand the host driver a guest-chosen pointer."*

★ That refusal is **correct and stays**. It governs the **guest → us** direction. R25 is
**us → host RM**, over a range the isolate itself owns and wrote. Two directions, one class
id. ⚠ The available mistake is exactly the one the shape invites: a future edit that
"unblocks the class" because the host path needs it would **delete the boundary rather than
cross it**. `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR`'s new doc says so where a reader will be.

**(2) ⊘ The `NV_ESC_REGISTER_FD` prerequisite is ALREADY HELD, and porting it would be a
regression.** The brief said *"⚠ including the `NV_ESC_REGISTER_FD` prerequisite at
`C:7503-7509` … That is a trap; port the comment with it."* The C carries it as a lazy
`m2_gpu_registered` flag checked on every descriptor alloc. **This port already binds the GPU
node to the control session in `RmConnection::open`'s R3** — for the connection's whole life,
with the C's own `0x23 INVALID_CLIENT` in the comment. So by the time any caller reaches
`alloc_os_descriptor` the prerequisite is a **structural property of the type**, not a step.
★ Porting the flag would have added a second, weaker copy of an invariant we already hold —
and the second copy is the one that later drifts.

**(3) ⊘ The brief's flag citation named the wrong site AND omitted the required flag.**
`C:6110-6113` is a **comment** inside the M5.19 pushbuffer-forwarding path, not the
descriptor call. The flags are at `C:7519-7524`, they are `0x40001010`, and they carry
**`MAPPING_NO_MAP`** — which the C's own comment says is **required**: *"without it the
driver returns EINVAL (it tried to auto-map a describe-only allocation)."* The brief named
only the coherency bit. The word is now reassembled from four named constants and pinned by
`nvos02_flags_encode_a_value_into_their_field`, so a constant that drifts into the wrong
field breaks a unit test rather than surfacing as `NV_ERR_INVALID_FLAGS` on a bench.

**(4) ⊘ `GuestWindow::place` is not at `kayfabe-isolate/src/lib.rs:348`.** That line is a
doc-comment on `RmError::NotExportableAsMemory` *mentioning* it. It lives in
`kayfabe-linux-raw::window_unsafe`. Minor, and it belongs in the list because the whole
paragraph it anchored — *"and already rejects `Backing::DeviceFile`"* — was read off the
mention rather than the code.

**(5) ⊘ AND THE CENTRAL PREMISE: the increment's real obstacle was not the fd crossing.**
The brief's "★★★ NEW — and this is the real size of the increment" was an fd-carrying
`Request::ImportBacking`. It is not what stood in the way. The load-bearing obstacle is that
**`OS_DESCRIPTOR` requires handing a host CPU address to a driver, and `kayfabe-linux-raw`
forbids one from crossing its boundary in any representation** — §4.2.1 refusal 3, held by a
compile-fail test (`tests/ui/no_base_address.rs`) that names `region.base()` and
`reservation.base()` as errors.

★ The architecture had **already written the answer down**, in the field's own docs:
`Nvos02ParametersWithFd::p_memory` says *"a backend that needs to must route the address
through `kayfabe_linux_raw::Indirect`, which is the only place one may exist."* The rung was
built to that sentence.

### 16.67.2 THE INCREMENT — `Indirect` gains a second target

`Indirect::describing(at, region, offset, len)` names a **bounded region** where
`Indirect::new` names a `&mut [u8]`. The address is minted inside `chardev_unsafe.rs`,
patched for the duration of one `ioctl`, and **scrubbed** by the same unconditional loop as
every other. `MappedRegion::addr_at` is `pub(crate)`, by the precedent
`GuestWindow::userspace_addr_at` set for KVM memslots.

⚠ **It is the one indirect whose effect outlives the syscall.** RM `pin_user_pages`-walks the
range and holds those pages until the object is freed; unmapping our view does **not** un-pin
them. The bounds check is load-bearing in a way the others are not — a `len` past the end of
the mapping is not a bad read of our heap, it is **the driver pinning whatever this process
mapped next**. The bound is therefore established at construction **and** re-established in
`ioctl`'s pre-pass — the pass that runs before any byte of `arg` is touched, so a patch loop
cannot fail partway and leave a live address in a caller's buffer with no scrub behind it.

### 16.67.3 ★★★★★ THE RESULT — all four falsifier cells, and the fourth did not fire

| arm | predicted reading | outcome on the GA106 run |
|---|---|---|
| **A** | it is a **port** | ★ **THIS ONE.** `placed at 0x300400000 AS ASKED, CE retired (sem 0x1), dst[0] 0xa112fffe -> 0x5eed0001, 65536 of 65536 bytes compared EQUAL` |
| **B** | refused to a cap-dropped child ⇒ redirects everything | ⊘ **did not fire** — see below, and the test is stronger than "root worked" |
| **C** | `map_gpu_va` returns a different VA ⇒ shadow-forwarding cannot work as designed | ⊘ did not fire; `got_va == asked_va` exactly |
| **⊘** | coherency: placed and retired but the GPU saw other pages | ⊘ did not fire; **every** word of 65536 matched |

★★★ **Arm B was tested, not assumed away.** The rung was run a second time under
`setpriv --reuid=65534 --regid=65534 --clear-groups --no-new-privs`, and the privilege state
was captured **in the same log**: `Uid: 65534 65534 65534 65534`, `CapPrm/CapEff/CapAmb = 0`,
`NoNewPrivs: 1`. The identical green came out. ⇒ **RM does not require privilege to let a
process describe its own pages.** `euid` is printed on the `info` line of every run for
exactly this reason — a rung whose answer depends on privilege must say which privilege it
had.

⚠ **Scope, stated rather than implied:** `setpriv` is not the isolate's sandbox. It removes
capabilities, which is the variable `osIsAdministrator()` reads, and that is arm B's
substance. It does **not** exercise the user namespace, the mount namespace or seccomp — a
filter that blocked `memfd_create` would be **our** refusal, not RM's, and would look nothing
like this.

### 16.67.4 ⊘ AN INSTRUMENT DEFECT I SHIPPED AND THEN FOUND IN MY OWN OUTPUT

The first passing run printed `65536 of 65536 bytes match`. Both numbers came from **the same
variable** (`e.bytes`, formatted twice). The *gate* was correct — `mismatch.is_none()` — but
the number a human reads was a **tautology**, and would have printed `65536 of 65536` over a
comparison loop that examined nothing.

⇒ `bytes_compared` is now incremented **inside** the loop, and `compared_everything()` joined
the `reached()` conjunction: a `None` mismatch over an empty loop is not agreement and can no
longer be printed as one. A partial loop with a clean verdict prints `FAIL … VOID`, and it is
checked **before** the coherency arm so it can never be reported as one — "the loop did not
run" and "the pages disagree" are different subjects.

★ This is `measure_at_the_boundary_not_inside` in miniature: **a reported count must come
from the thing that did the counting.**

### 16.67.5 ★★★ AND A NEGATIVE CONTROL, because a green with no reachable red proves nothing

`--osdesc-negative` (`OsDescSeed::Never`) runs the **identical** chain over a memfd nobody
wrote. **MEASURED** 2026-08-10 on the RTX 3060 GA106 at driver 580.159.04, binary
stamped `rev 40d44db84` — `traces/real_ga106/rmladder_r25_osdescriptor_real_ga106.txt`,
arms A and C, at both privilege levels:

```
ok    R25 neg control    = the memfd was never written and the CE delivered 0x00000000 at
                           word 0 where the pattern would have been 0x5eed0001
```

⇒ the comparison **has been watched to fail on the same hardware that produced the green**.
⚠ It is not a "skip the check" flag: both arms compare every word and differ only in what the
correct answer is.

### 16.67.6 ⊘ WHAT R25 CANNOT SEE — stated now rather than discovered later

- **Not a guest VA.** The address is one *we* chose (`0x3_0040_0000`, deliberately not R9's
  constant, so a pass cannot be a remembered address). Whether a host GPU walking a host VAS
  built from **guest** VAs would miss is invisible from here — and with fault delivery
  unbuilt, such a miss is a **hang** inside UVM's replayable-fault loop, not an error.
- **Not the fd crossing.** Nothing here carries a descriptor from a VMM to an isolate. The
  memory is minted, mapped, written and described **inside one process**. `Request::ImportBacking`
  remains unbuilt, and is the next increment — but it is now a *transport* question with no RM
  answer left in it, which is a much smaller thing than the brief thought.
- **Not the sandbox.** See 16.67.3's scope note.
- **Not any individual flag.** The run says the four flags together are sufficient; it never
  says each is necessary. `injection_measures_necessity_never_sufficiency`, read the other way.

### 16.67.7 The rung number

⊘ **This is R25, not the R20 the brief asked for.** R20 is taken — the `NV2081_BINAPI` probe
— and so are R21 (gpu-info sweep), R22 (bus-info sweep), R23 (atomics) and R24 (PCE mask). A
rung number is how a bench result is attributed months later; two rungs sharing one is how a
green line gets read as evidence for the wrong thing.

---

## §16.68 ★★★★★ `R26` — a host channel whose GPFIFO ring is at an address **WE DICTATE**, and the host-Xid watcher that makes its failure legible

`[measured 2026-08-10, RTX 3060 GA106 / 580.159.04, bench `vh`, binary stamped`
`REV_UNDER_TEST=ed51a26a77f06d537701b43abefa899201e4d6cd — a 40-character DERIVED stamp,`
`traces/real_ga106/rmladder_r26_dictated_ring_real_ga106.txt]`

### 16.68.1 ⊘ WHAT I REFUTED FIRST — four of the brief's claims, and one of my own instruments

**(1) ⊘ "No harness has ever read the host's `dmesg`."** Two do, and both predate this rung:
`scripts/bench/gpu_fault_containment.sh:34` (`xid() { dmesg 2>/dev/null | grep -ci xid; }`) and
`scripts/bench/gpu_wedge_containment.sh:37`, each running on the **host** over `ssh root@BOX`.
★ The *substance* of the brief's point survives and is why this rung still built one: those
two are **containment experiments**, self-contained scripts that read `dmesg` about their own
deliberate fault. Nothing wrapped the **ladder**, so no RM-level run this project has ever
taken was watched. The gap was the ladder's, not the tree's.

**(2) ⊘ The line citations were drifted, all four of them.** `rm.rs:2129` (*"map ring into the
Vas (RM chooses the VA)"*) is at **`:2350`**, with `alloc_channel` at `:2383` and the body it
delegates to, `alloc_channel_on`, at **`:2739`**. `map_gpu_va(at)` is not at `:2280-2301`; the
trait impl is at **`:2503`** and the primitive that actually sets `DMA_OFFSET_FIXED_TRUE` is
`raw_map_dma` at **`:1259-1290`**. Minor individually — but the third one matters, because the
*real* obstacle lives at `raw_map_dma` and not at either site the brief named.

**(3) ⊘ THE ACTUAL WALL was a reasoned refusal, not a missing parameter.** `raw_map_dma`'s
docs already argued the opposite of this rung, on purpose: *"`None` is **not** a weakening of
`#102` … A channel's own ring is exactly that, and demanding a fixed address for it would
mean inventing a host-private VA window — a policy this rung has no way to enforce and every
way to get wrong."* A parallel read of the whole test tree found this to be **the single
strongest wall against the increment**, and it is prose, not a test. ⇒ It was answered rather
than deleted: the paragraph is amended in place, naming what it got right (the *policy* is not
that function's) and what no longer describes the tree (a caller now supplies the address).
⊘ A design doc that silently stops being true is the trap; a stale comment is what the port
was bitten by two rungs ago.

**(4) ⊘ "`alloc_channel_at` should be the port's verb" — NOT this rung, and the tree says so.**
`RmBackend` gains **no method**. Nothing in the core has a dictated ring VA to pass; the
shadow-forward that will is unbuilt, and a trait verb with no caller is exactly the bolt-on
`alloc_engine_object`'s own docs warn about one method up. A verb-set sweep priced the
alternative: a new trait method needs four backend impls, a new `Request` variant (which the
runtime set-assertion `proto.rs:988` pins), a `VerbKind`, and — ★ the nasty one — an entry in
`kayfabe-mocks/src/lib.rs:2440`'s acquisition fold, whose `_ => None` arm would have compiled
fine and then fired `"★★ DANGLING …"` teardown failures across every audited suite with
nothing pointing at the cause. The increment is instead `HostRmBackend::alloc_channel_at`,
`pub` for exactly the reason `alloc_channel_on` is: the ladder is the only thing that can ask
hardware this question.

**(5) ⊘⊘ AND ONE OF MINE — `REV_UNDER_TEST` WAS NEVER DERIVED FOR THIS BINARY.** The first
`--dictated-ring` run on real hardware printed **`REV_UNDER_TEST=unstamped`**. `KAYFABE_BUILD_REV`
is emitted by **`kayfabe-qemu-raw/build.rs`**, and `cargo:rustc-env` applies to *that crate
only*; `kayfabe-rm-ladder` lives in `kayfabe-isolate-host`, so its `option_env!` was `None`
unless someone exported the variable by hand. ⇒ **Every earlier ladder trace's revision was an
operator's assertion about the tree, made by the same person making the claim, standing where
a derivation was supposed to be.** A stale binary rebuilt with a freshly typed variable would
have claimed the fresh revision — the precise failure `CLAUDE.md`'s rev-stamp trap exists to
catch, defeated by the trap's own instrument. `kayfabe-isolate-host/build.rs` now derives it
from `git` with the same 40-hex shape check and the same `-dirty` suffix, and the run below was
re-taken at a stamped, clean binary rather than reasoned about.

★★★ **And this is not an inference — the committed traces prove it by their LENGTH.**
`[measured 2026-08-10]`, over `traces/real_ga106/*.txt`, the historical `REV_UNDER_TEST`
values fall into three populations:

| value | length | what it can only have come from |
|---|---|---|
| `4e79a140f35eb2741bd620bba2bf129db5abb551` (R22) | **40** | `git rev-parse HEAD`'s own output — a real commit in this repo. ⚠ Machine-*generated*, but still operator-*run*: before this rung the ladder had no derivation, so it reached the binary as an exported variable |
| `40d44db84` (R25), `8dac2705d` (R25 re-run), `6f8239835` (R18), `1d5704dd9`, `6c9e3d2bb` | **9** | ⊘ **a HUMAN.** The derivation shape-checks for *exactly 40* hex and falls back to `unknown`; it is structurally incapable of emitting an abbreviated sha |
| `unstamped` (R21, R24) | — | nothing was exported at all |

⇒ **Every 9-character stamp in this repository is an operator's assertion wearing a
derivation's clothes**, and the certain half of that is the converse rather than the
forward direction: a 9-character value **cannot** have come from the shape-checked
derivation, whereas a 40-character one merely *might* have. ⊘ Stated that way round on
purpose — the strong reading ("40 chars ⇒ derived") is the one the evidence does not carry,
and R22's row is exactly the counter-example. ⚠ In particular `fd4ffe7`'s commit message — *"§16.67 R25 re-run at the
SHIPPED head — the stamp read off the binary, not the checkout"* — is describing a stamp that
was inside the binary **because a person put it there**. The re-run was still the right call
and its four arms still measured green; what it did **not** do is what its title claims, and
the distinction is the entire content of the rev-stamp trap.

★ The tell was available the whole time and nobody had a reason to look: **a real sha is 40
characters.** A provenance marker that can be produced by typing is not provenance, and the
cheapest possible check on one is its shape.

⚠ The build-shim half of the rev-stamp trap (`CARGO_TARGET_DIR` vs `build_qom_shim.sh:38`)
**does not apply**: R26 is host-only, there is no hypervisor and no QOM shim in the path. Said
explicitly rather than skipped silently.

### 16.68.2 The increment

`HostRmBackend::alloc_channel_at(vas, engine_type, ring_at: Option<GpuVa>)` is
`alloc_channel_on`'s body with one degree of freedom; `alloc_channel_on` is now
`alloc_channel_at(.., None)` and is byte-for-byte the previous behaviour. `Some(va)` passes
`DMA_OFFSET_FIXED_TRUE` and then **checks RM's `[OUT]` `dmaOffset` against the ask**, unwinding
into `RmError::PlacementRefused` on a mismatch.

★★ `ring_at` names the **ring object's base**, not `gpFifoOffset` — the two differ by
`GPFIFO_OFFSET`, and confusing them is a silent off-by-a-page in which hardware fetches 64
bytes of *pushbuffer* as GPFIFO entries and fails nowhere near this call.

⊘ **It is deliberately not the guest's `gpFifoOffset` yet.** A shadow-forwarded ring is guest
memory with a guest layout; this establishes the one fact that stood between here and there.

`HostRmBackend::channel_ring_va` reads the placement back out of `ChannelParts`, so a
diagnostic can check the placement **without asking the call under test**.

### 16.68.3 ★★★★ THE FALSIFIER, committed before the run — and the fourth cell is the point

| arm | reading | outcome |
|---|---|---|
| **A** | it is a **port** | ★ **THIS ONE**, at both privilege levels |
| **B** | `PlacementRefused` — RM chooses, and shadow-forwarding is dead as designed | ⊘ did not fire |
| **C** | RM refuses the channel outright — the address is legal to *ask* and not to *use* | ⊘ did not fire |
| **⊘ D** | **INERT CHANNEL** — placed as asked, and `GP_GET` never moved | ⊘ did not fire — ★ **and this is the cell a "did the alloc succeed?" test scores green** |

★★★ **Cell D is why the rung submits.** `alloc_channel_at` returning `Ok` is the thing under
test, so verifying it by checking that it returned `Ok` measures nothing — the R25 tautology
(`§16.67.4`) one plane over. Worse, cell D can hold *while every green above it holds*: RM
records a mapping at our address, the channel allocates, a token is minted, and hardware never
fetches a byte. That is the C's M5.47 shape, which produced **zero utilisation and no Xid**.
So the bar is two facts, and the second is `GP_GET` — the one word in this crate hardware
writes and we do not.

### 16.68.4 ★★★★★ THE RESULT

```
ok    R26 placement       = RM reports the ring at 0x0000000411000000 AS ASKED (token 0x00000004)
                            — necessary, and NOT yet sufficient
★     R26 dictated ring   = ring placed at 0x0000000411000000 AS ASKED, GP_GET 1 caught GP_PUT 1,
                            sem 0x1dea0026 (want 0x1dea0026) — the GPU FETCHED from an address we chose
[xid-watch] ★ HOST Xid CLEAN across the command — 0 before, 0 after
```

★★ **Arm B tested, not assumed away**, on R25's precedent: re-run under
`setpriv --reuid=65534 --regid=65534 --clear-groups --no-new-privs`, with
`Uid: 65534`, `CapPrm/CapEff/CapAmb = 0`, `NoNewPrivs: 1` captured in the same trace.
Identical green. ⇒ **RM does not require privilege to let a process dictate where its own
channel's ring lives.**

★★★ **AND A NEGATIVE CONTROL, because a green with no reachable red proves nothing.**
`--dictated-ring-negative` maps a 64 KiB object at `0x4_1100_0000` **first**, then asks for a
channel ring at the same address:

```
ok    R26n occupied       = 0x0000000411000000 is now taken by another object
★     R26n CONTROL FIRED  = RM refused the channel at an occupied 0x0000000411000000 with
                            NoMemory — the address is enforced by the driver
```

⇒ the placement is **RM's fact, not our formatting**. ⊘ The control's third arm — a channel
built at an address another object already occupies — is printed as `FAIL` and would have been
a *finding* about address identity, not a control failure.

### 16.68.5 ★★★★ THE HOST-Xid WATCHER — `scripts/bench/host_xid_watch.sh`

Landed in the same commit, because without it *"the copy silently did nothing"* and *"the GPU
faulted"* are **the same observation**: a semaphore that never landed, a destination that never
changed, and every ioctl returning `NV_OK`. A host GPU missing on a host VAS built from our
addresses raises **no guest fault** — the host driver services it and prints `Xid 31 FAULT_PDE`
into the **host** ring buffer, which no ladder run had ever read.

⊘ **The nasty edge, and the reason phase 0 exists: `dmesg` *failing* and `dmesg` finding *no
Xid* produce the same empty grep.** `kernel.dmesg_restrict=1`, a container without
`CAP_SYSLOG`, or a wrapped ring buffer all yield "no Xid lines" from an instrument that
observed nothing — and a green from such a watcher is *worse* than no watcher, because it reads
as a watched clean. So readability is asserted **positively and first** (`dmesg` must exit 0
**and** produce ≥2 lines), the count is recorded in the log as `host_dmesg_lines=N`, and an
unreadable log exits **7**, never 0.

★★★ **Both failure paths were watched to fire before the instrument was trusted**, on the same
discipline R25's negative control established:

| control | expected | observed |
|---|---|---|
| `dmesg` shimmed to exit 1 | **7**, "UNWATCHED" | ★ fired, `rc=7`, and it did **not** print a clean |
| a synthetic `Xid 31 … FAULT_PDE` injected mid-run, watched command exiting **0** | **6**, run void | ★ fired, `rc=6` — the command's own success did not rescue it |

⚠ It exits **6** on a new Xid even when the watched command returned 0, deliberately: *"the run
is VOID whatever it printed"*.

### 16.68.6 ⊘ WHAT R26 CANNOT SEE

- **Not a guest VA.** `0x4_1100_0000` is ours, chosen to be neither R25's `0x3_0040_0000` nor
  R9's constant so a pass cannot be a remembered address. That the number is *ours to pick* is
  the finding; that a *guest's* number would be accepted is not tested.
- **Not the guest's ring LAYOUT.** `gpFifoOffset` is still derived from our `GPFIFO_OFFSET`
  over our own 64 KiB object.
- **Not a forwarded doorbell.** The submission is the isolate's own. A guest doorbell is the
  next rung, and per the standing sequencing it should land with the RC/error-notifier path
  wired, so a ring-gate refusal becomes an error rather than a hang.
- **Not fault DELIVERY.** Nothing here delivers a fault to a guest. ⚠ And the reason not to
  build it half-way, stated precisely rather than repeated: `ogkm-580:
  uvm_ampere_host.c:140` spins on the CHRAM `FAULTED` bit via `UVM_SPIN_WHILE`, which is
  `for (uvm_spin_loop_init(spin); (cond); UVM_SPIN_LOOP(spin))`
  (`ogkm-580: uvm_common.h:302-304`). ★ The correction worth carrying: `UVM_SPIN_LOOP`
  **does** compute a timeout status — and this call site **discards it**, so the loop's only
  exit is the bit itself. It is not that the driver lacks a timeout mechanism; it is that
  this path declines to read it. Delivering a fault and failing to set that bit hangs a
  guest kernel thread forever.
- **Not `NoMemory`'s exact RM status.** The negative control's refusal is reported through the
  port's error type; which NVIDIA status produced it was not decoded, and the control does not
  need it.

### 16.68.7 ⊘ The re-run at the SHIPPED head, because the delta LOOKED inert

The four arms were first taken at a work-in-progress revision; the tree then moved by a
formatting reflow, a `free` of the negative control's squatter object, a dead-constant
deletion in an unrelated crate and this section's prose. ⊘ **That delta looks inert, which is
exactly the reading the rev-stamp trap punishes** — and one item in it is not inert at all:
freeing the squatter changes what the *second* run of the control would find. So it was
re-run rather than assumed, at `ed51a26a77f06d537701b43abefa899201e4d6cd` on a clean tree —
and then AGAIN when clippy found that the insertion had attached R26's doc-comment to the
negative control, because "only the function order moved" is another delta that looks inert,
and the committed trace carries that stamp.

★ The stamp in that trace is **40 characters**, and this is the first ladder trace in the
repository for which that is true by construction rather than by an operator's typing.

★★ **And when later commits moved HEAD past the revision that trace names, the attribution
was CHECKED rather than argued:**

```
git diff --name-only ed51a26 HEAD
  docs/design/execution_plane_increments.md
  tests/tests/cpu_ce_executor.rs
  tests/tests/gsp_rm_alloc.rs
  traces/real_ga106/rmladder_r26_dictated_ring_real_ga106.txt
```

⇒ none of them is in `kayfabe-rm-ladder`'s dependency tree, so the binary the trace names is
still the binary this tree builds. ⊘ *"That commit was only tests and docs"* is the same
sentence as *"the delta looks inert"*; the difference is that this one was **run** — the
question "which files does the artifact actually depend on?" has an answer a command can
give, and taking it from a command is the whole discipline.
