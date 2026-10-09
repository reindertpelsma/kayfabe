# V3 — why kayfabe interrupts the guest: source-tagged raise counters, and the interrupt-source findings

**STATUS: LIVE, 2026-10-09 — code and GPU-free tests on branch `claude/irq-source-trace-20261009`
(base `claude/windows-reset-20261009` = `2e5ddc5c`); NOT yet run on hardware.** §1 is counters that are always on
and quiet; §2-§4 are investigation reports (code read plus the VFIO reference trace `vfio-dvi-20261008/boot3`),
one of them with a default-off experiment flag (§3). Evidence tags: **[read]** = read from code, a header or ogkm in
this session (file:line); **[measured]** = counted from a run's trace (the RTX 4070 VFIO reference, boot3,
2026-10-08, or kayfabe runs 100-105 of 2026-10-08/09) with the script named; **[inferred]** = a reading that neither
states. Nothing here changes default behaviour.

## 0. Corrections to the run-104 record (read first)

These are about `traces/windows_reset_20261009/README.md` §13.2 on `claude/windows-pde-run104-20261009`, which this
branch does not contain; fold them into that README (above the text they correct) when the two meet.

1. **"GSP stall 155 pending at ISR time: hardware 24 % of 69,282 leaf-4 reads, kayfabe 0.2 % of 16,623" is a statement about
   two phases of the boot, not about the lock screen.** [measured] (`traces/irq_source_trace_20261009/tools/leaf4_bit27_by_phase.py` over `boot3/trace.log`, per-phase
   counts of `LEAF(4)` = `0xb81010` reads with bit 27 set): the 15,172 pending reads of the first 2 s
   (`20:10:10`-`20:10:12`, 18,404 reads, 82 %) and the 1,292 of the last 10 s (the driver teardown) are 97 % of the
   16.7 k; the busy phase `20:10:14`-`20:10:20` is 0.6 % (79 of 13,288), the quiet phase 0.8 %, the lock-screen idle
   `20:10:54`-`20:11:08` 0.3 % (14 of 4,609), and `20:11:10`-`20:11:39` 0.1 % (4 of 3,781). At the phase the Windows runs
   die in, hardware and kayfabe are both near zero. So the GSP interrupt rate is not the lock-screen difference.
2. **Replies never raise the GSP interrupt on hardware either** (§2.3), so "kayfabe does not raise it when it posts
   replies" is not a divergence.

## 1. What is counted, where, and how to read it

### 1.1 The counters

