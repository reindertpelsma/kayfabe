# Candidate-5 interactions

FALSIFIERS stated before the runs: reset strands a flip or shows a tainted boot
layer; a console-only colour refusal taints the store; a new life reuses the old
latch-time framebuffer address; active diagnostics grow beyond their caps or
consume a request through reset.

MEASURED: all four integration regressions pass in the focused QEMU run:
- Pending colour copy completes before life reset; its flip is released, the next
  life strands no completion, and the boot layer is hidden until clean.
- A colour refusal increments console_copies_skipped while the clean boot store
  remains displayable. A real RM life taints it. Source assertions pin the worker
  choice to BootClean rather than the refusal counter.
- The latch-time scanout binding survives an unbind in the same life, is forgotten
  by reset_life, then resolves a new framebuffer for identical identifiers.
- 4096 resets with enabled event recording and step stats keep at most 48 events
  and 6 dumps; reset acknowledgements complete and the request is spent once.

MEASURED: the two separate LUT lifetime regressions also pass; all colour slots
are cleared and snapshot tokens stay monotonic across lives. Their separate
EXPECTED RED/fix and two-test mutation evidence are on the LUT feature branch.
MEASURED focused QEMU: 377 passed, zero failed/ignored (356 unit + 21 integration).

CODE READ: scan.reset_life clears latched bindings and per-life colour slots.
BootClean uses atomic taint/generation state, not log parsing or refusal counts.
Requests/event recording is relay-owned and reset Ack operations are atomic;
no new vCPU lock or wait is introduced. Scrub completion runs on the VA worker
with its manager taken out of the initialization mutex; realize zeroing is awaited
before publication. These checks are GPU-free pure-state and source-wiring checks;
they do not claim a hardware boot or live-GPU reset measurement.

MEASURED: the ten other changed Rust files reproduce exactly from rustfmt of the
merged source. Formatting changes carry no product behaviour change. Full post-
merge and final verification are recorded separately when complete.
