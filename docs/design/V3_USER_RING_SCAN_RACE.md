**STATUS: LIVE, 2026-10-14.** Why the owner's Windows 11 resets the display driver when Edge starts, and
the two fixes. Branch `claude/user-ring-scan-20261014` (from `claude/linux-oracle-planes-20261014` `08fa664a`).
Labels: **[measured]** = a run named here, evidence under `traces/user_ring_scan_20261014/` (`INDEX.txt` maps
runs to revisions); **[read]** = public source (open-gpu-doc `dev_pbdma.ref`/`dev_ram.ref` for Ampere, ogkm
580.65/580.159) or kayfabe code; **[inferred]** = reasoning, never written as evidence.

# V3 — the Windows USER-ring relay: reading too early, and not reading at all

## 0. Short answer

Win11 26100 + NVIDIA 580.88 under kayfabe reset its display driver (GSP `Running → Suspending`) whenever Edge
started. The previous agent found the resets follow `USER_RING_REFUSED … USER_RING_SCAN …` and a refused
`RESET_CHANNEL` (G16/G17). Two root causes, both in the USER-ring relay (`crates/kf-qemu/src/userpump.rs`), both
about **when** it reads the guest's ring — not about the scanner's grammar:

1. **It read pushbuffers the GPU would not yet fetch** [measured]. The guest publishes GP entries behind a
   `GP_ENTRY1_SYNC_WAIT` NOP whose pushbuffers it writes only later, gated by a semaphore acquire in the preceding
   segment — legal: Host does not fetch past a SYNC_WAIT entry until the preceding segment has been processed
   [read, `dev_pbdma.ref` "NV_PPBDMA_GP_ENTRY1_SYNC"]. The relay scanned them the moment `GP_PUT` named them:
   all-zero or half-written segments, which it refused (`Legacy`/`Header`/`SubDeviceMask` on data words) or, worse,
   sized a stale data word as a packet header and lost its place for every later segment (`MethodOverflow` far
   downstream). **Fix: `kf_chan::user_gate::SyncGate`** — before reading anything at or after a guest SYNC_WAIT
   entry, author the same wait on the private ring (barrier NOP + witness, the mechanism staged segments already
   use) and read on only once the witness has begun processing.
2. **It stopped reading `GP_PUT` when its own ring drained** [measured]. The guest rings the doorbell for a few
   of its `GP_PUT` advances only (205 doorbells against ~70 000 new-work `GP_PUT` reads on one channel); hardware
   synchronizes `GP_PUT` from USERD on its own while the channel is loaded [read, `dev_pbdma.ref` "NV_PPBDMA_GP_PUT"].
   The relay read `GP_PUT` only on a doorbell or while its own ring was in flight, so work published after a drain
   sat unread for 1.6-55 s (30 `IDLE_STALL`s in one run), the guest's 2 s timeout detection fired, its per-channel
   `RESET_CHANNEL` was refused, and it reset the adapter. **Fix: `kf_chan::user_gate::Poll`** — read `GP_PUT` every
   worker tick (1 ms) for 250 ms after new work, then at least every 20 ms.
3. `RESET_CHANNEL` (G17) stays refused; §7 says why. With both fixes it is no longer reached in the lanes run
   here [measured, §8].

The scanner's grammar was never the defect: every one of 19 traced refusals is explained by (1) — 15 were bytes
that changed under the scan and rescanned clean 1 ms later, 4 were downstream of a stale straddle [measured].

## 1. Hypotheses and falsifiers (each written before its run; `INDEX.txt` names the run)

| # | hypothesis | falsifier (written before) | result |
|---|---|---|---|
| H-RACE | the scan reads GP entry / PB bytes before the guest's writes are visible | the 1/20/200 ms re-reads return the same words and refuse at the same word | **holds** as "the bytes change under the scan": 15/19 refusals changed by +1 ms and rescanned clean; the other 4 carried a packet from an earlier stale straddle (`879043c5` …`9fedc8bf`) |
| H-DESYNC | the scanner sizes some legal packet differently from hardware | header chain from a known boundary is consistent and the refused word sits at a true header position | **falsified**: refused words are data (`0x18c00000`, pointers, pixels); every desync traces to a stale read, none to a sizing rule |
| H-APERTURE | the scan reads the wrong backing (stale mirror row) | the backing pieces at scan and at re-read are identical | **falsified**: 182/182 stale reads at the same backing (`9fedc8bf`, `backing_moved=Some(false)`) |
| H-SETTLE | a 1 ms delay after `GP_PUT` is enough | stale reads persist past it | **falsified**: all-zero 88-word segments read after the settle (`9854bbe6`/`9fedc8bf`, one later refusal) |
| H-SYNC | the stale bytes are pushbuffers behind an unreached SYNC_WAIT | with the gate and no settle, VERIFY counts stale reads before the GPU began the entry, or any scan refusal in boot + 5 min Edge | **holds**: every refused entry's GP ring shows a SYNC_WAIT entry the GPU had not reached (15/15 direct refusals); with the gate 0 stale reads in ~100 000 verified segments and 0 refusals (`ba165078`) |
| H-IDLE | the remaining resets come from `GP_PUT` advances without a doorbell after the relay drained | with polling, `IDLE_STALL`s still appear, or resets persist with no stall/refusal | **holds** (§8) |
| H-ESCALATE | the refused `RESET_CHANNEL` turns a channel recovery into an adapter reset | a reset cluster without a preceding `906f0102` refusal | consistent: every reset in runs with refusals was preceded by one [measured]; not acted on (§7) |

