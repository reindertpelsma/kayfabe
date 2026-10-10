# V3 — the recovery wall: the System process (pid 4) as the kernel driver's process

**STATUS: LIVE (default OFF, EXPERIMENT `KF3_WIN_KERNEL_PID4=1`), 2026-10-09 — code and GPU-free tests on branch
`claude/recovery-wall-20261009`, NOT hardware-verified. Owner decision pending (section 6).**
The analysis it implements is section 11 of `traces/windows_reset_20261009/README.md` on branch
`claude/windows-reset-20261009` (referenced, not edited here). Code: `kf_rm::chanlink::windows_user_work`
(`crates/kf-rm/src/chanlink.rs`), switch read in `Device::realize` (`crates/kf-qemu/src/device.rs`).

Convention: **[measured]** names a log line or source line; **[inferred]** is reasoning from them.

## 1. The wall

**[measured, run 113, `/var/lib/kf-windows-20261005/boundary-kayfabe-113/qemu.log`]** Before the first
`UnloadingGuestDriver` (line 56868) the classifier learns the kernel driver's process id from the first
guest-kernel, non-RM-internal channel: `ProcessID=0x350` (line 8493, `0xc1d00013:0xff040000`). Every
kernel-driver channel after it declares 0x350 (14 channels) and the compositor and D3D clients declare their
own (0x3b8, 0x554, 0x54c) and are classified `USER WORK -> Passthrough`. After the unload, **every** channel
declares `ProcessID=4`, including the RM-internal ones (lines 57239, 57606) and the restart's:

| line | channel | engine | ctx share | pid | classified |
|---|---|---|---|---|---|
| 63705 | `0xc1d00049:0xff040000` | CE (0xb) | 0 | 4 | `USER WORK -> Passthrough` (pid != 0x350) |
| 63777 | `0xc1d0004b:0xff040001` | GR (1) | 0 | 4 | kernel work -> Translated (no context share) |
| 63779 | same | | | | `birth REFUSED ... KernelInUserSpace(1)`, RmAlloc `0x40` |
| 63912 | `0xc1d0004e:0xff040002` | CE (0xb) | 0 | 4 | `USER WORK -> Passthrough` (a second space) |

Both of the first two are in `VasKey(13965662697862203504)`; the T-space rule (`V3_P1P2_TSPACE.md` section 4.2,
`TwinState::try_kernel`) refuses a Translated channel in a space a user channel runs in. The guest reads
status `0x40`, StartDevice fails, the live dump follows, then bugcheck `0x116`.

## 2. Where the process id comes from (answer to "is it guest-controlled?")

**[measured, source]** `ProcessID` is the field at offset 296 of `NV_CHANNEL_ALLOC_PARAMS` (generated layout
`kf_abi::generated::matrix::NV_CHANNEL_ALLOC_PARAMS`, `chanlink.rs` `on_alloc`, via `layout_u32(.., "ProcessID")`).
It is **bytes of the guest's RmAlloc RPC**. The host derives nothing: there is no host-side pid lookup, and no
other field is read. In an honest guest the guest kernel's RM stamps it from the process that created the RM client
(`docs/design/fable_leg_b_solution_space.md:33,198`, `client.c:112` / `kernel_channel.c:293`, **[source]**), so
a user process cannot choose it through normal APIs; but a hostile guest (rules: guest userspace and root are
untrusted) writes whatever it likes into the RPC, so kayfabe must treat it as a free-form guest declaration.
The kernel driver's pid is also learnt from guest bytes (the first kernel-stamped, non-internal channel;
`windows_kernel_pid`, never reset: after the reload it is still 0x350).

Consequence, **[inferred from the code]**: the Passthrough route is *already* reachable by any guest that
declares a pid other than the learnt one (with a context share for GR). `windows_user_work` documents this: it is
"not a security boundary"; a misclassified channel only loses the Translated route's inspection, and a
Passthrough twin is an unprivileged host channel in this VM's own VA space (fail closed in hardware).

## 3. What the flag does

`KF3_WIN_KERNEL_PID4=1` (read once; `kf_rm::chanlink::kernel_pid4_enabled`, said once in the realize log and as
` EXPERIMENT KF3_WIN_KERNEL_PID4` at the end of the `kf3:` status line) adds one test to `windows_user_work`, after
the existing "kernel driver's own process" test:

```
if system_pid_is_kernel && pid == 4 { Err("the Windows System process (EXPERIMENT KF3_WIN_KERNEL_PID4)") }
```

`Err` means "not user work": the channel keeps the route it has by default (Translated for a guest-kernel-stamped
channel). It needs `KF3_WIN_USER_CHANNELS_PASSTHROUGH=1` to matter (without it nothing is user work).
With the flag off the function is the old one (the facts struct gains a `false` field), pinned by a test that
compares a 2430-case grid with a copy of the old rule.

