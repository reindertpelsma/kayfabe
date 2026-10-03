# Decisions waiting on the owner — 2026-10-03

**STATUS: LIVE, 2026-10-03.** Four decisions, each with the facts behind it and a recommendation.
When the owner answers one, the answer goes into `docs/OWNER_RULINGS.md` and the item here is marked
ANSWERED with a pointer. Nothing below is decided yet.

| # | decision | blocks | recommendation |
|---|---|---|---|
| 1 | the sweep and install plan: go-ahead, and Q2–Q8 | the installable binary and every support claim | approve; answers below |
| 2 | `GF100_DISP_SW` for X11 desktops | the stock Mint desktop (its default session is X11) | option A, guarded, with B as the fallback |
| 3 | the archived traces that contain a full VBIOS | nothing technical; a legal liability | scrub the PROM values forward in both public repos |
| 4 | renting a GPU box for display work | every display test, every merge bar | a standing weekly budget, one box at a time |

## 1. The sweep and install plan (`design/V3_SWEEP_AND_INSTALL.md`)

The design has two halves:

- **an unattended sweep** that rents one vast KVM box per GPU family (Turing, Ampere, Ada,
  Blackwell) and runs every chosen host/guest driver pair through kayfabe's existing per-box runner;
- **one installable tarball**: QEMU 10.2.4 with kf3 built in, a launcher (`kayfabe-run`), a
  preflight check and a manifest, built in a pinned container in GitHub CI.

It is a design only, on branch `v3-sweep` (two commits, not merged). It has 22 tasks; 13 of them need
no box. Q1, the licence, was answered on 2026-10-02 (`OWNER_RULINGS.md` §G). Seven questions remain.
The owner's order puts this second, after the display path, so the answers are needed before the
work starts rather than now. Answering early means nothing blocks when it does start.

**Q2 — ship our own QEMU.** A shipped QEMU makes kayfabe responsible for its security fixes: every
10.2.x stable release means a rebuild and a smoke sweep (about $1). The proposed feature set is slirp
on, TPM on, `qemu-img` included, and VNC on a unix socket only.

- *Recommendation: yes, with that feature set.* kf3 is an out-of-tree device, so until the
  mediated-device route exists (`design/V3_VFIO_USER_FRONTEND.md` §2, option 0) a rebuilt QEMU is
  the only install path. VNC on a unix socket only is right because the broker becomes the product
  display path. QEMU's TPM backend needs `swtpm` installed on the host; the preflight check should
  name it when it is missing instead of kayfabe bundling it.

**Q3 — spend and cadence.** Estimates (§1.12): smoke $0.40–1.40, band $5–18, full $9–30 per run.
Proposed cadence: smoke on every merge candidate, band on every release, full on each new NVIDIA
tag, at most 8 boxes at once.

- *Recommendation: accept that cadence.* Add a hard cap of $40 per run (the full tier's high
  estimate plus a third) and a monthly ceiling chosen by the owner; $150 a month until the first
  full run shows real costs. The coordinator should refuse to start a run the remaining monthly
  budget cannot cover.

**Q4 — the band (host/guest version mismatch).**

- *n−1 counts production branches only.* The feature branches 555, 560 and 565 had short lives;
  they stay in the full tier as layout classes, not in the band.
- *A guest newer than its host is required within one branch, and extra across branches.* A guest
  that updates before its host is the common case after a distribution update.
- *Refuse out-of-band pairs by name in code: yes.* An unexplained failure later costs the user more
  than a clear refusal at start.
- *The device refuses, before fn 1, a guest whose queue element it does not serve: yes,* for the
  same reason. It also turns the undeclared lane's FAIL cell into a named refusal.
- *`--guest-driver` stays required in `kayfabe-run`.* Plain QEMU still runs undeclared pairs, but
  `SUPPORT.md` promises only declared ones.

**Q5 — closed kernel modules.** vast's template ships the closed 575.51.03 module; only
provisioning insists on the open one, and no kayfabe code checks it. NVIDIA has installed the open
module by default on Turing and later since the R560 driver.

- *Hosts: open module only for the release.* The preflight check names a closed host module and
  refuses, since no host has been swept with one. Add it as a sweep axis later: vast and older
  installs run it.
- *Guests: add one closed-module cell per family to the band tier.* Users install whatever their
  distribution picks, and the cost is one cell per family.

