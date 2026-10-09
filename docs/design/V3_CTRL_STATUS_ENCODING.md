# V3 — Where a refused control's status travels (`KF3_CTRL_STATUS_IN_BODY`)

**STATUS: LIVE (default OFF, EXPERIMENT `KF3_CTRL_STATUS_IN_BODY=1`), 2026-10-09 — code and GPU-free tests on branch
`claude/ctrl-status-body-20261009`, NOT hardware-verified. The protocol-fidelity finding is measured; that it explains the
Windows sign-in crash is NOT: it is an inferred suspect, and the hardware falsifier in section 5 decides it.**
Branched from `claude/gss-native-20261009` (so one binary carries this flag, `KF3_GSS_NATIVE`, `KF3_WIN_KERNEL_PID4` and
`KF3_SCHEDULE_LATE_JOINERS`). Code: `kf_gsp::GspFsm::wire_form` (`crates/kf-gsp/src/boot.rs`), switched on in
`kf_qemu::device::Device::realize`.

Convention: **[measured]** names a log, a run or a source line read; **[inferred]** is reasoning from them.

## 1. The finding

**[measured by the coordinator, 2026-10-09; not re-run for this document]** The real GSP answers a *failed*
`GSP_RM_CONTROL` (RPC function 76) with the VRPC header `rpc_result` = 0 and the control's NV status in the reply
**body**: `rpc_gsp_rm_control_v` is `hClient`@0, `hObject`@4, `cmd`@8, `status`@12, `paramsSize`@16, `flags`@20, then the
params, after the 32-byte VRPC header. Evidence:

- `/var/lib/kf-windows-20261005/vfio-dvi-20261008/boot3/gsp.jsonl` (real hardware, VFIO): 43 control replies with a non-zero
  body status and header 0, for example `0x730285` -> `0x56` x7 and `0x1f` x4, `0x730288` -> `0x1f` x8.
- `/var/lib/kf-windows-20261005/vfio-ablation-20261009/runs/*/` (VFIO ablation boots, rule-rewritten controls): header 0,
  body `0x56`/`0x57`.
- The guest side agrees (**[measured]**, ogkm `src/nvidia/src/kernel/vgpu/rpc.c`, `rpcRmApiControl_GSP`):
  `if (rpc_params->status != NV_OK) status = rpc_params->status`, and it stays quiet for `NV_ERR_NOT_SUPPORTED` and
  `NV_ERR_OBJECT_NOT_FOUND`. A failed **alloc** (function 103) is different on real hardware and kayfabe already matches it:
  both the header and the params `status` are set.

**[measured by the coordinator]** kayfabe does the opposite for controls. In the kayfabe boundary boots 152 and 190
(`gsp.jsonl`, same parsers: `/tmp/bs3.py` on the host, `traces/windows_reset_20261009/run104/tools/*.py`) 302 and 351
control replies have header `rpc_result` = `0x56` and body status 0 (`(76, hdr!=0, body=0)`). For allocations (function 103)
it sets header and body, as the real GSP does.

**[inferred]** In the guest's RM, `_issueRpcAndWait` treats a non-zero header `rpc_result` as an RPC failure (error log, RPC
history, possible escalation), a different path from a normal reply that carries an error status inside. A Windows driver
may treat repeated header-level RPC failures as a GSP or hang indication. The Windows guest dies at the lock-screen
sign-in under kayfabe (`VIDEO_TDR_TIMEOUT_DETECTED` `0x117`, then `0x116`) and survives it on real hardware, and the
sign-in sends many controls kayfabe refuses. This is a protocol-fidelity gap and a *suspect*, not a shown cause.

## 2. Where kayfabe builds a refused control's reply (list)

One encoding point is enough: every one of these ends in `OutgoingRpc.rpc_result`, posted by `GspFsm::post` (or, for a
joined large control, split by `large::split_reply` after `answer_large`).

