# V3 — one host TSG with real host context shares: feasibility study

**STATUS: RESEARCH, 2026-10-18; IMPLEMENTATION DESIGN (section 10) LIVE, 2026-10-18, branch `claude/tsg-mirror-20261018`.** Phase A (source + repo reading) and Phase B (native, unprivileged, bare-metal probe on the
host RTX 4070 / driver 595.91.07, 72 runs, 0 Xid) are both done. Nothing in a `kf-*` crate was changed. Not merged.
Branch `claude/tsg-subctx-feasibility-20261018` (from `integration/prod-candidate-4-20261017`). Probe:
`scripts/bench/tsg_subctx_probe/`. Evidence summaries: `traces/tsg_subctx_probe_20261018/summaries_all_batches.txt`. Raw logs
(not in git): host `/data/claude-tsgprobe-20261018/raw/b1..b9`.

Labels. **MEASURED** = a probe run named here (script number in brackets). **CODE READ** = source, with the ogkm tree named.
**TRACE** = a recorded guest/VFIO trace in the repo or on the host. **INFERRED** = reasoning, never evidence.
ogkm trees: **[580]** `/data/nouveau-trace-research/ogkm-580.65.06` (full tree), **[575]** `.../ogkm-575.51.03` (the files
used here are identical to 580 or differ only in log strings), **[595]** `/workspace/codex-perf/private/ogkm-595.84` (PARTIAL:
headers, `kernel-open`, `mem_mgr`; no fifo `.c`). The host runs 595.91.07 with GSP firmware: channel, TSG, ctxshare and
object logic runs CPU-RM first, then again in the (unreadable) firmware, so every CODE READ rule below was checked against
the real host by the probe.

## 0. Verdict

**FEASIBLE**, for guest TSGs that use explicit context shares (every Windows D3D12 device, every Vulkan device, CUDA):
one host TSG per guest TSG holding several REAL host `FERMI_CONTEXT_SHARE_A` objects, as an UNPRIVILEGED client, with no
Xid in any of 72 runs. Conditions that the probe made hard requirements (details in section 5):

1. **The host channel must carry the guest's RUNQUEUE bit** (`NVOS04_FLAGS_GROUP_CHANNEL_RUNQUEUE`, flag bit 4). kayfabe forwards
   no channel flag today (only `DENY_PHYSICAL_MODE_CE`, `kf-host/src/channel.rs:31-32,568-571`). Without it the GSP refuses
   (`NV_ERR_INVALID_OBJECT`, 0x31) a compute object on an ASYNC subcontext whenever another subcontext of the TSG holds a 3D
   object. Windows D3D12 sets it on exactly its joiner channels: all 1 393 channels of ctxshares `0201`..`0209` in the host logs have it, none of the 11 496 channels of `0200` and none of the legacy channels (TRACE, section 3).
2. Every host ctxshare must name an explicit host VA space handle (`hVASpace = 0` makes the channel's ring unreachable, 42).
3. Free order: channels, then ctxshare, then TSG (a ctxshare freed under a live channel kills the channel silently, 95).
4. A guest TSG is either all-ctxshare or all-legacy (host RM refuses the mix, 53; guest RM enforces the same rule).

**Recommended: option A, restricted to guest TSGs that use explicit ctxshares (A')**, legacy-mode TSGs (kernel driver, UVM,
video) unchanged. Option B ("mirror only when a TSG has more than one ctxshare") is **not implementable**: the second
ctxshare of a D3D12 TSG arrives after the first channel was born and scheduled, and a host group cannot change mode
(section 6). The late-joiner patch becomes unnecessary under A' (a member added to a scheduled TSG runs without its own
schedule, 60-62, 64-66, 90) but only after the Windows gate in section 7.

**The latency motivation is supported natively (MEASURED, section 4.2):** three tiny jobs on three channels that kayfabe
puts in three host TSGs complete serially (compute-only ~60 us per extra TSG, 180 us for three; with a 3D context in the mix
~290 us per extra TSG, 578 us for three), in ONE host TSG with three subcontexts they complete concurrently in 6 us. That
the same cost is what the guest sees (390 / 1213 us GPU leg) remains INFERRED: the probe has no relay and no guest.

## 1. Facts per question (Phase A)

### 1.1 Which channels may share a TSG (engine types)

