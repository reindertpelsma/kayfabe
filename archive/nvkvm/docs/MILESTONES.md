# Milestones

## Mode-2 (emulated GPU + faked GSP) — single-process apps at native parity (2026-06-16)

Stock NVIDIA driver in the guest drives an **emulated GA106 + faked GSP**; we recover the
guest's compute intent and forward it to a real host GPU. Validated on **bare-metal** box .32
(RTX 3050 = GA106, non-nested) against host **open driver 595.71.05** (our stack builds for
575/580):

- **`cuCtxCreate → PTX JIT → cuLaunchKernel → matmul → DtoH`, byte-exact** at scale
  (cup8 N=1024, `bad=0 maxerr=0`).
- **llama.cpp LLM inference** (small Qwen2 GGUF) — coherent, completes, **49.9 tok/s ≈
  host-native 47.5 tok/s on the same 3050 = ~zero forwarding overhead on bare metal**. (The
  vast.ai 20→50 t/s gap was entirely nested-virt vmexit tax, not Mode-2 design.)
- **PyTorch 2.5.1 single-process** — full workload byte-correct, `rc=0`: CUDA events
  (`torch.cuda.Event` timing), non-default streams, a 50-step training loop (autograd + SGD).
  The init "hang" was a CE zero-fill stomping a live channel's USERD page (fix 32c5115), not
  an event bug.

So three app classes (general compute, LLM, PyTorch training) run single-process through Mode-2
at near-native throughput on a commodity consumer GPU, no vGPU/SR-IOV/license.

### In progress — multi-process / multi-context (#12)

Two concurrent or sequential CUDA contexts is the gate to in-guest usefulness. It's a stack of
context-lifecycle bugs, being peeled layer by layer:

- **L1 (fixed, 7680305):** drop a context's channel/VAS bookkeeping on `GSP_RM_FREE` so the next
  context doesn't inherit stale VAS routing (was: ctx2 gpfifo faults).
- **L2 (fixed, 05ac359):** `UNLOADING_GUEST_DRIVER` (fn 47) fires on GPU-idle release, not just
  rmmod; we no longer zero the GSP-RPC seqNum before acking it, so teardown stops corrupting the
  queue (was: Xid 119). **Also fixes "driver reload crashes QEMU" without a VM restart.**
- **L3a (fixed, 011843d):** a CE completion-semaphore page collision — `nvkvm_chan_translate`'s
  blind VAS fallback collapsed CeUtils' and a UVM channel's semaphores (both at guest VA
  `0x121000010` in their own per-client VASes) onto one phys page, so a low payload read as a 2³²
  backward jump (`uvm_gpu_semaphore.c:776` + `ce_utils.c:349`). Fix: a client key on `chan_vas[]` +
  prefer the executing channel's own-client VAS before the blind pass. Bench-confirmed de-aliased;
  cup8 byte-exact, no single-process regression. (The fn-47 teardown fires *after* the rewind, so it
  was not the cause — the earlier "discriminate idle vs reload" direction was wrong.)
- **L3b (open):** after de-aliasing, a distinct **intra-UVM temporal page-reuse** remains — a live
  UVM channel tracking semaphore observes a backward payload as its slot/page is recycled across the
  ctx1→ctx2 boundary (UVM only canary-resets a slot on channel free, after which the tracking sema is
  destroyed; our emulation lets a low release land on a still-live slot). Next: instrument the sema's
  resolved guest-phys vs the guest CPU mapping + the channel free→cursor reset.

Security: multi-tenant isolation is **not yet honest** for Mode-2; the per-GR-VAS-keyed isolate
(the GMMU-aligned boundary, superseding the CR3 plan) lands with the Rust rewrite + review passes.

## v0.1 — first LLM inference through nvkvm (2026-05-28, eb6e16f)

End-to-end CUDA pipeline working:

- `cuInit / cuDeviceGet / cuCtxCreate / cuMemAlloc / cuMemcpyHtoD / cuMemcpyDtoH` — all green
- `cuMemcpy` round-trip byte-exact up to **256 MB**
- `cuLaunchKernel` — `vector_add` and **1024×1024 fp32 GEMM** (1B FMAs, CPU-verified)
- `cuModuleLoadData` — PTX JIT works (libnvidia-ptxjitcompiler must match libcuda version)
- Multi-process within one VM boot — cumemalloc + vec_add + matmul interleaved
- **Qwen2.5-0.5B-Instruct GGUF via llama.cpp** answers "Q: What is the capital of France?" → "Paris"
  - 298 tok/s prompt, 29.6 tok/s generation
  - all 25 layers offloaded to GPU (`-ngl 99`)
  - built with `-DGGML_CUDA=ON -DCMAKE_CUDA_ARCHITECTURES=86` on guest RTX 3060

Host environment: vast.ai RTX 3060, NVIDIA open kmd 575.51.03, Ubuntu 22.04 + KVM/QEMU + nvkvm-guest.ko in an Ubuntu 24.04 guest VM. No driver patches required for production runtime (debug printks are stripped before release).

## Known issues at v0.1

- Security audit (2026-05-28) flagged cross-session handle-table reach and stub seccomp disabled-for-debug. Tracked separately; functional path is solid, multi-tenant claim is not yet honest. See memory/security_audit_2026_05_28.md.
