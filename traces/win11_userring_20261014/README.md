# Win11 USER-ring relay: the Minecraft in-world Xid 32 and the Edge NVDEC TDR

**STATUS: RESEARCH, 2026-10-14.** Branch `claude/win11-userring-20261014` (from `claude/win11-opengl-20261014`
`033dbf42`). VM 2990 (owner's Win11, RTX 4070 AD104 host, host driver 595.91.07, guest 580.88). Labels:
**[measured]** = a run or log named here; **[read]** = public source (open-gpu-doc `9fdf5c40` manuals/classes,
ogkm 580.159.04) or kayfabe code; **[inferred]** = reasoning, never written as evidence.

## 1. Defect 2 — Edge/YouTube TDR: the kernel NVDEC channel decoded as copy-engine work

[measured, `nvdec_refusal_0c5fedff.txt`, run 2990 at `0c5fedff`, `qemu-2990-0c5fedff-full.log:111267`] The Windows
kernel driver's NVDEC channel (`0xc1d00018:0xff040003`, engine `0x13`, host video object class `0xC9B0`, born
Translated at boot) sends its first segment when Edge starts video: three host semaphore acquires/releases, then on
subchannel 4 `NVC9B0_SEMAPHORE_A/B/C` (`0x240-0x248`), `SEMAPHORE_D = 0` (a 32-bit release) and
`SEMAPHORE_D = 0x30000` (`OPERATION_TRAP`: a non-stall interrupt), then a host acquire on the same address. kf3 refused
it as `NoCeObject { subch: 4, method: 0x240 }` and declared the channel DEAD; the guest TDR'd and recovered.

[read] Root cause: the T-mode decoder (`kf_chan::tmode`) knows two engine tiers, copy engine (subchannel 4 on the
graphics runlist) and GR. A kernel VIDEO ring (born on its engine's runlist since `b52da0c7`, with a real owned video
object) had no tier, so subchannel 4 was decoded as CE work, and the host video ring refused any non-empty push.
ogkm carries only the video classes' ids (`clc9b0.h` defines `NVC9B0_VIDEO_DECODER` and nothing else); their method
headers are public in NVIDIA/open-gpu-doc `classes/video/`.

Fix (`1c503f87`): the VIDEO tier (`kf_chan::vtables`). Method offsets and the semaphore methods' fields are DERIVED
(`tools/derive_video_methods.py`: `gcc -E -dM` macro table, compiled DRF evaluation; `video_generated.rs`, classes
C5B0/C5B7/C6B0/C7B0/C7B7/C9B0/C9B7/C9D1); only the disposition is by hand, keyed by NAME: `NOP` consumed;
`SEMAPHORE_A/B` an address (resolved through the channel's rows, emitted by the perimeter `put_video_semaphore` as a
window address); `SEMAPHORE_C`, `SET_SEMAPHORE_PAYLOAD_*` data; `SEMAPHORE_D` decoded against the class's own fields
(every bit in a named field, every value named, `RELEASE` or `TRAP` only, bytes from `STRUCTURE_SIZE`/`PAYLOAD_SIZE`
re-derived in the perimeter); every other named method refused by name (`EXECUTE`, every codec operand, the
monitored-fence address); unnamed offsets unclassified. A class with no public header has no tier (refused by name).

| hypothesis | falsifier (before the run) | result |
|---|---|---|
| H-V1: the TDR is the refused first segment (no video tier) | with the tier, Edge+YouTube still TDRs with a `ring REFUSED` / `DEAD` on the NVDEC kernel channel, or TDRs without one | §4 |
| H-V2: the kernel NVDEC channel carries only synchronisation (semaphores, trap), not codec work | a later segment refused `VideoMethod { EXECUTE … }` or a codec operand | §4 |

Tests (fail before, pass after): `tmode::video_tier_tests::*` replay the measured 32 words (refused `NoCeObject`
without the tier; admitted with it, every address the window address of the guest's row, no guest VA word emitted;
hostile words refused by name; a semaphore outside the VM's rows or past a row's end refused at bind);
`vtables::tests::*` (derived layouts per class: `C9B7` `PAYLOAD_SIZE` is bit 2, `C9D1` `UPPER` is 25 bits).

## 2. Defect 1 — Minecraft in-world Xid 32 PBENTRY on javaw's USER-ring twin

