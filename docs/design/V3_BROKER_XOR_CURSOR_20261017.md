# V3 broker — the true XOR cursor, composed by the broker (D2), and the cursor size rule

**STATUS: LIVE, 2026-10-17 (v2, branch `claude/cursor-xor-wire-20261017`).** Three parts: (1) **BUILT, GPU-FREE-TESTED, live look OWED:** the cursor
size rule (§1). (2) **BUILT (2026-10-17, owner chose OPTION A and then refined it, §0a), tested GPU-free and against a real headless mutter (§7-§9), live look OWED:**
the true XOR inversion, composed by the broker from the frame it presents, on the sealed-shm tier AND on a LINEAR system-memory dma-buf, with the
approximation as the standing fallback (the old "NOT built, waits for an owner choice" text of §2-§5 is SUPERSEDED in place below). (3) The deletion of
kayfabe's own cursor composing (§6) is still a separate change. Binding context: `OWNER_RULINGS.md` §AJ (2026-10-17: kayfabe composes no cursor; the
broker owns the whole cursor plane including true inversion).

Evidence labels: **MEASURED** (ran it / read the live log), **CODE READ**, **INFERRED**.

## 0. Decisions in force

* D1 (settled earlier): the broker owns the whole cursor plane, kayfabe is mode-free (`CAP_CURSOR_STREAM`, atomic MOVE pair, hover hot-spot latch).
* D2 (2026-10-17): an XOR cursor (the Windows I-beam) is a TRUE inversion of what is under it, composed by the broker. Kayfabe keeps sending the
  inversion mask with `CURSOR_F_XOR` and composes nothing. The approximation (`nb_cursor_xor_approx`: black fill, white outline) is a stopgap.
* D3 (2026-10-17): the cursor is oversized on the owner's screen (§1).
* Owner ruling (2026-10-17, `OWNER_RULINGS.md` §AJ): kayfabe-side cursor composing is to be DELETED, not kept as a fallback (list in §6).

### 0a. Owner refinement of D2 (2026-10-17, relayed by the coordinator; binding)

The owner PREFERS TO AVOID the wl_shm tier: no unnecessary CPU copy of large frames anywhere in the present path, and GPU APIs where composing is better done
there. Consequences built here: **Option A does not depend on the shm tier** (it reads the cursor's rows from a sealed memfd OR from a mmap'ed LINEAR dma-buf bracketed by
`DMA_BUF_IOCTL_SYNC`, never the frame: a few bytes per update); a tiled or device-memory dma-buf, or any failed mapping, falls back to the approximation with one
log line per change of choice. A GPU path was evaluated and is NOT built (§8, with the reason and the conditions under which it becomes right). The investigation
the owner asked for into why the later brokers of run 3001 presented through wl_shm is `V3_BROKER_PRESENT_TIER_20261017.md` (cause: not a refusal; fixed in the broker
where the fix is the broker's).

## 1. The oversize (task 1)

### 1.1 What the log and the code say

* **MEASURED** (live `broker-20261017-145100.log`, run 3001): `cursor: host pointer image 239x239 (buffer 359x359), hot spot 0,0 of the guest's 0,0,
  frame fit 1915/2048, output scale 180/120`; later, after the window grew, `306x306 (buffer 459x459) ... frame fit 2446/2048`. The cursor image is
  256x256. Hot spots latched: `14,15` (most often), `24,24` (twice-seen), also `12,13`, `9,15`, `11,15`, `7,17`, `0,12`.
* **CODE READ** (before the change): `gc_geom_now()`: logical `g` = `nb_cursor_scaled_geom(image, fit)` = `256 * 1915/2048` = **239**; device `b` =
  `nb_cursor_device_geom(..., fit * scale_120/120)` = 359; the viewport destination of the cursor surface was set to `g` (239) and the buffer scale
  was never set (1). So the formula was already the first-principles rule, and the logged numbers equal it.

### 1.2 The rule (derived, not guessed)

A guest cursor pixel is one GUEST frame pixel. The frame is shown at `fit = fit_w / buf_w` logical (surface-local) pixels per guest pixel.
So the cursor's surface-local size is `image_dim * fit` and **the output scale is not a factor of it**. Protocol text (installed
`wayland.xml`, `viewporter.xml`): `wl_pointer.set_cursor` — the hot spot is "in surface-local coordinates"; a surface's size is its buffer size
divided by `wl_surface.set_buffer_scale`, or the `wp_viewport` destination when one is set. A fractional output scale (`wp_fractional_scale_v1`,
units of 1/120) therefore only decides how many BUFFER pixels carry those logical pixels.

Computed example (guest 2048x1155, window frame fit 1915/2048, output scale 1.5): logical = 256 * 1915/2048 = 239.4 -> **239**; on screen that is
239 * 1.5 = 358.5 physical pixels = 256 guest px * 1.4004 physical px per guest px, i.e. exactly the picture's own magnification. Buffer
= 239 * ceil(1.5) = 478 at `buffer_scale 2`.

### 1.3 Hypotheses and verdicts (falsifier stated before reading the size path)