`kf_trap::irqsrc` (`crates/kf-trap/src/irqsrc.rs`): per `(source, vector)` the number of latches that **sent** a
message (the tree's `Raise::Message`) and that stayed **held** (a leaf or top enable clear), plus out-of-range per source.
Two relaxed `fetch_add`s and one relaxed load per latch; no lock, no allocation, no print. The vector is the CPU
interrupt-tree vector (leaf×32+bit, the number in the served kernel table), **not** the MSI-X vector (always 0).
Every call of `Device::latch_and_deliver(vector, source)` names itself:

| source (status name) | where it is raised | file |
|---|---|---|
| `dtim` DisplayTiming | display worker, after a frame edge left an ENABLED head-timing event pending (`rm_dispatch != 0`) | `display.rs` (the `latch_and_deliver(DISP_STALL_VECTOR, src)` after `dp.publish_events`) |
| `dawk` DisplayAwaken / `dsem` DisplaySem | same raise when no head-timing event is pending but an AWAKEN / `SEM_WIN` event is (priority timing > awaken > sem; one source per raise) | `display.rs` |
| `den` DisplayEnable | the guest's own write to `RM_INTR_EN_HEAD_TIMING` enabled an already-pending event (`trap_write` returned true) | `device.rs` `bar0_write_inner`, display arm |
| `eng` EngineNonstall(slot) | a host engine's notifier edge (`PT-NSI host <eng> notifier wake`) raised the engine's own non-stall vector | `device.rs` `worker_loop` `on_other` |
| `fifo` FifoRelay | a host `FIFO_EVENT_MTHD` edge relayed (`KF3_PT_NSI_RELAY`) | `device.rs` `worker_loop` |
| `tick` NsiTick | what optional pacing owed, released by the worker tick | `device.rs` `worker_loop` |
| `grr` / `cer` | Translated GR-tier relay / copy-engine relay (`KF3_TRANSLATED_CE_RELAY`) | `device.rs` `run_translated` |
| `rc` / `hot` / `pre` | `RC_TRIGGERED` / hotplug `POST_EVENT` / `RUNLIST_PREEMPT_COMPLETE` `POST_EVENT` posted to the status queue → GSP stall vector | `device.rs` `deliver_rc`, `deliver_hotplug`, `deliver_preempt_complete` |
| `trig` GuestTrigger | the guest's own `CPU_INTR_LEAF_TRIGGER` write (the `_osVerifyInterrupts` loopback), on the vector it names | `device.rs` `bar0_write_inner`, tree arm |
| `enr` GuestEnable | a guest `LEAF_EN_SET`/`TOP_EN_SET` that released already-pending vectors (bucket `v*`: one write can release several, one message) | same |
| `other` | a call site that did not name itself (none should remain) | — |

No raise is sent anywhere else: `Device::deliver` (`device.rs`) is the only place an irqfd is written, and every call of it
is either `latch_and_deliver` or the tree arm above.

### 1.2 The status-line segment

`kf3: …  irq[writes=W raised=R held=H oor=O by_vec[…] w1c[…] gsp[…]] …` (the existing four counters are unchanged):

* `by_vec[v154=120[dtim=120] v155=3[rc=2,hot=1]/h1[rc=1] v*=4[enr=4]]` — per vector with traffic: messages sent, then
  `[source=n,…]`; `/h<n>[…]` the latches that stayed held. `v*` is the "any vector" bucket.
* `w1c[L4=49/49,L0=2908/2908]` — the guest's write-1-to-clear writes per tree leaf and the pending bits they actually
  cleared (`kf_trap::cpuintr::CpuIntr::w1c_summary`). A write that clears nothing was a clear of a bit that was not
  pending.
* `gsp[replies=N events=M stall_sent=S stall_held=H]` — messages the GSP model posted to the guest's status queue (replies and
  events apart; `kf_gsp::PostStats`, kept across a device reset) and how many latches of the GSP stall vector (`0x9b` =
  155, `MC_ENGINE_IDX_GSP` 50) were sent / held. `stall_sent` is the number of times kayfabe interrupted the guest for the GSP.
* Sum rule (held by `irqsrc` tests): the per-source counts of a vector add up to the vector's sent / held; `raised` (the
  eventfd writes) must equal the sum of every `by_vec` sent count, because every path that writes the eventfd also names a
  source (a mismatch on a real line is a raise path without a source; §1.4 (c)). `held` is the sum of the `/h` counts.

Read it from the run's `qemu.log`: `grep -o 'irq\[writes=.*gsp\[replies=[0-9]* events=[0-9]* stall_sent=[0-9]* stall_held=[0-9]*\]\]' qemu.log | tail -1`
(the drainer prints the line every 2 s while it changes).

### 1.3 The trace event (trace mode only)

With `KF3_BAR0_READ_TRACE=1` (perturbing, default off) every latch is also queued in a 4096-slot MPSC ring
(`irqsrc::RaiseRing`, producers = any thread, consumer = the QEMU main loop). The C device's `kf3_msi_user` (the trace
mode's MSI path, `qemu/hw/misc/kf3/kf3.c`) drains the ring and writes one

```
kf3_irq_raise  (0000:01:00.0) vector 155 source 0xa outcome 1
```

