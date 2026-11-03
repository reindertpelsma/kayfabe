# Approved USER relay family admission

STATUS: LIVE, 2026-10-19. Narrow approval: helper, call site and tests only.

Hypothesis H-U1 (CODE READ): the Ampere-only declaration predicate refuses
public generated non-Ada channel classes, including Turing c46f. Falsifier:
the unchanged predicate accepts all generated non-Ada channel declarations
with valid geometry. Commit unchanged extraction and failing tests first.

AD104 preservation: Ada generated channels include c36f and c46f in addition
to c56f. Both extras remain REFUSED by this topic. Ada admission stays exactly
c56f, as before; no native birth widening is requested or inferred. An exhaustive
16-bit class test plus high out-of-range values pins this boundary. Other
families use their generated ChannelGpfifo membership.

Mutations planned before running: Ampere-only regression, removing the Ada
restriction, admitting arbitrary classes, and bypassing geometry. Each must
be killed at runtime by at least two distinct tests. Preserve every existing
test; compare full release results and clippy diagnostic identities with base.
Rustfmt new helper/call/tests, preserving unrelated inherited formatting.

No changes to USERD/subdevice ownership, permit/native birth, incarnation,
lifecycle, lazy observe, scan-off, doorbells, groups, latejoin or userpump.
