# The T-space is hardwired: what was deleted, what rule 4 delivers, what hardware must show

**STATUS: LIVE, 2026-10-10. Branch `claude/hardwire-tspace-20261010` (from
`claude/tmode-pieces-20261009` at `c6fff2e3`, merging `claude/window-exposure-review-20261009`).
GPU-free tests only. No hardware has run this revision.** Owner rulings: `docs/OWNER_RULINGS.md`
§AB. It is a follow-up to `V3_FLAG_INVENTORY.md` (branch `claude/flag-inventory-20261009`), whose
`KF3_TSPACE`, `KF3_INCA_REFUSE` and `KF3_KERNEL_*` rows it supersedes. It completes
`V3_P1P2_TSPACE.md` (status corrected there) and answers `V3_WINDOW_EXPOSURE_REVIEW.md` (status
corrected there).

## 1. Code deleted

- `kf_qemu::tspace::enabled` (`KF3_TSPACE`) and `inca_strict` (`KF3_INCA_REFUSE`). The constant
  `tspace::INCA_STRICT = true` replaces the latter.
- The legacy P5 mirror path. `create_mirror` and `prewarm` no longer map a store window or a
  guest-RAM window. The spare-reuse arm no longer reinstalls windows or the ring region.
  `retire_mirror` keeps nothing of ours. `Mirror` and `Spare` lose `fb_base`, `fb_len`, `ram` and
  `rings`, and keep only `negctl_window` (the positive control's window). `vmm_ranges` is deleted.
- The Translated path's dependence on mirrors: the legacy rewriter windows (`chan.rs` `Windows`,
  `SlotWindow`; the pump now passes `NoMirrorWindow`, which T-mode rings never consult) and the
  mirror ring slots (`HostRing::on_engine_at` in a mirror, `give_ring_slot(&mirror.rings, …)`).
  The `tspace_ring` slot flag and the `!tmode` `force_kernel` flip (`TwinState::force_kernel`) are
  deleted. So are the pre-inc-A row cut (`legacy_cut_rows`) and the shadow wiring
  (`tshadow_on`, `shadow_windows`, `negctl_shadow`).
- From the merged review: `DefaultWindows`, `default_windows`, `default_mirror`, `spare_of`,
  `default_spare`, `default_reuse_reserved` and the test
  `known_violation_the_default_path_maps_kayfabe_placements_in_every_mirror`.
- ⚠ Kept, reachable only by tests: `kf-chan`'s non-T-mode rewriter (`TranslatedRing::new`,
  `translated::rewrite_counted`, `tmode::Shadow`) and the count-only arms of `set_inca(false, …)`
  and `heap_gate`. Deleting them is a `kf-chan` refactor with its own test suite; it was not done
  here.

## 2. Flags

