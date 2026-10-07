# Mode-2 cuCtxCreate / UVM Dataplane Resume (2026-06-09)

This is the current handoff for branch `mode-2`. A fresh session with repo access and the `vh`/`vg`
SSH aliases can resume from this document alone.

## 0. Project Context

Mode-2 runs the stock open NVIDIA driver 580.159.04 inside a KVM/QEMU guest against the emulated
GA106 device `nvkvm-gpu-emul` in `src/qemu/nvkvm_gpu_emul.c`. QEMU forwards real GPU work to the
host RTX 3060 through the unprivileged isolate/stub path.

Standing constraints:

- Host GPU forwarding is the only supported Mode-2 path. Keep `m2fwd` and `m2exec` default-on.
- Host GPU tests are strictly serial. Kill old QEMU/stub processes before a new run.
- Use a fresh QEMU boot for each clean CUDA run. A second `nvidia.ko` load in the same VM commonly
  hits dirty GSP/WPR state and returns `cuInit 999`.
- The emulated GPU must be on q35 root slot `addr=0x7`.
- Debug plumbing is allowed for bring-up, but production fixes must not depend on LD_PRELOAD or a
  trusted guest userspace. Debug code should eventually be gated behind `NVKVM_MODE2_DEBUG`.
- If committing another milestone, update this file first.

## 0.1 Latest Checkpoint (2026-06-09)

Vast status: the scheduled VM host is usable. `ssh vh` reaches the RTX 3060 host
(`77.104.167.149:58385` at the time of this checkpoint), `/dev/kvm` is present, and the host driver
is `580.159.04`. `ssh vg` reaches the QEMU guest through the host proxy. No replacement instance is
needed unless this host disappears.

Current broad compute status:

- The 4-byte `cup2_pause` UVM proof still passes with the normal service-interrupt mask, no
  `LD_PRELOAD`, and no `NVUVM_SHADOW`, using the debug guest-kernel UVM bridge.
- Fresh-boot `scripts/mode2_diag/ctx_probe.c minimal` and `ctx_probe full` both pass in the guest
  with the debug UVM bridge loaded, default service-interrupt settings, and
  `NVKVM_M2_RUN_MAPDMA_SELFTEST` left off. The current `ctx_probe full` proof returns through
  `cuDeviceTotalMem` and `cuCtxCreate`:

```text
ok   cuDeviceTotalMem(&total, d)
totalMem=11909 MiB
ok   cuCtxCreate(&ctx, 0, d)
CTX OK
```

- The previous `ctx_probe full` blocker was a timeout in `cuCtxCreate` after device query and
  `cuDeviceTotalMem`. The syscall trace looped on `NV2080_CTRL_CMD_MC_SERVICE_INTERRUPTS`
  (`0x20801702`) with returned content `ffffffff`.
- Root cause of that full-ctx timeout was missing completion-drain credits after a burst of host
  completions. A diagnostic fresh boot with
  `NVKVM_M2_SERVICE_INTERRUPTS_HOST_ZERO_BUDGET=16` made `ctx_probe full` pass. The code now
  accumulates the host zero-service budget per delivered host completion, so the default path passes
  without the override.
- Gating the old mapdma/osdesc selftests exposed two production dependencies that used to happen as
  diagnostic side effects: doorbell setup needed the per-client GR mapper, and legacy per-client GR
  VAS still needed sysmem context buffers mapped there. The current code explicitly creates the GR
  mapper during doorbell setup and runs `M8.114 legacy GR sysmem prime` over newly snooped sysmem
  `va_map` rows without running the old sentinel selftests.
- A fresh-boot `NVKVM_M2_EVENT34_MASK=0` experiment forced host completion events through the 0x3c
  direct POST_EVENT path. QEMU posted 0x3c events/tokens, but `ctx_probe full` still timed out on the
  same `0x20801702 -> ffffffff` loop. Event tag selection alone is ruled out for the old ctx
  blocker.
- `matmul_pause 8` previously reached `cuLaunchKernel` and then timed out in `cuCtxSynchronize`,
  with QEMU logging plausible host-ring output words for C. After the latest selftest gate/CVAS
  routing and ctx-completion changes, the next target is a fresh-boot `matmul_pause 8`, then
  `matmul_pause 64`, then a real small LLM workload.
- `NVKVM_M2_SERVICE_INTERRUPTS_ZERO=1` was tried as a diagnostic and made the path worse; it is not a
  fix.
- `NVKVM_M2_POST_EVENT_PACKED_DATA=1` was tried as a diagnostic. It did not make `cuCtxCreate`
  return, and guest `nv_post_event` still saw `data_valid=0`, `info32=0`, `info16=0`.

Clean passing `ctx_probe minimal` trace fingerprint:

- The debug UVM bridge must be loaded. A no-bridge run is not a valid UVM dataplane test because QEMU
  never receives `UVM_MAP_EXTERNAL_ALLOCATION` backing records.
- The run records the initial UVM external ranges (`0x200000000`, `0x10000000000`,
  `0x10002000000`, high process VAs, `0x200200000`, and the `0x200400000` command ring), then gets
  past the previous stop around `0x204600000`/`0x204a00000`.
- The passing run continues through later UVM external allocations including `0x204c00000`,
  `0x204e00000`, high process VA ranges, and a larger high range before returning from
  `cuCtxCreate`.
- QEMU posts the first OS-event group (`0x3800000c`, `0x38000019`, `0x3800001a`) and then the
  native second group (`0x30000001`, `0x30000003`, `0x30000004`, `0x30000002`). `M8.107` suppresses
  duplicate local completion posts for the same token/event set while still allowing posts after new
  OS events are allocated.
- The repeated `0x120064000` GPFIFO lookahead rows were not all-zero work. The lookahead dumped
  entries after `GP_PUT`; the actual current entries were valid CE memset/scrub packets and QEMU
  released `0x12006c004` payloads. `M8.102` guards genuinely empty wrap-tail entries.
- Host dmesg still shows residual production debt around host RM/channel cleanup after process exit,
  including `kgspExecuteBooterUnloadIfNeeded_TU102: failed to execute Booter Unload: WPR2 is still
  up` and an `osinit.c:2363` assert. Treat those as follow-up unless they block matmul.

