# L5 hypotheses and falsifiers

STATUS: RESEARCH, 2026-10-18. Recorded before inspecting the implementation.
Base: 0dbcc8a1463f0ae3ae58ed867373a603b69e6ba8.
Time box: 2026-10-18 01:52:39–06:52:39 UTC.

- O1 hypothesis (INFERRED): CPU-window placements survive the old RM's unload because
  unload retirement does not remove that life's placements. FALSIFIER: the unload
  path already removes every old-life placement, or an ordered log attributes the
  overlaps to a new/live BAR1-virtual life before the refused re-seed. Do not remove
  placements without a proven ownership/lifetime boundary.
- O2 hypothesis (INFERRED): C >= G protects the boot range only while the console
  reservation remains active; it does not by itself prove that displaying the
  range until a head arms is safe. FALSIFIER: public RM source proves the entire
  reservation stays unavailable to every non-console allocation until a head is
  armed, including modeset/reinitialization/failure paths. Retain the current taint
  unless that stronger ordering is established.
- O3 hypothesis (INFERRED): a repeated PHYSICAL BAR1-mode write after seed retirement
  can request an unload zero without a proven RM-unload boundary. FALSIFIER: every
  caller requires a proven unload event, or an explicit mode-transition/life guard
  already rejects repeated initialization writes. Test the smallest certain
  defect before changing behavior.
- O4 hypothesis (INFERRED): existing diagnostic and QMP/broker interfaces can support
  the requested host capture plan without a product change. FALSIFIER: one required
  observation has no existing safe diagnostic path; document that missing interface
  rather than inventing a claimed measurement.
- O5 hypothesis (INFERRED): the boot-layer diff respects the vCPU/lock, flag and unsafe
  rules. FALSIFIER: a new guest-triggered path blocks a vCPU or holds a vCPU-shared
  lock across blocking work, a flag becomes necessary to boot, or unsafe code is
  added outside the allowed crates/files. Report exact source sites.

Verdicts, tests and limitations are recorded in FINDINGS.md when established.

- O5a discovered during the reset audit (CODE READ): `CpuWindow::sink_all` iterates
  only `placed`, whereas the boot seed is intentionally outside `placed`.
  Hypothesis (INFERRED): the memory-stage preparation leaves a live seed reachable
  before the first BAR1 change and after an unload/reseed. FALSIFIER, before tests:
  either phase's production `sink_all` removes the seed and records scratch before
  aperture release. Options: use existing `retire_seed` in the scrub preparation;
  or retain the mapping and withdraw the documented no-CPU-view guarantee. The
  existing contract selects the first; no new scrub or timing policy is needed.

- Validation follow-up (before retry): the full post-fix kf-mem run failed only
  `batch::tests::the_act_thread_waits_for_a_chunk_not_for_the_operation` (a 100001 us
  ledger-lock wait); both new seed tests passed. Hypothesis (INFERRED): shared-dev-box
  scheduling affected this existing wall-clock assertion. FALSIFIER: the unchanged
  test also fails when run alone. Do not alter a timer, bound, priority or test oracle.

- O3a (CODE READ discovery, before its failing tests): the production stale-life
  callback returns an error, but `restore_physical_view_scrubbed` still calls
  `reseed`. Hypothesis (INFERRED): already-detected stale work can replace a new
  life's scratch boot range, including when its live placements lie outside G.
  FALSIFIER: either production-helper fixture leaves the seed retired after a
  callback that explicitly refuses because fn 72 began a newer life. Preserve
  the separate rule that an ordinary host scrub refusal still permits reseed.
