# GPU family coverage and archived evidence

STATUS: LIVE, 2026-10-27. GPU-free archival audit; no new hardware qualification.

CODE READ (2026-10-27, L0 start): the pinned `origin/master` is `e54bab5550f09d740d222d667ad497dc8c640ebe`.
The observed `origin/next` is `858ca1e8ad4a2b264656b8e499d820856bcc7310`.
Reachability below is fixed to those tips even if the batch branches move.

This page transcribes the supplied family audit dated 2026-10-27 and checks the available files.
It does not certify the current master or next tree. A result belongs to its recorded revision,
driver, flags and workload. An ancestor check establishes code reachability, not retesting.

## Evidence and correctness rules

- **MEASURED M** (2026-10-27 input): audit says the evidence was already on master/next.
- **MEASURED Mb**: input audit says the evidence was on a branch. This PR imports bounded
  artifacts at their original paths; they remain branch-only until this PR is merged.
- **CLAIMED C**: input has prose, a lane report or a PR claim rather than an identified direct
  result. Its label stays claimed, including when this collection adds corroborating text.
- **INFERRED I**: a hypothesis or interpretation, preserved as an inference.
- **UNTESTED**: no result in the input audit. It is not an assertion that no private test exists.
- **unrecorded / unknown**: the input does not pin that cell to a clean commit. Dirty-build
  ancestry identifies the tracked base only; no exact-tree equivalence is claimed.

CODE READ (2026-10-27, owner rules supplied with the family audit): a rental failure needs a
green same-box host reference or a passing native Windows-on-Vast reference before it can
count against kayfabe. Correctness requires mirror, lazy observe and pointer lifecycle ON.
Historical bars often predate those features or omit their state; their original PASS label
does not establish the full-feature configuration or a current-code family bar.

**Windows TDR-storm caveat.** MEASURED (2026-10-27, TUR1 leases57882297/57908255, [T5], [T6],
[T7], [A12], [A14]): TU106 and GA106 reach Code 0 and run early probes, then storm under
workload. The supplied audit records66 events in 8 min on TU106 and54 events at 6 min on GA106,
with116 RM unloads in 17 min on GA106. The storm persists after the lost-ring fix. These cells
remain FAIL/PARTIAL; a driver load or short-probe success is not a stable Windows workload bar.
CLAIMED C (2026-10-27 input audit): AD104 is the only owner-host stable Windows workload
account, with unresolved Enscape sequencing and performance caveats.

## Per-die matrix

CODE READ (2026-10-27 input audit): thin means the Linux30-arm fast suite; ladder means
cup2/cup3/cup8/cup8bench. W-driver is a Windows driver load result; W-apps covers workloads
and stability. Disp-L/Disp-W are display on Linux/Windows. Native capture-tool revisions
are explicitly separate from kayfabe revisions. Hardware n=1 and unavailable spread apply
unless the cited result says otherwise. The master column is the ancestry result.

### Turing