Active lead after this checkpoint: run `matmul_pause 8` from a fresh VM boot with the debug UVM
bridge loaded, then scale to `matmul_pause 64`, then a real small LLM workload. If matmul fails,
compare the full host run to the guest run using the same `libcuda` and test binary, including
non-ioctl syscalls (`mmap`, `poll`, BAR/userd mappings), not only RM ioctls. The visible current
suspects are forwarded `PROMOTE_CTX` failures (`st=0x1b`), high-UVM forwarded mappings that still
fall back to local/debug backing, and residual host `dmaAllocMapping_GM107` / Xid 32 from high-UVM CE
packets.

For Mode-2 production, keep the UVM rule strict: QEMU must not read guest userspace VAs;
`/tmp/m2_pbmap.txt`, pagemap export, and the uprobe bridge are debug-only. Production should consume
GPA/GPGA/GR-VA state and host RM mappings, with CR3 only as an opaque isolate identity.

## 0.2 consolidation-branch checkpoint (2026-06-10, Fable5/Opus session)

Restructure underway on branch `consolidation` (from clean base 41bd25c). Findings from
fresh-boot runs of the clean base (no uprobe bridge) on the RTX 3060 host:

- **M5.30 (committed, HW-validated):** capture the UVM vaspace page-directory root from
  `NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY` (0x801813) — previously echoed+discarded — into
  `chan_vas[]` with a `root_sys` flag, and extend the GMMU VER2 walker to accept a sysmem-rooted
  PDB (`nvkvm_walk_pdb_root`). This is the **production replacement for the uprobe-bridge UVM
  shadow**: a UVM device pointer now resolves VA→guest-GPA by walking the guest's own page tables
  (in guest RAM, via the GPA window), no guest-userspace read. Captured roots on hardware:
  hVASpace 0xcaf00005 PDB 0x3400000, 0xcaf00062 PDB 0x3401000 (both FB-rooted). NOTE: 0xcaf00005
  has a *different* root from RESERVED_PDES (0x3114000) vs SET_PAGE_DIRECTORY (0x3400000) — M5.30
  APPENDS the SET_PAGE_DIRECTORY root as an extra resolver candidate (non-destructive).

- **cuCtxCreate blocker PINNED (corrects §4 and the kickoff-doc "unbacked UVM" premise):** the
  clean base reaches cuCtxCreate and dies with `rbp=0` SIGSEGV at libcuda+0x300560 — but that is
  *downstream*. CRASHWIN (auto-armed at 0xc7c0) shows the guest RM busy-looping a GR-VAS page-table
  walk via BAR2 (`0x2f3392000→0x2efbc3000→4000→5000`, dual-PDE big→`0x2efbc6000`, PTE 0x2efbc61a0
  = 0x...2efa6201) to **FB 0x2efa62000** and polling it (100k-capped same-chain reads). This is the
  GR **golden-context content poll = §X Poll #2**. The M9 (CTRL-CLAMP) and M10 (ECC/NVLink
  NV_ERR_NOT_SUPPORTED 0x56) rbp-clobber fixes are ALREADY present on the clean base, so they are
  not the cause — the guest correctly reaches the golden-ctx poll and dies there.

- **Why it never clears on the clean base:** `nvkvm_m2_exec_doorbell` (M5.9, the real GR
  doorbell-forward that would map the GR ctx vidmem leaf into the host GR VAS so the host FECS
  fills the golden context) **fires 0 times**. The clean base instead runs the `chan_execute`
  *faking* path (38 DOORBELL / CE_SEM_RELEASE) which satisfies the **CE scrubber** sems
  (0x121xxxxx) by writing them itself — but the GR golden-ctx page is never written → poll spins →
  crash. The machinery to fix it (populate_cvas / leaf_flush GPGA double-mmap / exec_doorbell)
  EXISTS on the clean base but is not engaging for the GR channel.

- **Oracle mechanism map (from 7fb47f1, how it cleared this):** primary = **M7 R2 GPGA**
  (`nvkvm_m2_gpga_obj` + `populate_cvas` + `leaf_flush`): blank host vidmem object, double-mmapped
  (CPU overlay for the guest BAR2 read + FIXED map_dma into host GR VAS) so the real host FECS DMA-
  writes the golden ctx; triggered at the first GR doorbell. Fallback = **M8.45 reactive sempage
  release** (forge the captured GR report-sem payload into the poll page on first read). Neither
  depends on the uprobe bridge. The bridge (BAR0 0xFFF500-508) only serves the UVM CE-channel
  completion, NOT this GR poll.