per record, **in front of** the `vfio_msi_interrupt` line of the same wake, through QEMU's own trace backend, so both are in
one `trace.log` with the same timestamps. `source` = class in the low byte (`irqsrc::IrqSource::class`, names in the table
above; engine slot in the next byte), `outcome` = 1 sent / 0 held / 2 out of range. Several raises that coalesce into one
wake (the eventfd is cleared once) show as several `kf3_irq_raise` lines and **one** `vfio_msi_interrupt`; a held latch
(no message) is written at the next wake. The definition is appended to `hw/vfio/trace-events` of the QEMU tree by
`scripts/bench/build_kf3.sh` (the generated `trace/trace-hw_vfio.h` is already included by `kf3.c`), and
`scripts/bench/trace-events-kf3-reference.txt` is the VFIO reference's event list plus `kf3_irq_raise` (used by
`windows_broker.sh` and, for a kf3 boot, `win_vm.sh`). `KF3_ABI` is 26 (`kf3_irq_raise_next`).
**Not verified here:** the C change was not compiled (no QEMU tree is built in this session); `wire_mirror` (header ↔ Rust
declarations) and all GPU-free tests pass. A QEMU build error in `kf3.c` is the first thing to look for.

### 1.4 Falsifier (stated before the code)

The counters are wrong if (a) the per-source counts of a vector do not sum to the tree's sends for that vector (tested), or (b) a
ring record is torn or reordered within a producer (tested, 4 threads × 500), or (c) on hardware the `eng` + `fifo` + `grr` +
`cer` + `tick` + `dtim` + … counts do not add up to the `raised` counter of the same line, which would mean a raise path
without a source. (c) is checked by reading the first line from a run.

## 2. The GSP interrupt: does kayfabe raise it when it posts?

**Falsifier (stated before the reading).** "Kayfabe raises the GSP stall vector for a reply" is falsified by finding no
`latch_and_deliver(GSP_STALL_VECTOR, …)` on the reply path; "hardware raises it for a reply" is falsified by replies that are
not followed by a pending bit 27 of `LEAF(4)` or an MSI more often than chance.

### 2.1 Kayfabe [read, code at 2e5ddc5c]

* A reply is posted by `GspFsm::post` (`crates/kf-gsp/src/boot.rs:2354`), which writes the element, advances the write pointer,
  sets `swgen0_pending = true` (`:2406`) and counts it (`PostStats`). That flag is the **IRQSTAT register shadow**: the drainer's
  `publish` (`device.rs:2193`) stores `GspFalconIrqstat` as one of its four edge registers after the data, so the guest *can* read
  SWGEN0 pending in the falcon's `IRQSTAT`.
* **No interrupt leaves with it.** `ServiceReport::raise_status_irq` (`boot.rs:280, 1358, 1850`) is the FSM's "announce"
  flag, and **nothing in `kf-qemu` reads it** (`grep raise_status_irq crates/kf-qemu` is empty; its only consumer is the
  `kf-crec` replay). The reply paths — `apply_register` → `publish` (`device.rs:3482`), `release_settled` → `publish`
  (`device.rs:2839`) — never call `latch_and_deliver`.
* **Events raise only from three places**, each unconditionally after `posted > 0`, outside the GSP lock, on
  `GSP_STALL_VECTOR`: `deliver_rc` (`device.rs:2922`, RC_TRIGGERED), `deliver_hotplug` (`:3011`, a LIST `POST_EVENT`),
  `deliver_preempt_complete` (`:3071`, `RUNLIST_PREEMPT_COMPLETE`, only with `KF3_ASYNC_PREEMPT`). `GSP_INIT_DONE`
  (`boot.rs:1740`), `UCODE_LIBOS_PRINT`, `GSP_POST_NOCAT_RECORD` and every other event the FSM can post raise nothing, and
  the os-event batch (`GspFsm::deliver_events`, `boot.rs:2674`) is not called from `kf-qemu` at all.
* Whether a raise reaches the guest follows the guest's enables (`CpuIntr::latch`: sent iff leaf and top enable are set,
  else the bit stays pending and the enable sends it later, `cpuintr.rs`).
* Origin [read, the history and the test comment]: this is the old C emulator's behaviour. `crates/kf-crec/tests/cap1_differential.rs:31`
  (F-1): the C posted 202 status elements in `cap1` and announced none; `nvkvm_gsp_raise_swgen0` was reachable only from
  `nvkvm_gsp_deliver_events`. It worked because the guest polls (`kgspWaitForRmInitDone`).