| id | hypothesis | falsifier | verdict |
|---|---|---|---|
| H-a | frame-fit and output scale multiplied on an already-scaled image | logical `g` equals `256*fit` and the destination is `g`, not `b` | **FALSIFIED** (CODE READ + MEASURED: 239 = 256*1915/2048, destination 239). Double scaling of the LOGICAL size cannot happen; a regression test now pins it (`the output scale changed the logical size`). |
| H-b | scaling by the 256 buffer instead of the visible extent | `g == image_dim * fit` | **FALSIFIED as a defect**: the 256x256 is the guest's own image; the arrow's padding is the guest's data, one guest px = one frame px. (The visible arrow is small inside it; that is correct.) |
| H-c | the compositor does not apply the `wp_viewport` destination to a cursor-role surface, so the 359 px buffer shows as 359 logical px = **exactly 1.5x** `g` | the owner looks at the build below and the cursor is still too big | **UNDECIDED, the only mechanism left that yields exactly the output scale.** INFERRED, not measured here (this box has mutter 50.1 but no pointer device to drive a cursor surface, so it was not tried; the owner's compositor version is unknown). |
| H-d | the guest itself draws a large cursor | hot `24,24` latched twice => a 48x48 sprite if it is a centred resize cursor (INFERRED); frame shown 1.40 physical px per guest px | **PARTLY TRUE (INFERRED from the latched hot spots: 48 px = Windows at 150% scaling)**: the guest's 2048-wide desktop is shown in 2872 physical px, so every guest element, the 48 px cursor included (~67 physical px, ~45 logical), is ~1.9x a native GNOME pointer (24 logical). That is faithful, not a broker defect. Nothing to fix in the broker for that part; making the guest's cursor smaller is a guest setting (Windows pointer size) or a future owner-chosen cursor-size policy. |

### 1.4 The change (commit "cursor surface size rule extracted")

* `nb_cursor_surface_geom()` (`broker/nb_cursor.c`, pure): logical = frame fit alone; `buffer_scale = ceil(scale_120/120)` clamped 1..4; buffer = logical x
  scale, bounded by `NB_CURSOR_SURFACE_MAX_BUF` (1024). Hover (the host pointer surface) and grab (the overlay subsurface) call the SAME function.
* No `wp_viewport` on the cursor surfaces any more: `wl_surface.set_buffer_scale(k)` is the path every toolkit uses for HiDPI cursors and does
  not depend on the compositor honouring a viewport on a cursor-role surface. Cost: at a fractional scale the compositor resamples the buffer
  from k to scale (as before from the viewport).
* The log line now says `buffer_scale N`.
* Tests (`broker/test/test_cursor.c`, `test_surface_geom`): a table over fit < 1 / = 1 / > 1 x scale 1.0 / 1.25 / 1.5 / 2.0 / unknown / below 1, the owner's
  1915/2048 @ 1.5 and 2446/2048 @ 1.5, non-square, resize (125 -> 250), the no-double-scaling invariant, 40000 hostile inputs (buffer = logical x scale,
  hot spot inside, bounded). Mutation check (each mutant fails): fold the output scale into the logical size; `k = 1`; buffer not x `k`; floor instead of ceil;
  clamp ignoring `k`.
* **Not verifiable without the owner's screen:** whether H-c was the cause. See the report's look-list.

## 2. Facts that constrain a true inversion

* (a) **A Wayland pointer cursor or a subsurface cannot invert what is beneath it.** Core surfaces blend with premultiplied alpha only
  (`out = src + dst * (1 - alpha)`): no negative coefficient, so `255 - dst` is unreachable; no protocol offers a difference/xor blend
  (`wp_alpha_modifier` scales alpha only). CODE READ + protocol XML. X11 core has `GXxor`/`GXinvert` GCs, but see X11 below.
* (b) **Therefore** a true inversion means someone computes `below ^ colour` from the frame's pixels: the broker (this ruling), in a compose pass or an
  equivalent, and presents the result.
* **The broker has no pixel access today (CODE READ):** its Makefile states "no libdrm, no EGL, no GL, no libgbm"; a frame reaches the screen as a
  dma-buf the COMPOSITOR imports, or (shm tier) as a memfd the compositor maps (`wl_shm_pool` on the client's fd). The broker never reads a frame.
  That is its security property (`broker/README.md`): "the VMM never touches the display server".
* (c) ⊘ **CORRECTED 2026-10-17:** the wl_shm line below was the VMM's choice after a false trip of its acknowledgement detector, not a display refusal; `V3_BROKER_PRESENT_TIER_20261017.md`. **What the frames are, on the owner's setup (MEASURED, run-3001 log):** the compositor is an AMD GPU's GNOME (24 advertised pairs: implicit, AMD
  tiled, LINEAR). `QUERY_FORMAT XR24 NVIDIA-modifier -> NO`, `XR24 LINEAR -> YES` (import proven), and then `presenting through wl_shm: the display refused
  every dma-buf layout offered`, `Present: COPY`. So the actual tier was **wl_shm: the frame is a memfd** (sealed `F_SEAL_SHRINK`, enforced in
  `nb_validate_desc`, because the X11 backend and the compositor map it; mapping it read-only is already inside the broker's threat model).
  On a dma-buf tier (LINEAR or the NVIDIA modifier) CPU access means `mmap` of a dma-buf: possible for LINEAR system memory, **untested** for the
  NVIDIA VRAM export, impossible for tiled; reads of VRAM are slow (uncached, PCIe).
* (d) Cursor memfds are read with `pread`, never `mmap` (SIGBUS in the privileged process). A sealed frame memfd may be mapped; a dma-buf cannot be `pread`.
* How hover draws today (CODE READ): the host pointer is the compositor's own pointer with `wl_pointer.set_cursor(gc_surf)`; the compositor moves it with zero
  latency, and the broker neither knows nor needs the pixels below. Grab draws an overlay subsurface at the last MOVE.
* The old kayfabe path (retired by §AJ) did this on the GPU: `out = below ^ (src & 0x00ffffff)`, hiding the host pointer while an XOR cursor was active. Its two
  failure causes were the XOR <-> ARGB switch across two processes (arrow before the I-beam left the frame), not the inversion itself.

## 3. Options (all keep kayfabe free of cursor composition)

| # | mechanism | cost / frame | hover | grab | X11 | failure modes |
|---|---|---|---|---|---|---|
| A | **region patch overlay (recommended).** On each committed frame, while an XOR cursor is up, read ONLY the mask's pixels from the frame (sealed shm memfd mapped read-only), compute `0xff000000 \| (below ^ colour)` (`nb_cursor_xor_patch`, built) and show it as the existing overlay subsurface, committed together with the frame (sync subsurface). | mask-bounding-box pixels: an I-beam ~7x16 = 112 px; a few microseconds; nothing while no XOR cursor | host pointer HIDDEN while XOR is up; overlay at the broker's own pointer position (a software cursor: ~1 frame of lag while moving; no kayfabe round trip, no cross-process switch) | overlay at the last MOVE (as now) | not advertised; unchanged | sub-pixel registration: the overlay is placed at an integer logical position and scaled by the compositor, so up to 0.5 logical px off the frame's grid when fit != 1 (inversion is affine, so a scaled patch equals the scaled inversion up to this offset); stale for one frame if the overlay commit is not tied to the frame commit; dma-buf tiers: not mappable -> approximation fallback |
| B | **bake into a broker-owned copy.** Copy the frame into a broker shm buffer with the cursor XORed in (`nb_cursor_xor_apply`, built) and present THAT while an XOR cursor is up. | whole frame: 2048x1155x4 = 9.5 MB per frame, ~1-2 ms CPU at 60 Hz (~10% of a core), plus a second shm buffer set and release tracking | same hide-host-pointer rule as A | exact registration | no | the broker becomes a frame copier for hostile memory; doubles memory traffic in the shm tier (the compositor already uploads once); latency of a frame; dma-buf tiers as A |
| C | **GL / Vulkan pass in the broker.** Import the dma-buf, draw the XOR, present. | one GPU pass; exact | works | works | no | adds EGL/Vulkan + libdrm/gbm to the privileged broker (its stated property), the NVIDIA-export import from a second GPU stack is untested, and a hostile dma-buf now reaches a GPU driver in the privileged process. Not contained; owner decision |
| D | **X11 `GXxor` fills** drawn after the Present completes. | tiny | n/a | n/a | the only X11 route: draw into the WINDOW after the PresentComplete | flicker between the present and the fill; Present COPY with vsync races the fill; the backend does not stream cursors today. Not recommended |
| E | **keep the approximation** (black fill, white outline) | none | works | works | n/a | not an inversion; D2 says it is not enough. The standing fallback (§4) |

Option "reuse that semantics, composed in the broker" is A and B. The semantics are the retired kernel's: **`out = below ^ (mask & 0x00ffffff)`, the X/alpha byte
untouched, a clear mask pixel never changes anything, an empty mask is nothing.** Oracle for the tests, reusable as is:
`kf_disp::scanout::compose_reference` (XOR branch), `crates/kf-disp/tests/compose_kernel.rs` (XOR cases), the doc `V3_DISPLAY_CURSOR_FORMATS.md` §3/§4,
engine test `an_a1r5g5b5_xor_cursor_is_scanned_as_one`, and the kf-broker `host_cursor.rs` `xor_mask` I-beam shape.

## 4. Recommendation

**A, shm tier first, the approximation everywhere else.** Reasons: it needs only a read-only map of a buffer the broker's own validator already seals, it costs
microseconds, it leaves the compositor's pointer path alone whenever the cursor is not an XOR one, and it keeps kayfabe out of composing. B is the
fallback if the owner dislikes the sub-pixel offset; C needs an owner decision on the broker's no-GPU-stack property.

**SUPERSEDED 2026-10-17 — BUILT, see §7.** (The paragraph below is the reasoning of the first agent, kept because its four concerns are exactly what §7's bounds and §9's live checks answer.) **Why it was not implemented in the first pass** (the brief's containment test): (1) it keeps a mapping of each shm frame in the broker (new lifetime, SIGBUS and
bounds rules in the privileged process) and a per-commit hook synchronised with the frame; (2) hover must switch from the compositor's pointer to a software overlay
while an XOR cursor is up (pointer tracking in the broker, hiding and restoring the host pointer: the surface where the old bug lived); (3) it spans the core (the
mask must be kept next to the approximation, `nb_cursor_xor_approx` runs at SET time and discards it), the Wayland backend and the test backend: roughly 350+ lines in
a 5000-line file; (4) nothing here exercises it: no compositor with a pointer, no screen; a defect would regress the live viewer's cursor, which works. So the decision
points are the owner's: A vs B vs C, and accepting a mapping in the broker.

**Built (step 1 of A and B, dead code until wired):** `nb_cursor_xor_apply` / `nb_cursor_xor_patch` in `broker/nb_cursor.c`, tests `test_xor_compose`
(table: invert under white / black / colour, per-channel colour, top byte kept, alpha 0 transparent, empty mask nothing, involution, clipping on all four edges
and extreme positions, stride, hot spot irrelevant, a scaled cursor equals the scaled composite, the patch never writes the frame, and a 30000-case
differential test against the kernel formula). Mutation check: 8 mutants (OR for XOR; the alpha byte XORed in; selecting on colour not alpha; right edge off by
one; no left clip; patch not inverted; patch not opaque; stride check dropped), each fails.

**Steps to finish A — ALL DONE 2026-10-17, as built in §7 (i: `nb_cursor_set_xor`; ii: `nb_frame.c`, opened where the buffer is accepted, not lazily, because the fd is borrowed; iii: `ov_refresh`/`ov_draw_xor` before the frame's parent commit, log line `nb_xor_choice_line`; iv: `nb_xor_decide`; v: counters live in `nb_xor_ctx`, logged never per frame):** (i) keep the mask beside the approximation (`struct nb_cursor` gets `xor` and a second pixel set; `nb_cursor_xor_approx` stops overwriting);
(ii) Wayland: keep a read-only mapping per shm `nb_wl_buf` (mapped lazily, unmapped in `wl_buf_destroy`; sealed, so bounded by `stride*height+offset`); (iii) at commit time, when
an XOR cursor is visible and the pending buffer is mappable, build the patch with `nb_cursor_xor_patch`, attach it to the overlay (sync subsurface, `buffer_scale` rule of §1),
and log `cursor: XOR composed by the broker` once; when it is not (dma-buf tier, unmappable), log `cursor: XOR approximated (frame not readable: <tier>)` once and keep today's
behaviour; (iv) hover while composing: `set_cursor(NULL)` and place the overlay from `wl_pointer.motion`; restore on leaving; (v) a counter in the status/log line
(`xor_composed`, `xor_approx`), never per frame. Test plan: the pure tests above; a test-backend case that feeds a frame and checks the patch hash; the selftest/testclient
unchanged; live: the owner's screen (below).

## 5. What the owner should look at (SUPERSEDED by the live-check list in §9)

See the final report; in short: the cursor size in hover and in grab after this build (should match each other and the picture's magnification), and, once A or B
is built, the I-beam over text of different backgrounds.

## 6. Deletion list: kayfabe-side cursor composing (NOT done here; a separate change, with its own tests and a Linux broker retest)

Grep-level inventory at this commit (the deleting change must re-derive it). After it, **a broker without `CAP_CURSOR_STREAM` shows the guest's own cursor
(what the guest draws into its frame in software) or none — kayfabe never composes a cursor.** The guest's own software cursor is not kayfabe's.

* **GPU compose kernel and its plumbing:** `cuda/display/kf_scanout.ptx` flags bit3 (XOR) and the cursor layers it serves; `kf_cuda::display` `ComposeLayer` cursor use,
  `cursor_upload`, `compose_cursor` (`crates/kf-cuda/src/display.rs` ~871-903); `COMPOSE_XOR` and its compile-time pin in `crates/kf-qemu/src/display.rs` (~4084);
  the bring-up GPU self-test's XOR pixel.
* **kf-disp (`crates/kf-disp/src/scanout.rs`):** `plan_cursor`, `plan_cursor_any`, `plan_cursor_bpp`, `CursorExpand` (A1R5G5B5 expansion for the composer), the `COMPOSE_*`
  cursor flags, `compose_reference` for cursor use; **keep** `plan_host_cursor`, `plan_host_cursor_xor`, `xor_mask` and the host-cursor reads (the stream still needs them), and
  `cursor_composition` / `cursor_debug_*` (diagnostics). Tests: `crates/kf-disp/tests/compose_kernel.rs` XOR/cursor cases; the A1R5G5B5 compose tests
  (`an_a1r5g5b5_blend_cursor_expands_to_a_32_bit_premultiplied_layer`, `an_a1r5g5b5_xor_cursor_inverts_under_white_and_an_empty_one_is_nothing`,
  `an_opaque_a1r5g5b5_composition_keeps_every_pixel`, `an_a1r5g5b5_cursor_is_clipped_like_an_argb_one`, `an_a1r5g5b5_cursor_is_bounded_like_every_cursor`) — move the
  value assertions into the broker's `test_xor_compose` oracle first.
* **kf-qemu (`crates/kf-qemu/src/display.rs`, `broker.rs`, `device.rs`, `ffi_unsafe.rs`):** the per-pass cursor composition step, the "host cursor COMPOSED" / "REFUSED" /
  "scanout REFUSED cursor" counters and log lines (`host_cursor_refused`, `host_cursor_reads` as compose evidence), hiding the host pointer while an XOR cursor is composed,
  `HotTracker` use (display.rs ~35, 3946, 4177), `ConsoleCursor` use (QEMU console cursor sink).
* **kf-broker (`crates/kf-broker/src/{cursor,conn,console,lib}.rs`):** `CursorMode` (`Off`/`Grabbed`/`Hover`), `CursorMode::composes`, `CursorMode::reads` as the composing switch, free fn
  `composes`, `CursorWant::Composed`, the `Unshowable -> Composed` arm of `want_of`, `HotTracker` (the stream sends the programmed hot spot), the non-stream arm of the relay's
  cursor sync (`frame_composed`, `owed`, the "ONE cursor" rule), `ConsoleCursor` (console.rs: the "COMPOSED" lines ~477-614). Tests: `crates/kf-broker/tests/host_cursor.rs`
  composing/convergence models (keep those about the stream), `proto_mirror.rs` mirrors of removed items.
* **Flags:** `KF3_DEBUG_CURSOR` (display.rs ~3845) is diagnostic, keep or retire with the owner; check `docs/design/V3_FLAG_INVENTORY.md` for any composing flag.
* **Protocol bits that become dead:** none in the shared header — `CAP_CURSOR_STREAM` becomes mandatory for any cursor; a broker without it gets no cursor from kayfabe.
  (The old bit-12 broker generations are already incompatible.)
* **Also affected:** QMP `screendump` / the QEMU console and VNC show the composed frame today, so they lose the cursor; the broker control CLI `screendump` (roadmap B) composes
  the cursor from the broker's state, and should land before or with the deletion.
* **Scripts/docs:** `scripts/bench/display/broker_lane.sh` assertions about composed cursors; `V3_DISPLAY.md` §8.12, `V3_DISPLAY_CURSOR_FORMATS.md` §4–§7 (mark SUPERSEDED in place).
* `docs/RELEASE_CLEANUP_LEDGER.md` does not exist on this base; the task is recorded here and in `OWNER_RULINGS.md` §AJ.

## 7. As built (2026-10-17, branch `claude/cursor-xor-wire-20261017`)

Evidence labels as above. **CODE READ** = read in the source; **MEASURED** = ran it (unit tests, a real headless mutter 50.1 with a software renderer on this box, a
real udmabuf); **INFERRED** = reasoned, not run. The Wayland-specific code is kept thin; every decision is a pure function behind a test.

### 7.1 The pieces

| piece | where | what |
|---|---|---|
| the record | `nb_cursor.c` `nb_cursor_set_xor` | an XOR SET keeps the **mask** (`c->mask`, `c->xor`, its bounding box `bx,by,bw,bh`) beside the approximation (`c->px`, unchanged for every backend that cannot invert; X11 is untouched). `nb_cursor_load` clears `xor`, a published copy carries both. |
| the frame reader | `nb_frame.c` | a read-only window onto the frame being presented: a **sealed memfd** (`F_SEAL_SHRINK` re-proven), or a **LINEAR dma-buf whose exporter is system memory** (`udmabuf`, dma-heap `system`; read from `/proc/self/fdinfo`, allowlist in `nb_frame_exporter_ok`): `mmap(PROT_READ)` of exactly the described extent, every read bracketed by `DMA_BUF_IOCTL_SYNC` (START/END, READ). Opened where the buffer is accepted (`wl_frame_open`), closed with it and on client detach. |
| the decision | `nb_xor.c` `nb_xor_decide` | **pure**: (cursor none/plain/XOR) x (hover/grab) x (frame readable) x (pointer inside) x (enter serial present) x (placed) -> what the host pointer does (untouched / the guest's image / hidden), what the overlay shows (off / the approximation image / the true patch), and the standing choice (none / plain / composed / approximated). 96 rows. |
| the driver | `nb_xor.c` `nb_xor_eval` | gathers the view, decides, reads ONLY the cursor's rows (its bounding box clipped to the frame), composes `0xff000000 \| (below ^ colour)` where the mask selects and the pixel is on the frame, **re-decides if the read fails**, reports `patch_changed` against what the display holds, and `choice_changed` for the one log line. Shared by the Wayland and the test backend. |
| placement | `nb_xor_place`, `nb_xor_ptr_to_frame` | pure: patch edges to logical pixels; host pointer to the guest pixel under it. |
| Wayland | `nb_session_wl.c` | `xo_eval` (gather), `cur_apply`/`gc_refresh_ptr` (obey `ptr`), `ov_refresh`/`ov_draw_xor` (obey `ov`), `ptr_enter/motion/leave` (position), `wl_commit` (the patch for THIS frame, attached just before the parent commit). |

### 7.2 Keeping the patch in step with its frame (the question the brief asked to be stated precisely)

* **The patch is a function of (the frame buffer being shown, the position)** and nothing else is stored about it. `nb_xor_eval` recomputes it from the mapping whenever either changes.
* **At a frame commit** (`wl_commit`): the XOR patch is computed from the NEW frame (`ov_refresh(w, sl, true)`), attached to the overlay surface, which is a **synchronised
  subsurface while the choice is "composed"** (its commit is cached, applied by the parent's NEXT commit), and then `wl_surface_commit(w->surf)` applies frame and patch
  **together**. MEASURED on mutter: after a new frame with different pixels under the pointer, every one of the 21 patch pixels in the screenshot is the inversion of the NEW
  frame and the pixels beside the patch are the new frame (`scratchpad` screenshot check, 21/21).
* **Between frames** a pointer move recomputes the patch from the frame that is showing, at the new position, and commits overlay + parent together. Reads cost 1-2 microseconds (§8.1).
* **Pointer faster than frames**: each motion event recomputes from the frame on screen; a patch is only given a new `wl_buffer` when its pixels or size changed (a position change alone is
  a `set_position` and a commit), so an I-beam crossing flat text costs no buffers. Two patch buffers rotate; **if both are still held by the compositor the previous patch stays up and the
  release re-runs the update** (`ov_starved`): the display never shows a patch drawn into a buffer the compositor may be reading. Consequence, stated: while the pixels under the cursor change
  AND both buffers are held, the patch can be one or two frames old (it is the inversion of a slightly older frame); it never shows a patch of a *different position than the pointer it
  was last given*. MEASURED: 60 pointer positions in one burst ended with the patch exactly at the last position (test backend, same driver); on mutter 12 steps ended at the last.
* **Registration**: the overlay sits at an integer logical position and is scaled by the compositor, the frame by its viewport. Each patch **edge** is rounded to the nearest logical pixel on its own
  (`nb_xor_place`), so each edge is within **0.5 logical pixel** of the frame's own pixel boundary, exact at fit 1. PROVEN over a sweep of 10 fits x 9 output scales x ~390 positions (rational
  arithmetic, `|x*dx - tl*nx|*2 <= dx`) and hostile inputs. The scaled patch is the box-averaged inversion where it shrinks (so a 1-pixel stroke becomes partial coverage, as the frame's own
  text does) and nearest where it grows (crisp). MEASURED on mutter at fit 1, 2.0 and 0.625 (screenshots: clean inversion, 2x I-beam, thinner lighter I-beam).
* **Output scale**: `buffer_scale = ceil(scale_120/120)` exactly as the cursor surfaces (§1), no viewport. NOT exercised on a fractional-scale compositor here (mutter headless is scale 1); the size
  arithmetic is the tested pure function.

### 7.3 Hover, grab, and the host pointer (the place the old bug lived)

* The host pointer's state is `nb_xor_decide(...).ptr`, applied by `cur_apply` and `gc_refresh_ptr`: **hidden while the true patch is composed, the guest's image otherwise, untouched when the pointer
  is not ours** (on the title bar, outside, or no enter serial). **Restored the moment the XOR cursor is not what is composed**: a new plain SET, HIDE, an unreadable frame, a dialog, a grab. The whole
  table, the "never two cursors" invariants (a hover overlay always comes with the host pointer hidden; a guest image on the host pointer never comes with an overlay), and the transitions
  (I-beam -> arrow, composed -> unreadable, HIDE, hover <-> grab) are `test_state_table`.
* MEASURED on mutter with the screencast carrying the host pointer (`cursor-mode embedded`): (a) I-beam composed: only the I-beam, no host pointer; (b) a plain 32x32 red/green cursor SET: that cursor
  is the host pointer, the I-beam is gone; (c) the XOR cursor SET again: host pointer hidden, I-beam back.
* **Grab** places the same patch at the guest's own position (`c->x, c->y`, the last MOVE, clipped); an unreadable frame draws the approximation image as before. NOT exercised on a real compositor: mutter
  headless did not deliver keyboard focus to the window, so CTRL+ALT+G never fired; grab uses the same `ov_draw_xor` as hover with a different top-left, covered by the test backend.

### 7.4 Hostile-guest bounds

Guest/VMM-controlled inputs: the frame description, the frame bytes, the mask, the MOVE position, the SET hot spot. CODE READ + TESTED:

* a description that does not fit the measured extent is refused before mapping (`NB_FW_GEOMETRY`, 64-bit arithmetic: huge stride, wrap, zero, > max dim, offset past the end); the mapping is exactly the
  needed extent (a sparse 4 GiB memfd is not mapped whole; a dma-buf mapping may not exceed its buffer);
* **per frame** the description is re-checked against the mapping (`nb_frame_check`: any change of size/stride/offset/format is `STALE` -> approximation), and every row end is checked against the
  mapping length in 64 bits before it is copied; a test builds a frame whose description outruns a one-page mapping followed by a guard page and the read is refused, not performed (3 probes, each in a child);
* the rectangle is the cursor's bounding box clipped to the frame, at most 256x256 pixels (<= 256 KiB, **MEASURED 135 us worst case** per evaluation for a full-size hostile mask: 8% of a core at a
  1 kHz pointer; typical I-beam 1.3 us);
* a **sealed memfd cannot be truncated under the reader** (`ftruncate` answers EPERM, tested), so a read inside the mapping cannot SIGBUS; a dma-buf cannot change size; the mapping is `PROT_READ`
  (`/proc/self/maps` says `r--s`, and a store faults: tested for memfd and udmabuf);
* an exporter that is not known to be RAM, a tiled modifier, an unreadable `fdinfo`, a failed `mmap`/`dup`, a failed sync ioctl: all `not readable` with a named reason -> the approximation;
* nothing here writes the frame; a failed read re-decides in the same evaluation (`mutant #27`).

### 7.5 Carriers, and what a CPU read costs (the owner's question; MEASURED on this box, kernel 7.0, `/dev/udmabuf`)

| carrier | readable? | cost |
|---|---|---|
| sealed memfd (wl_shm) | yes | `0.47 us` per evaluation (16x16 mask) |
| LINEAR dma-buf, exporter `udmabuf` / `system` (kayfabe's rung 1: the udmabuf of its host-memory backing) | yes | the pages are ordinary cached RAM: **`1.25 us`** per evaluation incl. both sync ioctls (`0.75 us` for sync + 16 rows alone, `0.05 us` without the ioctls). **First touch of each NEW buffer: ~0.9 ms once** (81 us building the udmabuf scatter table on the first `SYNC_START` + the page faults), at most 8 buffers per connection. |
| LINEAR dma-buf, GPU exporter (`amdgpu`, `i915`, `nvidia`) | **refused by policy** | a read of device memory is uncached across PCIe. INFERRED, not measured here: on the order of a round trip per 4-64 bytes, i.e. ~0.1-0.2 ms for an I-beam and ~65 ms for a hostile 256x256 mask. **Option A is the wrong tool for that case**, so the exporter allowlist is the whole policy (no timing heuristics): `NB_FW_EXPORTER`, the approximation, one line. |
| tiled / block-linear (the native VRAM rung, AMD tiled) | no | `NB_FW_MODIFIER`. |

### 7.6 Tests (all GPU-free; the owner's ASan/UBSan target green)

| suite | what | result |
|---|---|---|
| `test/test_xor.c` | the 96-row decision table vs an independently written oracle + 7 invariants x 96 + 9 named rows + 6 transitions; placement sweep; pointer map; fdinfo/allowlist/layout helpers; the frame reader on a sealed memfd (open, per-frame check, bounded reads, resize/stale, unsealed, pipe, mapping failure by `RLIMIT_AS`, guard page, read-only); the same on a real udmabuf; the driver against the kernel formula (`below ^ (w & 0x00ffffff)`: white, black, colour, alpha 0, all four edges, off the frame, grab, a resize, an unreadable frame with ONE choice change in 505 evaluations, a failing sync -> fallback, a random 3000-case differential); the box scaler; the cursor record | **80752 checks, 0 failed** |
| `test/test_xor.py` | the real socket + ATTACH validator + `--backend test` (the same `nb_xor_eval` the Wayland backend runs): choice line once, hover patch hash recomputed in Python from the frame bytes, pointer step, same position (nothing), 60 positions between frames, new frame, identical frame, edges, resize, unreadable frame (ONE line, then quiet), readable again, I-beam <-> arrow, HIDE/SHOW, pointer outside, grab and its end, a LINEAR **udmabuf** frame ("exporter udmabuf"), the carrier line per change | **49 checks, 0 failed** |
| `test/mutate_xor.py` (in `make check`) | 29 mutants, each in a COPY of the sources, each must fail >= 2 distinct checks | **29/29 killed** (table below) |
| `test/test_probe.py` (`make check-probe`, needs mutter + udmabuf, skips otherwise) | the probe-window fix of `V3_BROKER_PRESENT_TIER_20261017.md` against a real headless mutter | **7 checks pass; the previous broker fails 5 of them** |
| existing | `test-cursor` 66138 checks, `test-geom` 65431, selftest 82, adopted-socket, clipboard, lifecycle, `test_cursor.py` | all unchanged and green |
| `make check`, `make check-sanitize` | whole tree | **exit 0 both** |

Mutation table (mutant -> failing checks / distinct): hover XOR leaves the host pointer visible 10/7; hover readable draws no overlay 29/20; the enter serial ignored 49/12; pointer-outside ignored 49/12; an unreadable
frame hides the pointer 12/7; grab with a readable frame still draws the approximation 14/12; grab draws before the first MOVE 17/7; an arrow in hover is not the host pointer 6/3; no cursor leaves the host pointer
alone 6/3; the choice is composed when unreadable 28/13; the choice announced every evaluation 8/6; placement truncates 11881/3; the right edge not rounded on its own 3627/2; the pointer map does not clamp 3/3;
a memfd without the seal is read 10/6; the row end unchecked 3/3; the rectangle may run past the frame 3/3; the per-frame description check skipped 7/7; any exporter accepted 3/3; a tiled modifier accepted 3/3;
the description may exceed the buffer 5/5; a frame mapped read-write 3/3; the shrinking scaler samples one pixel 2/2; ... does not average columns 2/2; the patch ORs instead of XORs 19/11; the pointer not offset by
the hot spot 18/10; a failed read does not fall back 2/2; the rectangle origin not clipped 6/4; new pixels under the cursor not noticed 3/3.

## 8. Option C (a GPU compose in the broker): evaluated, NOT built

* **What it would be.** `EGL_EXT_image_dma_buf_import` of the frame into a GL texture (or Vulkan `VK_EXT_external_memory_dma_buf`), a shader that computes `texel ^ mask` for the cursor rectangle only
  (GLES 3 integer ops after a float round trip, or desktop GL `glLogicOp(GL_XOR)`; **GLES has no logic op and no blend equation expresses a coloured XOR**: `KHR_blend_equation_advanced` DIFFERENCE is `|d-c|`, equal to
  `d^c` only for white), rendered into a render-target dma-buf that is handed to the compositor as the overlay's `wl_buffer`. No CPU copy of the frame; the CPU would still not touch frame pixels.
* **What it costs the broker (CODE READ: `broker/Makefile` header, `broker/README.md`).** The broker's stated property is "no libdrm, no EGL, no GL, no libgbm"; a GPU path adds `libEGL`/`libGLESv2` (or Vulkan), `libgbm`, `libdrm`,
  the Mesa/vendor userspace driver loaded INTO the privileged process, a render node opened by it, and **a hostile dma-buf reaching a GPU driver's import path in that process** (kernel and userspace driver attack
  surface that today only the compositor, a different process, sees). A driver fault kills the broker and the window. It cannot be tested here (no render node); llvmpipe could cover the logic, not the vendor drivers.
* **What it would buy.** Nothing for the cases in §7.5 that Option A already serves at about a microsecond. It would be the ONLY true inversion for **frames that live in device memory or are tiled**
  (kayfabe's GPU-copy rung 0: block-linear NVIDIA VRAM). That rung is not usable on the owner's setup today (`QUERY_FORMAT XR24 NVIDIA-modifier -> NO`: the compositor is on the AMD GPU) and would force the compose onto the
  producer's GPU, which §AJ ruled out for kayfabe.
* **Recommendation: do not build it now.** Revisit only if a host appears where the compositor shares the NVIDIA GPU and the owner wants the zero-copy native rung with a true inversion; then it is an owner decision on the broker's
  no-GPU-stack property (the same decision §3 row C already named), and the first step is a design with a sandboxed helper process rather than EGL in the broker.

## 9. What the owner should look at (the live-check list; nothing below can be confirmed without the live window)

The one log line that says which choice is active (`--verbose` is not needed for it):

* `cursor: XOR composed by the broker (frame read from sealed shared memory: only the cursor's rows are read)` or `... (frame read from a LINEAR dma-buf, exporter udmabuf: ...)`
* `cursor: XOR approximated, black fill and white outline (frame not readable: <reason>)` with the reason (`the dma-buf is not known to live in system memory`, `a tiled dma-buf cannot be read by the CPU`,
  `the frame could not be mapped read-only`, `the buffer on screen is not the one that was mapped`, ...).

It appears once per change of choice and is never printed per frame. With `--verbose` each new patch also logs `cursor: XOR patch WxH px (N inverted) on the frame at X,Y -> overlay ...`.

What it should look like:

1. **Hover over text, I-beam**: a crisp I-beam at the pointer that is the INVERSE of whatever is under it (black on a white page, white on a dark one, the opposite colour on a colour; it is split exactly at the
   edge of a stripe if it sits on one). There must be **no second pointer** (the host arrow hidden). Moving the pointer fast: the I-beam keeps up with the pointer; it lags at most about a frame.
2. **Hover over a button / the desktop**: the arrow returns **at once** as the host pointer, and nothing of the I-beam stays behind. Arrow -> I-beam -> arrow repeatedly must never leave two cursors, none, or a stuck patch.
3. **Over the title bar / outside the window**: the compositor's own pointer; no I-beam patch drawn on the window.
4. **Typing in a text box in the guest**: the I-beam over changing text re-inverts with each frame (it may be one frame behind for a moment while the compositor still holds both patch buffers).
5. **Grab (CTRL+ALT+G)**: the I-beam patch sits where the guest places it (the last MOVE) and inverts the pixels under it; ending the grab puts it back at the host pointer (hover). (Not exercised on a compositor here, see §7.3.)
6. **Resize the window** (smaller, larger, fullscreen): the patch scales with the picture (a shrunk I-beam is thinner and lighter, never missing a serif); the hot spot stays under the pointer.
7. **Sizes**: arrow in hover and grab are the same size and match the picture's magnification (§1); unchanged by this work.
8. **If the frame comes as a tiled/VRAM dma-buf** (or any unreadable one) the log line says `approximated` with the reason, and the I-beam is the black/white-outline substitute exactly as before.
9. After a **broker restart** the first picture must appear at once (not after the guest's next redraw) and the log must not say "frames now arrive as plain shared memory" for the dma-buf rung (`V3_BROKER_PRESENT_TIER_20261017.md`).

What this work could NOT verify (only a live look can): fractional output scales on the owner's compositor (the 1.5x GNOME on the AMD GPU); the real NVIDIA/AMD dma-buf path (here: udmabuf and a software mutter); grab
on a real compositor; the subjective lag of the software cursor; the look of the box-filtered I-beam at the owner's window scales; the LINEAR dma-buf path against the owner's mutter (here: a software mutter
accepted the udmabuf import, the owner's refused nothing either, `QUERY_FORMAT XR24 LINEAR -> YES`); the `probe_failed` release path (CODE READ only: a software mutter does not refuse the import).

## 10. Roadmap and follow-ups

`V3_BROKER_ROADMAP_20261015.md` section A gets one line (done). The kayfabe-side deletion (§6) is the next step and may now proceed: the broker composes the XOR cursor everywhere it can read the frame.