| flag | action | why |
|---|---|---|
| `KF3_TSPACE` | **deleted**, behaviour ON | §AB.1 |
| `KF3_INCA_REFUSE` | **deleted**, behaviour ON | it was "always ON with `KF3_TSPACE=1`" (`tspace.rs:49` before) |
| `KF3_KERNEL_NVDEC_CTX`, `KF3_KERNEL_NVENC_CTX`, `KF3_KERNEL_OFA_CTX` | **deleted**, behaviour ON (the Windows profile's value) | They were read only in `birth`, ANDed with `KF3_TSPACE`, and are measured as required to boot Windows (inventory, runs 21-24). The route fails closed: an owned decoder context, and every codec submission refused. **[inferred]** A Linux guest kernel creates no kernel video channel, so Linux is unchanged |
| `KF3_KERNEL_GR_CE` | **kept** (default off; the Windows profile sets it) | Switched on, it changes the route of a channel every Linux boot creates: the guest RM's internal kernel GR channel (the golden-image channel, "not born" today) would become a Translated ring with an owned host GR context. No Linux run with it exists. Without `KF3_KERNEL_GR_WORK` it is a ring that refuses every GR segment. The two need one owner ruling together |
| `KF3_KERNEL_GR_WORK` | **kept** (default off) | Binding: `OWNER_RULINGS.md` §S.2 says "default-off flag until proven". The owner has not ruled it proven |
| `KF3_TSHADOW` | **kept** (measurement): it still turns on the census | Its shadow half **cannot be kept**. The shadow ran the T-mode decoder beside the legacy rewriter, and no ring runs that rewriter any more |
| `KF3_NEGCTL_SHADOW` | **cannot be kept** (its reader is deleted) | It is the shadow's positive control, so it has nothing to control (see `KF3_TSHADOW`) |
| `KF3_NEGCTL_TWIN_WINDOW` | **kept** (control) | The one path that still maps a window into a mirror. It breaks §AB.2 by design: use it only in a dedicated control run (the box-log gate's known positive) |
| `KF3_NEGCTL_CARVE`, `_TWIN`, `_HEAP`, `_STALE_BIND`, `_TSPACE_OVERSIZE`, `KF3_TCENSUS`, every trace | kept, unchanged | owner instruction 2026-10-10 |

`scripts/bench/windows/windows_broker.sh` no longer lists the deleted flags. The older
`pc_*_experiment.py` harnesses still set `KF3_TSPACE`, which is now read nowhere and so harmless.
`scripts/p1p2/tspace_log_gate.py` marks its `--default`, `--census` and `--windows` arms as
reading older logs only.

## 3. Rule 4: what is delivered, what remains

**Today's routes.** A Translated birth happens only for `kernel_client && !user_work`. This is a
guest-kernel channel: the guest RM's `PRIVILEGE = KERNEL` stamp, or one of RM's internal clients
(`kf_rm::chanlink::kernel_channel`), and not a Windows channel the link classified as per-process
user work (OWNER_RULINGS §V, born Passthrough). Every other channel is Passthrough. So no
unprivileged Translated channel exists, and rule 4 holds today **[read]**.

**Delivered (enforced by type, not by route).** `kf_qemu::tspace::Privileged` can be made only by
`Privileged::of(kernel_client, user_work)`. `TSpace::ring` and `TSpace::windows` require one, and
these are the only ways to get a T-space ring or the T-space windows the rewriter binds against.
The Translated birth makes the witness before anything else. Without one it refuses by name
(`tspace::UNPRIVILEGED_TRANSLATED`, counted in `tspace_refused=`). That arm cannot fire today; it
stops a future route (for example the deferred API on a user twin, or a T-mode decoder for
Windows user work) from silently inheriting whole-RAM reach.

**Remains (not built).** The space an unprivileged Translated channel would need: no whole-RAM or
whole-store window, only maps of exactly the bytes its validated operands name.
`TSpaceHost::map_fixed_4k` exists. What is missing is a design that keeps the non-stall rule
(`V3_NONSTALL_THREADS.md`):
- every per-operand map is a host RM call, so it must not run on the worker that pumps the ring.
  It would go to the VA thread as a ticket, as a split does (`Inbox::request_split`), with the ring
  parked on it;
- each map would live until the fence of the last piece that uses it, and be unmapped on the VA
  thread after that;
- the binder (`kf_chan::tmode::push_bound`) would bind against a per-channel piece table instead of
  `TWindows`.
This is a large change, and no channel kind needs it yet. Build it when a route first needs an
unprivileged Translated channel.

**Observation, not a change.** Windows' kernel-stamped per-process VIDEO channels are privileged
by the guest RM's stamp, so they are born Translated in the T-space (with the NVDEC/NVENC/OFA
context). Every codec submission on them is refused, so no guest work reaches the windows.

## 4. Constraints

- **Hostile guest:** nothing new reads guest bytes. The witness comes from facts the guest RM
  stamps, and guest root affecting isolation inside the guest is §A.9's accepted scope. The
  carve-out refusal now also covers guest-kernel mirrors (review row 6), so guest root cannot map
  kayfabe's firmware region into any twin.
- **Unsafe:** none added. `kf-chan` stays `unsafe_code = forbid`.
- **No thread that serves input may stall:** no new lock or wait. Mirror creation and prewarm make
  fewer host calls than the legacy path (no window maps). The T-space build is now on every boot.
  It runs on the VA thread at prewarm, before the guest driver loads; 128.6 ms was measured in run
  14 (inventory). The legacy prewarm also mapped two windows there, but its duration was not
  measured, so whether the VA thread is now deaf for longer is **[unmeasured]**; the status line's
  VA pass maximum shows it. A Translated birth is one ring object, three FIXED maps and a CPU view
  on the act thread, as in every Windows run.
- **Derive, never capture / all families:** the carve-out comes from `kf_chip::bar0::fb_layout`,
  the 40-bit ring bound from the legacy semaphore width, and the windows from host verbs. Nothing
  is per die. The T-space has run on hardware only in the Windows runs (runs 14-103, per the
  inventory). Every family those runs did not use, and every Linux guest, is **[unmeasured]**.

## 5. Hardware verification owed before merge (strictly serial, one box)

1. `scripts/bench/v3_gates.sh` 9/9. The host-side gates must not regress (T-space build, USER
   births).
2. `build_kf3.sh`, rebuild the raw client and the fast guest, then `KF_DEVICE=kf3
   scripts/fastguest/fast_suite.sh <tag> 180`, 30/30. This is the first Linux run where every
   kernel CE channel is T-mode and inc A is strict. Also check the boot log with
   `scripts/p1p2/tspace_log_gate.py LOG` (T-TSPACE-BUILD, WINDOWS=NONE, COUNTERS with
   `carve_kernel=` read, NO-DEAD).
3. Linux broker lane (`broker_lane.sh`, `interactive.sh`): display, interrupts and the drainer run
   with no mirror window (the memory rule for display, interrupt and drainer changes).
4. The CUDA ladder (`cuda_ladder.sh`): UVM's kernel CE channels (vidmem GPFIFO, paging copies,
   `MAX_PIECES`) under strict inc A and the carve-out refusal in kernel spaces.
5. A Windows boot with the production flag set (now without the deleted flags). This confirms the
   hardwiring equals the profile it replaced, and that refusing carve-out leaves in kernel spaces
   does not move a wall.
6. Optional control: one `KF3_NEGCTL_TWIN_WINDOW=1` boot, where the gate's WINDOWS=NONE must FAIL.
