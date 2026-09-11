# RESUME HERE — w417 (supersedes `RESUME_HERE_w414.md`)

**STATUS: LIVE, 2026-09-11.** Supersedes `RESUME_HERE_w414.md` in full — that doc's central
claim (*"there is NO general operand path"*) is **half right and the other half was the bug**.

## Where the LLM actually stands

The wall was ours, in two layers, and both are now fixed. Neither was a missing mechanism.

### Layer 1 — the pin pass never ran (`w415llm`, fixed at `05df1d6b`)

Moving publication off the vCPU onto the worker introduced
`ctx.vas_publish = VasPublishArm::Publish` in both worker lanes, to force publication
regardless of the environment arm. `Publish.measures_pin_rate()` is **false**, so the
override silently switched off `measure_guest_ram_pin_rate` — the only pass that pins
guest-RAM rows for anything but a channel's ring. `VAS_PUBLISH=drain` was inert on every
boot.

⊘ **The tell was an ABSENCE.** `⊘ NO DRAIN` prints *unconditionally* on that pass's
non-doorbelled path, so `grep -c` returning **0** means the pass never executed. Nothing
logged an error; `pins=0` read as a pin that failed rather than a pass that never ran.

### Layer 2 — the pass only sampled 256 rows (`w416llm`, fixed at `cfd6db36`)

`VAS_PINRATE_ROWS = 256` bounded a pass that used to run **on the vCPU inside its own MMIO
exit**, where a 13 000-row drain is a ~4 s trap. Off the vCPU that licence is gone, and the
invalidate is a synchronization point we are *allowed* to block. The cap now lifts whenever
the pass holds an off-vCPU token.

## Measured effect, w415 → w416 → w417

| signal | w415 | w416 | w417 |
|---|---|---|---|
| `NO DRAIN` (pass ran) | 0 | 1278 | yes |
| `pins=` | 0 | 16 897 | — |
| `guest_ram=` unbacked | 13 313 | 0 | — |
| largest pin | — | `asked=256 pinned=256` | `asked=12288 pinned=12288` |
| worst refresh | — | 233.01 ms | **54.52 ms** |

★ And the fault CHANGED KIND at layer 1, which is the real evidence of movement:

    w415  CE3_PBDMA0 HUBCLIENT_ESC @ 0x2_03000000     FAULT_PDE ACCESS_TYPE_VIRT_READ
    w416  CE2        HUBCLIENT_CE0 @ 0x7683_27600000  FAULT_PDE ACCESS_TYPE_VIRT_WRITE
    w417  CE2        HUBCLIENT_CE0 @ 0x7cac_33600000  FAULT_PDE ACCESS_TYPE_VIRT_WRITE

A PBDMA failing to **fetch** its pushbuffer became a copy engine's data client failing to
**write** its destination. The engine now runs.

## ⊘ THE OPEN QUESTION, and it is ONE question

w417 pinned `0x7cac33600000+0x533000` **whole** (`asked=1331 pinned=1331 refused=0`) and
`CE2` still faulted at that range's **base**. A successful pin and a fault on the same range
are only contradictory **if the pin happened first**.

⚠ Nothing in either log could say which came first: ours carried a DURATION (`in 80 ms`),
`dmesg -T` carries only absolute time. `w417` adds `at=<unix seconds>` to the pin line for
exactly this join. **Read that first.**

- If the pin is LATER than the Xid ⇒ we back it **too late**. The trigger is the problem, not
  the backing. Next: which transport declares this range, and does any of our three
  synchronization points fire before the engine runs? Note
  [[there_is_no_universal_publish_trigger]] — a guest ioctl can map with nothing observable.
- If the pin is EARLIER ⇒ we back it into the **wrong place**, or something un-backs it.

⊘ **One hypothesis is already RETIRED, by reading, not by a boot:** the pin does NOT land in
a different host VAS than the channel walks. `plan_pin_guest_ram` takes `vas.host_vas` for
`(gpu, pdb)`; the channel-birth path at `kayfabe-fwd/src/lib.rs:4014` takes the same field of
the same `Vas`.

