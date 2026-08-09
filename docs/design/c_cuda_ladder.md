# The C's CUDA ladder — every wall between a working `nvidia-smi` and a correct matmul

**Purpose.** kayfabe is one wall from `nvidia-smi` (the `memmgrTestCeUtils` CE copy). The wall
after that is the CUDA ladder, and the C research artifact already climbed all of it, up to
`cuCtxCreate → 2048² matmul at bad=0 maxerr=0` on a stock guest. This file is the ordered wall
list, the rung table, and the run recipe, so the Rust does not rediscover any of it. It records
the evidence and the ordering only — it proposes no Rust design beyond §6's inherit-vs-precluded
verdict.

**⊘ No new claims.** Every line is tagged `[measured]` (the C ran it — a bench date / capture /
driver is named), `[src]` (read from code), or `[inferred]` (the record reasons but did not run
it). A citation into source is a **reading**, never a measurement. Where the C's own record
contradicts itself, that contradiction is reported rather than resolved.

**Citation prefixes.**

| tag | tree |
|---|---|
| `C:` | `/workspace/nvidia-gpu-passthrough` (the C research artifact, branch `consolidation`) |
| `mem:` | `/root/.claude/projects/-workspace-nvidia-gpu-passthrough/memory/` (the C-era agent memory) |
| `rs:` | this repo (`/workspace/nvkvm-rs`) |

---

## 1. The rungs, in order

Six standalone driver-API probes form the ladder. Each is a foreground CUDA program that prints
where it stops; pick the next one by what the last one established.

| rung | program | what it establishes | first thing it fails on | minimum that must work to pass |
|---|---|---|---|---|
| R0 | `cup.c` | `cuInit → device → ctx → 4-byte CE memcpy round-trip`; prints where it stops | whatever blocks `cuCtxCreate` or the CE round-trip | ctx create + one CE HtoD/DtoH |
| R1 | `cup2.c` | the smoke gate: `cuInit`, enumerate (`RTX 3060`, cc 8.6, 11909 MiB), `cuCtxCreate`, `cuMemAlloc(4096)`, `cuMemcpyHtoD/DtoH` 4 B, assert `rv==0xabcd1234` | `cuCtxCreate`, then the CE round-trip | a real host CE copy completes and the byte comes back exact |
| R2 | `cup4.c` | first REAL GR compute: `N=16` fp32 matmul, one block, verified against a CPU reference | the GR/compute execution path (golden-ctx forwarding) | one forwarded GR kernel runs and its output lands in guest RAM |
| R3 | `cupctx2_min.c` | the context LIFECYCLE (`create → destroy → create`, `ITERS=2`, **no compute**) | the #12 2nd-context hang — CTX2 `cuCtxCreate` hangs, `rc=124` | teardown + recreate without corrupting GSP/UVM state across the boundary |
| R4 | `cup8.c` | matmul AT SCALE: `2048²` (default), 2D grid, PTX JIT, closed-form check `bad=0 maxerr=0` | `cuModuleLoadData` (needs the PTX JIT lib), then map-on-touch of the full A/B/C working set | full GR compute forwarding with every leaf of the 48 MiB working set backed |
| R5 | `cup8_iter.c` | repeated matmuls (varying `N` 512…2048) in ONE context/module (`ITERS=5`) | the #13 multi-iter hang at `ITER 3` (`N=2048`), host `Xid 31 FAULT_PDE` | repeated alloc/free/launch/sync in one context with no stale/unbacked mapping |

`[src]` R0–R5 read from `C: tests/mode2/cup.c`, `cup2.c`, `cup4.c`, `cupctx2_min.c`, `cup8.c`,
`cup8_iter.c`. (`cup3.c`, `cup5.c`, `cup6.c`, `cup7.c` and the full `cupctx2.c` — the same
lifecycle *with* compute — also exist as intermediate steps.)

**How to pick the next one.** `cup2` proves the control plane + a CE round-trip but touches no GR
engine. `cup4`/`cup8` are the first programs whose result is *un-forgeable* — the CE software path
cannot multiply and sum, so a green matmul means the host GR engine really ran (`[src]`
`C: tests/mode2/cup4.c` header). `cupctx2_min` is the *only* rung that strips compute out entirely,
to decide whether a hang needs CTX1 to have done compute first or reproduces from a bare
create/destroy cycle (`[src]` `C: tests/mode2/cupctx2_min.c` header). `cup8_iter` reuses one
context to stress repeated launch/alloc/free (`[src]` `C: tests/mode2/cup8_iter.c` header).

