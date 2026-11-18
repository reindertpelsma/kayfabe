# Blackwell Windows user-work route: the creo-04 Xid 32 and a Blackwell-capable private ring

**STATUS: DESIGN-ONLY, 2026-11-18.** Section 1 is a root-cause analysis from on-disk evidence: each link is
marked MEASURED or INFERRED. Section 2 is a design. Nothing in section 2 is implemented or measured on
hardware yet. The GPU-free reproduction is `crates/kf-qemu/src/chan.rs` `mod replay_creo04_gb203_xid32`
(fixture `crates/kf-qemu/tests/fixtures/creo04_gb203_xid32_replay.txt`); its invariant test is ignored as a
known defect, and a pin test proves the replay still reproduces.

Coordinator decision, 2026-11-18: closing this route gap is a work item, not a parked owner question. All
families are first-class (`AGENTS.md`), and the decline now has a measured Blackwell failure. This supersedes,
for Blackwell, the sentence "A Blackwell-capable private ring needs another progress proof, which is an owner
decision" in `V3_WINDOWS_USER_RING_RELAY.md` (2026-11-01 correction).

## 1. Root cause of the creo-04 failure (SPEC acceptance run, RTX 5070 Ti GB203, host 595.91.07, kf3 049fbf3c)

Symptom (MEASURED, 2026-11-18, kf3 `049fbf3c`): one host `Xid 32 ... channel 0x00000016 intr 00800000` at boot age about 26 min (02:36:23,
dmesg 8468.11). The guest's RM unloaded 7 s later (kf3 log `GSP fn 47 UNLOADING_GUEST_DRIVER` at
`t=1580.0s`, phase `Running -> Halted`). That is the Windows TDR recovery after the Xid, not a cause of it. The
driver re-initialised, and the row failed with `reason=GUEST`.

**Update, 2026-11-18 (second occurrence; reported by the acceptance lane, not yet re-checked here).** Same box, SPEC S2,
boot 3 (enscape-01, boot started 03:29Z): a host `NVRM: Xid 32 ... channel 0x1f intr 00800000` at 03:49:23Z, about 20
min in, with kf3-vamgr freeing mappings 10 s later. This time the guest SURVIVED: the licence probe answered at boot
age 47 min, QEMU was alive at 71 min, and the viewset continued. Xid 32 with this `intr` therefore recurs on this die
(2 in 3 boots so far) and is not always fatal.

Caveat: the kf3 log of boot 3 stopped being written at 03:38:55, ten minutes before that Xid, so it cannot cover the
Xid window.

Open checks, against the evidence copy the lane refreshes after it ends:
- whether 0x1f is again a `0xca6f` Windows user-work twin on the USERD relay (its birth line and the boot's
  `USER_RING_ROUTE_DECLINED` line);
- whether a `0x50800104` and an unmap/remap burst precede the Xid. The log stops early, so a registration after
  03:38:55 cannot be seen in it;
- the guest-side recovery: TDR, display-driver reset, or a per-process device-lost only. Hypothesis, unmeasured:
  survival means Windows recovered the one process's device without unloading the RM, whereas creo-04's boot logged
  `UNLOADING_GUEST_DRIVER`. Falsifier: a guest RM unload (fn 47) near 03:49 in boot 3.

The chain, link by link:

1. **Channel 0x16 is a Windows user-work Passthrough GR twin of class `0xca6f`.** MEASURED. Host token
   `0x40000016` = guest token `0x11`, engine 1 (GR), born 16 times (one per test process), last at log line
   16959 (`... WINDOWS-USER-WORK ... (USERD relay: doorbells go to a worker)`). Its alloc is class `0xca6f`
   (line 16956).
2. **On Blackwell such a channel is kept off the private ring.** MEASURED. Line 2799:
   `USER_RING_ROUTE_DECLINED family=Blackwell class=0xca6f: USER_RING_FAMILY_WRITES_NO_USERD`
   (`userrelay::ring_relay_serves`). On Ada the private ring is the default route
   (`KF3_WIN_USER_RING_RELAY` unset).