**Q6 — a non-nested host.** Every vast box is itself a VM, so its guests are nested, and nested
boxes hide host coherency bugs (the 2026-09-26 `CACHE_SNOOP` case). The doorbell-helper decision
(`OWNER_RULINGS.md` §I) waits on the same host.

- *Recommendation: name one physical machine* (the RTX 3050 kiosk PC, or `172.22.1.20` once it is
  reachable) *and give ssh access.* Task H4 never changes its packages or driver, so it can run
  unattended. That one host unblocks both the sweep's non-nested row and the doorbell baseline.

**Q7 — where unattended sweeps run, and the backstop.** The dev host is not durable, and a timer on
it dies with it.

- *Recommendation: both backstops.* (a) Each box powers itself off at its deadline plus a margin,
  relied on only after task H1 shows that a powered-off vast VM stops GPU billing. (b) For full
  sweeps, a second timer on a machine the owner controls. That machine needs a vast API key; if vast
  can issue a key limited to listing and destroying instances, use that one.

**Q8 — what "supported" means.**

- *Refuse out-of-range host tags by name* (615.71.09 is out of range; 545.23.08 does not build on a
  Linux 6.8 host). Accepting them only produces failures nobody can support.
- *Do not refuse in-range tags no sweep has run.* That would lock users out of every new point
  release until a sweep runs. Print "accepted, untested" at start instead.
- *`SUPPORT.md` lists only swept cells as supported,* every other accepted tag as "accepted,
  untested".

## 2. `GF100_DISP_SW` (X11 desktops)

**The problem.** X11 compositors and X11 Vulkan presentation allocate a display-software object
(class `0x9072`) on their 3D channel. Its methods ask for a semaphore release at the next vblank.
Software methods are serviced by the RM of the GPU that runs the channel, which here is the host GPU.
Without a host object, the host raises Xid 32; the 2026-09-30 run `m3c` that offered the object
without one logged 186 host Xid 32 and 1.3 FPS GL. So kayfabe refuses it today (`kf_disp::model`,
`no_display_sw`).

- **What works today:** the Wayland desktops (Mint 22.3's Cinnamon Wayland session, weston), and
  Xorg with the NVIDIA X driver (glxgears vsync-locked).
- **What fails:** Cinnamon's X11 session segfaults in `libnvidia-glcore` and falls back to the
  fallback-mode dialog, and X11 Vulkan fails at `vkCreateSwapchainKHR`.

**The options.**

- **A:** create a host twin of the object with parameters kayfabe chooses (head 0, displayMask 0).
- **B:** keep refusing it.
- The 2026-09-30 note also named "A only with a headless guard" and "a kayfabe-serviced vblank".
  The second has no mechanism: the methods execute on the host GPU, and kayfabe never parses a
  passthrough channel.

**New facts, read on 2026-10-03 in NVIDIA's source** (`ogkm-580: src/nvidia/src/kernel/disp/disp_sw.c`):

- The host object is only a vblank timer. Its methods release a semaphore or notifier at an address
  that RM checks against the calling client's own mappings (`CliGetDmaMappingInfo`, `:146`). The
  twin's client is the guest's own, so a guest can write only into its own memory, at host vblank
  times. It cannot flip, set a mode or change host display state.
- The host allocation needs a display engine (`:69`) and a valid head (`:83`). On GPUs without one,
  such as data-centre parts, it fails, so B stays the fallback there.
- On a host GPU with no monitor, RM runs each vblank callback immediately (`V3_DISPLAY.md` cites
  `vblank.c:87, 209-243`), so X11 would run unthrottled.
- On a host GPU that drives a monitor, the guest's X11 is paced by that monitor's refresh, not by
  kayfabe's virtual 60 Hz. The pacing is wrong when the two differ, but it works.

**Why it matters more than the earlier note said.** Linux Mint's Cinnamon edition logs into X11 by
default; its Wayland session is labelled experimental in Mint 22. With B, a stock Mint guest, which
is the owner's display target, logs into a crashed session. B blocks step 2 of the display order
("a stock guest display with no tweaks") on Mint.

**Recommendation: A, guarded.** Twin the object with authored parameters (head 0, displayMask 0)
when the host allocation succeeds, and refuse it by name (B) when it fails. The guest learns the
host's vblank timing, a minor side channel. kf-rm's rule *"NOT twinned: host RM's dispsw acts on
HOST display heads"* (`crates/kf-rm/src/chanlink.rs`, `alloc_shape`) becomes "twinned with an
authored head; it writes only into the twin's own address space". That is a rule change, hence the
owner's call. The box test: the Cinnamon X11 session starts, X11 `vkcube` presents, there are zero
host Xid 32, and frame rates are recorded.

## 3. The archived traces that contain a full VBIOS

**Found on 2026-10-03** by decoding the files with the repo's own record format
(`archive/nvkvm/scripts/mode2_diag/rec_dump.py` layout; PROM = BAR0 `0x300000`–`0x3fffff`):

- All five traces in `archive/nvkvm/traces/mode2_c_reference/` hold **559 104 bytes of PROM reads:
  one contiguous image from offset 0 to `0x887ff`, starting with the option-ROM signature `55 aa`**.
  That is the complete VBIOS image of the RTX 3060 the C emulator was fed (VBIOS 94.06.25.00.FC; md5
  `48df40a0…` in the README), not fragments.
- Two more copies, uncompressed, sit at `traces/cap1_coldboot_hermetic.rec` and
  `traces/cap1b_coldboot_hermetic_d6.rec` (13–14 MB each). Live tests read them: kf-crec's
  `cap1_differential.rs` and `cap1b_differential.rs` panic when the file is missing (`:67`, `:92`).
- The same five archived files are in the research repository `reindertpelsma/nvkvm`
  (`traces/mode2_c_reference/`), which is public too. kayfabe has one fork.
- The trace headers also carry the rented GPU's UUID and the bench host's kernel string. These are
  harmless, but nothing needs them.
- kayfabe v3 needs no real VBIOS. kf3 generates its ROM from `GENERATED_FWSEC` plus the host's PCI
  identity and VBIOS version (`crates/kf-abi/src/vbios.rs:552-560`).

**Why it matters.** The VBIOS is NVIDIA's copyrighted, signed firmware, and it includes the FWSEC
microcode. The licence grant does not cover it (`LICENSE`, exception 4), but publishing it is
redistribution whatever the licence text says. VBIOS collections such as TechPowerUp's host thousands
of dumps, and NVIDIA is not known to pursue them, so the practical risk is low. It is still a
needless liability in a project that otherwise keeps clear of NVIDIA's IP (no faked vGPU licence,
§H).

