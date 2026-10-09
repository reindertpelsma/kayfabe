# V3 completion audit — completions, interrupts and notifications a Windows guest can wait on

**STATUS: RESEARCH, 2026-10-09.** Read-only code audit at `claude/va-split-straddle-20261009` (`a1d26cc8`) plus the
committed traces of runs 100-103 (`traces/windows_reset_20261009/`). Nothing was run on hardware; no behaviour changed.
Trigger: a Windows 11 guest stalls in `VidSchSuspendAdapter -> VidSchWaitForEvents` (every engine node must go idle), the
2 s / 30 s TDR fires, the driver resets and the screen goes black. Coordinator evidence (run 232, ETW): the **paging
node (the copy-engine queue) loses packets**: 3 submitted at t=108.39 s and 11 more at 119.68 s never complete until the
TDR recovery pass force-completes all 14 at once; 4651 earlier packets matched.
*Measured* = read from code or from a committed trace; *inferred* = reasoning not yet tested.

## 0. What is already known to be healthy (so the next reader does not redo it)

- **The paging node is a Translated ring.** Windows' kernel copy channel is `token 0x80c`, engine `0xb` (COPY2),
  `BORN Translated` (run102 qemu log line 10302); it also issues the `SPLIT-REQUEST` tickets. The runs-100-103
  *stall snapshot* (`KF3_PT_STALL_SNAPSHOT`) covers **Passthrough twins only**, so "host-side twin queues drained"
  says nothing about this ring. In run 102 the ring was drained at its first-life retire (`put == gp_get == 6794`,
  `dead=None`, 639/639 splits), so the paging ring itself did not stall there. In run 232 it is **unmeasured**.
- **Interrupt delivery was alive during the run-102 stall** (BAR0 trace, `run102-trace.log.gz`, 23:45:47.8-23:45:50):
  the guest ISR ran ~60 times a second and each time read `leaf0 = 0x6` (CE2 vec 1 + CE3 vec 2) and cleared both, with
  `TOP = 5`. So "no MSI at all" is not the stall mechanism of that run. (Scan of runs 100/102/103: no ISR read
  `TOP=1, leaf0=0` - the visible signature of finding 2 below was never seen; it is only visible when both words are stale.)
- The guest's ISR protocol (trace): `LEAF_EN_CLEAR(2)=0x101`, `LEAF_EN_CLEAR(4)=0xffffffff`, `TOP_EN_CLEAR=0x8`, read
  leaves, `LEAF_EN_SET`/`TOP_EN_SET(=0xfffffffe)` to re-enable, read TOP and `leaf0`, W1C each pending bit **then**
  service it (`ogkm-595.84 intr_nonstall_tu102.c:355-375`, `intrClearLeafVector` before `intrServiceNotificationRecords`).
  `CpuIntr::{LeafEnSet,TopEnSet}` re-send an already-pending bit (`cpuintr.rs:213-226`), so the re-enable window is covered.
- Token machine (`kf-trap/token.rs`), wake word, `try_park(seen)`, the drainer/worker/VA eventfd paths and the MSI-X
  mask/unmask path (`kf3.c:858-910`, KVM irqfd keeps a raise made while masked) have no lost-wake window (three
  independent reads agree). `chan[contended=0]` in every status line of runs 100-103: `serve()`'s `try_lock` loss never happened there.

## 1. Ranked findings (12)

### 1. A Translated ring dies silently: no RC, no fence, no interrupt, never pumped again
- `crates/kf-qemu/src/chan.rs:6829-6866` (any `ChanError` -> `g.dead = Some`, `completions.clear`); `serve()` returns at
  `chan.rs:6637` forever after; `crates/kf-chan/src/host.rs:1778` (`"no room for a completion tail"` kills the channel when
  pieces were pushed and the tail fence finds the ring full); also `ChanError::Bind/Sw/Publish` (split refused).
  Translated rings arm no error notifier (`chan.rs:1235`) and `rc_queue` is fed only for Passthrough twins.