**⚠ `cup8_iter` is NOT a valid acceptance oracle — the C's own record says so three times, and
disagrees with itself.** The bench log first recorded, on 2026-07-19, "cup8_iter rc=0 (#13, 5/5
iters) … #13 stays fixed" (`[measured]` `C: docs/BENCH_REBUILD_NOTES.md:475-476`). Task #95 on
2026-07-29 re-measured the bit-identical binary and got **1 PASS / 2 HANG**, and struck the earlier
line as "a single lucky sample" (`[measured]` `C: docs/BENCH_REBUILD_NOTES.md:180-196`). Task #104
on 2026-07-30 then got **9/9 PASS** on that same binary and concluded #13 is "**NOT reproducible on
demand, root cause UNKNOWN**" (`[measured]` `C: docs/BENCH_REBUILD_NOTES.md:67-87`;
`mem: mode2_13_multiiter_idle_hang.md` header). The durable finding across all three: any hardware
acceptance criterion must be *deterministic* (byte-exactness, or an assertion about what got mapped
where), never the absence of an intermittent hang.

---

## 2. Every wall between `cuInit` and first compute, in the order encountered

`cuInit` + device enumeration + attributes + `cuDeviceTotalMem` all PASS early; the guest reaches
`cuCtxCreate`, and every wall below is inside or just past it
(`[measured]` `C: docs/design/mode2_cuctxcreate_problem.md:20-28`, 2026-06-06).

### Wall A — the rbp-clobber SIGSEGV in `cuCtxCreate`

- **How it presented.** libcuda SIGSEGVs on an internal worker thread at the instruction
  `mov -0x38(%rbp),%rax` with **`rbp = 0`**; strace shows `si_addr = 0xffffffffffffffc8` (= −0x38).
  A frame-pointer / stack corruption, not a clean error return, immediately after a **successful**
  `RM_ALLOC` of the GR compute object (class `0xc7c0`, `AMPERE_COMPUTE_B`)
  (`[measured]` `C: docs/design/mode2_cuctxcreate_problem.md:22-28`, 2026-06-06).
- **What it actually was.** `[measured]` The `0xc7c0` alloc **reply** zeroed `pAllocParms` bytes
  8–15 where the host preserves them (`C: docs/design/mode2_cuctxcreate_resume.md:265-281`,
  2026-06-10, byte-diff of host vs guest `0xc7c0` reply on the `consolidation` branch).
- ⊘ **CORRECTED 2026-08-09 — the MECHANISM this entry used to state was WRONG, and it pointed
  porters at the wrong field.** The retracted text read: *"the reply set semantic `paramsSize=0` but
  left the RPC element SHORT, so the guest zero-padded its local params buffer"*, tagged
  `[measured]`. It was a **reconstruction**, and both halves are refuted against driver source:
  - ⊘ **`paramsSize` is INERT on this path.** `ogkm-580: src/nvidia/src/kernel/vgpu/rpc.c:11177`
    takes the size from the **class** (`rmapiGetClassAllocParamSize`), and `:11237-11241` copies
    `class_size` bytes from the reply payload **without ever reading `rpc_params->paramsSize`**
    (identical `ogkm-610: :11043-11047`). `serverDeserializeAllocUp` returns `NV_OK` without
    touching it on the non-serialized path.
  - ⊘ **The element was never SHORT.** `rpc_length = 32 + payload` = 64, element header 48 ⇒ 112
    bytes; with params echoed, 128. **Both are one element of 4096** — the element count does not
    change. Receive copies whole elements (`ogkm-580: message_queue_cpu.c:648-650`) and
    `rpc.length` is only a checksum extent (`:680-682`).
  ⇒ ★ **The real invariant is simply: supply `class_size` valid bytes at the params offset.** The
  crash came from handing over a **zero-filled** window, not from a length.
  ⚠ The *"those bytes held libcuda's saved `rbp`"* story is `[inferred]`, not measured: the SIGSEGV
  address and `rbp=0` are gdb observations, but that bytes 8–15 **are** the saved-rbp slot requires
  libcuda's stack buffer to be <16 bytes, which nothing measures.
