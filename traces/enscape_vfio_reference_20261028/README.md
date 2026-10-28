# Enscape: archived VFIO reference and proposed TDR experiment

**STATUS: RESEARCH / PREREGISTERED, 2026-10-28. No new hardware run.** The owner
requested an archived reference and preparation only. The proposed experiment in
[PREREGISTRATION.md](PREREGISTRATION.md) requires a separate execution go-ahead.

MEASURED from archived VF1 text, re-extracted on 2026-10-28: both selected
SPECviewperf 15.x iterations completed all nine selected workloads and all 150
subtests, including `enscape-01/06_SanMiguelFullUltraHQ` twice. This is an actual
archived workload-completion record, not the owner's assertion alone and not a
Heaven result. It does not mean every workload shipped in SPECviewperf 15 was run.
[reference.json](reference.json) gives the status-only extraction;
[archive_manifest.json](archive_manifest.json) binds its input text with SHA256.
[timeline_evidence.json](timeline_evidence.json) separates retained QEMU request
intervals and censoring from the coordinator's relayed cross-clock observations.

| Iteration/run identifier | Workload statuses | Subtest statuses | Enscape 06 |
|---|---|---|---|
| `result_2026-10-21-23-51-37` | Nine Completed | 75 Completed | Completed |
| `result_2026-10-22-01-15-28` | Nine Completed | 75 Completed | Completed |

REPORTED in the VF1 archive: one native Windows boot on 2026-10-22, physical RTX
4070 through QEMU/KVM VFIO, Windows 11 Pro build 26200, installed NVIDIA 580.88,
QEMU source `89fc8a22291dd7bda69c3077ba829b1e934b11db`, 12 GiB RAM/eight vCPUs,
FullHD benchmark selection and a 3840x2160 native desktop. The two host-local
intervals were reported as 01:51:37–03:15:28 and 03:15:28–04:39:02 CEST. Original
guest result-directory clocks were two hours behind host local time. Exact SPEC
package/patch hashes were not sealed; the log's internal version string alone is
not a package-content identity.

CODE READ: original evidence commit `26766c5002e96defb2603392db41728bc0888259`
and the later labelled archive
[`b27a1dcb15aa684c6ea062d4593d92733e10d398`](https://github.com/reindertpelsma/kayfabe/tree/b27a1dcb15aa684c6ea062d4593d92733e10d398/traces/specviewperf_vfio_20261022)
retain the full-suite completion metadata and limitations. No source branch is
merged by this preparation. The [completion metadata](https://github.com/reindertpelsma/kayfabe/blob/b27a1dcb15aa684c6ea062d4593d92733e10d398/traces/specviewperf_vfio_20261022/full-suite-completion.json)
records a **null CLI exit code**. Later command-channel loss caused the unchanged
watchdog to quit QEMU; clean Windows shutdown and later guest events were not
verified. Completed benchmark statuses do not prove pixel/byte correctness.

The owner reports the same image/GPU passed under VFIO. The archive supports the
same reported RTX 4070 host/GPU context and owner-image lineage. It does **not**
establish byte-identical VF1 and ENS12 parent images: VF1's recorded parent mtime
is 2026-10-21T23:22:47.682478Z, while ENS12's parent record gives
2026-10-22T22:30:03.392864Z; the retained operator lineage labels also differ. No image content
hash or preserved VF1 overlay links the two. VF1's parent was recorded read-only
and unchanged during that session; that establishes preservation, not later
identity. No stable physical-device identifier was compared in this preparation.

UNKNOWN: VF1 `TdrDelay`, `TdrDdiDelay` and other TDR registry overrides. A bounded
search of its archived textual evidence found no registry readback. Thus this
The VF1 archive supplies **no default-TDR registry control**. ENS12
A7416/A7417/A7418 do archive absent `TdrDelay`/`TdrDdiDelay` readbacks; their planned
reboot was skipped because the keys were already absent. Native GPU capacity and
desktop configuration also differ from kayfabe. Do not silently turn this historical
reference into a matched control for the proposed intervention.

MEASURED in VF1 `result_2026-10-21-23-51-37` at `26766c500`: the harness invoked
the vendor batch script with `enscape 6` immediately after naming `06_SanMiguelFullUltraHQ`:

```text
"C:\Program Files\SPECviewperf 15\common\tools\run_vset.bat" enscape 6
```

CODE READ: the archived top-level CLI help selects workloads, not individual
subtests. No invented `--subtest` option is used. The current installed batch
script identity and independent FullHD setup still need verification before a
future isolated run; seeing an invocation within a complete workload does not
prove that its surrounding setup can be omitted safely.

Status-only evidence is published here; no SPEC performance score, screenshot,
model, driver/firmware binary or raw private log is copied into this directory.
Configuration: SPECviewperf 15.x, Windows guest, QEMU/KVM VFIO physical RTX 4070;
proposed kayfabe configuration is specified separately in the preregistration.

(nc) = non-compliant: run in a virtualised configuration, not an official SPEC result and not submission-compliant; SPEC and SPECviewperf are trademarks of the Standard Performance Evaluation Corporation, https://www.spec.org