| Die | Surface | Input label and result | Revision | Master | Evidence |
| --- | --- | --- | --- | --- | --- |
| TU102 | guest surfaces | UNTESTED: Thin, ladder/apps, Windows and display untested | n/a | n/a | audit only |
| TU102 | Native | MEASURED M: Tier-2 host probe only; no family qualification | unrecorded | unknown | [T0] |
| TU104 | thin | UNTESTED: No result in supplied audit; no revision recorded | n/a | n/a | audit only |
| TU104 | ladder/apps | UNTESTED: No result in supplied audit; no revision recorded | n/a | n/a | audit only |
| TU104 | W-driver | UNTESTED: No result in supplied audit; no revision recorded | n/a | n/a | audit only |
| TU104 | W-apps | UNTESTED: No result in supplied audit; no revision recorded | n/a | n/a | audit only |
| TU104 | Disp-L | UNTESTED: No result in supplied audit; no revision recorded | n/a | n/a | audit only |
| TU104 | Disp-W | UNTESTED: No result in supplied audit; no revision recorded | n/a | n/a | audit only |
| TU104 | Native | UNTESTED: No result in supplied audit; no revision recorded | n/a | n/a | audit only |
| TU106 | thin 180s | MEASURED Mb: PARTIAL 29/30; copy arm exceeds the original oracle | at `67339dc0a` | no | [T1] |
| TU106 | thin 300s | MEASURED Mb: 30/30 at expanded 300s; copy arm 187s; dirty build | at `67339dc0a` | no | [T2],[T10] |
| TU106 | ladder/apps | MEASURED Mb: ladder 4/4 with guest-driver=580.159.04; apps untested | at `67339dc0a` | no | [T3] |
| TU106 | W-driver | MEASURED Mb: Code 0, 20 winprod flags and display=on; dirty branch build | at `67339dc0a` | no | [T4],[T10] |
| TU106 | W-apps | MEASURED Mb: FAIL n=2 boots: early probes then TDR storm / desktop-up storm | at `e3c4483a0` | no | [T5],[T6],[T7],[T10] |
| TU106 | Disp-L | UNTESTED: No Linux display result | n/a | n/a | audit only |
| TU106 | Disp-W | INFERRED I: PARTIAL: realize with draft164; flips stall; acquire unmodelled | at `e3c4483a0` | no | [T10] |
| TU106 | Native Linux | MEASURED Mb: gates 9/9 + gate 10, bare 30/30, ladder4; approved HSCE controls | at `67339dc0a` | no | [T8],[T9],[T10] |
| TU106 | Native Windows | MEASURED Mb: Code 0 and D3D 60 frames; no continuous load hold | at `541033eaf` | no | [W1] |
| TU116 | thin | MEASURED M: 30/30 historical bar | at `1915bd71` | yes | [T11] |
| TU116 | ladder/apps | MEASURED M: ladder 4/4; apps untested | at `1915bd71` | yes | [T11] |
| TU116 | W-driver | UNTESTED: No kayfabe result | n/a | n/a | audit only |
| TU116 | W-apps | UNTESTED: No kayfabe result | n/a | n/a | audit only |
| TU116 | Disp-L | UNTESTED: No kayfabe result | n/a | n/a | audit only |
| TU116 | Disp-W | UNTESTED: No kayfabe result | n/a | n/a | audit only |
| TU116 | Native Linux | MEASURED M: bare 30/30 and gates 9/9; predates display/relay changes | at `1915bd71` | yes | [T11] |
| TU116 | Native Windows | MEASURED Mb: Capture completed; no kayfabe verdict | at `541033eaf` | no | [W2] |
| TU117 | thin | UNTESTED: No result in supplied audit; no revision recorded | n/a | n/a | audit only |
| TU117 | ladder/apps | UNTESTED: No result in supplied audit; no revision recorded | n/a | n/a | audit only |
| TU117 | W-driver | UNTESTED: No result in supplied audit; no revision recorded | n/a | n/a | audit only |
| TU117 | W-apps | UNTESTED: No result in supplied audit; no revision recorded | n/a | n/a | audit only |
| TU117 | Disp-L | UNTESTED: No result in supplied audit; no revision recorded | n/a | n/a | audit only |
| TU117 | Disp-W | UNTESTED: No result in supplied audit; no revision recorded | n/a | n/a | audit only |
| TU117 | Native | UNTESTED: No result in supplied audit; no revision recorded | n/a | n/a | audit only |

### Ampere