FSM, `crates/kf-gsp/src/boot.rs` (line numbers on this branch):

- `answer`, no policy answered (`Unserviced`, the default named refusal): `:2340-2355` (`cmd.reply(NV_ERR_NOT_SUPPORTED, &[])`
  for a control; `reply_alloc(...)` for an alloc).
- `answer_large`, no policy answered: `:2217-2224`. `drain_commands`, a `LargeRefusal` (`LARGE-RPC REFUSED`): `:2086`.
- `settle_deferred` (`:498-535`), a host act's final status written over a held reply (`rpc.rpc_result = status`, `:527`).
- `release_held` (`:2640-2660`) and `answer` (`:2395-2400`) post the reply and then note the ledger.
- Constructors: `RpcCommand::reply` (`crates/kf-gsp/src/rpc.rs:489`, body clamped to the request's size, zero-filled),
  `reply_alloc` (`:628`), `ack` (`:673`).

Policies in `kf-rm` (a `Reply { rpc_result: <non-zero>, body }`; `body` is empty, so the FSM zero-fills it, except where
noted):

- `crates/kf-rm/src/rmrpc/policy.rs:707-720` `refusal_reply` (every `BridgeRefusal`, including
  `GspRuleControlUnserviced`, `rmrpc/mod.rs:683`, status `NV_ERR_NOT_SUPPORTED` from `rmrpc/mod.rs:934`);
  `policy.rs:825` (deferred-API refusals).
- `crates/kf-rm/src/chanlink.rs:805-810` `ChannelPolicy::refusal`, which is every `ChanAnswer::Refused` and every channel
  plane refusal (about 30 call sites between `:881` and `:1946`); **its body is the request payload (an echo)**, so with
  the switch on the echo stays and only the status word is overwritten.
- `crates/kf-rm/src/sticky.rs:413`, `staticinfo.rs:515,529,545`, `guestsysinfo.rs:162`, `inittables.rs:1781,2040`,
  `barpde.rs:200`, `display.rs:598`, `zbc.rs:237`, `vfguest.rs:245`.

Instruments that already read the **logical** status (the `Reply`, before the FSM) and so are unchanged by the switch:
`crates/kf-rm/src/census.rs:380` (`rpc-trace ... result=`) and `:499` (`note_served`), `census.rs:522,532` (arming).
The refusal ledger (`crates/kf-gsp/src/refusal.rs`; `kf3: GSP REFUSED` at `crates/kf-qemu/src/device.rs`
`log_fresh_refusals`; `gsp_refusals[...]` in the heartbeat) is noted from the `OutgoingRpc` the caller holds, which keeps the
logical status. Scripts that read those lines (`scripts/bench/refusal_audit/rf_hook.sh`, `rf_ledger.py`,
`scripts/bench/windows/rpc_diff.py`) therefore see the refusal exactly as before. The GSP observer
(`x-gsp-observer`, `tools/vfio-gsp-observer/`) records the **wire**, so with the switch on `gsp.jsonl` shows header 0 and
the body status, which is the real-hardware shape its parsers already expect.

## 3. What the flag does

`KF3_CTRL_STATUS_IN_BODY=1`, read once in `Device::realize` and handed to the GSP state machine
(`GspFsm::with_ctrl_status_in_body`); said once (`kf3: ⚠ EXPERIMENT KF3_CTRL_STATUS_IN_BODY ON ...`); the status line ends
` EXPERIMENT KF3_CTRL_STATUS_IN_BODY encoded=N` (N = replies re-encoded). Off, the segment is absent and no code path is
taken: every posted byte is today's (a GPU-free test compares the ring bytes).

`GspFsm::wire_form` runs in `GspFsm::post` and, for a joined large control, in `answer_large` **before** the split (the
guest reads `rpc_result` from the last message of a large reply, so every fragment must carry 0). For a reply whose
function is `GSP_RM_CONTROL` and whose `rpc_result` is non-zero it:

- writes the status at `rm_control_wire().status_off` (12 at every measured tag) of the body,
- sets `rpc_result` and `rpc_result_private` to 0,
- leaves the body's size and every other byte as the policy built them (the zeroed OUT area for a refusal, never an echo;
  a policy that returns partial OUT data keeps it; a stale word at offset 12 is replaced by the logical status),
