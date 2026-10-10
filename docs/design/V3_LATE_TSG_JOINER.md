# V3 — the late TSG joiner: a twin born after the guest's one schedule

**STATUS: LIVE (hardwired), 2026-10-10 — hardware-verified, section 6 falsifier NOT met.** [measured, Windows 11 / RTX 4070,
`traces/windows_tdr_hunt_20261010/README.md` (branch `claude/tdr-opus-20261010`) runs 287, 288 vs 290, base binary 81cf89c8]
Without the schedule, the compute+copy joiners of a D3D12 device's TSG (ctxShare 0xff0e0201/0202, logged
`late_joiner_schedule=Some(SwitchOff)`) kept host `GP_GET=0` with `GP_PUT=0x10` after their doorbell. Their first
submissions never completed, the context flush hung and Windows reset the GPU (run 287: 1 TDR in a 240 s hold; run 288: 4 in
900 s, ETW: first render submissions on three new contexts never complete). With `KF3_SCHEDULE_LATE_JOINERS=1` (run 290,
one variable vs 288) there were 15 joiners `Authored`, none left unfetched, 0 TDR in boot, sign-in, Edge and a 300 s hold,
and 0 new host Xid. The switch is removed; the schedule is unconditional. (Section 2.3's hardware rule remains inferred: the
VFIO reference has no doorbell trace of a late joiner.) The text below is the 2026-10-09 state:

**STATUS (superseded above): LIVE (default OFF), 2026-10-09 — code and GPU-free tests on branch
`claude/late-tsg-joiner-20261009`, NOT hardware-verified.** The fix is behind
`KF3_SCHEDULE_LATE_JOINERS=1` because the rule it follows on real hardware is inferred, not shown
(section 2.3). Flip the default only after the falsifier in section 6 has passed on a box.
Evidence: `traces/windows_reset_20261009/late-tsg-joiner/`. Code: `crates/kf-qemu/src/latejoin.rs`
(rule and tests), wired in `crates/kf-qemu/src/chan.rs` (`schedule_statement`, the birth act, `free`).

Convention: **[measured]** names a log line or source line; **[inferred]** is reasoning from them.

## 1. What the logs show

### 1.1 Run 111 [measured] (`boundary-kayfabe-111/qemu.log`, parsed by `twins.py`)

23 Passthrough births. 20 are the first (or only) member of their client's TSG and each is followed
by exactly one guest `0xa06c0101` (TSG schedule, `result=0x0`) and one authored `act schedule ...
enable=true on 1 twin(s)`. Three are not. All three belong to client `0xc1d00046`, the D3D12
device:

| line | chan | token | host | ctxShare | TSG | guest schedule after birth |
|---|---|---|---|---|---|---|
| 71593 | `0xff04001b` | 0x1d | 0x44 | `0xff0e0200` | `0xff0e0000` | line 71989 `0xa06c0101`, authored 71992 (`on 1 twin(s)`) |
| 72350 | `0xff0e0102` | 0x1e | 0x45 | `0xff0e0201` | `0xff0e0000` | none |
| 72541 | `0xff0e0103` | 0x1f | 0x46 | `0xff0e0201` | `0xff0e0000` | none |
| 76229 | `0xff0e0104` | 0x20 | 0x47 | `0xff0e0202` | `0xff0e0000` | none |

The guest sent ONE `0xa06c0101` for that TSG (line 71989) and no channel-level `0xa06f0103` for any
of the four. At the stall snapshots (lines 81690-81797): 0x1d `GPGet=GPPut=0x16`; 0x1e
`GPGet=0 GPPut=0x10`, `doorbells=1`; 0x1f `GPGet=0 GPPut=0x12`, `doorbells=1`; 0x20 `0/0`, no
doorbell. So the distinguishing property of the three is: **born after the guest's schedule of
their TSG, and the guest sent no further schedule.** The authored schedule said `on 1 twin(s)`
because it runs over the twins that exist at the statement (`ChanPlane::schedule_statement`).
Hypothesis confirmed for run 111 by the logs; the causal step (an unscheduled host group is never
fetched) is [inferred] but is what the verb's documentation says (`ctrla06fgpfifo.h:36-40`:
"schedules a channel in hardware ... enabled in addition to being added to the appropriate
runlist").

### 1.2 Run 103 is a different failure [measured]

`boundary-kayfabe-103` has 11 births and **no late joiner** (every twin is a first member and is
scheduled; `run103-twins.txt`). Its one non-equal snapshot, token 0x15 (`0xff040013`), is
`GPGet=0x2f0 GPPut=0x3d7` twice with `doorbells=25`: the twin was scheduled, fetched 0x2f0 entries
(`GPGet` advanced through 0x185, 0x1f5, 0x244 in earlier snapshots) and then stopped. That is a
stall of a scheduled channel, not this defect; this fix does not touch it. Across all 115 logs on
the host only runs 83 and 111 have late-joiner births (`0xff0e01xx`); run 83 predates `PT-SNAP`,
so run 111 is the only one with a verdict.

### 1.3 The same pattern on the real GPU [measured] (VFIO boot3, `gsp.jsonl`)

