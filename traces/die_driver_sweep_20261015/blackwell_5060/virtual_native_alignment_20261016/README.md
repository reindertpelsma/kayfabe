# Native and virtual Blackwell startup comparison

**STATUS: RESEARCH, 2026-10-16. Ordered API comparison measured; full byte comparison pending.**

**Update, 2026-10-16 01:13 UTC:** The FMC observer replay recovered both
virtual sequence-zero prefixes:387 command+385 status records, zero forward
gaps. Strict census still fails109 anomalies/39 unpaired controls; see
RESULT_FMC1.md and fmc/. Its richer metadata distinguishes envelope and body
statuses: the first observed status-field difference among common aligned calls
is0x20800a87 (native envelope0/body0x56; virtual envelope0x56/erased body).
The collapsed-status comparison below hid that distinction. It remains useful
only within its stated effective-status limits. Full reply bytes were not
exported; no first-body-byte or Code43 cause is proved. H-FMC1 strict-validity
falsifier fired even though attachment/prefix recovery succeeded.

Owned56165326, fresh unprivileged RM tuple arch0x1b0/impl0x6/GB206 on host
595.91.07, UID65534/CapEff0. Native Windows actual32.0.15.8088/GSP580.65.05
passes CUDA copy/kernel/matmul, hardware D3D11/12 and short zero-TDR/Code0
validation. Its complete trace has 4807 command and 4872 status messages,
both sequence-zero prefixes, zero losses/anomalies/unbalanced controls.
See ../native_trace_revisit_20261016/RESULT_HBC4.md.

Virtual source72fca168a, binarySHA256
4250d1a7c4c19db0de5717a8262d356e79b3cdba5dbc216d721d46aa13232773,
host595.91.07/public guestABI580.65.06. Actual process receipts assert the
executable, full KF3 environment and argv. First boot driver binding records
actual32.0.15.8088 and CM_PROB_FAILED_POST_START. Second boot QGA answers,
but evidence.ps1 receive times out and the wrapper exits1 at00:33:54UTC;
there is no second-boot fresh numeric adapter-code or app validation. Own
second VM39327 stopped through QGA/ACPI/exact TERM, exit0 at00:43:43UTC.

The unchanged shared observer exported zero records on both boots. Second
footer: one bootstrap attempt, one invalid bootstrap, zero attached tables.
Therefore these files cannot support a virtual byte-for-byte reply comparison.
qemu.log.gz is sanitized text, compressed locally because raw text exceeds
1MiB. No raw GSP capture or executable was brought back from the rental.

## Ordered API/status comparison

virtual-api.tsv contains382 pretty RPC trace records before first unload;
rpc sequence fields are all0, so alignment uses ordered function/control/class
keys. For trace result "none", effective_result is the same-key explicit
GSP REFUSED ledger status, not captured reply bytes. Other results are pretty
trace statuses. Native responses pair FIFO per function/key; all4807 replies
are accounted for, but metadata omits handles/bodies. Repeated identical keys
can be ambiguous. align_metadata.py bounds the native startup selection to
500 selected calls, aligns order-preserving matching blocks, and writes
aligned-api.tsv:196 common calls,40 status differences. Unmatched operations
and reply bodies remain unexamined. A matching status does not prove equal
output bytes or semantics.

| Native cmd/status seq | API | Native effective status | Virtual ordinal/status | Next native command |
|---|---|---|---|---|
|29/48|0x20800afe INIT_USER_SHARED_DATA|0|28/0x56|seq30:0x20800aff|
|30/49|0x20800aff USER_SHARED_DATA_SET_DATA_POLL|0|29/0x56|seq31:function0x67, RmAlloc class0|
|52/71|0x20800a89 SEND_CMC_LIBOS_BUFFER_INFO|0|51/0x56|seq53:0x20800a80|
|61/80|0x20800ab2 CCU sample size|0|55/0x56|seq62:0x20800ab3 CCU_MAP|
|163/182|0xcbcb RmAlloc, zero params|0|149/0x56|seq164:function0xa RmFree|

The earliest differing status among aligned common calls is0x20800afe, not
proof of the first differing byte or the cause of adapter failure. Native
0x20800a87 also returns0x56, agreeing with virtual startup. Do not replace
all refusals with success. a89/ab2/CBCB remain candidates because their native
successes and virtual refusals are measured. ab4 was absent from native's
successful120s window. Public580.65.06 g_finn_rm_api.h names the matching
0xcbcb00 scheduler namespace, but does not establish constructor semantics.

## Falsifiers and ranked next work

H-BC4 complete native capture: did not fire in one run. H-CCU2 host-derived
cross-ABI sample equality: FIRED; host595 inverse17488 differs from native580
oracle17296. The public unprivileged query is available, but its value cannot
be labelled580 or replaced with a captured constant. No runtime change follows.

1. H-FMC1, one diagnostic change: recognize public FMC mailbox indirection,
   require both sequence-zero prefixes and clean decoder. Replay pending.
2. Compare full replies on common startup calls; investigate source-derived
   CMC/CCU semantics before changing their statuses. Host metadata format needs
   explicit ABI policy after H-CCU2 failed.
3. Investigate the bounded scheduler object lifetime from native alloc/free
   and public interfaces. Namespace naming alone does not justify an ack.

The original rental timer01:39:19UTC/hardcap01:44:19UTC remains binding. Stop
and rank at the wall, then W3 Turing hold/R34; no endless boot/refusal loop.
