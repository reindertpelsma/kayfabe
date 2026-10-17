STATUS: RESEARCH, 2026-10-17. CPU diagnostic verified; hardware candidate UNRUN.

The new allocation-only topic is `codex/runlist-581-allocation-20261017`, test-first
`da63e54eb`, implementation `90364d9d8`, both pushed; [draft PR46](https://github.com/reindertpelsma/kayfabe/pull/46)
against `claude/gpu-types-20261015`. This result does not change the
prior AD102 Windows581.51 guest **Code43** or native581.51 **PASS**.

Hypothesis stated before running: the exact public580.94.02 allocation vocabulary
removes the first B297 unsupported-cell refusal for an AD102 host. Falsifier: a valid
exact identity/layout still refuse, or allocation admission authorizes scheduling.
MEASURED: failing-first test reports the same unsupported-driver-cell reason as the
guest (`first_seq216`). After implementation, all **1382 kf-abi/kf-rm CPU tests pass,
zero failures/ignored**, including other-die/adjacent-build refusals, identity revocation,
parameter/engine/handle checks, delivered-byte bounds, log cap and no new scheduling ack.

MEASURED provenance: compiler-derived public nvos.h layout from NVIDIA580.94.02
commita86526f4e235ae3d9e2842a11a3bb11db77450d8 reproduces byte for byte. The generator
checks both retained native metadata digests and four successful allocation pairs per
stream. Altered digest, declared-size mismatch and wrong source tag reject. New class
binding is tagged measured:AD102,Windows581.51,2026-10-17 and restricted to host
architecture0x190/implementation2 and exact guest identity. No new closed-binary read.

INFERRED: associating the measured class/size with this public parameter type remains
an explicit diagnostic hypothesis. Native parameter values, parent registration and
constructor effects are not verified by the metadata. The admitted object is bounded,
unscheduled guest metadata behind the existing opt-in, with no host operation.

New-build control0x20801111 succeeds natively with40bytes (96 request/reply pairs in
each retained stream), but reply-body semantics are **NOT DERIVABLE** from retained
metadata/public headers. The new allocation entry cannot authorize that control; no
neighboring-build echo is copied. This may leave a later Windows wall and is deliberate.

Hardware falsifier still UNRUN: allocation succeeds but cold-boot teardown remains at
the same point. Fresh private build/native gates/old580 regression/full Linux+Windows
validation are required before any hardware claim. The allocation candidate alone is
not a complete Windows581 fix. No family is complete.

No new rental, no GPU use, no new rental cost. Own56302930 remains retired/APIabsence
verified00:40:19UTC; no old alias was accessed. See parent native_windows58151/RESULT.md.
