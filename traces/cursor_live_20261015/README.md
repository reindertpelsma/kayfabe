# Live cursor validation, 2026-10-15 (Win11 run 2990, host RTX 4070, Wayland/GNOME)

**Measured** by a live agent run (about 90 repetitions on the old broker over 30 min, then the new broker). Raw evidence stayed on
the host (`/data/claude-cursor2/live/`, 72 MB, not committed); this file is the summary.

- **Old broker (nvkvm-pv badf2d7 build, Oct 8) on kf3 004d970a:** bug A (the cursor is invisible after a click on the I-beam, at every
  non-text position, until the pointer leaves the window and re-enters) reproduced every time (slow runs 10/10, screenshots 4/4);
  without a click the cursor stayed visible. Bug B (I-beam and arrow drawn together) was seen for about 0.4 s in the fast series.
  So bug A is NOT fixed by the kayfabe-side change.
- **New broker (CAP_CURSOR_STREAM, HELLO 0x1fff), kayfabe composing no cursors:** the plain click series (fast 20, slow 10) showed no
  invisible or doubled cursor; the broker drew the I-beam substitute (black fill, outline); in grab (Ctrl+Alt+G) the overlay followed
  relative motion and vanished when the guest hid its pointer. Bug A still reproduced with "guest types (it hides its pointer), click,
  move": 3/3. In grab two I-beams were drawn: the overlay plus a stale host sprite at the lock point. Window scaling in grab was not tested.
- **Cause of A (inferred, then confirmed by an intervention):** `cur_apply` passed `last_serial` to `wl_pointer_set_cursor`, and
  `ptr_button` overwrites it with the button serial; the compositor ignores `set_cursor` unless the serial is the pointer-ENTER serial, so a
  cursor hidden before the click never came back. With a separate enter serial (`ptr_serial`): A did not reproduce on either broker
  (new broker 5/5 on a trace build, 4/4 on a clean build; old broker 3/3), and the stale sprite under grab disappeared too.
- The earlier candidate (the cursor buffer starving for a `release`, `gc_starved`) is falsified: no such events in a trace build.
- This is a bug of the broker program, not of kayfabe; the same code is in the nvkvm-pv copy (bug fixes go back there).
