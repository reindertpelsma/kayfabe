STATUS: LIVE, 2026-10-22. CODE READ / documentation-only tier audit; hardware n=0.

Read V3_CONSTANT_STRATEGY, OWNER_RULINGS AF/AH, kf_rm::consttab and kf-chip family rows first.
No native capture was read or replayed by this audit. No production code, values, T3 rows,
SDK tables or tests changed. No build, GPU job, rental or host/foreign-tree write.

inventory.py reads only generated class TSVs and the committed AUTHORED family report. It
asserts all41 CA73 fields exist and agree across five public SDK layouts, then emits every
field's bit range, current model main/overlay values and source-policy status. Those values
(2026-10-22) are source review, not a new build or hardware measurement. Rerunning must reproduce
precomp_fields.tsv and inventory.json byte for byte.

family_mechanism.json records the pinned public generated IP/class binding for five dies
across five tags, verified (2026-10-22) from git blobs even when a reference checkout is sparse. This
proves common layout selection; it does not prove equal physical caps. Literal FVLUT search
scope and the sparse-checkout limit are explicit. linux_sources.json pins the inspected
MIT nouveau CA window/head sources; none supplies a native CAPC value rule.

Complete result, per-field tier matrix, generation-wide resolver design and falsifiers:
docs/design/V3_BLACKWELL_PRECOMP_TIER_AUDIT_20261022.md. The older whole-page T2 publication
is held in draft PR85; its green tests are not presented as proof of the new field policy.
Captured GB202 words are not added to a rule, family table or runtime fallback. No numeric
rule for an unknown die or unsupported CA feature is invented.
