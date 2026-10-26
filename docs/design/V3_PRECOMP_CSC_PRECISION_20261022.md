**STATUS: RESEARCH, 2026-10-22 03:45 UTC.** Public-source precision candidate; precision-only Windows sufficiency FALSIFIED on a clean native baseline.

MEASURED correction above the original hypothesis: own RTX 5090 lease `57148807`, product
`8f3a9bb3a5d652b3f536ca015bf95735ec588924`, passed native 595.91.07 N0 (raw30/30,
CUDA4/4, gates9/9+10 and corrected GR/CE); no unmatched Xid. Windows580.88 was sealed disabled
(Code22), then a fresh display/GOP ON sibling crashed with bugcheck0x50 after the fourth successful
CA7A allocation. Actual Code0 was not observed. The canonical trace has278 RPC rows, zero unparsed
records and reached EOF; the old276 summary omits two Other(4) records. WER and dump-header
metadata independently confirm0x50. This clean precision-only run triggers the stated falsifier.
FVLUT necessity, complete numerical fidelity and the crash's instruction-level cause remain UNRESOLVED;
none is proved by this failure. No captured-word fallback is authorized by it. Evidence:
[arm receipts](../../traces/blackwell_validation4_20261022/arms/on_precision/),
[native verdict](../../traces/blackwell_validation4_20261022/lease_metadata/N0_verdict.json).
AD104 behavior remains hardware UNRUN after this declaration. Keep PR69 draft.
Draft [PR #69](https://github.com/reindertpelsma/kayfabe/pull/69) is isolated on
`codex/bw-resume-cont-20261022-capc`, based on the read-only Claude product lineage.

The virtual CSC00/01 stages are already declared present but report zero precision. Four T1
layer rows declare precision 14 for those existing main/overlay stages. Each class/version's
generated layout still supplies addresses and masks; inactive windows remain zero. This is a
candidate rule for the virtual engine, not a declaration of equal physical capabilities on every die.

CODE READ — the public layout and the numeric basis establish different facts:

- NVIDIA ogkm 580.159.04, commit `b81d58ee0224d1d290bef1c080592b619e184042` (MIT),
  [clca73.h:846–861](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/b81d58ee0224d1d290bef1c080592b619e184042/src/common/sdk/nvidia/inc/class/clca73.h#L846)
  defines `PRECOMP_WIN_PIPE_HDR_CAPC(i)` at `0x788 + i*32`, with 32 layout entries.
  `CSC00_PRECISION` occupies bits 4:0; `CSC01_PRECISION` occupies bits 20:16.
  Both fields can encode 14. The header supplies no named precision-14 constant, requirement to
  choose 14, or reset value of 14. Older generated classes expose `PRECISION_INIT=0`.
- The same revision's
  [nvkms-evo3.c:374–432](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/b81d58ee0224d1d290bef1c080592b619e184042/src/nvidia-modeset/src/nvkms-evo3.c#L374)
  describes CSC coefficients with 14 fractional bits, represented through its 16-fractional-bit
  software format. That is the independent numeric basis for this virtual-stage candidate.
  Mapping that precision to the capability fields is an INFERRED rule. The header alone does
  not prove that mapping, Windows sufficiency, or the virtual pipeline's complete numeric fidelity.
- FVLUT is a separate CAPC bit, 13; DIRECT is bit 15; unity-clamp bits are 5 and 21.
  Boolean encodings do not specify their complete behavior. Public CA7E exposes CURVE controls,
  but the FVLUT-to-CURVE relationship, required LUT implementation and driver expectations remain
  UNRESOLVED. This patch advertises none of FVLUT, DIRECT or unity clamping.

CODE READ (commit 007ba1eb0:42): quoted historical statement, evidence not revalidated by BW9. MEASURED earlier: the bisect isolated CAPC **as a whole** on GB202. Those words changed precision
and FVLUT together; precision alone was not isolated. The baseline also contained an unmatched
native Xid 32, so these comparisons remain exploratory. Hypothesis for a future clean-baseline
derived-only ON arm: precision 14 avoids the previous 0x50. Falsifier: the same 0x50 recurs.
Then precision alone is insufficient; this does not justify importing captured words or claiming FVLUT.

Test-first history is preserved with `cherry-pick -x`: original EXPECTED RED `7b8cdc550` becomes
`b1da436f5`, followed by fix `e80903277` as `d8657cb98`. The public-source clarification is documentation.
Retained [verification](../../traces/blackwell_resume_cont_20261022/precomp_precision/validation.json):
`kf-disp` release tests 293 passed / 0 failed / 5 ignored → 296 / 0 / 5; three mutants each
killed by two tests; clippy 77 → 77, added 0; rustfmt passed. Five generated driver tables and all
display-family rows are covered. The generated fixture adds 32 CAPC words across four class layouts
and eight active windows per layout; no T3 row or captured word is added.

The historical AD104 f5 comparator explicitly checks both new precision fields, then retains full
equality for every other field after projecting only those two changes. No test is dropped or newly
ignored. Changed AD104 behavior and Windows precision sufficiency remain hardware UNRUN. Keep draft.
