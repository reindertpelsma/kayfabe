# V3 guest-influence audit: host-side paths a guest can steer, and their bounds

**STATUS: LIVE, 2026-11-20 (claude/guest-influence-20261120; GPU-free; no hardware run yet. Input: the
adversarial reading in draft PR #537, `docs/design/V3_DRIVER_PAIR_CONFORMANCE.md` §3.3-3.4, re-derived here
from the code, not taken over.)**

Every severity below is a reading of the code at `next` 7fb1c98a0 plus, where marked, of ogkm 580.95.05
source. Nothing here was run on a GPU. Severity: HIGH = can hurt the host, another VM or another process on
the shared GPU; MEDIUM = host resource exhaustion bounded by a per-VM quota; LOW = guest-local effect.

## The table

| id | path | file:line (at this branch) | what a hostile guest can do | privilege, OS | bound before | severity | GPU-free test | fix |
|---|---|---|---|---|---|---|---|---|
| G-1 | guest `RM_ALLOC` class `GT200_DEBUGGER` (0x83de) -> `ChanStatement::Debugger` -> one host debugger alloc per RPC | `kf-rm/src/chanlink.rs:1099` -> `kf-qemu/src/chan.rs:5754` (`debugger`) -> `kf-host/src/channel.rs:1891` | Allocate host debugger sessions without limit over its own GR object. In host RM each session also builds its own internal client, device and subdevice (`kernel_sm_debugger_session.c` `_ksmdbgssnInitClient`, ogkm 580.95.05; INFERRED cost, not measured). The act is queued at statement time, BEFORE the object seat (`kf-rm/src/lib.rs:685`: the channel link is first in the chain), so even an alloc the object seat then refuses (a duplicate handle, or the graph full at `MAX_LIVE_HANDLES` 2^18, `rmgraph.rs:1667`) leaves a host session, and a re-sent handle replaced the map entry, so the old session stayed until its GR object was freed. Host kernel memory and host RM / GSP objects are shared with every other VM and process on the GPU. | guest userspace (CUDA allocates this class from user mode; distinct handles suffice once the graph is full); guest root for re-sent handles. Linux and Windows (no OS gate on the statement). | none on the count (`ACT_QUEUE_CAP` 4096 bounds in-flight acts only) | **HIGH** (inferred: host-side cost per session read from ogkm source, not measured) | `chan::debugger_tests::*` (9 tests) | PR A: busy handle refused, `DEBUGGERS_PER_GR_OBJECT` = 4 and per VM = the declared `channel-budget`, pending sessions counted, seat released when the act ends or is dropped |
| G-2 | guest engine-class alloc under a passthrough twin -> `ChanStatement::EngineObject` -> one host engine object | `kf-rm/src/chanlink.rs:1313` -> `kf-qemu/src/chan.rs:5145` (`engine_object`), `:2803` (`admit_engine_object`) | Add one host RM object per fresh handle on one channel. A refusal by the object seat IS withdrawn (`attach_engine_withdraw`), so the total is bounded by the graph's 2^18 live handles for the whole VM. | guest userspace; Linux and Windows | busy handle and 64 pending (`PENDING_ENGINE_OBJECTS`); committed objects only by `MAX_LIVE_HANDLES` (2^18 per VM) | **MEDIUM** (per-VM bound exists; whether 2^18 host objects exhaust GSP heap is unmeasured and could make it HIGH) | `chan::dispsw_tests::engine_objects_per_twin_are_capped`, `the_engine_object_cap_counts_pending_admissions`, `a_twin_below_the_engine_object_cap_admits_exactly_up_to_it` | PR A: `ENGINE_OBJECTS_PER_TWIN` = 128 (committed + pending) |
| G-3 | host handle counter `HostRm::mint` | `kf-host/src/lib.rs:1177`, `:159` (`next_handle`), `:169` (`admit_want`) | Drive about 889 million mints (allocs, frees not needed to be slow) until the counter wraps through `0` (host RM then picks a handle) into kayfabe's own client handle and every handle issued. A live handle is refused by RM; a stale one kayfabe still holds could then name a newer object of THIS VM's own host client. | guest userspace (any alloc loop); Linux and Windows | none (`wrapping_add`) | **LOW** (same host client only; needs a second, stale-reference defect to matter) | `mint_tests::*` (5 tests) | PR A: the counter never wraps; past `u32::MAX` every alloc is refused by name (`HOST_HANDLES_EXHAUSTED` 0x4B7A) before any ioctl |
| G-4 | guest `NV2080_CTRL_CMD_GR_CTXSW_ZCULL_BIND` -> host ZCULL_BIND with the guest's 64-bit VA | `kf-rm/src/chanlink.rs:1641` -> `kf-qemu/src/chan.rs:4253` -> `kf-host/src/channel.rs:1948` | Point the zcull save buffer of its own GR twin at any VA of the twin's space, including a kayfabe placement (`MapTarget::reserved`, e.g. the read-only USERD page). The context-switch writes happen in the twin's own VA space with its own PTEs; a read-only PTE should fault the guest's own context. Whether the context-switch engine honours a read-only PTE is NOT measured. Everything writable there is already writable by the guest's own GPU work. | guest userspace (GL/Vulkan drivers send it); Linux and Windows | size 24, own client, mode <= 2 | **LOW** (guest-local unless the read-only PTE is ignored; then still only this VM's own twin placements) | proposed `zcull_bind_refuses_a_va_inside_a_vmm_placement` | NOT FIXED here: needs the twin's reserved set at the plane; hardware question below |
| G-5 | `HOST_CONTROLS` is an inventory: `control_carry` passes an unlisted id through at the bench and at `[580.65.06, 581)` | `kf-abi/src/hostabi.rs:1232` | Nothing directly: a guest never chooses a host control id (below). The risk is a FUTURE kayfabe call site that sends an id with no row: its layout would go unchecked at the nine measured 580.x hosts. | n/a | none (a review habit) | **LOW / hardening** | `kf-abi/tests/control_partition.rs` (4 tests) | PR B: the static inventory; the default stays as is (see below) |