### 2.2 The guest's side [read, ogkm 595.84]

* A reply is **polled**: `_kgspRpcRecvPoll` loops `_kgspRpcDrainEvents` until its expected function/sequence arrives
  (`kernel_gsp.c:2682-2830`). No interrupt is needed for an RPC to complete.
* The interrupt path is `kgspService_TU102` (`kernel_gsp_tu102.c:1042-1117`): it reads `IRQSTAT`, and on SWGEN0 clears it
  (`IRQSCLR`, "BEFORE (and never after) servicing", `:1086-1093`) and calls `kgspRpcRecvEvents` →
  `_kgspRpcDrainEvents(… FUNCTION_NUM_FUNCTIONS …)` (`kernel_gsp.c:6068-6082`), i.e. it drains whatever the queue holds. So the
  interrupt exists to deliver **events**; replies are collected by the poller.
* The firmware side (when the GSP raises SWGEN0) is not in ogkm (GSP-RM is a binary); it is read from the trace below.

### 2.3 Hardware [measured, RTX 4070 VFIO boot3 2026-10-08: `trace.log` + `gsp.jsonl`, scripts `traces/irq_source_trace_20261009/tools/{gsp_msg_vs_irq,gsp_msg_vs_irq_window,leaf4_bit27_by_phase,msi_leaf4_per_second}.py`]

* Over the boot the GSP stream holds 5,232 replies (command → reply pairs) and 78 events (72 `POST_EVENT`: 60 × notify 139
  `RUNLIST_PREEMPT_COMPLETE`, 11 × 33 `PSTATE_CHANGE`, 1 × 34 `HDCP_STATUS_CHANGE`; plus `GSP_INIT_DONE`, `GSP_RUN_CPU_SEQUENCER`,
  2 × `GSP_POST_NOCAT_RECORD`, 2 × `UCODE_LIBOS_PRINT`). The guest cleared `LEAF(4)` bit 27 by write-1-to-clear **49** times
  (`0xb81010 ← 0x8000000`), read `IRQSTAT` (`0x110008`) as `0x40` (SWGEN0) 48 + 1 times and `0` 49 times: about 49 services of
  the GSP interrupt for 78 events — **not** one per reply.
* After the init window (messages between `20:10:12` and `20:11:40`; 2,023 replies, 44 events), the share of messages followed
  within 2 ms by a pending bit 27 in a `LEAF(4)` read: **events 41 of 44** (MSI within 2 ms: 43 of 44); **replies 36 of 2,022
  (1.8 %)** (MSI within 2 ms: 440 of 2,023 = 22 %, which is the background rate of 18-500 MSIs/s). Replies carry no GSP
  interrupt; events do.
* The init window is different: `GSP_INIT_DONE` was posted at `20:10:09.79`, the guest enabled vector 155 at `20:10:10.039`
  (`0xb81210 ← 0x8000000`) and read bit 27 pending on **every** `LEAF(4)` read for 1.2 s (18,404 reads at 26 µs) until its first clear
  at `20:10:11.24`. So hardware's INIT_DONE (an event) latched the leaf, and the poll-mode boot left it pending; kayfabe latches
  nothing for INIT_DONE, so its pending bit is 0 there. This is where the 24 % comes from (§0).

### 2.4 Answer

| | kayfabe | hardware |
|---|---|---|
| reply posted | IRQSTAT SWGEN0 shadow set; **no** tree latch, **no** MSI | no GSP interrupt seen (1.8 % of replies, about chance) |
| event posted | tree latch of vector 155 + MSI **only** for RC, hotplug `POST_EVENT`, `RUNLIST_PREEMPT_COMPLETE`; none for INIT_DONE/prints/PSTATE/HDCP/NOCAT | every event: pending bit 27 and an MSI (41/44, 43/44) |
| condition | unconditional after the post, then the guest's enables decide | n/a |

