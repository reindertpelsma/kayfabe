# V3 — GSS-legacy controls carried to the host RM (`KF3_GSS_NATIVE`)

**STATUS: LIVE (default OFF, EXPERIMENT `KF3_GSS_NATIVE=1`), 2026-10-09 — code and GPU-free tests on branch
`claude/gss-native-20261009`, NOT hardware-verified. The product policy question (section 6) is the owner's.**
Branched from `claude/recovery-wall-20261009` (so one binary carries this flag, `KF3_WIN_KERNEL_PID4` and
`KF3_SCHEDULE_LATE_JOINERS`). Code: `kf_rm::gssnative` (`crates/kf-rm/src/gssnative.rs`), the object seat's
hook (`kf_rm::rmrpc::ObjectPolicy::with_gss_native`), the act (`kf_qemu::chan::ChanPlane::gss_forward`), the host
call (`kf_host::HostRm::raw_control_opaque`), the held reply's patch (`kf_gsp::Deferred::set_reply_patch`).

Convention: **[measured]** names a log line, a run or a source line read; **[inferred]** is reasoning from them.

## 1. Why

**[measured, reported by the coordinator from the Windows runs]** kayfabe refuses about 190-200 RM controls per Windows
boot with `NV_ERR_NOT_SUPPORTED` (`0x56`) that real hardware's GSP answers `0`. Among them is the GSS-legacy family
(bit 15 of the command set): `0x20809004` (1544 B), `0x2080b201` (2188 B), `0x2080852e` (528 B), `0x2080852f` (776 B),
`0x2080a0d1` (2024 B, about 1 Hz), `0x2080a0a8`, `0x20809037`, `0x20808539`, `0x20809038`, `0x2080a080`, `0x2080a097`
(`docs/design/V3_REFUSAL_AUDIT.md`, `docs/reference/gsp_control_classification.tsv`: `GSS-LEGACY`, no SDK define, no
params struct). kayfabe cannot answer them from a table: the bodies are GSP firmware's. The only table answers it has
are the measured ones (`kf_abi::gssreplay`, `kf_abi::gsslegacy`). This experiment asks the one party that can: the host.

## 2. What the flag does

`KF3_GSS_NATIVE=1`, read once in `Device::realize` (said once: `kf3: ⚠ EXPERIMENT KF3_GSS_NATIVE ON ...`; the status
line ends ` EXPERIMENT KF3_GSS_NATIVE gss[...]`). Off, the segment is absent and no code path is taken: the object seat
has no GSS link and the line is the line it was.

A `GSP_RM_CONTROL` that no earlier link answered, reaching the object seat, is **forwarded** iff all of:

| test | else |
|---|---|
| the command was parsed from the RPC (`decode_rpc_control`) | falls through (never defaulted into this rule) |
| `cmd & 0x8000` (GSS-legacy) | not ours: every ordinary control and every display control (`0x73xxxx`, `0x5070xxxx`) |
| `(cmd & 0xC000) != 0xC000` (not the privileged pattern) | refused as today (`0x56`) |
| `cmd >> 16 == 0x2080` (subdevice class) | refused as today |
| not FINN-serialized | refused as today |
| the object is a live `NV20_SUBDEVICE_0` in the guest's object graph (`RmObjects::is_subdevice`, the graph the object seat already keeps) | refused as today |
| `paramsSize <= 64 KiB` | refused as today |
| the declared params lie inside the declared RPC payload (the reply is clamped to it, `RpcCommand::reply`) | refused as today |
| fewer than 20 000 forwards this boot | refused as today |

"Refused as today" is literal: the link returns `None`, the chain reaches the unserviced ledger, the FSM posts `0x56`
with an empty body. Each reason also bumps `refused` and is said once. The test
`what_is_not_forwardable_is_answered_exactly_as_with_the_experiment_off` compares reply, cell and ledger count against
a chain built without the seat for every row above.

**The host call.** `paramsSize` bytes (the guest's declared size, no padding) are copied into a host-side buffer and
issued as one `NV_ESC_RM_CONTROL` on kayfabe's host client, **on kayfabe's host subdevice** (the guest's handle is only
logged), with the same `cmd`. The host's `NV_STATUS` is returned unmapped. On `0` the host's bytes (exactly `paramsSize`)
replace the reply's params; on any other status the guest reads that status in the envelope (the place the existing
refusals use; `rpcRmApiControl_GSP` returns it, `ogkm-595.84: rpc.c:11221-11234`: the transport status is returned first) and no bytes. If the host cannot be asked
at all (ioctl error) the guest reads `NV_ERR_INVALID_STATE` (`0x40`), never `0`. Nothing is echoed, defaulted or invented.

