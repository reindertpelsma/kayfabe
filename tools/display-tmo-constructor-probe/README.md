# Display TMO constructor experiment

**STATUS: RESEARCH, 2026-10-05.** Default off. This experiment cannot provide a
working display or demonstrate tone mapping.

Windows J passes both checked SYSRAM registrations, then fails display
initialization. Its [journal and pinned retail branch analysis](../windows-debug-capture/evidence/2026-10-05-startdevice-j.md)
strongly suggest a zero-sized TMO buffer: Kayfabe advertises no window with the
TMO capability, while this Windows path still requires a nonempty secondary
buffer. The journal does not include that local size value. The
[public source contract](../windows-debug-capture/evidence/display-tmo-20261005/README.md)
identifies TMO as memory-backed tone mapping; advertising it in production would
require the actual LUT/color-transform behavior.

`KF3_DISPLAY_TMO_CONSTRUCTOR_PROBE=1` tests this constructor hypothesis by doing
both of the following at display-plane construction, before publication:

- Set each window's TMO_PRESENT field using the already compiler-derived class
  field, array and TRUE value.
- Create an engine which refuses every DMA and cursor PIO display method before
  decoding or enqueueing it. The channel gets a named exception, GET stays at
  its previous value, and no assembly, armed state, acquire or completion occurs.

These actions are paired by one host-only flag. There is no switch on a live
engine. Free/reallocate cannot remove the refusal mode. Repeated rejected PIO
writes do not grow a queue. The related normal halted-cursor path also now stops
before enqueueing, fixing a queue-growth issue found during this review.

This deliberately advertises a capability without implementing it **only for a
constructor test**, with all display-method execution disabled. It is a departure
from normal capability semantics, not a production workaround or a bypass-only
TMO implementation. Unlike a cap-only change, it cannot silently complete an
unsupported tone-mapping method. Ordinary display behavior stays selected unless
the host explicitly sets the flag. Existing display-context creation/import and
optional boot-image copies still exist; the refusal claim concerns channel
methods and their effects, not all GPU activity in the VMM.

Compatibility is governed by the existing exact display tables: guest 580.65.06
and 580.159.04, C573/C673/C773/CA73 display families. Missing definitions refuse
realization; no driver fallback or captured GPU-die values are introduced. Chips
without a native display retain the existing no-display policy. Host-driver
behavior and permissions are unchanged. Source/table coverage is not hardware
coverage of those combinations.

Validation: the complete kf-disp library suite passes 92 tests, including eight
source-derived driver/family capability cells, missing TMO semantics, every DMA
channel kind, cursor PIO, repeated writes, free/reallocate, and inert vblank and
acquire polling. The new tests check the absence of effects and queue growth,
not just capability bytes. `cargo check -p kf-qemu --lib`, Python syntax and
runner help also pass. Independent review found no blocker for the isolated
experiment. This is not the full hardware/isolation merge bar.

Use the existing fresh-overlay Windows runner with all J options plus
`--display-tmo-constructor-probe`. The runner clears an inherited environment
flag and only sets it when requested. Always pin the exact built revision and
save both initialization and refusal evidence. Never report this mode as a
working Windows GPU or display.
