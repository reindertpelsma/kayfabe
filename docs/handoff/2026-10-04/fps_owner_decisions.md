# display-max-fps — owner decisions (2026-10-04), authoritative over fps_design.json §13

Owner, verbatim: "Why copying when not flipping. Yes a tearing copy seems not great though. The rest seems good to md"

- D1 tearing gate: ON. Tearing/async flips count against the cap (in kayfabe a flip copies a finished
  buffer, so it never tears; the gate is about rate only).
- D2 REPLACED (coordinator proposal, given to the owner with the explanation; adopt unless the owner objects):
  non-flip copies exist for front-buffer rendering (fbcon/efifb/GOP console, X11 without a compositor,
  front-buffer apps): there is no physical scanout, so without copies the host never sees those writes.
  New rule instead of a fixed 30 Hz / 250 ms refresh:
  1. non-flip copies happen AT the head's emulated (clamped) vblank tick — same cadence/phase as real
     scanout, tearing no worse than bare metal;
  2. send only on change: a GPU-side checksum of the surface per tick (one reduction kernel), copy/publish
     only when it differs from the last published one;
  3. no non-flip copies while nobody watches (no broker attached, no console client); screendump requests
     an on-demand fresh copy instead of relying on background refresh.
- D3 refuse values > 75: accepted.
- D4 §M premise correction (X11 vsync paced by kayfabe's own tick; tick replaces NOTIFY_ON_VBLANK/doorbell
  levers): accepted, PENDING the box run display-max-fps=30 with x11-dispsw=on; fold the correction into §M
  in-text only after that run.
- D5 unset → cap 75, EDID byte-identical: accepted.