**Threads (constraint 35).** On the drainer the link only classifies, copies and queues; it makes no host call. The act
(`ChanPlane::gss_forward`) runs on `kf3-chan-act` in statement order, the reply is HELD on a `Deferred` (the FSM's
existing mechanism), and the act resolves the cell with the host's status and, before that, sets the reply patch the FSM
applies when it posts. A host call that blocks on the host RM API lock blocks the act queue; it is **not hidden**: it
shows in `acts_run` / `act_worst_us` / `act_total_us` of the existing status line. `gss[off_act=N]` appears only if
the host call ever ran on another thread (it must never). Per-command in-flight is 1 by construction: a GSP client's
RPCs are synchronous and the act queue is one FIFO (`a_repeat_while_the_first_is_pending_waits_behind_it`).

**Observability.** `gss[fwd=N ok=N host_err=N refused=N top=0x........:N]` (atomics only; `top` is the most forwarded
among the first 16 distinct commands). Once-lines, no per-RPC print (the act thread's own per-act line is suppressed for
this act): `kf3: GSS-NATIVE forward cmd 0x2080a0d1 paramsSize 2024 host status 0x0` for each of the first 16 distinct
commands, and `kf3: GSS-NATIVE refused cmd ... : <reason> (0x56; said once per reason)`. `klog_limited!` does not exist
in this tree; the bounded/once idiom of `crate::bar1phys::bounded` and `AtomicBool::swap` is used.

## 3. Safety argument, and what is not shown

### 3.1 The privilege boundary (the part that is CPU-RM's, read)

**[measured, source read]** `ogkm-595.84 src/nvidia/interface/deprecated/rmapi_deprecated.h:41-43` defines
`RM_GSS_LEGACY_MASK 0x8000`, `..._NON_PRIVILEGED 0x8000`, `..._PRIVILEGED 0xC000`.
`rmapi_deprecated_control.c:95` (`IsGssLegacyCall(cmd) = !!(cmd & 0x8000)`), `:130-136` (a GSP client routes every such
command to `RmGssLegacyRpcCmd`). `rmapi_gss_legacy_control.c:33-37`: *"Some clients are still making these legacy GSS
controls... just forward all of them to GSP and let it deal with what is or isn't valid"*; `:56-60` returns
`NV_ERR_INSUFFICIENT_PERMISSIONS` for the `0xC000` pattern unless `privLevel >= RS_PRIV_LEVEL_USER_ROOT`; `:73` copies
`paramsSize` bytes in with one flat `portMemExCopyFromUser`, `:110-127` forwards the buffer, `:145-151` copies it back
out on `NV_OK`. The guest's own RM does the same toward its (virtual) GSP: `vgpu/rpc.c:11127` and `:11264` (cache
read / write for GSS-legacy replies).

**[inferred]** the privilege boundary is therefore *the host RM's*, but it checks the **host client's** privilege, and
kayfabe's host client is whatever user QEMU runs as (root on the benches). The host would admit a `0xC000` command from
a root client. **kayfabe's own refusal of the `0xC000` pattern is the boundary that matters**, which is why the rule is
`(cmd & 0xC000) == 0x8000` and not nvkvm-pv's `cmd & 0x8000` (`/workspace/nvkvm-pv/src/qemu/nvkvm_ctrl_allowlist.h:368-385`,
which also admits the privileged variant). The class gate (`0x2080` + a live guest subdevice) is kayfabe's too; the host
sees only kayfabe's own subdevice.

### 3.2 "The params never contain CPU pointers" — what I found

* **gVisor nvproxy** **[measured, source read]** `pkg/sentry/devices/nvproxy/frontend.go:894-905`: a command with
  `RM_GSS_LEGACY_MASK` set is passed through `rmControlSimple` because it is *"a 'legacy GSS control' that is implemented
  by the GPU System Processor. Consequently, its parameters cannot reasonably contain application pointers, and the
  control is in any case undocumented"*; `version.go:84-90`: such commands *"are not versioned"*. ⊘ This is an argument
  **by construction**, not a measurement of any control's params.
