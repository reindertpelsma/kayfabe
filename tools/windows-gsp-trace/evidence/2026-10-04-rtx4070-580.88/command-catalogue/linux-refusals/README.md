# Consequences of rejecting Linux-observed Windows control IDs

**STATUS: RESEARCH, 2026-10-04. Existing-run audit; no new failure injection.**

The original 68 Linux-counterexample IDs split into **35 observed refused**, **15 served in the guest**, and **18 seen only in native Linux samples**. `result=none` alone is not a returned status: the refusal ledger independently confirms `0x56` for all 34 such IDs, plus one selector of GPU_GET_INFO_V2.

Baseline: NVIDIA Linux 580.65.06 on GA106, Kayfabe binary `b98bdbec`, retained in `v3-windows` at `c50fad9a`. The same captured boot reaches SMI_RC=0 and display handoff with black_frames=0. Its display probe build **failed** and modetest reported no connected output: this is not a complete display-feature pass. Separate saved CUDA ladder boots of the same build pass cup2/cup3/cup8; they do not independently correlate each of these 35 refusals to a CUDA call.

The result proves survival of the observed initialization path, not harmlessness across workloads, dies, versions or Windows. The 15 successful IDs were not deliberately rejected. For the 18 native-only IDs there is no corresponding guest response here. Unknown feature effects stay unknown.

The prior [v3 refusal audit](../../../../../../docs/design/V3_REFUSAL_AUDIT.md) is essential context: refusing MC_SERVICE_INTERRUPTS once ended waits early; a preemption-mode refusal broke one Vulkan application despite other Vulkan tests passing; a missing SM-issue-rate query was fatal on Blackwell but unreached on GA102. These are separate examples, not newly demonstrated failures among all 68 rows.

[Machine-readable evidence and hashes](refusals.json). Baseline logs are under [baseline/](baseline/). Source interpretations below use OGKM 580.65.06 at `307159f2623d3bf45feb9177bd2da52ffbc5ddf9`; the golden-channel fallback additionally has the historical audit above.

