# CLAUDE.md — repository navigation

This file is a **map**, not a rulebook. It points to the authoritative docs; read those for
detail. Keep it short and navigational — do not inline implementation specifics that will rot.

## What this project is

`nvkvm` — WSL2-style NVIDIA GPU **ioctl/RPC forwarding** for KVM/QEMU guests on commodity
hardware, so an unprivileged host process drives a real host GPU on behalf of a guest. Two modes:

- **Mode 1** — guest forwards its NVIDIA *userspace ioctls* to a host stub that replays them on the
  host `/dev/nvidia*`. Mature: CUDA, multi-process, graphics/Vulkan, NVENC, ~host parity. Must not
  regress.
- **Mode 2** — guest runs the **stock** NVIDIA kernel driver against an **emulated GPU + faked
  GSP** (`src/qemu/nvkvm_gpu_emul.c`); we recover the guest's intent and forward *real* compute to
  the host. North star: `cuCtxCreate → first compute → matmul`, numerically correct. In progress.

Start here: `README.md`, `PLAN.md`, `docs/ARCHITECTURE.md`, `docs/MILESTONES.md`.

## ★★★ THIS REPO'S ROLE CHANGED (2026-07-29) — read before you touch anything

This is the **research artifact**. The **product** is now `kayfabe`, a clean-slate Rust
rewrite at **`/workspace/nvkvm-rs`** (GitHub `reindertpelsma/kayfabe`). This repo is not
history and is not dead — it is the **standing differential oracle** for that rewrite, and it
is maintained for exactly that purpose.

