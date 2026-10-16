# GB206 Windows580 display-on/off reference

**STATUS: ANSWERED, 2026-10-16 20:23UTC. Both Code43; same teardown ordinal falsifies the leading Windows caps explanation.**

MEASURED own56286918/GB206/PCI2d04, host595.91.07, actualWindows580.88 /
32.0.15.8088 and per-deviceGSP1. Sourcef8485f6db/private QEMU SHAe974...25a6,
public guest ABI580.65.06. Both cold processes start from fresh sibling overlays
of the SAME STOPPED driver-bound parent and VARS. Actual process receipts,
flags, cfgmgr32census and owned shutdown logs accompany this result.

|Setting|PID|Actual flags|Code|Commands|Display objects|Explicit unload|
|---|---:|---|---:|---:|---|---:|
|displayon/GOPon/no standardVGA|98863|P19+RPC_TRACE1+RPC_BODY_TRACE1|43|385|CA70once@232;CA7Afour@270/272/274/276;21channelallocations|384|
|displayoff/standardVGA|99358|same21flags|43|356|noNV0073/CA70/CA7A allocation|355|

Exact full21flag sets are in gb206phase-{on,off}-flags.txt. These diagnostics
are not flagless or production acceptance. DWM1/Explorer1 survive on both,
but Code43means no passing NVIDIA Windows cell. Both owned VMs stop cleanly.

On: last initial cursor immediatePIO allocationCA7A@276, display teardown
SET_RMFREE_FLAGS50700117@277, then Free. A previous50700117@234occurs during
normal setup and is NOT the abort boundary. Last RPC before explicit unload is
20800AFE@383; UnloadingGuestDriver47@384. Neither73010C norC3700201
(GET_LOCKPINS_CAPS) is issued. Passing Ada instead continuesC3700201@282after
its lastC67Acursor@281without the native-headless immediate four73010Cbatch.

Off: teardown also begins@277, immediately after refused RmAllocCFB0@276,
with NO display class allocated. No CFB0allocation occurs in on. Explicit
UnloadingGuestDriver shifts to355; preceding RPC is20800AFE@354. My initial
comparison used only the final fn47ordinal and incorrectly said the prompt's
same-ordinal falsifier did not fire. The correct start-of-teardown comparison
is277in BOTH. The literal H-B1rank1 falsifier FIRED: no display classes yet the
same teardown ordinal. Follow the prompt and deprioritize Windows thin caps;
restart from the first unaligned RPC in the new trace.

MEASURED earlier divergence:20800A4B@16has on0/offnone; API keys first differ
at17. Off then runs a different allocation path and eventually refusesCFB0,
where on accepts a cursor. INFERRED limitation: equal numerical ordinals alone
do not prove a shared causal wall in differently aligned sequences. A thin
page could still explain the originalon failure, but it is not a sufficient
explanation for the pair and remains unproved. Linux B2's independent sourced
mode-validation hypothesis still gets its single allowed experiment. No CFB0
ack is guessed or stacked onto that experiment.

On exports44posted public NV0073/a58body records, all untruncated, and21known
public PIO/DMAallocation summaries. All21allocation body/envelope statuses are
0; lastCA7Ahas channelInstance3, hObjectNotify0 and pControl_nonzero=true.
pControl VALUES are omitted; no host/guest pointer is published.48opaque or
other-allocation records are omitted before return. Off exports no public
display body/allocation record and omits89opaque/allocation records. Metadata
quotas never truncate either run; full385/356command sequences are retained.
Bodies of Other4/static/board/private controls are not inferred or published.

Policy trace_result precedes held actions and effective_result staysUNKNOWN;
actual posted statuses exist only for the screened body subset. Aligner matches
255APIkeys between Ada1000and on385,252between on385and off356. Genericnative_*
columns mean the first VIRTUAL reference here, never physicalVFIO. Matching
and FIFO association are inferred; no body equality or semantic identity claim.

Next is ONE source-derived virtual-tiling group (PR29/integration36d134bc5):
compare Linux mode validation and record any change at this Windows on boundary. Unchanged
unload/mode rejection falsifies the candidate within the stated1.5h B2box.
No full native page injection, guessedCBCB/CCUreply or private-interface value.
CBCB remains NOT DERIVABLE (prior native120byte bodies/43distinct replies).