- Scenario: a burst of paging packets fills the host ring; the tail fence is refused; ring declared DEAD. Every later
  doorbell on token `0x80c` is absorbed; the guest's fences are never written, no `RC_TRIGGERED`; the paging node never
  idles. Exactly the coordinator's picture (3 packets, then 11 more behind them, all force-completed by the TDR).
- Makes a scheduler wait forever: **yes**. Confidence: mechanism *measured from code*; occurrence in run 232 *inferred*.
- **Falsifier in one grep** on run 232's qemu log: `chan token 0x80c .* DEAD:`; and the 2-second status line's
  `tokens=[0x80c:...put=,gp_get=]` (`put > gp_get` at the stall = the ring is behind, not an interrupt problem).
- Test: `kf-chan` unit test with a fake host whose `fence_releasing` returns `Busy` after `push` accepted pieces; today the
  pump returns `Err`; required behaviour is to keep the work and retry on the next completion (or post `RC_TRIGGERED`).

### 2. CPU_INTR shadow publication is a load-then-store race: a latched leaf bit can be hidden after its MSI was already consumed
- `crates/kf-qemu/src/device.rs:1529-1535` (vCPU: `intr.write` then `intr.shadow` then `deliver`) against
  `device.rs:1730-1738` (`latch_and_deliver`: `intr.latch` then `shadow_all` then `deliver`, from workers, drainer, display
  thread); `crates/kf-trap/src/cpuintr.rs:207-215` (W1C), `:242-258` (`shadow` loads then `put`s), `:261-264`.
  The BAR0 reads are served from this shadow (`shadow_word`, `device.rs:1267`), the atomics are never read by the guest.
- Interleaving (inputs -> outcome): guest ISR W1C `leaf0 &= !2` on the vCPU (atomic cleared, loads `leaf0=0`);
  worker: `latch(vec 1)` sets bit 1, `shadow_all` stores `leaf0=2, TOP=1`, `eventfd` write (MSI); vCPU resumes and stores
  its stale `leaf0=0` and `TOP=0`. The MSI is delivered, the ISR reads `TOP=0`/`leaf0=0`, services nothing. The real
  atomic keeps bit 1. Only the **next latch of any vector** (it republishes all leaves) or a guest write to that leaf heals
  it. During the idle lock screen (`one flip per 30 s`, run 103; `vblirq` ~0) nothing latches, so the last completion of a
  burst stays invisible until the TDR - the 30 s of silence. Latch-vs-latch lost updates self-heal (the second MSI's ISR
  reads truth after its own W1C republish).
- Makes a scheduler wait forever: **yes, until the next latch**. Confidence: race *measured from code*; rate *inferred*
  (window ~100 ns per ISR clear; thousands of clears per second in bursts). Not observed in the traces (see §0).
- Test (loom/shuttle, `kf-trap` + a shadow trait): thread A = `write(Leaf(0),2)` + `shadow`, thread B = `latch(1)` +
  `shadow_all`; invariant after both: every published word equals the atomics. Fails today. Fix shapes: publish under one
  small mutex held by both paths, or loop `publish; if state changed re-publish`.

### 3. Non-stall edges are dropped when kayfabe does not see an arm; hardware latches them regardless and nothing catches up
- `crates/kf-chan/src/ptnsi.rs:388-391` (`NotArmed`), `crates/kf-qemu/src/chan.rs:2432-2500`, arms learned only from
  observed allocs (`crates/kf-rm/src/osevent.rs:310-340`) and zeroed at every fn 1 (`osevent.rs:371-380`, `:669`).
  Run 102 final: `edges=12135 not_armed=3071` (GR0 435, CE2 1216, CE3 630); `CE2 armed=0` the whole run, `arm_clears=2`.
- Scenario: host work retires between the fn-1 zeroing and the new KMD's arm (or the KMD arms through a form
  `nonstall_slot()` does not recognise); no leaf bit is set, and nothing remembers the edge; a later arm on already-completed
  work never fires (`the_interrupt_arming_model.md` requires "arm after completion fires at once"). On hardware the leaf pends
  regardless and RM services whatever is pending, firing FIFO_EVENT_MTHD for every non-stall (`intr.c:1203-1213`).