| Die | Surface | Input label and result | Revision | Master | Evidence |
| --- | --- | --- | --- | --- | --- |
| GA102 | thin production | MEASURED Mb: 30/30; lease 57723513, same-box native reference | at `714ec29d3` | no | [A1] |
| GA102 | thin mirror | CLAIMED C: Round-two header claimed30/30; kept as claimed | at `6564dd597` | no | audit only |
| GA102 | ladder/apps | CLAIMED C: 3080Ti ladder 4/4 plus gfx; apps untested | at `68fb3768` | yes | audit only |
| GA102 | W-driver | MEASURED Mb: BLOCKED: sealed install, bind1200s ceiling, zeroRPCs | at `6e3fd3fd6` | no | [A15],[A16] |
| GA102 | W-apps | UNTESTED: No workload verdict | n/a | n/a | audit only |
| GA102 | Disp-L | CLAIMED C: PARTIAL: displayless Vast description; no display proof | unrecorded | unknown | audit only |
| GA102 | Disp-W | UNTESTED: No result | n/a | n/a | audit only |
| GA102 | Native Linux | MEASURED Mb: Same-box gates 9/9 + gate 10 and bare 30/30 at SW5 production | at `714ec29d3` | no | [A1] |
| GA102 | Native CUDA | MEASURED M (report): Audit says4/4 at lease 57209908; cited native_complete path absent | unrecorded | unknown | audit only |
| GA102 | Native Windows | UNTESTED: No native Windows reference in supplied audit | n/a | n/a | audit only |
| GA103 | thin | UNTESTED: No result in supplied audit; no revision recorded | n/a | n/a | audit only |
| GA103 | ladder/apps | UNTESTED: No result in supplied audit; no revision recorded | n/a | n/a | audit only |
| GA103 | W-driver | UNTESTED: No result in supplied audit; no revision recorded | n/a | n/a | audit only |
| GA103 | W-apps | UNTESTED: No result in supplied audit; no revision recorded | n/a | n/a | audit only |
| GA103 | Disp-L | UNTESTED: No result in supplied audit; no revision recorded | n/a | n/a | audit only |
| GA103 | Disp-W | UNTESTED: No result in supplied audit; no revision recorded | n/a | n/a | audit only |
| GA103 | Native | UNTESTED: No result in supplied audit; no revision recorded | n/a | n/a | audit only |
| GA104 | thin historical | MEASURED M: 3060Ti30/30, floor swept | at `25edb757` | yes | [A4] |
| GA104 | thin production | MEASURED M: 30/30; lease 57568731, RM-bound GA104 | at `714ec29d3` | no | [A2] |
| GA104 | ladder/apps | MEASURED M: cup3/cup8; apps34/65 on3070, pre-fix and unrepeated | at `738c90e5` | yes | [A5] |
| GA104 | W-driver | MEASURED M: PARTIAL A4000 boot; two UNCLEAN-kill stops | unrecorded | unknown | audit only |
| GA104 | W-apps | UNTESTED: No result in supplied audit | n/a | n/a | audit only |
| GA104 | Disp-L | UNTESTED: No result in supplied audit | n/a | n/a | audit only |
| GA104 | Disp-W | UNTESTED: No result in supplied audit | n/a | n/a | audit only |
| GA104 | Native Linux | MEASURED M: SW3 same-box gates 9/9 + gate 10 and bare 30/30 | at `714ec29d3` | no | [A2] |
| GA104 | Native Windows | MEASURED M: 596.51 capture only; no load hold | unrecorded | unknown | audit only |
| GA106 | thin historical | MEASURED M: 30/30 historical Linux bar | at `c0ef7b75` | yes | [A7] |
| GA106 | thin historical | MEASURED M: 30/30 historical Linux bar | at `afb552ea` | yes | [A8] |
| GA106 | thin production | MEASURED Mb: 30/30; lease 57726516 | at `714ec29d3` | no | [A3] |
| GA106 | ladder/apps | MEASURED M: 60/65 at4c48ca0c; README61/65 is a separate claim | at `4c48ca0c` | yes | [A6] |
| GA106 | W-driver | MEASURED Mb: Code 0 in TUR1 Ampere control | at `385c9285` | no | [A11],[T10] |
| GA106 | W-apps | MEASURED Mb: FAIL: TDR storm;54 events at 6 min,116 unloads at 17 min | at `340c6b5a` | no | [A12],[A13],[A14],[T10] |
| GA106 | Disp-L | CLAIMED C: Input says owner-host display PASS; die/revision not substantiated | unrecorded | unknown | audit only |
| GA106 | Disp-W | UNTESTED: No standalone display qualification | n/a | n/a | audit only |
| GA106 | Native Linux | MEASURED Mb: Green before/after TUR1 Ampere run; scoped roundtrip | at `340c6b5a` | no | [A9],[A10] |
| GA106 | Native Windows | MEASURED M (report): Seven builds captured; no load hold | unrecorded | unknown | audit only |
| GA107 | thin | UNTESTED: No result in supplied audit; no revision recorded | n/a | n/a | audit only |
| GA107 | ladder/apps | UNTESTED: No result in supplied audit; no revision recorded | n/a | n/a | audit only |
| GA107 | W-driver | UNTESTED: No result in supplied audit; no revision recorded | n/a | n/a | audit only |
| GA107 | W-apps | UNTESTED: No result in supplied audit; no revision recorded | n/a | n/a | audit only |
| GA107 | Disp-L | UNTESTED: No result in supplied audit; no revision recorded | n/a | n/a | audit only |
| GA107 | Disp-W | UNTESTED: No result in supplied audit; no revision recorded | n/a | n/a | audit only |
| GA107 | Native | UNTESTED: No result in supplied audit; no revision recorded | n/a | n/a | audit only |

### Ada

