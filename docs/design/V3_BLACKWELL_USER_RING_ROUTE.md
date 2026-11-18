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
   - Control: the same tree on an Ada box (creo-04 PASS).

### 2.4 Stage 1 as built (GPU-free, not wired)

`crates/kf-chan/src/user_semaphore.rs` (2026-11-18). The route stays declined on Blackwell by default. The
family gate (`userrelay::ring_relay_serves`) flips only in the PR that carries the Blackwell box evidence.

- `ProgressSource::for_family`: `EngineUserd` wherever `engine_writes_userd_gp_get`, `SemaphoreMarker` on
  Blackwell.
- `marker_cell`:
  - On `EngineUserd` it returns `user_storage::host_marker_words`, the words `write_host_marker` writes. A test
    pins them byte for byte for every non-Blackwell family. `write_host_marker` now calls that function; its
    bytes are unchanged.
  - On `SemaphoreMarker` it returns `[SEM_ADDR_LO x5 (incrementing): lo, hi, value, 0, SEM_EXECUTE 1]`
    then `[NON_STALL_INTERRUPT 0]`, 32 bytes. This is UVM's own sequence on ca6f:
    `uvm_hal_hopper_host_semaphore_release` (`ogkm-595 kernel-open/nvidia-uvm/uvm_hopper_host.c:31-52`; the
    push is at :45), reached by `BLACKWELL_CHANNEL_GPFIFO_B` through `uvm_hal.c:198-232`. The interrupt is
    `uvm_hal_turing_host_interrupt`, `uvm_turing_host.c:79-81`.
  - Difference from UVM (design choice, unmeasured): UVM precedes the release with `uvm_hal_wfi_membar`. The
    relay needs only "processed", not memory ordering of the guest's work, so the cell has none.
- `SemaphoreProgress`: the credit ledger. Tests cover:
  - credit from processed markers only;
  - publication bounded by capacity - 1;
  - the 16-bit count extended across its wrap;
  - fail-closed on untagged, not-issued and regressing words, sticky.
- `tests/user_turn_unit.rs` `with_the_semaphore_source_the_first_gate_opens_on_blackwell`: the twin of
  `without_engine_written_userd_words_the_first_gate_never_opens`, through the real relay turn.

### 2.5 Hostile-guest analysis of the semaphore word

**Where it lives (stage 2 design).** One host-owned 4 KiB vidmem page per Windows user-work private ring, in
the ring's private storage allocation. It is mapped GPU-WRITABLE at a kayfabe-chosen VA in the twin's restricted
address space, which is the guest's mirrored space: the twin's GPU work, the guest's included, runs there. The
ring, descriptors and marker cells stay read-only mappings, as today. So the word IS guest-addressable: any
forwarded pushbuffer of that channel can release a semaphore to its VA.

**What a forging guest can achieve** (every case bounded by `SemaphoreProgress`):

| forged write | effect | bound |
|---|---|---|
| untagged value, or `0` after progress | the channel fails closed (sticky). The relay stops publishing, and the existing dead path RCs the guest's own channel | no host state touched |
| a tagged value of a marker not issued, or older than the last seen | fails closed, as above | same |
| the NEXT issued value, early (before the engine processed the marker) | the relay believes entries were consumed early. It may reuse its own private slots and open its own SYNC_WAIT gates early. The guest's own channel then runs its own work out of order, or re-reads its own descriptors | self-harm to that channel only. Publication never exceeds capacity - 1 (`free()`), slots are this ring's own, staging stays in this ring's fixed cells (`CELL_BYTES`), and no host verb or mapping is taken from the word |
| any value, to wrap the 16-bit count | refused: at most `entries` <= 0x8000 markers are outstanding, so a wrapped value has delta > outstanding | fails closed |

Not reachable from the word (by construction of stage 1 and the existing storage):
- writes into another channel's memory, or into host memory outside this page;
- unbounded publication;
- a credit beyond what was published;
- any host RM call.

The word is readable by the guest's GPU work too. It only holds this channel's own marker counts.

