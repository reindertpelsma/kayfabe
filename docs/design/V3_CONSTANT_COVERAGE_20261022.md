# V3 constants coverage: same-die evidence and remaining gaps

**STATUS: LIVE, 2026-10-22 (CODE READ inventory; no new GPU or Windows run).**

CODE READ: the comparison base is master `e9c6d39e5`. The
[historical matrix](V3_CONSTANT_MATRIX_20261014.md) describes its source branch,
not this base. This page indexes existing interface measurements so they need not
be repeated, and keeps implementation and acceptance separate.

## Sources and evidence classes

| key | source branch and pinned revision | relevant content |
|---|---|---|
| M | `claude/gpu-types-20261015` at `1cb0486100a0c9b434ae56880ca6f23971e9a34a` | matrix; tier-2 probes; GA104/GA106 Windows bodies; 92 data rows |
| S | `codex/sweep-dies-20261022` at `bc3a967e061aa0a267467b59393afaf10eb0ec70` | inherited table with 96 data rows, 39 ZBC rows on 13 dies; seven caps rows on six dies |
| D | `codex/die-driver-sweep-20261015` at `728e6febde5e42d53c8085675e2bf1209ab38375` | AD102/TU106 native caps evidence, pruned from S |
| B | `codex/sweep-blackwell-20261015` at `24b6a4256ebdd4d806854696d417b93eddf34df4` | GB205/GB206 native caps evidence, pruned from S |
| R | `claude/blackwell-resume-20261020` at `df87431d5dc7019061d9d95a2d847f34fa657d59` | GB202 branch-580 caps scalar and same-box branch-595 oracle |

CODE READ: source hashes and text selections are in
[`source_manifest.tsv`](../../traces/cw1_constants_20261018/source_manifest.tsv).
MEASURED below describes the 2026-10-14..20 artifacts at the specified paths; each records that
interface observation; it does not mean CW1 reran it. CODE READ means a source
inspection. INFERRED means a conclusion from that inspection, including absence
within the inspected corpus.

CODE READ: **T** = tier-2 host probe, **Z** = ZBC defaults, **G** = cudart/8162
probe, **S** = native Windows request shapes/census, **V** = native Windows
`rm.*` values, **C** = native caps word `0x00c`. Host-side request-size scans are
marked `host-S`, not Windows-S. **Inherited B0** means the shared master
`kf-chip` class/register/boot machinery and `kf_rm::hostfacts` query path;
it is code coverage only, not a successful boot or a same-die table observation.
The actual host must supply floorsweeping-dependent fields at runtime.

## Consumer and workstation dies, Turing and newer

CODE READ: product names below identify only the SKUs seen in the artifacts.
Workstation evidence is called out explicitly; a consumer observation does not
establish a workstation board value. A dash means no matching artifact in this
inventory, not an authored zero. Dates/drivers remain in the linked files.