## 2. The measurement trace — `KF3_USER_RING_TRACE=<dir>` (default off; measurement only)

Off, nothing changes (one cached flag load on the doorbell path). On, the pump keeps per channel the last 64
scanned segments (≤ 1 MiB of words) and writes, into `<dir>`:
- `user-ring-refusal-<client>-<chan>-<cursor>.txt` once per refused channel: GP descriptor and its backing, the
  header chain the scanner read, the GP ring around the cursor, three re-reads of the same GP slot and segment at
  1/20/200 ms rescanned from the same carried state, the history;
- `STRADDLE` lines + files: a segment that starts at a packet boundary and ends inside a packet it started (legal —
  a 1057-word inline shader upload is split across entries on every channel — but the shape a desync leaves), with
  deferred re-reads;
- `VERIFY`/`STALE`: every VRAM segment ≤ 1024 words re-read 0.2 and 2 ms after its scan; a change before the GPU
  began the entry (`prefetch=true`) is a real scanner/GPU mismatch;
- `STALL`/`IDLE_STALL`: no progress for 1.5 s while turning / no turn for 1.5 s while `GP_PUT` is ahead;
- doorbell counts per token and by route; `MIGRATED` detail (the new ring's USERD and first entries).
Guest words go only to those files; the log carries one bounded line per event.

## 3. Root cause 1 — reading behind an unreached SYNC_WAIT

[measured, `user-ring-refusal-c1d00054-ff040013-453.txt`] The refused entry 453 (a 56-word VRAM segment) read as
`00000000 ×21, 00000003, 18c00000, 318c0300, 80000e68, e0000000`; 1 ms later the same backing held a complete
stream (`80000800 80110810 a0020815 … a0070045 [7 data] 80000e6a e0000000`) that scans clean. The GP ring:
`447 SYNC NOP, 448 seg, 449 seg, 450 seg, 451 SYNC NOP, 452 seg, 453 seg` with the GPU's `GP_GET` at 449: the
hardware had not even begun 451. The same shape in every one of the 15 direct refusals (`INDEX.txt` runs): an
unreached SYNC_WAIT NOP 1-4 entries before the refused one.

[measured, `user-ring-stale-c1d0002a-ff04000d-10-2000.txt`] On every channel at birth, entries 27-31 (five
88-word VRAM segments behind the SYNC_WAIT NOP at 25) read entirely as zeros ≥ 1 ms after `GP_PUT` named them,
with the GPU at entry 17; 2 ms later they held commands. A time-based settle could not cover this (H-SETTLE).

A stale read that parses is worse than a refusal: zeros parse as NOPs, and a data word that happens to decode as
an incrementing header with a large count (`0x318c0300` = 4492 arguments at `0xc00`) makes the scanner treat the
following segments as that packet's data, until the method address overflows thousands of words later
(`user-ring-refusal-c1d000f0-ff040011-152.head.txt`: the carried packet started in a straddle whose first 24
words were zeros at scan and a valid stream at the +1 ms re-read).

**The rule** [read]: `GP_ENTRY1_SYNC_WAIT` — "Host waits until the preceding PB segment has been processed by Host
before beginning to fetch the current PB segment … if SYNC_WAIT is specified [for LENGTH 0], no further GP entries
will be processed until the wait finishes"; `GP_GET` is "the next GP entry to begin being processed". So a guest
may fill pushbuffers behind a SYNC_WAIT until the preceding segment's semaphore acquire is released.

**The fix** (`SyncGate`, `c45ac776`): at a guest entry with SYNC_WAIT, publish the host barrier (SYNC_WAIT NOP)
and witness on the private ring, and read nothing until the private `GP_GET` passes the witness — the barrier's
wait (the preceding segment's processing) is then complete, which is when the hardware would fetch. The guest's
own SYNC entry is then relayed unchanged. Engine test `a_segment_behind_an_unreached_sync_wait_is_not_read_until
_the_witness_runs` replays the measured ring and fails without the gate.

## 4. Root cause 2 — not reading `GP_PUT` without a doorbell

[measured, `run-e0a6c5ed.lines.txt`] With the gate in, the stall trace showed pumps with no turn for 1.6-55 s
while the guest's `GP_PUT` was ahead of the cursor (`tick=false pending=false`), 30 times in one boot + Edge run,
and one adapter reset. The doorbell census: all doorbells arrive on the relay's (Translated) tokens, none lost, but
few — `puts_with_doorbell=205 puts_without_doorbell=69971` on Edge's main channel. The relay's worker turned a pump
only on a doorbell, a software-object wake, or while its private ring had descriptors in flight
(`tick = fetched != issued`); after the private ring drained, a guest that believes its channel is still loaded
advances `GP_PUT` and rings nothing.

**The rule** [read]: `NV_PPBDMA_GP_PUT` — software writes `NV_RAMUSERD_GP_PUT`; the register "is not immediately
synchronized … there will be a delay" — Host picks the new value up by itself while the channel is loaded.

**The fix** (`Poll`, `50068268`): keep the worker's 1 ms tick for 250 ms after the last new work, and let the
worker's park tick (≤ 50 ms) turn any relay whose last read is older than 20 ms. Unit test
`a_drained_relay_still_reads_put_without_a_doorbell` fails under the old rule. Cost: one USERD read per active ring
per ms, one per idle ring per ≤ 50 ms.

## 5. Kept as defence in depth

`Grace` (`kf_chan::user_gate`): a scan refusal of a segment never published is re-read and rescanned for 50 ms
before it is final, with one `USER_RING_SCAN_RETRY` line per channel (first four). After the two fixes no
retry was observed (§8); it exists so a visibility window nobody has measured yet costs a delay rather than a dead
channel. A refusal after the grace is the same refusal as before.

## 6. Linux / ogkm oracle

Not built. The relay under test exists only for Windows USER rings (`KF3_WIN_USER_CHANNELS_PASSTHROUGH`); a Linux
guest's user channels are Passthrough and never scanned, so a Linux program submitting the same stream shapes
would not reach it. Both root causes are timing contracts of the Windows driver (fill-behind-SYNC_WAIT, doorbell
elision) that the scanner's grammar tests cannot reproduce; the engine replay in §3 encodes the measured ring.

## 7. `RESET_CHANNEL` (G17) — why it stays refused

[read] `NV906F_CTRL_CMD_RESET_CHANNEL` "resets the channel … and also resets the specified engine"; CPU-RM fills
`bIsRcPending` and RPCs it verbatim; the physical implementation is in GSP-RM, not public. Serving it means one of:
- **authoring the host's own `RESET_CHANNEL` on the twin** (the verb is `RMCTRL_FLAGS_NON_PRIVILEGED`): its engine
  reset is not shown to be confined to our channel on a shared GPU, and public source cannot say what it does;
- **answering NV_OK from kayfabe's own model** (drop the pump state, resync to the guest's `GP_PUT`, clear `dead`):
  a completion kayfabe did not perform; what the guest's KMD expects of `GP_GET`/`GP_PUT` and pending work after
  an engine reset is not public either. If it is wrong, the guest's next TDR comes within seconds and Windows'
  TDR limit ends in bugcheck 0x116 — worse than today's adapter reset, which recovers.
Both root causes that made the guest ask for it are fixed; with them it was not reached [measured, §8]. The
decision on a served form (and the forced-channel-fault lane that would measure the guest's expectations) is the
owner's.

## 8. Validation

See `docs/STATUS_AND_HANDOFF.md` (UPDATE 2026-10-14 03:xx) for the final numbers with their revisions; the
per-run table is `traces/user_ring_scan_20261014/INDEX.txt`.

## 9. What remains

- G17 (§7): owner decision.
- Performance: each guest SYNC_WAIT entry now costs one barrier+witness round trip on the private ring (the
  hardware waits for the same processing; kayfabe adds its `GP_GET` observation latency, ≤ 1 ms while ticking).
  Not measured beyond "4K60 YouTube plays, desktop at 60 Hz".
- `USER_RING_PUT_READ: Gone` (one, `25e57fb9`): the USERD lease vanished under a live pump (channel teardown race);
  not investigated.
- The ring-growth migration (`USER_RING_MIGRATED`) was reached once (`ba165078`) and followed by a reset before the
  polling fix; whether it is exercised cleanly with both fixes is in §8's run log.
