STATUS: DESIGN-ONLY, 2026-11-13. GPU-free implementation; no app behavior qualified by this lane.

# Proprietary-image app automation

CODE READ: `scripts/bench/windows/propimage_apps.py` replaces coordinate calibration and
window-exists passes with per-run evidence collection. `scripts/ci/test_propimage_apps.py`
uses hand-authored fixtures, not captured vendor output. No third-party code was copied or run.

INFERRED hypothesis, falsifier first: if a stale file, launch-only record, API failure, or old
finished label earns a pass, the verdict contract is false. The offline tests and in-memory
mutants exercise these counterexamples; they cannot establish that an app did GPU work.

CODE READ: each guest check writes `C:\kfapps\<random-run-id>\<app>\result.json` (UTF-8 JSON).
The host pulls that file and allowlisted `uia.json` / `model.step` into
`<out>/<app>/<run-id>/`, parses local files, and writes `verdict.json` and `result.tsv`.
Each pass records `FILE` or `UIA`; unqualified checks report `NONE` and fail closed.
SCREEN verdicts are permitted by the owner's refinement but are not implemented here.
Screenshots alone do not establish successful work.

CODE READ: FILE checks bind the app and random run id, require guest-relative modification
 time strictly after start and no later than completion, matching nonzero byte length,
API success and recognizable STEP exchange structure with entity data. Host wall time is
not compared with the image's deliberately shifted clock. This is a structural check,
not a complete ISO STEP validator or a hostile-guest attestation.

| App | Chosen path and evidence | Saved artifact / missing qualification |
|---|---|---|
| SolidWorks 2026 | CODE/DOC READ: public COM API `OpenDoc6`, `ActivateDoc3`, `ClearSelection2`, `SaveAs`; implemented export probe | CODE READ: `model.step` plus API outcome in `result.json`; FILE means sample opened and STEP export succeeded, **not** rotation, GPU rendering, or performance. INFERRED: installed sample directory, COM activation, licensing and export need guest proof. |
| eDrawings | DOC READ: public OLE/ActiveX API exposes OpenDoc and Save; INFERRED: a native ActiveX host is preferable once installed control registration and event handling are known | CODE READ: current adapter collects `uia.json`, no default work pass. INFERRED: hardware agent must identify sample, registered control/version, load-success event and saved format; do not pretend standalone viewer is an attachable SolidWorks COM server. |
| 3DMark (activated edition unspecified) | DOC READ: Professional has CLI automation and XML export; activation alone does not establish Professional. INFERRED: choose CLI if entitlement is confirmed, otherwise collect UIA Run/Save/result controls | CODE READ: current adapter collects `uia.json`, no default work pass. DOC READ: CLI supports `--definition`, `--out`, `--export` and offline `--online=off`. INFERRED: need actual edition, installed definition, fresh `.3dmark-result` and exported XML with score/status schema before implementing its parser. No unpacking of undocumented native results. |
| Resolve 21 free | DOC READ: external scripting is Studio-only; console access is distinct. INFERRED: UIA may drive a prepared local project and render queue, or invoke an in-app console if the exact free build exposes it | CODE READ: current adapter collects `uia.json`, no default work pass. INFERRED: need project/timeline, render preset, completion controls and a new media file; validate media streams/frame count with an available trusted parser, not just size or extension. |
| Superposition Basic | INFERRED: Basic UIA result exposure is unverified on the guest. DOC READ: documented CLI automation and CSV reports are Pro-only. INFERRED: collect UIA Run, progress and finished/results controls for Basic | CODE READ: current adapter collects `uia.json`, no default work pass. INFERRED: if Basic cannot save machine-readable results, an observed new progress-to-finished transition can qualify UIA evidence after controls are measured. A renderer log or “Benchmark stopped” alone is insufficient. |
| Black Myth Wukong benchmark via Steam | DOC READ: vendor Steam app id 3132990 is the benchmark tool. CODE READ: only launches the existing desktop shortcut, never drives Steam; records fresh candidate game process ids separately as `launched` | CODE READ: launch-only JSON is never a work pass. INFERRED: candidate process names and shortcut target need hardware confirmation; no public result-file schema was established. Hardware agent should supply any result file and its provenance for a parser; Steam stays signed out. |
| Edge | DOC READ: Microsoft supports WebDriver/Playwright/Puppeteer automation; INFERRED: prefer WebDriver plus a local HTML workload and saved DOM assertions when a matching driver is available | CODE READ: current adapter collects `uia.json`, no default work pass. INFERRED: installed driver/runtime are unknown; do not download at check time. A local page title, PDF header or screenshot alone would not prove GPU work. Need a defined local rendering workload and semantic completion assertion. |

