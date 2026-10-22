**STATUS: RESEARCH, 2026-10-22.** GPU-free CAPC comparison and host-read access audit; no hardware/rental authorized by this document.

The clean precision-only ON failure leaves one named CAPC difference: CSC0LUT_FVLUT.
The earlier native read was privileged BAR0 mapping, not an unprivileged RM control. The
unprivileged alternative remains under review; do not treat the old tool as a runtime T2 source.
This is a field-level comparison, not a pasted page or a production captured-word table.

MEASURED retained input: own GB202 native measurement text SHA256
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

Public source: NVIDIA/open-gpu-kernel-modules580.159.04, MIT,
`b81d58ee0224d1d290bef1c080592b619e184042`,
[clca73.h:846–861](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/b81d58ee0224d1d290bef1c080592b619e184042/src/common/sdk/nvidia/inc/class/clca73.h#L846).
This defines every compared field and the differing FVLUT field's boolean encoding. It does NOT
say that GB202 must set FVLUT or that the virtual pipeline implements it. The observed1 is a
host measurement fact, not a public-source derivation.
[clca7e.h:606–621](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/b81d58ee0224d1d290bef1c080592b619e184042/src/common/sdk/nvidia/inc/class/clca7e.h#L606)
defines normalization scaling, interpolation, mirror, enable and CURVE selections PQ/HLG/DIRECT.
Those are relevant public method semantics; neither this header nor the inspected NVKMS source
establishes an equivalence between CURVE support and the FVLUT capability. Do not invent that rule.

MEASURED earlier whole-CAPC replacement avoided0x50 but its native baseline had an unmatched32.
The clean precision-only ON reproduces0x50. INFERRED candidate: FVLUT is the remaining relevant
CAPC variable. FALSIFIER for any later lawful host-sourced arm:0x50 persists with same-die CAPC
successfully obtained, while the source/binary/flags and fresh sibling remain fixed. No captured
fallback or blanket FVLUT declaration follows from the current comparison.