* The TSG has ONE engine type and ONE runlist. `NV_CHANNEL_GROUP_ALLOCATION_PARAMETERS.engineType` ("engine to which all
  channels in this TSG are associated", `nvos.h:2904-2911`, [580]=[595]); `kchangrpapiConstruct` rejects an invalid type and
  stores it (`kernel_channel_group_api.c:161-179`, [580]); `kchangrpInit` takes `kfifoGetDefaultRunlist(engineType)`
  (`kernel_channel_group.c:152-154`). Without MIG only GR0 is accepted of the GR types (`:212-219`).
* A channel that names an engine type must equal the TSG's, else `NV_ERR_INVALID_ARGUMENT`
  (`kernel_channel.c:730-742`, [580]); engine 0 inherits the TSG's (`:746`). **MEASURED** a COPY0 channel in a GR TSG: status
  0x1f (22). A COPY0 TSG with a COPY0 channel runs (23).
* The runlist is finally fixed at object alloc: `chandesConstruct` ends in `kfifoRunlistSetIdByEngine` (`channel_descendant.c:242-252`),
  which refuses (`NV_ERR_INVALID_STATE`) an object whose engine has another runlist than the TSG's
  (`kernel_fifo_gm107.c:399-453,467-507`). GR, compute and the GR-partner copy engine (GRCE) share the GR runlist:
  a copy-class object on a GR-engine channel is supported (`channel_descendant.c:105-157`, the "GRCE case"). **MEASURED**: c7b5 on
  a GR channel works on a SYNC and on an ASYNC subcontext, all three engine classes do engine work (01, 02, 20, 21).
* Mixed classes in one GR TSG: 3D (c997), compute (c9c0) and copy (c7b5) on GR-engine channels all run in one TSG (10, 20, 90).
* **What the guest sends:** Windows D3D12 TSG alloc: class `a06c`, engineType 1 (GR0), hVASpace 0, all four of its channels
  engine 1 (TRACE `windows_code43_walls_20261007/run53-requests.log`, `windows_reset_20261009/late-tsg-joiner/run111-twins.txt:165-170`).
  CUDA TSG engine type, and the Linux Vulkan TSG, are **not recorded** in the repo (`params=-` in the CUDA recorder; no Vulkan
  TSG trace). kayfabe passes the engine type of the FIRST member (`kf-host/src/channel.rs:1347-1370`; guest TSG engine is only
  used to default a channel that declares 0, `kf-rm/src/chanlink.rs:1327-1330`).

### 1.2 Context shares (FERMI_CONTEXT_SHARE_A 0x9067)

* Params `{hVASpace, flags, subctxId}`; flags 1:0 = SYNC 0 / ASYNC 1 / SPECIFIED 2 / ASYNC_PREFER_LOWER 3 (`nvos.h:3226-3245`,
  [580]=[595]). The whole 32-bit flag word is compared, any extra bit = `NV_ERR_INVALID_ARGUMENT` (`kernel_ctxshare.c:442-446`).
* VEIDs: SYNC = fixed 0; ASYNC = highest free (grows down) falling back to 0; PREFER_LOWER = lowest free >= 1; SPECIFIED = the
  given id (`kernel_ctxshare.c:408-441`). **MEASURED** (00_veid_rules, 00_info_ctxcount): SYNC -> 0; a second SYNC -> 0x51
  (`NV_ERR_NO_MEMORY`); ASYNC -> 0x3f, 0x3e, ...; PREFER_LOWER -> 0x80000001 (bit 31 = success, VEID 1); SPECIFIED 3 works,
  repeated 3 fails 0x51, SPECIFIED 0 after SYNC fails 0x51; **63 ASYNC + 1 SYNC = 64**, the 64th ASYNC fails 0x51.
  `NV2080_CTRL_CMD_FIFO_GET_INFO` on the AD104: `MAX_SUBCONTEXT_PER_GROUP = 64`, `MAX_LOWER_SUBCONTEXT = 64`.
  An unprivileged client may use all four flags: no privilege check in the constructor, classes are `RS_FLAGS_ALLOC_NON_PRIVILEGED`
  (`resource_list.h:406-416,464-475`, `alloc_free.c:611-675`). **MEASURED**: every ctxshare in the probe was allocated by uid 1000.
* Legacy mode (a TSG whose channels pass `hContextShare = 0`): RM builds a SYNC (VEID0) and an ASYNC (VEID1) ctxshare itself
  (`kernel_channel_group_api.c:936-983`, `kernel_channel.c:654-677`); the channel picks by `NVOS04_FLAGS_GROUP_CHANNEL_THREAD`
  (flags 29:28; 0 = SYNC, else ASYNC). A TSG cannot mix explicit and legacy channels (`kernel_channel.c:606-626`);
  **MEASURED** 53: 0x1f. **MEASURED** legacy `thread=1` is refused (0x1f) unless the channel also has runqueue 1 (52 vs 81).
* VA: a TSG channel must not pass `hVASpace` (it takes its ctxshare's, `kernel_channel.c:422-429,692`); each ctxshare carries its own
  VAS, no code forces one VAS per TSG (`kernel_channel_group.c:215-216`, `kernel_ctxshare.c:487-529`). **MEASURED**: SYNC in VA
  space 0 + ASYNC in VA space 1, each channel's ring mapped in its own space, both run (41). TSG `hVASpace = 0` with explicit
  ctxshares works (43). **ctxshare `hVASpace = 0`: the allocs succeed but the channel never fetches** (host semaphore never
  written, no Xid, 42, and 83) - never author 0.
* **Class compatibility per VEID (CODE READ vs MEASURED differ, section 5).** [580] `kfifoValidateEngineAndSubctxType_GP102`
  (`kernel_fifo_gp102.c:133-179`) says: SYNC takes every engine, ASYNC only GR / GRCE / SW. MEASURED: an ASYNC ctxshare in a COPY0
  TSG accepted a c7b5 alloc (24, execution not tested) - firmware is looser than the open code there. In the other direction
  the firmware is STRICTER than anything in the open code: see 1.2a.
* Bind/schedule order: ctxshare before the channel that names it (`kernel_channel.c:632-647`); the channel is a resserv dependant
  of its ctxshare (`kernel_channel.c:988-994`). TSG BIND (`NVA06C_CTRL_CMD_BIND`) binds existing members only
  (`kernel_channel_group_api.c:1209-1275`); TSG schedule walks the members present at the call (`:1065-1203`, 1.4).

**1.2a Measured class/runqueue rule (firmware, not in the open code).** Table, all with the TSG schedule and the objects allocated
after the channels (scripts in section 4.1):

| SYNC subcontext holds | ASYNC channel (runqueue 0) | ASYNC channel (runqueue 1) |
|---|---|---|
| 3D + compute | compute alloc **refused 0x31** (10v1/b3, 31, 34, 36, 38, 39), 3D alloc refused (37) | compute works (80, 84, 10, 92) |
| compute only | compute works (30, 3a), 3D alloc refused (3b) | not run |
| nothing | compute works (32, 33) | not run |
| (ASYNC compute allocated first, then 3D on SYNC) | 3D alloc refused 0x31 (35) | works (84) |

A compute channel with runqueue 1 on the SYNC subcontext: compute refused (82). INFERRED: the GSP requires compute that coexists
with a graphics context in one TSG to sit on the second GR runqueue (ASYNC VEID + runqueue 1); the open code only says "runqueue 1
is for GR / GRCE / SW" (`kernel_fifo_gp102.c:75-119`).

### 1.3 Late members, schedule and bind

* TSG schedule loops over the members that exist at the call; CPU-RM does nothing for a later member (`kernel_channel_group_api.c:1065-1206`,
  [580]; confirmed by `V3_LATE_TSG_JOINER.md` section 2). `NVOS04_FLAGS_DELAY_CHANNEL_SCHEDULING` documents the default: a new channel in a
  scheduled group IS scheduled immediately (`alloc_channel.h:145-157`, [595]); no `.c` implements it (firmware).
* **MEASURED (the open question of `V3_LATE_TSG_JOINER.md` 2.3):** a member allocated AFTER the TSG schedule runs without any schedule of its
  own: same ctxshare (60), new ASYNC ctxshare with runqueue 1 (61), after a channel BIND only (62), legacy mode (64), no BIND at all (65),
  a late copy-class member on a new ASYNC ctxshare (66), and the exact Windows sequence (90). With `DELAY_CHANNEL_SCHEDULING` set it does NOT run
  until its own channel schedule (63), and then runs. Token (`GET_WORK_SUBMIT_TOKEN`) needs a bound channel (`INVALID_STATE` before; harness fix
  in b2) - the TSG schedule alone binds (90).

### 1.4 Controls, privilege, scope

* Export flags (generated tables, [580]): TSG schedule, BIND, SET_TIMESLICE, GET_TIMESLICE, PREEMPT, channel schedule/BIND: `NON_PRIVILEGED`; SET_INTERLEAVE_LEVEL
  `NON_PRIVILEGED|PRIVILEGED_IF_RS_ACCESS_DISABLED` with `RS_ACCESS_DEBUG`; MAKE_REALTIME needs `RS_ACCESS_NICE`. **MEASURED as uid 1000**: SET_TIMESLICE, TSG schedule,
  channel schedule, PREEMPT (bWait, with and without manual timeout), `DISABLE_CHANNELS` all succeed on a 3-subcontext TSG.
* **MEASURED scope** (97, 98, 99, 9a, 9b): channel-level `GPFIFO_SCHEDULE(bEnable=0)` stops THAT channel only and `bEnable=1` brings it back (98);
  `DISABLE_CHANNELS` on one channel stops only that channel, `enable` restores it (99); naming every channel stops all (9a). **TSG-level `GPFIFO_SCHEDULE(bEnable=0)`
  does not stop channels that are already running** (97, 9a: work after it completes). So the "disable" of `chan.rs:4105-4146` is carried by `DISABLE_CHANNELS`; its group
  `schedule_enable(false)` is a no-op on a running group today and must not be relied on.
* Max channels per TSG 128 (`kernel_fifo_gv100.c:132-144`); **MEASURED** 8 channels + one ctxshare + one TSG schedule all run (96).

## 2. kayfabe today (CODE READ, rev 0452613b)

| What | Where | Fact |
|---|---|---|
| host group key | `kf-qemu/src/chan.rs:2325-2335` | `groups: HashMap<(hClient, hTsg, hContextShare), (host group, live members)>`: one host TSG per guest ctxshare |
| host TSG | `kf-host/src/channel.rs:1338-1372` (`birth_group`) | engineType of the first member; `hVASpace` of its twin space; no ctxshare |
| host channel | `kf-host/src/channel.rs:568-574` | `flags = DENY_PHYSICAL_MODE_CE` only; **`h_context_share: 0`, `h_va_space: 0`**; runqueue/thread never set |
| guest ctxshare | `kf-rm/src/chanlink.rs:864-865,1259-1264,1308-1316,1352-1354` | kept as `(client, hCtxShare) -> hVASpace` only; flags and subctxId dropped (`kf-abi/src/view.rs:383-393`); reply is the guest's own echo |
| host ctxshare | `kf-host`, `kf-qemu` | never allocated; `NvCtxshareAllocationParameters` and the matrix row exist (`kf-abi/src/generated/classes.rs:1243-1255`, `generated/matrix.rs:11224`), `raw_alloc` only refuses channel classes (`birth.rs:119`) |
| late joiner | `kf-qemu/src/latejoin.rs`, `chan.rs:3606-3623,5415,6841`, `userrelay.rs:655` | authored group schedule for a twin born after the guest's schedule |
| TSG schedule | `chan.rs:3502-3601` | authored on every host group that has a live member at the statement (`schedule_group_once`, dedup by group); a mid-loop failure leaves earlier groups scheduled |
| SET_TIMESLICE | `chan.rs:3866-3930` | every host group existing at the statement; **not remembered**, a group born later runs at the host default (D3D12 sends it BEFORE the schedule, so ctxshare groups born later never get it) |
| PREEMPT | `chan.rs:4407-4447` | one host call per twin, not deduplicated per group |
| STOP_CHANNEL / evict | `chan.rs:4105-4146,4593-4640` | `DISABLE_CHANNELS` (channel) then `schedule_enable(false)` on the twin's WHOLE group; the comment at `:4094` ("the twin is its own host group") is stale for CUDA already |
| SET_INTERLEAVE / GET_TIMESLICE / MAKE_REALTIME / GET_INFO | `kf-abi/src/capability.rs:873-875` | not allowlisted, not served |
| tests | `kf-qemu/src/latejoin.rs` (5), `chan.rs` scope tests | **no unit test covers the `groups` map, `release_twin` counting or `schedule_statement`'s multi-group loop** |

## 3. What the guest sends, per workload (TRACE / CODE READ)

| Workload | TSG alloc | ctxshares (flags, VEID) | Channels (class / engine / flags / ctxshare) | Objects | Order | Evidence |
|---|---|---|---|---|---|---|
| **Windows D3D12 device** (one TSG per device) | `a06c`, engine 1 (GR0), hVASpace 0 | 3: `0xff0e0200`, `0201`, `0202`; **flags and VEID not recorded** (VFIO alloc requests are truncated to the 64-byte header) | 4 x `c56f`, engine 1: ch1 on `0200` flags `0x..20` (no runqueue bit); two on `0201`, one on `0202`, flags `0x..30` = **runqueue bit set**; `hVASpace=0` | ch1: 5080, c7b5, a140, 902d, c997, c9c0; each joiner: 5080, c7b5, c9c0 | TSG, `a06c010a`, ctx 0200, ch1, objs, `a06c0103` SET_TIMESLICE, `a06c0101` SCHEDULE (`bEnable=1, bSkipSubmit=1`), then ctx 0201, ch, objs, ch, objs, ctx 0202, ch, objs; **no schedule after the joiners, no BIND at all**. GSP replies to every alloc incl. joiner c9c0: result 0 | VFIO boots 1-3 (`vfio-boot3-client-0xc1d00063.txt`, reply directions parsed again in this study from `boot3-gsp.jsonl.gz`); channel flags: kf3 `CHAN-FACTS` lines of host runs 3006/3007 etc. (counted over every `boundary-kayfabe-*/qemu.log` on the host: bit 4 set on 764 channels of `0201`, 369 of `0202` and every channel of `0203`..`0209`, i.e. 1 393 of 1 393; on 11 496 channels of `0200` and 20 054 legacy channels it is never set) |
| Windows single-queue clients (DWM, D3D11, apps) | engine 1, hVASpace 0 | 1 (`0200`) | 1 channel with the full object set, flags `0x2xxxxx20` / `0x80xxxxxx20` | full set | TSG, 010a, ctx, ch, objs, 0103, 0101 | VFIO histogram 22/33/15 x (1 ctxshare, 1 channel); `run64-census-births-facts.txt` |
| Windows kernel driver / video | per client TSG (engine 0xb COPY2 or GR; video engines TSG-less) | 0 (legacy, `hContextShare = 0`) | 1 channel | 5080 + c7b5 (copy) / full | - | VFIO (0,1) x 22/47/62; `run64-census-births-facts.txt` |
| **CUDA (Linux)** | engine type not recorded | 1 per TSG, VEID `0x3f` (top: ASYNC, INFERRED); flags not recorded | 8 x `c56f` (compute c7c0 + copy c7b5 on GRCE `0x0a`); ctxshare named by channel: INFERRED (RM rule); 2 more TSGs of 4 copy channels, COPY2/COPY3, no ctxshare | per channel | TSG, ctx, 8 x (ch, c7c0, c7b5), ONE `a06c0101` after all; later `0103` (0x800 us) and `0105` | `traces/guest_boots/run_w217_2f616e2_grpush_probe.log:653-830`; `kf-gsp/src/rpc.rs:598-606` (host returns 0x3f) |
| **Vulkan (Linux)** | not recorded | 2 (graphics VEID0, async compute) | graphics + async-compute GR channels; per-channel flags not recorded | not recorded | not recorded | doc only: `V3_VIDEO_ENGINES.md:146-160`, `traces/v3_int_ga106/int_d536595d.txt:64,71` (`GFX_S3` pass with per-ctxshare host groups; the failing-run log is not in the repo) |
| Video engines (Linux NVDEC/NVENC/OFA) | not recorded | not recorded | `0x80000013/15` handles only | - | - | `int_d536595d.txt`; Windows video channels are TSG-less, ctxshare 0 |
| UVM | engine 0xb / 0xc, kernel privilege | 0 (`ctxShare = 0`) | 2 channels per TSG (one TSG per pool); channel-level `a06f0103` per channel (CODE READ via `V3_LATE_TSG_JOINER.md:88-92`) | - | - | `traces/windows_matrix_20261011/.../fast_codexMemoryAccess_uvm-mean_qemu.log` CHAN-FACTS |

Gaps stated plainly: ctxshare flags / returned VEID for every workload, CUDA and Vulkan TSG engine type, and Vulkan's channel flags are not in
any repo evidence. The probe therefore tests every flag combination natively instead of assuming the guest's.

## 4. Native probe (Phase B)

Setup. Host trusted root@172.22.1.20, RTX 4070, driver 595.91.07, kernel 7.0.0-34. The owner's VM (run 3007) was stopped through
the stop file (launcher exit 0 at 22:37:20), the probe ran under `flock /tmp/kayfabe-fastguest.lock` as **uid 1000 (`ubuntu`)** against
`/dev/nvidiactl` (unprivileged RM client), the VM was restarted as run 3008 (`OWNER_SESSION_READY` 22:51:25), stopped once more for the control-scope
batch (b9) and restarted again as run 3008 (`OWNER_SESSION_READY` 22:59:59). Probe: `tsg_subctx_probe.c` (C over the RM ioctls, public ogkm 595 headers,
descended from `scripts/bench/setobj/setobj_probe.c`, whose positive and negative controls were measured on this host), driven by 61 one-shape scripts
(`scripts/`, the first version of the ones edited afterwards is in `scripts_v1/`). Every run = fresh RM client. Per run: engine work on every object
of every channel (SET_OBJECT + class report-semaphore for 3D and compute, `LAUNCH_DMA` semaphore release for copy), a host semaphore, the channel error
notifier twice, the host journal window and a total-Xid delta, `nvidia-smi` after each run. **72 runs, Xid delta 0 on every one**, `nvidia-smi`
healthy throughout; the owner VM came back with the Xid total unchanged at 507.

Harness note (b1): the first two baseline runs MISSed everything because the work-submit token was requested before the channel was bound (RM:
`INVALID_STATE`, journal line `kfifoGenerateWorkSubmitTokenHal`). Fixed by asking on first use; b2 repeats them. That is a harness defect, not a finding.

### 4.1 Accept / refuse / Xid per shape (falsifier stated before the run is in the script header comment)

Notation: S = SYNC ctxshare, A = ASYNC, rq1 = channel flag bit 4, "objs" in alloc order. "OK" = all engine semaphores and the host semaphore arrived,
notifier clean, no Xid.

| # | Shape | Falsifier ("this shape is impossible if...") | Result (MEASURED) |
|---|---|---|---|
| 01/02 | baseline: 1 TSG(GR), legacy / explicit SYNC, one GR channel 3D+cmp+copy | harness cannot run engine work | OK; 3D, compute, copy-on-GR all execute (b2) |
| 10/11/12 | **shape 1**: S{g: 3D+cmp} + A{c: cmp} rq1, group bind+schedule / objects after schedule / channel bind+schedule | an unprivileged client cannot build SYNC+ASYNC in one TSG | OK in all three orders; concurrent work OK; `bench` 4 us (b7). With rq0: ASYNC compute alloc **refused 0x31** (b3) |
| 20 | S{3D+cmp} + A{cmp} + A{copy c7b5 on GR channel} (3 subcontexts) | copy class refused on ASYNC | OK (3 subcontexts, VEIDs 0, 0x3f, 0x3e) |
| 21 | copy c7b5 on SYNC channel + compute on ASYNC | | OK |
| 22 | COPY0-engine channel in a GR TSG | | **refused 0x1f** (matches CODE READ) |
| 23 | COPY0 TSG + COPY0 channel (control) | | OK |
| 24 | ASYNC ctxshare in a COPY0 TSG, c7b5 alloc | open code says refused | **accepted** (alloc only; CODE READ contradicted) |
| 25 | legacy: g thread0 3D+cmp, k thread0 copy, c thread1 | | c refused 0x1f; g, k OK |
| 30-3b | isolation of the 0x31 (SYNC/ASYNC x 3D/compute x order) | see 1.2a | table 1.2a; the refusal is firmware (`GspRmAlloc failed ... paramsStatus=0x31` in the journal), no Xid |
| 36 | ASYNC_PREFER_LOWER instead of ASYNC (rq0 only) | | same as ASYNC at rq0: compute refused 0x31 (rq1 not run with PREFER_LOWER) |
| 40 | both ctxshares name the TSG's VA space explicitly | | OK |
| 41 | S in VA space 0, A in VA space 1, rings mapped per space | one VA space per TSG | **OK** (different VA spaces per subcontext work) |
| 42/83 | ctxshare `hVASpace = 0` | | allocs OK, **channel never fetches** (silent) |
| 43 | TSG `hVASpace = 0`, ctxshares explicit (0 / 1) | | OK |
| 50/51 | **old failing shape**: graphics + a compute channel both on the legacy SYNC subcontext, rq0 (what kayfabe's merged host group did) | Xid 69 does not reproduce natively | **NOT REPRODUCED**: no Xid, work OK, bench 8 us; with 3D on both channels also OK (51). The Vulkan Xid 69 therefore needs the real workload (c797 method 0x2620) |
| 52/81 | legacy `thread=1` without / with rq1 | | refused 0x1f / OK |
| 53 | legacy + explicit channel in one TSG | | **refused 0x1f** |
| 60-62, 64-66 | **late member** after the TSG schedule: same ctxshare / new ASYNC (rq1) / +channel BIND / legacy / no BIND / late copy | a late member needs its own schedule | **runs with no schedule of its own** in all six |
| 63 | late member with `DELAY_CHANNEL_SCHEDULING` | | does not run until its channel schedule, then runs |
| 80/84 | SYNC{3D+cmp} + ASYNC{cmp} rq1, either allocation order | | OK, concurrent (bench 4 us) |
| 82 | compute channel rq1 on the SYNC ctxshare | | compute **refused 0x31** |
| 90 | **D3D12 exact**: TSG, S{5080,c7b5,c997,c9c0}, timeslice, schedule(skipSubmit), A1 x2 channels, A2 x1 channel (rq1, 5080,c7b5,c9c0), NO bind, NO joiner schedule | the Windows shape cannot be built by an unprivileged client | **OK**: all four channels run all classes, bench 7 us |
| 91 | 90 + group bind + channel schedule for every joiner | | OK |
| 92 | Vulkan-like: S{3D+cmp+copy} + A{cmp} rq1 | | OK, concurrent |
| 95 | free order | | freeing a ctxshare under a live channel returns 0 and **the channel stops** (host semaphore MISS); channel-then-ctxshare, VEID `0x3f` reused, new ctxshare + channel OK |
| 96 | CUDA-like: 1 TSG, 1 ctxshare, 8 channels (cmp + copy), 1 schedule | | OK, bench 13 us for all eight |
| 97/9a | TSG schedule `bEnable=0` / `DISABLE_CHANNELS` all | | TSG schedule(0) **does not stop** running channels; `DISABLE_CHANNELS` stops them, `enable` restores |
| 98/99 | one channel of three subcontexts: channel schedule(0) / `DISABLE_CHANNELS` | the control stops the siblings | only that channel stops, siblings keep running, re-enable works |
| 9b | TSG PREEMPT (wait, manual timeout / no wait) | | OK (status 0) |

### 4.2 Latency: separate host TSGs vs one TSG with subcontexts (tiny jobs, host otherwise idle, 300-500 iterations)

One iteration = submit one engine-semaphore release to every channel together, wait for all. p90 in brackets.

| Shape | all-done median (p90) | per channel median |
|---|---|---|
| 3 compute channels, **3 TSGs** (70) | 180 us (1288) | 62 / 121 / 180 us (serial ~60 us steps) |
| same with an explicit SYNC ctxshare each (73) | 180 us (1287) | 61 / 120 / 180 |
| 3 compute channels, **1 TSG, S + 2 A** (71) | 6 us (6) | 6 / 4 / 5 |
| 3 compute channels, 1 TSG, 1 ctxshare (72, CUDA-like) | 6 us (6) | 6 / 4 / 5 |
| D3D12-like direct(3D) + copy + compute, **3 TSGs** (74) | 578 us (579) | 287 / 288 / 578 |
| same, **1 TSG, S + A1(rq1) + A2(rq1)** (75) | 6 us (6) | 6 / 4 / 5 |
| exact D3D12 shape, 4 channels (90 / 91) | 7 us / 7 us | - |
| 8 channels, 1 TSG, 1 ctxshare (96) | 13 us | 5..13 |
| a single channel alone | 2-4 us | - |

The first round trip of a fresh channel is typically 500-800 us; that is the idle-engine wake, not scheduling. The steady figures are the
ones above. Each additional host TSG that has to be switched to costs ~60 us for compute-only contexts and ~290 us when a 3D context is involved; a
shared TSG pays nothing. kayfabe's measured private-channel markers at 554 / 623 / 678 us (`traces/submit_latency_20261018/README.md`, MEASURED there)
are 55-70 us apart, the same step as the compute-only TSG hop here. **INFERRED, not shown:** that this is the cause of kayfabe's 390 / 1213 us GPU
leg. The falsifier for the inference is the hardware gate in section 7 (guest GPU leg stays > 300 us after the change).

## 5. Where the probe contradicted or extended the code reading

1. ASYNC compute alongside 3D needs **runqueue 1** (1.2a): not derivable from the open code; kayfabe forwards no channel flags at all. This is also the best
   candidate mechanism for the Xid 69 of `V3_VIDEO_ENGINES.md:146-160`: the guest's async-compute channel (ASYNC ctxshare + runqueue 1) was merged by the host
   onto the legacy SYNC subcontext at runqueue 0, next to the graphics channel. **INFERRED**, because the shape did not Xid with tiny jobs (50, 51).
2. Late members need no schedule in the same TSG (section 1.3): `latejoin.rs` exists only because kayfabe's per-ctxshare groups made the joiner's group a new,
   never-scheduled TSG (`V3_LATE_TSG_JOINER.md` section 3).
3. TSG-level `schedule(bEnable=0)` is not a stop (1.4).
4. ASYNC ctxshare in a COPY TSG is accepted (24); ctxshare `hVASpace = 0` is silently useless (42).

## 6. Options

All options keep legacy-mode guest TSGs (no ctxshare: Windows kernel driver, UVM, video) on today's path: one host TSG, `hContextShare = 0`.

**(A) Full mirror** - one host TSG per guest TSG, one host ctxshare per guest ctxshare, host flags authored from the validated guest facts.
**(B) Mirror only where a guest TSG has more than one ctxshare**, today's shape for single-ctxshare TSGs.
**(C) Keep per-ctxshare groups, fix fan-out only** (timeslice remembered per group, schedule/preempt/stop deduplicated, no subcontexts).
**(D) A, restricted to explicit-ctxshare TSGs, plus the control-scope clean-up** (this is the recommendation; called A' below).

| | A / A' (D) | B | C |
|---|---|---|---|
| CUDA 8-channel TSG | same host shape as today (1 TSG, 8 channels); the host ctxshare is added (guest's kind, VEID 0x3f ASYNC). 96 OK natively. **Risk:** changes every CUDA TSG from legacy SYNC to ASYNC VEID | unchanged | unchanged |
| Vulkan (Xid 69 case) | S + A(rq1) in one TSG: 10, 80, 92 OK; the async compute channel keeps its guest runqueue bit | **cannot be decided at birth** (below) | per-ctxshare groups: works as today (no Xid), cross-TSG switch cost stays |
| Windows D3D12 | 90/91 OK; joiners need no schedule; removes `latejoin.rs` after the gate | same problem as Vulkan | per-ctxshare groups + late joiner patch stay; ~290 us per extra TSG hop stays (74 vs 75) |
| Video / UVM | legacy path, unchanged | unchanged | unchanged |
| deletes | `groups` third key, `latejoin.rs`, `GuestTsgSched`, multi-group loops in schedule / timeslice / preempt | none | none |
| latency | removes the cross-TSG switch (4.2) | same as A for multi-ctxshare TSGs, but unreachable | none (cost is inherent to separate TSGs, 70/73/74) |
| falsifier | (i) guest GPU leg stays > 300 us after the change, (ii) any Xid in the CUDA/Vulkan/D3D12 gates, (iii) a firmware refusal of a guest-authored combination that Windows/Vulkan do on bare metal | B is dead on arrival: the Windows TSG gets ctxshare 2 and 3 only after channel 1 was born, bound and scheduled, and a host group cannot change from legacy to explicit mode (53: refused 0x1f); legacy mode has exactly two subcontexts (SYNC, ASYNC), D3D12 needs three. Falsify by finding a Windows/Vulkan workload whose ctxshare count is known at the first channel | C cannot lower the switch cost: natively 180-578 us vs 6 us (4.2) regardless of timeslice bookkeeping (a shorter host timeslice was not tried) |

Hostile-guest rules for A' (host actions authored from validated facts, never forwarded bytes):

* The host ctxshare `kind` is a two-valued choice (SYNC / ASYNC) decoded from the guest's flags; PREFER_LOWER maps to ASYNC; SPECIFIED maps to SYNC for id 0 and
  ASYNC otherwise. The host picks the VEID; the guest's echoed `subctxId` is never forwarded to the host (the guest cannot name a VEID).
* The host channel flags are built from an allowlist: `DENY_PHYSICAL_MODE_CE` (as now), runqueue bit 4 (guest value, 0 or 1), thread bits 29:28 only for legacy-mode
  TSGs (0 or 1, 1 only together with runqueue 1). `DELAY_CHANNEL_SCHEDULING`, `PRIVILEGED_CHANNEL`, `SKIP_SCRUBBER` and every other bit are never set.
* Cap the number of ctxshares per guest TSG that a client may realise (the host limit is 64 VEIDs: 63 ASYNC + 1 SYNC, measured) at a kayfabe constant well
  below that; host `NO_MEMORY` is returned to the guest as a named error.
* A second SYNC from the guest cannot be allocated on the host (0x51): the host ctxshare is allocated lazily at the first channel of that guest ctxshare, so a
  hostile or buggy guest meets the refusal at its channel alloc; reply with the host status, unwind (free channel, then the ctxshare if it was new, then the TSG if it was
  new), keep the group map unchanged, **never fall back silently to a per-ctxshare group**.
* ctxshare `hVASpace` is always the explicit host twin space of the guest VA space that the guest ctxshare names (never 0).

## 7. Recommendation and implementation plan (option A', one consolidated change)

State machine. `groups: (hClient, hTsg) -> Group { host_tsg, mode: Legacy | Explicit, ctxshares: guest_ctx -> {host_ctx, space, kind, members}, members }`.

1. **Decode** (`kf-abi`, `kf-rm`): keep ctxshare `flags` in `AllocFacts` and `RmGraph::CtxShare` (today dropped, `rmrpc/mod.rs:201-202`); carry the guest channel's
   runqueue/thread bits in `ChannelAlloc` (flags are already decoded into `ChannelAlloc.flags`, `chanlink.rs:205-206`). Validate: flag word must be one of the four
   subcontext values; refuse unknown bits by name.
2. **Host** (`kf-host`): `birth_ctxshare(tsg, space, kind) -> handle` (matrix row exists); `ChannelAllocParams` gets `h_context_share` and an authored `flags`;
   `Channel` gets the ctxshare; `birth_member` takes it; free order channel -> ctxshare -> TSG; TSG free keeps working through `forget`. Host TSG `hVASpace` = the first
   member's twin space (43 shows TSG space is irrelevant with explicit ctxshares).
3. **Plane** (`kf-qemu/src/chan.rs`, `userrelay.rs`): replace the `groups` key; birth act = ensure group, ensure ctxshare, birth member, record counts; `release_twin`
   counts per ctxshare and per group; mixed legacy/explicit refused by name before the host call. Schedule, timeslice and preempt authored once per guest TSG against the one
   host group; **remember the timeslice per group** and apply it at group birth; STOP/evict use `DISABLE_CHANNELS` plus the channel-level schedule(false), not the group verb (1.4).
4. **Delete** `latejoin.rs`, `GuestTsgSched` and the `schedule_late_joiner` calls **only after** the Windows gate below; until then keep the call but make it a no-op
   check (log if it would have fired) so the gate can show the rule is dead.
5. **Tests (GPU-free)**: new unit tests for the group/ctxshare bookkeeping (none exist today): first member creates group + ctxshare; second member of the same
   ctxshare joins; new ctxshare in a scheduled group; mixed-mode refusal; host refusal at ctxshare / channel step unwinds exactly what was created; free order; timeslice remembered.
   `cargo test -p kf-host -p kf-qemu -p kf-rm`.
6. **Native regression**: add `scripts/bench/tsg_subctx_probe/gate.sh` that runs scripts 10, 20, 41, 60, 61, 80, 90, 92, 96, 98 and fails on any non-OK line or Xid
   (keep them as the host-RM drift detector; the firmware refusals in 1.2a are the contract kayfabe now depends on).
7. **Hardware gates** (strictly serial, one box, per `CLAUDE.md`): `v3_gates.sh` 9/9 -> `build_kf3.sh` -> rebuild raw client and fast guest -> `fast_suite.sh` 30/30
   (CUDA 8-channel TSG; `V3_BUILD.md` SKEDCHECK05 second-stream case via `cuda_ladder.sh`) -> **Vulkan `int_gfx`** (`GFX_S3` must PASS, no Xid 69; this is the only place the old
   failure can reproduce) -> `video_lane.sh` -> `llm_parity.sh` / `gfx_suite.sh` -> UVM lane -> **Windows D3D12 (run 290-style: sign-in, Edge, 300 s hold) with the late-joiner call disabled:
   0 TDR, 0 new Xid, every joiner fetched** -> `broker_lane.sh` / `interactive.sh` (changes touch the drainer's act path) -> re-run `scripts/bench/submit_latency` DIRECT/COMPUTE/COPY and
   compare the GPU leg with 5.6 / 390 / 1213 us.
8. **Late-joiner removal condition:** the Windows gate above passes with the rule disabled AND probe 60-66/90 still OK on the host driver in use (they are in the native
   regression). Keep `KF3_DELAY` semantics out: kayfabe never sets `DELAY_CHANNEL_SCHEDULING`, so the "joiner runs unscheduled" behaviour is the default.

Order of work: 1 -> 2 -> 5 (host + decode + unit tests, no hardware), 3, then 6 and the gates in the order of item 7. Expected cost: `kf-host` and `kf-rm` small, `chan.rs`
bookkeeping is the bulk (the `groups` map is used at birth, free, census and `userrelay.rs:575,642-659,736,988`).

## 8. What is still unknown

* ctxshare flags and returned VEIDs and the channel runqueue bits for CUDA and Vulkan on Linux: no trace. A guest-side recording with `tools/cudart_probe/nvdiff_shim.c` (`scripts/nvdiff_inband.py`)
  would settle it; A' does not depend on the answer if the decode forwards the guest's own bits, but the CUDA gate checks it.
* Whether the Xid 69 is really the runqueue mismatch (not reproduced with tiny jobs: 50, 51). The Vulkan hardware gate is the test.
* Whether GPU_PROMOTE_CTX buffers (kayfabe's "Translated" promote path) and the Windows ctx-buffer bookkeeping behave identically when the context is an ASYNC VEID instead of legacy
  VEID0 for single-ctxshare TSGs (CUDA, apps): untested; the 96 shape ran, but without promote or real kernels.
* Real kernels and the SKEDCHECK05 local-memory case on 8 channels of one ASYNC ctxshare: the probe ran semaphore releases only.
* Whether the 4.2 cross-TSG cost is the cause of the guest's 390 / 1213 us (INFERRED); whether a shorter host timeslice would reduce it (option C, not tried).
* ASYNC ctxshare in a COPY TSG (24): only the alloc was run.
* Interleave level / MAKE_REALTIME as an unprivileged client: not run.
* Windows-side TSG schedule semantics `bSkipSubmit` / `bSkipEnable` (kayfabe forces both 0; probe 90 used `bSkipSubmit=1` and worked).

## 9. Reproduce

```
cc -O1 -Wall -I$H -I$H/class -I$H/ctrl -I$H/alloc scripts/bench/tsg_subctx_probe/tsg_subctx_probe.c -o tsg_subctx_probe   # H = ogkm 595 src/common/sdk/nvidia/inc
# host, owner VM stopped via /tmp/kf-stop-claude-manual:
flock /tmp/kayfabe-fastguest.lock bash run_probe.sh OUTDIR scripts/90_w1_d3d12_exact_order.txt ...   # runs as uid 1000
bash sum.sh OUTDIR '*.log'
```
Binaries used: md5 `18628f92` (b1), `067bb4a1` (b2-b3), `10a1ee87` (b4-b8), `e6db725f` (b9; the final one on the host). Scripts changed between batches only as noted
(rq1 added to ASYNC channels after b3; `scripts_v1/` = as run in b3, b4).

## 10. Implementation design (A'), settled before code

**STATUS: LIVE (design of the code on `claude/tsg-mirror-20261018`), 2026-10-18.** Labels as above. "Base" = `0452613b5`
(`integration/prod-candidate-4-20261017`), every `file:line` below is base. This section changes nothing in sections 0-9; where the
plan of section 7 is made concrete or refined the refinement is named.

### 10.1 What kayfabe observes, and what it validates

* **Guest ctxshare alloc.** Today `kf-rm/src/chanlink.rs:1259-1264` keeps `(client, hCtxShare) -> hVASpace` and drops `flags`,
  `subctxId` and the parent (`kf-abi/src/view.rs:389-393` has one field). New: `CtxShareFacts { parent, h_vaspace, flags, subctx_id }`
  per `(client, handle)`, and one bounded log line per guest ctxshare alloc (`CTXSHARE-FACTS`, the first 256 per process, then one
  `suppressed` line): decoded flags, the decoded kind, the declared `subctxId`, `hVASpace`, parent TSG. **Limit, stated:** the guest's
  returned VEID is NOT visible to kayfabe: the alloc reply is an echo of the guest's own params (`kf-gsp/src/rpc.rs:598-608`), so the
  `subctxId` seen is what the guest wrote INTO the request. The VEID that exists is the HOST's, logged in the `TSG-MIRROR CTX` line
  (below). The guest-side VEID needs the guest recorder named in section 8.
* **Guest channel alloc.** `ChannelAlloc` (`kf-rm/src/chanlink.rs:205-206,238`) already carries `flags`, `ctx_share`, `tsg`; new field
  `ctx: Option<CtxShareFacts>` = the facts of the ctxshare the channel names, `Some` only if this link saw that ctxshare allocated
  (`ctx_share_known` today). The existing per-channel `CHAN-FACTS` line (`chanlink.rs:1417+`) already prints flags; it is unchanged.
* **Validation, all before any host call, all by name, all counted:**
  1. channel names a ctxshare the link never saw -> refused `NV_ERR_OBJECT_NOT_FOUND` (0x57), counter `ctx_unknown` (today it silently
     became its own host group; "never fall back silently");
  2. the ctxshare's recorded parent is not the channel's TSG -> 0x1f;
  3. ctxshare `flags` outside {0 SYNC, 1 ASYNC, 2 SPECIFIED, 3 PREFER_LOWER} (the guest RM compares the whole word,
     `kernel_ctxshare.c:442-446`) -> 0x1f, counter `flag_refused`;
  4. channel flag word through the allowlist (10.2) -> 0x1f for a refused bit, counter `flag_refused`;
  5. mode contradiction (10.3) -> 0x1f, counter `mode_refused`;
  6. ctxshare space or engine contradiction with the group's record (10.3) -> 0x1f;
  7. the 65th ctxshare of one TSG -> `NV_ERR_NO_MEMORY` (0x51, what the host answers), counter `cap_refused`.

### 10.2 The host-flag allowlist (explicit-mode channels only)

The host channel flag word is **built**, never copied: `DENY_PHYSICAL_MODE_CE` (bit 7, always, as today) plus, only if the guest set it,
`GROUP_CHANNEL_RUNQUEUE` (bit 4). No other bit of the guest word can reach the host word. Classes of the guest bits (`alloc_channel.h`, [595]):

| guest bits | class | rule |
|---|---|---|
| 4 `RUNQUEUE` | FORWARD | host bit 4 = guest bit 4 (probe 1.2a: a compute channel next to a 3D subcontext needs it; Windows sets it on all 1 393 joiners) |
| 1:0 `CHANNEL_TYPE`, 2 `VPR`/`CC_SECURE`, 25 `SET_EVICT_LAST_CE_PREFETCH`, 26 `VGPU_PLUGIN_CONTEXT`, 29:28 `THREAD` = 3 (undefined) | REFUSE | loud named refusal, 0x1f, `flag_refused` (no real guest sets them: absent from every `CHAN-FACTS` word in `traces/` and `docs/`; confidential-compute, VPR and vGPU-plugin bits carry meaning the host must not grant) |
| 3, 5 `PRIVILEGED_CHANNEL`, 6 `DELAY_CHANNEL_SCHEDULING`, 7, 8-11, 12-21 (USERD index), 22, 23 `SKIP_SCRUBBER`, 24, 27, 29:28 = 0..2, 30, 31 | CONSUMED | the guest RM's own business, present in real guest words (`0xa00120`, `0x80201620`, `0x201520` in the repo's CHAN-FACTS histogram); NOT forwarded. Bit 5 never reaches the host (channels stay USER, `birth_privilege`); bit 6 is never set (late members run unscheduled by design, section 1.3); thread bits are not forwarded either |

Legacy-mode channels are NOT put through the allowlist and get today's word (`DENY_PHYSICAL_MODE_CE` only): legacy stays bit for bit
(refinement of section 6, which allowed `thread` bits for legacy; kayfabe does not use that allowance).
Host ctxshare `kind` = two-valued: SYNC (flags 0, or SPECIFIED with id 0), else ASYNC (flags 1, 3, SPECIFIED id != 0). The guest's
`subctxId` is never forwarded (the host picks the VEID); ctxshare `hVASpace` = the channel's resolved twin space handle, never 0.

### 10.3 The group map and the mode

`groups: (hClient, hTsg, shard) -> Group { host_tsg, mode, engine, members, ctxs: guest_ctx -> {host_ctx, kind, space, members} }`
where `shard == 0` always, except under `KF3_TSG_MIRROR=0` where `shard == hContextShare` (today's key, mode forced LEGACY: the A/B kill
switch is the same code with the old key). Mode is fixed by the FIRST channel that creates the group: `hContextShare == 0` -> LEGACY,
else EXPLICIT. A later channel that disagrees (a legacy channel into an explicit TSG or the reverse) is refused 0x1f BY NAME before any
host call - the status the host gives (probe 53) and the guest RM gives. A channel whose engine type differs from the group's, or whose
resolved twin space differs from the ctxshare's recorded space, or whose ctxshare handle was reused with a different (kind, space) while
members live, is refused 0x1f the same way. A channel without a known TSG is not in the map (bare channel: today's path).

### 10.4 Order of host operations and unwinding (explicit mode)

First channel of a guest TSG: `birth_group` (engine of the first member, `hVASpace` = its twin space) -> `ctxshare` (parent = host TSG,
kind, `hVASpace` = twin space) -> channel (`hContextShare` = host ctxshare, flags per 10.2, `hVASpace` 0) -> `NVA06C BIND` of the group
(first member of the host TSG only) -> work-submit token. A later channel of a known ctxshare: channel -> channel `BIND` -> token.
A later channel of a new ctxshare (the D3D12 joiners): ctxshare -> channel -> channel `BIND` -> token; the group is already scheduled
and the joiner needs no schedule of its own (probes 60-66, 90, 91). The ctxshare is allocated **lazily at the first channel that names
it** (not at the guest's ctxshare alloc, which the plane never sees as a statement): a ctxshare without a channel has no effect on the
host and its order relative to its channels is preserved. Refinement of "as the guest allocates them", stated so it is not read as a miss.
Unwinding, every step: the channel step fails -> free ctxshare iff it was created by this birth, then the group iff created by this birth;
the ctxshare step fails -> free the group iff new; the group step fails -> nothing. The group map is committed only after the host
channel exists, so a refusal leaves no half group. A failed unwind step is logged and counted (`unwind_failed`); the object it leaked
is the host RM client's, freed with the TSG (`forget` cascades) or at the client teardown.

### 10.5 Free order

`release_twin` frees the channel first (as today: `free_member`); then, if it was the last member of its guest ctxshare, the host
ctxshare; then, if it was the last member of the group, the host TSG. The map is updated before the host verbs (as today) so a refused
free cannot make a second free decrement twice. Legacy mode: exactly today (`free_member`, then `free(tsg)` on the last).

### 10.6 Guest TSG controls on the one host TSG (and what they did at base)

| Control | At base | Now (explicit group) |
|---|---|---|
| `GPFIFO_SCHEDULE` TSG `0xa06c0101` | `chan.rs:3553-3600`: every twin of the guest TSG; `schedule_group_once` (`latejoin.rs:131`) dedups by host group, i.e. one verb PER guest ctxshare (several host groups) | the same code; the dedup collapses to ONE verb on the one host TSG |
| late joiner | `chan.rs:6841`, `userrelay.rs:655`: an authored schedule of the joiner's own (new, unscheduled) host group | not needed: the joiner joins the already-scheduled host TSG. The call is **skipped for explicit groups** and counted (`latejoin_skipped`) and printed in the BORN line (`late_joiner_schedule=SkippedExplicit`). Legacy groups: as today |
| `SET_TIMESLICE` `0xa06c0103` | `chan.rs:3866-3930`: every host group existing at the statement; not remembered (a ctxshare group born later, i.e. all D3D12 joiners, ran at the host default) | same code, one group: the host TSG exists from the first channel, so the timeslice D3D12 sends before its schedule applies to every later joiner. No remembering needed (section 7 item 3 refined) |
| `PREEMPT` `0xa06c0105` | `chan.rs:4407-4447`: one host verb per twin (not deduplicated) | explicit groups: one verb per host TSG; legacy groups: unchanged (one per twin) |
| `STOP_CHANNEL` | `chan.rs:4105-4146`: `DISABLE_CHANNELS` on the twin, then TSG-level `schedule_enable(false)` (`:4139`) on its whole group | explicit groups: `DISABLE_CHANNELS` then the CHANNEL-level `NVA06F GPFIFO_SCHEDULE(false)` (probe 98: only that channel stops; TSG-level `false` is not a stop, 1.4). Legacy: unchanged |
| `GPU_EVICT_CTX`, Windows user-work evict | `chan.rs:4634`, `:5241` (`schedule_enable(false)`, TSG level) | explicit groups: channel-level `false`. Legacy: unchanged |
| re-enable after STOP | `chan.rs:3567-3586`: `DISABLE_CHANNELS(false)` then TSG schedule | explicit groups: additionally the channel-level `true` for a twin STOP took off |
| channel disable (`DISABLE_CHANNELS`) | channel-scoped (`HostRm::disable_channels`) | unchanged (probe 99) |

The channel-level verb is new in `kf-host` (`HostRm::schedule_channel_enable`, `NVA06F_CTRL_CMD_GPFIFO_SCHEDULE` `0xa06f0103`, the
same params as the TSG one, forced `bSkipSubmit = bSkipEnable = 0`).

### 10.7 Kill switch, counters, logs

`KF3_TSG_MIRROR=0` (measurement class, default ON, read once at plane construction) restores base behaviour: shard = ctxshare handle,
mode always LEGACY, no ctxshare, no allowlist, late-joiner call as at base, STOP/evict/preempt as at base. Counters (plain atomics,
bounded by construction, in the `chan[...]` status line as `mirror=[...]`): host ctxshares created / freed, groups created, `mode_refused`,
`flag_refused`, `ctx_unknown`, `cap_refused`, `latejoin_skipped`, `unwind_failed`. Logs: one `TSG-MIRROR BIRTH` line per guest TSG
(mode, first ctxshare, kinds, guest flags and host rq bit), one `TSG-MIRROR CTX` line per host ctxshare created (guest handle, kind,
host handle, host-returned VEID); both capped at 512 lines per process.
