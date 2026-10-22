STATUS: LIVE, 2026-10-21. GPU-free follow-ups accepted; hardware still pending.

Native oracle family mismatch repaired in harness only:12→15release tests. Product GR router unchanged. Initial EXPECTED RED fixture tests had operand-index errors, corrected in separate test-only commits; final RED3/3 at the old constructor, then GREEN3/3. No old test/production refusal removed.

CA73 SDR OLUT: public clca73.h CAPB684+32i OLUT_SZ12:1; derive1025samples from the existing LUT_BYTES bound minus4headerentries, DIRECT/SFCLOAD=true, LOGNR0(one table), unusedtileszero. Public NVKMS documents control SIZE=header+samplecount; the interpretation of the capability sample count is a model-backed hypothesis pending Windows. No measured physical page/width is imported. All5 driver tables tested with the real color decoder. Golden regeneration adds4CA73words only; older family rows unchanged. kf-disp288→290release tests.

Full7crates2750passed/0failed/8ignored; original6counts2733→2735, no drop. Four compiling mutants killed3/2/2/2; restored focused runs pass. Clippy against98e41cff1:582→582, added0; inherited denies collected identically via cap-lints=warn. Rustfmt3files pass. Separate draftPR60OLUT andPR61nativefixture, both based on own draftPR59 alias; no merges. Native/virtual differential limitations and next hardware falsifiers recorded before the new lease.
