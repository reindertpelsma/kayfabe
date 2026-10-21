**STATUS: RESEARCH, 2026-10-22.** Isolated draft of SW2's CAPC precision topic. Original test-first
commit7b8cdc550 and fixe80903277 from codex/bw-resume-cont-20261022 are preserved by cherry-pick -x.
No diagnostic page, captured word or native fault exception is part of this topic.

The existing virtual CSC00/01 stages are declared present but their precision fields are zero.
The candidate declares14 for those matrices in main and overlay windows through the existing T1 row
mechanism. All addresses/field positions remain generated for each class/version; inactive windows are
zero. This is a source-backed virtual-engine rule, not a claim that every physical die has equal caps.

Exactly what the public sources establish:
- NVIDIA ogkm580.159.04 @b81d58ee0224d1d290bef1c080592b619e184042, MIT,
  src/common/sdk/nvidia/inc/class/clca73.h defines CAPC at0x788+i*32 with32entries;
  CSC00_PRECISION occupies4:0 and CSC01_PRECISION20:16. Each can encode14.
  The header DOES NOT define a named precision14 constant, require14 or state a reset14 value.
  Older generated classes even expose PRECISION_INIT=0. A layout alone is not a derivation of14.
- The same public revision, src/nvidia-modeset/src/nvkms-evo3.c:374-432, documents its existing CSC
  matrices as S5.14 coefficients in S5.16 software encoding. This is the independent basis for choosing14
  for the already implemented virtual stages. The mapping from that documented numeric precision to the
  named capability fields is the candidate rule; Windows sufficiency still requires hardware validation.
- FVLUT is a SEPARATE CAPC bit13; DIRECT is bit15, clamp bits5/21. The header defines boolean encodings,
  not their complete behavioral contract. CA7E's CURVE control vocabulary exists, but its exact relation
  to the FVLUT capability and required implementation remains UNRESOLVED. No FVLUT/DIRECT/clamp support
  is claimed by this patch, and no captured whole CAPC word is used to fill that gap.

Earlier hardware isolated CAPC as a whole, including both precision fields and FVLUT. That does not
isolate which bit prevents0x50. Expectation for the next derived-only ON run: no0x50. Falsifier: the
same0x50 signature recurs; then precision alone is insufficient and the remaining fields stay open.
The prior native baseline had an unmatched Xid32, so its arm comparisons are exploratory, not acceptance.

Verification retained in traces/blackwell_resume_cont_20261022/precomp_precision/:
release293/0/5 ->296/0/5; three mutants each2killers; clippy77->77/added0; rustfmt. Only32 new generated
CAPC words (four classes x8windows). No T3 changes. The AD104 historical f5 test checks both new fields
exactly and retains full equality for every other word after projecting ONLY those fields. None dropped
or newly ignored. Candidate hardware validation UNRUN; draft only, no merge.