| ID | Observed guest result | Consequence / limit |
|---|---|---|
| `0x00730102` | 0x0×4 | No refusal experiment for this call/selector is present in this baseline. |
| `0x00730108` | 0x0×5 | No refusal experiment for this call/selector is present in this baseline. |
| `0x0073010c` | 0x0×16 | No refusal experiment for this call/selector is present in this baseline. |
| `0x0073012c` | none×1; posted 0x56 (ledger line 2083) | VRR notification: TellRMAboutVrrHead logs a warning and continues (OGKM 580.65.06 nvkms-vrr.c:418-423). VRR correctness was not tested. |
| `0x00730250` | 0x0×1 | No refusal experiment for this call/selector is present in this baseline. |
| `0x0073028b` | 0x0×4 | No refusal experiment for this call/selector is present in this baseline. |
| `0x007302a4` | 0x0×2 | No refusal experiment for this call/selector is present in this baseline. |
| `0x00731140` | 0x0×1 | No refusal experiment for this call/selector is present in this baseline. |
| `0x00731142` | 0x0×3 | No refusal experiment for this call/selector is present in this baseline. |
| `0x00731144` | none×1; posted 0x56 (ledger line 2079) | ELD audio capabilities: nvkms-hdmi.c:1147-1168 logs the failure and continues audio-power cleanup. Audio setup may be lost; not a tested audio pass. |
| `0x00800294` | none×2; posted 0x56 (ledger line 1019) | Per-call feature loss not isolated; no global GPU-init failure in this boot. The public meaning does not prove the userspace caller ignores failure. |
| `0x20800102` | 0x0×2, 0x56×1; posted 0x56 (ledger line 2130) | Selector-dependent: two served requests and one refusal. Log names UnmeasuredForwardedIndex 35. Other selectors, including Windows requests, need separate treatment. |
| `0x2080012b` | none×2; posted 0x56 (ledger line 919) | Golden-image kernel channel in this boot has no twin. Prior v3 refusal audit documents lazy-context fallback for NOT_SUPPORTED; this does not justify refusing user-channel promotion. |
| `0x2080012f` | none×1; posted 0x56 (ledger line 2344) | Per-call feature loss not isolated; no global GPU-init failure in this boot. The public meaning does not prove the userspace caller ignores failure. |
| `0x2080014b` | none×10; posted 0x56 (ledger line 1041) | Per-call feature loss not isolated; no global GPU-init failure in this boot. The public meaning does not prove the userspace caller ignores failure. |
| `0x20800156` | none×1; posted 0x56 (ledger line 2150) | Per-call feature loss not isolated; no global GPU-init failure in this boot. The public meaning does not prove the userspace caller ignores failure. |
| `0x20800157` | none×2; posted 0x56 (ledger line 1045) | Per-call feature loss not isolated; no global GPU-init failure in this boot. The public meaning does not prove the userspace caller ignores failure. |
| `0x208001a4` | none×1; posted 0x56 (ledger line 2139) | Per-call feature loss not isolated; no global GPU-init failure in this boot. The public meaning does not prove the userspace caller ignores failure. |
| `0x20800a38` | none×1; posted 0x56 (ledger line 1219) | FECS tracing: fecs_event_list.c:1527-1535 returns from tracing setup on failure. The tracing feature is affected; this boot continues. |
| `0x20800aff` | none×9; posted 0x56 (ledger line 603) | Shared-data polling: gpu_user_shared_data.c:369-378 propagates the failure and does not update the requested polling mask. This is not a successful no-op; this boot still initializes. |
| `0x20801208` | not observed | No refusal experiment for this call/selector is present in this baseline. |
| `0x20801303` | not observed | No refusal experiment for this call/selector is present in this baseline. |
| `0x20801322` | none×1; posted 0x56 (ledger line 2352) | Per-call feature loss not isolated; no global GPU-init failure in this boot. The public meaning does not prove the userspace caller ignores failure. |
| `0x20801344` | none×1; posted 0x56 (ledger line 2356) | Per-call feature loss not isolated; no global GPU-init failure in this boot. The public meaning does not prove the userspace caller ignores failure. |
| `0x20801357` | none×2; posted 0x56 (ledger line 1061) | Per-call feature loss not isolated; no global GPU-init failure in this boot. The public meaning does not prove the userspace caller ignores failure. |
| `0x20801813` | none×2; posted 0x56 (ledger line 2196) | Per-call feature loss not isolated; no global GPU-init failure in this boot. The public meaning does not prove the userspace caller ignores failure. |
| `0x20801819` | none×2; posted 0x56 (ledger line 2203) | Per-call feature loss not isolated; no global GPU-init failure in this boot. The public meaning does not prove the userspace caller ignores failure. |
| `0x20801823` | 0x0×3 | No refusal experiment for this call/selector is present in this baseline. |
| `0x20801829` | none×1; posted 0x56 (ledger line 2210) | Per-call feature loss not isolated; no global GPU-init failure in this boot. The public meaning does not prove the userspace caller ignores failure. |
| `0x20801830` | none×1; posted 0x56 (ledger line 2214) | Per-call feature loss not isolated; no global GPU-init failure in this boot. The public meaning does not prove the userspace caller ignores failure. |
| `0x20802a08` | 0x0×10 | No refusal experiment for this call/selector is present in this baseline. |
| `0x20803083` | none×1; posted 0x56 (ledger line 2146) | Per-call feature loss not isolated; no global GPU-init failure in this boot. The public meaning does not prove the userspace caller ignores failure. |
| `0x20803400` | not observed | No refusal experiment for this call/selector is present in this baseline. |
| `0x20803401` | none×1; posted 0x56 (ledger line 2340) | Per-call feature loss not isolated; no global GPU-init failure in this boot. The public meaning does not prove the userspace caller ignores failure. |
| `0x20803404` | none×1; posted 0x56 (ledger line 2348) | Per-call feature loss not isolated; no global GPU-init failure in this boot. The public meaning does not prove the userspace caller ignores failure. |
| `0x20808159` | not observed | No refusal experiment for this call/selector is present in this baseline. |
| `0x2080852a` | none×1; posted 0x56 (ledger line 2368) | Per-call feature loss not isolated; no global GPU-init failure in this boot. Semantics remain unresolved. |
| `0x2080852c` | not observed | No refusal experiment for this call/selector is present in this baseline. |
| `0x2080852e` | none×2; posted 0x56 (ledger line 1066) | Per-call feature loss not isolated; no global GPU-init failure in this boot. Semantics remain unresolved. |
| `0x2080852f` | not observed | No refusal experiment for this call/selector is present in this baseline. |
| `0x20808536` | none×1; posted 0x56 (ledger line 2377) | Per-call feature loss not isolated; no global GPU-init failure in this boot. Semantics remain unresolved. |
| `0x2080853a` | not observed | No refusal experiment for this call/selector is present in this baseline. |
| `0x20808542` | not observed | No refusal experiment for this call/selector is present in this baseline. |
| `0x20808546` | none×18; posted 0x56 (ledger line 2234) | Per-call feature loss not isolated; no global GPU-init failure in this boot. Semantics remain unresolved. |
| `0x20809001` | not observed | No refusal experiment for this call/selector is present in this baseline. |
| `0x20809004` | none×4; posted 0x56 (ledger line 2403) | Per-call feature loss not isolated; no global GPU-init failure in this boot. Semantics remain unresolved. |
| `0x20809019` | not observed | No refusal experiment for this call/selector is present in this baseline. |
| `0x20809038` | none×1; posted 0x56 (ledger line 2303) | Per-call feature loss not isolated; no global GPU-init failure in this boot. Semantics remain unresolved. |
| `0x2080a026` | not observed | No refusal experiment for this call/selector is present in this baseline. |
| `0x2080a028` | not observed | No refusal experiment for this call/selector is present in this baseline. |
| `0x2080a080` | none×1; posted 0x56 (ledger line 2229) | Per-call feature loss not isolated; no global GPU-init failure in this boot. Semantics remain unresolved. |
| `0x2080a084` | 0x0×1 | No refusal experiment for this call/selector is present in this baseline. |
| `0x2080a0a4` | not observed | No refusal experiment for this call/selector is present in this baseline. |
| `0x2080a0a7` | none×1; posted 0x56 (ledger line 2424) | Per-call feature loss not isolated; no global GPU-init failure in this boot. Semantics remain unresolved. |
| `0x2080a0a8` | not observed | No refusal experiment for this call/selector is present in this baseline. |
| `0x2080a0d1` | not observed | No refusal experiment for this call/selector is present in this baseline. |
| `0x2080a0f2` | none×1; posted 0x56 (ledger line 2395) | Per-call feature loss not isolated; no global GPU-init failure in this boot. Semantics remain unresolved. |
| `0x2080a612` | none×2; posted 0x56 (ledger line 1091) | Per-call feature loss not isolated; no global GPU-init failure in this boot. Semantics remain unresolved. |
| `0x2080a618` | none×2; posted 0x56 (ledger line 1086) | Per-call feature loss not isolated; no global GPU-init failure in this boot. Semantics remain unresolved. |
| `0x2080a63c` | none×1; posted 0x56 (ledger line 2390) | Per-call feature loss not isolated; no global GPU-init failure in this boot. Semantics remain unresolved. |
| `0x20810108` | none×3; posted 0x56 (ledger line 1025) | Per-call feature loss not isolated; no global GPU-init failure in this boot. Semantics remain unresolved. |
| `0x90e70113` | not observed | No refusal experiment for this call/selector is present in this baseline. |
| `0x90f10106` | 0x0×9 | No refusal experiment for this call/selector is present in this baseline. |
| `0xa06c0101` | not observed | No refusal experiment for this call/selector is present in this baseline. |
| `0xa06c0103` | not observed | No refusal experiment for this call/selector is present in this baseline. |
| `0xa06f0103` | 0x0×4 | No refusal experiment for this call/selector is present in this baseline. |
| `0xc3700104` | 0x0×14 | No refusal experiment for this call/selector is present in this baseline. |
| `0xc3720101` | 0x0×30 | No refusal experiment for this call/selector is present in this baseline. |