| Die | Surface | Input label and result | Revision | Master | Evidence |
| --- | --- | --- | --- | --- | --- |
| AD102 | thin | MEASURED Mb: 30/30; lease 57144505 | at `7f6f7e7a7` | no | [D1],[D2] |
| AD102 | ladder/apps | MEASURED Mb: Native CUDA 4/4; guest apps untested | at `7f6f7e7a7` | no | [D3] |
| AD102 | W-driver | MEASURED M: Code 0,1920x1080,DWM; historical window result | at `7f6f7e7a7` | no | [D4] |
| AD102 | W-apps | CLAIMED C: 600s desktop hold / apps13/112; lease 57193877 summary | unrecorded | unknown | [D5] |
| AD102 | Disp-L | UNTESTED: No result | n/a | n/a | audit only |
| AD102 | Disp-W | CLAIMED C: PARTIAL desktop/DWM evidence; no broad display qualification | unrecorded | unknown | [D4],[D5] |
| AD102 | Native | MEASURED Mb: Host30/30 and CUDA 4/4; no native Windows | at `7f6f7e7a7` | no | [D1],[D3] |
| AD103 | thin/ladder/apps | UNTESTED: No Linux guest run; SW5 had no eligible offer | n/a | n/a | audit only |
| AD103 | W-driver | MEASURED M: Historical boot and gates 9/9 | unrecorded | unknown | [D6] |
| AD103 | W-apps | CLAIMED C: PARTIAL per source README; no workload stability bar | unrecorded | unknown | audit only |
| AD103 | Disp-L | UNTESTED: No result | n/a | n/a | audit only |
| AD103 | Disp-W | CLAIMED C: PARTIAL historical description | unrecorded | unknown | audit only |
| AD103 | Native | MEASURED M: Historical gates 9/9 + gate 10; no new L0 test | unrecorded | unknown | [D6] |
| AD104 | thin | MEASURED M: 30/30; vast53568615 | at `fc2dadc4` | yes | [D8] |
| AD104 | family bar | MEASURED M: AD104 historical bar named in reachability audit | at `f442980e` | yes | [D8] |
| AD104 | ladder/apps | MEASURED M: ladder 4/4 historical; full owner app-set is claimed | at `fc2dadc4` | yes | [D8] |
| AD104 | W-driver | CLAIMED C: Owner-host PASS; exact Windows revision not supplied | unrecorded | unknown | audit only |
| AD104 | W-apps | CLAIMED C: Owner-host PASS with performance/Enscape caveats; no blanket pass | unrecorded | unknown | audit only |
| AD104 | Disp-L | CLAIMED C: Owner-host PASS; exact build not supplied | unrecorded | unknown | audit only |
| AD104 | Disp-W | CLAIMED C: Owner-host PASS; exact build not supplied | unrecorded | unknown | audit only |
| AD104 | Native | MEASURED M: Historical bare 30/30; same-owner-GPU VFIO is the cited reference | at `fc2dadc4` | yes | [D8] |
| AD106 | thin historical | MEASURED M: 30/30; vast52660152 | at `1d6bb323` | yes | [D9] |
| AD106 | thin production | MEASURED Mb: 30/30; lease 57731910 | at `714ec29d3` | no | [D10] |
| AD106 | thin mirror | CLAIMED C: Round-two header claims30/30; kept as claimed | at `6564dd597` | no | audit only |
| AD106 | ladder/apps | MEASURED M: Ladder in v3_adasys; apps untested | unrecorded | unknown | [D11] |
| AD106 | W-driver | MEASURED M: Historical Code 0 | unrecorded | unknown | [D7] |
| AD106 | W-apps | CLAIMED C: PARTIAL per README | unrecorded | unknown | audit only |
| AD106 | Disp-L | UNTESTED: No result | n/a | n/a | audit only |
| AD106 | Disp-W | CLAIMED C: PARTIAL historical description | unrecorded | unknown | audit only |
| AD106 | Native | MEASURED Mb: gates 9/9 + gate 10 and bare 30/30 on same SW5 box | at `714ec29d3` | no | [D10] |
| AD107 | thin | UNTESTED: No result in supplied audit; no revision recorded | n/a | n/a | audit only |
| AD107 | ladder/apps | UNTESTED: No result in supplied audit; no revision recorded | n/a | n/a | audit only |
| AD107 | W-driver | UNTESTED: No result in supplied audit; no revision recorded | n/a | n/a | audit only |
| AD107 | W-apps | UNTESTED: No result in supplied audit; no revision recorded | n/a | n/a | audit only |
| AD107 | Disp-L | UNTESTED: No result in supplied audit; no revision recorded | n/a | n/a | audit only |
| AD107 | Disp-W | UNTESTED: No result in supplied audit; no revision recorded | n/a | n/a | audit only |
| AD107 | Native | UNTESTED: No result in supplied audit; no revision recorded | n/a | n/a | audit only |

### Blackwell