## The client can finally SEE this class

`--ce-client-guest-ram [--guest-ram-decoys N]` (default 13 000) — a CE copy whose **source**
is `NV01_MEMORY_SYSTEM`, declared behind N further guest-RAM rows so the operand is the
freshest row a bounded pass reaches last. Prints `R34_OUTCOME=(P)/(F)`, and says
`R34 SHALLOW` with the depth it ACTUALLY reached when the allocator gives out early.

⚠ **Why this was missing and why it mattered:** `prove_ce_copy` allocates BOTH operands with
`alloc_device_local` — **vidmem**. No engine in any existing client rung read guest RAM at
all. That is how a green client (110 guest-RAM rows) coexisted with a dead LLM (13 313) for
the whole of w415. ★ **R34 IS GREEN ON BARE METAL** at depth 0 and depth 2000 (`[measured w417]`): 4096 bytes
moved byte-correct, semaphore released. So the *pass* half of
`BARE-METAL PASS + GUEST FAIL ⇒ KAYFABE BUG` now exists, and `w418_r34_boot.sh` supplies the
guest half in a boot that costs seconds rather than an hour.

⊘⊘ **Getting there cost two defects IN THE RUNG, and both looked like kayfabe defects.**

1. R34 allocated with `alloc_notifier_mem`, which sets `NVOS02_FLAGS_MAPPING_NO_MAP`, then
   called `map_cpu` on it. `Other(31)` = `NV_ERR_INVALID_ARGUMENT`. **The guest and bare metal
   refused identically**, which is the only reason it was not filed as a kayfabe bug — I had
   already written a commit titled *"R34 REPRODUCES THE LLM FAULT"*, and the control refuted
   it.
2. The "fix" — dropping `_NO_MAP` — produced `Other(0x8000_0016)` = **errno 22, EINVAL from
   the ioctl**, because RM then builds an mmap context on our `fd: -1`
   (`ogkm-580: escape.c:341-359`, `nv-usermap.c:44-46`). ⊘ `rm.rs:6604` has carried that
   citation since `w288nc1`. Both primitives I wrote already existed, correct, in the same
   file: `alloc_sysmem` and `map_cpu_on(MapNode::Ctl, …)` — a sysmem object maps through the
   **ctl** node, not the **gpu** node.

⚠ Transferable: **the wrong allocator returned `Ok`.** The refusal surfaced several calls
later, on a call that was correct, against a handle that was valid. Attributing the error to
a STEP is what made it readable — `Other(31)` names a status and never a call.

## Harness traps paid for in this window

- `build_qom_shim.sh` needs `KAYFABE_SHIM_FEATURES=host-isolates`, or QEMU refuses at realize
  with *"this archive was built without the `host-isolates` feature"*. Cost one boot.
- The box's git remote is **`gh`**, not `origin`. `git fetch origin` failed, the binary rebuilt
  cleanly **from the old source**, and every downstream signal looked healthy. Cost one boot.
  The runner now gates on `rev-parse HEAD` matching local.
- A non-login `ssh host '...'` does not source `~/.cargo/env`. The script died at 127
  correctly; the CALLER piped it and read **tail's** status. Cost one boot. `build_qom_shim.sh`
  now names this in its own preflight.

⇒ All three shared one shape: **the failure was upstream of the measurement and every signal
downstream of it looked healthy.** Gate on the ARTEFACT (binary mtime, `rev-parse`, a string
census), never on a step's reported status.


---

# w419 — TWO HYPOTHESES DEAD, ONE VARIABLE LEFT

`[measured w419, in the guest, rev 485a40cd]` R34 is **green in the guest**:

    depth 0     ★ src 0x120000000 dst 0x120001000  dst[0] 0xc0ffee34  dst[last] 0xc0fff233
    depth 2000  ★ src 0x1207d0000 dst 0x1207d1000  dst[0] 0xc0ffee34  dst[last] 0xc0fff233

A CE copy whose source and destination are both `NV01_MEMORY_SYSTEM` moves its bytes inside
the guest, with 2000 further guest-RAM rows declared ahead of the operand. Bare metal is green
at the same depths.