- Scheduler forever: yes if the node's wake rides a subscription kayfabe does not count. Confidence: mechanism *code +
  counters*; causal link *inferred*. Falsifier: a switch that latches the vector unconditionally; stall persists => cleared.
- Note: the Translated-CE relay is **not** gated on arms (`device.rs:3352`, `latch_and_deliver` straight); only host-edge
  relays are. The two share nothing but `latch_and_deliver`/`CpuIntr` (§3 of the questions below).

### 4. RUNLIST_PREEMPT_COMPLETE (139) has silent-drop paths; default config refuses the async preempt (TDR exactly TdrDelay later)
- Default (`KF3_ASYNC_PREEMPT` unset): `crates/kf-rm/src/chanlink.rs:246-259, 1747-1766` refuses `DISABLE_CHANNELS`
  carrying `pRunlistPreemptEvent` (`[measured runs 78/79]` TDR follows). With it on (all Windows runs):
  `chan.rs:3522-3527` skips the push when 64 are queued; `device.rs:3090-3105` consumes the entry with no requeue when there
  is no display model or no live registration (registrations are retired at every fn 1, `kf-rm/src/display.rs:955-963`);
  `chan.rs:3451/3453-3461` answer `NotOurs`/refuse a whole list when any entry has no twin (a TSG whose members are
  Translated rings or empty) with no 139; `chanlink.rs:256` refuses an event with `bDisable=0` where RM ignores it;
  `take_preempt_done` returns nothing while **any** reply is held (`chan.rs:2620`); `preempt_done` is not cleared at fn 1.
- Scenario: VidSchSuspend preempts the runlist, gets NV_OK, never gets 139 (list contains the Translated paging ring only
  through its TSG handle, which `resolve_disable_list`'s group closure does not expand for Translated rings,
  `chan.rs:3440-3461`) => waits for ever. Scheduler forever: **yes**, and it is literally the observed call chain.
  Confidence: drop points *measured from code*; firing in run 232 *unknown* (runs 100/102 posted 2 of 2 early). Falsifier:
  grep run 232 for `DISABLE_CHANNELS` / `RUNLIST_PREEMPT_COMPLETE` lines near t~108 s.

