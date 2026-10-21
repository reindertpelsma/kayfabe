# V3 broker — why the later brokers of run 3001 presented through wl_shm, and the fix that is the broker's

**STATUS: ANSWERED, 2026-10-17 (branch `claude/cursor-xor-wire-20261017`).** The broker-side defect is fixed and tested against a real headless compositor; the
kayfabe-side part is described precisely in §4 for the coordinator and is NOT touched here. Evidence labels: **MEASURED** (read from the live logs, or ran it),
**CODE READ**, **INFERRED**.

The question (owner, via the coordinator): the first broker of run 3001 (`/opt/nvkvm-broker`, the 2026-10-08 `badf2d7` build, `broker-20261017-144557.log`) presented through
dma-buf, but both later brokers (`broker-20261017-145100.log`, the previous cursor branch's `a07a9ba0` build; `broker-20261017-145951.log`, the `badf2d7`+ptr_serial TRACE build)
log `presenting through wl_shm: the display refused every dma-buf layout offered`. The goal is zero-copy dma-buf presentation after ANY broker restart.

## 1. What the logs say (MEASURED; host `172.22.1.20`, read-only: the three broker logs and `qemu.log` of `boundary-kayfabe-3001`)

* **The display refused nothing.** In both later logs the sequence is `QUERY_FORMAT XR24 LINEAR -> YES`, `probing whether this display can import XR24 modifier 0x0 (one frame is dropped to find out)`,
  `this display CAN import XR24 modifier 0x0 — proven by import, not by advertisement`, and only then the wl_shm line. There is no `ATTACH ... refused`, no `this display ADVERTISED ... and then refused`,
  no EV_FORMAT x=0 for LINEAR, and the detach lines say `0 rejected`. The words "the display refused every dma-buf layout offered" were wrong: the broker printed them for ANY F_SHM frame.
* **The VMM chose shared memory.** `qemu.log` (kayfabe's relay) has, in every one of the three connections, right after `frames go as a LINEAR dma-buf` and `the display CAN show XR24 LINEAR`
  (lines 80-81, 16994-17005, 20715-20725):
  `dma-buf frames are not acknowledged (3 committed, no RELEASE for 1000 ms): the broker drops a dma-buf it cannot prove ... so frames go as shared memory for 5 s, then the dma-buf rung is tried again (trip 1 / 2 / 3)`,
  and about 5 s later `frames go as a LINEAR dma-buf` again, with no second trip on that connection (lines 112, 17012-17014, 20732-20734). The wl_shm line of the broker is the first F_SHM frame of that 5 s window.
  (The first broker's log has no such line; INFERRED: no frame was sent in its 5 s window. Its connection tripped too, line 112.)
* **The same trip every time, at the connect burst.** "3 committed" in each; `reclaimed frame slot 0: no RELEASE 1000 ms after its commit, and a newer frame was committed` is logged just before the first trip.
* **RELEASEs of a dma-buf frame come only when it is replaced, seconds apart on a quiet guest.** The same log carries `reclaimed frame slot N: no RELEASE 2283 / 1483 / 3983 / 16767 / 17983 ms after its commit, and a newer frame was
  committed` — the compositor holds a dma-buf until a newer frame replaces it.
* A separate, persistent shm cause is in the same log and is NOT a fault: `guest resolution is now 2040x1155` (line 41852) -> `frames go as shared memory (F_SHM)` (41853): a 2040-wide frame has a pitch of 8160 bytes, not a multiple of 256, and kayfabe's own
  rule (`dmabuf_pitch_ok`, `crates/kf-broker/src/conn.rs`: "an unaligned frame goes as shared memory ... instead of as an import the compositor may refuse", from the 2080x1171 measurement) sends it as shm. At 2048x1155 (8192) the dma-buf rung applies.

## 2. Reproduced against a real compositor (MEASURED here, 2026-10-17)

mutter 50.1 `--headless` (software renderer, advertises XR24 LINEAR) + real udmabuf frames + the broker's Wayland backend; a client that does what kayfabe does at a reconnect (a burst of three
frames at once, then nothing), reading the broker's EV_RELEASE events:

| | previous broker (`51967937f`) | this branch |
|---|---|---|
| burst of 3 frames at connect | **no RELEASE for any of the three in the following 3 s** (the probe dropped the first, the ones that arrived while it was out too); nothing on screen until the guest redraws | RELEASE for the first two at once; the third is imported the moment its layout is proven and shown |
| a 4th frame 3 s later | shown; its RELEASE arrives only when a 5th replaces it (`5.170 RELEASE buf#0` at the instant of the next commit) | the 3rd is released when it is replaced (same compositor behaviour: **a dma-buf is released only on replacement**) |

So kayfabe's detector (`Ack`, `DETECT_COMMITS = 3`, `DETECT_MS = 1000`) sees three commits and no RELEASE and trips, deterministically, whenever the guest is quiet for the first second of a connection.

## 3. Cause (the chain; M = measured, I = inferred)

1. (M) A connection's first frame of a layout goes through the broker's asynchronous probe, which drops it; every frame that arrives while the probe is in flight (and any frame of a pair whose slot says `probing`) was dropped too —
   `wl_attach` returned 0 with nothing pending, and the COMMIT answered `-ENOENT`. At a reconnect the burst is dropped whole: **nothing is shown and nothing is acknowledged** until the guest next draws.
2. (M) The compositor releases a dma-buf only on replacement, so on a quiet guest no RELEASE can follow the one frame that did get shown.
3. (M, CODE READ) kayfabe's detector reads "3 commits, no RELEASE for 1000 ms" as a broker that silently drops dma-bufs it cannot prove (`/proc/self/fdinfo`), backs the dma-buf rung off for 5 s (doubling to 60 s)
   and sends F_SHM meanwhile (`Ack`, `choose`, `detect`, `conn.rs`).
4. (I) The guest is quiet during that first second because a reconnect re-sends only the last frame (`re-sent geometry 2048x1155 and the last frame to the new broker`) and the Windows desktop is idle; the burst of 3 is that replay
   plus what the head presents while the window resizes (the broker window starts at 1920x1080 and EV_SURFACE moves it). Not separately measured.
5. (M) The first connection of the run tripped the same way (`trip 1`, line 112): it is not a property of the later brokers, and not of the display being AMD with an NVIDIA-produced frame (the udmabuf is system memory; both GPUs import it).

Candidates considered and ruled out by the above: the guest geometry at attach (2048x1155 is pitch-aligned and was proven by import, `CAN import`); the NVIDIA-modifier refusal (it is the GPU-copy rung, refused before LINEAR is tried: `QUERY_FORMAT ... NVIDIA -> NO`, by design); a state kayfabe keeps across a reconnect (`Conn` is dropped whole, the detector restarts per connection);
the AMD/NVIDIA mismatch (LINEAR import proven).

## 4. What is fixed in the broker, and what is kayfabe's (precisely)

**Fixed here (broker, `nb_session_wl.c`, tested):**

* The newest dma-buf dropped by a probe window is kept (a `dup` of its fd: at most one, so the fd intake bound holds), and **imported and committed the moment its layout is proven**: the first picture after a restart appears at once, not at the guest's next redraw.
* Every older dropped frame is **released immediately** (`EV_RELEASE`, whose contract is "the display is no longer reading that buffer": true of a buffer never imported); a failed probe releases the kept one as well; a client detach drops it silently.
  The RELEASE of the kept, now-shown frame comes on replacement as always, so kayfabe never reuses a buffer that is on screen.
* The wl_shm line no longer lies: the core says once per change of carrier, per connection: `frames now arrive as plain shared memory (F_SHM): the VMM chose this carrier; this display has refused N dma-buf layout(s) on this connection, ...`
  and `frames arrive as dma-buf (zero-copy) again, after shared memory`.
* Tests: `make check-probe` (`test/test_probe.py`, a real headless mutter, skips when mutter/udmabuf are missing): the previous broker fails 5 of its 7 checks, this one passes; `test/test_xor.py::run_tier`: the carrier line, once per change, honest, afresh per connection.
* Effect on the detector, MEASURED in the reproduction: with the burst, the dma-buf class is acknowledged within milliseconds (two RELEASEs), so no trip. With a single replay frame there is only one commit (< `DETECT_COMMITS`), no trip.

**Kayfabe-side (NOT changed here; for the coordinator):**

* `Ack` in `crates/kf-broker/src/conn.rs` (`DETECT_COMMITS = 3`, `DETECT_MS = 1000`, `trip_at`, `committed`, `released`) counts every committed dma-buf frame as evidence of a silent drop until one RELEASE arrives. It cannot tell "dropped by the broker's probe", "shown and held until replaced" and "silently dropped as unprovable". Its documented premise, "a RELEASE names only a buffer the compositor imported", is not what the protocol says (`EV_RELEASE`: no longer reading) and is now not what the broker does for probe-dropped frames.
* The robust fix is on that side, and the broker change above is a second line of defence, not a replacement: count a commit toward a trip only when it is **older than the newest commit** (a frame that has been replaced on screen is expected to have been released: the measured compositor behaviour), and not before the broker has said the pair is proven (EV_FORMAT x=1 after a probe is not sent today; adding that event on the broker side is a protocol addition and needs the owner/coordinator). A quiet guest then never trips.
* Independently, `dmabuf_pitch_ok` sends every frame whose pitch is not a multiple of 256 as shm permanently (the 2040x1155 case). If zero-copy matters at such geometries, the guest's mode choice (the width the broker SUGGESTS is rounded to 64 already, `nb_hint_width`) or an importer that accepts unaligned pitches is the lever; the evidence for the rule is one measurement (2080x1171 refused by an AMD compositor; its comment says INFERRED).
* Not worth a change: the 5 s back-off itself is bounded and self-heals (3 of 3 connections returned to LINEAR); the cost is ~5 s of shm per restart, and a restart under the old broker also shows nothing until the guest redraws.

## 5. Not verified

The owner's compositor (GNOME on AMD, hardware import) behaves like the software mutter in the one fact that matters (dma-buf released on replacement: the live log shows 1-17 s release delays), but the reproduction there was not run; the `probe_failed` release path is CODE READ only; the full run-3001 timeline (how many frames the replay really sent) is inferred from the "3 committed" counter.
