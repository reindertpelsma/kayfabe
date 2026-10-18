# Asynchronous walk guard and worker store completion

MEASURED: the first full run after bootlayer and all four interactions ended
3612/1/19. Its sole failure was only_the_probe_synchronizes: two direct context
synchronizations instead of the old global-one expectation. Memory and channel
passed; the named channel case also passed alone with peak one-minute load 2.492.

CODE READ: the second site is the reviewed bootlayer sync_store method. The lane
and owner ruling AM require awaited zeroing/scrub on VA/realize threads. The three
callers are realize, reset_memory_finish, and WalkZeroer::complete; the asynchronous
walk verbs still contain no synchronize. Removing this completion would violate
the approved design. Updating the obsolete global assumption preserves that design.
This is an explained source-guard mismatch, not an unexplained quiet-box failure.

FALSIFIER before adaptation: a direct or aliased synchronization enters the walk
path, or an unknown method hides a context synchronization. Keep the probe and
explicit worker barrier as the only direct sites, require each one, and allow no
sync_store call anywhere inside walk.rs (its declaration is the only occurrence).
The existing seven walk-verb guards remain and additionally reject the barrier.

MEASURED EXPECTED RED 530cca916: tests only, old predicate unchanged; four assertion
failures (three new fixtures and the existing source guard), no compile failure.
MEASURED green: 74 kf-cuda tests passed, zero failed/ignored. Seven new fixtures
accept the required completion and reject missing or moved completions and aliases.

| Mutant | Distinct failing tests |
| --- | ---: |
| Old global-one predicate |4 |
| Global-two without method scope |4 |
| Allow a walk-path call to the worker barrier |2 |

MEASURED: restored package passes 74/0/0. No product code, timer, bound or polling
change was made. The complete final verification is repeated after this test-only
integration correction; its actual counts are recorded separately.
