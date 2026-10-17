# V3 broker — the true XOR cursor, composed by the broker (D2), and the cursor size rule

**STATUS: LIVE, 2026-10-17.** Branch `claude/cursor-xor-20261017`. Two parts: (1) **BUILT, GPU-FREE-TESTED, live look OWED:** the cursor
size rule (§1). (2) **OPTIONS + one PURE function BUILT; the on-screen mechanism is NOT built and waits for an owner choice** (§2–§5):
the mechanism reads frame memory in the privileged broker and changes the hover path, which is more than a contained change, and it cannot be
exercised here (no compositor with a pointer, no GPU). Binding context: `OWNER_RULINGS.md` §AJ (2026-10-17: kayfabe composes no cursor; the
broker owns the whole cursor plane including true inversion).

Evidence labels: **MEASURED** (ran it / read the live log), **CODE READ**, **INFERRED**.

## 0. Decisions in force

* D1 (settled earlier): the broker owns the whole cursor plane, kayfabe is mode-free (`CAP_CURSOR_STREAM`, atomic MOVE pair, hover hot-spot latch).
* D2 (2026-10-17): an XOR cursor (the Windows I-beam) is a TRUE inversion of what is under it, composed by the broker. Kayfabe keeps sending the
  inversion mask with `CURSOR_F_XOR` and composes nothing. The approximation (`nb_cursor_xor_approx`: black fill, white outline) is a stopgap.
* D3 (2026-10-17): the cursor is oversized on the owner's screen (§1).
* Owner ruling (2026-10-17, `OWNER_RULINGS.md` §AJ): kayfabe-side cursor composing is to be DELETED, not kept as a fallback (list in §6).

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
* (c) **What the frames are, on the owner's setup (MEASURED, run-3001 log):** the compositor is an AMD GPU's GNOME (24 advertised pairs: implicit, AMD
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

**Why it is not implemented here** (the brief's containment test): (1) it keeps a mapping of each shm frame in the broker (new lifetime, SIGBUS and
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

**Steps to finish A** (each small): (i) keep the mask beside the approximation (`struct nb_cursor` gets `xor` and a second pixel set; `nb_cursor_xor_approx` stops overwriting);
(ii) Wayland: keep a read-only mapping per shm `nb_wl_buf` (mapped lazily, unmapped in `wl_buf_destroy`; sealed, so bounded by `stride*height+offset`); (iii) at commit time, when
an XOR cursor is visible and the pending buffer is mappable, build the patch with `nb_cursor_xor_patch`, attach it to the overlay (sync subsurface, `buffer_scale` rule of §1),
and log `cursor: XOR composed by the broker` once; when it is not (dma-buf tier, unmappable), log `cursor: XOR approximated (frame not readable: <tier>)` once and keep today's
behaviour; (iv) hover while composing: `set_cursor(NULL)` and place the overlay from `wl_pointer.motion`; restore on leaving; (v) a counter in the status/log line
(`xor_composed`, `xor_approx`), never per frame. Test plan: the pure tests above; a test-backend case that feeds a frame and checks the patch hash; the selftest/testclient
unchanged; live: the owner's screen (below).

## 5. What the owner should look at (not verifiable without the screen)

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
