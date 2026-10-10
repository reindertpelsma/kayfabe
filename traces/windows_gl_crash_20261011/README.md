# Windows OpenGL / CUDA crash, 2026-10-11

**STATUS: LIVE, 2026-10-11.**

## HANDOFF (top)
Root cause MEASURED offline from run 501's `qemu.log`; fix in `claude/gl-icd-crash-20261011` (build + hardware verification pending, see Log).

## Log (newest first)
- **Cause (measured, run 501 qemu.log, host `/var/lib/kf-windows-20261005/boundary-kayfabe-501/`)**: `kf3: act birth passthrough REFUSED (0x1a): token 0x41: OverDeclaredCap { twin: Channel, cap: 64, asked: 65 }`
  followed by `GSP REFUSED fn103/0x0000c56f=0x1a (GSP_RM_ALLOC)`. The log holds 5 such refusals at lines 52660, 52940 (Minecraft
  launches), 79364, 79955, 80343 (the three kf_glgears launches); a replay of BORN/released lines shows ~64 live channel twins on an idle
  Windows desktop, so the first process that needs one more (OpenGL ICD, CUDA `cuCtxCreate` -> 999) is refused. The watcher's
  `birth_refused` counter does not count this refusal class (it stayed 0), which is why it looked like "no refused birth".
- The cap is a hardcoded `VmCaps::from_declared(64, ...)` in `kf-qemu/src/chan.rs`, contradicting `kf-core/src/caps.rs` ("the cap is the
  number we told the guest"): the guest is told `numChannels = 0x800` per runlist (`AUTHORED_FIFO_CHANNELS`).
- Fix: `kf_rm::authored::declared_channel_cap(row, engines)` = channels_per_runlist x distinct host-driven runlists, passed to `ChanPlane::new`.
- **Inferred (not yet measured)**: that the ICD dereferences the failed channel alloc at fault offset 0xb6d516; hardware run pending.
- Not needed after the measurement: WER dump and KF3_RPC_TRACE run (kept as fallback if the hardware check fails).
