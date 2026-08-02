# The execution plane — the increments from here to a guest CE copy on the host GPU

> **Status: PLAN, 2026-08-02.** Written at master `cf3aae9`; increment **E0** built and
> measured at `e10a6bf` on the RTX 3060 bench, **E3** at `6e4f66f`, **E0b + E1** at
> `853a311` (§6) and **E2** at `5c1f501` (§7). Every row is `[src]` at `cf3aae9`,
> `[measured]` with a named run and revision, or explicitly `[assumed]`. **E4–E6 are not
> built.**

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
| **E5** ◐ | the address table is populated from the guest's own bindings, so the CE operands resolve in the isolate's host VAS | a guest VA that *was* bound resolves; the copy's operands are found | ★ a VA that was **never bound** must FAULT (`mode2_address_table.md`: miss = fault, never a reverse-resolve) — **PARTIAL, §9: source 1 whole, source 2 reaches a ROOT PAGE and no further** |
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

### 9.3 ⊘ What §9 does NOT establish

1. **No boot proves any of it.** `only_live_boots_are_proof`. The guest still never submits
   (`doorbells: 0 arrived` at this wall), so `read_pushbuffer` remains **latent** on the
   product path — what changed is that it is now *correct* when it becomes live.
2. **Nothing about the aperture refusal on real traffic.** `FwdFault::PushbufferAperture` is
   reachable and tested, but no measurement says whether a real GA106 guest ever puts its
   pushbuffer in vidmem. `ogkm-580: mem_utils_gm107.c:812-820` has RM refusing *"USERD in
   sysmem and PushBuffer/GPFIFO in vidmem"* as a WAR, which is suggestive and is **not** the
   same statement.
3. **Nothing about E6.** No `VerbPlan` ran, no doorbell was rung, no `ce_copy`.
