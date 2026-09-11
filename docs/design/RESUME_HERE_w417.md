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
the whole of w415. ⊘ **R34 has not yet been run**, on bare metal or in the guest.

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
