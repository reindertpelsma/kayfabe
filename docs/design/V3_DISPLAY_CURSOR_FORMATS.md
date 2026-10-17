# V3 display — cursor formats: the Windows I-beam (A1R5G5B5)

**STATUS: LIVE, 2026-10-15 — BUILT, GPU-FREE-TESTED; the guest's bytes are NOT yet measured (the dump is built, the run is
waiting).** Branch `claude/cursor-formats-20261015`. Supersedes the sentence "`A1R5G5B5` stays refused by name"
in `V3_DISPLAY.md` §8.12 (the correction is folded there, above the text it corrects).

## 1. The defect

Owner report, Win11 VM 2990 on kf3 `eb9f2aa3`: every cursor works (arrow, resize, hand — the `A8R8G8B8` alpha
cursors) except the text I-beam, which is invisible. The run's `qemu.log` (read only) shows, with the I-beam
over a text field, both paths refusing a cursor whose format is not `A8R8G8B8`:

```
kf3: display: host cursor REFUSED: host cursor head 0: only an A8R8G8B8 cursor can be the host's — composed into the frame instead, and the host's is hidden (N so far)
kf3: display: scanout REFUSED cursor: cursor head 0: only an A8R8G8B8 cursor is composable
```

The host path hides the host pointer (a refused cursor is "composed instead"), the composer refuses it too, so
nothing is drawn. Same lines in `traces/windows_matrix_20261011/manual-1380` (counters `host_cursor_reads=71195
host_cursor_refused=7684`): not new, just first reported.

## 2. What the guest programs — MEASURED vs INFERRED

* MEASURED (log): the format field of the cursor control word is not `A8R8G8B8` (0xCF) while the I-beam shows
  (that is the only test in `argb8888`); the arrow is `A8R8G8B8` with composition word `0x072ff`
  (PREMULT_ALPHA). The log does not carry the raw format value, the mode, the pitch or any byte.
* INFERRED (not evidence): the format is `A1R5G5B5` (0xE9, the class's only other format, identical in
  `clc37d.h`/`clc57d.h`/`clc67d.h`/`clc97d.h`/`clca7d.h`), probably with `MODE_XOR`: Windows' I-beam is an inverting
  (AND/XOR) pointer, which the stock driver can only express on this hardware as an XOR cursor.
* FALSIFIER of the inference (stated before the run): the dump (§5) shows format 0xCF, or a value other than 0xE9,
  or mode 0 with 0xE9. Mode 0 + 0xE9 would still be handled (blend path); another format value would be refused by
  name WITH its value.
* The dump is `KF3_DEBUG_CURSOR=1` (§5). The owner's running VM predates it; it needs a restart on the new build.

## 3. The class vocabulary is the headers (derive, never capture)

`HEAD_SET_CONTROL_CURSOR_FORMAT_{A1R5G5B5,A8R8G8B8}` and `…_COMPOSITION_MODE_{BLEND,XOR}` are rows of the
derived class tables (`tools/derive_display_classes.sh`); `CursorVocab` now resolves `A1R5G5B5` from the table like
`A8R8G8B8` (`None` where a table lacks it). Test `the_cursor_format_vocabulary_is_the_headers` asserts the four
values in every cursor-capable class of both table versions (C37D C57D C67D C77D C97D CA7D, 580.159.04 and
580.65.06) and, where the ogkm checkout is present (`KF_OGKM_CLASS_DIR`, default `/data/claude-ogkm-580/...`),
re-reads the eight class headers themselves. Values: `A1R5G5B5` 0xE9, `A8R8G8B8` 0xCF, `MODE_BLEND` 0, `MODE_XOR` 1.
`CursorScan` keeps the raw `format`.

## 4. The change

* `kf_disp::scanout::plan_cursor_any` plans both formats: geometry, clipping, bounds in the context DMA, mode and
  factors are the one code path (`plan_cursor_bpp`); `A1R5G5B5` reads rows `max(size * 2, 256)` bytes apart.
  ⊘ INFERRED pitch: NVKMS's `max(size * 4, 256)` rule (`nvkms-evo3.c:6531-6552`) at 16 bits; nothing in the open
  sources programs this format. The dump prints the image at that pitch.
* The kernel moves 32-bit pixels, so an `A1R5G5B5` cursor is a `CursorExpand`: the GPU copies the (bounded) source
  span into a buffer kf owns (`read_store`, as the host cursor already does), the CPU expands THAT COPY
  (`CursorExpand::expand`), the pixels go to a kf-owned device buffer (`DisplayGpu::cursor_upload`, 2 x 256 KiB, the
  host-to-device copy is synchronous on the blocking display stream) and the existing kernel composes them
  (`compose_cursor`). The CPU never reads guest memory; the PTX is unchanged.
* Expansion (a DEFINITION): bit 15 alpha, 14:10 R, 9:5 G, 4:0 B; a 5-bit channel `v` widens to `v<<3 | v>>2`.
  Blend: alpha 1 -> opaque, alpha 0 -> empty (colour dropped: a premultiplied blend must not add colour where
  nothing is covered; an OPAQUE composition keeps every pixel, as for `A8R8G8B8`). XOR: the colour of the pixels the
  alpha bit selects, so an all-zero image composes to nothing, a white opaque pixel inverts, and a clear-alpha pixel
  never flips anything.
