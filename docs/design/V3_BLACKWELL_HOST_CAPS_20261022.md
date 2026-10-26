**STATUS: RESEARCH, 2026-10-22.** Public access candidate; whole-PRECOMP guest overlay HELD after owner-directed tier audit. Hardware n=0; no rental.

2026-10-22 owner-guidance correction: derive each field from public/generated sources and the
virtual implementation before considering host facts. The whole-word T2 override described below
is the current code, not the approved final design: it can overwrite existing T1 rules and advertise
unimplemented CA features. PR85 remains draft/HELD pending a field-level revision. Precision14 is
an inferred virtual candidate, not a proven physical-generation constant. Complete41-field inventory,
GB202/203/205/206/207 layout mechanism, per-field tiers and falsifiers:
[V3_BLACKWELL_PRECOMP_TIER_AUDIT_20261022.md](V3_BLACKWELL_PRECOMP_TIER_AUDIT_20261022.md).
No captured page is imported or replayed in this review. The earlier access audit remains valid.


The clean precision-only ON failure leaves one named CAPC difference: CSC0LUT_FVLUT.
The earlier native read was privileged BAR0 mapping, not an unprivileged RM control. The
public unprivileged alternative is implemented below, subject to the driver's register allowlist; do not treat the old tool as a runtime T2 source.
This is a field-level comparison, not a pasted page or a production captured-word table.

CODE READ (commit 007ba1eb0:18): quoted historical statement, evidence not revalidated by BW9. MEASURED retained input: own GB202 native measurement text SHA256
`6f9d39cff8daf9681d19e2560c919094dbc07ed59616b6a13497ee466efb6287`.
CODE READ derived comparison: product8f3a9bb3a, caps_rows.tsv SHA256
`4a6d22792c9cf48d29b8a4b82bb6ac05515c969d31a1aab1b49a482f13f0e2d3`.
The native input's header says read-only32-bit BAR0 loads at the display capabilities aperture.
No new host observation is made here, and the page is not claimed to be freshly read on57148807.

| Public CAPC field | Bits | Native windows0–7 | Derived windows0–7 | Meaning / source boundary |
|---|---|---|---|---|
| CSC00_PRECISION |4:0|14|14|Precision declared for the first CSC matrix. NVKMS documents S5.14 coefficients; the class header can encode14 but does not require it. |
| CSC00_UNITY_CLAMP |5|0|0|Boolean unity-clamp capability for that matrix. Header gives TRUE/FALSE, not complete clamp arithmetic. |
| CSC0LUT_FVLUT |13|1|0|The sole differing named CAPC field: CSC0 LUT's FVLUT capability. The acronym's expansion and complete behavior are not defined by the capability declaration. |
| CSC0LUT_DIRECT |15|0|0|Separate direct LUT capability. It is not interchangeable with FVLUT. The capability header alone does not give the runtime mapping. |
| CSC01_PRECISION |20:16|14|14|Precision of the CSC matrix after the CSC0 LUT stage; same numeric-source limitation as CSC00. |
| CSC01_UNITY_CLAMP |21|0|0|Separate unity-clamp capability for CSC01; same documented scope as CSC00. |

All six named fields are covered. No unassigned bit is set in the retained native CAPC words.
Native slots8–31 have all six fields zero; the virtual page's inactive slots are zero. The generated
CAPC layout has32 slots, while this comparison's active set is0–7. This says nothing about CAPA/B/D/E/F
or POSTCOMP. Machine-readable decoded-only receipt: traces/blackwell_host_caps_review_20261022/field_comparison.json.