**The rule this needs.** OWNER_RULINGS §AI, owner clarification 2026-10-12 (verbatim): *"no host owned maps
should remain for linux guests, but can be lifted for windows if its the only thing fixing the migration linux
doesn't use". The owner also authorizes necessary Windows-specific rule changes, with an afterwards report of
what changed and why.*

The same section says the read-only target stands "because the native reference mechanism works without a
GPU-writable private fence". That premise fails on Blackwell, where no engine writes `REFERENCE`. The "only
thing" condition holds while O2 (authored native relocation) is unproven on GB20x. If O2 is proven there
first, the lift is not needed. OWNER_RULINGS.md is not edited by this change.

### 2.6 REPORT for the owner (Windows-only GPU-writable private word), to be shown before stage 2 merges

- **What changes:** a Windows user-work private ring on a family whose engine writes no `USERD` back (today:
  Blackwell) gets one host-owned 4 KiB vidmem page mapped GPU-writable in that twin's restricted address space.
  Linux guests: unchanged. No host-owned writable map is ever made for them, and `WindowsUserRingPermit` stays
  Windows-only. Ada and every other family: unchanged, byte for byte.
- **What is written:** one 32-bit word, the marker value (`0x4B46_0000 | count`). It is written by the channel's
  host engine executing kayfabe's own marker cell (read-only, authored), or by the guest's own forwarded methods
  if it forges.
- **Who reads it:** the kayfabe relay worker, through its CPU view, never on a vCPU. The guest's GPU work could
  read it too; it holds only this channel's own counts.
- **Bounds:**
  - one page per ring, freed with the twin;
  - credit at most capacity - 1, from markers that were issued only;
  - monotonic;
  - 16-bit extension safe because at most 0x8000 markers are outstanding;
  - no host action derived from it.
- **Failure mode:** any value the relay did not issue (untagged, not issued, regressed) fails that channel
  closed: the relay stops, and an RC goes to that guest channel. Other channels and the host are untouched.
  A forged early-but-valid value can only reorder that channel's own work.
- **Why:** SPEC creo-04 on GB203 took Xid 32 because Blackwell Windows user-work twins cannot use the private
  ring (§1). The private ring is the route that serves Windows' ring-growth trigger, and on Blackwell its only
  progress proof needs a GPU-written word.

### 2.7 Probe: is `SET_REFERENCE` the METHOD Xid on GB20x? (INFERRED today)

**Result, MEASURED 2026-11-18 05:30-05:32Z** (SPEC box, RTX 5070 Ti GB203, host 595.91.07, bare metal, no VM; probe
`d360cd651`, binary sha256 `f0bdeb65...`; USER channel class `0xca6f`; scrubbed outputs kept on the box under
`/workspace/semprobe_out`):

| variant | attempts | host Xid | word |
|---|---|---|---|
| A `SET_REFERENCE` then witness | 1 (the run stopped after it, fail-closed) | `Xid 32 ... channel 0x00000002 intr 00200000` | witness `0x57170041` LANDED |
| B stage-1 semaphore cell | 5 (2 as canary, then 3 children) | none | `0x4b460001` every time |
| C `NON_STALL_INTERRUPT` then witness | 3 children | none | witness `0x57170043` every time |

**Two intr bits, two causes:**
- **`intr 00200000` (bit 21, METHOD in the GA100 meaning)** is what `SET_REFERENCE` (`0x50`) raises on GB203. So on GB203
  the private ring's own `SET_REFERENCE` marker is refused, not only its progress words. INFERRED (B1n ran on a GB205,
  not measured here): GB205 round B1n's "Xid 32 METHOD at the relay's first publication" is the same marker.
  - The PBDMA raised the Xid AND executed the later witness release.
  - The probe's first verdict expected the witness to be missing, so it stopped the run. The verdict is now
    "exactly one Xid 32 with intr bit 21, whatever the witness".
