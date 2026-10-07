# Mode-2 cuCtxCreate blocker — full problem statement, analysis, and plan

_Status 2026-06-06. Branch `mode-2`. Author handoff after a long diagnostic push._

## 1. The goal (Mode-2)

Run the **stock, unmodified open NVIDIA driver (580.159.04)** inside a KVM/QEMU guest against a
fully **emulated GA106 GPU** (`nvkvm-gpu-emul` in `src/qemu/nvkvm_gpu_emul.c` + a fake GSP), and
forward the **real compute work** to the host's real RTX 3060 / GA106 via **unprivileged** host
`nvidia` ioctls (the Mode-1-proven double-mmap forwarding). North star: `cuCtxCreate` → first real
compute on the guest.

**Principle (do not violate):** forward *userspace* GPU operations (allocs, controls, command
submission, object creation) to the host as unprivileged ioctls; *simulate* only the
**guest-kernel-internal / privileged** operations (GSP init, firmware, golden-ctx, PDB/GPGA, the
GSP-RPC internal controls), because we only need to satisfy the (fully open-source) guest kernel
module — the host owns the real context. See `memory/mode2_control_forward_vs_replay.md`.

## 2. Where we are

- **cuInit, device enumeration, attributes, totalMem: PASS.** The guest reaches `cuCtxCreate`.
- **`cuCtxCreate` crashes**: libcuda SIGSEGVs on an internal worker thread. Confirmed signature:
  `mov -0x38(%rbp),%rax` with **`rbp = 0`** (strace shows `SIGSEGV ... si_addr=0xffffffffffffffc8`
  = −0x38). It's a **frame-pointer / stack corruption**, not a clean error return.
- The crash happens **immediately after a successful `RM_ALLOC` of the GR compute object (class
  `0xc7c0`, AMPERE_COMPUTE_B)** — that alloc returns `status=0x0` (the guest kernel ACCEPTS it), and
  libcuda dies in userspace before issuing the next ioctl the host issues (`0x906f0101`, `c7b5`…).

## 3. What the host-vs-guest diffing established (this is the important part)

We built an LD_PRELOAD ioctl tracer (`scripts/mode2_diag/nvioctl_trace.c`) that decodes NVOS54
(RM_CONTROL) / NVOS21 (RM_ALLOC) and runs **identically on the guest (`cup2`) and a native host
(`cup2_host`)**, plus full `strace -f -v -s 256`. Findings:

1. **The entire control/ioctl SURFACE matches the host.** Same ioctl *sequence*; identical control
   *statuses* (after we fixed one: `GPU_QUERY_ECC_STATUS 0x2080012f` was faking `NV_OK`, host
   returns `NOT_SUPPORTED 0x56` — fixed; same for NVLINK `0x20803002`); identical libcuda buffer
   sizes (`paramsSize`); and all reply *content* differences are provably benign (the gpuId/BDF
   `0x10000` vs `7`, ASLR pointers, client/object handles).
2. **So the crash is NOT in the control plane.** A deterministic libcuda gets identical ioctl
   answers and still crashes → the divergence is in something the libcuda-ioctl diff cannot see.
3. **The one hard divergence is an mmap SIZE** (found only via full strace):
   ```
   GR VA-region MAP_FIXED mmaps on /dev/nvidiactl:
     HOST :  0x200400000 + 48 MiB (50331648)   then 0x204400000/0x204600000/0x204800000 (2 MiB ea)
     GUEST:  0x200400000 + 64 MiB (67108864)   then SIGSEGV (never reaches the rest)
   ```
   The guest sizes the **GR context-buffer region 16 MiB larger** than the host.

## 4. Root cause (localized, not yet fixed)

The GR ctx-buffer region size is computed **kernel-side** (invisible to a libcuda-ioctl diff) from
the **GR static info the guest gets from our fake GSP**. The global ctx buffers
(BUNDLE_CB/circular, PAGEPOOL_GLOBAL, ATTRIBUTE_CB, PRIV_ACCESS_MAP) are read directly from
`pContextBuffersInfo->engine[...].size` — i.e. our **replayed `0x20800a32`
(`INTERNAL_STATIC_KGR_GET_CONTEXT_BUFFERS_INFO`)** blob (`src/qemu/mode2_initctrl_ga106.h`,
`ctl_20800a32`). The per-context GFXP buffers (SPILL/BETA_CB/PAGEPOOL/RTV_CB, `0xffffffff` =
"dynamic" in that blob) are computed by the guest kernel from the **TPC/PPC floorsweep**
(`0x20803801` `GRMGR_GET_GR_FS_INFO`, also a replayed blob) + attribute-buffer-size.