* Host cursor: an `A1R5G5B5` blend-mode cursor is converted to premultiplied ARGB (`HostCursorSrc::bpp`, one pixel
  accessor shared by `image` and `alpha_census`). An XOR cursor is still not the host's; it is composed into the frame
  and the host's pointer is hidden, now visibly. Its log line says `host cursor COMPOSED` (by design), not `REFUSED`.
* Hostile bounds (tests): a size that is not 32/64/128/256 is refused by name before any arithmetic (`size * bpp`
  cannot overflow); an unknown format is refused by name WITH its value; an unknown mode is refused; a system-memory
  or block-linear surface is refused; a span one byte past the context DMA is refused; nothing is read until a plan
  exists; `expand` takes exactly the copy it planned. `A8R8G8B8` plans are byte-identical (the existing tests pass
  unchanged).

## 5. Measurement: `KF3_DEBUG_CURSOR`

Unset or `0`: nothing (one `OnceLock` read). `=1`: for each distinct (by FNV hash of the first <= 64 KiB, format,
mode) cursor that the host path or the composer refuses/composes-by-design — at most 16 contents, at most one GPU
copy per 200 ms, only a span wholly inside a video-memory pitch context DMA — logs
`kf3: display: DEBUG cursor (host|scanout) format 0x.. (A8R8G8B8 b, A1R5G5B5 b) composition 0x.. = NAME mode M size SxS hot X,Y
point X,Y offset 0x.. pitch16 P: N bytes copied, NZ non-zero, fnv H; first 64 bytes <hex>` and up to 32 text rows of the image
read as `A1R5G5B5` at `pitch16`: ` ` alpha 0 / colour 0, `.` alpha 0 / colour, `#` opaque light, `o` opaque dark
(colour 0), `+` opaque other. `=/some/dir` also writes `cursor-<key>.bin` (the surface bytes) there.

**Decision rule for the ambiguous reading** (the open question of `V3_DISPLAY.md` §8.12, now on a real producer):
* Image is mostly ` ` with a few `#`: the alpha-gated reading built here is right (transparent surround, white =
  invert).
* Image is mostly `o` (opaque, colour 0) with `#`: the *masked-colour* reading (alpha is the AND mask: 1 = keep what is
  below, 0 = replace by the colour; colour then XORed) is the hardware's, and this build would draw an invisible
  surround but also an invisible shape (`o` XOR nothing). Then two layers express it with the existing kernel: an
  opaque-blend layer of the alpha-0 pixels, and an XOR layer of the alpha-1 pixels. Not built: the bytes decide.
* A format other than 0xE9/0xCF: the dump names it; nothing is guessed.

## 6. Tests (GPU-free, `cargo test -p kf-disp`: 160 lib + the PTX interpreter suites all pass)

`an_a1r5g5b5_pixel_widens_to_bgra`, `an_a1r5g5b5_blend_cursor_expands_to_a_32_bit_premultiplied_layer` (composed
over grey: white, black, transparent, red), `an_a1r5g5b5_xor_cursor_inverts_under_white_and_an_empty_one_is_nothing`,
`an_opaque_a1r5g5b5_composition_keeps_every_pixel`, `an_a1r5g5b5_cursor_is_clipped_like_an_argb_one`,
`an_a1r5g5b5_cursor_is_bounded_like_every_cursor` (hostile), `the_host_takes_an_a1r5g5b5_blend_cursor_as_premultiplied_argb`,
`the_debug_dump_is_bounded_and_names_the_format`, engine `an_a1r5g5b5_xor_cursor_is_scanned_as_one`,
`the_cursor_format_vocabulary_is_the_headers`. NOT tested: the GPU path of `cursor_upload`/`compose_cursor` (no GPU
in the lane; it launches the same kernel `compose_layer` does, from a different base pointer).

## 7. Verification (Windows VM, after the restart on the new revision)

See `docs/STATUS_AND_HANDOFF.md` / the coordinator's note; summary: start with `KF3_DEBUG_CURSOR=1`, put the pointer
over a Notepad text area, read `grep -a "DEBUG cursor" qemu.log`, screenshot with `python3 /data/claude-shot.py`
(QMP `screendump` = the composed frame, so a composed I-beam is in it; a host-pointer ARGB cursor is NOT, it travels
through the broker), and pass when the I-beam is visible and there is no `scanout REFUSED cursor` and no `host cursor
REFUSED` line.

## 8. The cursor stream: the broker draws the cursor in every mode (2026-10-15, branch `claude/cursor-hover-20261015`)

