# nvkvm parity & hardening hunt — execution plan

Source of gaps: `docs/audits/nvproxy_gap_analysis.md` + the 2026-05-29 security audit.

## Orchestration model (why hybrid, not pure fan-out)

Hard constraints:
1. **Singleton test bench** — one vast.ai host, one GPU, one QEMU/VM. Build→deploy→GPU
   test cannot run concurrently. ALL verification serializes.
2. **Overlapping hot files** — `src/qemu/nvkvm_isolate_handlers.c` (#76/#77/#66),
   `src/guest/nvkvm_main.c` (#73/#78). Parallel writers conflict.

Therefore:
- **Parallel sub-agents** only for read-only research / spec extraction (no GPU, no writes).
- **One serial implementation lane** applies + tests + commits, grouped by file.
- This conversation = orchestrator + the single integration-test lane.

## Tiered work

### Tier 1 — security moat
- **#77** bounds-check `GPU_EXEC_REG_OPS` (0x20800122) embedded reg-op array. (warmup)
- **#76** RM control-command allowlist + capability tags (default-deny ~185 cmds).
- **#67** cross-VM hClient isolation PoC (prove TYPE_CLIENT denial empirically).
- **M-2/M-3** stub seccomp hardening; **H-4** adversarial teardown reclamp.

### Tier 2 — correctness
- **#78** NV_GR/compute alloc-class sizing entries (latent breakage).
- **#73** signal/interrupt delivery (designed; stub-heavy).
- **#65/#60** finalize mm-principal access model + invariant/warn; **#61** UVM teardown audit.

### Tier 3 — compat polish
- **#66** nvidia-smi memory via QEMU-init-ns GET_PID_INFO query.
- CUDA-IPC export/import FD translation (0x3d05/06/08/0b/0c); P2P caps (NCCL);
  `/proc/driver/nvidia/params`.

### Tier 4 — scaling / headline
- **#55** real 64-bit PCI BAR; **canonical stub redeploy**; **#27** 7B+ LLM inference.

## Execution order (file-grouped, test-serialized)
1. Research agents (parallel): #76 control-cmd table, #78 NV_GR sizes. [read-only]
2. #77 reg-ops bounds (inline) → test.
3. #76 control allowlist (uses #76 research) → test.  [same file as #77 → after it]
4. #78 sizing entries → test.
5. #66 QEMU-init-ns query → test.
6. Tier 2/3/4 as scheduled.
