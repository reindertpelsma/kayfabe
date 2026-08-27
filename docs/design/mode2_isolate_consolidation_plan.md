# Mode-2 isolate consolidation — proper fix for the single-isolate handle collision

**Status:** PLANNED (2026-06-06). Chosen over the interim per-client handle remap. This is the
"reuse Mode-1's spine" work the user has asked for repeatedly. Unblocks compute-client forwarding
(the `grmapper st=0x57` collision) and retires the whole handle-collision class.

## The problem being fixed
Mode-2 currently forwards the guest's RM alloc stream through **ONE** host isolate (`m2_iso`,
`m2_iso_id`) = one host RM session = one host handle namespace, with its own ad-hoc handle/fd
bookkeeping in `nvkvm_gpu_emul.c` (`m2_ht`, `nvkvm_m2_client*`, stored fds). The guest **reuses RM
handles across clients** (VAS handle `0xcaf00000` is alloc'd by `0xc1e00004/05/06/09`,
`0xc1d0000a/0b`, `0xc1d00001`, …). In one shared session those collide → a later client's
virtmem-over-`0xcaf00000` is cross-client → RM denies (`0x57`). Proof: the lone client with a unique
VAS handle (`0xc1e00008→0x0000000a`) forwarded + mapped successfully; all `0xcaf00000`-reusers fail.

## Target architecture (from [[mode2_isolation_cr3_key]], [[access_model_split]], [[hclient_not_fd_scoped]])
- **One host isolate per guest RM client** (`hClient` from `NV01_ROOT`/client alloc). Each client →
  its own host RM session → its own host handle namespace, so reused guest handles in *different*
  clients never collide. (Within one client, handles are unique by RM construction.)
- **Drop the parallel m2 bookkeeping**: remove `m2_iso`/`m2_iso_id`/`m2_ht` and all fd storage from
  `gpu_emul.c`. Use Mode-1's global handle table (handle→object, fd-by-handle) and isolate registry.
  `gpu_emul.c` keeps only the Mode-2-specific data path (mmap / KVM region / BAR / DMA / GSP-RPC
  shim); identity + handle/fd translation come from the Mode-1 spine.
- **Cross-client references** (UVM `DUP_OBJECT`, a channel referencing another client's VAS, etc.)
  → `COPY_HANDLE_TO_ISOLATE` (Mode-1 already implements this dance).
- **Stub fully shared** with Mode-1 (already is).
- The doorbell/USERD MMIO path keys on **vCPU CR3** (guest userspace address space) per
  [[mode2_isolation_cr3_key]]; the **RPC/alloc forwarding path keys on `hClient`** (the cmdq is a
  single shared queue, so per-RPC CR3 attribution isn't reliable — `hClient` is the per-RPC identity).

## Steps
1. **Map the current m2fwd handle/session surface.** Enumerate every use of `m2_iso`, `m2_iso_id`,
   `m2_ht`, `nvkvm_m2_client`, `nvkvm_m2_client_known`, stored `m2_*_h` fds, `m2_gpu_h`, `m2_ctl_h`.
   These all assume one session.
2. **Introduce a client→isolate registry** (reusing Mode-1's isolate create/lookup). On the guest's
   `NV01_ROOT` client alloc (forwarded), create/lookup a host isolate keyed by the guest `hClient`.
   Route every subsequent forwarded ioctl for that client to its isolate.
3. **Route OS_DESCRIPTOR / map_dma / grmapper / control through the per-client isolate** instead of
   the single `m2_iso`. `back_and_map_sys`, `grmapper`, `os_descriptor`, `map_dma`, `control1`,
   `alloc1`, `host_alloc_map_vidmem` all take/derive the isolate from `client`.
4. **REGISTER_FD per isolate**: each client's session needs its own ctl-fd + nvidia0-fd registration
   (today `m2_gpu_registered`/`m2_ctl_h` are global — make them per-isolate).
5. **Cross-client COPY_HANDLE** for the few cross-client refs (identify them in the trace first:
   channel.hVASpace under a different client, UVM dups).
6. **Delete** `m2_ht` + fd storage from `gpu_emul.c` once everything routes through Mode-1's table.
7. **Validate**: re-run cup2 with m2fwd+m2exec. Expect the compute client's `grmapper` to now succeed
   (its `0xcaf00000` is in its own session) and `M5.19 fwd-map pushbuffer … MAPPED` for the compute
   pushbuffer (`0x120000000`) + sema. Then host-GPFIFO write + ring `host_token` (`m2ring`).

## Risks
- `nvkvm_m2_client*` is used pervasively in the forwarding path; rerouting touches a lot. Do it
  incrementally and keep the **CeUtils path working** (it's the regression canary — it currently
  forwards+maps successfully).
- Per-isolate REGISTER_FD ordering (ctl fd before device fd) must be preserved per session.
- Don't break the Mode-1 forwarding (shared stub/tables) — guard Mode-2 routing behind the Mode-2
  path only.

## After this lands
Compute client forwards → pushbuffer + sema mapped into its host GR VAS (WB, proven by M5.19) →
write GP entries into the host channel's own GPFIFO → ring `host_token` → host GPU runs real compute
and writes the completion the guest polls → **first compute**. See [[mode2_first_compute_blocker]].