### 5. The device model has no reset: guest reboot, FLR and a GSP life cycle leave every Rust-side state alive
- `qemu/hw/misc/kf3/kf3.c:1947-1965` (`class_init` sets no reset); `GspFsm::device_reset` (`crates/kf-gsp/src/boot.rs:1172`)
  has no caller in `kf-qemu`; `CpuIntr` has no reset (`cpuintr.rs`); fn 47 is inert (`kf-rm/src/inert.rs:119`) and
  `observe_unloading` only reseeds BAR1 (`device.rs:2766`). `enter_halted` (`boot.rs:1460`) drops the queue binding but keeps
  `held`, `pending_command_doorbells`, the BAR0 read shadow, `OsEventLog` (cleared at fn 1 only), `preempt_done`, `rc_queue`,
  display `ScanState`/pacer/channel registry (fork audit: `kf-rm/src/display.rs:955-975` retires only registrations),
  `pt/by_obj/scopes` (rely on the guest's FREEs).
- Scenario: bugcheck 0x116 -> `reboot=reset` inside the same QEMU: GSP phase stays `Running` with the dead life's binding
  (run 102 tail: `PeerWritePtrOutOfRange {8192,63}` x2 on stale queue memory, then Code 43 "no NVIDIA adapter"). This is the
  "loop" the coordinator saw. Does not explain the first stall. Confidence: *measured from code* and from the run-102 tail.

### 6. MMU_INVALIDATE stays armed for ever on every `unreconciled` path; nothing retries
- `crates/kf-mem/src/vasmgr.rs:1610-1612` (also 1036, 1298, 1333, 1641), VA loop `device.rs:1970` (`last_seq == req.seq`
  stops re-picking it). Paths: refused UNMAP, refused host invalidate, slot overflow, walk submit/poll error, > MAX_SPACES,
  and **a root that moves during the walk** (`vasmgr.rs:1390-1393` queues `Want::Root` but the invalidate branch only
  requeues on `partial`, not `rewalk`, `:1553/:1610`). Owner ruling AA covers *absence* only. Run 223: Windows polled the
  trigger bit until TDR 0x117 (`kern_gmmu_tu102.c:112-120`).
- Scheduler forever: yes (the guest spins under its GPU lock; no idle completion is processed). Confidence: high that the
  state is stuck (code); medium that it matches run 232 (no `mem ... trigger armed` line is known there - grep it).

### 7. Held replies: head-of-line blocking, no deadline, and no retry when a post fails
- `crates/kf-gsp/src/boot.rs:2550-2570` (`release_held`: FIFO, a settle-pending head blocks the rest), `device.rs:2833-2860`
  (`release_settled` needs `inbox.all_settled()`, i.e. the VA thread momentarily idle), `device.rs:3086` (139 gated on
  `held_len()==0`), `boot.rs:2589` + `device.rs:3449` (a failed `answer` - `QueueFull`, `ProcessorSuspended`,
  `QueueNotBound` - decrements the doorbell count first, logs "REFUSED", breaks; the command is left owed, nothing re-kicks it;
  the guest times out). `enter_halted` keeps `held` -> a stale-sequence reply can land in the next life's queue.
- Scheduler forever: possible via #4 (139 behind a held map reply) and any reply the KMD waits on. Confidence: *code*;
  held replies balanced 205/205 in run 102, so not hit there.

### 8. Display completions are gated on an optional console-frame copy, and `ScanState.failed` is permanent
- `crates/kf-qemu/src/display.rs:2796-2840, 3170-3188` (notifier/release/GET and `Effect::Heads` wait on `scan.barrier`);
  `:2825-2830, 3865-3869` (`failed` is never reset: every later Notify/Release/GET is dropped and `halt_scanout` runs, also
  for the next driver life); `:2900, :2929` (a notifier/semaphore write that fails to resolve is dropped while GET still
  advances, no event bit raised); `kf-rm/src/display.rs:955-975` (display channel/pacer/`core_head_state` survive fn 47/fn 1,
  `engine.free(Core)` emits no `Effect::Heads`). Not in the paging node's chain, but `FlushScheduler` also waits on the
  flip/present nodes. Confidence: *code*; stall link *inferred* (runs 98/99 show the display KMD waiting on exactly these).

### 9. Hotplug is a kayfabe-authored trigger for the observed stack
- `device.rs:3008-3070` (`queue_hotplug` from UI/broker resize), `kf-disp/model.rs:642-652`. A monitor change posts
  HOTPLUG; Windows answers with `DxgkPollDisplayChildren -> ... -> AcquireCoreSync -> SuspendScheduler` at an arbitrary time
  while GPU work is outstanding. It converts any pre-existing lost completion into the observed stall; it is not a loss itself.
  Falsifier: correlate `display: hotplug posted` lines with the stall time in run 232.

### 10. Events are dropped, not requeued, when the GSP phase is not `Running`; late `IRQSCLR` can wipe a newer event's status
- `boot.rs:2668-2680` (`post_event` -> `NotRunning`), `device.rs:2987/3021/3134` (RC, hotplug, 139 consume and log);
  `boot.rs:1405-1430` (IRQSCLR is a trapped ring write applied later by the drainer; an RC/hotplug/139 posted in between
  has its `IRQSTAT` cleared; `publish()` dedups on `published`, `device.rs:2214`, so the shadow is not restored) -> the guest ISR
  finds status 0 and returns; the message waits until the next RPC. Confidence: code, narrow window.

### 11. `serve()`'s `try_lock` consumes the ring; `schedule_translated` mis-reads contention as "not stopped"
- `chan.rs:6631-6636` (contended -> `return false`, token released as idle: a doorbell/completion ring with no pump),
  `chan.rs:3820` (`slot.try_lock().is_ok_and(|g| g.stopped)`: on contention the restart branch is skipped, `stopped` stays
  true and the ring is never re-enabled), act holders at `chan.rs:2900, 3499, 3643, 3764-3772, 3847` (GR/disable/schedule
  acts hold the slot lock around host verbs). The relay path already learned this once (run 77, `chan.rs:4998`).
  **Measured `contended=0` in runs 100-103**, so downgraded; still a real lost-doorbell hazard for Translated rings.

### 12. A failed disable leaves the Translated ring disabled for good; stale queues carry into the next life
- `chan.rs:3502` sets `g.disabled = true` before the host verb, `chan.rs:3511` returns on error without undoing it
  (`serve` then never pumps that ring); `preempt_done`/`rc_queue`/`pt` are not cleared at fn 1 and Windows reuses
  `0xc1d0xxxx`/`0xff04xxxx` handles, so a queued 139/RC of a dead life can reach the new one. Confidence: code, low-medium.

## 2. The coordinator's four questions (CE completion -> leaf bit -> MSI)

(a) **Set, cleared by the guest, set again - lost edge?** The atomics never lose it: `latch` is `fetch_or`, the guest's W1C
is `fetch_and(!v)`, and RM clears **before** it services (`intr_nonstall_tu102.c:369-375`), so an edge after the clear
re-pends. The loss is on the *published copy* the guest reads (finding 2). A second event while the leaf is still set is
merged into the same bit (hardware does the same) and raises another MSI (`asserted_leaf`) - never dropped, but one
service covers both: correct only because the GPU wrote every fence before the latch.
(b) **11 packets in 1 ms:** `serve()` retires them in one or a few pumps; each pump with a moved `GP_GET` bumps
`cpending` once per engine (`chan.rs:6716-6733`), `for_each_ce_relay` (`chan.rs:2660`) swaps it and the worker does one
`latch_and_deliver` per `run_translated`. So at most one latch+MSI per pump, not per packet; the ISR sees the whole leaf.
Safe, because the GPU wrote the guest's fences before the host fence the pump reacts to (`translated.rs` forwards the
guest's semaphore words unchanged).
(c) **Order at the ack:** vCPU: `fetch_and` -> `shadow()` stores (leaf, EN words, TOP recomputed) -> `deliver` (no MSI for a
W1C). Worker: `fetch_or` -> `asserted_leaf` -> `shadow_all` -> `eventfd`. The two shadow store sequences are unordered
(finding 2). No lock is shared; `shadow_store` is a plain atomic store into the BAR0 shadow page.
(d) **Shared state, Translated-CE relay vs Passthrough relay:** only `CpuIntr`/`latch_and_deliver`. Translated: per-engine
`cpending` (`chan.rs:313, 2662`), unconditional on arms. Passthrough/host edges: `Relay`/`Pacer` (`ptnsi.rs`), gated on
`NonstallArms`. Both can land on CE2's bit (vec 1): `host_notify_vector` prefers an *unarmed* engine's vector as the
FIFO_EVENT_MTHD carrier (`ptnsi.rs`), which in run 102 was CE2 (`CE2 armed=0`, `v1 raised=5010`).

## 3. What to measure next (cheapest first)

1. run 232 qemu log: `DEAD:` lines; the 2 s status line `tokens=[0x80c: ... put=, gp_get=]` at the stall (separates "ring
   behind" from "interrupt lost"); `HELD-REPLY` without `-POSTED`; `trigger armed` without `idle`; `RUNLIST_PREEMPT_COMPLETE`.
2. `KF3_COMPLETION_PROBE=<ms>` (already implemented, `chan.rs:probe_tick`): HOST-FENCE-OVERDUE dumps for Translated rings.
3. Extend the stall snapshot to Translated rings (put/get/dead/disabled/stopped/unresolved/split ticket).
4. A debug counter in the BAR0 trace: at each MSI also record the atomics (`snapshot()`, `cpuintr.rs:166`) beside what the guest
   reads; any divergence is finding 2.
