# Graphics grade pipe status

**STATUS: LIVE, 2026-10-15.** GPU-free grading fix; native replay follows.

The second native graphics oracle reports NVIDIA renderer, reference-fails0,
GLX app rc0, yet S5FAIL. Hypothesis before replay: an early grep-q exit under
pipefail makes the echo producer fail, creating a false negative. Falsifier:
correct long captured-output fixture passes the original condition, or the
live native condition still fails after removing the producer pipe.

The long-output regression fails before; here-string grep predicates pass it
without changing the required renderer/reference checks. The negative fixture
still fails. All21 harness tests pass after. Actual native failure causality
remains inferred until the replay; no failed guest graphics result was claimed.
