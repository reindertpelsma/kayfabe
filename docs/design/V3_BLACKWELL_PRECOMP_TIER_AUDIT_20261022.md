**STATUS: RESEARCH, 2026-10-22.** Owner-directed derivation-first review; documentation/evidence only. Whole-PRECOMP publication in draft PR85 is HELD pending a field-level revision. No hardware or rental.

The current host-read code answers a physical-hardware question, but it cannot by itself decide
what the virtual display implements. Tier1 must remain first for each field. The whole-word T2
overlay in codeab91b4acc replaces already authored T1 fields and can introduce capabilities whose
CA implementation is unverified. Its previous release/mutation results prove that overlay's coded
contract, not compliance with this revised field-level contract. Keep PR85 draft and do not execute
the prior whole-page ON plan from it. This review changes no production code or shipped constant.

I read V3_CONSTANT_STRATEGY.md completely, OWNER_RULINGS §AF/§AH, kf_rm::consttab and its validators,
kf_chip::display family rows, the generated class tables, caps::page/sdr_page, caps_rows.tsv,
capsderive::derive, hostcaps and the color decoder/GPU kernel before considering capture use.
No native capture was replayed, imported or used as an input to this review's inventory. The older
field comparison remains validation evidence; it does not supply a replacement page or family value.

What the public mechanism establishes for the generation

Five pinned ogkm tags580.65.06/580.94.02/580.159.04/590.48.01/595.91.07 explicitly map
GB202,GB203,GB205,GB206,GB207 to DISPv0502 and list CA73 capabilities plus CA7E window classes.
This is a mechanism for sharing a layout, not a mechanism for equal physical capability values.
The generated CA73 PRECOMP vocabulary has41 named fields across CAPA–F, identical in these five
tags. The audit verifies25 die/tag memberships and all five complete field layouts. Sources and
hashes are in traces/blackwell_host_caps_review_20261022/tier_audit/{family_mechanism,inventory}.json.