**Behaviour differs on events, not on replies.** [inferred] Candidates that the counters answer on the next run: (a) `gsp[events=…]`
vs hardware's 72 `POST_EVENT` in 100 s (kayfabe run 104: 2 in 12 s, both notify 139): a guest that waits on a `POST_EVENT` kayfabe never
posts (`PSTATE_CHANGE`, `HDCP_STATUS_CHANGE`) would not be woken; (b) `RUNLIST_PREEMPT_COMPLETE` is posted only after a host
disable+preempt completes (`KF3_ASYNC_PREEMPT` is in the Windows flag list) — hardware posts 60. Which requests the 60 answer is not
established (README §13.2 item 4).

**No flag is added.** The one change the code could take, raising the GSP vector on every post (`KF3_GSP_IRQ_ON_POST`), would
raise 5,232 interrupts for replies the hardware never interrupts for, i.e. add the divergence it is meant to remove; raising on
`GSP_INIT_DONE` and the other events kayfabe posts silently would match the boot3 trace but is a behaviour question about what
those guests do with it. Both are cheap to add; neither is clearly right. **Open question for the owner/coordinator:** after the
next run, if `irq[… gsp[events=N stall_sent=S]]` shows `S` much below `N`, an `KF3_GSP_IRQ_ON_EVENT=1` that latches the vector for
every posted event (and INIT_DONE) is the shape of the boot3 trace (2026-10-08).

## 3. The kernel interrupt table (`INTERNAL_INTR_GET_KERNEL_TABLE`, `0x20800a5c`)

**Falsifier (stated before the reading).** "The extra rows come from the host's static table, not from a capture" is falsified
if `query_intr_table` reads no host control for them; "the shaped table equals the real GSP's for every row both state" is
falsified by `authored::hw_shape` tests failing.

### 3.1 How the reply is built [read, code at 2e5ddc5c]

`hostquery::query_intr_table` (`kf-rm/src/hostquery.rs:626`): (1) the host's `NV2080_CTRL_CMD_MC_GET_STATIC_INTR_TABLE`
(`0x2080170e`), re-keyed from `NV2080_INTR_TYPE` to `MC_ENGINE_IDX` by `hostfacts::mc_engine_idx_of_intr_type`
(`hostfacts.rs:818`), verbatim including `intrVectorNonStall` (`derive_static_intr_table`, `hostfacts.rs:910-932`); (2)
`authored::engine_notification_rows` (`authored.rs:198`): one non-stall row per host engine, numbered by runlist; (3)
`authored::with_gsp_and_disp_rows` (`authored.rs:159`): the GSP (50 → `0x9b`) and DISP (2 → `0x9a`) stall rows. Introduced by
`28638efa` (2026-09-24, "HostFacts from the real host RM") and `7320292f` ("we are the GSP", w827). So **everything is host-derived or
authored by rule; nothing is captured** — the GA106 capture is the test oracle only.

### 3.2 Each discrepancy (hardware boot3 vs kayfabe run 100 of 2026-10-08, RTX 4070; tables dumped by `traces/irq_source_trace_20261009/tools/kernel_table_dump.py`)

