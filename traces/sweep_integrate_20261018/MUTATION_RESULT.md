# Mutation acceptance checkpoint

STATUS: LIVE, 2026-10-19. GPU-free runtime kills; fixed sources restored.

MEASURED (2026-10-19): F-2 four mutants killed3/2/2/3 tests; family admission four killed
3/2/4/2; bounded rowserve notices three killed2/2/2.
The additional stack sweep ran17 mutants;16 immediately had >=2 kills and
ignored branch scope had1. Added an independent exact-row property test and
reran with cargo --no-fail-fast so both test binaries execute:2 kills.

The12 supplementary mutations include that branch rerun. Die/branch caps,
metadata quotas, GID, QGA, guest profile, preflight, image range, file staging
and VFIO FMC are recorded in extra_mutations.json. GID initially had1 kill;
an independent bounded UUID reply-carry test raises the recheck to2.
Unittest subTest suffixes initially hid parent test names from the result
parser; corrected counts use the unchanged original runtime logs and count
each distinct parent once. Host-family2 and image-source2 kills are valid.

Final coverage:39 distinct mutants across all tables, each killed by >=2
distinct runtime tests. Failed compilations are never counted as kills.
C2 and GID rechecks supersede the earlier under-covered records; all historical
failures remain visible. No mutation is committed to production.
See f2_mutations.json, userrelay_mutations.json, rowserve_notice_mutations.json,
stack_mutations.json, extra_mutations.json and recheck_mutations.json.
