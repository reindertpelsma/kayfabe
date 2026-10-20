# C6 authorized continuation

STATUS: LIVE, 2026-10-20. Supersedes the earlier wrapper-stop checkpoint.

CODE READ: coordinator approved explicit guest-os=windows|linux (default Linux),
a separately reviewed derived-caps replacement, and corrections to own tooling.
Both earlier design choices are resolved. Stop only for unresolved product
correctness, a binding owner conflict or host/GPU safety. Deadline 06:30 CEST.
No GPU/VM slot was granted. Stages 5 and 6 remain gated.

MEASURED: wrapper correction has four tests; all product mutation campaigns
were rerun with at least two named failures per mutant. Tests/build failures
are classified separately. Topic fixes and Stage 3 install deliverables are
committed; see [TOPICS](TOPICS.md) and the current [report](README.md).

INFERRED falsifiers registered before implementation: effective Windows settings
mismatch, Linux default change, lost candidate-5 test, process-global OS choice,
captured production page or ownership bypass defeats the profile/caps change.
Any dry-run that launches/writes state, modifying doctor, required KF3 env,
system-QEMU replacement or support claim beyond evidence defeats installation.
