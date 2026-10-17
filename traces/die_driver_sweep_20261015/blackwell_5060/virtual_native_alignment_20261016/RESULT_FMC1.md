# Blackwell FMC observer replay

**STATUS: RESEARCH, 2026-10-16. Prefix recovery PASS; strict validity falsifier FIRED.**

Own56165326, integrated3934a089550c8f6673d1bb26f8c0670e4385fa41,
binarySHA256f38954e683908c3a8fe94a85f88f4b664aa0e24a03803dd3f00f42f1cca2d18f.
Only diagnostic FMC mailbox indirection changed from72fca168a. Host595.91.07,
public guestABI580.65.06, unchanged Windows overlay previously bound actual
32.0.15.8088 with CM_PROB_FAILED_POST_START. Fresh FMC light query produced no
validation output; no new adapter-code/app/TDR success claim. Actual process
42721 environment/executable/argv/hostABI asserted in process receipt.

Started00:59:57UTC, QGA answered01:01:48, observer window240seconds. Own VM
stopped using QGA then ACPI then exact TERM, exit0 at01:06:20; controller
42689 exited0. Controller success means collection completed, not oracle PASS.

| Check | Measured result |
|---|---|
| Table attachment |1;2 bootstrap attempts (direct then FMC),0 invalid bootstrap|
| Command queue |387 records, sequence0..386|
| Status queue |385 records, sequence0..384|
| Forward gaps / drops / read failures / unstable snapshots |0 /0 /0 /0|
| Footer |file export complete,772 records,468776 source bytes|
| Slot-probe counter |5953 failed element probes; scanner examines empty/old slots repeatedly, not5953 missing messages|
| Strict census + ordered exporter |both exit1;109 anomalies,39 unpaired controls|
| Anomalies |60 trailing_bytes,49 control_extra_bytes|

Raw JSONL SHA256c3b0be1f50c711eb77810c3b5a50b3647184fe0e9a171a86d3b495a067648cda
stays rental-only; only footer/hash/metadata/census/receipts returned as text.
H-FMC1's attachment/prefix portion succeeds, but its clean-decoder criterion
fails. Do not call this a fully validated virtual reference or suppress anomalies.

## What the fuller metadata changed

The prior pretty comparison collapsed RPC-envelope and control-body statuses
into one effective status. It hid an earlier difference: native request12,
reply31,0x20800a87 has envelope0/controlstatus0x56/params1056; virtual request12,
reply11 has envelope0x56 and a zeroed control header. Native then issues
request13,0x20800a40, which virtual also issues. This is the first differing
envelope/control-status pair among the aligned common calls examined, not
proof of the first differing body byte or the cause of Code43. Refusal reply
pairing is inferred by function FIFO and adjacent agreeing reply keys because
virtual erases the refused control key. No successful-body key conflicts occur.

aligned-wire-metadata.tsv keeps envelope and body status distinct:196 aligned
calls,45 status-field differences. Payloads were not exported. The earlier
aligned-api.tsv remains accurate only for its explicitly collapsed effective
status and has weaker observability. a89/ab2/CBCB native success versus virtual
refusal remains measured. Nativeab4 remains absent from the successful window.

Two diagnostic mechanisms explain candidate anomaly classes, not a proof that
each is harmless: the observer preserves public fn103 allocation params beyond
rpc.length using flag8, while the census exact-length check ignores that flag;
Kayfabe deliberately refuses unknown controls in the envelope and zeroes the
body, so their erased keys cannot pair by control ID. boot.rs documents the
public copy-out short circuit and prior safety evidence for named refusals.
Do not change product error replies merely to make the census green. Parser
repairs need independent failing-first protocol tests and explicit coverage.

GCC/Clang ASan/UBSan core + decoder and pinned public compiler-layout checks
pass. Full QEMU build passes. The production writer test first failed because
libqom.a was not built; after building that prerequisite, both writer tests and
JSONL/checksum/hash/footer validation pass. Both attempts are saved.

## Stop and rank

1. Fix observer/census allocation-tail accounting and model explicit envelope
   refusals without hiding true unmatched replies; preserve strict failures.
2. Obtain screened native reply fields before erasing the next native image,
   then compare outputs on the common startup path. Current native raw capture
   is erased; full future byte comparisons need another native capture.
3. Source-derived CMC/CCU handling needs actual semantics and explicit ABI
   policy. H-CCU2 already falsified595-to580 sample equivalence.
4. CBCB has a public matching scheduler namespace and native zero-parameter
   alloc/free observations, but no justified constructor behavior yet.

No guessed success or captured per-die value was added. This bounded Blackwell
wave stops with a complete native reference and a useful but oracle-failing
virtual capture. Next requested hardware work is W3 TU1067min hold and R34.