**The options.**

- **(a) Keep the traces as they are.**
- **(b) Scrub forward.** Rewrite the seven files with every PROM read value replaced by zero, keep
  every other record byte-identical, and update `MD5SUMS` and the README. Check that kf-crec's
  differential tests still pass; they replay the boot and its RPC replies, and if one compares PROM
  values it would have to compare against kayfabe's generated ROM anyway. Git history still holds
  the old files.
- **(c) Do (b) and also rewrite history** in both public repos (`git filter-repo` plus a force
  push). That breaks every clone. The existing fork and any clones keep the image, and GitHub's
  cached views remain until its support purges them.

**Recommendation: (b) now, in both repositories.** Keep (c) in reserve for a request from NVIDIA: a
force push on a public repository with a fork cannot recall what is already out, and it costs every
clone. Optionally add a CI gate that refuses a committed trace holding a `55 aa` PROM stream. Test
the gate against the old file first, so it can be seen to fire.

## 4. Renting a GPU box for display work

**What needs a box.** kf3 needs a host NVIDIA GPU to realize, so these cannot run locally:

- the GOP option ROM on the real device: OVMF boot screen → efifb/simpledrm → the nvidia-drm
  takeover → Xorg with no BusID;
- the stock guest display;
- `GF100_DISP_SW` option A, if approved;
- the broker end to end;
- the unload tests;
- the managed-memory failure and the virtual-memory-API sample;
- the hardware merge bar every branch needs before master.

**Cost.** One RTX 3060-class vast KVM box costs about $0.15–0.50 an hour, judging by the sweep
estimates (`design/V3_SWEEP_AND_INSTALL.md` §1.12).

**How it is run** (unchanged rules):

- code stays local and the box pulls from GitHub;
- no secrets on the box;
- one box at a time;
- destroyed when idle, by id, and checked with `vastai show instances`.

**Recommendation: a standing budget for the display work** of one box at a time, up to about 8 hours
a day, and $25 a week, reported in each handoff. Until the owner answers, work stays local: the GOP
ROM can be written and tested against OVMF with a stand-in PCI device, and the broker relay's
protocol and pacing logic can be unit-tested without a GPU.