[measured] 3/3 in the previous agent's runs: host Xid 32 `intr0 0x40000` (`NV_PPBDMA_INTR_0_PBENTRY`) on javaw's GR
USER-ring twin, the guest's `RESET_CHANNEL` refused 0x56 four times, adapter reset. Ordering (this branch,
`minecraft_xid32_timeline.txt`): `dmesg -T` is ~4 s off on this host; the journal's precise kernel times are
17:04:41.396 (run e821) and 17:39:17.216 (run d86f), and the kf3 display clock puts the first `RESET_CHANNEL` at
≈17:04:43.4 and ≈17:39:19-20 — the Xid comes FIRST, ~2 s before the guest's recovery (its TDR timeout) [measured
ordering; causality inferred]. The ring-growth migration (`USER_RING_MIGRATED`, 32768 → 65536 entries) happened 20 s
(e821) and ~95 s (d86f) earlier: not adjacent. kf3 armed no RC notifier for USER-ring twins (`rc[armed=0 …]`): the
guest learned of the hang from its own timeout, not from kayfabe.

[read] `PBENTRY` = the PBDMA expected a PB instruction (a header) and the word did not decode, or the header's count
would overflow the method space (`dev_pbdma.ref` `NV_PPBDMA_INTR_0_PBENTRY`). The relay copies only GP entries; the
GPU fetches the guest's pushbuffers itself — except for what the relay INSERTS into its private ring.

### Ranked candidates (each with its falsifier, written before the run)

1. **H1 — the SYNC_WAIT gate's witness lands inside an open packet** [read + inferred; strongest]. `dev_pbdma.ref`
   (`NV_PPBDMA_GP_ENTRY1_SYNC`) explicitly allows a compressed method sequence whose header ends one segment and
   whose data is in the SYNC_WAIT segment; the PBDMA's parse state carries across GP entries. Packets straddling GP
   entries are measured on every Windows channel (`traces/user_ring_scan_20261014`: a 1057-argument inline upload,
   `carried_out=IncrementOnce sub1 m0x1b0 1/1057`). The gate (`c45ac776`) authors a barrier (control NOP, fine) and a
   WITNESS that is a 4-byte zero `DMA_NOP` word — regardless of the scan's carried state. With a packet open, the PBDMA
   takes that zero as the packet's next argument, every later guest word shifts by one and the packet's last data word
   is parsed as a header: PBENTRY when it does not decode (inline shader/texture/vertex data almost never does), and
   one corrupted word in the upload. World loading = heavy inline uploads.
   Fix (`1c503f87`): `kf_chan::user_gate::Witness` — while the scan carries an open packet the witness is a CONTROL
   NOP GP entry (no byte; it cannot begin processing before the barrier's wait finishes, and GP_GET passes an entry
   only once it began processing — the same proof); at a boundary the measured byte witness stays. Each occurrence is
   counted (`USER_RING_SYNC_CARRIED`, first four lines per channel + release summary; production, bounded).
   Test: `user_gate::tests::a_packet_carried_into_a_sync_wait_entry_gets_no_witness_byte` — a PBDMA parse model of
   the private ring must generate exactly the guest's methods; the old witness gives a header error at the shifted
   word (fails before, passes after).
   **Falsifier:** on the fixed build, Minecraft in-world still Xid 32 PBENTRY on the javaw twin; or a Minecraft run
   that would have failed shows zero `USER_RING_SYNC_CARRIED` (then the fix cannot be what changed).
2. **H2 — a false trigger in a stale/desynced scan splices the stream** (staging zero-fills and skips a "trigger"
   that is really mid-packet data). Falsifier: the STALL dump (below) shows no `staged copy` descriptor near the
   private GP_GET.
3. **H3 — a pushbuffer page read through a stale twin mapping** (the guest moved the allocation, the mirror lagged).
   Falsifier: the STALL dump's scan-time words of the segment holding PB GET equal a re-read, and no walk landed for
   that VA between publication and the fault.
4. **H4 — the ring-growth migration** (source switch). Falsifier: the faulting descriptor belongs to generation 2 and
   is more than a few hundred entries past the switch (measured gap 20-95 s argues against it already).

