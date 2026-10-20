# Derived display capabilities

STATUS: LIVE / GPU-FREE ONLY, updated 2026-10-20 for C6.

CODE READ: scoped import from sweep checkpoint `24906bf33`; originating topic
`claude/display-caps-derive-20261014` (`7c12dfd14`). The coordinator approved this
replacement for the captured Windows caps probe. It is not the full sweep.

CODE READ: `kf-disp/src/capsderive.rs` starts from the authored public class
layout and family topology. `caps_rows.tsv` adds main/overlay pipeline rules.
`caps_measured.tsv` supplies only the unnamed required word with an exact
architecture/implementation/class/guest-branch match and a measured source tag.
An unmeasured die/branch gets a named missing-measurement notice, never a neighbor
row. `caps_family_pages.tsv` is a generated comparison oracle, not runtime input.
Captured full pages exist only as test oracles, never production page sources.

CODE READ: virtual Blackwell tiles are authored only inside the new derivation;
the ordinary Linux `caps::page` function is unchanged. `HostFacts` is an empty
hook because no unprivileged host control supplies these display page fields.
No privileged host display query is introduced.

MEASURED C6: the scoped suite passes; six mutations (die, branch, required word,
layer application, provenance, tiles) are each killed by at least two tests.
See [caps review](../../traces/beta_candidate_20261020/CAPS_REVIEW.md) and
[mutation receipt](../../traces/beta_candidate_20261020/mutations.json).

CODE READ limitation: full strict AD104/native equality remains a separate gate;
the source report records 100 differing words. Passing the documented claimed
fields does not prove native page equality, Windows boot or workload stability.
C6 hardware is deferred to the coordinator's Stage 5 slot.