| discrepancy | why the row exists [measured] | derived or authored | minimal derived correction |
|---|---|---|---|
| **rows 61, 63, 64, 1 (TMR) with stall 132/133/134/148** | the host RM answers `0x2080170e` with 12 types, these four among them (`mc_engine_idx_of_intr_type`: 0x1→61, 0x2→63, 0x3→64, 0x7→1); the real GSP's kernel table has none of them (24 rows: 59, 62, 60, 73, FECS_LOG 156-163, GSP 50, DISP 2, engines) | **host-derived**, unfiltered | drop them: `ctrl2080mc.h:262` says the control is *"the static interrupts needed by VGPU from Host RM"*; the VF consumer `intr_vgpu.c:222-260` has no GSP to leave them to; for a GSP client `kgmmuRegisterIntrService_IMPL` registers 61/63 only `if (!IS_GSP_CLIENT)` (`kern_gmmu.c:2273-2286`). **INFO_FAULT (64) and TMR do have CPU-RM services in a GSP client** (`kern_gmmu.c:2237-2250`, `timer.c:1959`), so for these two the rule is the hardware measurement alone [inferred: GSP-RM owns them in the physical table]. |
| **`vectorNonStall == vectorStall` for 59/60/61/62/63/64/73/1** | the host's static reply echoes the stall vector in `intrVectorNonStall` (kayfabe copies `word(3)` verbatim, `hostfacts.rs:932`); this matches the VF consumer, which sets `intrVectorNonStall = intrVectorStall` itself (`intr_vgpu.c:84-85`, [measured]). Real kernel table: `-1` for 59/62/60/73 | **host-derived**, unfiltered | non-stall = invalid for every static row (they are stall types: faults, access counter, doorbell, FECS log). |
| **non-stall numbered 0-5, hardware 0,2,3,7,8,10,11,18** | `engine_notification_rows` numbers async CEs and video engines by runlist (`authored.rs:198-235`) because `MC_GET_ENGINE_NOTIFICATION_INTR_VECTORS` (`0x2080170d`) is `NOT_SUPPORTED` to a usermode client (measured `f4b78ed9`, GA106, 580.159.04; not re-measured on 595.91.07) and every guest interrupt is injected by us from a host event, so only distinctness matters | **authored by rule** (`Why::Advertised` in spirit) | none derivable: hardware's numbers look like the device-info interrupt ids, which no unprivileged host control reports. Harmless as long as distinct. |
| **no non-stall row for CE4 (19)** | the host's `GET_ENGINES_V2` list (`hostquery.rs:279`) has no CE4: the host's FIFO latency-buffer census (`run103 qemu.log:12`) lists engines `0x1 0x9 0xa 0xb 0xc 0x13 0x1c 0x22 0x33` = GR0, CE0-CE3, NVDEC0, NVENC1, SW, OFA — no `0xd` | **host-derived** (the engine set) | none in the table code: it follows the engine list. Whether the host list omits an engine the GSP lists on the same die is a question about the host control, not the table [open]. |
| **no non-stall row for SEC2 (47)** | `classify_engine` (`hostquery.rs:236`, doc `:230`): "NVJPG and SEC2 are read and deliberately not advertised" | **authored (a decision)** | none; advertising SEC2 is a served-engine-table decision, not an interrupt one. |
| (hardware only) rows CE0/CE1 (15, 16) with both vectors invalid | GRCEs get a row in the real table; `engine_notification_rows` gives a GRCE none (`authored.rs:211`) | — | add `INVALID/INVALID` rows for the host's GRCE mask (`grce_placeholder_rows`). |

### 3.3 The default-off flag

`KF3_INTR_TABLE_HW_SHAPE=1` (read once, `authored::intr_table_hw_shape`): `query_intr_table` runs the static rows through
`authored::hw_shape_static_rows` (drop `GSP_OWNED_STATIC_ROWS = [1, 61, 63, 64]`, non-stall invalid on the rest) and appends
`authored::grce_placeholder_rows`. Default behaviour is byte-identical; tests (`authored::hw_shape`): the shaped host rows equal the
real GSP's stall rows, no owned row survives, no GSP/DISP/engine row changes, and the relay's lookup still finds GR0 for a GRCE.
**Expected effect** [inferred]: the guest stops enabling vectors 132/133/134/148 (never raised by kayfabe, so no change in the
pending bits) and the 59/60/62/73 rows stop carrying a non-stall duplicate; interrupts counts should not move. It is a
consistency fix to be A/B'd, not a prediction of a cure. SEC2/CE4 and the numbering are not touched.

## 4. `0x611d80` — `NV_PDISP_FE_RM_INTR_EN_HEAD_TIMING(0)`

Read from ogkm 595.84 `src/common/inc/swref/published/disp/` (`v03_00`, `v04_00`, `v04_01` `dev_disp.h`),
`kern_disp_0300.c`, `kernel_head_0400.c`, `kernel_head_0401.c`.

### 4.1 The registers [read, ogkm headers]