## G-5 precisely: what "passes through unchecked" means

Read end to end (`kf-rm/src/lib.rs:739` the served chain; `kf-rm/src/chanlink.rs` `on_control`; the plane's
statements in `kf-qemu/src/chan.rs`; every host control call site in `crates/kf-*` outside `kf-harness`):

- **No guest control id reaches a host ioctl.** A guest `RM_CONTROL` is either answered by a served-chain
  policy (kayfabe is the GSP), turned into a typed `ChanStatement` that the plane executes with a control
  id kayfabe chose, or refused by name by the unserviced ledger / FSM. `raw_control_opaque` has no caller
  outside `kf-harness`.
- **Guest-influenced VALUES do reach the host**, inside kayfabe-authored parameter blocks, for these rows
  (all `HOST_CONTROLS` rows): `0x83de0309` exception mask (4 bytes), `0x20801210` preemption flags/modes,
  `0x20801208` zcull VA and mode (G-4), `0xa06c0103` timeslice, `0x00801909` CUDA limit, `0xa06c0101`
  schedule enable, `0xa06c0105` preempt, `0x2080110b` disable channels (own twins only), `0x20808163` /
  `0x20808164` NVENC session acquire/release (zero params, `ENC_SESSIONS_PER_VM` 3). The host RM validates
  the values; kayfabe validates the shape.
- "Pass-through" in `control_carry` is the LAYOUT decision for those kayfabe-authored blocks (no
  conversion where the host's layout is the bench's), not a forwarding of guest bytes.

**The partition (static, GPU-free, `crates/kf-abi/tests/control_partition.rs`):** the 421 served ids of
`tools/api_boundaries/served.json` are 71 `HOST_CONTROLS` rows plus 350 ids answered or refused by kayfabe and
never sent to a host, committed in `tools/api_boundaries/answered_never_forwarded.txt`. The source scan
resolves every host control call site (26 constant sites, 11 listed non-constant sites whose ids come from
authored tables) to a row. **The list of ids in neither set is EMPTY**, and no host call site names an id
without a row. Six rows are not in served.json (`0x205`, `0x3d05`, `0x3d06`, `0x83de0309`, `0xa06c0102`,
`0xa06c0105`: the derivation's class filter does not read them); that is a gap in `derive_served.py`, not
in the partition.

**Not flipped yet:** with the list empty, making `control_carry` refuse an unlisted id at every host
(fail closed) changes nothing for the code as scanned. It is still left for a separate change because three
non-constant sites (`layout.cmd`, `map_cmd`, the `cmd` variables of `kf_rm::hostquery`) are proven by reading,
not mechanically, and the `kf-harness` probes (two `raw_control` sites) would also be refused. Proposed
sequence: (1) this inventory in CI; (2) log-and-count an unlisted id at the bench and in the interval
(`HOST-ABI UNLISTED`), (3) the coordinator's hardware run below shows zero counts, (4) flip to refuse.

## Caps and their derivation (proposed DoS-limits property names, PR #538 §2.4)

| constant | default | derived from | proposed property |
|---|---|---|---|
| `DEBUGGERS_PER_GR_OBJECT` | 4 | measured 1 per CUDA context (every committed `act debugger` line in `traces/`, 145 files; w827) | `limit-debuggers-per-gr-object` |
| debugger sessions per VM | the declared `channel-budget` (256..2048) | one CUDA context needs a GR channel of its own and holds one session | (follows `channel-budget`) |
| `ENGINE_OBJECTS_PER_TWIN` | 128 | measured peak 6 per twin (`traces/windows_matrix_20261011/gravity-vulkan/…/qemu.log`, `traces/v3_display/dispsw_20261003/…/run_fin_on_d84086df_qemu.log`) | `limit-engine-objects-per-channel` |

The committed traces hold only part of the lanes' logs (bulk logs are in the evidence archives,
`traces/EVIDENCE_ARCHIVE.md`), so the measured peaks are a lower bound on the lanes' real peaks; the margins
(4×, more than 20×) are chosen for that.

## What only hardware can prove (for the coordinator)

1. No lane trips a new cap: `v3_gates.sh` 9/9, `fast_suite.sh` 30/30 (raw/thin), `cuda_ladder.sh`, the
   66/66 app matrix and the Windows gates at this branch, with `grep -c 'DEBUGGER_.*BUDGET\|ENGINE_OBJECT_TWIN_BUDGET\|HOST HANDLES EXHAUSTED'` = 0 in every qemu log.
2. G-1's host cost: a guest loop of `GT200_DEBUGGER` allocs over one GR object on `next` before this
   branch, counting host RM clients / `nvidia-smi` memory and GSP heap; on this branch the fifth alloc is
   refused by name.
3. G-4: a zcull bind at a read-only twin page (USERD) and whether the context switch faults (guest-local) or
   writes it.
4. G-5: the log-and-count step on a non-bench 580.x host (e.g. 580.95.05) and on the bench.
