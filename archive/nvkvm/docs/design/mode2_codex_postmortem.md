# Mode-2 Codex run — postmortem / mechanism extraction (2026-06-10)

A Codex (OpenAI) agent ran ~2 days on `mode-2` (commits `e7f94ea..7fb47f1`, +WIP snapshot
`05313f0`). It **got past the cuCtxCreate wall** — cuCtxCreate now completes and a **4-byte
HtoD/DtoH round-trip passes on the default path** (no LD_PRELOAD) — but at the cost of **doubling
`gpu_emul.c` (6.4k → ~14k lines)** with heavy diagnostic scaffolding, and the hard sub-problem
(copy-channel Xid 32) is still open. This is the analysis (from commits + diff + committed traces;
the 154 MB Codex transcript wasn't cleanly parseable, so the causal story is partly inferred).

Preserved reference: branch `backup/codex-ctxcreate-roundtrip-20260610`, WIP commit `05313f0`.

## The real fix (LOAD-BEARING — port this into the #128 consolidation)

Codex's approach: **back the UVM dataplane** (my earlier "rbp-clobber from divergent reply bytes /
gpuId enum" lead was likely a *downstream symptom* of the data plane being unbacked — libcuda
crashed reading uninitialized/garbage memory during cuCtxCreate's internal transfers). The
load-bearing pieces:

- **`m2_uvm_ext[256]` table** (`nvkvm_gpu_emul.c:631`) — records each `UVM_MAP_EXTERNAL_ALLOCATION`
  range: `{va, size, hClient, hMemory, obj_idx, per-CVAS residency masks}`. The VA→identity map for
  UVM-managed memory.
- **`m2_objs[].forwarded` flag** — distinguishes real forwarded UVM memory from QEMU synthetic
  scratch. When `forwarded`, QEMU **does NOT zero** the backing (the guest/UVM already placed real
  data). This non-clearing semantics is essential.
- **`nvkvm_m2_uvm_ext_record()` / `nvkvm_m2_uvm_ext_ensure_obj()`** — record a range, then lazily
  create + link a host RM object for it in `m2_objs[]`.
- **`nvkvm_m2_uvm_ext_invalidate_span()`** — flush host CPU caches on the coherent guest-RAM shadow
  before the GPU/CE reads it (coherency for the round-trip).
- **Pushbuffer resolution** (~`:10819`) at doorbell time: if a pushbuffer VA falls in a forwarded
  UVM range, flush + use it directly instead of the pbmap-copy fallback.
- **Service-interrupt credit accounting (M8.108)** — *orthogonal but essential*: fixed a cuCtxCreate
  **timeout** (not the crash) where `MC_SERVICE_INTERRUPTS` polled forever; accumulate host
  completion service credits per delivered completion so the poll terminates.
- **GR VAS priming (M8.114)** — legacy per-client GR VA mapping now done explicitly during doorbell
  setup via `OS_DESCRIPTOR`/`RM_MAP_MEMORY_DMA`, not as a selftest side-effect.

So there were really **two gates**: the crash (missing UVM backing) **and** a hang
(interrupt-credit accounting). Both had to be fixed to reach the round-trip.

## Throwaway scaffolding (DISCARD in consolidation — do NOT ship)

- **`scripts/mode2_diag/nvkvm_uvm_uprobe_bridge.c`** (943 lines) — a guest-kernel module that
  **uprobes `cuMemcpyHtoD` in libcuda** + libc `ioctl`, copies the userspace source into a
  memfd-backed guest GPA, and reports `<VA, shadowGPA, size>` to a BAR0 debug aperture. This is
  proof-of-concept ONLY — production must not depend on kernel uprobes.
- **`m2_uvm_shadow[262144]`** (`:620`) — BAR0 in-band debug reporting target for the uprobe bridge.
- **`m2_pbmap[8192]`** (`:606`) — pagemap-based pushbuffer shadow; debug fallback only.
- `*_pause.c` harnesses, `NVKVM_M2_RUN_MAPDMA_SELFTEST` + old osdesc selftests, the 256/128-gated
  `qemu_log` DIAG spam (~468 mentions).

## Residual / still broken

- **matmul** not validated (alloc + launch work, compute result unchecked).
- **Copy-channel Xid 32 + `dmaAllocMapping_GM107: can't alloc VA space`** persist on high-UVM CE
  packets — the same copy-channel-VAS collision wall from the per-channel-VAS work. The 4-byte
  round-trip only works because it lives in a **low VA range** the uprobe happened to shadow; UVM
  ranges outside that scope still fault the host. Codex's own note: production must capture **all**
  UVM external ranges, not just what a uprobe intercepts.

## The key insight to carry forward

The UVM-backing mechanism is right; the *capture method* is wrong. **Production should snoop the
`UVM_MAP_EXTERNAL_ALLOCATION` identity from the RM/GSP-RPC stream we already intercept** (Codex even
documented this in commit `dd78c82` "production UVM residency model") and feed the *same*
`m2_uvm_ext` / `m2_objs` linkage — but backed by the **GPGA/GPA range tables**, not userspace
page-table walks or kernel uprobes. That collapses Codex's scaffolding into the GPGA model that was
already the intended end-state (see `mode2_cuctxcreate_resume.md` §5).

## Consolidation (#128) port list

KEEP: `m2_uvm_ext[]` + record/ensure_obj/invalidate_span, `m2_objs[].forwarded` non-zero semantics,
the doorbell pushbuffer-resolution check, service-interrupt credit budget (M8.108), GR VAS priming
(M8.114). DISCARD: uprobe bridge, `m2_uvm_shadow[]`, `m2_pbmap[]`, selftests, DIAG spam. RE-BACK the
kept mechanism on GPGA tables + RM-stream UVM-range capture. Then re-attack the copy-channel Xid 32.
