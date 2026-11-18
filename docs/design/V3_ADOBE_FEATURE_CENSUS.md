# Adobe GPU feature census

**STATUS: DESIGN-ONLY, 2026-11-18.** The offline parser and comparison have GPU-free tests. Guest commands and paired application observations await the first hardware run.

## First discovery controls

The 2026-11-16 PC night plan lists After Effects `aeparallel` and `aeadv3d`,
Photoshop's Display P3 conversion error, the InDesign access violation in
`Public.dll`, and the Premiere Pro / Media Encoder export matrix as discovery
items. These are the first suspected gaps. Compare each with the same workload
on the reference. A launch, a crash, a timed-out script, and general GPU
activity cannot establish the state of an individual feature.

## Evidence by app

CITED marks a source-backed interface or observation. INFERRED marks a
collection approach or field mapping that needs validation on the installed
build. Where a stable non-GUI interface has not been established, the census
accepts a reviewed observation with a source and otherwise reports `unknown`.

| App | CITED | INFERRED and collection path |
| --- | --- | --- |
| Photoshop | Adobe documents Help > System Info, the installed `sniffer.exe`, the `sniffer-out.txt` log, Performance preferences, and per-document GPU Mode. D3D12 is the preferred Windows mode. Metal is applicable to macOS. See [System Info](https://helpx.adobe.com/ro/photoshop/desktop/troubleshoot/troubleshoot-tools-resources/use-system-info-to-diagnose-issues.html), [sniffer](https://helpx.adobe.com/in/photoshop/kb/error-photoshop-has-encountered-a-problem-with-the-display-drive.html), [preferences](https://helpx.adobe.com/photoshop/desktop/get-started/settings-and-preferences/configure-photoshop-performance-preferences.html), and [GPU modes](https://helpx.adobe.com/ph_fil/photoshop/desktop/get-started/technical-requirements-installation/photoshop-and-graphics-processor-gpu-card-usage.html). | The JSX tries the legacy `app.systemInformation` property and allowlists three fields. Validate those labels against this build. Adobe's [UXP API](https://developer.adobe.com/photoshop/uxp/2021/ps_reference/) exposes batchPlay, but a stable equivalent getter was not established. Search the per-version settings directory in the guest profile for `sniffer-out.txt` if the app creates one. `PSUserConfig.txt` and other profile logs can describe overrides or diagnostics and are unsuitable as proof of active mode. Record any explicit D3D12/CPU mode from the test document separately from OpenCL and the GPU preference. |
| Premiere Pro | Project Settings > General > Renderer shows the active Mercury backend. Adobe documents CUDA/OpenCL selection and removal of the Software Only menu choice in 25.2. The System Compatibility Report shows driver issues. The committed sniffer trace reports CUDA/OpenCL device counts. See [renderer](https://helpx.adobe.com/premiere/desktop/get-started/technical-requirements/enable-mercury-playback-engine-gpu-accelerated-renderer.html), [version change](https://helpx.adobe.com/premiere/desktop/troubleshooting/limitations-and-known-issues/software-rendering-update.html), and [compatibility report](https://helpx.adobe.com/premiere/desktop/get-started/download-and-install/check-if-your-system-is-compatible-with-premiere.html). | Use `GPUSniffer.exe` for device observations. The [Adobe CEP sample](https://raw.githubusercontent.com/Adobe-CEP/Samples/master/PProPanel/jsx/PPRO/Premiere.jsx) uses `app.enableQE()`. Neither it nor the reviewed [UXP Project API](https://developer.adobe.com/premiere-pro/uxp/ppro-reference/classes/project) established a supported renderer-list getter or a stable scripting `Renderer` property. Record the selected value and explicitly available options from a reviewed project observation. Treat per-profile logs and project-file renderer fields as build-specific until matched to the displayed selection. |
| After Effects | Adobe documents the project acceleration getter and available types in [Application](https://developer.adobe.com/after-effects/uxp/after-effects-api/application) and [Project](https://developer.adobe.com/after-effects/uxp/after-effects-api/project), plus [Previews > GPU Information](https://helpx.adobe.com/in/after-effects/desktop/get-started/preferences-and-settings/preferences.html). The existing row uses ExtendScript to read and set project acceleration and calls `aerender` on a saved project. | The census JSX reads without changing selection. Use the same saved project and active comp. Compare the headless render and the multi-frame render control separately. A successful `aerender` run alone cannot label the renderer in use. |
| Illustrator | Performance preferences show GPU Performance, Animated Zoom and Real-Time Drawing and Editing. View switches between GPU Preview and Preview on CPU. Adobe describes a compatibility report for driver issues. See [performance preferences](https://helpx.adobe.com/illustrator/desktop/troubleshoot/cloud-document-issues/configure-gpu-performance-settings.html) and [driver diagnostics](https://helpx.adobe.com/illustrator/desktop/troubleshoot/application-crash-issues/update-gpu-drivers-for-illustrator.html). | GPU-specific preference keys and the AiSniffer format are build-specific candidates. Avoid guessing keys in the scripting preferences object. Record named UI controls as reviewed observations until a key or log has been correlated with them. |
| InDesign | Adobe documents [GPU Performance](https://helpx.adobe.com/indesign/desktop/get-started/system-and-product-info/gpu-performance.html) and its relationship to document display performance. | Capture GPU Performance and GPU Preview separately. Display quality alone cannot establish actual GPU preview. A stable scripting preference/read-only log format for these Windows fields was not established. |
| Media Encoder | Adobe documents the separate Mercury renderer choice, hardware decoding preference, and H.264/HEVC Export Settings > Performance. The committed sniffer format is shared with Premiere. See [renderer](https://helpx.adobe.com/premiere/desktop/get-started/technical-requirements/enable-mercury-playback-engine-gpu-accelerated-renderer.html), [preferences](https://helpx.adobe.com/tr/media-encoder/desktop/encoding-quick-start-and-basics/preferences.html), and [export settings](https://helpx.adobe.com/no/media-encoder/desktop/encoding-and-exporting/export-settings-reference.html). | Use the sniffer for devices. Observe hardware encode/decode per codec and fixed preset or input profile. A hardware encoding option does not name NVENC, and a hardware decoding preference does not name NVDEC. Require explicit backend evidence from the export or an isolated engine control before setting the corresponding NVIDIA feature. No stable non-GUI preset capability list was established. Apply the same evidence rules to Premiere. |
| Acrobat | CITED. Adobe documents the conditional Windows checkbox in [Page Display > Rendering](https://helpx.adobe.com/acrobat/using/viewing-pdfs-viewing-preferences.html/kvieb4v85.html). | Use a reviewed checkbox observation from the same app build and PDF. A stable non-GUI getter and registry mapping were not established. Keep an absent checkbox unknown until the expected control and conditions are established. |
| Creative Cloud | Adobe describes the [desktop application's requirements](https://helpx.adobe.com/ph_en/download-install/apps/system-requirements/creative-cloud-desktop-app-system-requirements.html) separately from the applications it manages. | Record the executable's version. No supported desktop GPU feature reporting interface was identified. Keep desktop acceleration unknown pending explicit app evidence, and omit sign-in, licensing and installer logs. |

OS context is independent of Adobe selection. The guest snippet records model
and driver fields from [Win32_VideoController](https://learn.microsoft.com/en-us/windows/win32/cimwin32prov/win32-videocontroller)
and projects model/architecture from `nvidia-smi -q -x` in memory. INFERRED
follow-ups are display fields from the DxDiag XML report (`dxdiag /x` writes a
full machine report, so it is outside the supplied collector), a helper calling
DXGI `EnumAdapters1`, `vulkaninfo --summary` with an allowlist, and
`clinfo` or `clGetDeviceIDs` with an allowlist. Run helpers in the same
interactive session and bitness as the app. The app's own sniffer or selection
is needed to establish what the app saw. Keep identifiers, paths with user
data, full diagnostic XML, account and licensing information out of evidence.

## Contract and offline use

Each census is a JSON array of `{app, version, features, raw_files}`.
Features carry `{name, state, reason, source}`. `source` is the basename
in `raw_files`, or null for a missing observation. Record
`enabled` for an explicitly selected, offered or positively observed feature
as named, `disabled` for an explicit inactive selection, `unsupported` for an
explicit capability rejection, and `unknown` when there is no adequate evidence.
Device enumeration and selected renderer have different feature names.
The differ reports the union where exactly one side is enabled, and includes
both versions, states, reason strings and sources. A pair with different app
versions is still visible in the report and must be controlled before triage.

The evidence manifest is a mapping from app name to a list of
`{"format":"observations","path":"app/version.tsv"}` objects. Paths are
relative to the manifest or absolute. The CLI emits all eight apps, with
uncollected features and versions set to unknown. Available formats are
`observations` for all apps, `system-info` for Photoshop, and `sniffer` for
Photoshop, After Effects, Premiere Pro and Media Encoder. The TSV observation
format uses literal tabs between the following fields.

```text
version	27.0.0
renderer	cuda	Mercury Playback Engine GPU Acceleration (CUDA)
feature	hardware-encoding.h264	disabled	Performance shows Software Encoding
```

Use one observation per feature and one source of version for each app.
`renderer` is available for the three Mercury apps. It accepts `cuda`,
`opencl`, `software`, or `unknown`, with an optional original reason. For
a UI-only field, use a reviewed TSV line with the displayed reason. Record
only allowlisted feature observations and technical reason text. The renderer
and preset examples above are synthetic.

```sh
export PYTHONPATH=scripts/ci
python3 -m adobe_census parse reference-inputs.json > reference.json
python3 -m adobe_census parse kf3-inputs.json > kf3.json
python3 -m adobe_census validate reference.json
python3 -m adobe_census diff reference.json kf3.json > diff.md
python3 -m adobe_census diff reference.json kf3.json --format json > diff.json
```

CITED fixture `mercury-sniffer.txt` contains three lines from the
[Premiere sniffer trace](../../traces/imageb_apps_20261116/rows/gpudiag/sniffer_premiere.txt).
The After Effects and Photoshop fixtures are INFERRED synthetic text for
parser testing. The workspace also has the [After Effects toggle trace](../../traces/imageb_apps_20261116/second_pass/rows/aetoggle/ae_toggle.txt)
showing the forced settings and render times. These are workload observations
under explicit setter calls, so the census does not import them as startup
state.

## Guest commands and first paired run

Run in an interactive disposable clone using the row's installed application
paths and the selected image configuration. Set the PowerShell variables
`$Tools`, `$AppExe`, `$Sniffer`, `$AfterFX`, `$Project`, `$Aerender` and
`$Out` from that configuration and the test asset. Use a fresh `$Out` per
app. These command examples are INFERRED until executed in the guest.

```powershell
# Premiere Pro, Media Encoder, or After Effects with its GPUSniffer.exe
& "$Tools\collect_probe.ps1" -AppExe $AppExe -Sniffer $Sniffer -Out $Out

# Photoshop with its sniffer.exe
& "$Tools\collect_probe.ps1" -AppExe $AppExe -Sniffer $Sniffer -Photoshop -Out $Out

# Illustrator, InDesign, Acrobat, or Creative Cloud version
& "$Tools\collect_probe.ps1" -AppExe $AppExe -Out $Out

# Photoshop legacy script from the interactive row or File > Scripts > Browse
$ps = New-Object -ComObject Photoshop.Application
$ps.DoJavaScriptFile("$Tools\photoshop.jsx")
# Use the sanitized %TEMP%\kf-ps-system-info.txt as system-info.

# With the intended test comp active, and scripting file writes enabled
& $AfterFX '-r' "$Tools\after_effects.jsx"
# Use the sanitized %TEMP%\kf-ae-observations.tsv as observations.

# Existing render control, with -comp and frame range supplied by the row
& $Aerender '-project' $Project

& "$Tools\os_context.ps1" -Out "$Out\os-context.json"
```

Remove any stale temporary output before launching a JSX probe. Wait for
completion with the row's existing bounded runner. When System Info or the AE
JSX reports a version, omit that app's version-only TSV from the manifest.
Where only UI controls are established, use manual observations or a future
accessibility collector keyed to control names. DESIGN INTENT. Keep guest collection observational until a guest run establishes the formats.

Run kf3 inside the existing proprietary Windows app rows with the harness's
text evidence allowlist. Reference preference is real VFIO on the PC, subject
to owner approval and the safe-bind rules. Native Windows on a Vast box is
an alternative reference pending a later decision. Match the GPU die, driver,
Windows and app builds, application preferences, display configuration,
test projects and media presets. Use cold starts and comparable disposable
clones and record the kf3 source revision. Hardware and PC commands are
outside this change.

Hypothesis for each suspected gap is that the reference enables the target
feature while kf3 does not. It is falsified by matching states with adequate
evidence on both sides, or by the reverse directional result. Reproduce both
sides with the same observation method before assigning each candidate to
documented limitation, kayfabe gap with a test first, app-harness bug with a
control, or our configuration error.