⊘ **Dead, both with evidence:**
- *"There is no general operand path; guest-RAM operands are unreachable."* They are reachable.
- *"Scale is the discriminator — 110 client rows vs the LLM's 13 313."* Green at 2000, which is
  18× the client and the same order as the LLM.

## ★ The one variable left: the ADDRESS

R34's operands sit at `0x1_2000_0000` — our operand space, RM-placed. The LLM faults at
`0x7cac_3360_0000`, a process-VA-shaped address ~137 TB up, which is what CUDA's unified
addressing hands out for a device pointer. Same aperture, same engine (`CE2 HUBCLIENT_CE0`),
same direction (`ACCESS_TYPE_VIRT_WRITE`), one difference.

⇒ `--guest-ram-at <hex>` dictates the operands' GPU VA so the two can be compared with one
variable moved. **Run the sweep first**: RM-placed (control), `0x120000000`, `0x7cac33600000`,
`0x768327600000`, `0x7f0000000000`, `0x400000000`, all at `decoys=0`, bare metal and guest.

## ⚠ Harness traps paid for in w418/w419 — all three were INVERSIONS

1. **`strings "$B" | grep -q X` returns 141 when X IS PRESENT.** `grep -q` exits on match,
   `strings` takes SIGPIPE, `pipefail` reports 141. Finding the string faster is what makes the
   check fail. Use `grep -a X "$B"`. ⊘ My first reading blamed a missing `strings`; it is at
   `/usr/bin/strings`.
2. **`timeout N sudo CMD` does not bound `CMD`.** The signal goes to `sudo`, which does not
   forward it. The wrapper dies, the work runs on as root, the ssh pipe never closes and the
   boot reads as WEDGED. `pgrep -a` prints `[rmladder]` in brackets, which reads as a kernel
   thread. Use `sudo timeout N CMD`. Eleven sites fixed, including `w392d_mean_hook.sh` — the
   mean-client hook the whole client is graded on.
3. **Three of the five inherited red CI gates were firing on PROSE** — a comment saying a file
   avoids `unsafe`, a doc line naming `RmEvent`. A permanently red gate is a gate nobody reads,
   so those were one defect, not three, and it had been protecting nothing. ⊘ The CLAIM-LEDGER
   pair is NOT of this kind: it is working correctly against a real documentation backlog.


---

# w420 — THE RUNG'S THIRD INSTRUMENT DEFECT, AND THE ARM THAT MATTERS

## ★ R34's blind spot was its own operand pair

R33 could not see guest RAM because **both** its operands are `alloc_device_local`. R34 could
not see an H2D write because **both** of its operands are `NV01_MEMORY_SYSTEM`.

⇒ **A rung's blind spot is whatever its operands have in COMMON**, and both rungs had a
uniform pair. The LLM's fault is `CE2 HUBCLIENT_CE0 … ACCESS_TYPE_VIRT_**WRITE**` — a copy
engine writing its destination — and `.to('cuda')` is an H2D upload: **source guest RAM,
destination device memory**. Neither rung had that pair until `--guest-ram-dst-vidmem`.

## ⊘ And the first H2D result was the rung again

`[measured w420, BARE METAL, decoys=0]`:

    dst aperture   address            result
    vidmem         RM-placed          (P)  src 0x120000000  dst 0x120010000   (+64 KiB)
    vidmem         0x7cac33600000     (F)  refused at map_dma_both(dst): NoMemory
    sysmem         0x7cac33600000     (P)  src 0x7cac33600000 dst 0x7cac33601000

*"Device memory cannot be mapped at a high VA"* would have been a striking finding. **RM's own
placement refutes it**: `+0x10000` for vidmem against `+0x1000` for sysmem. Device-local is
**64 KiB big-page** granular here, and the rung offset the destination a flat 4 KiB, so it was
asking RM to host a 64 KiB-page mapping at a 4 KiB-aligned VA. `NoMemory` is the right answer.