Measurement for H2-H4 (only if H1 is falsified), `KF3_USER_RING_TRACE=<dir>` (default off, never required): every
private descriptor is named (`trace_slots`), and the STALL report (no progress 1.5 s) adds `STALL_USERD` (our private
USERD as written back: PB GET, TOP_LEVEL_GET, GP_GET, PUT), `STALL_SLOT` (the descriptors around the private GP_GET)
and `STALL_SEGMENT` (the scanned guest segment holding PB GET, its words at scan time and carried state).

## 3. RESET_CHANNEL (`0x906f0102`) for the VM's own USER twin — decision: stays refused in this branch

See `docs/STATUS_AND_HANDOFF.md` §0 for the argument (the measured requests are nvlddmkm's engine-reset step inside
the guest's TDR; the only rule-conforming host act is the NON_PRIVILEGED host `RESET_CHANNEL` on our own twin, whose
effect on a PRIVATE ring/USERD is implemented in closed GSP-RM and not measured; an NV_OK without it is a forged
recovery; both measured triggers are fixed at source; a served form needs RC notifiers armed for USER twins and a
forced-fault lane).

## 4. Windows verification

**Run 1** (`run1_a8424ae2_summary.txt`): VM 2990 on kf3 `a8424ae2`, production profile (no `KF3_` measurement flag),
DMA-FQ launcher, 2026-10-14 20:09-20:32 CEST. Graceful stop of the previous VM via kfagent first.
- Minecraft Launcher (Store app) → Play → Java 26.3 title screen → Singleplayer → existing world, 6 min in-world
  walking (QMP keys) [measured]: **0 host Xid** (journal), 0 GSP reset, 0 `USER_RING_REFUSED`/`SCAN`, no new guest
  nvlddmkm/dxgkrnl event, no new LiveKernelReports dump (the 81 WER 1001 `LiveKernelEvent 141` entries are the queue
  re-reporting 2026-10-13/14 dumps: every one attaches a `WATCHDOG-*` file dated before this run).
- **H1 not falsified, and the hazard is pervasive** [measured]: javaw's GR USER ring gated
  **25 153** guest SYNC_WAIT entries with an OPEN packet in those 6 minutes (`USER_RING_SYNC_CARRIED SUMMARY
  … gated_with_open_packet=25153`), the first four at world load (`IncrementOnce sub0 m0x3870 4/29`: a packet whose
  header and 4 arguments end one segment and whose remaining 25 arguments are in the SYNC_WAIT segment — exactly
  the manual's case). Each was a one-word stream shift under the old witness. Before the fix the same scenario failed
  3/3 within seconds of the world loading; after it, 0 Xid in 6 min [the causal link is inferred from the mechanism
  plus the before/after; the old binary was not re-run].
- Edge + YouTube (Big Buck Bunny 4K60, 10:34 played to the credits, hardware decode: a user NVDEC twin with a
  `0xC9B0` object) [measured]: 0 host Xid, 0 GSP reset, 0 `ring REFUSED`/`DEAD`, no guest nvlddmkm/dxgkrnl event.
  H-V1 not falsified. H-V2 (kernel NVDEC channel = synchronisation only) needs the channel's RETIRED line (at VM
  stop): pending.
- Two `display: STALL … ready, waiting for a vblank` 1 s reports (the known D-EARLY report; recovered, 60 fps).
- Kernel NVDEC ring retire lines (`run1_queue_timeline.txt`) [measured]: `forwarded=35/70/105`, `dead=None` in every
  guest life — admitted by the VIDEO tier, no `VideoMethod` refusal: H-V2 not falsified.

**App queue, same VM life / QEMU process, production profile** (`apps/`, `run1_queue_timeline.txt`) [measured]:

| app | result | numbers | resets / host Xid during the app |
|---|---|---|---|
| GravityMark 1.89 D3D12, 1280x720, 20k asteroids | **PASS** (TDR before: H-d3d12) | score 8834, 52.9 fps, 167 s | 0 / 0 |
| GravityMark 1.89 Vulkan, default route, no relay flag | **PASS** | score 3817, 22.8 fps, 167 s | 0 / 0 |
| Unigine Heaven 4.0 OpenGL windowed 1280x720, full Phoronix pass | **PASS** (early exit before: H-heaven) | 157.1 fps, 260.6 s | 0 / 0 |
| Unigine Heaven 4.0 OpenGL fullscreen 1920x1080, full pass | **PASS** | 160.1 fps, 260.6 s | 0 / 0 |
| Unigine Heaven 4.0 D3D11 windowed | **FAIL** at ~12 s | — | 1 GSP reset / host Xid 13 `Graphics Exception: ESR 0x404000=0x80000010` (DISPATCH; bit 4 not named publicly) on Heaven's USER-ring twin, class `C9C0` |
| Edge + YouTube, continued (autoplay) after the 10 min check | **FAIL** at ~16 min (20:36:31) | — | TDR without any host Xid or kf3 refusal, `RESET_CHANNEL` refused, a reset cascade (8 GSP cycles, Xid 13 DISPATCH on DWM-era channels after the first reset) and bugcheck **0x116** at 20:46; the guest rebooted under the same QEMU and came back to the desktop |