| register | offset | fields (bit) | access |
|---|---|---|---|
| `EVT_STAT_HEAD_TIMING(i)` | `0x611800+4i` | `LAST_DATA` 1, `VBLANK` 2, `RG_LINE_A` 5, `RG_LINE_B` 6, `RG_SEM(0..5)` 16-21 (v04_01). Bit 0 is **not named** in any published header (kayfabe calls it LOADV for the `KF3_DISPLAY_LOADV` experiment). RW, write-1-to-clear (`_RESET` = 1) | latched event |
| `RM_INTR_EN_HEAD_TIMING(i)` | `0x611D80+4i` | `LAST_DATA` 1 is the **only** field the header names (`v04_00 dev_disp.h:37-40`, RW, init 0) | enable, plain RW |
| `RM_INTR_STAT_HEAD_TIMING(i)` | `0x611C00+4i` | `LAST_DATA` 1, `RG_LINE_A` 5, `RG_LINE_B` 6, `RG_SEM(i)` 16+i; R-only (`v03_00 dev_disp.h:209-222`, `v04_01 :46`) | `EVT_STAT & EN`: what reaches RM |
| `RM_INTR_DISPATCH` | `0x611EC0` | bit i = head i has an RM head-timing event pending | summary |

### 4.2 What `0x3f0060` is

`0x3f0060 = 0x20 | 0x40 | 0x3f0000` = **bit 5 `RG_LINE_A`, bit 6 `RG_LINE_B`, bits 16-21 `RG_SEM(0..5)`** (the six raster-generator
semaphores) — every RM-routed head-timing event **except** `LAST_DATA` (bit 1), whose enable is the vblank interrupt, and
`VBLANK` (bit 2, used only for panel replay: `kheadReadPendingVblank_v03_00` tests `VBLANK` only if `bIsPanelReplayEnabled`, else
`LAST_DATA`). The bit-5/6/16-21 names for the *enable* register are [inferred] from the identical layout of the stat register (the
enable header only lists bit 1).

**Who sets bits 5, 6, 16-21.** [read + measured, boot3 2026-10-08] Not CPU-RM, not the Windows guest: ogkm's only writer of the enable is
`kheadWriteVblankIntrEnable_KERNEL_v04_00` (`kernel_head_0400.c:62-79`), which does a read-modify-write of bit 1 only; in boot3 the
very first read of `0x611d80` (`20:10:12.570`) is already `0x3f0060`, and all 226 writes are `0x3f0062`/`0x3f0060` (the read value with
bit 1 set/cleared). [inferred] They are set before the guest touches the register, by GSP-RM's display init (the physical-RM half of
`kdispStateLoad`; the RG_LINE enables are driven through `NV2080_CTRL_CMD_INTERNAL_DISPLAY_SETUP_RG_LINE_INTR`,
`rg_line_callback.c:74`) or the VBIOS/devinit; ogkm cannot say which, the firmware is closed.