⚠ The sysmem arm passing **at the same address** is what makes the page-size reading provable
and the VA-range reading false. A single failing arm would have supported both.

## ★★★ Three instrument defects in one rung, one shape

A notifier allocator used as a general one · a gpu-node CPU map of a ctl-node object · a 4 KiB
offset for a 64 KiB-page aperture. **Every refusal was real, correct, and about the probe**, and
each looked like a product finding until a control ran. ⇒ Never report a rung's first red.

## The command surface now

    --guest-ram-decoys N      rows declared ahead of the operand (scale)
    --guest-ram-at 0xHEX      dictate the operands' GPU VA (address)
    --guest-ram-dst-vidmem    destination in device memory — the real H2D shape (aperture)

Three dimensions, movable one at a time. `scripts/bench/w418_r34_boot.sh` runs depth, address
and H2D sweeps in the guest; bare metal is the control for every arm.

## ★★★ BARE METAL IS GREEN ON ALL THREE DIMENSIONS

`[measured w421, BARE METAL]`, destination aligned to its own page size:

    RM-placed        src 0x120000000     dst 0x120010000     (P)
    0x7cac33600000   src 0x7cac33600000  dst 0x7cac33610000  (P)   <- the LLM's faulting VA
    0x768327600000   src 0x768327600000  dst 0x768327610000  (P)
    0x7f0000000000   src 0x7f0000000000  dst 0x7f0000010000  (P)