- **ROOT CAUSE PINNED to one line:** `nvkvm_m2_populate_cvas` bails:
  `M5.28 populate_cvas: client=0xc1d00003 tsg=0x5c000012 — no own PDB (VAS not snooped yet);
  reactive map only`. `nvkvm_chan_own_pdb()` returns 0 for the **GR compute client 0xc1d00003**
  (its guest GR VAS→PDB linkage `chan_client → m2_devvas[].vas → chan_vas[].pdb` doesn't resolve),
  so the GR context leaves — including the golden-ctx page (FB 0x2efa6xxx) — are never enumerated
  and never GPGA-double-mmapped, so the host FECS never fills them. Everything ELSE is in place:
  M5.28 made the fresh per-channel VAS 0xce20002d, M5.8 allocated AMPERE_USERMODE_A + got GR
  work-submit token 0xc + GPFIFO_SCHEDULE'd TSG 0x5c000012 (ring deferred until pushbuffers mapped).
  M5.7 EXEC backed 3/6 FB working-set buffers. So the SINGLE missing piece is the GR client's
  own-PDB resolution feeding populate_cvas.

- **THE FIX (next increment):** make `nvkvm_chan_own_pdb` resolve the GR client 0xc1d00003's PDB —
  **M5.30 now captures SET_PAGE_DIRECTORY roots, which is exactly the missing PDB source**. Need to
  (a) confirm which captured root is the GR ctx VAS (the GR client's hVASpace; grmapper showed
  0xcaf00005 is client 0xc1d00001's VAS, so identify 0xc1d00003's), and (b) wire that PDB into
  chan_own_pdb / populate_cvas so the golden-ctx leaf gets enumerated + backed. Then the host FECS
  fills it via the existing GPGA double-mmap and the §X Poll #2 clears. This converts the
  "multi-week keystone" into a specific, bounded VAS→PDB-linkage fix.

## 0.3 GOVERNING RULE for map-vs-stub (user-set, 2026-06-10) — binding

Decide every emulated buffer/field by these rules, in order:
1. **Observable from guest userspace?** → MAP/forward the real host resource so observations match
   (this is the Mode-1 "forward, don't emulate" default). Applies *especially* to **completion data**
   (semaphores/fences/USERD GP_GET that libcuda reads to know work finished) — those MUST be
   host-written via the forwarded execution/semaphore path, never stubbed.
2. **Kernel↔hardware only (incl. GSP-managed), invisible to host AND guest userspace?** → may
   SIMULATE/stub, but ONLY if BOTH hold:
   - (a) the buffer is observable *only* from the guest NVIDIA kernel module (never exposed to host,
     never to guest userspace), AND
   - (b) whatever value is stubbed, the guest kernel module does not compute/process/propagate it in
     any way that reaches guest userspace.
   If either (a) or (b) fails, fall back to rule 1 (map the real thing).
3. **Context-switch / scheduling state** → always report "our task is scheduled/running/ready" — guest
   userspace cannot observe GPU scheduling (same as a vCPU thread's placement), and transparent
   pause/resume must not break apps.

Verification gate (proves rule-2(b) empirically): a stubbed value is only legitimate if a real
end-to-end matmul through the forwarded path returns the **numerically correct** result with real
host GR utilization. Correct result = the stub was content-irrelevant (legitimate). Wrong/idle =
content mattered → revert to map. Never accept a green guest log alone.

### Application to the cuCtxCreate poll (GR-VAS page-table population)
The clean-base blocker is the guest RM busy-walking its GR-VAS page tables waiting for the GR
ctx-buffer mappings GSP installs in GSP-client mode (small-page PDE at FB 0x2efbc5000 stuck at 0;
the target page 0x2efa62000 is NEVER read — only the PDEs/PTEs that map it). Per the rules:
- The **page tables / ctx-buffer content** are kernel↔hardware↔GSP, not host- or guest-userspace-
  observable, and the guest only checks the mapping *exists* (never reads the content behind it) →
  rule 2 applies: install/stub the missing guest GR-VAS PTEs so the walk stops. Use the **real**
  guest-phys from the PROMOTE_CTX snoop (VA→phys→bufferId) — i.e. write exactly the mapping GSP would
  have written — so it's correct-by-construction, not arbitrary.
- The **completion semaphores** (0x2efbaf000) and **USERD GP_GET** (0x420208c) the guest also reads
  are completion data → rule 1: keep them on the host-written/forwarded path, never stub.
- Verify with the matmul-correctness gate before declaring the ctx-buffer stub legitimate.

## 0.4 Source + instrumented determination (2026-06-10) — map vs stub for the GR poll

Read the open guest kernel (research_clones/ogkm) + one instrumented boot (M5.31 logging):

- **gmmu_walk.c:633** — for `IS_GSP_CLIENT`, the root-PDB update callback is a **noop** ("Noop
  inside a guest or CPU RM"); GSP/instance-block owns the root. The PDE/PTE *leaf* callbacks
  (`_gmmuWalkCBUpdatePde`, memmgrMemWrite) are CPU-side for *client* ctx-buffer maps
  (`kgraphicsMapCtxBuffer → dmaMapBuffer_HAL`, runs for GSP clients). So there are TWO populators:
  CPU-RM for client maps, GSP for the **golden-image channel** (`kernel_graphics.c:368`: "GSP_CLIENT
  creates the golden context channel GR post load").
- **PROMOTE_CTX bufferIds decoded** (low byte = type; `0x0001xxxx`=mapped, `0x0101xxxx`=NONMAPPED):
  the GR compute client `0xc1d00003` (the one that crashes) promotes its ctx buffers **NONMAPPED /
  va=0** (MAIN at phys 0x3e00000, sz 0xea000). The polled page `0x2efa62000` is **NOT** any client's
  PROMOTE buffer.
- **Instrumented GR-PT writes (M5.31):** post-0xc7c0 the guest writes **only zeros** (10310 writes,
  zero non-zero) into the polled page-table region (0x2efbc0000-0x2efbd000) — it CLEARS and then
  polls (~12k reads of the PDE/PTE chain). It DID write valid PTEs earlier (pre-probe, during VAS
  setup), proving it writes via the logged path when it intends to. So post-0xc7c0 it deliberately
  clears + **awaits an external fill**.

**CONCLUSION (map-vs-stub, per §0.3 rule):** the polled GR-VAS entries are **GSP-populated,
kernel-only vidmem page tables** — invisible to host and guest userspace; the guest only checks the
mapping exists (target page `0x2efa62000` never read). Condition (1) holds. → **legitimate stub/fill:
we (fake GSP) write the page-table entries GSP would populate for the golden-image/GR-init path.**
Verify condition (2) with the matmul-correctness gate. NOTE this is distinct from the *client* ctx
maps (CPU-RM-written, must be backed for real, not stubbed) and from the completion semaphore
0x2efbaf000 + USERD 0x420208c (completion data → forwarded/host-written, never stubbed).

**NEXT:** implement the GMMU VER2 page-table FILLER (write valid PDE/PTE entries into the guest's
GR-VAS that GSP would install) — bounded MMU-mechanics, the existing `nvkvm_walk_pdb_root` is the
read-side reference. Determine the exact awaited VA range/entries (the stuck small-page PDE
0x2efbc5000 + sub-table), fill them, boot, confirm the poll-spin stops, then drive matmul + verify
numeric correctness + host GR util. Instrumentation committed: M5.31 (GRPT-WR + PROMOTE bufId logs).

## 0.5 MAJOR CORRECTION (2026-06-10) — the "GR-VAS page-table poll" is POST-CRASH TEARDOWN

Verifying the Fable subagent's RPC-poll theory against the trace overturned BOTH it and the
long-standing "Poll #2 / GR-VAS page-table population" diagnosis (§0.2/0.4 and dataplane-doc §X):

- cup2 **segfaults DURING cuCtxCreate** (never prints CTX OK; rbp=0 at libcuda+0x300560).
- Immediately after the 0xc7c0 alloc echo (crashwin arm), the trace shows: `0x801814`
  **UNSET_PAGE_DIRECTORY** → a **storm of fn=10 RPCs** (fn=10 = NV_VGPU_MSG_FUNCTION_FREE; the clean
  base forwards it as RM_FREE @ gpu_emul.c:3565) → 100k GR-VAS page-table reads.
- That sequence (unset page dir + free-everything + walk-to-free page tables) is the guest kernel
  **tearing down the crashed process's resources** when its /dev/nvidia* fds close — NOT a poll the
  guest is blocked on. The 100k page-table reads are the teardown walk; the value-stability check
  (0x2efbaf000/0x2efbc5000/0x420208c all constant 0) is consistent with teardown, not a live poll.

**Consequence:** the real blocker is the **rbp=0 stack clobber DURING cuCtxCreate**, upstream of all
the page-table activity. The "install GR-VAS PTEs / golden-ctx fill / stub the poll" plan (§0.3-0.4)
targets the teardown and would do nothing. DEPRIORITIZE the page-table-fill path.

- The crash is a STACK CORRUPTION (gdb: crash at a function epilogue, saved-rbp popped as 0, a
  zero-run over the frame) — i.e. some control/alloc **reply copyout writes more zeros than
  libcuda's stack buffer holds**, clobbering the saved rbp. This is the M9/M10 family (CTRL-CLAMP +
  ECC/NVLink NOT_SUPPORTED) — those fixed two specific over-copies, but a **residual one remains**.

**NEXT (re-scoped, tractable):** find the specific RM control/alloc whose reply over-copies onto
libcuda's stack during cuCtxCreate (after the ECC/NVLink ones M10 already fixed). Method: host-vs-
guest diff of RM_ALLOC/RM_CONTROL reply sizes+bytes with nvioctl_trace (NVALLOC/NVCTRL dump) across
a native cup2_host run and the guest run; the divergent reply size is the clobber. Family:
[[abi_struct_truncation]] / [[nvos64_abi_fix]] / [[writeback_bug_pattern]]. NOTE: the M5.30 UVM-VAS
capture remains correct/useful for later (UVM device-ptr resolution post-ctx); it just isn't the
ctx blocker.

## 0.6 MILESTONE (2026-06-10): cuCtxCreate CRASH FIXED + VERIFIED (M8.4), next = MC_SERVICE_INTERRUPTS hang

The weeks-old cuCtxCreate `rbp=0` crash is **fixed and hardware-verified** on the `consolidation`
branch. Method that found it (no slop, all hardware-grounded):
- Captured RM_ALLOC reply bytes host (native cup2_host, real GPU, cuCtxCreate succeeds) vs guest
  (cup2 under nvioctl_trace) — `apre`/`areply`/`outer` dumps.
- Fable-5 byte-diff: the guest's `0xc7c0` (AMPERE_COMPUTE_B) alloc reply zeroes pAllocParms bytes
  8-15; the host preserves them. Those bytes held libcuda's saved rbp → `pop %rbp`=0 → SIGSEGV at
  libcuda+0x300560. Exact match to the gdb signature.
- Root mechanism: M8.1 set reply `paramsSize=0` but left the rpc element SHORT, so the guest
  GSP-client deserialize zero-padded its local params buffer and the guest RM's class-size (16B)
  copy_to_user wrote those zeros over libcuda's stack.
- **FIX = M8.4 (ported from oracle 7fb47f1):** keep the request params bytes in the response payload
  (`memcpy(resp+112, cmd+112, req_psize)`) AND extend the element length (`stl_le_p(resp+56,
  64+req_psize)`) so the copyout restores libcuda's stack; still report semantic paramsSize=0.
- **VERIFIED:** cup2 no longer segfaults during cuCtxCreate (`CUP2-ALIVE-HUNG`, no segfault, no
  FREE-storm teardown). Committed.

**NEXT BLOCKER (confirmed, expected):** cuCtxCreate now HANGS in the `MC_SERVICE_INTERRUPTS`
(0x20801702) poll loop — QEMU echoes `NV_OK+zeros` and the guest polls forever (118 occurrences).
This is the documented gate the oracle's **M8.108** (service-interrupt completion-credit accounting)
fixed: arm a "service-zero" credit per delivered completion, return it from the 0x20801702 handler
so the poll terminates. PORT M8.108 (oracle lines ~346-350 fields, ~1505-1580 arm/take helpers, +
the 0x20801702 handler), wired into the completion-delivery path. RULE CHECK before porting: confirm
the guest only POLLS the MC_SERVICE_INTERRUPTS result (kernel-internal interrupt bookkeeping, not
guest-userspace-observable) — if so it's a legitimate simulate per §0.3 rule-2/3; verify with the
matmul-correctness gate downstream. (oracle also has env overrides
NVKVM_M2_SERVICE_INTERRUPTS_HOST_ZERO_BUDGET for diagnosis.)

## 0.7 MC_SERVICE_INTERRUPTS direction DECIDED (2026-06-10): real completion via reused Mode-1 poll

User principle (governing): guest userspace does NOT use interrupts — it POLLS (eventfd / nvidia fd).
So for any op where a REAL host GPU would raise an interrupt, QEMU must POLL the corresponding host fd
and forward it; only skip polling if a real host GPU also wouldn't interrupt. Guard the race
(host-event-happened → poll-missed → guest-not-woken).

Investigation conclusions (confirmed against code):
- **Mode-1 #127 poll ABI is reusable as-is** for the polling half. Per isolate: ONE reader thread,
  ONE `ppoll()` over {control socket + armed os-event fds} (`nvkvm_stub.c:2780`), NOT thread-per-fd.
  Add-fd-while-polling works via control-socket wakeup (ISOLATE_CMD_POLL rebuilds pfds). One-shot
  re-arm. QEMU `nvkvm_virtio_push_evt` (`virtio_nvgpu.c:733`) serializes delivery via a BH; has
  level-triggered re-fire recovery if the evt queue is full. `nvkvm_isolate_poll`/`_unpoll`
  (`nvkvm_isolate.c:1991/2013`) is the hook. Mode-2 already uses isolates → can call these directly.
- **Mode-2-specific:** reuse the POLL half (arm host eventfds, ppoll, ISOLATE_RESP_POLL_EVENT), but
  the DELIVERY hop is the emulated GSP POST_EVENT (nvkvm_gpu_emul.c M8.38), NOT VQ_EVT (the stock
  guest has no nvkvm module). TODO: Fable-verify the host eventfd stays level-readable until consumed
  in the Mode-2 path (race-freedom).
- **Decision: route B (real completion), NOT the M8.108 credit-shortcut.** The shortcut fakes the
  completion without running the work = the oracle's dead end (green poll, no matmul). Per the rule,
  forward the real GR execution so the host raises the real interrupt.

**Keystone reduces to: engage execution-forward for the GR channel.** The missing links (resume §6,
0.2): `nvkvm_m2_exec_doorbell` (M5.9) fires 0x for GR because `nvkvm_m2_populate_cvas` bails
(`chan_own_pdb` returns 0 for GR client 0xc1d00003 — GR VAS PDB unresolved; M5.30 SET_PAGE_DIRECTORY
capture is the PDB source to wire in). Once exec-forward runs: host runs GR ctx-init → raises
completion on a host os-event eventfd → reuse #127 poll → POST_EVENT to guest → MC_SERVICE_INTERRUPTS
returns serviced for real → cuCtxCreate proceeds. Verify end-to-end with the matmul-correctness gate.

## 1. 4-Byte UVM Proof Status

The narrow `cup2_pause` CUDA proof reaches:

- `cuInit(0)` PASS.
- Device query path PASS (`RTX 3060`, compute 8.6, 11909 MiB).
- `cuCtxCreate` PASS.
- `cuMemAlloc` PASS.
- A 4-byte `cuMemcpyHtoD` / `cuMemcpyDtoH` round-trip PASS with no `LD_PRELOAD` and no
  `NVUVM_SHADOW`, using the guest-kernel debug UVM uprobe bridge:

```text
pid=2222
ok   cuCtxCreate(&ctx, 0, d)
CTX OK
ok   cuMemAlloc(&dp, 4096)
MEMALLOC OK 0x753d1e200000
ok   cuMemcpyHtoD(dp, &hv, 4)
HTOD OK dp=0x753d1e200000 sleeping-before-dtoh
ok   cuMemcpyDtoH(&rv, dp, 4)
CE rv=0xabcd1234 want=0xabcd1234 -> PASS
DONE
```

The important reframe: the old active blocker was `cuCtxCreate` crashing after `c7c0`. That crash is
not the current signature, and fresh-boot bridge-backed `ctx_probe minimal` and `ctx_probe full` now
both pass. Treat `cuCtxCreate` as sufficiently unblocked for the matmul loop, but not as
production-clean until the debug UVM bridge and residual high-UVM Xid path are removed.

Before the uprobe bridge, a no-shadow control run narrowed the final DtoH failure to:

- Destination staging sysmem resolved through pbmap.
- Source `dp` faulted because it is a UVM external allocation that QEMU does not own.

The passing no-`LD_PRELOAD` run proves that if QEMU can resolve the device pointer source to coherent
backing, the existing local CE copy path writes the correct bytes into the guest DtoH staging page.
This is still debug bring-up plumbing: the backing is created by a guest kernel uprobe module that
copies HtoD source bytes into guest RAM and reports them to QEMU through a BAR0 debug aperture.

## 2. Last Run Proof

Artifacts:

- `docs/design/mode2_traces/guest_uvm_uprobe_bridge_pass.txt`
- `docs/design/mode2_traces/qemu_uvm_uprobe_bridge_pass.txt`
- `docs/design/mode2_traces/host_uvm_uprobe_bridge_residual_xid.txt`

Guest run, with no `LD_PRELOAD` and no `NVUVM_SHADOW`:

```text
ok   cuCtxCreate(&ctx, 0, d)
CTX OK
ok   cuMemAlloc(&dp, 4096)
MEMALLOC OK 0x753d1e200000
ok   cuMemcpyHtoD(dp, &hv, 4)
HTOD OK dp=0x753d1e200000 sleeping-before-dtoh
ok   cuMemcpyDtoH(&rv, dp, 4)
CE rv=0xabcd1234 want=0xabcd1234 -> PASS
DONE
```

Guest kernel bridge proof:

```text
[   70.217075] nvkvm_uvm_bridge: mapped BAR0 debug aperture at 0000:00:07.0 start=0xfb000000
[   70.226711] nvkvm_uvm_bridge: registered cuMemcpyHtoD at /usr/local/nvidia-guest/lib/libcuda.so.580.159.04+0x378af0
[   70.226744] nvkvm_uvm_bridge: registered cuMemcpyHtoD_v2 at /usr/local/nvidia-guest/lib/libcuda.so.580.159.04+0x37aab0
[   70.226749] nvkvm_uvm_bridge: loaded max_bytes=4096
[  129.252353] nvkvm_uvm_bridge: HtoD dst=0x753d1e200000 bytes=0x4 gpa=0x13a989000 first=0xabcd1234 slot=1
```

QEMU CE proof:

```text
nvkvm-gpu[GA106] M8.14 UVM-SHADOW[0] VA=0x753d1e200000 GPA=0x13a989000 size=0x4 commit=0x1
nvkvm-gpu[GA106] M5:   COPY[0] src 0x13a989000(sys)=0xabcd1234 -> dst 0x13730d100(sys)
nvkvm-gpu[GA106] M5: CE COPY in=0x753d1e200000(virt) out=0x753d22800100(virt) bytes=4 const=0x0
```

Host dmesg still showed `dmaAllocMapping_GM107: can't alloc VA space for mapping` and Xid 32 from
other high-UVM CE packets during the run:

```text
NVRM: dmaAllocMapping_GM107: can't alloc VA space for mapping.
NVRM: Xid (PCI:0000:00:07): 32, pid=162582, name=nvkvm_stub, channel 0x01000008 intr 00800000
NVRM: Xid (PCI:0000:00:07): 32, pid=162582, name=nvkvm_stub, channel 0x01000008 intr1 00000004 HCE_DBG0 00000300 HCE_DBG1 04002186
NVRM: Xid (PCI:0000:00:07): 32, pid=162582, name=nvkvm_stub, channel 0x00000004 intr0 00000000 intr1 80000000
```

Treat the PASS as a proof of the missing UVM source backing, not a production-clean first-compute
milestone.

## 3. What Is Implemented Locally

Tracked local changes:

- `src/qemu/nvkvm_gpu_emul.c`
  - Adds `m2pbmap=/tmp/m2_pbmap.txt` device property and reloadable VA-to-GPA table.
  - CE virtual address resolution checks pbmap before channel page-table translation.
  - CE copy-fault logging now includes virtual address, resolved aperture, physical address, and
    phys-mode fields.
  - Adds M8.14 guest-kernel UVM shadow rows reported through BAR0 writes:
    - `0xFFF520` / `0xFFF524`: CUDA device VA low/high.
    - `0xFFF528` / `0xFFF52c`: shadow guest PA low/high.
    - `0xFFF530` / `0xFFF534`: size low/high.
    - `0xFFF538`: commit token.
  - CE write, read, and resolve paths check the M8.14 shadow table before falling back to channel
    page-table translation.
  - M8.102: guards empty wrapped GPFIFO tail entries instead of advancing through unresolved work.
  - M8.103: captures forwarded GPFIFO channel allocation `internalFlags` and patches the guest fake
    reply to match the host RM reply.
  - M8.104: advertises legacy INTA#, clears the GSP SWGEN0 interrupt vector, and deasserts legacy IRQ
    when the interrupt tree becomes idle.
  - M8.105: tracks per-channel host GR work in flight and completes `GP_GET` from host USERD before
    delivering host completion events.
  - M8.106: maps pbmap/channel target pages into the current CVAS instead of falling back to the
    legacy/default mapper.
  - M8.107: suppresses duplicate local completion posts for the same work-submit token and OS-event
    set. This is the change that made the bridge-backed `ctx_probe minimal` run return `CTX OK`.
  - M8.108/M8.31 diagnostics: optionally force one `MC_SERVICE_INTERRUPTS` zero result after local
    and/or host completion delivery. Use `NVKVM_M2_SERVICE_INTERRUPTS_ZERO_AFTER_LOCAL`,
    `NVKVM_M2_SERVICE_INTERRUPTS_ZERO_AFTER_HOST`, or
    `NVKVM_M2_SERVICE_INTERRUPTS_ZERO_AFTER_COMPLETION`; the default host budget is one per
    delivered host completion and can be capped with `NVKVM_M2_SERVICE_INTERRUPTS_HOST_ZERO_BUDGET`.
  - M8.112: gates the old mapdma/osdesc sentinel selftests behind
    `NVKVM_M2_RUN_MAPDMA_SELFTEST=1`. They were diagnostic probes after `0xc7c0`, not production
    mapping work.
  - M8.113: routes CVAS creation and UVM/working-set maps through the active CVAS host device handle
    instead of always picking the first forwarded device for the RM client. This prevents maps for a
    guest TSG from silently landing under the wrong host `hDev`.
  - M5.8/M8.114: makes the doorbell GR mapper a production dependency instead of a mapdma-selftest
    side effect, and primes newly snooped sysmem `va_map` rows into the legacy per-client GR VAS via
    OS_DESCRIPTOR/RM_MAP_MEMORY_DMA. This is the production-form subset of the old osdesc selftest
    side effect and is what lets default-path `ctx_probe minimal` keep passing with
    `NVKVM_M2_RUN_MAPDMA_SELFTEST` left off.

- `scripts/mode2_diag/nvkvm_uvm_uprobe_bridge.c`
  - Guest kernel debug module.
  - Registers uprobes on `cuMemcpyHtoD` and `cuMemcpyHtoD_v2` in guest `libcuda.so.580.159.04`.
  - On HtoD entry, copies up to `max_bytes` from the user source into a kernel page, computes the
    guest PA with `virt_to_phys`, and reports `<dst deviceVA, shadowGPA, size>` through the BAR0
    aperture above.
  - This removes the previous trusted guest userspace `LD_PRELOAD` requirement for the 4-byte proof.

- `scripts/mode2_diag/build_uvm_uprobe_bridge.sh`
  - Guest-side build/load helper for the bridge module.
  - Derives `cuMemcpyHtoD` and `cuMemcpyHtoD_v2` offsets with `readelf -Ws`.

- `scripts/mode2_diag/nvioctl_trace.c`
  - Existing RM ioctl tracing remains.
  - Adds `UVM_MAP_EXTERNAL_ALLOCATION` logging.
  - Adds opt-in CUDA memory-copy wrappers. With `NVUVM_SHADOW=1`, successful `cuMemcpyHtoD` calls
    copy the bytes into an anonymous guest page and log `CUDA_HTOD dst=... shadow=...`.
  - Logs `CUDA_DTOH` result bytes for confirmation.

- `scripts/mode2_diag/gcup2_pbmap.sh`
  - Exports live guest user pages for `/dev/nvidiactl`, `/dev/nvidia-uvm`, and `/dev/zero` staging
    maps.
  - Parses `CUDA_HTOD` records from `NVKVM_UVM_TRACE` (default `/tmp/guest_uvm_trace.txt`) and emits
    synthetic `<device VA> <shadow guest GPA> <size>` rows.
  - For the new uprobe-bridge PASS, pbmap is still used for ordinary guest staging pages, but not for
    HtoD shadow rows.
  - This is diagnostic-only plumbing. Production Mode-2 must not rely on QEMU reading guest userspace
    VAs, `/proc/$pid/pagemap`, or `/tmp/m2_pbmap.txt`.

- `scripts/mode2_diag/cup2_pause.c`
  - Paused CUDA probe that sleeps after HtoD so the live pbmap exporter can catch shadow/staging pages
    before DtoH.

Do not confuse the debug uprobe bridge with a production fix. It is a controlled proof that source
backing is the missing piece.

## 4. What Is Ruled Out

- The old `c7c0` / rbp SIGSEGV line is no longer the live failure. The later full-ctx failure was a
  timeout in `MC_SERVICE_INTERRUPTS` polling, not a libcuda SIGSEGV, and is now fixed on the default
  path by counted host completion service-zero credits.
- The 0x34-vs-0x3c host completion event tag was not sufficient to explain the old full-ctx timeout.
  Forcing `NVKVM_M2_EVENT34_MASK=0` posted 0x3c events and still looped on
  `0x20801702 -> ffffffff`.
- The NV0000 gpuId divergence and `0x20800102` high bit lead were already mostly ruled out after
  root-slot `addr=0x7` and response normalization.
- The final DtoH mismatch was not a destination staging problem. `/dev/zero` and command-window pbmap
  coverage fixed the destination.
- A no-`LD_PRELOAD` / no-`NVUVM_SHADOW` control run without the M8.14 bridge reached HtoD but read
  back zero from DtoH because the source device VA faulted in QEMU.
- The UVM allocation handle `hMemory=0x5c00007f` is guest RM/UVM state. QEMU has no matching
  shadow-forwarded host object for it, so "map the existing hMemory on the host" is not currently a
  valid fix path.

## 5. Production UVM Path

After the matmul/LLM bring-up loop, replace the debug guest-kernel uprobe proof with the real Mode-2
UVM external-allocation path.
The production rule is documented in `docs/design/mode2_dataplane_architecture.md`: QEMU must track
guest GR VA, GPGA/GPA, PDB leaves, and isolate-owned host mappings. Guest userspace VAs are opaque
except for debug probes, and CR3 is only an isolate/process key.

Concrete path:

1. Capture `UVM_MAP_EXTERNAL_ALLOCATION` identity through a guest-kernel or VMM-visible reporting
   path: `<base, len, hClient, hMemory, offset>`. Treat `base` as a GPU VA/range identity, not as
   an invitation for QEMU to read the guest process address space.
2. Ensure guest-visible UVM residency is system-memory/host-RAM resident. In-guest UVM migration is
   not a valid Mode-2 boundary because unprivileged QEMU cannot observe host GPU-vs-CPU residency and
   does not receive the host NVIDIA driver's migration interrupts. The guest may believe the page is
   sysmem-resident while the host NVIDIA kernel migrates it for GPU access; a later guest CPU access
   should resolve through the same GPA and host-side UVM/fault handling, not through a QEMU
   userspace-VA read.
3. For UVM sysmem leaves, map the guest-RAM GPA backing into the owning host isolate/context VAS at
   the same GPU VA using OS_DESCRIPTOR/RM_MAP_MEMORY_DMA, as in Mode 1. For GPGA leaves, use the GPGA
   range table and host-backed `gpu_memory_object`.
4. Let the host NVIDIA kernel own any later GPU faults and page migration. If the host migrates a
   page for GPU access, the guest is not notified; a later guest CPU access reaches the same GPA
   through KVM and must be resolved by host-side UVM/fault handling below QEMU.
5. Treat `/tmp/m2_pbmap.txt` and its `/dev/zero` staging rows as debug-only. Those rows are a
   proc/pagemap-derived way to locate anonymous CUDA staging pages during bring-up; they are not a
   production data source and do not justify reading guest userspace VAs from QEMU.
6. Remove or hard-gate the local CE copy parser from the production path. Host CE/GR work should run
   on the host channel; QEMU parsing is bring-up diagnostics only.
7. Investigate the remaining high-UVM CE packets that still cause host `dmaAllocMapping_GM107` spam
   and Xid 32. They are not required for the 4-byte debug PASS, but they are not production-clean.

## 6. Repro Recipe

Host and guest:

- `ssh vh` is the Vast host with the GPU and QEMU.
- `ssh vg` is the guest through QEMU user networking.

Deploy QEMU:

```bash
scp -q src/qemu/nvkvm_gpu_emul.c vh:/opt/qemu-src/hw/misc/nvkvm_gpu_emul.c
ssh vh 'cd /opt/qemu-src/build && ninja install'
```

Kill stale host processes:

```bash
ssh vh 'bash -s' <<'SH'
pids=$(ps -eo pid=,args= | awk '/[q]emu-system-x86_64/ {print $1}')
[ -n "$pids" ] && kill -9 $pids
pids=$(ps -eo pid=,args= | awk '/[n]vkvm_stub/ {print $1}')
[ -n "$pids" ] && kill -9 $pids
SH
```

Direct QEMU launch used for the PASS:

```bash
/opt/qemu-nvkvm/bin/qemu-system-x86_64 \
  -machine q35,accel=kvm,memory-backend=pcram \
  -object memory-backend-memfd,id=pcram,size=8G,share=on \
  -cpu host -m 8G -smp 4 \
  -drive if=none,id=hd0,file=/opt/nvkvm-guest/mode2-overlay.qcow2,format=qcow2 \
  -device virtio-blk-pci,drive=hd0,addr=0x9 \
  -drive if=none,id=seed,file=/opt/nvkvm-guest/seed.iso,format=raw,readonly=on \
  -device virtio-blk-pci,drive=seed,addr=0xa \
  -netdev user,id=net0,hostfwd=tcp::2222-:22 \
  -device virtio-net-pci,netdev=net0,addr=0x2 \
  -device nvkvm-gpu-emul,addr=0x7,vbios=/opt/nvkvm-guest/ga106_vbios.rom,m2fwd=on,m2exec=on,m2pbmap=/tmp/m2_pbmap.txt \
  -virtfs local,path=/usr/src/nvidia-580.159.04,mount_tag=ogkm,security_model=mapped,readonly=on \
  -virtfs local,path=/usr/lib/firmware/nvidia/580.159.04,mount_tag=nvfw,security_model=mapped,readonly=on \
  -virtfs local,path=/workspace/nvkvm,mount_tag=nvkvm_src,security_model=mapped \
  -serial file:/tmp/m0_serial.log -D /tmp/m0_qemu.log -d unimp,guest_errors -display none
```

Guest setup after every fresh boot:

```bash
scp -q \
  scripts/mode2_diag/cup2_pause.c \
  scripts/mode2_diag/nvioctl_trace.c \
  scripts/mode2_diag/nvkvm_uvm_uprobe_bridge.c \
  scripts/mode2_diag/build_uvm_uprobe_bridge.sh \
  vg:/tmp/
ssh vg 'bash -s' <<'SH'
set -euo pipefail
NVMODS=/home/ubuntu/nvmods
sudo systemctl isolate multi-user.target 2>/dev/null || true
sleep 2
sudo rmmod nvidia_uvm nvidia nvkvm_guest 2>/dev/null || true
sudo modprobe ecdh_generic ecc 2>/dev/null || true
sudo sysctl -w kernel.yama.ptrace_scope=0 >/dev/null 2>&1 || true
sudo dmesg -C || true
sudo insmod "$NVMODS/nvidia.ko" NVreg_EnableGpuFirmware=1 NVreg_RegistryDwords="RmGspBootRetryAttempts=1" 2>&1 | tail -1 || true
sudo insmod "$NVMODS/nvidia-uvm.ko" 2>&1 | tail -1 || true
UVM_MAJ=$(awk '$2=="nvidia-uvm"{print $1}' /proc/devices)
sudo mknod /dev/nvidia0 c 195 0 2>/dev/null || true
sudo mknod /dev/nvidiactl c 195 255 2>/dev/null || true
if [ -n "$UVM_MAJ" ]; then
  sudo rm -f /dev/nvidia-uvm /dev/nvidia-uvm-tools
  sudo mknod /dev/nvidia-uvm c "$UVM_MAJ" 0 2>/dev/null || true
  sudo mknod /dev/nvidia-uvm-tools c "$UVM_MAJ" 1 2>/dev/null || true
fi
sudo chmod 666 /dev/nvidia* /dev/nvidiactl 2>/dev/null || true
sudo ln -sf /usr/local/nvidia-guest/lib/libcuda.so.580.159.04 /lib/x86_64-linux-gnu/libcuda.so.1
gcc -O0 -g -o /tmp/cup2 /tmp/cup2_pause.c -I/usr/include -L/usr/lib/x86_64-linux-gnu/stubs -lcuda
gcc -shared -fPIC -O2 -o /tmp/nvioctl_trace.so /tmp/nvioctl_trace.c -ldl
SH
```

Build and load the no-`LD_PRELOAD` debug UVM bridge:

```bash
ssh vg 'bash -s' <<'SH'
set -euo pipefail
bash /tmp/build_uvm_uprobe_bridge.sh
SH
```

When using the 9p-mounted repo instead of copied files, do not `chmod` files under `/mnt/nvkvm_src`;
9p may reject it. Run the helper through `bash`:

```bash
SRC=/mnt/nvkvm_src/scripts/mode2_diag/nvkvm_uvm_uprobe_bridge.c \
  bash /mnt/nvkvm_src/scripts/mode2_diag/build_uvm_uprobe_bridge.sh
```

Launch the no-`LD_PRELOAD` run:

```bash
ssh vg 'bash -s' <<'SH'
rm -f /tmp/cup2_live.out /tmp/cup2_live.pid /tmp/guest_uvm_trace_absent.txt
GUESTLIB=/usr/local/nvidia-guest/lib
(LD_LIBRARY_PATH=$GUESTLIB stdbuf -oL -eL /tmp/cup2 > /tmp/cup2_live.out 2>&1 & echo $! > /tmp/cup2_live.pid)
SH
```

Refresh pbmap during the pre-DtoH sleep. This is still required for ordinary guest staging pages, but
the HtoD source row comes from the uprobe bridge, not from `NVUVM_SHADOW`:

```bash
for i in $(seq 1 55); do
  NVKVM_PBMAP_AHEAD_PAGES=16 NVKVM_UVM_TRACE=/tmp/guest_uvm_trace_absent.txt \
    scripts/mode2_diag/gcup2_pbmap.sh /tmp/m2_pbmap.txt
  ssh vg 'tail -n 12 /tmp/cup2_live.out'
  ssh vg 'grep -q "CE rv=" /tmp/cup2_live.out' && break
  sleep 2
done
```

## 7. Architecture Notes

- Channels: `c56f` = GPFIFO channel, `a06c` = channel group/TSG, `9067` = context share, `90f1` =
  VASPACE, `0070` = memory virtual, `c7c0` = compute, `c7b5` = copy.
- GR/compute channels and COPY channels use different RM clients during `cuCtxCreate`. COPY channels
  do most of the scrub/init work.
- Guest CPU-side UVM mappings are not visible to QEMU through GSP RPCs. The debug trace sees them
  only because it hooks the guest userspace `ioctl` and CUDA API.
- The intended end state is still a range-table model:
  `guest GPU VA -> channel PDB -> GPGA/range table -> backing object + offset`.
- Do not refactor `nvkvm_gpu_emul.c` yet. It still has duplicate doorbell/exec paths, multiple
  resolvers, and debug probes. Save cleanup for TASK #128 after first-compute is production-clean.

## 8. Supporting Traces

Previously committed:

- `docs/design/mode2_traces/host_cup2_trace.txt`
- `docs/design/mode2_traces/guest_cup2_trace.txt`
- `docs/design/mode2_traces/guest_root7_trace.txt`
- `docs/design/mode2_traces/ctrl_divergence.txt`

Added for this milestone:

- `docs/design/mode2_traces/guest_uvm_shadow_trace.txt`
- `docs/design/mode2_traces/qemu_ce_shadow_pass.txt`
- `docs/design/mode2_traces/pbmap_shadow_row.txt`

Added for the M8.14 no-`LD_PRELOAD` bridge milestone:

- `docs/design/mode2_traces/guest_uvm_uprobe_bridge_pass.txt`
- `docs/design/mode2_traces/qemu_uvm_uprobe_bridge_pass.txt`
- `docs/design/mode2_traces/host_uvm_uprobe_bridge_residual_xid.txt`
