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


Derivation result:590class/register/layout compilation succeeds from public
ogkm590.48.01 commit2ccbad25e1af6a6ee6f38cf569f89f8b65d658ab.
595.91.07 commit9f087a6d4e86d85acc0ce1d354d6276fe5047b29 removes
NVC370_CTRL_CMD_SET_SWAPRDY_GPIO_WAR: the original generator fails there.
The updated generator conditionally emits available control IDs and diagnoses
MISSING by name; required structures/fields/other constants still must compile.
The595 table omits the removed ID.590 and595 layout bodies otherwise match.
A model-claims test proves590 claims the derived ID and595 does not.

All4 before tests fail: family vocabulary, GPU-free realize, exact layouts and
removed-control model claims. Full isolated CPU suite at8149f0b53 passes:
3243passed/0failed/14ignored, target-driver590. Both new layout tests
and all old-family tests pass. No hardware run of this candidate yet.
Workspace-wide cargo fmt check still reports inherited formatting differences;
new layout.rs tests have been rustfmt-formatted. No CI/merge-green claim.

Generated files:6878class rows,341register rows,296/295layout lines.
SHA256 classes590 c0884d4f5f3716f44ac752eecf416e6cba7789a57b1ea83529702259c43dbe12;
regs590 00d8618e471a822242d5a8f2995719e44a53d907e7c9730225885bec3e94ca8f;
layouts590 e210da4c647aad98e5f16a6ea09ee81b0b81be6595c6439dedeb11f3f47a7f9d;
layouts595 9c6c5876e91e28813ee390bb2a6a27d5af0fe45922507089a66ad407274052bb.
