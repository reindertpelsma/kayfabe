# Step0: complete Blackwell native/virtual alignment

**STATUS: ANSWERED, 2026-10-16 11:54 UTC. Offline alignment complete; Code43 remains.**

MEASURED input/source observations, no new GPU run or rental. Prompt authority:
/root/codex_prompt_dies_20261016.md section2, Step0. Code-only draftPR21,
source3008bc834 (test-firstf8bb42da1). Nine synthetic regressions fail before
and pass after. The evidence entry point now calls the tested tools aligner;
old aligned-api.tsv remains the original196-call limited snapshot at577f15a7e.

| Die | OS/build | Flags | Status | Evidence |
|---|---|---|---|---|
|GB206|NativeWindows580.88 actual32.0.15.8088/GSP580.65.05|N/A native|MEASURED reference4807commands/4872statuses; no new run|../native_trace_revisit_20261016/revisitwindow580/sequence.tsv|
|GB206|KayfabeWindows580.88, source72fca168a|P19 below plusKF3_RPC_TRACE=1|MEASURED priorCode43; all382indexed calls covered,301APIpattern matches/81virtual-only|aligned-full.tsv|
|GB206|same virtual raw pretty log|sameflags|MEASURED385calls through first explicit unload; two Other(4) and unload were missing from old382-call export|virtual-complete.tsv,aligned-complete.tsv|
|AD102|passingWindows580.88 short-suite source da0569a6|P19|UNRUN ordered display comparison: saved passinglog lacks rpc-trace lines; served-versus-never-issued unknown|TH:ad102_4090/windows/qemu.log|

P19 exact current Windows flag set (recorded process receipt also asserts
KF3_RPC_TRACE=1,20actual variables). This is not a flagless claim:

```text
KF3_ASYNC_PREEMPT=1
KF3_DEFERRED_API=1
KF3_DISPLAY_CAPS_PROBE=1
KF3_DISPLAY_CTRL_PROBE=1
KF3_DISPLAY_ILUT_OFFSET_256=1
KF3_DISPLAY_IMP_ENABLE=1
KF3_DISPLAY_LUT_MIRROR=1
KF3_DISPLAY_SDR_COLOR=1
KF3_GFX_POOL_PROBE=1
KF3_KERNEL_GR_CE=1
KF3_KERNEL_GR_WORK=1
KF3_MEMORY_LIST_PROBE=1
KF3_PREEMPT_BIND_PROBE=1
KF3_SW_RUNLIST_HOST_OWNED=1
KF3_SW_RUNLIST_PROBE=1
KF3_TRANSLATED_CE_RELAY=1
KF3_WIN_TWIN_DEFAPI_OBJECT=1
KF3_WIN_USER_CHANNELS_PASSTHROUGH=1
KF3_ZCULL_BIND_PROBE=1
```

MEASURED first divergence after previous anchor: virtual206/native285 both
0x730108,effectiveNV_OK. Virtual207requests0x730245(GET_EDID_V2), absent from
native. Native286requests0x73011d(GET_CONNECTOR_TABLE), then1960routing
requests0x73010b at287..2246, absent from virtual. Both rejoin on0x730109 at
virtual208/native2247. INFERRED: monitor environment explains this part;
native headless is not a valid connected-display reply oracle.

MEASURED CORRECTION to the prompt's all-OK224..274statement: virtual224
refuses2080080b/effective0x56. All225..274have effectiveNV_OK. Last successful
call274 is RmAlloc classca7a, not an a58 control; the preceding a58 is273.
Native matches the alloc pattern through2311. Virtual275starts
SET_RMFREE_FLAGS50700117,276Free, then more teardown. Native instead issues
73010c at2312..2315 andc3700201 at2316. INFERRED: reply-body or non-RPC state
is a leading candidate; no immediate status failure at the boundary is
measured. Earlier refusals, including224, may affect a later decision; not
excluded. Do not change a refusal to success based on this correlation.

MEASURED exact final RPC before explicit unload: legacyvirtual381 (complete
ordinal383), control20800afe, trace_resultnone/effective0x56 from refusal
ledger. Next completeordinal384 is function47(decimal), UnloadingGuestDriver,
NV_OK, sourceqemu.log.gz line1823. This is distinct from the beginning of
display teardown at legacy275/complete277. Two other extra raw calls are
Other(4) at complete212/218, bothNV_OK. Their semantics are not guessed.

MEASURED scope limits: every native4807 and oldvirtual382 command occurs once
in aligned-full.tsv; every completevirtual385 call is retained separately.
No synchronous native reply is left unpaired. Same-function FIFO association
and sequence API matching remain INFERRED; repeated keys are marked and
handles/bodies absent. Native Free matches in the tail are not corresponding
objects. Separate envelope/body fields stay separate; collapsed effective
equality cannot establish wire-layer equality. Existing virtual FMC strict
census109anomalies/39unpaired controls is not repaired by this analysis.

H-AL0 falsifier, evaluated on synthetic tests and saved metadata: any omitted
command, successful reply-key conflict accepted, hidden status layer or
unknown reply reported as success fails the analysis. Falsifier did not fire
after the fix. Prompt verification found the ordinal224correction above.
H-B1 cause is still INFERRED; Code43 is not fixed by an offline tool.

Next per prompt order: prepare one Blackwell lease, B3console-ablation first
at unchanged180s/samebinary; then same-binary display=off, record display
allocations/teardown/unload ordinals; bounded display replies/a58 bodies.
Use Ada passing same-flags trace as display reference once available. Native
Windows on the same die is the stock-driver reference for display-independent
controls/constants; export screened allowed reply bodies before reimage.
No captured page or guessed constant ships. Constant tiers remain AF/AH:
compute/publicsources across tags, unprivhostquery, privilegedmeasurement,
with a written reason for moving tiers; never neighbour reuse.

B2 oracle-page diagnostic requires owner authorization in the prompt;
header-derived group experiments are already authorized. No page injection
or shipped table performed. Full Ada display-subtree diff is UNRUN until an
ordered passing reference exists. No owner GPU/VM touched.

Cost/time: Step0started11:30UTC, no new rental/compute charges. Campaign
conservative ceiling4.36USD plus unbilled transfers; APIaccount IDs[] verified
11:32UTC. Prior owned leases retired. Windows-on-vast is reference tooling;
installation completion remains distinct from real GPU workload acceptance.
