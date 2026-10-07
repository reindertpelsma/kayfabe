# Two research-repo corrections that were never pushed (preserved 2026-09-30)

The C research repo `reindertpelsma/nvkvm` is **archived** on GitHub (read-only), so these two commits
from the dev host's checkout could not be pushed there (HTTP 403). They are kept here as `git am`-able
patches so they do not live only on one machine. Both are documentation corrections to the research
repo; kayfabe's frozen copy of that repo is `archive/nvkvm/` (merged before these were written).

- `0001-…-USERD-…` (`8319499`, 2026-09-15, branch `master`): `docs/design/userd_is_not_the_ring.md` §3
  was REFUTED — the guest's CPU-RM ships USERD's physical address in `NV_CHANNEL_ALLOC_PARAMS.userdMem`.
- `0001-…-CLAUDE.md-POLARITY…` (`dd37778`, 2026-08-14, branch `w324-invalidate-boundary`): the research
  repo's `CLAUDE.md` read `m2hostsem=0` as "forge OFF"; the source says the opposite (the CPU forge runs
  on the compute plane at every flag setting). ⚠ The research repo's pushed `CLAUDE.md` still carries the
  uncorrected reading.

Apply to a checkout of that repo with `git am <patch>` (not to kayfabe).