- **`intr 00800000` (bit 23, DEVICE)** is the creo-04 / enscape-01 event (§1): a software-subchannel method kicked
  back to RM, the unserved ring-growth trigger. SET_REFERENCE is NOT that event.
- **The stage-1 marker** (`SEM_*` release plus `NON_STALL_INTERRUPT`) processes cleanly on a USER ca6f channel, 5 of 5.
  `NON_STALL_INTERRUPT` alone is clean too, 3 of 3.
- The host stayed healthy after the deliberate A Xid: the later B/C runs raised no Xid, and `nvidia-smi -q` changed
  only in throughput and temperature.

**Built (2026-11-18):** `kf-harness` binary `kf-semmarker-probe`, pure part `kf_harness::semprobe` (unit-tested).
It is a v3 tool. Nothing in the frozen `kayfabe-rm-ladder` is touched, and there is no unsafe code.

How it differs from the sketch below:
- The channel is a USER **copy-engine** channel. Host methods (`SET_REFERENCE`, `SEM_*`,
  `NON_STALL_INTERRUPT`) are PBDMA methods whatever the engine, and B1n's METHOD Xids hit both its GR and CE
  rings.
- Instead of a non-stall event count, A and C end with a WITNESS release into the owned word. It lands only if
  the PBDMA processed everything before it.

Each attempt is one child process with:
- a fresh VA space and USER channel;
- a read-only command mapping;
- one owned GPU-writable 64 KiB buffer;
- USERD with no GPU mapping.

Safety:
- The parent kills a child after 20 s (10 s to reap) and reads the dmesg delta per attempt.
- It records the exact words and methods.
- It STOPS at the first outcome that is neither `Clean` nor (A only) `MethodRefused`: exactly one Xid 32 with
  the witness missing.

Verdicts:
- A: `MethodRefused` supports the inference; `Clean` falsifies it.
- B: must be `Clean` with the word reading `0x4B460001`.
- C: must be `Clean`.

On a Blackwell box, bare metal first (the raw client, `kayfabe-rm-ladder` / `kf-rawclient`). A USER `0xca6f`
channel on GR, three variants, each its own process with a fresh channel, N=3 each:

| variant | pushes | if 0x50 is invalid on GB20x |
|---|---|---|
| A | `SET_REFERENCE 0x4B460001` alone | Xid 32 with `intr 00200000` (METHOD, bit 21, GA100 meaning) |
| B | the stage-1 semaphore cell into an owned word | no Xid; the word reads `0x4B460001`; one non-stall event |
| C | `NON_STALL_INTERRUPT 0` alone | no Xid |

Capture:
- host dmesg (`Xid` lines with `intr`) in a journal window per process;
- the word's value;
- the non-stall event count;
- the binary's revision.

Falsifier: A gives no Xid (then B1n's METHOD has another cause, to be named from its saved trace).

### 2.8 Hardware runs to ask for

1. **Now (stage 1 is GPU-free):** nothing on a Windows workload. Stage 1 is not wired, and a property-on run
   needs stage 2.
2. **After the probe arms exist (small raw-client PR):** §2.7 on one Blackwell box (GB203 or GB205),
   bare metal, N=3 per variant. Pass for O1: A/B/C as in the table's right column, with B correct.
3. **After stage 2 (the route admitted behind a property, default declined):** one Blackwell box, serial:
   - `v3_gates.sh` 9/9;
   - `build_kf3.sh`;
   - the fast suite 30/30.
   Then, with the property on:
   - SPEC creo-04 (N=2) and GravityMark 1.89 Vulkan (N=1), each with `KF3_RPC_TRACE=1`;
   - capture: the kf3 log, host dmesg with timestamps, SMI before/after, and the rows.
   PASS means all of:
   - no host Xid;
   - no `USER_RING_ROUTE_DECLINED` line for `0xca6f`;
   - a `0x50800104` registration served by the relay (the migration path logged);
   - the rows PASS.
   Control: the same tree on an Ada box, creo-04 PASS. Repeat property-off once on the Blackwell box to show
   the failure is the route (H-F).
