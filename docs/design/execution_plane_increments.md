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

`[built]` at `62a06af` + `d79b67f`, GPU-free; ⊘ **no boot of this revision has been taken yet**,
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
