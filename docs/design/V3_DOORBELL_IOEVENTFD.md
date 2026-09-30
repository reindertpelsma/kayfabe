# V3 — the doorbell fast path: one KVM ioeventfd per live token, serviced by the register drainer

**STATUS: BUILT + GPU-FREE-TESTED, 2026-09-30; hardware verdicts and nested measurements in §7 as
they land. Branch `v3-ioeventfd` (off `v3-ci`).** Default **OFF** (`-device kf3-gpu,…,
doorbell-ioeventfd=on` turns it on) until the measurements in §7 justify a default. Nothing here is
a non-nested result: every number is from a Vast KVM box (itself a KVM guest) or from the
development workspace (also virtualized), and is labelled **nested**.

## 0. Owner direction, in order (binding; `docs/OWNER_RULINGS.md` §D)

- **2026-09-28:** non-nested baseline and host-side / ioeventfd work first, optional guest helper
  afterwards. Keep the table fallback for unmatched / unregistered doorbells. **No timers, no
  deliberate batching delay**: act promptly; coalescing only when several notifications are already
  pending; never lose a notification.
- **2026-09-30 (refinement, supersedes the "passthrough only" scope of the first brief):** the
  ioeventfd path applies to **all** doorbells — Passthrough **and** Translated/Emulated — because
  every doorbell must be forwarded as quickly as possible. The eventfds are serviced on the existing
  **register drainer** (which does no blocking work, and must stay that way): a Passthrough token's
  host twin is rung **inline in the drainer**; a Translated/Emulated token is handed to its
  channel's existing translation/execution path **immediately**, with the same ordering and
  real-GPU completion semantics as today — only the notification transport changes.
- **2026-09-30:** a registration KVM refuses must leave that token on the trapped path, named and
  counted, and a test must exhaust the limit and show every overflowed token still works via the
  trap (§5, §6). The drainer waits on the doorbell eventfds with its other work and never
  batches/delays them; its wake-to-ring latency (p50/p99) under concurrent register traffic is
  measured, besides the vCPU exit cost (§7).

## 1. What changes, and what does not

```
 guest store of V to the doorbell (BAR0 usermode page +0x90, or a Hopper+ BAR1 view of it)
   │
   ├─ V is a live, registered token's exact value ──► KVM (in the host kernel, no exit to QEMU):
   │                                                    eventfd_signal(token's eventfd)
   │                                                      │
   │                                          register drainer (epoll: its wake + every doorbell fd)
   │                                            drain the counter, then deliver ONCE:
   │                                            Plane::trap_write(Class::Doorbell, V)  ◄── the trap's own arm
   │                                              Passthrough → rm.doorbell(host twin)   (inline, drainer)
   │                                              Translated  → RUNG + bitmap + worker wake
   │                                              dead / unknown → nothing
   │
   └─ anything else (unregistered token, non-canonical value, a site not yet placed, a refused
      registration) ──► exit to QEMU ──► kf3_bar0_write ──► the same arm, on the vCPU — unchanged
```

- **The decision is the trap's, by construction.** `kf_chan::dbfast::Sink::deliver` is implemented by
  the device as `Plane::trap_write(Class::Doorbell, 0, doorbell, V, 4)` — the same function a trapped
  write reaches — so the route, the host token, the RUNG/BUSY_RUNG protocol and the §5.2 stamp are
  the trapped path's. Nothing new reads a pushbuffer, a GP entry or a cursor.
- **Identity still comes from the token table, never from the trap** (`THE_CONSTRAINTS.md` §47). The
  ioeventfd's DATAMATCH only says *"a store of exactly V happened"*; which channel V means, and
  whether it is live, is read from the token word at delivery.
- **Completions are untouched.** Passthrough completions are the GPU's; Translated completions are
  still host events on the completion fd, retiring through the same worker (`§8`, no forge).
- **The vCPU does less, never more.** A matched store costs the guest a VM exit handled in the host
  kernel (instruction emulation + `eventfd_signal`) instead of an exit to userspace. The unmatched
  path is byte-for-byte today's.

## 2. Code map

