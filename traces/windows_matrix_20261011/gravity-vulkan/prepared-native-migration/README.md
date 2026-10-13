# Prepared native USER GP/USERD relocation discriminator

STATUS: RESEARCH, 2026-10-13. Native NOP/trigger pairs PASS as UID 65534 and
UID 0; no production caller. The test plan below predates this result.

## Recovered hardware result

Completed 2026-10-12 23:18:02–23:18:19 UTC on RTX 4070 / stock 595.91.07,
source `1a666d78301d444c4b35b021483db12841ff8980`. Collected 2026-10-13 from
`172.22.1.20:/var/lib/kf-windows-20261005/codex-prepared-native-1a666d783`.
Binary SHA256 `d6b1a34479b70d017a6fedcc0c541490904a512b763dfe10e8a9acad6948cbab`.

All four rows in [results.json](result/results.json) PASS: process rc 0,
cleanup successful, no Xid in each precise attributed journal window. The
UID 65534 registration log records all four UIDs 65534 and CapEff zero. Both
trigger cases execute the replacement-only GPU marker 2 and reach replacement
USERD GET 2. Both NOP controls leave marker/GET zero with identical 128 kicks.
The pre-run falsifier did not occur. Campaign/build/raw process/health/journal
logs are retained here; collector EXIT 0 is separate from these four verdicts.

This overturns the unavailable-native-path inference, not the earlier
same-config fault measurement. Duplicate replacement descriptors do not resolve
initial GET or 64-entry capacity. No Windows or security-boundary guarantee is
established. See the [authority assessment and next gates](../../../../docs/design/V3_NATIVE_FIFO_RELOCATION_20261013.md).

## Original test plan

Native caller remains USER; registration UID/effective capability are logged at
registration, not inferred from channel privilege. Public V2 field layout and
CPU-RM USERD memory registration, no private-derived interface or constant.

Same live mapped64KiB native object: old GP0x1000/32, USERD0x2000, marker0x3010,
PB0x4000; replacement GP0x5000/64, USERD0x6000, PB0x7000, marker0x8010. Distinct
replacement PB contains only native WFI semaphore marker2; old PB only marker1
and 5080 NOP/trigger. Both replacement slots0/1 are identical harmless references,
replacementPUT2; this accommodates GET0/1 but does not establish initialGET or
64-entry capacity. V2 registers owned memory before trigger; all prepared bytes
have a release fence before originalPUT/native doorbell. GPU writes both markers;
CPU only initializes. Positive requires replacementmarker2 and replacementGET2,
not just the oldtail or registration status. NOP uses identical prepared fields
and matched128 post-tail token kicks; its exact post-kick hold is2s.

Independent source review found two confounds, both fixed: positive-only extra
kicks, and memory revocation after uncertain native free. Both cases now get the
same fixed kicks; on uncertain free, own RM session closes before CPU view drops
and explicit backing unmap/free is skipped. Review cleared this isolated native
fixture; caller metadata logging was then added. Compile-only cargo test passes
(0 unit tests), not a functional test claim.

Supervisor runs uid65534 control/trigger then uid0 control/trigger, each fresh
process with precise attributed journal window and2s late-fault observation.
Per-mode PASS requires native observable/cleanup/noXid; collectionEXIT0 only means
all rows recorded. Negative does not identify inner status or prove all variants
unavailable. Any unprivileged positive requires a security/authority assessment
and per-driver capability evidence before product use. No crossVM availability
claim. Run only after Windows campaign frees global GPU lock; do not build on
host during timed guest work. Runtime under Windows remains exactd8afb1e3.