Decoding the guest-to-GSP RPCs (`vfio_parse_schedule.py`; `RmControl` params start at +40 of the
body, `g_rpc-structures.h:1545-1557`): 38 TSGs, 51 channel allocs, 37 `0xa06c0101`, 9 `0xa06f0103`
(all on TSG-less channels). Exactly one TSG has late joiners: client `0xc1d00063`, TSG
`0xff0e0000`, first channel `0xff040026`, then `0xa06c0103` + `0xa06c0101` (t=60.4616), then three
more channels `0xff0e0102/3/4` with ctx shares `0xff0e0201/0201/0202`, each with `0x5080`, `0xc7b5`,
`0xc9c0` objects (t=60.4757-60.5667), and **no schedule of any kind afterwards**
(`vfio-boot3-client-0xc1d00063.txt`). The handles equal run 111's. The schedule params of that TSG
are `bEnable=1, bSkipSubmit=1, bSkipEnable=0` (33 of the 37 TSG schedules carry `bSkipSubmit=1`);
kf3 does not mirror the skip bits (`ChanStatement::Schedule` carries `enable` only;
`HostRm::schedule_enable` always authors `bSkipSubmit=0, bSkipEnable=0`).
The D3D12 probe on that boot completed DIRECT and COPY fences (`ps-d3d12_signal_probe.out`);
which queue used which channel is not recoverable from the RPC trace.

## 2. What RM does [measured source, ogkm 595.84]

1. **Group schedule** `kchangrpapiCtrlCmdGpFifoSchedule_IMPL`
   (`kernel_channel_group_api.c:1065-1203`): walks `pChanList` AT THE CALL (the members that exist
   then): checks every one `kchannelIsSchedulable` (:1107), forces a runlist on those without one
   (:1123-1173), then on a GSP client forwards the group control as an RPC and returns
   (:1176-1192; the :1201 internal control is the physical-RM path). A channel allocated afterwards is not in that walk.
2. **Channel schedule** `kchannelCtrlCmdGpFifoSchedule_IMPL` (`kernel_channel.c:3003-3062`): checks
   that one channel (:3022), marks its runlist immutable (:3029), RPCs it (:3039-3046) with the
   channel handle. Its documentation: "schedules a channel in hardware ... When set, the channel
   will be enabled in addition to being added to the appropriate runlist"
   (`ctrla06fgpfifo.h:36-65`). Both verbs are exported `NON_PRIVILEGED` (flags `0x10008`,
   `g_kernel_channel_nvoc.c:313-318`, `g_kernel_channel_group_api_nvoc.c:210-215`).
3. **RM expects joiners.** `kchannelCtrlCmdBind_IMPL` (`kernel_channel.c:3106-3112`): "This may be
   valid request if we added new channel to TSG that is already running."
4. **The in-tree driver schedules every channel it adds to a TSG, with the CHANNEL verb.** UVM makes
   one TSG per pool (`uvm_channel.c:2636-2646`, `channel_manager_num_tsgs` returns 1) and allocates
   the pool's channels into it one at a time; `nv_gpu_ops.c:6249-6256` (`engineAllocate`) issues
   `NVA06F_CTRL_CMD_GPFIFO_SCHEDULE` on each new channel. `nvidia-push-init.c:329-362,453` does the
   same for its channels (bind, channel-level schedule) and allocates the engine objects after.
   Nothing in the tree uses the group verb; that one is userspace's.
5. **Not in the source**: what the GSP firmware does for a channel allocated into a TSG whose
   group it has already enabled. "All real hardware management is done in the host"
   (`kernel_channel.c:2977, 3034`), and `kchangrpapiCtrlCmdInternalGpFifoSchedule` /
   the channel counterpart have no body in ogkm.

### 2.3 What hardware does with the guest's late joiners [inferred]

Windows sends no schedule for a late joiner on the real GPU (1.3) and the probe completes, so the
GSP must enable such a channel itself, or Windows would not rely on it. Not shown: no doorbell
trace of a late-joiner channel on the VFIO boot. That is why the fix is default off.

## 3. Why the twins stay unscheduled [measured code + inferred grouping]

* kf3 schedules at the guest's statement, over the twins that exist then
  (`schedule_statement`: `pt` entries with `v.tsg == Some(object)`), once per host group
  (`schedule_group_once`), with `HostRm::schedule_enable` = `NVA06C_CTRL_CMD_GPFIFO_SCHEDULE` on the
  twin's host group, `bEnable` from the guest (`kf-host/src/channel.rs:1448`).
* A twin's host group is keyed by `(hClient, hTsg, hContextShare)` (`ChanPlane::groups`, the birth
  act, `gkey = a.tsg.map(|t| (a.client, t, a.ctx_share))`). The first channel's key is
  `(..., 0xff0e0200)`. 0x1e's key `(..., 0xff0e0201)` is new, so by the code it **births a new
  host group** that nothing ever schedules; 0x1f has the same key and joins 0x1e's group; 0x20 is a
  third group. [inferred from the code and the logged ctxShare values; the logs do not print the
  host group handle.] The fix does not depend on which of the two cases it is: a new group needs
  its first schedule, a group that already holds scheduled members is re-sent the same verb.