| where | what |
|---|---|
| `crates/kf-chan/src/dbfast.rs` | the fast path: registry (sites × tokens, budget), per-registration eventfd, drainer half (`service_ready`, `on_ready`), removal with final drain + acknowledgement, counters, histograms |
| `crates/kf-trap/src/tokenindex.rs` `GuestTokenFormat` | the exact 32-bit value the guest stores for `(runlist, chid)`, per die group (§3) |
| `crates/kf-qemu/src/device.rs` | `impl Sink for Device` (the trap's arm), drainer loop (doorbells first, before every privileged apply, and after every wait), `enable_doorbell_fast_path`, `dbfast[…]` in the status line |
| `crates/kf-qemu/src/chan.rs` | `fast_register` after every birth (Passthrough and Translated), `deregister` before every free; `DOORBELL-LEDGER` carries both transports |
| `crates/kf-qemu/src/{ffi,raw}_unsafe.rs`, `qemu/hw/misc/kf3/kf3.[ch]` | ABI 9: property `doorbell-ioeventfd` (default off), `doorbell-ioeventfd-max` (default 256); `KVM_IOEVENTFD` issued directly by the C device; doorbell sites reported by its memory listener |
| `crates/kf-linux-raw/src/kvm_unsafe.rs` | `KvmVm::ioeventfd` and `register_coalesced_mmio` (the GPU-free tests' kernel doors) |
| `crates/kf-chan/tests/dbfast_{kvm,exhaust,bench}.rs` | real-KVM guests: match/fallback, churn, exhaustion, measurements (§6, §7) |

## 3. The datamatch value — per die group, derived

The guest RM computes its channels' work-submit tokens itself: `kchannelCtrlCmdGpfifoGetWorkSubmitToken_IMPL`
(`ogkm-580: kernel_channel.c:3283-3349`) asks the host by RPC only when `IS_VIRTUAL` (a vGPU guest,
`:3297-3302`) — kayfabe's guest is not one — and otherwise calls `kfifoGenerateWorkSubmitToken_HAL`.
The HAL a die group binds is a code fact (`generated/g_kernel_fifo_nvoc.c:636-656`), and each body
writes through one header family:

| die group | HAL | fields | value for `(runlist, chid)` |
|---|---|---|---|
| TU10x | `_TU102` (`kernel_fifo_tu102.c:116-117`) | `NV_CTRL_VF_DOORBELL_RUNLIST_ID` 22:16, `_VECTOR` 11:0 | `runlist<<16 \| chid` |
| GA100, GA10x, AD10x, GH100 | `_GA100` (`kernel_fifo_ga100.c:226-227`) | same | `runlist<<16 \| chid` |
| GB10x | `_GB100` (`kernel_fifo_gb100.c:114-115`) | `NV_VIRTUAL_FUNCTION_DOORBELL_RUNLIST_ID` 22:16, `_VECTOR` 11:0 | `runlist<<16 \| chid` |
| GB20x | `_GB202` (`kernel_fifo_gb202.c:73-76`) | as GB10x + `_RUNLIST_DOORBELL` 30:30 `= _ENABLE (1)` | `1<<30 \| runlist<<16 \| chid` |

`GuestTokenFormat::for_die_group` holds only the `(die group → HAL, header prefix, sets
RUNLIST_DOORBELL)` row by hand; every bit position and the `_ENABLE` value resolve from the generated
hwref table (`kf_chip::hwref`), and an unresolvable field leaves every doorbell trapped (named at
realize). A registration is refused unless the device's own index finds the channel's slot from the
value (`TokenIndex::of_doorbell(value) == of_channel(runlist, chid)`), so a wrong prediction can cost
only speed — the guest's real value then matches nothing and traps — never correctness.
⊘ A non-canonical store (undecoded bits 15:12 or 31:23 set) masks into the same slot on the trap
path but matches no ioeventfd: it traps, and the trap serves it (tested).

## 4. The protocol

**Threads.** The channel **act** thread registers at birth and deregisters at free (it may sleep: a
KVM deassign waits for an SRCU grace period); the **main loop** (the C device's memory listener)
reports doorbell sites; the **register drainer** services eventfds. No vCPU touches any of it, and
the drainer never makes a KVM ioctl, never waits for another thread, and has no timer.

**Registration** (act thread, after the token word and the twin exist): create an eventfd → watch
it in the drainer's epoll set → queue `Add` for the drainer → place a `KVM_IOEVENTFD` (4 bytes,
DATAMATCH) at every current site, under the budget. Until a placement lands the trap serves the
token; after it, the eventfd does. A count KVM makes before the drainer has applied `Add` is still
reported (level-triggered) and the drainer applies queued commands before it calls a tag stale.

**Delivery** (drainer): read — and so reset — the counter, **then** deliver once through the trap's
arm. A store counted after the read leaves the fd ready for the next poll (drain-before-act). The
drainer polls doorbells (non-blocking, one `epoll_wait(0)`, skipped with no syscall when nothing is
registered) **at the top of its loop and before every privileged register write it applies**, and
wakes on them in its blocking wait with its own wake fd. ⇒ A doorbell a vCPU stored before a later
privileged write is delivered before that write is applied — the order the trap gave them.

**Removal** (act thread, before the twin is freed): remove every KVM placement (after the deassign
returns the kernel will never signal that registration again) → unwatch → queue `Remove` and **wait
for the drainer's acknowledgement**. The drainer applies it with a **final drain**, delivers any last
count through the token word, and acknowledges with the registration's ledger. Only then does the
act free the twin, and only a later act can re-birth the token.

**Why no doorbell is lost.** A matched store lands in exactly one place: the eventfd if a placement
existed at that instant, otherwise an exit to the trap. An eventfd count is delivered by a normal
poll or by the final drain, and the final drain happens after the last possible signal. Proven by
the churn test (§6): trapped + eventfd-delivered = guest stores, exactly, per token.

**Why a stale eventfd never rings the wrong twin (generation safety).** Every registration has its
own eventfd; a re-born token gets a new one, and counts never move between eventfds. Delivery reads
the token word at that instant. For a Passthrough free the token word is retired on the drainer at
the statement (before the removal act), so anything the old eventfd still delivers is absorbed; for a
Translated free the removal happens before `retire`, so its last count reaches the still-live token
(whose pump is already stopped) and nothing later. The removal is acknowledged before the twin is
freed and before any later act can re-birth the token. Proven by the churn test
(`crates/kf-chan/tests/dbfast_kvm.rs`, 2026-09-30): every eventfd ring names its own registration's
twin, and none names a freed one.

**Ledger.** `DOORBELL-LEDGER` keeps its per-token rule and its field names: for a Passthrough token
`rung`/`forwarded` now count both transports (`trap=` + `fast=`), plus `fast_wakes` (deliveries;
`fast/fast_wakes` is the incidental coalescing), `fast_absorbed`, `fast_failed`, `fast_sites`,
`fast_refused`. Translated lines gain the same `fast*` fields. The status line carries
`dbfast[…]` (registrations, placements, refusals by cause, polls, `wake_to_deliver` and `ack_wait`
histograms).

## 5. Limits: the budget, the kernel, descriptors

- ⊘ **KVM does not cap ioeventfds at 1 000.** `NR_IOBUS_DEVS` (1000) excludes them:
  `kvm_io_bus_register_dev` refuses only when `dev_count - ioeventfd_count > NR_IOBUS_DEVS - 1`,
  commented *"exclude ioeventfd which is limited by maximum fd"* (`linux: virt/kvm/kvm_main.c`,
  present since 2016). A registration is refused with `ENOSPC` only when the MMIO bus holds 1 000
  **non**-ioeventfd devices (coalesced-MMIO zones, in-kernel devices) — and the exhaustion test does
  exactly that to provoke it. What binds ioeventfds on a modern kernel is **the descriptor limit**
  (one eventfd per token) and memory.
- ★ **The real reason for a budget is dispatch cost.** The kernel finds a store's device by binary
  search on `(addr, len)` and then walks every device registered at that same address until one
  accepts the value (`__kvm_io_bus_write`), so every matched store costs O(its position) and every
  unmatched store O(all registered). Measured in §7 A.
- **The budget** (`doorbell-ioeventfd-max`, default 256 placements = tokens × sites) bounds both.
  Over it, the token stays trapped (`refused(budget=…)`).
- **Every refusal leaves the token on the trapped path, named and counted**: `kf3: DBFAST tok=…
  value=… site=… REFUSED by the budget | by KVM (errno N[: the MMIO bus is full])` (first 32 by name),
  `dbfast[refused(budget= kvm= enospc= eexist= fd=)]` (all).
- **Why the C device issues `KVM_IOEVENTFD` itself.** QEMU's `memory_region_add_eventfd` would follow
  BAR moves for free, but `kvm_mem_ioeventfd_add` calls `abort()` on any kernel refusal
  (`accel/kvm/kvm-all.c`) — a refused registration would kill the VM instead of leaving the token
  trapped. So `kf3_ioeventfd` calls `kvm_vm_ioctl(KVM_IOEVENTFD)` directly and returns the errno, and
  the listener reports doorbell **sites** (BAR0's usermode piece; every Hopper+ BAR1 view alias of the
  usermode page) so registrations follow the guest's BAR programming (`region_del` then
  `region_add`). ⚠ A PCI config write that moves BAR0 runs the listener on that vCPU thread, with the
  BQL, and each live registration costs a deassign/assign there — QEMU's own ioeventfd listener does
  the same on the same thread; it happens at BAR programming, which precedes any channel.

## 6. Tests (GPU-free)

| test | what it proves |
|---|---|
| `kf-linux-raw` `datamatch_ioeventfd_on_a_read_only_slot_…` | the kernel mechanism on a read-only memslot: routed tokens signal and never exit; everything else exits in order with its exact value; deassign/reassign; `EEXIST`/`ENOENT`; the backing never changes |
| `kf-trap` `the_guest_token_is_runlist_or_chid_and_gb20x_sets_bit_30` | the datamatch value per die group, and that the device's index finds the channel's slot from it |
| `kf-chan` `dbfast::tests::*` (unit, fake KVM) | off until enabled; sites × tokens bookkeeping; budget and kernel refusals trapped and counted; a count pending at removal is delivered before the acknowledgement and never after; a re-born token never inherits the old registration's counts; a registration replaced under a live token (a free that never came) discards its old count instead of ringing the new generation |
| `dbfast_kvm::registered_tokens_skip_the_exit_and_everything_else_traps_in_order` | real guest, Passthrough + Translated tokens: exits are exactly `[unknown, non-canonical]`; the drainer rings the Passthrough twin once for two stores (incidental coalescing) and the Translated token is RUNG and served by a worker; after removal the token traps again |
| `dbfast_kvm::churning_births_and_frees_never_lose_a_store_or_ring_the_wrong_twin` | a vCPU stores two tokens (one Passthrough, one Translated) 20 000 times each while another thread frees and re-births both ~150 times in the device's order; **trap + eventfd = 20 000 per token, exactly**; no eventfd ring of a freed twin; every eventfd ring names its own generation's twin |
| `dbfast_exhaust::every_token_refused_by_the_kernel_the_fd_limit_or_the_budget_still_works_through_the_trap` | three arms against a real guest: the MMIO bus filled with 1 000 coalesced zones → the kernel's `ENOSPC`; the process out of descriptors → `EMFILE`; the budget. In each, exactly the refused tokens exit, in order, and the trap rings their twins; the placed ones are delivered by the drainer |

Local result (development workspace, Linux 7.0.0-34-generic, itself virtualized): all pass; the churn
test split ~46 % trapped / 54 % eventfd over 136–157 rounds with 0 late fast rings, 0 acknowledgement
timeouts, 0 failed deassigns (three runs).

## 7. Measurements — ALL NESTED

Box: vast 53510558 — RTX 3060 (GA106), AMD EPYC 7K62 host (machine 30524, the same CPU model as the
historical `vh` LLM box), 11 vCPUs, **itself a KVM guest**, Linux 6.8.0-59, host driver 580.159.04.
Evidence: `traces/v3_ioeventfd/`. ⊘ Not one number here is non-nested.

### 7.1 Correctness, ON vs OFF — identical verdicts (2026-09-30)

| check | fast path OFF (the default) | fast path ON |
|---|---|---|
| crate tests / v3 gates / kf3 build / bare metal | 1700/0, 9/9, KF3_RC=0, 30/30 (`merge_check` at `43293417`) | — (the gates run no QEMU) |
| thin guest suite, budget 180 s | **30/30** (merge bar) | **30/30** (`dbfast_lane.sh`, same revision); arm times equal within 1 s |
| CUDA ladder, fat guest (cup2, cup3, cup8, cup8bench) | **4/4** | **4/4**; per-token `forwarded>0`, `emulated=0` for every guest token |
| where passthrough doorbells went (thin suite, all 30 arms) | trapped | **823 of 823 by eventfd, 0 trapped** (84 ledger rows) — the datamatch prediction is exact on GA106 / 580 |
| Translated (CeUtils, UVM kernel channels) | trapped | by eventfd, handed by the drainer (e.g. `--concurrency`: 830 hand-offs). `cup8bench` boot: CeUtils `0x801` 52 doorbells in 45 wakes; UVM-owned (`PRIVILEGE=KERNEL+UVM_OWNED`) `0x803` 201 in 160, `0x804`/`0x1005`/`0x1006` 1 each — every one `fast_forwarded`, `emulated=0` |

### 7.2 The vCPU cost of one doorbell store (guest-timed, `dbfast_exitbench.c`)

200 000 stores × 3 repetitions per row, timed in batches of 100 inside the guest; p50 per store.
`probe` is the measurement registration (a slot no channel holds, so the device then does nothing
either way) — the transport alone:

| store | OFF boot | ON boot |
|---|---|---|
| `probe` 0x007f07ff to the doorbell | trapped: 20.4 / 20.4 / 17.4 µs | **ioeventfd: 15.7 / 15.7 / 15.8 µs** |
| `unknown` 0x007f07fe to the doorbell (never registered) | trapped: 20.4 / 21.0 / 20.5 µs | trapped: 17.5 / 17.4 / 20.4 µs |
| dummy page, lockless no-op MMIO (floor of an exit to QEMU) | 18.9 µs | 18.9 µs |
| dummy page, BQL no-op MMIO | 19.4 µs | 19.2 µs |
| dummy page, plain ioeventfd with NO memslot | 11.9 µs | 11.7 µs |

⇒ On this nested box a matched doorbell costs the vCPU **~15.7 µs instead of ~17.4–21 µs: 2–5 µs
(10–23 %) less**. ★ The exit itself dominates, not QEMU: the in-kernel path still takes the nested VM
exit and emulates the store instruction (the doorbell page is a read-only memslot, so the store is an
EPT violation → emulation); the userspace round trip it removes is only the last few µs. The
no-memslot page shows the floor a read-only memslot cannot reach (11.8 µs) — but the usermode page
must stay a memslot, because its timer registers are read with no exit.
GPU-free on the same box (`dbfast_bench` A, a minimal harness exit): trapped 17.8 µs vs ioeventfd
13.2 µs; each OTHER token registered at the same address adds ~36 ns to every store (511 others:
+18 µs) — the kernel's linear same-address scan, and the reason the budget defaults to 256.

### 7.3 Host notification latency — the drainer's side

| where | p50 | p99 |
|---|---|---|
| in-device `wake_to_deliver` (epoll return → delivered), thin suite arms | 8–20 µs | 26–49 µs |
| in-device, CUDA ladder boots | 5–7 µs | — |
| in-device, the exit probe (a store every ~16 µs: the drainer never sleeps) | 1.5 µs | 2.8 µs |
| GPU-free signal → delivery on the box, **idle drainer** (includes its wake-up) | **45.7 µs** | 92.2 µs |
| … with 2 000 register writes/s at 20 µs each | 40.0 µs | 67.3 µs |
| … with 10 000/s at 20 µs (the drainer mostly awake) | 12.8 µs | 23.5 µs |
| … with 10 000/s at 60 µs (a doorbell waits behind an apply) | 18.6 µs | 62.5 µs |

⇒ ★ **The fast path is only as fast as the drainer's wake-up, and on this nested box an idle drainer
wakes in ~46 µs** (a cross-CPU wake-up in a nested guest). The trapped path rings the twin inside the
exit, with no wake-up at all. So the vCPU returns sooner, and the GPU hears about the work later.
Concurrent register traffic *helps* the latency (the drainer is awake) until an apply is long enough
to make a doorbell wait behind it (p99 62.5 µs at 60 µs applies): the drainer polls doorbells between
applies, never inside one.

### 7.4 What an application sees

`cup8bench` in the fat guest (the CUDA ladder's per-launch benchmark), same boot shape, one run each:

| N=16 (launch-bound) | OFF | ON |
|---|---|---|
| single launch + sync, median | 37 µs (submit 30 + sync 5) | **44 µs** (submit 26 + sync 19) |
| batched launches, per launch | 30 µs | **23 µs** |
| N=2048 (compute-bound) | 22.3 ms | 22.3 ms |

⇒ Exactly the trade the owner predicted (2026-09-28): asynchronous dispatch **helps a deep queue**
(batched −23 %: the vCPU returns sooner and keeps submitting) and **hurts an idle, synchronous launch**
(+19 %: the sync waits ~14 µs longer for the drainer to wake and ring).

LLM decode (the `llm_parity` lane, ON vs OFF and the spin experiment): §7.5.

### 7.x The protocol for a NON-nested host (not yet reachable, 2026-09-30)

A physical host was not reachable from this workspace on 2026-09-28/29 (`V3_DOORBELL_BASELINE.md`,
"Non-nested baseline protocol" — its steps 1–5 still govern: shared-host exclusions, recording the
exact configuration, stock host CUDA control first, paired repeated runs, counters bound to the
launched QEMU). On top of them, the fast path adds exactly this, on ONE host, strictly serial:

1. Provision as for any box (`scripts/bench/box/provision_full.sh`) and run the merge bar at the
   candidate revision (`merge_check.sh`: tests, gates 9/9, kf3 build, bare-metal 30/30, thin 30/30
   with the fast path OFF — the property's default). Record `nproc`, CPU model, `uname -r`,
   `cat /sys/module/kvm*/parameters/*`, and that `/proc/cpuinfo` has no `hypervisor` flag.
2. `bash scripts/bench/dbfast_lane.sh <tag>` from that checkout: thin suite ON (30/30 expected),
   guest-timed doorbell stores ON/OFF (`DBL_EXIT` rows: `probe` = the transport alone, `unknown` =
   the trapped reference in the same boot, `dummy_*` = the exit floors), the CUDA ladder OFF/ON
   (`CL_ROW` grades + `cup8bench` per-launch `GUEST_BSUM`), and the GPU-free rows (A: vCPU cost per
   store vs same-address registrations; B: signal→drainer delivery under register traffic).
3. LLM decode, paired and repeated: `llm_parity_box.sh <tag> <kf3 bin> gprov,hprov,host` once, then
   alternate `KF3_DEV_EXTRA=doorbell-ioeventfd=on llm_parity_box.sh <tag>_onN <bin> guest_pm` and the
   same without `KF3_DEV_EXTRA` (`_offN`), N = 1..3. Read `LLM_RUN … decode_tok_s=` (warm, 512 and 2048; `llm_parity_summary.py` tabulates),
   `LLM_KVM_EXITS`, `LLM_DOORBELLS`, `LLM_THREAD_CPU` (vcpu/drainer/workers ms) per process, and the
   status line's `dbfast[…]` (`wake_to_deliver` p50/p99, coalescing = `doorbells/wakes`).
4. Optionally the spin experiment (`KF3_DBFAST_SPIN_US=50` and `=200` with the fast path ON): tok/s
   against drainer CPU; it becomes a default only if the gain is worth a core.
5. Report ratios per box (guest/host on the same box), never across boxes, and label every row
   **non-nested** only when step 1's `hypervisor` check says so.

## 8. Side question: could the guest's token simply EQUAL the host twin's token?

*(Asked 2026-09-30: then the real host doorbell page could be mapped straight into the guest with no
trap and no helper, as nvkvm-pv Mode 1 does — there guest processes hold real host channels.)*

**Verdict: not possible for kayfabe's channel mix; possible only in a narrow configuration that
kayfabe never has.** Sourced from ogkm-580.159.04:

1. **The guest computes the token from its own chid and runlist** (`kernel_channel.c:3283-3349` →
   `kfifoGenerateWorkSubmitTokenHal_*`, §3). Only a vGPU guest (`IS_VIRTUAL`) asks its host by RPC
   (`:3297-3320`); presenting the guest as a vGPU would change what the whole guest driver believes
   it is running on, far beyond a doorbell.
2. **(a) Host twin allocated at the guest's chid — mechanically available, not reliably usable.** The
   host RM honours a caller-chosen chid: `NVOS04_FLAGS_CHANNEL_USERD_INDEX_PAGE_FIXED` /
   `_USERD_INDEX_FIXED` are read in `kchannelAllocHwID_GM107` (`kernel_channel_gm107.c:456-475`) with
   no privilege check there, and `kfifoChidMgrAllocChid` turns them into a
   `NVOS32_ALLOC_FLAGS_FIXED_ADDRESS_ALLOCATE` at `page × granularity + index` in the host's chid heap
   (`kernel_fifo.c:780-787`). But that heap is **the host's, shared by every host process and every
   VM**: the request fails if anyone holds that chid, the lowest `KFIFO_NUM_GSP_RESERVED_CHANNELS` (1) is
   reserved for GSP (`:58`, `:800-809`), and `_kfifoUserdOwnerComparator` (`:825`) binds each whole USERD page (8 chids) to one
   isolation domain — so a guest chid next to another tenant's channel fails too. kayfabe's own host
   channels (its Translated rings, the walker's CUDA context) draw from the same heap. The runlist IDs
   must match as well (the guest's comes from the FIFO table kayfabe serves).
3. **(b) Steering the guest's chid — not possible per channel.** The guest CPU-RM allocates chids from
   its own heap and only informs GSP of the result (`kernel_channel.c:2793-2802` sets
   `USERD_INDEX_PAGE_FIXED` in the RPC it sends us). kayfabe can shape the declared channel count,
   not which chid a given allocation takes.
4. ⊘ **Decisive: the doorbell register is shared with the TRANSLATED kernel channels** (the CeUtils
   scrubber, UVM's copy channels). Their stores must reach kayfabe, which rewrites their physically
   addressed work onto its own host ring (whose host token is unrelated to the guest's). A doorbell
   page mapped straight through would send those stores to hardware as hints for whatever host channel
   carries that value, and the kernel channels would never run. `THE_CONSTRAINTS.md` §47 states the
   same boundary: the page can go untrapped only when every channel rung through it is Passthrough —
   never true here, since the guest RM always has a CeUtils channel.
5. Were the page passed through, guest userspace could ring any host token — the posture the owner
   accepted for the guest-root helper (2026-09-26); equality would not change that.

⇒ Equal tokens buy nothing while the page must still be trapped (or matched per token, as this
fast path does); the fast path and the optional guest helper remain the routes.

## 9. Known limits and follow-ups

- **The trap path's own load→ring window** (pre-existing, unchanged): a vCPU that loaded a
  Passthrough token word just before a free can ring the old host token after the twin is released
  (the vCPU is not excluded from the free). A ring is only a hint to re-read that channel's `GP_PUT`,
  so it is harmless, but it is not generation-safe; the fast path is. The churn test reports it
  (`trap_late_rings`; 0 in every local run).
- The drainer wakes per doorbell burst; a bounded spin-then-park (§48.2) could cut wake latency when
  doorbells are dense, at a CPU cost — to be measured before it is proposed, never as a timer.