3. **Its twin's host `NV50_DEFERRED_API` holds no registration.** MEASURED (line 16962, `NOTHING registered: a
   0x200 on it fails closed`). By design, the guest's `NV5080_CTRL_CMD_DEFERRED_API*` controls stay
   guest-graph only (`chan.rs` `twin_defapi_act`).
4. **The guest registered one deferred API on that object, within the Xid's window.** MEASURED. Line 17354,
   `cmd=0x50800104` (`NV5080_CTRL_CMD_DEFERRED_API_INTERNAL`, the kernel RM's forward of `_V2`,
   `ogkm-595 ctrl5080.h:184`) on `0xc1d0009e:0xff1fe010`, the 5080 object of channel 0x16. It is the only
   `0x50800104` of the whole boot (70 others are `0x50800101`).
   - Time: the status line ticks every 2.007 s (measured over lines 3023 to 18968). Anchored on host dmesg
     (QEMU start about dmesg 6895.3 = 02:10:10), the registration falls at t of about 1572-1574 s and the Xid
     at t of about 1572.8 s, ±2 s.
   - Just before it, the same VA space unmaps 1024+1018 runs and remaps them (lines 17349-17356), the shape
     of a ring relocation.
5. **The bundle is `NV2080_CTRL_CMD_FIFO_UPDATE_CHANNEL_INFO` (a GPFIFO ring growth).** INFERRED. The log
   does not print the bundle. Support: only `_V2` carries `ChannelInfoUpdate`/`DisableChannels`
   (`ctrl5080.h:150-180`); GravityMark Vulkan's measured late registration on the same kind of twin was
   exactly this (old 32768 entries, new 65536; channel 0x16's ring here has 32768 entries).
6. **Its `0x200` trigger, executed natively from the guest's own ring, finds nothing registered and host RM
   fails the software method.** INFERRED for the GSP step. `_class5080DeferredApiV2` answers an unknown
   handle with `NV_ERR_INVALID_DATA` (`ogkm-595 deferred_api.c:119-135, 458-461`). The PBDMA service that
   turns that into an RC is closed firmware.
7. **`intr 00800000` is PBDMA `INTR_0_DEVICE` (bit 23): a method on a software subchannel kicked back to
   software.** INFERRED for GB20x. This is the GA100 `dev_pbdma.ref` meaning, already cited in `chan.rs`
   (`SwMethod`); ogkm-595 publishes no GB20x `INTR_0` layout (only `gb100/dev_esched_pbdma.h`, CHRAM).

Same signature on Ada, earlier (MEASURED): GravityMark Vulkan runs 1361/1510/1540 and nbody GL, Xid 32 `intr
00800000` on a Windows user-work Passthrough twin after a `FIFO_UPDATE_CHANNEL_INFO` registration
(`V3_FEATURE_GAPS.md`, `V3_WINDOWS_USER_FIFO_MIGRATION_DECISION_20261011.md`). The private ring (default since then) serves
that trigger in software on Ada (`c5d9693f4`: the trigger segment published with the trigger as DMA_NOPs,
the migration done by the relay). That is why the same viewset passed on an RTX 4090 on 2026-11-16. INFERRED
for that run: its kf3 log was not kept, only rows, dmesg and SMI at `kf3-bins/06086008`.

Hypotheses (falsifiers stated before the log was read):

| id | hypothesis | falsifier | result |
|---|---|---|---|
| H-G | route decline, trigger with nothing registered (above) | Xid with no deferred-API activity on the channel in its window | not falsified; the only `0x50800104` sits in the window |
| H-A | Blackwell PTE kind/aperture | a fetch-level interrupt (Xid 31 MMU fault, `PBENTRY`) | `DEVICE` is method-level; no Xid 31 (INFERRED ranking) |
| H-B | 580 guest method invalid on 595/GB20x | the same channel class passes 15 earlier lives | 15 earlier lives of 0x16 ran the same objects; subsumed by H-G |
| H-C | VA-manager race unmapped the pushbuffer | no unmap in the seconds before the Xid | an unmap/remap is in the window, but a fetch from an unmapped page faults (Xid 31), not `DEVICE` |
| H-D | doorbell/token path | relay refusals or stale tokens near the Xid | `refused=0`, relay normal |
| H-E | census differs from the 4090 pass | identical census | not decidable: no kf3 log of the 4090 pass |
| H-F | flaky, not kayfabe | reruns reproduce | pending (acceptance reruns). Under H-G the intermittency is expected: Windows grows the ring only under some loads |

Observation that would settle links 5-7: on a Blackwell box, `KF3_RELAY_PB_PEEK=after-fifo-update` (it logs
`RELAY-PEEK ARMED after FIFO_UPDATE_CHANNEL_INFO registration ... cmd=...` and the next segments of the
channel), with the same creo-04 viewset.

## 2. Design: a Blackwell-capable route for Windows user-work twins

### 2.1 What `USER_RING_FAMILY_WRITES_NO_USERD` means

The private ring relay reads two words that the engine writes back into its private USERD: `GP_GET` (fetch
credit: which ring slots it may reuse) and `REFERENCE` (the marker proof: every method before the marker was
processed, written by the marker cell `[SET_REFERENCE value, NON_STALL_INTERRUPT 0]`,
`kf-chan user_storage.rs` `write_host_marker`).

- Blackwell's control struct is `Ignored00[0x23]` then `GPPut` (`clca6f.h:27-31`, `clc96f.h`).
- The GB203 bare-metal client measured that `GP_GET` stays 0 for 22 s after the release
  (`kf_chip::Family::engine_writes_userd_gp_get`).
- The relay therefore never sees a marker; `kf-chan tests/user_turn_unit.rs`
  `without_engine_written_userd_words_the_first_gate_never_opens` pins that.

A second finding (INFERRED): the public Hopper and Blackwell channel headers define no `SET_REFERENCE` (method
`0x50`). `clc86f.h`, `clc96f.h` and `clca6f.h` list only `SET_OBJECT`, `MEM_OP_*`, `SEM_*` (and `WFI` on c86f).
UVM's host HAL never emits `SET_REFERENCE` on any class. So GB205 round B1n's "both relayed DWM rings took Xid
32 METHOD at the relay's first publication" may be the marker's `SET_REFERENCE` itself, not only the missing
progress words. Falsifier: B1n's PBDMA method register names `0x50`; or a ca6f channel executes
`SET_REFERENCE` without an Xid on bare metal.

What UVM does emit on `BLACKWELL_CHANNEL_GPFIFO_B` (read in public source, `ogkm-595 kernel-open/nvidia-uvm`): the HAL chain is ca6f, then
c96f, then Hopper c86f, then Ampere, then Turing (`uvm_hal.c:196-232`).

- Semaphore release: `SEM_ADDR_LO/HI, SEM_PAYLOAD_LO, SEM_EXECUTE(OPERATION_RELEASE, PAYLOAD_SIZE_32BIT,
  RELEASE_WFI_DIS)` (`uvm_hopper_host.c:40-55`).
- Interrupt: `NON_STALL_INTERRUPT 0` (inherited Turing `.interrupt`, `uvm_turing_host.c:79-81`).
- The GB203 bare-metal client already uses the semaphore as Blackwell's completion
  (`V3_FAMILY_PORT_BLACKWELL.md` §3, MEASURED 30/30).

### 2.2 Options

| | option | progress source | hostile-guest bounds | measurable on a 5070 Ti | cost |
|---|---|---|---|---|---|
| O1 | **private ring with a semaphore marker** | the marker cell becomes `[SEM_ADDR_LO, SEM_ADDR_HI, SEM_PAYLOAD_LO value, SEM_EXECUTE release 32-bit, NON_STALL_INTERRUPT 0]` into one private 4-byte word per ring; fetch credit is derived from it (a processed marker at slot k proves every entry before k was fetched) | needs a GPU-WRITABLE private word mapped in the twin's restricted VA space (§AI today: read-only command mappings; its 2026-10-12 owner clarification permits a Windows-only lift when it is the only fix, with a report). The guest's own forwarded methods can write that word, so a forged value must only ever mis-time the guest's own ring (as today's untagged-`REFERENCE` rule, `user_gate.rs`); never a host action and never a slot outside the private ring | yes: the semaphore path is measured on GB203 bare metal; a fast-suite arm can measure the marker on the twin | kf-chan marker and credit (per-family), kf-qemu storage mapping, one §AI report |
| O2 | **authored native relocation on today's USERD-relay route** | none needed: the guest's own ring keeps running natively; kayfabe registers, on the twin's host 5080, an AUTHORED `FIFO_UPDATE_CHANNEL_INFO` (validated new GPFIFO span inside the twin's refreshed VA space, USERD = the twin's own private USERD, handle checked free), so the guest's `0x200` finds a registration | guest-chosen handle and bundle: validated, never replayed (`V3_NATIVE_FIFO_RELOCATION_20261013.md` gates). Execution is the host's, at the twin's privilege | the capability is MEASURED only on Ada (RTX 4070 / 595.91.07, UID 65534 / CapEff 0, `1a666d783`, `242647766`); unmeasured on GB20x. The same-configuration faults and the cursor rule remain open | smaller code; larger unknowns |
| O3 | RM-reported GP_GET | a host control that reads a channel's GP_GET | n/a | not available: no public control reads it (grep of ogkm-595 `ctrl/`) | rejected |
| O4 | usermode region | per-channel state readable from the usermode/doorbell page | n/a | INFERRED none: the Blackwell usermode page is doorbell and time | rejected |
| M | mitigation: avoid ring growth | — | — | refusing the registration is falsified (GravityMark run 1540: device lost, Xid 32). No guest setting or initial ring size that avoids growth is documented in public sources or measured here. Changing the served latency-buffer answer (`0x0080170e`) would fabricate a host fact (derive, never capture) and is unmeasured | none known |

### 2.3 Recommendation

**O1.** It keeps one route for every family: the private ring that already serves the trigger on Ada, with
app passes on Ada (`c5d9693f4`). Its only new GPU behaviour is the semaphore release that UVM emits on ca6f and that the
GB203 bare-metal client measures (`V3_FAMILY_PORT_BLACKWELL.md` §3). O2 depends on a capability measured on one Ada die (`1a666d783`, 2026-10-12) with two open gates
(cursor rule, an unexplained same-configuration fault).

Run O2's discriminator only if O1's §AI lift is refused.

Stages:

1. **GPU-free** (kf-chan): a per-family progress source, `EngineUserd` for Turing through Hopper and
   `SemaphoreMarker` for Blackwell.
   - The marker cell encoder emits only the methods of 2.1 on Blackwell, and the Ada cell is unchanged.
   - The relay derives fetch credit from a processed marker when the family writes no `GP_GET`.
   - Red-first tests:
     - the existing `without_engine_written_userd_words_the_first_gate_never_opens` gets a twin: with the
       semaphore source, the gate opens;
     - a forged or regressing semaphore value never advances the credit past the last published marker;
     - the Ada marker cell is byte-identical to today's.
2. **GPU-free** (kf-qemu): the private semaphore word in the private storage. Host-owned vidmem, one 4 KiB
   page per ring, mapped GPU-writable only for a Windows user-work twin. The route admits Blackwell
   (`ring_relay_serves`) only when stage 1's source exists. The replay invariant test above turns green, and
   its pin is deleted.
3. **Hardware**, one Blackwell box, serial (`AGENTS.md` *Verification*): bare-metal first, then the fast suite
   (30/30), then the user-ring lane. Then SPEC creo-04 twice and GravityMark 1.89 Vulkan once, each with
   `KF3_RPC_TRACE=1`.
   - Falsifier: any host Xid on a user-work twin, or a `USER_RING_ROUTE_DECLINED` line for `0xca6f`.
   - Baseline for that falsifier on the SPEC 5070 Ti: 2 host Xid 32 (`intr 00800000`) in 3 SPEC viewset boots
     (2026-11-18, S2). "No host Xid 32 over full SPEC viewset boots" is measurable against it.
   - Control: the same tree on an Ada box (creo-04 PASS).