| die | same-die evidence (MEASURED at cited path) | inherited facts (CODE READ) | unmeasured or withheld constants (INFERRED from sources M/S/D/B/R) | source |
|---|---|---|---|---|
| TU102 | T/Z/G, RTX 2080 Ti, host 595.91.07: `traces/constants_measure_20261014b/tier2/raw_53783014.txt` | B0; host-S 595 | Windows-S/V/C; workstation variants | M/S |
| TU104 | —; Quadro RTX 4000 attempt had no probe session | B0 | all table groups, including workstation; no same-die row | M/S; gap search below |
| TU106 | T/Z/G, RTX 2070, host 595.84: `traces/constants_measure_20261014b/tier2/raw_49886923.txt`; C, RTX 2060 Super, Windows 580.88: `traces/die_driver_sweep_20261015/turing_2060super/native-module-caps580.txt`; V subset (2801/2806): `traces/die_driver_sweep_20261015/turing_2060super/native-small-caps-gsp580.txt` | B0; host-S 595 | remaining V; a caps read is not a complete Windows GSP census | M/S/D |
| TU116 | T/Z/G, GTX 1660 Super, host 580.178.04: `traces/constants_measure_20261014b/tier2/raw_55845469.txt` | B0; host-S 580 | Windows-S/V/C | M/S |
| TU117 | — | B0 | all table groups; no same-die row | gap search below |
| GA102 | T/Z/G, RTX 3090, host 570.86.15: `traces/constants_measure_20261014b/tier2/raw_18990232.txt`; workstation RTX A5000, host 580.126.09: `traces/constants_measure_20261014b/tier2/raw_55657856.txt` | B0; host-S 570/580 | Windows-S/V/C; 8162 is board-dependent, no die-only fallback | M/S |
| GA103 | — | B0 | all table groups; no same-die row | gap search below |
| GA104 | T/Z/G, RTX 3060 Ti: `traces/constants_measure_20261014b/tier2/raw_46485962.txt`; workstation RTX A4000 S/V, Windows 596.51, GSP 595.76: `traces/constants_measure_20261014b/ga104_596.51/C/` | B0; host-S 575/580 | C; other Windows firmware branches | M/S |
| GA106 | T/Z/G, RTX 3060: `traces/constants_measure_20261014b/tier2/raw_48529478.txt`; S on seven Windows builds and V on six GSP majors: `traces/constants_measure_20261014b/ga106_sizes_by_windows_driver.tsv` and `traces/constants_measure_20261014b/ga106_572.83/C/` through `ga106_610.88/C/` | B0 | C; display-common rows that need a head | M/S |
| GA107 | — | B0 | all table groups; no same-die row | gap search below |
| AD102 | T/Z/G, RTX 4090, host 595.84: `traces/constants_measure_20261014b/tier2/raw_36762817.txt`; C, Windows 580.88: `traces/die_driver_sweep_20261015/ad102_4090/native-module-caps-580.txt`; V subset (2801/2806): `traces/die_driver_sweep_20261015/ad102_4090/native-small-caps-gsp580.txt` | B0; host-S 595 | remaining V; Windows 581.51 census is source-only, see below | M/S/D/B |
| AD103 | T/Z/G, RTX 4070 Ti Super: `traces/constants_measure_20261014b/tier2/raw_55027878.txt`; S, Windows 580.88: `traces/real_windows_20261014/` (already on master) | B0; host-S 595 | V/C; other Windows branches | M/S/master |
| AD104 | T/Z/G, RTX 4070: `traces/constants_night_20261014/`; workstation RTX 4500 Ada: `traces/constants_measure_20261014b/tier2/raw_55819288.txt`; Z on L4: `traces/constants_measure_20261014b/tier2/raw_54285092.txt` | B0; home Windows/host observations indexed by historical matrix §5.1 | D-1 GMMU has text evidence but no row; 9001/a001 board-dependent; caps source in S cites home host/capture, not a restored standalone caps file | M/S/master |
| AD106 | T/Z/G, RTX 4060 Ti, host 590.48.01: `traces/constants_measure_20261014b/tier2/raw_50710714.txt` | B0; host-S 590 | Windows-S/V/C | M/S |
| AD107 | — | B0 | all table groups; no same-die row | gap search below |
| GB202 | T/Z/G, RTX 5090: `traces/constants_measure_20261014b/tier2/raw_48219165.txt`; C, Linux 580.159.04: `traces/blackwell_resume_20261020/caps/bwr_caps_page_580.txt` | B0; host-S 610; later Windows-S source-only below | V; no extrapolation to neighbouring GB20x dies | M/S/R |
| GB203 | T/Z/G, RTX 5070 Ti: `traces/constants_measure_20261014b/tier2/raw_38751926.txt` | B0; host-S 580/595 | Windows-S/V/C | M/S |
| GB205 | C, RTX 5070, Linux 595.91.07 and Windows 580.88: `traces/die_driver_sweep_20261015/blackwell_5070/linux595_20261016/bwbatch_native_caps.txt` and `traces/die_driver_sweep_20261015/blackwell_5070/native_windows580_20261016/native-caps580-recovered.txt` | B0 | T/Z/G, Windows-S/V; caps alone does not fill them | S/B |
| GB206 | T/Z/G, RTX 5060 Ti: `traces/constants_measure_20261014b/tier2/raw_51352327.txt`; C, RTX 5060, Windows 580.88: `traces/die_driver_sweep_20261015/blackwell_5060/native-module-caps580.txt` | B0; host-S 580; later Windows-S source-only below | V; the first caps attempt's failed RPC capture supplies no scalar row | M/S/B |
| GB207 | — | B0 | all table groups; no same-die row | gap search below |