## 4. The fix

`latejoin::schedule_late_joiner`, called in the birth act after the host channel exists and before
the token route is allocated: if the switch is on, the channel is in a TSG, and
`GuestTsgSched` says the guest's last schedule statement naming that TSG was `bEnable=1`, author
`schedule_enable(chan, true)` on the twin's own host group. One verb, on the act thread, no wait;
the reply to the guest's alloc is held until it returns (strict order). A host refusal refuses the
birth by name (`release_twin` unwinds; no silent stall).

* **State** is the guest's own words: `ChanPlane::statement` records `(client, object) -> bEnable`
  at STATEMENT time for every `GPFIFO_SCHEDULE` that is not refused (statement order is the act
  thread's order, so a birth act that is queued when the schedule statement arrives still sees it,
  and a schedule whose snapshot missed that twin does not leave it unscheduled). Forgotten when the
  object (or its client) is freed, and when the schedule act itself fails.
* **Not scheduled**: a TSG the guest never scheduled; one it scheduled with `bEnable=0` or disabled
  later; a freed TSG whose handle came back; a channel in no TSG; the switch off.
* **Idempotent with the guest's own later schedule**: that statement's snapshot now includes the
  joiner; `schedule_group_once` asks each host group once per statement; re-asking an enabled group
  changes nothing in RM's model (section 2.1 walks all members every time).
* **Verb choice**: the group verb, the same one every first member already gets and which hardware
  has run for months, on a group holding only channels the guest enabled. The channel verb is what
  UVM uses for a channel added to a TSG (2.4) and is the alternative if the group re-send on a
  joined group ever disturbs a running member; it needs a `NVA06F_CTRL_CMD_GPFIFO_SCHEDULE` row in
  `kf_abi::hostabi::HOST_CONTROLS` and its own hardware measurement, so it is NOT used here.
* **Why a flag**: the rule hardware follows is inferred (2.3), the schedule is authored BEFORE the
  twin's engine objects exist (hardware ran objects after the TSG schedule in 1.3 and
  `nvidia-push` allocates objects after scheduling, but kf3's host has not), and it changes every
  guest's late joiners, not only Windows'. Windows runs add `KF3_SCHEDULE_LATE_JOINERS` to
  `WIN_FLAGS` (`scripts/bench/windows/winpass_cycle.sh`).

## 5. Tests (GPU-free, fake host seam `GroupScheduler`)

`cargo test -p kf-qemu latejoin` (5 tests): a twin born into an already-scheduled TSG gets exactly
one authored schedule, on its own group; none in an unscheduled TSG, none outside a TSG, none with
the switch off, none for another client's same-numbered TSG; none after the guest disabled the TSG
or freed it (handle reuse), and a client free forgets all its rows; the guest's own later schedule
over all twins is idempotent (one verb per distinct group, same enabled set, a repeat changes
nothing); a host refusal is returned by name. Not covered without a GPU: the `ChanPlane` wiring
(statement-time recording, `free`, the birth call); it is a few lines and is what the hardware run
checks.

## 6. Falsifier for the hardware run (coordinator)

Rent/boot as run 111 (Windows, D3D12 probe), add `KF3_SCHEDULE_LATE_JOINERS` to `WIN_FLAGS`.
* **Expected if the hypothesis is right and the fix works**: the BORN lines of the three joiners
  end `late_joiner_schedule=Some(Authored)`; their `PT-SNAP` shows `GPGet == GPPut` (0x1e/0x1f with
  the work they were rung with: 0x10, 0x12 entries consumed); no `PT-SNAP` twin with
  `GPGet != GPPut` in a boot that previously had one; the D3D12 probe's COPY/DIRECT fences signal;
  no 0x116 at the point run 111 died.
* **Falsified (hypothesis wrong)** if the lines say `Authored`, the host accepts the schedule, and
  a joiner still shows `GPGet=0, GPPut>0` after its doorbell: then an unscheduled host group was
  not the reason (look at the objects-after-schedule order, then the channel verb, 4).
* **Fix wrong but hypothesis right** if the birth is refused by name (`birth ... host ...`), the
  host Xid count rises, or an already-running twin's `GPGet` stops advancing after a joiner's
  schedule (the group re-send disturbs members): switch to the channel verb.
* A Linux guest app with a late joiner (a CUDA process that adds a channel to its TSG after its
  schedule, if libcuda ever does) would show the same shape without the fix: a channel whose
  `GPGet` stays at 0 while `GPPut` moves and a hung `cuStreamSynchronize`/launch on that channel's
  stream. libcuda sends its own schedule per TSG; whether it sends one for a later channel is not
  in the traces, so this is [inferred] and the Windows run is the test.

## 7. Not covered

Run 103's mid-ring stall (1.2). Per-channel `STOP`/`DISABLE_CHANNELS` state of a TSG's other
members (a newborn is neither). `bSkipSubmit`/`bSkipEnable` from the guest (not mirrored, before
and after). Translated (guest-kernel) channels: their TSG schedule already covers members by
`by_obj`, and they are not twins.
