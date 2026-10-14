# V3 channel budget — the count the guest is told is the cap kayfabe enforces

**STATUS: LIVE, 2026-10-11 (claude/gl-icd-crash-20261011; hardware verification in `traces/windows_gl_crash_20261011/README.md`).**

## Why it exists (measured)
Every guest channel is a real channel twin on the shared host GPU, and the host driver enforces no per-client
quota (`kf-core/src/caps.rs`, §9.1). The cap was a hardcoded `VmCaps::from_declared(64, ...)` while the guest was
told `numChannels = 2048` per runlist. A Windows desktop holds about 55-64 live twins idle (run 501 BORN/released
replay), so the 65th channel — a GL or CUDA context — was refused `0x1a` (`act birth passthrough REFUSED
OverDeclaredCap { cap: 64, asked: 65 }`, then `GSP REFUSED fn103/0x0000c56f=0x1a`): the OpenGL driver crashed
and `cuCtxCreate` returned 999. §9.1's own rule — "the cap is the number we told the guest" — had been broken.

## The rule
One number, `channel_budget` (channels **per runlist**):
- it is what the guest is told (`NV2080_CTRL_CMD_INTERNAL_FIFO_GET_NUM_CHANNELS`, `numChannels`, every runlist);
- it is the enforced cap, **per guest runlist** (`VmCaps::acquire_channel`, indexed by `TokenIndex::runlist_of`): a guest that
  fills one runlist cannot take another's share, and a guest using everything it was told cannot take more than that
  from the host;
- it is printed at realize (`kf3: channel budget N per runlist (Derived|Requested; derived default D, host limit L ...)`),
  and in every refusal (`OverDeclaredCap { cap: N, asked: N+1 }` plus the loud line
  `kf3: ⚠ CHANNEL BUDGET EXHAUSTED on guest runlist R ...`);
- refusals are counted by name in the status line: `chan[... birth_refused_cap=K ...]` (always on).

## Where the default comes from (derive, never capture)
The host's own unprivileged answer, `NV2080_CTRL_CMD_FIFO_GET_INFO` (`RMCTRL_FLAGS_NON_PRIVILEGED`), once per served
runlist (one host-driven engine per runlist of the served FIFO table): index 8 `MAX_CHANNEL_GROUPS_PER_ENGINE` (channels of
that runlist) and index 9 `CHANNEL_GROUPS_IN_USE_PER_ENGINE` (held by the host and all other clients)
(`ogkm kernel_fifo_ctrl.c:317-334`). `kf_abi::chanbudget::resolve`:
- **limit** = the smallest `total - in_use`, at most 2048 (the guest's token carries 11 chid bits);
- **default** = the smallest `total - in_use - total/8`: the host keeps one eighth of each runlist for its desktop and other clients;
- **minimum** = 256 (4x the ~64 an idle Windows desktop holds).
`RESERVE_DIVISOR = 8` and the minimum are the only policy constants. **Owner decision pending** on both; nothing else is a heuristic.

## The property
`kf3-gpu,channel-budget=<N>` (QEMU device property, ABI 26, set in the `-device kf3-gpu,...` string like `fb-mb` and `bar1-size`; e.g. `-device kf3-gpu,fb-mb=8192,channel-budget=512`). `0`/unset = derived. Realize **refuses by name** (never clamps): a value above the limit
(host free, or 2048), a value below 256, a host that refuses the query, and a default below 256 on a nearly full host.
Multi-VM hosts: each VM's default is computed from what is free at its realize; to divide a GPU fairly set `channel-budget`
on each VM explicitly (their sum should stay under the host's free channels).

## At the limit
The birth is refused by name with `NV_ERR_INVALID_STATE` (0x1a) to the guest, one loud log line, `birth_refused_cap`
incremented. Other VMs and the host are unaffected (the cap is the VM's own; nothing is taken from the host past it).

## Tests
`kf-abi chanbudget` (derivation, override, over-limit, minimum, full host, request/reply layout); `kf-qemu rmfacts` (realize-time
resolution over a fake host: default, override, guest-told count follows the property, refusals, ceiling = token extent);
`kf-core` `a_hostile_guest_past_its_channel_budget_is_refused_by_name_per_runlist`; `wire_mirror` (C signature, ABI 26).