INFERRED per-app falsifiers, before any hardware experiment: SolidWorks export fails or has no
entities; eDrawings Save precedes successful load; 3DMark saves an invalid/incomplete result;
Resolve produces no decodable frames; Superposition displays an old finished result; Wukong
starts Steam alone or a pre-existing process is counted; Edge loads its window but never
finishes the defined local workload. Each falsifies the respective proposed approach.

CODE READ: optional `run --plans FILE` accepts a JSON map keyed by app (except Wukong).
Each plan has `run` and `status` selectors with `id`, `name`, `type` (e.g. `ControlType.Text`),
`running`, `finished` exact text, and a bounded `timeout` in seconds. Empty selector name
allows changing status text; id/type and an unambiguous process-scoped match remain required.
No built-in plan invents app control names. A supplied plan invokes exactly one Run button,
requires a non-running/non-finished initial state, then running, then finished. UIA observations
are saved in JSON and independently checked against the host's plan. No coordinates or
keystrokes are injected. This simple plan does not yet drive menus, Save dialogs or render setup.

INFERRED: UIA fallback is appropriate only after proving the selected app cannot save usable
results and qualifying the control semantics. An arbitrary plan is not release certification.
No SCREEN fallback is authorized by code without an app-specific work assertion.

INFERRED hardware handoff: run one unconfigured adapter to collect `uia.json` in the
interactive scheduled-task session. Collect another tree during work and at completion,
including Run/Save controls' AutomationId, Name, ControlType, supported patterns and owning
process id. Establish launcher/child process ownership, locale, default directories, sample
availability, and whether the result control exposes Name, ValuePattern or TextPattern.
The supplied dump is capped at 2048 emitted elements; its PowerShell execution is unproven.
Do not enumerate authentication UI, credentials, or the whole desktop. If no usable controls
exist, report that limitation instead of making up a selector or pixel coordinate.

CODE READ: unconfigured probes leave apps open in the disposable VM for hardware inspection;
checks stop/unregister only their own scheduled task, never process-name kill sweeps.
Normal runs shut down their own VM through the existing launcher; this lane ran no VM.

DOC READ sources (official, consulted 2026-11-13):

- [SolidWorks SaveAs API](https://help.solidworks.com/2026/English/api/sldworksapi/SOLIDWORKS.Interop.sldworks~SOLIDWORKS.Interop.sldworks.IModelDocExtension~SaveAs.html).
- [eDrawings API](https://help.solidworks.com/2026/english/eDrawings/t_Application_Programming_Interface.htm),
  [control methods](https://help.solidworks.com/2026/english/api/emodelapi/eDrawings.Interop.EModelViewControl~eDrawings.Interop.EModelViewControl.IEModelViewControl_methods.html).
- [UL edition features](https://support.benchmarks.ul.com/support/solutions/articles/44002132667-3dmark-features-by-edition),
  [CLI guide](https://support.benchmarks.ul.com/support/solutions/articles/44002145411-run-3dmark-benchmarks-from-the-command-line).
- [Resolve reference manual](https://documents.blackmagicdesign.com/UserManuals/DaVinciResolveReferenceManual.pdf), General / External Scripting Using.
- [Superposition manual](https://benchmark.unigine.com/docs/superposition/Superposition_Benchmark_User_Manual.pdf), Automation (Pro only).
- [Vendor benchmark page on Steam](https://store.steampowered.com/app/3132990/).
- [Microsoft Edge automation](https://learn.microsoft.com/en-us/microsoft-edge/test-and-automation/test-and-automation),
  [UIA FindAll](https://learn.microsoft.com/en-us/dotnet/api/system.windows.automation.automationelement.findall),
  [InvokePattern](https://learn.microsoft.com/en-us/dotnet/framework/ui-automation/invoke-a-control-using-ui-automation).

INFERRED owner/coordinator decisions: obtain actual 3DMark edition; approve the narrower
SolidWorks export probe or require a separate rendering workload; supply licensed local
samples/projects; arrange per-app hardware collection before treating any path as qualified.