- does nothing to a body too short to hold the word (no byte is invented).

`GSP_RM_ALLOC`, `FREE` and every other function are untouched, and so is any reply with status 0. The ledger, the
`Unserviced` report, the `GSP REFUSED` lines and the census stay keyed on the logical status. A device reset keeps the
switch (it is configuration, like the console cell).

What the flag does not do: the real GSP also echoes `hClient`, `hObject`, `cmd` and `paramsSize` into the refusal body;
kayfabe's refusal body has zeros there (today and with the flag on). **[inferred]** the guest reads only `status` and the
params on this path, so that was left alone; if the falsifier is inconclusive, filling those four words is the next
step. Protocol refusals of a malformed large fragment (`LARGE-RPC REFUSED` on a continuation record) stay header-level:
no real GSP counterpart exists.

## 4. Why it is an experiment and not the default

- A Linux guest works with the current encoding, and the change touches **every** refused control (hundreds per boot,
  across the Linux CUDA, graphics and video lanes). A wrong guess here is not local.
- Linux treats a header-level failure as a quiet `NV_ERR_NOT_SUPPORTED` for the controls it probes; with the switch on the
  same controls arrive as a normal reply carrying a status. That is what the real GSP does, but it is a different guest
  path, and no Linux lane has run it.
- The Windows crash cause is not shown. Making the switch the default before the falsifier is run would merge a
  whole-protocol change on a suspect.

## 5. The falsifier (hardware, owner-run)

With `WIN_FLAGS=KF3_CTRL_STATUS_IN_BODY` (the broker turns a bare word into `=1`), the **quiet Windows boot**
(the configuration of the boundary runs 190, 181 and 211, which each died at the lock-screen sign-in):

- it **survives** the sign-in: the encoding is the cause, or part of it. Next: Linux re-verification (below), then the
  default is the owner's decision.
- it **dies the same way** (`0x117` then `0x116`, at the sign-in): the encoding is exonerated as the cause. The flag stays
  an experiment; the refusals the sign-in sends remain the suspect (compare the control list with the real-hardware
  boot3 list, section 1). Read the heartbeat's `encoded=N` to confirm the switch was active (N > 0, about the refused
  control count of the boot).
- either way, `gsp.jsonl` of the run should now show `(76, hdr==0, body!=0)` for the refusals (parse with `/tmp/bs3.py`),
  the same shape as `vfio-dvi-20261008/boot3`.

Before the flag could become a default, Linux must be re-verified on a bench box, strictly serial:
`scripts/bench/v3_gates.sh` (9/9), `build_kf3.sh`, the fast suite `KF_DEVICE=kf3 scripts/fastguest/fast_suite.sh <tag> 180`
(30/30) **with the flag on**, and the Linux broker lane. Any new `Xid`, guest `NVRM:` error line or changed `gsp_refusals`
count against the flag-off baseline of the same revision is a regression.

## 6. Tests (GPU-free)

`cargo test -p kf-gsp --lib ctrl_status_in_body` (fake guest RAM, a bound queue pair, the guest's own decode of what was
posted): a refused control has header result 0 and body status `0x56` at offset 12 with the request's params length and a
zeroed OUT area; flag off is byte-identical to the message encoded independently of the switch; refused, unserviced and
successful allocs, refused frees and refused non-control functions, and a successful control, produce identical RAM in
both modes; held replies and joined large replies are encoded at the wire; the ledger row carries the logical status; the
offset is pinned against a hand-written ogkm-layout body (`hClient`, `hObject`, `cmd`, `status`, `paramsSize`, `flags`).
