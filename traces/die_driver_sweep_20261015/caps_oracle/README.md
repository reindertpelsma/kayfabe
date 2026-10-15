# Full display-capabilities equality gate

**STATUS: LIVE, 2026-10-16. GPU-free result; W1 remains RED.**

Compared every aligned word of all4096bytes, including omitted zero words, using
the actual runtime derivation at0b6c501fb (productc2092cf7b) plus measurement-only
examplea84059a10. `receipt.json` pins both source revisions, the example hash and
each input oracle hash. Driver ABI580.65.06, shipped exact-die580rows, SDRenabled;
no GPU, host driver or guest process exists in this check. No process-env claim.

| Target | Native oracle | Differing words | Words containing unnamed register/bits | Result |
|---|---|---:|---:|---|
| AD104 | owner's existing AD104 test fixture |100|19|FAIL exit1|
| AD102 | own nativeWindows580.88 measurement |100|19|FAIL exit1|
| TU106 | own nativeWindows580.88 measurement |91|19|FAIL exit1|
| GB206 | own nativeWindows580.88 measurement |125|65|FAIL exit1|
| AD106 | AD104 reference ONLY; no own native AD106 page |101|20|FAIL exit1; cross-die diagnostic only|
| GA104 | native A4000-class measurement missing |—|—|NOTRUN|

The hypothesis 'current derived page equals the full native oracle' is falsified
for every available exact-die oracle. Every differing word is in its TSV with
both complete32bit values, header-defined fields or named unknown cells, and
existing derivation provenance. A header defines an encoding, not the physical
value. Native-value derivations remain explicitly unknown. Some differences are
intentional virtual connector/features policy (e.g. TMDS instead of native DP);
others are unmodeled physical widths/limits, unnamed cells or family-specific
capability words. These classifications do not prove which causes a guest fault.
No captured page/value was added to product authoring by this comparison.

The new checker validates the native page's end count, alignment, duplicate
words and bounds. Two synthetic parser/difference tests pass. Exact equality is
a separate red release gate, not hidden by the existing 'named unclaimed fields'
test. W1(c)'s hardware falsifier (equal bytes but no window scan) is UNRUN because
the equality precondition is false. The Blackwell tiling candidate remains held.

Next ranked work: obtain sources for the named unknown fields and reconcile
virtual feature policy with the requested full-native-page equality criterion;
measure an actual GA104 page; separately pursue W2's complete native trace before
any proposed Blackwell CCU/CMC/CBCB replies. The AD104 oracle must not become an
AD106 product fact. Native raw page text remains a test oracle under traces.
