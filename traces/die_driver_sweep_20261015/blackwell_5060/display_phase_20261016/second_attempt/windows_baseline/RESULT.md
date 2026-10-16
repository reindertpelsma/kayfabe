# GB206 Windows580 display-on/off reference

**STATUS: ANSWERED, 2026-10-16 20:14UTC. Both Code43; tiling experiment pending.**

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

Off: explicit unload shifts to355; preceding RPC remains20800AFE@354. No
display object subtree exists. The earliest policy-status difference is already
20800A4B@16: on0/offnone; positional API keys diverge at17. Thus removing
display changes earlier initialization as well as omitting29commands. It is
not a clean same-abort-point result. H-B1-off's specific same-ordinal/no-display
class falsifier did NOT fire. INFERRED: an independent display-off refusal may
also explain its failure; this does not identify the cause of the originalon
abort. Rank1is neither proved nor killed by this mixed comparison.

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
compare Linux mode validation and this exact Windows on boundary. Unchanged
unload/mode rejection falsifies the candidate within the stated1.5h B2box.
No full native page injection, guessedCBCB/CCUreply or private-interface value.
CBCB remains NOT DERIVABLE (prior native120byte bodies/43distinct replies).
