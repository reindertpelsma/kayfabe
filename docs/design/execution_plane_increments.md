# The execution plane — the increments from here to a guest CE copy on the host GPU

> **Status: PLAN, 2026-08-01.** Written at master `cf3aae9`; increment **E0** built and
> measured at `e10a6bf` on the RTX 3060 bench. Every row is `[src]` at `cf3aae9`,
> `[measured]` with a named run and revision, or explicitly `[assumed]`. **E0b and E1–E6
> are not built.**

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
| **E0b** | ★ the spawn becomes **lazy**, so the first *guest* `GSP_RM_ALLOC` is what materializes the isolate — the increment E0's own measurement created (§3.6) | the child's first sighting is **after** the device-open phase line, not 28 s before it | variable unset → still zero children |
| **E1** | a **failed** real isolate stops being indistinguishable from the stillborn one (bench gap 7) | with the image stubbed, the seam reports a refusal whose text differs from `STILLBORN_WHY` | the unstubbed build reports neither |
| **E2** | the doorbell reaches the core: a guest MMIO write to the usermode doorbell aperture arrives at `kayfabe_rt::SharedDevice::doorbell` | a boot in which a guest doorbell write produces a `DoorbellOutcome`-or-named-`FwdFault`, counted | a non-doorbell BAR write in the same run produces neither |
| **E3** | ★ **`Ga10xArch::decode_doorbell` is built** and validated against real silicon | a token RM itself hands a channel decodes to that channel's own vChid, on hardware | a token from a *different* channel must decode to a different vChid — and a fabricated token must decode to `None` |
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

## 4. Order of work, and the one thing that should be re-decided

E0 → E1 → **E3** → E2 → E4 → E5 → E6.

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
- **Bites:** `scripts/bite_isolate_selector.py` — **5/5 fired** at `d6caffa` (`BS1` default
  moves to `Real`; `BS2` unknown value degrades silently; `BS3` case-insensitive parse;
  `BS4` host plane degrades instead of refusing without the feature; `BS5` refusal stops
  naming the planes). Restored-tree sanity check GREEN.