Why it still matters: it was **rebuilt from source on fresh hardware on 2026-07-29** and
reproduced `cuCtxCreate → 2048² matmul` at **`bad=0 maxerr=0` on a STOCK, unpatched guest**
(ladder: `cup2` → `cupctx2_min` (#12) → `cup8` → `cup8_iter` (#13)). It is the only
implementation a real NVIDIA driver has ever accepted end-to-end, so it can answer questions
no amount of Rust-side testing can.

> ### ⊘⊘⊘ CORRECTED HOURS LATER, 2026-08-12 — **THE BLOCK BELOW IS OVERDRAWN, AND THE
> ### OVERDRAW MISDIRECTED A WHOLE DAY.** Read this first; the text below is true of the **CE
> ### COPY plane** and **FALSE of the GR/COMPUTE plane**.
> `cap3_matmul_forwarding` — *the passing run* — carries a **self-describing header**
> (`docs/design/kernel_gr_channels_and_the_mme_exposure.md:372`, the captured `props:` line; quoted
> again at `docs/design/how_the_c_passed_the_gr_wall.md:119`):
> ⊘ **CITATION CORRECTED 2026-08-12 (w275).** This block previously cited
> `docs/BENCH_REBUILD_NOTES.md:119`. That file contains the string **zero times**
> (`grep -c m2hostsem docs/BENCH_REBUILD_NOTES.md` → `0`); its line 119 is about `cap1b`'s
> GSP-D6 continuation elements. The **flags are right and the reading is right** — only the
> pointer was wrong, almost certainly by carrying `:119` across from the doc that does quote
> them. ⚠ Same class this file already names: **citing the oracle is not the oracle being
> right**, and a wrong pointer in the most-read file in the repo is checked by nobody because
> it looks checked.
> **`m2fwd=1  m2exec=1  m2hostsem=0  m2cefwd=0  m2cexec=0`**
> ⇒ ⊘ **`m2cefwd=0`.** The claim below that it is *"the flag on every green run"* is **contradicted
> by the trace's own header.**
> ⇒ ★ **`m2exec=1`** — the execution plane was **ON**, and **`m2hostsem=0`** — the host-semaphore
> forge was **OFF**. The C's `SET_REPORT_SEMAPHORE` CPU-forge exists (`:6544-6573`) and **wrote
> nothing to the completion page in the green runs**. **The real host GR engine executed the
> guest's ctx-init pushbuffer and wrote `0x2_0440fff0` itself.**
> ⇒ ★★★ **So the C IS an oracle for GR engine execution** — precisely what the block below says it
> is *"no oracle at all"* for. The `:4228` forge is real but is scoped to the **kernel CeUtils
> scrubber channels** and explicitly excludes user GR/CE channels.
>
> ★★★★★ **AND HERE IS WHAT THE MIS-SCOPING COST:** reading the C as *"it forged everything"*
> retired the only question that mattered — **how did the C's host GR get a COMPLETE VAS?** The
> answer is stated in the C's own source at `src/qemu/nvkvm_gpu_emul.c:582`:
> *"**Fault-safe: a mapping is always backed before the engine that uses it runs.**"*
> ### ⊘⊘⊘ CORRECTED 2026-08-13 (w289) — **THE SOURCE LIST BELOW IS INCOMPLETE, AND THE MISSING
> ### ONE IS THE RM CAPTURE THE OWNER KEEPS ASKING ABOUT.**
> The two mechanisms named below are both **page-table-derived**, and this file therefore reads
> as *"the C was PDB-only; no RM capture involved"* — while `docs/design/mode2_address_table.md`
> says the co-equal sources are **(1) bind-time RPC/ioctl bindings** and (2) the observed CE
> write. **Two docs in this tree, opposite answers, to the question the fix turns on.**
> ★★★ **Settled from the C's source, 2026-08-13. `mode2_address_table.md` is RIGHT and THIS
> FILE WAS WRONG.** There is a **third source and it is an RPC capture**:
> `NV2080_CTRL_CMD_GPU_PROMOTE_CTX` (`0x2080012b`) is snooped in flight
> (`nvkvm_snoop_promote_ctx`, `src/qemu/nvkvm_gpu_emul.c:2446-2472`), its
> `{gpuPhysAddr, gpuVirtAddr, size, physAttr}` entries folded into a side table by
> `nvkvm_record_va_map` (`:2417-2440`), and that table is what `nvkvm_chan_translate`
> **consults FIRST** (`:305-309`, which cites `mode2_address_virtualization.md` *"capture path
> #2"*). The rows are then backed by `nvkvm_m2_back_and_map` (`:3902`).
> ⇒ **THREE sources, not two:** (1) the `GPU_PROMOTE_CTX` RPC capture, (2) the doorbell-time GR
> page-table sweep, (3) the observed CE page-table write at the release.
>
> ★★★★★ **AND THE DIFFERENCE THAT IS LOAD-BEARING FOR THE FIX — a single line.** The C rounds
> every promote-derived mapping **UP TO 64 KiB** before mapping it:
> `uint64_t asize = (size + 0xffff) & ~0xffffull;` (`:7920`). This port binds at the
> **declared length** (`kayfabe-core/src/promote.rs`; no rounding anywhere in it), which is what
> produces w277's `0x8600`-long, non-page-aligned rows and the **sub-page hole** it recorded:
> *"2 560 bytes our own `resolve` answers `Miss` for inside a page the guest has mapped"*, held
> open by the `CrossesEnd` refusal. **The C could not have that hole; we do, by construction.**
> ⚠ Not yet measured against a fault — stated as a mechanism with both sides cited, and it is
> the first thing to test on the 82-ioctl CE repro.
> ⊘ The C also treats `st == 0x51` (`NV_ERR_NO_MEMORY`) on a FIXED map as **success** —
> *"the VA is ALREADY mapped in the host VASpace"* (`:7935-7938`) — a semantic our side must
> match or it will refuse exactly the ctx buffers the host RM already placed.
>
> Mechanism (the two page-table-derived sources; **see the correction above for the third**):
> a **doorbell-time sweep of the guest's GR page tables** (`m2_gr_pt_set`, re-swept
> whenever a tracked PT page is written) plus **observed CE page-table writes decoded at the
> completion-semaphore release** (`nvkvm_m2_cpt_sync_at_release`, `:592-604`).
> ⇒ **The C mirrored the guest's page tables WHOLESALE and committed the mirror before any
> completion became observable.** Every fault this campaign has chased one at a time — pushbuffer
> VAs, the semaphore page, the CE operand, its extent, the GR write — is **one instance of that
> single missing invariant.** ⚠ The C never met any of them because the sweep made them
> **impossible as a class**, and it went green **without servicing or forwarding a single GPU
> fault** (there is no fault-buffer emulation anywhere in the file).
>
> ### ⊘⊘ SCOPE THE ORACLE — 2026-08-12, measured from this tree's own source
> ### ⚠ TRUE OF THE CE COPY PLANE ONLY — see the correction directly above.
> **The green is a CONTROL-PLANE result. The data plane was CPU + emulator, not hardware.**
> - `m2cefwd` — the flag on every green run — is defined at `src/qemu/nvkvm_gpu_emul.c:9931` as
>   *"real-back the user-CE copy dst … (**completion/CPU-copy unchanged**)"*. ⇒ the **CPU** did
>   the copies.
> - Real host-CE execution is a **different** flag, `m2cexec` (`fwd_ce = s->m2cexec && …`,
>   `:9052`), and it was **OFF in the only green run**; a verbatim forward measured
>   **untranslatable** (`dst_phys=1`).
> - Completions were **emulator-written**: `:4228` completes `finishPayload` for the kernel
>   CeUtils channels because *"the scrub is a **no-op** for our backing … complete now if no
>   real work"*.
>
> ⇒ **`bad=0 maxerr=0` means THE DATA WAS RIGHT, not that a hardware forwarding path worked.**
> The C is a strong oracle for **RM semantics, control-plane ordering, and the driver's
> acceptance criteria** — and **no oracle at all** for engine execution, hardware completion,
> or forwarding throughput. ⚠ Its perf figures (20→60 tok/s) are **CPU-copy** numbers; do not
> read them as a forwarding baseline.
>
> ★★★ **PARTLY ANSWERED 2026-08-12 — there is now a DATA-PLANE oracle, and it is native.**
> `docs/reference/native_dataplane_cup2_ga106.md` + `traces/native_dataplane_ga106/` record what a
> real GA106 (`580.159.04`, no QEMU, no emulated GPU) actually does for `cup2`: the ring, the
> pushbuffer method stream, USERD `GP_GET`/`GP_PUT`, and the report semaphore. Headlines: a 4-byte
> `cuMemcpyHtoD` uses **no copy engine** — it is the compute class's I2M unit with the **payload as
> a pushbuffer literal**; the semaphore is at **`0x2_0440_fff0`, page offset `+0xff0`, in host
> RAM** — the **same address our guest uses**; and the GPU's authorship is proved by a report
> timestamp that tracks CPU wall time to **43 ppm**, not by a watchpoint (a DMA write is invisible
> to x86 debug registers — that instrument is a negative control only).
> ⊘ It bounds one workload on one chip: it says nothing about our emulated path, cannot order the
> release against the `GP_GET` advance, and the **guest's own `cuMemcpyHtoD` pushbuffer has never
> been decoded by anyone**, so the native↔guest comparison has a hole. See §8 of that doc.

**New, and load-bearing:**
- ★ **`traces/mode2_c_reference/`** — the committed §6 replay captures (~11 MB zstd, dense,
  `n_errors=0`). `cap1_coldboot_hermetic` (359 062 records) is the **only trace a replay can be
  CLOSED over**; `cap2b` is the negative trace (378 GSP elements read out of arbitrary guest
  RAM and answered `NV_OK` — the guest-reachable defect, now a fixture). **These are the
  durable artifact**: a bootable C on a rented box is not.
- **`src/qemu/nvkvm_m2_rec.{c,h}`** — the recorder. Property `m2rec` (+ `m2recfile`,
  `m2recmask`), default OFF. ★ Never reuse `m2_trace` for capture: it is *not* observationally
  neutral (it re-arms an O(n) audit and the crashwin DIAG family). **Never sample or cap** —
  the consumer's `diff()` is positional, so a drop shifts every later index.
- **`docs/BENCH_REBUILD_NOTES.md`** — the authoritative first-person rebuild log. ★ **Read it
  before rebuilding a bench; do not re-derive it.**
- **`scripts/mode2_diag/bench_boot.sh` / `bench_wait.sh`** — the correct boot + wait sequence,
  with the traps encoded inline. `rec_capture.sh` / `rec_dump.py` drive and decode captures.

**Four operational traps that cost real cycles — all measured:**
- ★★★ **A KILLED background job is indistinguishable from a RUNNING one, if absence-of-result is
  your only check.** Measured 2026-08-10: a detached bench job was **killed by SIGTERM at the ssh
  timeout**, leaving a **zero-byte** output file — and that state was read as *"still in flight"*
  **three times**, then reported to the owner as a pending measurement. ⇒ **Zero bytes is not
  "not yet"; it is a state that needs its own check.** Have the job write a **start marker and an
  exit-status line**, so *"file exists but has no terminator"* is detectable at all.
  ⚠ Same class as the serial-log trap below and as the `dlen=0` oracle rows: **an empty artefact
  reads as benign, and only inspecting its content distinguishes "nothing happened" from "nothing
  was recorded".**
  ⊘⊘ **CORRECTED within the hour, and the correction is the load-bearing half: `143` and `124` mean
  OPPOSITE things and arrive as the same word.** `143` = **SIGTERM, the job itself killed**, nothing
  written ⇒ dead. `124` = **the LAUNCHER's `ssh`/`timeout` expired** while the detached job **kept
  running fine** (measured: 3 `cargo test` processes still alive, results still accumulating).
  ⇒ **A nonzero exit from the thing that STARTED the work tells you nothing about the work.** Never
  infer "died" from an empty file alone — that reading would have declared a healthy job dead.
  Check the work **directly**: process liveness **and** the terminator line. ⚠ And note the
  composition — that liveness check is exactly where the `pgrep` trap below fails in **both**
  directions, so it needs the bracket trick *and* an `ss`/port check, not a bare `pgrep`.
- ★★★ **The serial log is NOT where the driver's output is.** Measured 2026-08-01:
  `grep -ci nvrm /workspace/bench/run_*_serial.log` returns **0** for every boot of that night,
  while older boots (`run_t135a_serial.log`) contain it. The guest driver is `modprobe`d over
  ssh *after* boot, so its `dmesg` goes to whoever ran the command and **nowhere else** — six
  consecutive rung claims had their only evidence inside a session transcript. The serial log
  still exists, is ~70 KB, is freshly timestamped and is named after the boot, so **every
  signal says the evidence is there**; only grepping for the content shows it is not.
  ⇒ Persist `dmesg` to `run_<tag>_dmesg.log` beside the serial log, and **assert it is
  non-empty and contains `NVRM`** — a harness that writes an empty file and exits 0 is worse
  than none, because the file's existence reads as capture.
- ★★ **`pgrep` fails in BOTH directions, and both are now measured.** `pgrep -x qemu-system-x86_64`
  **can never match** (`/proc/PID/comm` truncates to 15 chars ⇒ `qemu-system-x86`), so any "verify
  none running" built on it passes **vacuously**. Use `pgrep -x qemu-system-x86` **and**
  `ss -tln | grep 2223`.
  ⇒ ★★ And the mirror image, measured **three times on 2026-08-10**: `pgrep -f <literal>`
  **always matches the asker**, because the pattern is in the searching command's own
  `/proc/PID/cmdline`. Twice it made a **finished** boot read as still-running for minutes.
  `boot_capture.sh` documents this *inside* the script; it applies just as much to anyone
  driving the bench from outside it. Fix = the bracket trick: `pgrep -f '[b]uild_qom_shim'`.
  ⚠ Note the two failures are opposite: one **never** fires, one **always** does — so a waiter
  and a "nothing is running" check need different fixes, and neither is safe by default.
- ★★ ⊘ **CORRECTED 2026-08-08 — this trap is HARNESS-SPECIFIC and does NOT apply to the Rust
  bench.** The `~/.ssh/config` mapping `localhost`/`127.0.0.1` to the guest key matters only for
  the ~30 `scripts/mode2_diag/*_host.sh` that run a **bare** `ssh -p 2223 ubuntu@localhost`.
  ⊘ **On the kayfabe bench `vh` there is no such file and none is needed** — `gssh_nv` reaches the
  guest as **`ubuntu@192.168.77.2` over the tap**, not `localhost:2223`. Repeating the ssh-config
  advice there sends people to fix a file that was never in the path.
  ★ What DOES still hold, and is what actually reads as "never booted": the guest needs
  **~20–25 s** to reach a login prompt, `-serial file:` output **lags**, and ⊘ **`nvktap0` does
  not survive a host reboot** while QEMU requires it to pre-exist — a guest can be sitting at a
  login prompt while the harness reports it never answered. A slow boot is not a crash.

⚠ **Any bench claim must carry the SOURCE REVISION it was measured at.** The bench silently
served a binary built from `862c7c2` for weeks — every newer revision failed
`-Werror=redundant-decls` on a duplicate forward declaration — so results attributed to HEAD
were not HEAD's.

## Source layout

- `src/qemu/` — the QEMU-side device. `nvkvm_gpu_emul.c` = Mode-2 emulated GPU + fake GSP; also the
  Mode-1 forwarding stack (`virtio_nvgpu.*`, isolate API). This is the big one.
- `src/stub/` — the unprivileged **host stub / isolate** that issues real host RM ioctls.
- `src/guest/` — guest kernel module (Mode-1 path).
- `src/abi/`, `src/common/` — NVIDIA ABI structs / shared helpers.
- `tests/` — `mode2/`, `integration/`, `perf/`, `abi_parity/`, `security/`, `unit/`.
- `scripts/` — `run_mode2_vm.sh` (boot a Mode-2 guest), `mode2_iter.sh` (build→boot→test on host),
  `run_test_vm.sh` (Mode-1), `mode2_diag/`, vast.ai helpers.

## Design docs (`docs/design/`)

★★★ **DOC HYGIENE — three rules, each paid for (2026-08-11).** This tree's most expensive recurring
failure is **not** a missing document; it is **a correct document that stopped being true and did not
say so**. Measured five times in two days, including a ruling superseded **the next day** that sent
two bench lanes at work already proved unnecessary, and a doc committed **the day before** that
already held the answer a rung then re-derived wrongly.
- ★ **Every design doc opens with a dated `STATUS` block.** LIVE / SUPERSEDED-BY / ANSWERED /
  DESIGN-ONLY. A doc with no status reads as current forever.
- ★★ **A correction FOLDS INTO its parent, above the thing it corrects — never beside it.** A
  correction living as its own file leaves the parent reading as current and requires the reader to
  already know the correction exists. That is the exact shape that cost the two lanes.
- ⊘ **Record supersession IN the superseded text, not only in the successor.** Nobody reads forward
  from a stale doc. ⇒ **A ruling's DATE and its ARCHITECTURE are both part of the citation**: ask
  *why* it decided that, and whether the why survives today's design.


- **Forwarding model (read first for Mode-2):** `mode2_forwarding_model.md` — translate guest
  *intent* to unprivileged host userspace ops; never replay privileged GSP-internal controls;
  correctness = observable end-states only.
- **★ Address table (the data-plane core, read with the forwarding model):** `mode2_address_table.md`
  — one authoritative per-VAS VA→GPGA table, forward-populated, never reverse-resolved; the table
  IS the guest's TLB; miss = fault.
  ⚠ **Two CO-EQUAL populate sources, and NOT "RPC + read-at-invalidate"** — that phrasing was
  refuted by §5's ★ CORRECTION (2026-07-22, audit S3) and must not be repeated: on the **Mode-2
  GSP-emulated compute path both invalidate transports measured ZERO** (`INVALIDATE_TLB` RPC
  fn=200 = 0; `MEM_OP`/`MMU_TLB_INVALIDATE` pushbuffer method = 0), as did `DMA_FILL_PTE_MEM`.
  The sources are **(1)** bind-time RPC/ioctl bindings and **(2)** the **observed CE page-table
  write**, attributed by destination-FB-address → owning PDB and latched at the **CE release
  semaphore** — the commit point that *replaces* the absent invalidate. Read-at-invalidate still
  governs the kernel/UVM/RM paths, where the transports do appear. `mode2_2nd_context_hang.md`
  = the #12 bug it dissolves (GSP-managed CE channel finishPayload, root-caused).
- Mode-2 compute path: `mode2_compute_forwarding.md`, `mode2_gr_forwarding.md`,
  `mode2_cuctxcreate_resume.md` (+ `_problem.md`), `mode2_execfwd_keystone_plan.md`.
- Mode-2 internals: `mode2_memory_model.md`, `mode2_address_virtualization.md`,
  `mode2_bar2_mmu.md`, `mode2_device_data_model.md`, `mode2_m3_gsp_rpc.md`,
  `mode2_dataplane_architecture.md`, `mode2_doorbell_chid.md`, `mode2_interrupt_delivery.md`,
  `mode2_uvm_residency.md`, `mode2_multiprocess_isolate.md` (per-process page-table-publication
  isolate — the multi-process/#14 design, deferred to the Rust rewrite), `nvidia_gpu_internals.md`.
- Feasibility / strategy: `device_simulation_feasibility.md`, `mode2_plan.md`.
- Mode-1 / shared: `mode1_poll_relay_plan.md`, `command_buffer.md`, `gpa_window_pci_bar.md`,
  `async_event_delivery.md`, `virtual_modeset.md`, `signal_interrupt_delivery.md`.

## Other docs

- Security: `docs/SECURITY_MODEL.md`, `docs/HARDENING_PLAN.md`, `docs/audits/`.
- Status/plans: `docs/MILESTONES.md`, `docs/PARITY_PLAN.md`, `docs/PRE_PUBLIC_CHECKLIST.md`,
  `docs/REFACTOR_PLAN.md`.
- Agent workflow & token-cost strategy: `docs/WORKFLOW_STRATEGY.md` (serialize the bench, fan out
  read-only analysis, keep `MEMORY.md` lean, treat each debug episode as restartable-from-disk).
- Kernel patches applied to the guest driver for Mode-2 bring-up: `docs/kernel_patches/`.
- Reference / RE notes: `docs/reference/`, `docs/research/`, `notes/`.

## Reference material (read before guessing driver semantics)

- gVisor `nvproxy` (vendored under `gvisor/`) — the canonical ioctl-dispatch / pointer-translation
  reference.
- Open NVIDIA kernel modules under `research_clones/` (e.g. `ogkm/`) — ground truth for RM
  semantics; the open driver is stricter, treat it as canonical. Both closed + open drivers must
  work.

## ★★★ The LIVE oracle — `tests/mode2/nvdiff/` + `traces/host_reference_ga106/`

★ Every other oracle here is **static** — recorded captures answer only the questions someone thought
to record. This one is **queryable**: the same CUDA program traced on a **real host** and in **our
guest**, diffed. It replaces *"I infer the wall"* with *"we diverge at record N, on this id."*

- `nvdiff_shim.c` — `LD_PRELOAD` recorder; every `/dev/nvidia*` ioctl with the parameter buffer
  **on both sides of the call**. ⊘ `strace` is **not** a floor here: without a before/after pair you
  cannot tell *"RM wrote nothing"* from *"we didn't capture the reply"*.
- `nvdiff.py` — align + classify (MISSING / EXTRA / SIZE / STATUS / VALUE). `nvd_prog.c` — staged
  workload (`ce` is the `cup2` shape). `scripts/mode2_diag/nvdiff_run_guest.sh` — the guest half.
- **Noise floor MEASURED ZERO** over 12 pairings / 6 stages, and `nvd_selftest.sh` checks the differ
  for **detection** offline with no GPU: it must find exactly **479** divergences (`dev` vs `ctx`) and
  **5** (`ctx` vs `alloc`). ⚠ Run it before trusting any diff.

**What it has already established, all measured:**
- The guest runs in **lockstep** with hardware to `UVM_MAP_EXTERNAL_ALLOCATION` — **221 of
  `cuCtxCreate`'s 479 ioctls (46.1 %)** — then calls `0x20801702` ×175 until killed. ★ Hardware calls
  that id **zero times** in the whole program.
- ★★ **Hardware returns non-OK EXACTLY ONCE in 613 records** (`0x2080012f`, in `cuInit`). So *"`0x56`
  is the forgiven status"* is no longer a heuristic — **every other `0x56` we emit is a divergence**.
- **A kernel launch costs zero RM ioctls** — the `launch` and `ce` stages are byte-identical. ⊘ This
  oracle bounds the **control plane only**; a green diff says nothing about doorbells or completions.
- `GPU_GET_NAME_STRING` returns the right size and **23 zero bytes** where hardware returns
  `"NVIDIA GeForce RTX 3060"` — which is why `nvidia-smi` prints `ERR!` in the Name column.

⚠ **Two rules it taught, both paid for:**
- ★ **RANK DIVERGENCES BY KIND, NEVER BY INDEX.** The first *by index* was `CARD_INFO` — environmental,
  because the reference host is a **five-GPU rig** running the **closed** driver.
- ★ **An ioctl number is not a length because its bits parse as one.** Trusting `_IOC_SIZE` produced
  **2672** phantom divergences: nvidia-uvm numbers are raw integers, and `UVM_INITIALIZE = 0x30000001`
  decodes as `size = 12288` against a **16-byte** struct — so ~500 bytes of unrelated stack were being
  recorded as "the parameter" and diffed.

## Reference traces (the oracle's output — `traces/mode2_c_reference/`)

| capture | records | hermetic? | what it is for |
|---|---|---|---|
| `cap1_coldboot_hermetic` | 359 062 | **yes** (`m2fwd=off m2exec=off m2romregs=off`) | the only **closeable** replay; PCI enumerate → 139 821 PROM/VBIOS reads → FWSEC/WPR2 → LibOS args → msgq → `GSP_INIT_DONE` |
| `cap2_stalequeue_negative` | 886 999 | no | the `WPR2-already-up` chain |
| `cap2b_stalequeue_nofn47` | 862 940 | no | ★ **the real negative** — 378 GSP elements parsed from arbitrary guest RAM, answered `NV_OK` |
| `cap3_matmul_forwarding` | 532 824 | no | decision planes; `cup8` at `bad=0 maxerr=0` |

★★★★★ **CORRECTION 2026-08-12 — THE FIFTH LIMIT BELOW UNDERCOUNTS, AND THE UNDERCOUNTED ROWS ARE
THE DANGEROUS ONES.** Re-derived from the header itself (`{cmd, status, psize, dlen, data}`, 56
rows): **11 EMPTY (`dlen=0`), 16 TRUNCATED (`0 < dlen < psize`), 29 COMPLETE.** ⇒ The text below
credits **45 rows** as *"carrying a body"* and matching byte for byte — **16 of those 45 are
partial**, and a partial row is not a body. Only **29/56 (51.8 %)** are complete.
⊘ **`dlen` is the CAPTURED length; `psize` is what RM would return. A short row decodes to
whatever follows in the struct — usually zeros — with no marker distinguishing it from a real
value.** Worst cases: `0x20800a22` missing **18 216 of 34 592** bytes, `0x20800a40` missing
**8 196 of 24 580**, `0x20800b03` missing **8 160 of 16 352**.
★ It has already bitten, **on display**: `0x20800a01` is `psize=36 dlen=32`, and
`numDispChannels` lives in the **missing 4 bytes** — read as 0, producing a NULL
`clientChannelTable`. And `0x20800a4b`, which **selects the whole display HAL**, is both an
empty row *and* already contradicted by a real GA106.
⇒ **The trustworthy set is `dlen >= psize`, not `dlen > 0`.** Refuse both empty *and* short rows
as **unmeasured**; never let a short row's tail decode to zeros.

★★★ **FIFTH LIMIT, and it is different in kind — the oracle is POSITIVELY WRONG here, not blind**
(measured 2026-08-01 against a real GA106, `../nvkvm-rs/traces/real_ga106/`). The captured control
table `src/qemu/mode2_initctrl_ga106.h` has **56 rows, of which 11 (19.6%) carry `dlen = 0`** — the
reply body was never captured. **Every `dlen=0` row checked against real hardware is CONTRADICTED**
(`0x20802a08`, `0x20802a06`, `0x2080017e`, `0x20800af3`, `0x20800a4b`, `0x20800aac`), while **every
row carrying a body matches BYTE FOR BYTE**.
⇒ `0x20802a08` (`CE_GET_FAULT_METHOD_BUFFER_SIZE`) decodes from its empty row as **size 0**; a real
GA106 returns **20480**. RM DMAs CE fault records into a buffer of exactly that size, so trusting the
empty row was a **buffer overrun with a hardware writer**, not merely a wrong number.
⊘ **An empty capture is evidence of NOTHING, not evidence of emptiness.** Treat `dlen=0` as
*unmeasured* and refuse it; do not decode it to zeros. ★ And note how it survived: a gate demanding
a `C:` citation was **satisfied** by a row that cited the empty body *as corroboration* — **citing
the oracle is not the oracle being right.** A citation gate checks a claim is *sourced*, never that
the source says what the claim says.
★ This **scopes** the oracle rather than devaluing it: the rows that were captured *in full* matched
exactly. The oracle is trustworthy precisely where it captured something — but see the 2026-08-12
correction above for what *"captured something"* actually means: **29 rows, not 45.**

★ **Four further measured limits before trusting any diff** (full text in
`../nvkvm-rs/docs/design/c_rust_trace_differential.md`): the **completion plane has NO C
oracle** — the C *forges* completions, so a green diff says nothing about it; **the diff can
never be green end-to-end** because the C has no refusal vocabulary; **forwarding-mode traces
are non-hermetic by construction** (`pci_dma_map` is an uninstrumented channel — the host GPU
DMAs into guest RAM behind every recorder); and `IrqRaise == 1` across the whole of `cap1`
with **zero** `IRQSCLR` writes, so event delivery is gated off after `INIT_DONE`.

## Working notes (conventions, not commands)

- Mode-2 dev loop runs on a remote GPU host (vast.ai): edit locally → sync to the host build tree
  → rebuild QEMU → fresh-boot the guest → run. The emulated GSP's WPR2 state only resets on a full
  QEMU restart, so **each clean run needs a fresh boot**, and GPU tests run **strictly serially**.
- The repo also carries persistent agent memory under
  `/root/.claude/projects/-workspace-nvidia-gpu-passthrough/memory/` (index `MEMORY.md`) with the
  live state of in-flight work — consult it for "where things are right now."