| Die | Surface | Input label and result | Revision | Master | Evidence |
| --- | --- | --- | --- | --- | --- |
| GB202 | thin | UNTESTED: No Linux guest/display run in supplied audit | n/a | n/a | audit only |
| GB202 | ladder | UNTESTED: No Linux guest/display run in supplied audit | n/a | n/a | audit only |
| GB202 | Disp-L | UNTESTED: No Linux guest/display run in supplied audit | n/a | n/a | audit only |
| GB202 | Disp-W | UNTESTED: No Linux guest/display run in supplied audit | n/a | n/a | audit only |
| GB202 | W-driver/apps BW6 | MEASURED Mb: FAIL n=1: Code 0 then0x141 and fatal0x113 about15s later | at `007ba1eb0` | no | [B1] |
| GB202 | W-driver/apps BW11 | MEASURED Mb: INCONCLUSIVE; different-machine rebuilt-parent run did not reproduce | at `007ba1eb0` | no | [B11] |
| GB202 | Native Linux BW6 | MEASURED Mb: Same-lease/boot before:9+10,bare 29/29,CUDA 4/4,GR; after green audit | at `6e3fd3fd6` | no | [B2],[B3],[B4],[B5],[B6] |
| GB202 | Native Windows BW7 | MEASURED Mb: Same physical machine, different lease/boot: Code 0,602s,no dumps | unrecorded | unknown | [B7],[B8],[B9],[B10] |
| GB203 | thin | MEASURED M: 30/30; gates 9/9; vast52730218 | at `256ec854` | yes | [B12] |
| GB203 | ladder | MEASURED M: Historical ladder PASS | at `256ec854` | yes | [B12] |
| GB203 | W-driver | UNTESTED: No result; display chain unmerged in supplied audit | n/a | n/a | audit only |
| GB203 | W-apps | UNTESTED: No result; display chain unmerged in supplied audit | n/a | n/a | audit only |
| GB203 | Disp-L | UNTESTED: No result; display chain unmerged in supplied audit | n/a | n/a | audit only |
| GB203 | Disp-W | UNTESTED: No result; display chain unmerged in supplied audit | n/a | n/a | audit only |
| GB203 | Native Linux | MEASURED M: bare 30/30 historical | at `54158daa` | yes | [B12] |
| GB203 | Native Windows | CLAIMED C: Capture failed on provider fault; no native qualification | unrecorded | unknown | audit only |
| GB205 | thin | MEASURED M: 30/30 historical bar | at `d40dafbf` | yes | [D8] |
| GB205 | ladder | MEASURED M: Historical4/4 | at `d40dafbf` | yes | [D8] |
| GB205 | Linux reload | MEASURED Mb: Unload/reload PASS from BW4 | at `cb80cbd29` | no | [B16] |
| GB205 | W-driver | MEASURED Mb: Code 0 and601s idle hold with unmerged fix; no apps | at `007ba1eb0` | no | [B13],[B14],[B15] |
| GB205 | W-apps | UNTESTED: Apps not exercised by the601s idle hold | n/a | n/a | audit only |
| GB205 | Disp-L | UNTESTED: No display qualification | n/a | n/a | audit only |
| GB205 | Disp-W | MEASURED Mb: PARTIAL:0x193/0x1A8/0x1B8 display dumps still appear | at `007ba1eb0` | no | [B14] |
| GB205 | Native Linux | MEASURED Mb: gates 9/9 + gate 10,bare 29/29,CUDA 4/4 on BW5 box | at `6e3fd3fd6` | no | [B13],[B15] |
| GB205 | Native Windows | CLAIMED C: Input: Code 0 10/10,disable/enable x3; native hold archived | unrecorded | unknown | [B17] |
| GB206 | thin | MEASURED M: 30/30 at older host580.159.04 revision | at `202c612e` | no | [B18] |
| GB206 | later GR fixture | CLAIMED C: 14/15; ce_completion_wakes_worker_fds failed; draft97 | unrecorded | unknown | audit only |
| GB206 | ladder | CLAIMED C: Native CUDA 4/4 claimed; guest ladder untested | unrecorded | unknown | audit only |
| GB206 | W-driver/apps | CLAIMED C: FAIL Code 43; later Code 0 then Xid32 pair/0x116 | at `cb80cbd29` | no | audit only |
| GB206 | Disp-L | UNTESTED: No result | n/a | n/a | audit only |
| GB206 | Disp-W | UNTESTED: No result | n/a | n/a | audit only |
| GB206 | Native Windows | MEASURED Mb: Native capture, idle only; no workload hold | at `541033eaf` | no | [W3] |
| GB207 | thin | UNTESTED: No result in supplied audit; no revision recorded | n/a | n/a | audit only |
| GB207 | ladder/apps | UNTESTED: No result in supplied audit; no revision recorded | n/a | n/a | audit only |
| GB207 | W-driver | UNTESTED: No result in supplied audit; no revision recorded | n/a | n/a | audit only |
| GB207 | W-apps | UNTESTED: No result in supplied audit; no revision recorded | n/a | n/a | audit only |
| GB207 | Disp-L | UNTESTED: No result in supplied audit; no revision recorded | n/a | n/a | audit only |
| GB207 | Disp-W | UNTESTED: No result in supplied audit; no revision recorded | n/a | n/a | audit only |
| GB207 | Native | UNTESTED: No result in supplied audit; no revision recorded | n/a | n/a | audit only |

### Other architectures

CODE READ (2026-10-27 input audit): GA100 and GB10B are refused by name; GB10x and GH100
remain source-derived without a hardware row in this collection. No Linux/Windows PASS,
VFIO result or constants-completeness claim is inferred for them.

## GB202 reference facts and attribution boundary

MEASURED (2026-10-24, BW6 lease 57556167, machine152098, at revision007ba1eb0, [B1]–[B6],
[B19]): the guest reached
Code 0 and then bugchecked0x113, with0x141 also retained. The pre-arm Linux baseline in that
same lease and boot records gates 9/9+gate10, bare 29/29, CUDA 4/4 and GR PASS. The post-arm
audit records cuInit0, one device, exactly5accepted native Xids and arm-Xid count0. The native
fixture revision is618f04318, the native product is6e3fd3fd6, and the guest arm is007ba1eb0.