Pinned public sources: ogkm595.91.07 `9f087a6d4e86d85acc0ce1d354d6276fe5047b29`,
[g_chips2halspec_nvoc.h:150](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/9f087a6d4e86d85acc0ce1d354d6276fe5047b29/src/nvidia/generated/g_chips2halspec_nvoc.h#L150),
[g_gpu_class_list.c:2830](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/9f087a6d4e86d85acc0ce1d354d6276fe5047b29/src/nvidia/generated/g_gpu_class_list.c#L2830),
and [clca73.h](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/9f087a6d4e86d85acc0ce1d354d6276fe5047b29/src/common/sdk/nvidia/inc/class/clca73.h).
The class header names masks, array bounds and Boolean encodings; it does not mandate a native
precision14 or native FVLUT=TRUE. Shared fields are not permission to use GB202's observed values
on GB203/205/206. Later dies use this same algorithm only when their own public identity/class/IP
binding is present and the required vocabulary/semantics match. Unknown membership is refused by
name; no catch-all architecture match or neighbouring-die row is introduced.

CAPC per-field disposition

The virtual and physical questions are kept separate below. T1-unclaimed means the virtual engine
claims no support; it does not assert that physical hardware lacks the feature. No T2 value has
been observed through the new runtime control on any die (n=0).

| CAPC field | Public bits | Tier1 virtual rule today | Physical fact / host-read status | What remains unresolved |
|---|---|---|---|---|
|CSC00_PRECISION|4:0|Candidate14 for an existing virtual CSC stage, using the same family-layer rule and generated encoding on every known class|T2 query candidate can observe own die; permission unverified|Mapping public coefficient precision to this capability is inferred; complete numeric fidelity and native generation-wide value not proved|
|CSC00_UNITY_CLAMP|5|T1-unclaimed/FALSE; no independent clamp capability advertised|Native state could be a T2 fact; never borrowed from GB202|Full unity-clamp arithmetic and truthful TRUE implementation|
|CSC0LUT_FVLUT|13|T1-unclaimed/FALSE pending implementation/meaning; no TRUE rule derived|T2 candidate physical fact only; not proof the virtual stage supports it|FVLUT capability-to-method correspondence, complete curve behavior and Windows requirement|
|CSC0LUT_DIRECT|15|T1-unclaimed/FALSE; distinct from FVLUT and from the method CURVE_DIRECT selector|T2 candidate physical fact only|No proven equivalence between this capability and selecting CURVE_DIRECT|
|CSC01_PRECISION|20:16|Same candidate14 rule for the second existing virtual stage|Same T2 access limitation|Same inference/numeric-fidelity limitation as CSC00|
|CSC01_UNITY_CLAMP|21|T1-unclaimed/FALSE for this separate stage|T2 candidate physical fact only|Separate clamp behavior; do not infer it from CSC00|

The precision candidate's independent numeric source is ogkm580.159.04
`b81d58ee0224d1d290bef1c080592b619e184042`,
[nvkms-evo3.c:374](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/b81d58ee0224d1d290bef1c080592b619e184042/src/nvidia-modeset/src/nvkms-evo3.c#L374),
which describes selected CSC matrices as S5.14 within software S5.16 method values. That motivates
four existing family-layer rows (main/overlay times CSC00/01). It does not prove that all physical
GB20x CSC engines report14, or that every virtual rounding/output path matches the capability.
The GPU kernel decodes coefficients with16 fractional bits and has both FP16 and fixed14 output
paths; these are different facts. A header accepting14 is not a derivation of14. Precision-only
Windows sufficiency was already falsified on clean GB202 and remains falsified.

The full PRECOMP disposition (all41 fields, not just the deciding word)

The machine-readable precomp_fields.tsv lists EVERY field, bit range, current T1 main/overlay
output and source-policy category. Values there are decoded from the committed authored family
report, not a native page or a fresh acceptance measurement. Categorization describes current
source provenance; it does not certify full CA numerical/feature fidelity.

| Group | Tier1 current source / policy | Remaining gap |
|---|---|---|
|CAPA CSC00/01/10/11_PRESENT, CSC0/1LUT_PRESENT|Existing virtual stage-presence claims|Legacy LUT presence is not proof of CA fixed-curve semantics; audit the CA path before increasing claims|
|CAPA SCLR_PRESENT / TMO_PRESENT|Virtual layer role and bounded SDR implementation; scaler on main layer, TMO in SDR path|A physical host's different pipeline cannot overwrite these design choices|
|CAPA GMA_PRESENT|T1-unclaimed|No implemented gamut-map feature justified by a native TRUE bit|
|CAPA FULL_WIDTH / UNIT_WIDTH / ALPHA_WIDTH|Unresolved explicit numeric rule; current zeros are not a proven native answer|Derive virtual limits and field meaning; use T2 only if a physical limit is actually relevant|
|CAPB ILUT_LOGSZ / LOGNR / DIRECT / SFCLOAD|Bounded DIRECT10 SDR LUT implementation and surface-load path|Independent of physical board; keep the virtual limits|
|CAPB FMT_PRECISION|Unresolved|Header gives encoding, not the correct virtual/native precision|
|CAPC|The six rows above|Two inferred precision declarations; four unclaimed Boolean features|
|CAPD TMO_LOGSZ / LOGNR / SFCLOAD|Bounded segmented SDR tone table and surface loading|Keep authored limits; full color correctness remains a separate oracle question|
|CAPD TMO_DIRECT|T1-unclaimed|The implemented segmented tone path does not justify a direct-mode claim|
|CAPD SCLR_SF_PRECISION / CI_PRECISION / VS_EXT_RGB / EXT_ALPHA / VS_MAX_SCALE_FACTOR / HS_MAX_SCALE_FACTOR|Unresolved|Six distinct semantics/limits, not a captured word to apply wholesale|
|CAPE CSC10_PRECISION / CSC11_PRECISION|Unresolved; no corresponding14 rule currently authored|Do not extend CAPC's inferred rule automatically to these stages|
|CAPE CSC10/11_UNITY_CLAMP, CSC1LUT_DIRECT / FVLUT|T1-unclaimed|Separate output-side stages require their own public behavior/implementation evidence|
|CAPF VSCLR_MAX_PIXELS_2TAP / 5TAP|Unresolved|Derive virtual scaler limits after establishing field semantics; no assumed physical zeros|

Totals:15 existing T1-authored fields,2 T1-authored precision candidates,10 explicitly unclaimed
fields and14 unresolved numeric/behavioral fields. All41 physical values are candidates for a
same-die read, but NONE is a verified runtime host-read result and none is automatically approved
as a virtual capability. Unassigned bits stay outside any field-level rule.

Why higher tiers are still needed, and what they may answer

T1 search covered the five pinned public ogkm SDKs, generated HAL/class bindings, NVKMS and
kayfabe's implementation. Literal FVLUT references in those available source checkouts are
recorded; some checkouts are sparse, so this is not an exhaustive absence proof. Nouveau's
MIT CA window/head files were also inspected at pinned Linux commit
`7be4bc52fd9e0e55dc9a2e0232b89427d2c4b614`:
[wndwca7e.c](https://github.com/torvalds/linux/blob/7be4bc52fd9e0e55dc9a2e0232b89427d2c4b614/drivers/gpu/drm/nouveau/dispnv50/wndwca7e.c)
and [headca7d.c](https://github.com/torvalds/linux/blob/7be4bc52fd9e0e55dc9a2e0232b89427d2c4b614/drivers/gpu/drm/nouveau/dispnv50/headca7d.c).
They give public method usage, including CA surface-backed ILUT/OLUT setup, not a full CAPC/FVLUT
capability-value rule. No general claim about all nouveau/nova or all historical tags is made.
This bounded search did not establish a family-wide native FVLUT=TRUE mechanism.

CA7E publicly exposes FP_NORM_SCALE and PQ/HLG/DIRECT curve selectors. The current color::pipeline
accepts only enable+interpolate with no mirror in its legacy inline CSC LUT path; nonzero CA CURVE
bits are refused. It does not implement a complete CA curve/normalization contract. Merely finding
those method names cannot establish FVLUT's complete semantics. This is the concrete reason a
native bit cannot be turned directly into a virtual feature declaration. The open-source method
implementation and numerical oracles remain the next derivation work, not another captured page.

T2 is justified only for a residual question about this host's physical capabilities/limits that
cannot be answered from the public mechanism or virtual state. The bounded unprivileged query is
still useful for such facts and as an independent validation oracle. The endpoint is exported
NON_PRIVILEGED, but the per-offset GSP allowlist is unknown. Effective CAP_SYS_ADMIN is cleared;
unknown ABI/permission/row/shape refusal is named. No privileged read is a runtime fallback.
A complete physical snapshot can be checked for integrity while only explicitly resolved fields
are eligible to influence a guest page. It must never replace an already derived virtual field
simply because the host provided a different physical value.

T3 is not used to fix CAPC here. `kf_rm::consttab` enforces exact die/branch/request scope, source
tags, no board data, no blob values, and mechanism-only wildcards. Those rules are the reference
for any later authorized residual measurement. The existing display caps_measured.tsv adapter is
separate legacy data for the unnamed page word0x00c; that word is outside PRECOMP and unchanged.
No CAPC/PRECOMP row is added there or in measured_constants.tsv. No direct kf-disp -> kf-rm
dependency is proposed. If a future exact-die measurement is needed, composition must preserve
that scoped provenance and never turn one capture into a family wildcard. The current owner
instruction uses captures only to validate a derivation, so no captured value is shipped by this work.

The family-wide derivation contract to implement next

For each public die -> display-IP/classes mapping, resolve fields by NAME in that driver's
GENERATED class table. For each active virtual window, compute its head/layer from the virtual
heads/windows; derive feature support and limits from the implemented pipeline. Encode only those
field values through the generated masks and TRUE/FALSE enums. Inactive windows claim nothing.
Unsupported optional features remain explicitly unclaimed; unknown required semantics are refused
by name. Do not infer an unknown physical constant from either that zero or another die.

For a residual physical fact, first record the failed T1 derivation reason; then attempt the
bounded own-die unprivileged read at realize. Bind the result to die/class/window count/layout.
Apply it only through that field's semantic rule and the virtual implementation's supported subset;
otherwise retain the known virtual rule or a named unsupported/refused result. Reject reserved
bits and unsupported capability widening rather than returning a whole physical word. This is one
algorithm for GB202/203/205/206/207; it has no GB202 value table or die-specific capability branch.

Before the candidate can become an arm: commit failing-first tests that a host cannot overwrite
an authored precision, LUT size/load policy, virtual topology, inactive window or unsupported
FVLUT/curve claim; prove at least two killers for each mutant; retain every current test through
explicitly revised assertions, full touched-crate counts, clippy added0 and rustfmt. Add cross-die
cases for GB203/205/206/207 plus an unknown future die, and a native differential for the public
color behavior. Current overlay tests intentionally assert the older whole-word behavior, so they
must not be presented as evidence that these new field-level requirements already pass.

Falsifiers: a published public mechanism contradicting a proposed field rule invalidates the rule;
a native observation inconsistent with a claimed physical family rule refutes its compression;
a guest requesting an advertised but unsupported CA curve refutes the virtual capability contract.
The old clean precision-only0x50 already falsified sufficiency. A future correctly derived and
implemented generation-wide candidate repeating0x50 would refute its proposed Windows fix.
There is no new hardware result in this audit.