⇒ Confirms w420's page-size reading and kills the VA-range one: the identical arms that failed
at a 4 KiB offset all pass at 64 KiB. Green on **scale** (0 and 2000 decoys), **address** (six
VAs including the LLM's own) and **aperture** (sysmem and device-local destinations).

⇒ **Every bare-metal half of `BARE-METAL PASS + GUEST FAIL ⇒ KAYFABE BUG` now exists.** The
guest arms are the entire remaining question.

## ⊘ Still open, and how to read it

The guest's address sweep passed `rm`, `0x120000000`, `0x400000000` and produced **no verdict**
for the three high addresses. ⚠ Do NOT read that as *"high addresses fail in the guest"*:
- The hook piped each arm into `grep -oE` and kept only matches, so the three blank arms'
  output was **discarded**. Fixed — it now captures raw first and prints the tail.
- That boot logged **0 host Xid lines**, `covered_pct=100.0000%`, no guest panic, and a clean
  `reboot: Power down`. Nothing faulted and nothing crashed.
⇒ `(E)` means UNMEASURED. Re-run with the fixed hook before drawing anything from it.

---

# w424–w426 — THE WALL IS A RACE AT SYNCHRONIZATION POINT (2)

## ★★★ The measurement that reframes everything

`[measured w425llm]`, joining our log to `dmesg -T` through the `at=` stamp:

    pin of the faulting range   at=1789096791   asked=1331 pinned=1331   in 383 ms
    CE2 HUBCLIENT_CE0 Xid       1789096791      FAULT_PDE ACCESS_TYPE_VIRT_WRITE @ 0x73c8_cf600000

**Same second**, with the pin consuming 383 ms of it. `last_pinned_va=0x73c8cfb32000` covers
the declared range `0x73c8cf600000+0x533000` exactly; every row says `placed_as_asked=true`.

⇒ **The engine ran while the pin was in flight.** *"We never back it"* is dead. ⊘ One-second
granularity cannot order two events inside one second — this establishes concurrency, not
which came first.

## ⊘ Three readings killed, each with its own control

- **The device-open wedge is not the LLM's blocker.** `LLM_SKIP_4X4=1` moved the GPU run one
  open earlier; it ran and produced the Xid itself (`XIDS=150/150/151`).
- **Not the address.** R34 passes on bare metal at the LLM's exact faulting VA.
- **Not the aperture.** R34 passes with a device-memory destination too, once the destination
  is aligned to its own 64 KiB page size.

## ★★★★★ THE SEPARATE, CHEAPER BUG: the guest wedges on the 4th device open

`[measured w423/w424]` opens #1–#3 pass, **#4 fails EIO, permanently**. The guest's own dmesg,
captured at the first failure:

    NVRM: Assertion failed: status == NV_OK @ ce_utils.c:304
      <- objCreate(&pScrubber->pCeUtils, ...)  NV_ERR_GENERIC (0xFFFF)
      <- scrubberConstruct -> memmgrScrubHandlePostSchedulingEnable_HAL
    NVRM: RmInitAdapter failed! (0x25:0xffff:1249)

`ce_utils.c:304` is the assert after `memmgrMemUtilsCopyEngineInitialize_HAL`, and inside it
`_memUtilsAllocCe_GM107` begins **`if (!pChannel->hTdCopyClass) return NV_ERR_GENERIC;`**.
`hTdCopyClass` comes from `memmgrMemUtilsGetCopyEngineClass_GM107`, which loops every CE
engine calling `gpuGetClassList(..., ENG_CE(eng))` and takes the first with `numClasses > 0`.

⇒ **On the 4th adapter init, no CE engine reports any class.** That is an engine/class
enumeration our GSP emulation serves, and it degrades across adapter inits. Repro is
`rmladder --guest-ram-decoys 0` four times — seconds, no CUDA.

⚠ It CONFOUNDS every multi-arm sweep: each arm is a process, hence an open. My address sweep
read as *"CUDA-shaped addresses fail"* when those arms never reached a map. **Run the
open-ordinal probe first** — it is now the first block in `w418_r34_hook.sh`.

## The live thread

The RPC-map path already has the right mechanism — `holds_for_refresh` → `HeldReply` →
released once the worker published. That IS synchronization point (2), which the owner's
ruling says may block.

⚠ It was SILENT: `self.held.push(...)` wrote no line, so a boot's log carrying no held-reply
record proves nothing. w425 adds `HELD-REPLY` and `HELD-REPLY-POSTED`. **Read them as a
pair** — holds without releases is a parked guest, neither is a guest that never waited.

⇒ **Next: read those two counters in the w426 boot.** If the hold never fires for the RPC that
declares the LLM's operand range, that is the defect and the fix is to make it fire. If it
fires and the fault still happens, the reply is being released before the pin finishes.


---

# ⊘⊘ STOPPED FOR AN OWNER DECISION — 2026-09-11, ~06:20

The diagnosis is complete and the identified fix needs a call that is not mine to make.

## The decision

The fix for the race is the owner's own GPGA design — reserve guest VRAM as one RM object at
boot so backing is eager rather than lazy, and derive the advertised size from a successful
reservation. **Re-measured tonight on the bench:**

    GPGA_LARGEST_RESERVABLE_MB = 6144      (advertised today: 12288)

⇒ Implementing the design as written **halves the VRAM the guest sees**, on a 12 GiB card.
The doc already says the advertised size follows the reservation, so this is that rule
arriving with a number attached — but it is a product-visible change and it should be yours.

Options, as I see them:
1. **Advertise 6144.** Simplest, matches the doc, costs the guest half the card.
2. **Find out why only half is reservable first.** `6144` is exactly half, which smells like a
   per-client or contiguity limit rather than real occupancy. ⊘ `reserve_gpga` is already
   NON-contiguous and page-aligned, so the obvious cause is already excluded.
3. **Reserve lazily in chunks** — keeps the advertised size, loses "refuse to boot on failure",
   which is the property that makes the design safe.

## Also open, and cheaper than the above

- **`PT-DECODE latched=0 requeued=1903 rounds=0`** on the w426 boot (w416 latched 53 in 2
  rounds). The earliest discovery point did nothing all boot. `EXEC-WITNESS` on the same boot
  says `⊘SKIPPED(w318 dirty gate …)` — check that gate first.
- **The 4th-device-open wedge** (`ce_utils.c:304`, no CE class enumerated on the 4th adapter
  init). Separate from the LLM, seconds to reproduce, and it confounds any sweep past 4 arms.
- **`holds_for_refresh` fires 0 times against 304 row-binding RPCs** — a real bug in
  synchronization point (2), irrelevant to the LLM (that boot issues zero map RPCs) but wrong
  for every path that does use one.