**What the guest's read-modify-write means.** The vblank on-demand pattern: `kheadWriteVblankIntrEnable` reads the register,
clears the pending `LAST_DATA` event (`kheadResetPendingVblank` → write `0x2` to `EVT_STAT`), then writes the read value with bit 1
set (enable) or cleared (disable). Boot3: 113 enable and 113 disable writes at 50-100 ms spacing, and 2,017 `EVT_STAT ← 0x2`
writes (the ISR's `kheadResetPendingLastData`, `kern_disp_0300.c:528-538`, plus these). On kayfabe the base is 0, so the guest
writes `0x2`/`0x0`: **functionally the same bit-1 behaviour**, but the register reads a different value and the guest writes
different words.

### 4.3 `STAT` (`0x611c00`) semantics, hardware vs kayfabe [measured, RTX 4070 boot3 2026-10-08]

* Hardware `EVT_STAT (0x611800)` reads `0x7` (1,902 reads) or `0x5` (115): bits 0, 1, 2 are **latched at every frame edge whether
  or not enabled** (README: `0x7` while `LAST_DATA` is disabled), and only a write-1-to-clear removes them (`0x2` clears
  `LAST_DATA`, leaving `0x5`). `RM_INTR_STAT (0x611c00)` = `EVT_STAT & EN`: it read `0x2` in 3,584 of 3,585 reads — it is read
  only from the ISR (after `RM_INTR_DISPATCH` bit 0 = 1: 3,592 of 14,508 reads), where `LAST_DATA` is pending by construction;
  one read of `0` is the stale one after a clear. Bits 5, 6 and 16-21 never appear in `EVT_STAT` (`0x7`/`0x5` only): nothing sets
  `RG_LINE`/`RG_SEM` events in this workload, so their enables do nothing observable.
* Kayfabe (`display.rs` `frame_edge`) raises `LAST_DATA | VBLANK` at every frame edge (the same latch) and derives
  `rm_head_timing = EVT & EN`, so with the guest's bit-1 RMW the `STAT`/`DISPATCH` reads and the interrupt per frame edge are the
  hardware's. The difference is the missing base: `EN` reads `0` instead of `0x3f0060`.

### 4.4 How the display model should present it (derived, not captured)

At reset the enable register of each head reads `RG_LINE_A | RG_LINE_B | ⋃ RG_SEM(i)` — the bit masks come from the same
generated field tables the model already uses (`NV_PDISP_FE_RM_INTR_STAT_HEAD_TIMING_RG_LINE_A/_B`,
`NV_PDISP_FE_EVT_STAT_HEAD_TIMING_RG_SEM(i)` for `i < __SIZE_1`), not the constant `0x3f0060`; `LAST_DATA` (and `VBLANK`) stay clear
until the guest sets them; the guest's write replaces the whole word (plain RW). Because `rm_head_timing = EVT & EN` and the model
never latches `RG_LINE`/`RG_SEM`, this changes nothing but the read-back and the words the guest writes. **Not implemented** (report
only): it is one initial value in `kf_disp::ports::Ports` (`head_timing_en`), but the field names exist only in the `v03_00`/`v04_01`
headers and the AD10x display IP's version mapping (`0x04040000`) has to be checked against them first [open].

## 5. Open questions

1. Which kayfabe source dominates the 150 MSIs/s at the idle lock screen? (§1: read `by_vec[...]` of the first line after the lock
   screen appears; `dtim` is the display frame edge, `eng`/`fifo` the relay.) Hardware: 17-19/s.
2. Does a guest need the events kayfabe never posts (`PSTATE_CHANGE`, `HDCP_STATUS_CHANGE`) or the interrupt for `GSP_INIT_DONE`?
   (§2.4.) Needs the requests these answer.
3. Is the host's `GET_ENGINES_V2` list missing CE4 relative to the GSP's on the same die? (§3.2.)
4. Is the C change to `kf3.c` accepted by the QEMU 10.2 build? (§1.3.)
5. Is the A/B of `KF3_INTR_TABLE_HW_SHAPE=1` worth a boot, given §3.3 predicts no interrupt change?

## 6. How to run it on hardware

Counters need no flag. In `wr-run.sh` (the run launcher), with the checkout `wreset` at this branch's tip and the binary built by
`scripts/bench/build_kf3.sh` for that revision:

```
flock -o /tmp/kayfabe-fastguest.lock bash /var/lib/kf-windows-20261005/wr-run.sh <N> <REV> ""                          # counters + kf3_irq_raise events
flock -o /tmp/kayfabe-fastguest.lock bash /var/lib/kf-windows-20261005/wr-run.sh <N+1> <REV> "KF3_INTR_TABLE_HW_SHAPE=1"  # the §3 A/B
```

Read: the last `irq[…]` line of `boundary-kayfabe-<N>/qemu.log` (§1.2) and the `kf3_irq_raise` lines in `trace.log` beside
`vfio_msi_interrupt`: `grep -c kf3_irq_raise trace.log`, and per source/vector
`grep kf3_irq_raise trace.log | awk '{print $5,$7}' | sort | uniq -c | sort -rn | head`.