Installers (guest `C:\kf\apps`, downloaded in the guest from the vendors, sha256 checked against
`scripts/bench/windows/appmatrix/manifest.json`): `GravityMark_1.89.msi` `38e32b47…169c8f` (extracted `msiexec /a`),
`Unigine_Heaven-4.0.exe` `497865a0…0e5f` (Inno `/VERYSILENT /DIR=C:\kf\apps\heaven`). Runners:
`scripts/bench/windows/kfagent_ogl/heaven.ps1` (`-extern_define PHORONIX,…`: one full pass, then exit) and `gm.ps1`
(the app matrix's GravityMark arguments).

## 4b. Defect 3 — host Xid 13 (`ESR 0x404000=0x80000010`, class `C9C0`) right after a USER-ring growth migration

[measured, `xid13_migration_trace.txt`, runs at `f163b85e` with `KF3_USER_RING_TRACE` + `KF3_RPC_TRACE`] Heaven D3D11
(3/3: 20:48, 21:11, 21:17) and Edge (21:44, the owner-observed TDR) fault the same way: the GR engine raises a DISPATCH
exception on the app's USER-ring twin seconds after `USER_RING_MIGRATED` (32768 → 65536 entries); the guest's
`RESET_CHANNEL` is refused, TDR, in the first production run a reset cascade and bugcheck 0x116. In both, the
migration's `FIFO_UPDATE_CHANNEL_INFO` trigger sits inside a front-end atomic sequence on the compute subchannel
(`clc7c0.h`): `… 8000a01e 8000a009 80002052 (FE_ATOMIC_SEQUENCE_BEGIN) 8001201e (WFI) [2001a080 <handle>] 80002053
(FE_ATOMIC_SEQUENCE_END)`, and the PBDMA was stuck on the staged one-word suffix (END) that the relay publishes only after
the action. [read] `0x404000` is the GR DISPATCH trap status (nouveau `gf100_gr_trap_intr`: "DISPATCH"; bits 0-2 named
there, bit 4 not named in any public source).
H-D4 (stated before the fix run): the relay leaves an FE atomic sequence OPEN with nothing to fetch while it services the
trigger (barrier, witness, action), long enough for the channel to be switched out; hardware services the software method
with the PBDMA stalled on it. Falsifier: with the remainder published in one piece, Heaven D3D11 still faults after the
migration. Fix `c5d9693f`: for `FIFO_UPDATE_CHANNEL_INFO` only (a source switch with no GPU-visible effect on the segment's
remaining methods) the whole remainder — prefix, the trigger zero-filled to `DMA_NOP`s, suffix — goes out as one staged copy
(`gapless_remainder`; test `a_channel_update_trigger_inside_an_atomic_sequence_is_published_without_a_gap` on the measured
words). Result [measured]: Heaven D3D11 full Phoronix pass 3/3 with 0 host Xid and 0 GSP reset (traced `c5d9693f`, production
windowed, production fullscreen), the migration logged in each: **H-D4 not falsified**. Whether the bit-4 DISPATCH error is the
context switch or the gap itself is inferred, not measured. (The event-loop branch's H-gate — reads ahead of processing via
GP_GET — was a second candidate for the same symptom; these runs still used the GP_GET gate and did not fault.)

| run (production unless noted) | Heaven D3D11 1280x720 | Heaven D3D11 1920x1080 fullscreen | Xid / resets |
|---|---|---|---|
| `a8424ae2` | Xid 13 at ~12 s | — | Xid 13, TDR |
| `c5d9693f` traced | 16.3 fps, 261 s, finished | — | 0 / 0 |
| `c5d9693f` | **17.0 fps**, 261 s, finished | **20.5 fps**, 261 s, finished | 0 / 0 |

Heaven's earlier early exit (29 s) is gone on both APIs: OpenGL 157/160 fps and D3D11 17/20.5 fps run the full 261 s pass.
Phoronix mode prints only the average fps (no min/max).

## 4c. Heaven D3D11 fps (H-perf) — numbers only, verdict for the event-loop branch

[measured, `heaven_d3d11_relay_perf_fc8ba3b8.txt`, traced 89 s Heaven D3D11 run at `fc8ba3b8`, ~1190 frames] Heaven's GR
USER ring: 2 314 629 GP segments relayed (≈26 000/s, ≈1 900 per frame), 2 486 644 relay turns (1.07 per segment), 5 418
SYNC_WAIT gates (61/s, **≈4.6 per frame**), gate drain avg **284 µs** (max 3.25 ms), 5 592 guest doorbells (63/s; the guest
rings for 0.24 % of its PUT advances), GSP RPCs ≈11/s (mostly `0x00809908`). Two `kf3-worker` threads at ~50 % CPU each.
H-perf ("frame rate limited by the 1 ms tick / SYNC_WAIT drain latency") — falsifier branch 2 applies: drains per frame ×
drain latency ≈ 4.6 × 0.28 ms ≈ 1.3 ms of a 59 ms frame (frame time / drains ≈ 13 ms ≫ 1 ms): **falsified**. What scales
with the frame time is the per-entry relay cost: ≈1 900 GP segments per frame, each relayed in its own turn (USERD PUT read,
GP + pushbuffer snapshot, scan, publication and a host doorbell) at ≈30-38 µs each [inferred from the totals]. Batching
entries per turn and per doorbell is the lever; that code belongs to `claude/userring-eventloop-20261014`.

## 4d. On the event loop (merge of `claude/userring-eventloop-20261014` 520d6c6b, kf3 `eb9f2aa3`), production

[measured, `marks-run8.log`, one VM life, apps back to back, 2026-10-14 22:53-23:19] 0 host Xid, 0 GSP resets in all six.

| app | before (`c5d9693f` / `a8424ae2`) | event loop `eb9f2aa3` |
|---|---|---|
| Heaven D3D11 windowed 1280x720 | 17.0 fps | **20.7 fps** (261 s pass) |
| Heaven D3D11 fullscreen 1920x1080 | 20.5 fps | **21.5 fps** |
| Heaven OpenGL windowed | 157.1 fps | **252.7 fps** |
| Heaven OpenGL fullscreen | 160.1 fps | **30.0 fps** — a regression; the overlay reads exactly 30 (every other vblank) [measured]; cause not investigated |
| GravityMark D3D12 | 52.9 fps, score 8834 | **121.7 fps, score 20346** |
| GravityMark Vulkan (default route) | 22.8 fps, score 3817 | **84.2 fps, score 14060** |

H-gate (the event-loop branch's "Xid 13 / Edge TDR come from reading ahead through GP_GET"): not separable from this
branch's gapless fix — Heaven D3D11 already ran 0-Xid on `c5d9693f` (old gate, gapless staging), and runs 0-Xid here. Edge:
a 32 min production run on `fc8ba3b8` (old gate + gapless) had 0 Xid / 0 reset but Edge did not migrate its ring in that
window (the 21:44 fault followed Edge's migration 24 min in), so it neither confirms nor refutes either fix for Edge.
Edge playback evidence (owner's question): the 10:34 film was shown playing at 20:21 (opening, frame changes between two
screenshots 5 s apart) and at its end credits at 20:31; on `f163b85e` it was still mid-film 20 min in (playback slower than
real time); later runs prove playback by two screenshots differing in the video area.
The one-command matrix is `scripts/bench/windows/regress_matrix.sh` (per-app guest reboot to capture the event loop's
`USER_RING_LOOP SUMMARY`, dated Markdown table).

## 5. Files

`nvdec_refusal_0c5fedff.txt` (the refused NVDEC segment and the channel's life), `minecraft_xid32_timeline.txt`
(births, migration, RESET_CHANNEL, GSP reset lines; marks). Raw logs stay on the host under `/data/claude-oglw/`
(previous agent) and `/data/claude-urelay/`.
