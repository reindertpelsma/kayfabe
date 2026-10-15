# Exact590 display vocabulary and590/595 wire layouts

**STATUS: RESEARCH, 2026-10-15.** Preparation for another driver branch while
own Turing native Windows installer runs. No590 hardware claim.

H-D590 before tests/derivation: compiling public ogkm590.48.01 class/register
headers and590.48.01/595.91.07 display control structs provides exact tables
for all four family rows and the physical display link. Falsifier: derivation
fails, vocabulary/realize/layout tests fail or subsequent hardware validation
fails. Never nearest-version lookup; unknown exact tags remain refused.

595 has exact class/register tables from the earlier topic but lacks exact
physical display wire layouts. Its passed raw/CUDA/headless graphics runs did
not validate the display=on broker, which remains NOTRUN. This work must not
retroactively turn that missing broker result into PASS.