**CORRECTED 2026-10-17 (owner D2 + `OWNER_RULINGS.md` §AJ), above the text it corrects.** (1) "The broker always draws the substitute: opaque black fill, one-pixel
opaque white outline" is a STOPGAP: the owner requires a TRUE inversion composed by the broker (`V3_BROKER_XOR_CURSOR_20261017.md`: options, the built pure compose
function `nb_cursor_xor_apply` / `nb_cursor_xor_patch` whose semantics are the retired kernel's `below ^ (mask & 0x00ffffff)`, and why the mechanism is not built yet).
(2) "It does NOT supersede §4 ... that stays the whole story for a broker that does not advertise the stream" is **superseded**: the composing path (§4, and the
composing paragraph of §5–§7) is to be deleted, not kept as the fallback. (3) The cursor surface size is `image x the frame fit` in surface-local pixels, the output scale only
chooses `wl_surface.set_buffer_scale` (no viewport on cursor surfaces), one function for hover and grab.


**STATUS: BUILT, GPU-FREE-TESTED; live validation OWED (Windows VM with this broker: hover, grab, the I-beam; the
Linux broker retest the owner's rule requires).** It does NOT supersede §4 (the composing of XOR cursors into the
frame): that stays the whole story for a broker that does not advertise the stream.

**Why.** The two reported symptoms (hover cursor invisible after the I-beam; an I-beam and a resize arrow drawn
together) sit on the XOR (composed in the frame, host cursor hidden) <-> ARGB (host cursor, frame without) switch.
*Measured* in the run-2990 log: 8064 "host cursor COMPOSED" lines (every 256th of ~2.07M XOR passes), `cursor_hides=127`,
`cursor_sets=369`, one mode line only (no grab). *Measured* (GPU-free model, `crates/kf-broker/tests/host_cursor.rs`,
the REAL relay, share and console cursor around a modelled worker): the relay sent the host cursor on the worker's
**post** (start of the copy) while the frame carrying the matching composed bit was published at its **end** and shown by
the broker only after the previous frame was presented, so for that whole time the broker drew both cursors (arrow
before the I-beam left the frame) or none; every viewer **does** converge once the guest's cursor stops changing (300
seeds x 4 copy latencies), so a state that stays wrong is NOT explained by the worker/relay/console logic — the persistent
"invisible until you leave the window" is *inferred* to be in the broker (a Wayland cursor-surface buffer starved of its
`release`, `gc_starved` in `nb_session_wl.c`: both slots busy leaves `set_cursor(NULL)`), unmeasured. The composing path
now follows the frame the broker was last committed (`FrameGeom::cursor`, `CursorConn::frame_composed`): fixes the
transient; the old composing path is otherwise untouched and is the fallback.

**The stream** (`CAP_CURSOR_STREAM`, HELLO `w1` bit 12, `broker/common/nvkvm_broker_proto.h`, the one header):

* kayfabe reports the guest's cursor and composes NOTHING in any mode: `CMD_CURSOR` `SET` (image, programmed/derived hot
  spot), `HIDE`/`SHOW` (hidden covers: the guest hid it; the cursor plane is off, so the guest draws a software cursor
  into the frame itself; an empty or unconvertible image), and the new `MOVE` — ONE atomic pair (⊘ CORRECTED 2026-10-15,
  below the first version, which sent the top-left under grab only): the image's top-left on the frame and the last ABSOLUTE
  pointer position the relay injected, in guest frame pixels, flag `CURSOR_F_ABS` clear after relative input; signed, clamped to
  -256..8192 by the sender, refused by the broker beyond +-2^20; sent in every mode (grab is the viewer's state, not the
  protocol's), one per change of the pair (the top-left is noted once per pass), never per input event. The broker derives the
  hover hot spot `pointer - top-left` and latches it on two equal consecutive differences (see
  `V3_BROKER_ROADMAP_20261015.md` section A); under the stream kayfabe sends the programmed hot spot and does not run the
  `HotTracker` (the composing fallback keeps it).
* An XOR cursor (the Windows I-beam) is converted by kayfabe to the inversion mask (`kf_disp::scanout::xor_mask`:
  alpha 0xff and the XOR colour where the colour bits are non-zero, regardless of the alpha bit; empty = hidden) and sent
  as a `SET` with `CURSOR_F_XOR`. The broker always draws the substitute: opaque black fill, one-pixel opaque white
  outline (8-neighbourhood), transparent elsewhere (`nb_cursor_xor_approx`).
* The broker draws: hover = the host pointer (MOVE ignored); grab = an overlay at the last MOVE, scaled with the frame
  (Wayland: a `wl_subsurface` above the content, never touching the dma-buf frame). X11 does not advertise the bit (no
  overlay implemented; nothing measured) and keeps the composing path. `MOVE` and `F_XOR` without the bit are framing
  violations.
* VMM-neutral code: the decisions (`kf_broker::want_of`, `composes`, the relay's `stream_sync`, `CursorShare`) are in
  `kf-broker`; the conversion is in `kf-disp`. `kf-qemu` only reads the GPU (`host_cursor_want`) and calls them.
  Still QEMU-bound: the display worker itself (`display.rs`, the GPU read and the per-pass loop) and the QEMU console
  cursor sink (`ConsoleCursor`, used on the composing path; the console defines NO cursor while the broker streams — a
  VNC viewer sees none then).