* **ogkm** **[measured, source read]** the same follows from `rmapi_gss_legacy_control.c:73,110-127,145-151`: one flat
  copy of `paramsSize` bytes each way, no FINN, no `RMAPI_PARAM_COPY` descriptor, **no pointer fix-up**. A user pointer
  embedded in the buffer would arrive at GSP as an integer no one translates, so no working GSS-legacy control can rely
  on one. **[inferred]** that makes CPU pointers in these params a dead field at worst.
* **nvkvm-pv** **[measured, source read]** forwards them: `src/qemu/nvkvm_ctrl_allowlist.h:368-385` (`if (cmd & 0x8000u)
  return NVKVM_CTRL_ALLOW_GSS`, "the wildcard", reported as `CTRL-AUDIT wildcard-only ALLOW` once per command,
  `src/qemu/nvkvm_isolate_handlers.c:118-145`), and the 2026-08-29 lesson at `:3608-3636` (a sentinel cmd of
  `0xffffffff` has bit 15 set and *skipped the whole allowlist*: classify only from the cmd actually parsed; an unreadable
  cmd is denied where it is discovered). Taken here: `GssNative::control` classifies only from `decode_rpc_control`, a
  failure falls through to the ledger.
* **⊘ What I could NOT find: any measurement.** This repo's own audit says it plainly:
  `archive/nvkvm/docs/internal/audit-guest-pointers.md:200-215` lists `0x2080a0d1` (and `0x2080852e`, `0x2080852f`,
  `0x20808159`, `0x20808162`) among 13 ids absent from the open tree whose *"params layout — and therefore whether they
  carry pointers — cannot be determined from the open tree"*, and of the two wildcards: *"The comment justifying them...
  asserts both are 'GSP-routed, no app pointers'. That assertion is not verifiable from the open tree and is not verified
  here."* No params dump of `0x20809004`, `0x2080b201` or `0x2080a0d1` is in the tree
  (`docs/reference/gsp_control_classification.tsv` carries measured in/out sizes for a few ids only). I did not read a
  CPU pointer in any params of these controls, because I have no params to read. **The no-CPU-pointer claim is by
  construction in two independent sources and measured in none.** Per the task's escalation rule: no CPU pointer was
  found, and none could be.
* **The echo precedent that must not be repeated** `crates/kf-abi/src/gsslegacy.rs:1-60`: the C artifact answered GSS
  controls by echoing the request under `NV_OK`; for `[OUT]` params that is a zero body the CUDA runtime read as real
  data (`cudaErrorInitializationError`, no log line). The forward returns the host's status and bytes only. The one
  measured identity (`0x20808159`, 332 bytes unchanged) is still answered earlier in the chain, not here.

### 3.3 Residual risks of an opaque forward (stated, not solved)

1. **Not read-only.** "Non-privileged" is a host-RM permission class, not "harmless". A `SET`-shaped GSS-legacy control
   the host admits to an unprivileged client changes **host GPU state** (clocks, power, perf limits). The experiment
   forwards whatever the guest sends, on a 20 000 per boot budget.
2. **Addresses are not CPU pointers but are addresses.** A control whose params carry a GPU VA or a DMA address would be
   interpreted against **kayfabe's host client** (its GPU VA space, the host IOMMU domain), not the guest's. Unknown for
   these ids (3.2).
3. **Layout skew.** The params are in the guest driver's layout and are handed to the host's GSP unchanged;
   `raw_control_opaque` deliberately skips the host-ABI carry gate (`HostAbi::control_carry`: for a control with no
   header it **refuses** any host outside `[580.65.06, 581)`, and the trusted host runs 595.91.07, so the gated path
   would make this experiment inert there). A mismatched layout reaches GSP, which validates `paramsSize` and answers
   its own error; that answer is returned. **⊘ Design judgment flagged for the owner:** the gate protects layouts
   kayfabe *writes*; here nothing is written, but "assume the guest's layout is the host's" is an assumption, not a fact.
4. **Reply caching in the guest.** The reply's `rmctrlFlags`/`rmctrlAccessRight` are zeroed by `StickyAnswerGuard` (every
   accepted control reply crosses it), so the guest cannot cache a forwarded answer (`rpc.c:11264-11270` needs them).

