# V3 — fair GPU sharing between VMs ("vGPU-like" scheduling)

**STATUS: DESIGN-ONLY, 2026-10-02 (owner idea, not built).** Owner: *"later for multi tenancy, as we
actually control the TSGs, we can maybe also issue the TSGs in such a way [the GPU] is fairly shared
between VMs, vGPU-like."*

## 1. What kayfabe already controls

kayfabe authors every host TSG: one host `KEPLER_CHANNEL_GROUP_A` per guest `(client, TSG, context
share)` (`kf-qemu/src/chan.rs` `groups`; `kf-host/src/channel.rs` `birth_group`/`birth_member`), and every
group-level control the guest issues is re-issued by kayfabe per host group. So a per-VM policy has one
place to live. The levers, all from kayfabe's unprivileged host verb set unless marked:

| lever | today | for fairness |
|---|---|---|
| timeslice (`NVA06C_CTRL_CMD_SET_TIMESLICE`) | the guest's value, applied per host group; host RM rounds/refuses | per-VM budget ÷ the VM's active host groups, rebalanced on birth/free; the guest's own values become relative weights inside its share |
| ctxsw preemption mode (`NV2080_CTRL_CMD_GR_SET_CTXSW_PREEMPTION_MODE`) | the guest's requested mode applied per host group | force preemptible modes (compute CTA/CILP, graphics GFXP) so one long kernel cannot hold the engine past its slice |
| interleave level (`NVA06C_CTRL_CMD_SET_INTERLEAVE_LEVEL`) | not used | how often a group recurs in the runlist; ⚠ check in ogkm whether an unprivileged client may set it |
| disable/enable (`NV2080_CTRL_CMD_FIFO_DISABLE_CHANNELS`, already in the host set) | used for teardown paths | hard caps: throttle a VM over its share for a window |
| GPU memory | each VM's store is a fixed size (`fb-mb`) | already a hard per-VM memory partition |
| NVENC sessions | slots held per guest client | per-VM session caps |

## 2. Policy sketch

- **Equal or weighted share:** total timeslice per VM proportional to its weight; per-group slice =
  VM budget ÷ active groups (so a VM cannot buy more GPU time by opening more contexts).
- **Preemptibility is not the guest's choice:** a guest may ask for a MORE preemptible mode, never a
  less preemptible one than the policy floor.
- **Hard caps (vGPU's "fixed share"):** throttle via disable/enable over a window. ⚠ Needs a per-VM GPU-time
  accounting source; none is identified yet — find one before promising caps.

## 3. Caveats

- The runlist algorithm itself is NVIDIA's (round-robin over groups with timeslices and interleave);
  kayfabe shapes it, it does not replace it. Host processes outside kayfabe are not governed.
- Small slices raise context-switch overhead; large GR contexts make preemption expensive.
- Async-compute/MPS concurrency is already lost to per-context-share host groups (see
  `V3_APP_MATRIX`-era note in `kf-qemu/src/chan.rs` `groups`); mirroring subcontexts exactly would
  interact with this policy.

## 4. First measurement

Two VMs on one GPU: one runs a hostile long-running kernel (no preemption points, many contexts), the
other an interactive desktop or a latency-sensitive inference loop. Measure the second VM's frame time /
latency with today's pass-through policy vs the sketch above.