MEASURED (2026-10-24, BW7 lease 57565048, [B7]–[B10]): native Windows580.88 on physical
machine152098, host702330, recorded11Code 0 samples spanning602.024182s and no1001/6008/4101
or nvlddmkm/Display errors. It is the same physical BW6 machine under a different lease and
boot, with an idle hold rather than workload stress. The source record contains an explicitly
INFERRED owner-rule interpretation; this page does not adopt that interpretation as a verdict.

MEASURED (2026-10-26, BW11 lease 57799086, [B11]): a rebuilt-parent run on different machine
138291 did not reproduce the0x141/0x113 sequence. The imported record is a branch summary
and hashes; its raw logs remain private. Its outcome stays INCONCLUSIVE, not a falsification
of BW6 or proof of a mechanism.

CODE READ (2026-10-27 L0 scope): BW6 result.json retains its original UNRESOLVED attribution
and UNVERIFIED-HOST/OBSERVATION markers. This collection makes the Linux and native Windows
reference facts available together. Attribution remains an owner decision under the stated
rule; n=1, different boots, idle-native scope and non-reproduction remain visible.

## Cell identity limits and unresolved source references

- CODE READ (2026-10-27 archival audit; TU106 thin 300s): Source README identifies e3c4483a0 plus test additions; exact dirty tree is not a clean commit.
- CODE READ (2026-10-27 archival audit; TU106 W-apps): Boot-specific dirty source identity is bounded by README; branch head49b4844c0 is not the tested tree.
- CODE READ (2026-10-27 archival audit; TU106 Native Windows): Revision is collector source, not kayfabe; no kayfabe verdict.
- CODE READ (2026-10-27 archival audit; GA102 thin mirror): Unpublished local round-two data is outside this branch-evidence import.
- CODE READ (2026-10-27 archival audit; GA102 Native CUDA): A later different-lease CUDA 4/4 log is included as A17, not substituted for the missing path.
- CODE READ (2026-10-27 archival audit; GA104 W-driver): Input groups the October14 revisions; exact A4000 per-arm revision is not resolved here.
- CODE READ (2026-10-27 archival audit; GA104 Native Windows): Native capture revision is unrecorded in supplied matrix.
- CODE READ (2026-10-27 archival audit; GA106 W-apps): Storm persists after lost-ring fix; do not treat absence of overdue dumps as stability.
- CODE READ (2026-10-27 archival audit; AD102 W-driver): Windows-specific source is stated in D4; Linux revision does not imply Windows tree equality.
- CODE READ (2026-10-27 archival audit; AD103 W-driver): Per-arm October14 revision must be read from D6; no current-code claim.
- CODE READ (2026-10-27 archival audit; AD104 Native): Only the supplied owner-host account claims VFIO; this import adds no VFIO run.
- CODE READ (2026-10-27 archival audit; AD106 W-driver): Per-arm revision must be read from D7; October14 revision group is retained.
- CODE READ (2026-10-27 archival audit; GB202 W-driver/apps BW11): Branch summary and private capture hashes only; raw run logs remain private.
- CODE READ (2026-10-27 archival audit; GB202 Native Windows BW7): Windows580.88. No kayfabe source revision applies to native Windows.
- CODE READ (2026-10-27 archival audit; GB205 Linux reload): Adjacent binary_sha256.txt pins cb80cbd2; this is a different build from the Windows fix revision.
- CODE READ (2026-10-27 archival audit; GB205 Native Linux): Native baseline6e3fd3fd6; separate GR fixture618f04318; guest fix007ba1eb0.
- CODE READ (2026-10-27 archival audit; GB205 Native Windows): Original C label retained; no attribution decision is adopted.
- CODE READ (2026-10-27 archival audit; GB206 later GR fixture): Revision not specified in input; this is not a replacement30-arm bar.
- CODE READ (2026-10-27 archival audit; GB206 W-driver/apps): GB205 fixes were not run on this die in the supplied audit.

- CODE READ (2026-10-27 audit): the supplied GA102 native-CUDA path
  `traces/sweep_dies_20261018/lease_57209908/native_complete/` is absent from the named refs.
  Its original MEASURED/M label is retained as reported, with an explicit missing-path limit.
  [A17] is a different-lease native CUDA log and is not a substitute for that missing record.
- CODE READ (2026-10-27 audit): October14 A4000/AD103/AD106 rows are linked to their existing
  result directories. The supplied revision group is202c612e/3d31acc6/b0de7572/6c6bb92b;
  this page does not guess which dirty or clean per-arm tree applies to an unpinned cell.
