# The execution plane — the increments from here to a guest CE copy on the host GPU

> **Status: PLAN, 2026-08-01.** Written at master `cf3aae9`; increment **E0** built and
> measured at `e10a6bf` on the RTX 3060 bench, **E3** at `6e4f66f`, and **E0b + E1** at
> `853a311` (§6). Every row is `[src]` at `cf3aae9`, `[measured]` with a named run and
> revision, or explicitly `[assumed]`. **E2 and E4–E6 are not built.**

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
| **E2** | the doorbell reaches the core: a guest MMIO write to the usermode doorbell aperture arrives at `kayfabe_rt::SharedDevice::doorbell` | a boot in which a guest doorbell write produces a `DoorbellOutcome`-or-named-`FwdFault`, counted | a non-doorbell BAR write in the same run produces neither |
| **E3** ✅ | ★ **`Ga10xArch::decode_doorbell` is built** and validated against real silicon | a token RM itself hands a channel decodes to that channel's own vChid, on hardware | a token from a *different* channel must decode to a different vChid — and a fabricated token must decode to `None`. **DONE — `doorbell_token_encoding.md`** |
| **E4** | GA10x `UserdModel` + `PushbufferAbi` replace `UnbuiltUserd`/`UnbuiltPushbuffer` | `read_pushbuffer` over bytes captured from a real boot yields `LAUNCH_DMA`/`SEM_EXECUTE` at the offsets the guest wrote them | garbage bytes must **fault**, not decode to a plausible method |
| **E5** | the address table is populated from the guest's own bindings, so the CE operands resolve in the isolate's host VAS | a guest VA that *was* bound resolves; the copy's operands are found | ★ a VA that was **never bound** must FAULT (`mode2_address_table.md`: miss = fault, never a reverse-resolve) |
| **E6** | the join: guest CE copy → `plan_doorbell` → `Worker::execute` → `HostRmBackend::ce_copy_outcome` | `CeEvidence::copied()` — R17's predicate, driven by the guest | the same boot with `KAYFABE_ISOLATES` unset must **not** produce it |

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
exactly **one** `Proc` — the system one. The multi-process claim above is carried by the
suite and by the code's key, **not** by any measurement on this hardware. The first arm that
could show it is E6 with two guest processes, and nothing here has been run against one.

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

E0 → E0b → E1 → **E3** → E2 → E4 → E5 → E6. **E0, E0b, E1 and E3 are done**; E2 is next.

★ E3 is pulled ahead of E2 deliberately. E2 (an ABI entry point for the doorbell) is
mechanical and can be written at any time; E3 is the increment most likely to be *wrong for
months without anything going red*, so it should be paid for while there is still a
hardware bench to settle it on, and while `kayfabe-rm-ladder` — which can allocate a
channel and print the token RM assigned it — is a live, passing instrument.
`c_rust_trace_differential.md` already records that the C oracle is **perishable**.

⚠ **To re-decide before E4:** whether the pushbuffer codec is transcribed from `ogkm` or
generated from it. The GMMU format went the transcription route at `#149` and
`955c79a` then had to build "a GMMU-format oracle out of NVIDIA's OWN encoder" to catch
five bites transcription missed. That is evidence for generating, and it is a bigger
decision than a row in a table.

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