CODE READ (commit 007ba1eb0:39): quoted historical statement, evidence not revalidated by BW9. Public source: NVIDIA/open-gpu-kernel-modules580.159.04, MIT,
`b81d58ee0224d1d290bef1c080592b619e184042`,
[clca73.h:846–861](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/b81d58ee0224d1d290bef1c080592b619e184042/src/common/sdk/nvidia/inc/class/clca73.h#L846).
This defines every compared field and the differing FVLUT field's boolean encoding. It does NOT
say that GB202 must set FVLUT or that the virtual pipeline implements it. The observed1 is a
host measurement fact, not a public-source derivation.
[clca7e.h:606–621](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/b81d58ee0224d1d290bef1c080592b619e184042/src/common/sdk/nvidia/inc/class/clca7e.h#L606)
defines normalization scaling, interpolation, mirror, enable and CURVE selections PQ/HLG/DIRECT.
Those are relevant public method semantics; neither this header nor the inspected NVKMS source
establishes an equivalence between CURVE support and the FVLUT capability. Do not invent that rule.

CODE READ (commit 007ba1eb0:50): quoted historical statement, evidence not revalidated by BW9. MEASURED earlier whole-CAPC replacement avoided0x50 but its native baseline had an unmatched32.
The clean precision-only ON reproduces0x50. INFERRED candidate: FVLUT is the remaining relevant
CAPC variable. FALSIFIER for any later lawful host-sourced arm:0x50 persists with same-die CAPC
successfully obtained, while the source/binary/flags and fresh sibling remain fixed. No captured
fallback or blanket FVLUT declaration follows from the current comparison.


Public access audit (CODE READ)

CODE READ (commit 007ba1eb0:59): quoted historical statement, evidence not revalidated by BW9. The original `read_display_caps_page.py` opens sysfs BAR0 `resource0` and maps it read-only.
That is a privileged measurement route (tier3), even though it performs no writes. It is not
an RM control kayfabe may silently substitute for an unprivileged runtime query.

The dedicated capability object does not solve this: `NVCA73_DISP_CAPABILITIES` is declared
`RS_FLAGS_ALLOC_PRIVILEGED` in [resource_list.h:1381](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/b81d58ee0224d1d290bef1c080592b619e184042/src/nvidia/src/kernel/rmapi/resource_list.h#L1381).
Its public implementation supplies an aperture base/size, not a lawful unprivileged page-read
control. `NV5070_CTRL_CMD_SYSTEM_GET_CAPS_V2` is a one-byte semantic flags table; it has no
CSC precision or FVLUT page payload. Neither is the implementation used here.

There is a distinct candidate: `NV2080_CTRL_CMD_GPU_EXEC_REG_OPS` (0x20800122), on the host's
own subdevice. The generated [595.91.07 control export](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/9f087a6d4e86d85acc0ce1d354d6276fe5047b29/src/nvidia/generated/g_subdevice_nvoc.c#L235)
carries flags0x10118 including `RMCTRL_FLAGS_NON_PRIVILEGED`. The [public register validator](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/9f087a6d4e86d85acc0ce1d354d6276fe5047b29/src/nvidia/src/kernel/gpu/gpu_access.c#L1177)
still applies `gpuGetUserRegisterAccessPermissions` when `osIsAdministrator()` is false.
The ordinary control calls validation with permission skipping false. Linux defines
`NV_IS_SUSER()` as `capable(CAP_SYS_ADMIN)` in
[nv-linux.h:502](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/9f087a6d4e86d85acc0ce1d354d6276fe5047b29/kernel-open/common/inc/nv-linux.h#L502);
clearing the effective capability therefore removes that administrator bypass even for UID0.
The per-offset GSP permission result for PRECOMP is not established by this source. A callable
unprivileged API is not proof that this range is readable. The GSP-plugin-only NOPTRS variant
is not used. Earlier suggestions that no unprivileged route exists are superseded by this
qualified candidate; the original root reader remains privileged.

Implementation and limits (CODE READ; hardware n=0)

`kf-host::precomp` authors READ32/GLOBAL requests, never guest bytes. Exact compound ABI layouts
were compiled from the five public SDKs listed in the adjacent `abi/sources.json`:580.65.06,
580.94.02,580.159.04,590.48.01,595.91.07. Each C measurement gives params48bytes, rows32bytes;
count offset20, pointer24, row operation0/type1/status2/register12/value20. The generator
reproduces the committed Rust layout. Unknown versions fail before an ioctl; there is no
nearest-version or scalar bench-ABI bypass. This supports only those measured host SDKs,
independently of the Windows guest's actual580.88 package.

At realize, the private display thread clears effective CAP_SYS_ADMIN for the synchronous
query and checks that posture again in HostRm. The host's generated class/register layout
forms six named PRECOMP words per active window, at most32windows/192words, in batches of32.
No public raw-offset CLI, privileged BAR mapping or file-backed page is added. Initial row
status is NOACCESS. Any unsupported ABI, absent control, denied row, changed request metadata,
short/long reply or all-zero/all-ones snapshot abandons the complete snapshot. There is no
partial host/derived PRECOMP blend. An unavailable query keeps the existing complete T1
PRECOMP rule. Other capability-page regions retain their existing derivation and precedence.

CODE READ (commit 007ba1eb0:101): quoted historical statement, evidence not revalidated by BW9. A successful immutable snapshot is tied to die architecture/implementation, caps class, active
window count and the full public PRECOMP layout. A guest vocabulary change cannot reinterpret
it. T2 replaces the corresponding T1 words, including host zeros, and records Host provenance;
it does not OR derived bits into the measured values. No captured word, per-die table or forced
FVLUT rule is in this topic. No new call runs on a vCPU, heartbeat drainer or lock a vCPU takes.
Protected channel/relay/lifecycle files are unchanged in the primary topic.

`cargo build --release -j2 -p kf-qemu --example precomp_host_read` builds a small native H0
probe. It drops CAP_SYS_ADMIN before opening its own RM client and calls the identical bounded
query. Its only argument selects the GPU; it prints identity/source/count, not register words.
A refusal exits2. H0 must run after clean N0 and before expensive Windows installation. Both
H0 success and the actual QEMU T2-consumption receipt are required for a host-read validation
claim. A fallback boot is permitted product behavior but is not a measured host-read arm.

This reads physical capabilities; it does not prove that the virtual pipeline implements all
advertised FVLUT behavior. Public headers define the field and distinguish DIRECT but do not
settle FVLUT's expansion, supported curve correspondence or complete arithmetic. Those semantics
and Windows acceptance remain open. The next run's falsifiers, proof capture and budget are in
[V3_BLACKWELL_HOST_PRECOMP_VALIDATION_20261022.md](V3_BLACKWELL_HOST_PRECOMP_VALIDATION_20261022.md).
Test-first commits, counts, mutation killers and lint comparison are in the adjacent runtime receipt.