- CLAIMED C (2026-10-27 input audit): SW5 mirror6564dd597 PASS claims for GA102 and AD106
  remain claimed here. Their round-two small data was local and unpublished at L0 start;
  the imported SW5 branch contains only its original production round and the late-addition
  UNRUN mirror columns. No result is invented to fill that gap.
- CODE READ (2026-10-27 audit): WC1 native captures are not kayfabe guest verdicts. TU106
  D3D60-frame readbacks and sampled Code 0 do not provide a continuous native load hold.
- INFERRED I (2026-10-27 supplied audit): display-on desktop, ignored software-runlist control
  and unmodelled Turing flip acquire remain leads. This import does not promote them to causes.
- CODE READ (2026-10-27 audit): no firmware/VBIOS bodies, dumps, VM images, executable kit
  code or large logs are added. Existing archive links point to historical base data; those
  binary archives are not copied by L0. No GPU, rental, guest or native test ran in L0.

## Revision reachability

CODE READ (2026-10-27; [ancestry JSON](../../traces/family_evidence_20261027/revisions.json)):
`git merge-base --is-ancestor <resolved revision> <pinned tip>` was run independently for
each revision below. The tested build and the evidence branch head are distinct identities.

| Revision | On pinned master | On next at start |
| --- | --- | --- |
| `1915bd71` | yes | yes |
| `f442980e` | yes | yes |
| `fc2dadc4` | yes | yes |
| `d40dafbf` | yes | yes |
| `256ec854` | yes | yes |
| `54158daa` | yes | yes |
| `1d6bb323` | yes | yes |
| `25edb757` | yes | yes |
| `c0ef7b75` | yes | yes |
| `afb552ea` | yes | yes |
| `714ec29d3` | no | no |
| `6564dd597` | no | no |
| `007ba1eb0` | no | no |
| `202c612e` | no | no |
| `3d31acc6` | no | no |
| `b0de7572` | no | no |
| `6c6bb92b` | no | no |
| `49b4844c0` | no | no |
| `67339dc0a` | no | no |
| `e3c4483a0` | no | no |
| `385c9285` | no | no |
| `340c6b5a` | no | no |
| `7f6f7e7a7` | no | no |
| `68fb3768` | yes | yes |
| `738c90e5` | yes | yes |
| `4c48ca0c` | yes | yes |
| `cb80cbd29` | no | no |
| `541033eaf` | no | no |
| `618f04318` | no | no |
| `6e3fd3fd6` | no | no |
| `11f9ae1b8` | no | no |
| `38fd948fd` | no | no |
| `b85d02bae` | no | no |
| `48b027267` | no | no |

## Import provenance

CODE READ (2026-10-27; [manifest](../../traces/family_evidence_20261027/manifest.json)):
the manifest pins each evidence source commit and original Git blob, source/import SHA256,
original path, size and redactions. Existing SW3 paths are reused from the pinned master.
GB205 lease 57503040 was found on claude/bw5-urtfix-20261024, also named in the input audit;
claude/blackwell-fixes-20261024 is its product source rather than its result archive.
Private paths, network addresses, GPU identifiers and credential-shaped values are redacted;
numeric verdicts, driver versions, revisions and guard markers are retained.

This is a documentation and data PR against next. It does not merge any tested product branch,
change a guard, accept a300s oracle as the180s bar, waive the8CPU admission floor, or resolve
the Windows storm and GB202 attribution decisions.