## 4. Cross-tenant leak (a policy question, not an experiment question)

The telemetry controls (`0x2080a0d1` perf sample, `0x2080a0a8`, `0x20809037`, `0x20808539`, the NvTopps loop) describe
the **whole GPU**: clocks, power, utilisation and so on, including other VMs' and the host's own load. For a single-guest
experiment that is the point; for the product it is the same policy question as per-VM RUSD (a per-VM view of host GPU
telemetry): forward, synthesise per VM, or refuse. This flag does not answer it and must not be on in a shared product.

## 5. The display exclusion

Display controls (`0x73xxxx`: `0x730285`, `0x730288`, `0x7302a5`; `0x5070xxxx`) have no bit 15 and address the **display
object**, not the subdevice. They are never forwarded, by three independent tests (no bit 15; class is not `0x2080`;
the object is not a subdevice) and the table test above. Forwarding them would put **host display state** (the host's
monitors, modes, heads) into the guest's virtual monitor, which kayfabe models itself (`V3_DISPLAY.md`). They stay
answered or refused exactly as they are. `refused` counts only bit-15 commands, so it never counts a display control.

## 6. Hardware falsifier (not run)

Boot Windows as the last run with `WIN_FLAGS` gaining `KF3_GSS_NATIVE KF3_WIN_KERNEL_PID4 KF3_SCHEDULE_LATE_JOINERS`.
Check the realize log shows `EXPERIMENT KF3_GSS_NATIVE ON` and the status line ends ` EXPERIMENT KF3_GSS_NATIVE gss[...]`.
**Predicted if the idea is right [inferred]:**

1. the GSS-legacy refusals per boot (`gsp_refusals[...]` rows whose control id has bit 15) drop from about 190 to near 0;
   the display controls (`0x730285`, `0x730288`, `0x7302a5`, `0x5070xxxx`) remain;
2. the guest's NvTopps loop (about 1 Hz `0x2080a0d1`, `0x2080a0a8`, `0x20809037`, `0x20808539`) appears in the GSP stream
   with `gss[fwd=...]` growing and `ok` close to `fwd`; the first-status once-lines say what the host answered;
3. the live guest log shows `NvapiAdapter initialized`.

**Falsifiers:** `host_err` close to `fwd` (the host refuses what real hardware's GSP accepted: the layout-skew risk 3,
or a host driver that does not answer these ids); `off_act` non-zero (a threading defect); the refused GSS count not
dropping (the commands are not reaching the object seat, or the subdevice check refuses them: read the once-lines);
the guest's behaviour unchanged while `ok` rises (the refusals were not what it was waiting on).

**Not promised:** whether the early deaths change. The class A deaths end after answered `SET_TIMESLICE` calls, not at a
refused control, so this flag is not expected to move them. A change would be a finding, not the aim.

## 7. Tests (GPU-free)

`crates/kf-rm/tests/gss_native.rs` (11 tests, through the whole served chain, a fake host and a fake act thread named
like the plane's): forward + the host's reply and status reach the guest (the host is gated shut and `respond` still
returns: the drainer neither waits on nor makes the call; the host records its thread name = `kf3-chan-act`); host
refusals `0x56/0x1d/0x1f/0x3` returned as the host said, no bytes; a host that cannot be asked gives `0x40`, never OK;
a table of non-forwardable controls (`0xC000` pattern, `0x730285/288/2a5`, `0x50700000`, device-class bit 15, no bit 15,
non-subdevice and unknown objects, serialized, declared-bigger-than-present, oversized, unparseable) answered exactly as
with the experiment off; display and `0x2080_0123` never counted as refused; the 64 KiB cap is inclusive; zero-length
params; the (cap+1)th forward refused; a repeat queues behind the first; `off_act` detects a call from the wrong thread;
a dead act thread refuses as today. `crates/kf-gsp/src/boot.rs`: the reply patch lands only on success, only inside the
reply, never grows it, never on a refused reply. `kf_rm::gssnative` unit tests: the id rule on the ids seen, and the
counter formats.

Not covered, because no GPU: a real host answering a real GSS-legacy control; the host-side blocking behaviour (the
`actq` head-of-line); whether the host's reply sizes equal `paramsSize` for every id.