**Strong hypothesis:** our captured GA106 static-info blobs (`mode2_initctrl_ga106.h`) were captured
from a **different GA106 floorsweep** than this vast.ai RTX 3060 (consumer cards are binned
differently), so the guest sizes ctx buffers for more TPCs → 64 MiB; this host's real config → 48
MiB. The 16 MiB ≈ extra GFXP/per-TPC buffers. The wrong layout then corrupts libcuda's state → the
`rbp=0` clobber. (Why invisible before: libcuda's *own* `GR_GET_INFO 0x20801201` matched the host,
but the **kernel-internal** sizing static-info we replay did not.)

**Open question that decides the fix:** is the 64 MiB region the *direct* cause (a too-big MAP_FIXED
clobbering an adjacent mapping / wrong offsets) or a *symptom* of a deeper static-info mismatch that
also corrupts elsewhere? Need ground truth (§6) to be sure.

## 5. Candidate solutions (ranked)

1. **Re-capture this host's real GR static info and replay it** (most aligned with the principle).
   Get the host's real `0x20800a32` (ctx buffer sizes), `0x20803801` (floorsweep), PPC masks, and
   GSP static info via a host-side printk in the open `nvidia.ko` (full permission granted), and
   regenerate `mode2_initctrl_ga106.h` from THIS card. Expect guest → 48 MiB → likely clears the
   crash. **Risk:** if the captures are already correct, the 16 MiB comes from a kernel computation
   we feed wrong elsewhere (then go to #2).
2. **Build the curated forward-vs-simulate control table** (the real architecture). Use the
   host-native trace as the oracle: forward every control where forwarding reproduces the
   host-native status+content; simulate (replay captured bytes) the GSP-internal rest. This kills
   the whole class of "we fake NV_OK / replay a stale blob" bugs, not just this one. Bigger build;
   needs the debug utility in §7 to be efficient.
3. **Make the guest size ctx buffers to match the host directly** — if the static-info is right but
   a computation (e.g. GFXP-enable for a compute channel, or a VEID/subctx count) diverges, patch
   the *value we report* for the field that drives it. Requires the ground-truth dump (§6) to name
   the field.

## 6. The decisive next experiment (ground truth)

Instrument the open driver's ctx-buffer sizing on **both host and guest** at the SAME function and
diff the per-buffer sizes + the TPC/config inputs:
- File: `research_clones/ogkm/src/nvidia/src/kernel/gpu/gr/arch/maxwell/kgraphics_gm200.c`,
  `kgraphicsAllocGlobalCtxBuffers` — printk `circularBufferSize / pagepoolBufferSize /
  attribBufferSize / privMapBufferSize` (all read from `pContextBuffersInfo->engine[...]`), and the
  per-context GFXP sizes + the TPC count used.
- This is the **RM core** (`src/nvidia`), which the fast DKMS/`make modules` build ships
  **precompiled** — so it needs a **full source build** (`make` from the ogkm root, compiles
  `src/nvidia` → `nv-kernel.o`). Slow (~10–30 min) but decisive. Do it on the host (real values)
  AND guest (our values); the divergent buffer + its input names the bug.

## 7. Proposed debug utility (HIGH VALUE — build this)

A reusable, host==guest-identical NVIDIA observation tracer that produces a *semantic* diff, far
better than raw strace. Two parts (LD_PRELOAD, no driver changes):

### 7a. ioctl + struct decoder (`nvioctl_trace` → grow into a Python-decoded format)
- Hook `ioctl`/`openat`/`read`/`mmap` (already partly done in `scripts/mode2_diag/nvioctl_trace.c`).
- Emit a machine-readable record per ioctl: fd→path, the NVOS opcode **decoded to its enum name**
  (NV_ESC_RM_CONTROL/ALLOC/MAP_MEMORY/…), and for controls the **cmd → control-name** + the full
  params struct **field-by-field** (not just a hexdump), via a **Python struct parser** generated
  from the open SDK headers (`src/common/sdk/nvidia/inc/ctrl/**`). Track **handle lifetimes**
  (hClient/hObject/hMemory) and **fd↔handle** maps so the diff can canonicalize handles (the
  benign-but-noisy differences) and surface only *semantic* divergences.
- Cover **UVM ioctls** too (`/dev/nvidia-uvm`, different encoding) — the original tracer wrongly
  filtered to `type==0x46` and missed them.
- Same tool on host + guest → a structured diff that ignores ASLR/handles and flags real field
  divergences (status, sizes, counts, flags, content).

### 7b. nvidia-mmap read/write trap (`mmaptrace`)
- Hook `mmap` of any `/dev/nvidia*` fd; record region (addr/len/offset/fd→object).
- For chosen regions, **mprotect(PROT_NONE) + a SIGSEGV handler that single-steps** to log every
  read/write (addr, size, value) to the device mapping — i.e. the doorbell/USERD/BAR/IOMMU-mapped
  memory libcuda touches directly (the data-plane the ioctl diff is blind to). Heavy but it is the
  only way to see the mmap'd-memory divergence.
- Run identically on host + guest → diff what libcuda *reads back* from GPU memory. This is where a
  Mode-2 emulation almost always diverges (a register/struct in mmap'd memory reads a wrong value),
  and it's exactly the surface we currently cannot compare.

**Why this matters:** a deterministic userspace (libcuda) crashing on guest but not host *must* see
an observable difference. We proved it's not the ioctl replies; the remaining unobserved surfaces
are (i) kernel-internal sizing (the 48-vs-64 MiB, addressed by §6) and (ii) **mmap'd-memory reads**
(7b). Building 7a+7b once, host==guest, turns "guess and re-boot" into "diff and see."

## 8. Operate / repro commands

VM disk: base `/opt/nvkvm-guest/ubuntu-24.04.qcow2`, persistent overlay
`/opt/nvkvm-guest/mode2-overlay.qcow2`. Host SSH alias `vh`; guest SSH on host port 2223.

```bash
# --- vast.ai instance (API key in memory/vastai_credentials.md) ---
vastai show instances            # status
vastai start instance <ID> ;  vastai stop instance <ID>
# reboot if the host GPU wedges (100% util / no procs):
ssh vh 'sudo rmmod nvidia_uvm nvidia_drm nvidia_modeset nvidia; sudo modprobe nvidia'

# --- start/stop the guest VM (on the host) ---
ssh vh 'NVKVM_M2FWD=on nohup bash /workspace/nvkvm/scripts/run_mode2_vm.sh >/tmp/m0_launch.log 2>&1 &'
ssh vh 'pkill -9 qemu-system'    # stop. add NVKVM_FRESH=1 to discard the overlay.
# guest shell:
ssh -p 2223 ubuntu@localhost     # (from the host) -- or `ssh vg` if aliased

# --- run the test natively on the HOST (ground truth) ---
ssh vh 'cd /tmp/mode2_diag && gcc -shared -fPIC -O2 -o nvioctl_trace.so nvioctl_trace.c -ldl'
ssh vh 'nvcc -o /tmp/cup2_host /workspace/nvkvm/tests/mode2/cup2.c -lcuda'
ssh vh 'LD_PRELOAD=/tmp/mode2_diag/nvioctl_trace.so NVTRACE=/tmp/host.trace NVCONTENT=48 NVALLOC=32 /tmp/cup2_host'
# or full strace:
ssh vh 'LD_LIBRARY_PATH=/usr/local/nvidia-guest/lib:/lib/x86_64-linux-gnu strace -f -v -s 256 -o /tmp/host_full.strace /tmp/cup2_host'

# --- run the test in the GUEST (m2fwd) ---
# helper scripts on the host /tmp: fullstrace_outer.sh (boot+strace+pull), trace_outer.sh,
# cup2_inner.sh (module load + cup2). cup2 must run NON-sudo with:
#   LD_LIBRARY_PATH=/usr/local/nvidia-guest/lib:/lib/x86_64-linux-gnu
ssh vh 'bash /tmp/fullstrace_outer.sh'   # boots guest, straces cup2, pulls /tmp/guest_full.strace

# --- rebuild QEMU after editing src/qemu/nvkvm_gpu_emul.c ---
rsync -az src/qemu/nvkvm_gpu_emul.c vh:/opt/qemu-src/hw/misc/nvkvm_gpu_emul.c
ssh vh 'cd /opt/qemu-src/build && ninja qemu-system-x86_64 && ninja install && cp -f qemu-system-x86_64 /opt/qemu-nvkvm/bin/'
```

Gotchas: fresh QEMU boot per `cup2` attempt (a crash wedges the emulated-GSP state); run GPU tests
strictly serially; the guest's real libcuda is `/usr/local/nvidia-guest/lib/libcuda.so.580.159.04`.

## 9. Key files
- QEMU emulated GPU + fake GSP + forwarding: `src/qemu/nvkvm_gpu_emul.c`
- Replayed GSP/control captures: `src/qemu/mode2_initctrl_ga106.h` (the `ctl_*` blobs, incl
  `ctl_20800a32`), `src/qemu/nvkvm_ctrl_allowlist.h`
- Diagnostic toolchain: `scripts/mode2_diag/` (`nvioctl_trace.c`, `report.py`, `README.md`)
- Open driver source (for printks / formulas): `research_clones/ogkm/`
- Memory: `mode2_cuctxcreate_rbp_clobber`, `mode2_control_forward_vs_replay`, `mode2_resume_state`,
  `mode2_grctx_privilege_wall`.