[T0]: ../../traces/constants_measure_20261014b/tier2/raw_53783014.txt
[T1]: ../../traces/tur1_20261027/run1/tur1a_suite.out
[T2]: ../../traces/tur1_20261027/run4_regress/tur1d_suite.out
[T3]: ../../traces/tur1_20261027/run2_ladder/cl_tur1g_guest.out
[T4]: ../../traces/tur1_20261027/run3_windows/nvidia_smi_windows_tu106_after_fixes.txt
[T5]: ../../traces/tur1_20261027/run3_windows/matrix_after_userring_fix_fresh_boot.txt
[T6]: ../../traces/tur1_20261027/run3_windows/matrix_second_fresh_boot_tdr_storm.txt
[T7]: ../../traces/tur1_20261027/run3_windows/w7_nvlddmkm153_bugcheck_xml.txt
[T8]: ../../traces/tur1_20261027/run1/tur_gates.log
[T9]: ../../traces/tur1_20261027/run4_regress/host_reference_after.txt
[T10]: ../../traces/tur1_20261027/README.md
[T11]: ../../traces/v3_turing_master/README.md
[W1]: ../../traces/wc1_20261026/round3/tu106/capture_receipt.json
[W2]: ../../traces/wc1_20261026/round3/tu116/capture_receipt.json
[W3]: ../../traces/wc1_20261026/round2/gb206/capture_receipt.json
[W4]: ../../traces/wc1_20261026/round2/gb205/capture_receipt.json
[W5]: ../../traces/wc1_20261026/README.md
[A1]: ../../traces/sw5_die_sweep_20261025/ga102_57723513/result.json
[A2]: ../../traces/sw3_die_sweep_20261024/ga104_57568731/result.json
[A3]: ../../traces/sw5_die_sweep_20261025/ga106_57726516/result.json
[A4]: ../../traces/real_ga104/README.md
[A5]: ../../traces/v3_app_matrix/vast53563077_rtx3070_738c90e5
[A6]: ../../docs/design/V3_APP_MATRIX.md
[A7]: ../../traces/v3_mc20
[A8]: ../../traces/v3_mc23
[A9]: ../../traces/tur1_20261027/iter2_ga106/hostref_before.txt
[A10]: ../../traces/tur1_20261027/iter2_ga106/hostref_after.txt
[A11]: ../../traces/tur1_20261027/iter2_ga106/ga106_o1_matrix.txt
[A12]: ../../traces/tur1_20261027/iter2_ga106/ga106_o1_timeline.csv
[A13]: ../../traces/tur1_20261027/iter2_ga106/ga106_o3_obs.run
[A14]: ../../traces/tur1_20261027/iter2_ga106/ga106_o3_timeline.csv
[A15]: ../../traces/sweep_dies_20261018/lease_57385378/bind_inconclusive/arms/bw580_bind/summary.json
[A16]: ../../traces/sweep_dies_20261018/lease_57385378/bind_inconclusive/bwr_bind_readiness_stage.json
[A17]: ../../traces/sweep_dies_20261018/lease_57385378/native_complete/sweep_cuda_host.log
[D1]: ../../traces/sweep_ada_20261018/lease_57144505/linux_complete/RESULT.md
[D2]: ../../traces/sweep_ada_20261018/lease_57144505/linux_complete/sweep_linux_guest595.log
[D3]: ../../traces/sweep_ada_20261018/lease_57144505/linux_complete/sweep_cuda_host.log
[D4]: ../../traces/sweep_ada_20261018/lease_57144505/windows_stopped/RESULT.md
[D5]: ../../traces/sweep_dies_20261018/lease_57193877/ADA_RESULT.md
[D6]: ../../traces/die_sweep_20261014/ad103_4070tisuper
[D7]: ../../traces/die_sweep_20261014/ad106_4060ti
[D8]: ../../traces/v3_families_master/README.md
[D9]: ../../docs/design/V3_FAMILY_PORT_ADA.md
[D10]: ../../traces/sw5_die_sweep_20261025/ad106_57731910/result.json
[D11]: ../../traces/v3_adasys
[B1]: ../../traces/blackwell_ab_20261023/ROUND_B1r_bw6/attempt1/arms/on_fix/bw6_result.json
[B2]: ../../traces/blackwell_ab_20261023/ROUND_B1r_bw6/attempt1/native_baseline/sweep_gates.log
[B3]: ../../traces/blackwell_ab_20261023/ROUND_B1r_bw6/attempt1/native_baseline/sweep_bare595.log
[B4]: ../../traces/blackwell_ab_20261023/ROUND_B1r_bw6/attempt1/native_baseline/sweep_cuda_host.log
[B5]: ../../traces/blackwell_ab_20261023/ROUND_B1r_bw6/attempt1/native_baseline/sweep_native_grtier.log
[B6]: ../../traces/blackwell_ab_20261023/ROUND_B1r_bw6/attempt1/lease_metadata/final_remote_audit.out
[B7]: ../../traces/blackwell_native_ref_20261018/BW7/attempt2/native_hold_result.json
[B8]: ../../traces/blackwell_native_ref_20261018/BW7/attempt2/offer_identity.json
[B9]: ../../traces/blackwell_native_ref_20261018/BW7/attempt2/hold.txt
[B10]: ../../traces/blackwell_native_ref_20261018/BW7/attempt2/compare_since_boot.txt
[B11]: ../../traces/bw11_20261026/README.md
[B12]: ../../traces/v3_blackwell
[B13]: ../../traces/blackwell_ab_20261023/ROUND_B1q_bw5/attempt1/arms/on_fix/arm_on_fix.log
[B14]: ../../traces/blackwell_ab_20261023/ROUND_B1q_bw5/attempt1/arms/on_fix/collect_on_fix.txt
[B15]: ../../traces/blackwell_ab_20261023/ROUND_B1q_bw5/attempt1/lease_metadata/observed_fix_binary.json
[B16]: ../../traces/blackwell_ab_20261023/ROUND_B1n_bw4/attempt1/arms/lx_reload/run_bw4_lx_reload_probe.log
[B17]: ../../traces/blackwell_ab_20261023/ROUND_REF_bw4/attempt1/native/hold.txt
[B18]: ../../traces/die_sweep_20261014/gb206_5060ti
[B19]: ../../traces/blackwell_ab_20261023/ROUND_B1r_bw6/attempt1/owned_instances.json