- **What fixed it.** Fix **M8.4** shipped two changes together: `memcpy(resp+112, cmd+112, req_psize)`
  **and** an element-length store. ★ Only the **memcpy** is load-bearing; the length store is inert
  (tidy — it brings the params under the guest's checksum). `[measured]` cup2 then no longer
  segfaults in `cuCtxCreate` (`C: docs/design/mode2_cuctxcreate_resume.md:277-281`, 2026-06-10) —
  ⚠ but **post hoc**, and since both changes landed together it discriminates neither.
- ⊘ **Do not trust this entry's provenance in the C.** `nvkvm_gpu_emul.c` carries **four mutually
  exclusive `PROVEN` stories** for this one symptom, in two directly-negating pairs (`:3440-3446`
  *"drop the params"* vs `:3012-3023` *"keep the params"*; M7 `:680-684` *"paramsSize→0 was moot"*
  vs M8.1 `:3000-3010` *"paramsSize=0 PROVED cuCtxCreate succeeds"*). Driver source settles it
  against M8.1. See `isolate_founding_rationale.md` §1c for the same failure in another subsystem
  the same night.
- ★ **Open rider, not fixed by M8.4**: echoing is a **restore, not a fill**.
  `NV_GR_ALLOCATION_PARAMETERS.caps` (offset 12) is an **OUT** field, and `0xc56f` / `0x83de` /
  `0x0079` are `RS_REQUIRED` with out-fields too. Echoing hands the guest **stale IN values where
  hardware writes results** — it stops the crash; it does not make the alloc correct.
- **★ The record self-corrects here.** An earlier diagnosis (`mode2_cuctxcreate_resume.md` §0.2)
  pinned the crash to a GR page-table poll; §0.5 (2026-06-10) then overturns that as **post-crash
  teardown**, not a live poll — the FREE-storm after the `0xc7c0` echo is the guest tearing down
  the crashed process, and the real blocker is the `rbp=0` clobber upstream
  (`[measured]` `C: docs/design/mode2_cuctxcreate_resume.md:234-263`, 2026-06-10). Read §0.5 before
  trusting any "GR page-table poll" framing of Wall A.

### Wall B — the bare `999` (`CUDA_ERROR_UNKNOWN`)

- **How it presented.** After the crash fix, `cuCtxCreate` progresses then returns CUDA error
  **999**; the guest allocates class `0x0041` (vidmem) instead of `0xc7b5` (`AMPERE_DMA_COPY_B`,
  the context's copy-engine object), then does an `RM_FREE`/`DUP` burst + `PERF_BOOST` = the bail
  path (`[measured]` `mem: mode2_cuctxcreate_999_diagnosis.md:18-27`, 2026-06-04, LD_PRELOAD ioctl
  trace of cup2 host-vs-guest).
- **What it actually was.** Two observable control-OUTPUT diffs from the host, both copy-engine
  related, both served guest-local from the GSP static-info cache: **(1)** `GPU_GET_ENGINES_V2`
  (`0x20800170`) returned 10 engines — a spurious `CE4` (`engine 0x0d`) — vs the host's 9; **(2)**
  `CE_GET_ALL_CAPS` (`0x20802a0a`) returned all zeros vs host `{0x3e3,0x3e3,0x3e2,0x3e2}`. libcuda
  saw no usable copy engine → skipped the `0xc7b5` copy-engine object → bailed 999
  (`[measured]` `mem: mode2_cuctxcreate_999_diagnosis.md:26-42`, 2026-06-04).
- **What fixed it.** Diff #1: drop `CE4` from the replayed device-info table so `GET_ENGINES_V2`
  returns 9 (committed `fe49ffc`). Diff #2: the zeros came from a truncated/zero `ceCaps[]` region
  in the replayed `GspStaticConfigInfo` blob (served on GSP RPC `fn=65`); populating `ceCaps[]` +
  the CE-present bits with host values cleared the 999 (`[measured]`
  `mem: mode2_cuctxcreate_999_diagnosis.md:51-86`, 2026-06-04, debug-injected host CE caps →
  `cuCtxCreate` no longer 999). `[inferred]` this is the same family as the empty-capture defect
  the standing oracle warns about — a zero-filled row is *unmeasured*, not zero
  (`rs: docs/design/c_rust_trace_differential.md`).

### Wall C — the `MC_SERVICE_INTERRUPTS` completion poll (`0x20801702`)

- **How it presented.** With CE caps fixed, `cuCtxCreate` hangs in a tight poll on
  `NV2080_CTRL_CMD_MC_SERVICE_INTERRUPTS` (`0x20801702`), `psize=4`, `IN=OUT=0xffffffff` forever;
  the host trace never calls it (`[measured]` `mem: mode2_cuctxcreate_999_diagnosis.md:99-104`,
  2026-06-04; `C: docs/design/mode2_cuctxcreate_resume.md:283-292`, 2026-06-10).
- **What it actually was.** libcuda submitted a context-finalization GPU op whose completion is
  signalled by a GPU **interrupt** the emulated device never raises → libcuda spins draining
  interrupts that never arrive (`[measured]` `mem: mode2_cuctxcreate_999_diagnosis.md:104-113`,
  2026-06-04).
- **What fixed it — and the C reversed its own fix.** The first fix (**M8.108**) faked the
  completion with a per-delivered-completion "service-zero" credit so the poll terminates
  (`[measured]` `C: docs/design/mode2_cuctxcreate_resume.md:283-292`, 2026-06-10). The C then
  **rejected that as a dead end** (green poll, no real work) and decided **route B — real
  completion**: forward the real GR/CE execution so the host raises a real interrupt, reuse the
  Mode-1 `#127` poll relay to deliver it (`[measured]` `C: docs/design/mode2_cuctxcreate_resume.md:294-321`,
  2026-06-10). This is the governing rule at `C: docs/design/mode2_cuctxcreate_resume.md:164-198`
  (§0.3): completion data (semaphores/fences/USERD) must be host-written, never stubbed, and the
  proof is a numerically-correct matmul, never a green guest log.

### Wall D — the GR golden-context / execution-forwarding keystone

- **How it presented.** The guest RM busy-walks its GR-VAS page tables via BAR2
  (`0x2f3392000 → 0x2efbc3000 → … → FB 0x2efa62000`) and polls a completion semaphore
  `0x2efbaf000` that never updates; libcuda spins (State=R, empty kernel stack)
  (`[measured]` `mem: mode2_grctx_privilege_wall.md:93-103`, 2026-06-05, CRASHWIN read-probe with
  `m2exec=on`).
- **What it actually was.** The semaphore never advances because the **host never RUNS** the
  submitted GR-init/scrubber work — the channel working set (incl. the semaphore page) is not
  mapped into the host channel VAS and the host doorbell is never rung. It is a COMPLETION /
  execution-forwarding gap, **not** golden-context-buffer content coherence (which is a later
  concern) (`[measured]` `mem: mode2_grctx_privilege_wall.md:100-108`, 2026-06-05).
- **What fixed it.** The keystone is EXECUTION forwarding: back + map the channel working set
  (semaphore page + pushbuffers) into the host GR channel VAS at the guest VAs and ring the
  doorbell, so the host GPU runs the work and writes the semaphore — this stays UNPRIVILEGED
  because GPFIFO places at `st=0x0` (`[measured]` `mem: mode2_grctx_privilege_wall.md:104-108`,
  2026-06-05). The general model is `C: docs/design/mode2_gr_forwarding.md:82-103` (phases B1–B5:
  golden-completion signal → address bridge → first forwarded compute → direct-map parity).

**Naming note.** The four walls map onto the memory index's shorthand "rbp-clobber (A),
page-table poll (D), GR-context privilege (D), and a bare 999 (B)"
(`mem: mode2_promote_ctx_and_uvm_wall.md`). The "page-table poll" and "GR-context privilege"
labels both point at Wall D; the page-table-poll framing was specifically corrected (Wall A note).

---

## 3. The GR / golden-context boundary

`[src]` The golden context is a **silicon** boundary: GR state is produced by FECS/GPCCS microcode
on real silicon, and the golden-image channel gate is `IS_GSP_CLIENT` (`kernel_graphics.c:478`,
"Nothing to do for non-GSPCLIENT") — so a GSP client creating and running the golden-image channel
is *correct behaviour*, not an artifact to suppress
(`C: docs/design/mode2_grctx_privilege_wall.md:110-119`, read against ogkm).

`[measured]` **What is forwarded, and at which call.** When the guest's GR compute-object alloc is
forwarded (the `0xc7c0` `AMPERE_COMPUTE_B` construct + GR channel bind, via `GSP_RM_ALLOC` `fn=103`
through the unprivileged Mode-1 stub), the **host** kernel-RM allocates AND self-maps its **own**
GR golden-context buffers into the host GR VAS at the *same guest GR VAs* (`0x120020000`…) — every
`back_and_map[ctx0..5]` returns `st=0x51 ALREADY-HOST-MAPPED`, while GPFIFO (which the host does
NOT auto-map) places at `st=0x0` (`mem: mode2_grctx_privilege_wall.md:13-27`, 2026-06-05, one
`m2exec` run on GA106 + 580). So the golden context is host-owned end-to-end and the guest never
runs its own GR engine — its golden buffer content may be garbage
(`C: docs/design/mode2_gr_forwarding.md:73-81`).

`[measured]` **What would happen if it were faked instead.** Two disjoint copies exist: the host's
valid ctx buffers (host GPU writes the golden ctx here) and the guest's blank double-mmap objects
(`GPGA → cpu_qva = zeros`). They are never connected; libcuda reads the guest side → stale/zero →
the `cuCtxCreate` `rbp=0` crash (`mem: mode2_grctx_privilege_wall.md:71-79`, 2026-06-05). The two
unprivileged escapes that could reconcile them — forwarding `PROMOTE_CTX` (`0x2080012b`) and
`GR_GET_CTX_BUFFER_INFO` — **both** return `st=0x1b INSUFFICIENT_PERMISSIONS`, because
`PROMOTE_CTX`'s flags (`0x10244` = `PRIVILEGED|ROUTE_TO_PHYSICAL|ROUTE_TO_VGPU_HOST|
GSP_PLUGIN_FOR_VGPU_GSP`) make it a kernel→GSP internal control, never a userspace API
(`mem: mode2_grctx_privilege_wall.md:30-36,61-69`, 2026-06-05, `st=0x1b` confirmed against ogkm
`nvstatuscodes.h:56`). ⇒ faking the golden context crashes; forwarding the GR execution to the host
is the only correct path, and it is why golden-ctx content coherence is a non-issue rather than a
wall.

`[src]` The Rust already encodes this as its Case-1/Case-2 split: the host kernel-RM builds and
self-promotes its own ctx on the engine-object forward, and `PROMOTE_CTX` is ack-only, never
replayed on an unprivileged isolate (`rs: docs/design/c_bug_regression_matrix.md:45`, row 24;
`rs: docs/design/gpu_promote_ctx.md` §0).

---

## 4. `GPU_PROMOTE_CTX` (`0x2080012b`)

`[src]` **What it carries.** A 560-byte params struct (16-entry max, `sizeof=560 align=8`) with a
9-field header and `promoteEntry[16]`, each 32 B: `{gpuPhysAddr, gpuVirtAddr, size, physAttr(1:0 =
aperture), bufferId(u16), bInitialize(u8), bNonmapped(u8)}`; the channel is named by `hChanClient`
(`rs: docs/design/gpu_promote_ctx.md` §1.2, pinned by compiling the vendored declarations, and
byte-identical at 580.159.04 and 610.43.02). It is the GSP-RM address-populating op for GR/compute
context buffers — the C snoops it into a per-client side-table so `nvkvm_chan_translate` resolves
GR/compute channel VAs (`[src]` `mem: mode2_promote_ctx_and_uvm_wall.md:19-25`, implemented at
commit `079feea`).

`[measured]` **⚠ CORRECTED: promote-ctx is NOT the gap to compute.** The Rust port's own design
doc records the correction on three grounds: (1) the host owns and self-maps the ranges promote-ctx
describes, so the `gpuPhysAddr` in a guest entry is a guest FB offset for a buffer the host never
touches; (2) for the client that actually crashed (`0xc1d00003`), it promoted **every** context
buffer NONMAPPED with `va=0` — zero table entries under any correct filter; (3) the compute working
set's leaf PTEs are published exclusively through the CE page-table-write data plane, with
`INVALIDATE_TLB` RPC = 0, `MMU_TLB_INVALIDATE` method = 0, `DMA_FILL_PTE_MEM` = 0 measured on the
Mode-2 GSP-emulated compute path (`rs: docs/design/gpu_promote_ctx.md` §0, citing
`C: docs/design/mode2_cuctxcreate_resume.md:210-213` and `C: docs/design/mode2_address_table.md:116-129`,
audit-S3, 2026-07-22). So promote-ctx is a narrow MISS=FAULT gap-filler for host-owned GR context
ranges, not the milestone.

`[src]` **The TWO BLOCKERS the port hit** (`rs: docs/design/gpu_promote_ctx.md` §4, §5):

- **Blocker 1 — the ABI generator cannot express the struct.** `promoteEntry[16]` is a fixed array
  of a nested struct; the codegen scalar table is deliberately closed and `ParseError::NestedAggregate`
  refuses nested bodies. Resolved by generating the all-scalar entry struct through the generator
  and hand-transcribing the 48-byte header into `transcribed.rs`, with the array as stride
  arithmetic (`rs: docs/design/gpu_promote_ctx.md` §4.1, §9.1).
- **Blocker 2 — the consumer is a new lock seam on top of in-flight L1-M1 lock work.** Harvesting a
  fact from an `AckOnly` control needs `&mut Proc` (a write-side act phase) and a resolved
  `(GpuId, Pdb)` from `hObject` in the `hChanClient` namespace — converting a read-lock fast path
  into a route/act/commit sequence that lands on the R1/R3/R5 invariants L1-M1 is mid-build
  (`rs: docs/design/gpu_promote_ctx.md` §5, §9.2).

`[src]` **The SEVEN C defects the port names and subtracts** (`rs: docs/design/gpu_promote_ctx.md`
§3, §9.4; the handler is `C: src/qemu/nvkvm_gpu_emul.c:2275-2306`):

| # | defect | class |
|---|---|---|
| D1 | `entryCount` clamped to 64 (comment says 20); the truth is **16** — an OOB read of 1536 B past the struct, inserted into `va_map[]` under the guest's own `hChanClient` on the hot resolve path (SECURITY, semantic injection) | subtracted; refuse `>16` + validate `paramsSize==560` |
| D2 | `bufferId` read 32-bit over a 16-bit field, so the value carries `bInitialize<<16 \| bNonmapped<<24` | subtracted; `u16` at +28, kept in the view |
| D3 | `!sz` silently swallows every promote-only (state-B) entry — 4 of 9 in the captured blob — a legitimate protocol case dropped without a name or count | subtracted; explicit 3-way classification, each non-bindable outcome named |
| D4 | aperture collapsed to a bool `(physAttr & 3) != 0`; the illegal value `3` mapped to sysmem | subtracted; total decode into `Aperture`, `3` a named refusal |
| D5 | `va_map[]` keyed on `hChanClient`, not PDB — the #12 root anti-pattern | subtracted by construction: the Rust key is the address space, never the client |
| D6 | silent table-full drop at 1024 entries (no log, no fault) | does not port: `AddressTable` has no capacity limit; the bound that exists is a loud refusal |
| D7 | the reply clobbers the guest's params with a foreign-boot capture and reports `NV_OK`, feeding stale `bInitialize` back into guest state (SECURITY) | subtracted; a Case-2 ACK writes back nothing |

`[measured]` Each of D1, D2, D3, D4, D7 has a regression test that was **seen to fail** when the fix
was poisoned; the bite ledger `scripts/bite_promote_ctx.py` plants 18 defect shapes and measured
16/18 firing at `rev 4a93d54`, both misses being findings (a one-proc world cannot witness a
wrong-owner bug; a projection-index `clear()` covered by nothing)
(`rs: docs/design/gpu_promote_ctx.md` §9.7, 2026-08-01).

---

## 5. UVM residency

**Is it required for `cup8` (a plain matmul), or only for managed-memory programs?**

`[src]` **NO — not for `cup8`.** UVM's fault/residency machinery only ever runs for **managed**
ranges (`cudaMallocManaged`). Explicit `cuMemAlloc` device memory — which is all `cup8`,
`cup8_iter`, and `cup2` use — is vidmem-resident, never faults, never migrates, and forwards
exactly like Mode-1. "So **UVM never blocks basic compute**; this doc is only about managed memory"
(`C: docs/design/mode2_uvm_residency.md:24-29`, DECIDED 2026-06-04).

`[src]` **What managed memory needs on this path** (the managed-memory programs only): the guest's
managed range is backed by a **host** `cudaMallocManaged` allocation; **host UVM owns residency**
and the **guest UVM is an inert fiction** held at static "sysmem-resident, GPU DMA-ing it"; the
guest CPU plays the host CPU's role, with its accesses arriving as EPT/NPT faults that host-side
UVM/HMM migration resolves through the same GPA (`C: docs/design/mode2_uvm_residency.md:31-56`).
The only genuinely new Mode-2 code is keeping the guest UVM quiescent — never honouring a
guest-side migrate-to-vidmem (`C: docs/design/mode2_uvm_residency.md:93-101`).

`[measured]` **Caveat from the bring-up record.** The `cup2` 4-byte CE round-trip and the early
`cuCtxCreate` passes on 2026-06-09 used a **debug guest-kernel UVM uprobe bridge** to supply the
managed source backing, explicitly "not a production fix"
(`C: docs/design/mode2_cuctxcreate_resume.md:323-361`). That bridge is orthogonal to `cup8`'s
explicit-device-memory path; it is called out only so a reader does not conclude managed memory was
production-clean when the ladder was climbed.

---

## 6. The C's own bug list on this path — inherit vs. precluded

The directive is "reproduce the C and SUBTRACT its named bugs." Each row states whether the Rust
would **inherit** the bug or its design already **precludes** it, and why. The authoritative
per-bug classification is `rs: docs/design/c_bug_regression_matrix.md` (decision #18B).

| C bug | what it was | verdict | why |
|---|---|---|---|
| **`dma_copy` missing `AMPERE_DMA_COPY` entries** → `engineType=0` → wrong runlist → the `401` (`CUDA_ERROR_ILLEGAL_STATE`) | the guest sanitizer's per-`hClass` size table had no `*_DMA_COPY_*` rows, so `ap_size` stayed 0, the aux buffer was never copied, and the kernel read `engineType=0` → `ENG_COPY(0)` → GR runlist → `GPFIFO_SCHEDULE` `NV_ERR_INVALID_STATE` | **PRECLUDED** (with a live successor concern) | the missing-table-entry *mechanism* lived in the C's hand-maintained guest module (`C: src/guest/nvkvm_main.c:1719-1723`; class list `C: src/abi/nvgpu.h:82-96`). The Rust models class alloc-params by **codegen ABI** (`kayfabe-abi`), so a missing/mis-sized row is a hard error, not a silent 0 (matrix row 25). ★ Do **not** read this as closing the topic: 2026-07-30 hardware measured that `engineType` *does* route and the **TSG group's** field is what routes, so engineType-routing correctness is a live `[src]` concern in the Rust work-submission stack — a different fact from this bug (`mem: dma_copy_class_alloc_params.md:77-105`) |
| **USERD wipe (#11)** | an emulator CE zero-fill whose FB span covered a live USERD page zeroed the ring `GP_PUT`/`GP_GET` → channel idle → hang | **split: state-half PRECLUDED/TESTED, content-half DEFERRED** | the parser is resolve-first, so an observed CE write never silently replaces live core state — tested by `cb11_ce_write_never_clobbers_live_binding`; the *byte-content* guard over a live USERD needs the FB-shadow/regs model and lands with that port (matrix row 1). The C fix was `nvkvm_fb_is_live_userd` (`C: src/qemu/nvkvm_gpu_emul.c:1316`) |
| **writeback / sanitizer-restore pattern** | the guest/stub sanitizer modified a request field for the host call but did not restore the caller's value on the writeback, so libcuda read sanitizer-internal state back | **PRECLUDED in core; DEFERRED for adapters** | the pure core holds no `#[repr(C)]`, no ioctls, no field-mutating sanitizer (grep-gated); the class re-arises only in `kayfabe-abi` and the L1 OS layer, with tests landing with those ports (matrix row 25). ★ Its exact species on THIS path — a reply clobbering the caller's params — is promote-ctx D7, already subtracted (`rs: docs/design/gpu_promote_ctx.md` §9.4) |
| **nvos64 ABI field-order fix** | `nvos64_parameters` had `pRightsRequested` after `paramsSize`; the stub read status at the wrong offset, hiding a `0x1f` failing alloc as `0x0` | **PRECLUDED** | the field order/truncation class is killed by Axis-A codegen: `kayfabe-abi` is generated from the open kmod declarations and diffed against the C's hand tables, so no hand-maintained `#[repr(C)]` order exists to get wrong (matrix row 25; `mem: nvos64_abi_fix.md`) |

**Inherit-vs-precluded verdict, in one line each:** `dma_copy` missing-entry — **precluded** by
codegen ABI (successor engineType-routing correctness is a separate live concern); USERD wipe —
**state-half precluded/tested, content-half deferred** to the FB-shadow port; writeback/restore —
**precluded** in the pure core, its on-path species already subtracted as promote-ctx D7; nvos64
field-order — **precluded** by codegen ABI.

---

## 7. How to RUN the ladder

`[src]` The run scripts are on the guest side and self-contained
(`C: scripts/mode2_diag/cup2_run_guest.sh`, `cup8_run_guest.sh`, `cupctx2_min_run_guest.sh`,
`cup8_iter_run_guest.sh`). Each does the same prep: `systemctl isolate multi-user.target`, `rmmod`,
copy firmware from the `nvfw` 9p share, `insmod nvidia.ko NVreg_EnableGpuFirmware=1
NVreg_RegistryDwords="RmGspBootRetryAttempts=1"` + `insmod nvidia-uvm.ko` from `/home/ubuntu/nvmods`,
`mknod` the device nodes, symlink `libcuda.so`, then `gcc -O0 -g` the test and run it under
`LD_LIBRARY_PATH=/usr/local/nvidia-guest/lib` bounded by `timeout`.

**Exact invocations.**

- **cup2:** `bash /tmp/cup2_run_guest.sh` (env `CUP2_TIMEOUT`, default 120 s). Pass = `rc=0` and the
  program prints `CE rv=0xabcd1234 want=0xabcd1234 -> PASS`
  (`C: scripts/mode2_diag/cup2_run_guest.sh`; `C: tests/mode2/cup2.c`).
- **cup8:** `CUP8_N=1024 bash /tmp/cup8_run_guest.sh` (default `CUP8_N=2048`, env `CUP8_TIMEOUT`
  default 180 s). Pass = `rc=0` and `CUP8 RESULT N=… bad=0 maxerr=0 … -> PASS`
  (`C: scripts/mode2_diag/cup8_run_guest.sh`; `C: tests/mode2/cup8.c`).

**What must be staged where.**

`[measured]` The runner and the `.c` must be scp'd to the guest `/tmp` **on every boot** — the boot
helper does `rm -f $OVL`, so the overlay is wiped each cycle (`C: docs/BENCH_REBUILD_NOTES.md:94-96`,
task #104, 2026-07-30). Each fresh overlay also lacks the unversioned
`/usr/lib/x86_64-linux-gnu/libcuda.so` that `gcc -lcuda` needs, so re-add the symlink after each
boot — the runner scripts do this, but a hand-run needs it (`C: docs/BENCH_REBUILD_NOTES.md:468-470`,
2026-07-19).

**Pass criterion.** `bad=0 maxerr=0 rc=0` for the matmuls, and byte-exact `CE rv=0xabcd1234` for
cup2. A green guest log alone is never accepted — the governing rule requires a numerically-correct
result through the forwarded path with real host GR utilization
(`C: docs/design/mode2_cuctxcreate_resume.md:182-198`, §0.3, 2026-06-10).

**⚠ Prerequisites the scripts assume.**

- `[measured]` **`~/nvmods/{nvidia,nvidia-uvm}.ko` must be rebuilt from source — NOT in the base
  image.** The runner insmods from `/home/ubuntu/nvmods`, which is populated by
  `C: scripts/mode2_diag/rebuild_guest_mods.sh` (mounts the `ogkm` 9p source, `make -j modules
  SYSSRC=…`, stages the two `.ko`). This was a documented recovery step after the overlay was lost
  (`C: docs/BENCH_REBUILD_NOTES.md:435-439`, 2026-07-19; `mem: mode2_promote_ctx_and_uvm_wall.md:125`).
- `[measured]` **The host must be on driver 580.159.04.** A 575 host deterministically hangs
  `cuCtxCreate`'s CE VAS resolution; 580 passes — the host-driver-version dependency is real and
  must match the known-good baseline (`C: docs/BENCH_REBUILD_NOTES.md:446-483`, 2026-07-19, RTX 3060
  GA106).
- `[measured]` **Fresh QEMU boot per GPU test.** The emulated GSP's WPR2 state resets only on a full
  QEMU restart; a second `cuInit`/context in the same boot commonly hits dirty GSP/WPR state
  (`C: docs/design/mode2_cuctxcreate_resume.md:16-18`, 2026-06-09; the bench runs GPU tests strictly
  serially).
- `[measured]` **cup8 needs `libnvidia-ptxjitcompiler` present in the guest lib dir** — it is
  REQUIRED for `cuModuleLoadData` of the PTX; the runner checks for it explicitly
  (`C: scripts/mode2_diag/cup8_run_guest.sh`, 2026-06-15).
- `[measured]` **The bench host needs a `~/.ssh/config`** mapping `localhost`/`127.0.0.1` to the
  guest key, because ~30 `*_host.sh` scripts run a bare `ssh -p 2223 ubuntu@localhost`; without it a
  healthy guest reads as "never booted" (`C: docs/BENCH_REBUILD_NOTES.md:262-290`, 2026-07-29).

---

## 8. Two standing self-contradictions in the C record (report, do not resolve)

`[measured]` **The guest was PATCHED during bring-up, then STOCK at reproduction.** The 2026-06-09
`cup2`/`cuCtxCreate` passes used a debug guest UVM uprobe bridge and a `0xFFF500` CE-sema backdoor
in `uvm_channel.c` (`C: docs/design/mode2_cuctxcreate_resume.md:323-361`;
`C: scripts/mode2_diag/rebuild_guest_mods.sh`). The 2026-07-29 ladder then ran ALL GREEN on a
**stock, unpatched** guest with `mode2_uvm_complete_proof.patch` explicitly NOT needed
(`C: docs/BENCH_REBUILD_NOTES.md:336-348`, task, 2026-07-29). Both are true of different dates: the
QEMU-side fixes (Walls A–D) superseded the guest backdoor. Cite the date, never the artifact.

`[measured]` **The `cuCtxCreate` crash diagnosis reversed itself twice** — pinned to a GR
page-table poll (§0.2), overturned to post-crash teardown (§0.5), then closed as the `rbp=0`
over-copy fixed by M8.4 (§0.6) (`C: docs/design/mode2_cuctxcreate_resume.md:105-281`, 2026-06-10).
The M8.4 diagnosis is the one that produced a live no-crash result; the earlier two are recorded so
their framings are not re-adopted.

---

*Provenance: `[src]`/`[measured]`/`[inferred]` per line. The C ladder programs, run scripts, and
bench logs are on the `consolidation` branch of the C artifact; the Rust-side facts are read from
this repo's `gpu_promote_ctx.md`, `c_bug_regression_matrix.md`, and `c_rust_trace_differential.md`.*