CODE READ: the S table has 96 non-header data rows: 9 `cudart.*` (including
one software-state rule), 2 `gss.*`, 46 `rm.*`, 39 `zbc.*`.
The historical M table has 92 data rows, superseding the earlier 80-row
snapshot cited by the audit. The audit also omitted the four later `rm.*` rows
for AD102/TU106 2801/2806; their small native artifacts are restored here. The 39 ZBC rows cover 13 dies and retain branch `*` as host-state
observations in that source schema; this is not authority to wildcard guest-driver
RM values. A future data PR must review that distinction against its loader rules.

CODE READ: `board_specific.tsv` on S refuses GA102 8162 and AD104 9001/a001;
`traces/constants_measure_20261014b/tier2_summary.tsv` and the linked raw outputs
show why. No value is borrowed from a consumer board for a workstation or L4.
An absent die/branch must use a named, counted refusal; this docs PR changes no
fallback behavior. Existing authored fallbacks and captured probe pages are
implementation debt, not complete table coverage.

## Windows capture and acceptance are separate

CODE READ: native Windows censuses also exist on
`codex/sweep-blackwell-20261015` at `24b6a4256` for AD102 Windows 581.51 and
GB206 Windows 580.88, and on `codex/sweep-integrate-20261018` at `0bbf3c55fe7679dd0d82fe8ff8de3dcbf5a83216`
under `traces/sweep_integrate_20261018/phase2_windows_gb202_56816449/native_trace/` for GB202 Windows
580.88. These later census paths remain branch-only in this small restoration;
request counts/sizes do not supply missing V bodies. Their availability supersedes
the historical matrix's blanket absence claims. Source paths/revisions must be
checked again before a future PR consumes them.

MEASURED: earlier kayfabe acceptance counters, revisions and diagnostic flags are
in `traces/die_sweep_20261014/`. Those runs include captured AD104 caps on
AD103/AD106 and display-off or failing cases on GA104/TU106/GB206; they do not
prove per-die caps/table wiring. No new Windows acceptance result is claimed here.
CODE READ: later acceptance work on S likewise does not turn inherited table
rows into a new constants sweep. D-1 requires Windows A/B; PR #85 remains held.

## Recheck of the six reported gaps

CODE READ: on 2026-10-22, a scan of 552 fetched `origin` refs found 16 distinct
blobs at `kf-rm/data/measured_constants.tsv`, `kf-rm/data/board_specific.tsv` and
`kf-disp/data/caps_measured.tsv`. None had a non-comment data row naming TU104,
TU117, GA103, GA107, AD107 or GB207. No trace pathname in those tip trees named
these dies. The TU104 attempt appears in M's summary as Quadro RTX 4000 with
`session=False`; it is not a constant observation.

INFERRED: these six dies remain unmeasured for the constant groups in this
inventory. This bounded search cannot establish that no unindexed log anywhere
mentions them; it records exactly the rows and trace paths checked. Falsifier,
stated before the scan: any same-die table row or matching trace artifact would
remove the corresponding gap. None was found in that scope.

## Datacenter and integrated dies (separate coverage)

| die/category | evidence class and source | constant coverage / gap |
|---|---|---|
| L4 (AD104) | MEASURED: `traces/constants_measure_20261014b/tier2/raw_49498406.txt` (595.84), `raw_54285092.txt` (615.71.09) | T/Z/G; 9001/a001 differ from RTX 4070, so host answer or refusal; no separate Windows V row |
| RTX PRO 6000D / GB202 variant | CODE READ: audit cites later sweep SKU; no constant artifact for that SKU restored here | GeForce RTX 5090 evidence above must not be relabelled as a workstation/datacenter measurement |
| GA100 | CODE READ: master `kf-chip/src/lib.rs` and M summary | no table row; A100 attempts returned no host session; realize boot model remains refused |
| GH100 / Hopper | CODE READ: master family/source machinery, M/S tables | no same-die constant row in the inventory; source-derived machinery is not a hardware result |
| GB10x / integrated Blackwell | CODE READ: master `kf-chip/src/lib.rs` and M/S tables | no same-die constant row; integrated/unbuilt boot models may be refused by name |

INFERRED: none of the gaps licenses another die's row, a complete-family claim,
or a runtime table for floorsweeping-dependent state. Derive fields or ask the
host first; keep the exact unsupported die/branch listed until evidence exists.