**Safety, the question asked.** Can pid 4 make a guest USER channel less strictly checked? **No.** The new test
only returns `Err`; the only route change it can cause is Passthrough -> Translated, the *default* route of every
guest-kernel-stamped channel. A test over the whole grid asserts "Passthrough with the flag on implies Passthrough
with it off". A hostile guest that declares pid 4 on its own channel gets exactly what it would get with the
feature off: Translated, inspected, authored. Residuals: **(1)** a guest lying "pid 4" keeps its channels
Translated (slower, never unsafe); **(2)** it can make its own pid-4 channel un-bornable by declaring it next to a
Passthrough one it also owns (`KernelInUserSpace`) - a refusal of the guest's own channel, as it can already
produce today. **(3) [inferred]** the Translated route is the stricter one: it reads the ring and authors the
host actions; a user channel mis-stamped kernel being Translated is the case `kernel_channel`'s comment warns about
for the PRIVILEGE stamp, but that stamp is unchanged here and the flag moves no channel from Passthrough *into*
a kernel-stamped state (it only declines to reclassify one).

## 4. Option (b), for the record

Re-learn the kernel pid per adapter start (the first kernel channel after an all-free). **[inferred]** Not
simpler and not safer: "first channel after an all-free" is decided by guest timing and guest frees, the learner
would have to detect the all-free in a policy that sees only RPCs, and a wrong re-learn (a user channel first)
makes the *user* pid the kernel pid and every real kernel channel user work - the dangerous direction. (a) is a
constant in a pure function and can only decline reclassification. Only (a) is implemented.

## 5. Tests (GPU-free)

* `chanlink::tests::user_work::kernel_pid4_off_changes_nothing` - the whole grid equals the pre-flag rule;
  pid 4 is still user work with the flag off (the defect, pinned).
* `...::kernel_pid4_on_moves_only_pid4_and_only_to_translated` - other pids unchanged; pid 4 moves only to
  `Err`; flag-on Passthrough implies flag-off Passthrough.
* `...::run113s_restart_channels_are_translated_with_kernel_pid4` - the run-113 facts.
* `chanlink::tests::the_link_routes_a_pid4_channel_by_the_flag` - through `ChannelPolicy::respond` with real
  `NV_CHANNEL_ALLOC_PARAMS` bytes: pid 0x350 teaches the kernel pid; off, pid 4 CE is `user_work`; on, not;
  0x554 / 0x3b8 are user work either way.
* `kf-qemu` `twin::tests::a_pid4_restart_pair_shares_a_space_only_with_kernel_pid4` - the plane's own
  `TwinState` on that pair: off, the second birth is `KernelInUserSpace(1)` (run 113); on, both are born in one
  space; a pid-0x554 user channel still cannot join that kernel space.

## 6. Open owner decision

Whether pid 4 (the System process) is to be the kernel driver's process in `windows_user_work` by default
(option a), re-learned per start (option b, not recommended, section 4), or left as is. The flag stays default
off until the falsifier below passes on a box.

## 7. Hardware falsifier (for the coordinator's boot)

Boot Windows as run 113 with `KF3_WIN_KERNEL_PID4` and `KF3_SCHEDULE_LATE_JOINERS` added to `WIN_FLAGS`
(next to `KF3_WIN_USER_CHANNELS_PASSTHROUGH`). Check the realize log shows `EXPERIMENT KF3_WIN_KERNEL_PID4 ON`
and the status line ends with ` EXPERIMENT KF3_WIN_KERNEL_PID4`. After the first TDR / driver unload
(`UnloadingGuestDriver`), the restart's channels (`ProcessID=4`) must show:

1. **No** `birth REFUSED ... KernelInUserSpace` line; the restart's `CLASSIFY` lines for pid 4 read
   `kernel work -> Translated (the Windows System process ...)`.
2. The guest's alloc status for those channels is `0` (no `GSP REFUSED fn103/0x0000c56f=0x40`).
3. Windows either **recovers** (Display event 4101 "stopped responding and has recovered" in the live System
   log, desktop alive, QGA answers, `nvidia-smi` or the D3D probe works) or **fails differently** (name what:
   a different refusal, a Xid, a different bugcheck, a hang).

If the `KernelInUserSpace` refusal still occurs, the flag is wrong or incomplete (e.g. pid 4 is not what the
refused pair declares in that run, or a third path creates the Passthrough twin). If the refusal is gone and
`0x116` remains with the same arguments (`0xc000009a`, `4`), the wall was not the only cause.

> ⊘ **UPDATE 2026-10-10 (integration/windows-20261010): `KF3_WIN_KERNEL_PID4` is HARDWIRED, the flag is
> gone** (`kf_rm::chanlink::kernel_pid4_enabled()` is `true`). `[measured]` Windows run 260 (production
> profile without the flag, kf3 `a6587d6a`): the first TDR reset's restart channel was judged user work
> and the second restart channel of the same VA space was refused `KernelInUserSpace(1)` (RmAlloc 0x40
> -> StartDevice 0xC000009A -> bugcheck 0x116 VIDEO_TDR_FAILURE, then Code 43 after the reboot). Run
> 262 (same build and profile plus the flag): no `KernelInUserSpace`, no refused birth, the guest
> survived the TDR resets, as run 245 did. The rule only keeps a System-process channel Translated and
> never moves one to Passthrough. Open, unmeasured: why the first TDR occurs and its rate.
