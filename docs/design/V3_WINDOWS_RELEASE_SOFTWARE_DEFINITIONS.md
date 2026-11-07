**STATUS: DESIGN-ONLY, 2026-11-07.** GPU-free definitions and checks; every release entry is `unverified`.
No Windows sample was built with MSVC or run on hardware in J1. No VM, rental, Worker call, upload or deployment.

# Windows release software definitions

**CODE READ:** [`release_software.json`](../../scripts/bench/windows/appmatrix/release_software.json) is the
packaging and verdict inventory. `gen_apps.py` generates its 17 approved definitions into `apps.json`;
`gate_profiles.json` classifies them and `gate_profiles.py ids release` resolves that exact list.
The payload still accepts only `KF_WINAPPS=small|mid|full`; absent still means no Windows apps stage.
`release` is a definition list for an eventual release bundle, with input blockers retained.
Software approval does not grant redistribution/storage rights. GravityMark and ffmpeg stay outside the open bundle.

**CODE READ:** the total inventory is now 120 apps: 87 open, 29 owner-decision, 4 manual.
All nine failures from the November hardware run remain in `full`. New source-blocked definitions also remain
visible; an unavailable sample is NOTRUN, never a pass. The previous 79-app run is historical evidence for its
old definitions, not qualification of these stricter/new rows. Heaven, Valley, Superposition and 3DMark have
no new release definitions; the older manual inventory is retained.

## The checks (CODE READ; intended hardware behavior remains unverified)

| Release definition | Non-vacuous verdict | Timeout |
|---|---|---:|
| `blender_cycles` | CUDA **and** OptiX complete the existing seed-1, 256-sample 960×540 scene; all RGBA values compare with hashed bare-metal references; MAE ≤ 1/255 and maximum error ≤ 8/255; reject empty, nonfinite and spatially flat images | 900 s |
| `llama_cuda_parity` | fixed prompt, seed 1, temperature 0, maximum 64 tokens; rc 0, positive full NVIDIA CUDA layer offload, nonempty normalized output exactly equals the same Windows executable/model's bare-metal output | 600 s (child 480 s) |
| `nvenc_h264`, `nvdec_h264` | existing ffmpeg rows; completed hardware encode/decode, rc 0, exactly 600/18000 frames, GPU proof and no failure marker | 180 s each |
| `d3d12_dxr`, `d3d12_mesh_shaders`, `d3d12_vrs` | source-built sample lives for 30 s; process-tree NVIDIA 3D/Compute activity required; no WARP flag; early exit fails | 120 s each |
| `d3d12_sampler_feedback` | same survival/proof predicate, but **SOURCE-SAMPLE-NEEDED**: pinned Microsoft tree has no sample; absent binary cannot pass | 120 s |
| `gravitymark_{d3d11,d3d12,vk,gl}` | existing benchmark mode, rc 0 and fresh image >10000 bytes, process-tree NVIDIA proof; stale output is removed first | 660 s each |
| `edge_{webgl,webgpu}_conformance` | offline focused subset: four checked results, nonzero frames, NVIDIA hardware and process GPU proof; failed/empty checks cannot pass | 180 s each |
| `deviceQuery_demo`, `bandwidthTest_demo` | existing CUDA sample's `Result = PASS` plus NVIDIA proof | 60 / 120 s |
| `torch_release_smoke` | explicit CUDA 64×64 matmul; all 4096 finite readback values compare to the CPU oracle at rtol=atol=1e-5; CPU fallback forbidden | 120 s |

**DESIGN:** the image tolerances above are initial explicit tolerances, not inferred measurements of GPU variation.
The browser pages are a focused conformance subset, not the complete Khronos/WebGPU CTS or a certification claim.
WebGL checks clear/readback, buffer transfer, shader draw/readback and invalid-enum reporting. WebGPU checks
shader compilation, compute readback, texture readback and a validation error. The current WebGPU fallback flag
is on `GPUAdapterInfo`; the page also understands the older adapter property and rejects an unknown flag
([W3C WebGPU, GPUAdapterInfo](https://www.w3.org/TR/2026/CRD-webgpu-20260915/#gpuadapterinfo)).

**Falsifiers before the oracle checks:** an empty or mismatching output passes; a guest/unidentified reference
is accepted; inference uses CPU or only partial layer offload; a patched VRS sample can run without VRS;
an unrecorded or modified asset reaches the ISO. The GPU-free tests exercise these negative paths and the
mutation harness requires two named failing tests for each mutant. They establish software checks, not GPU results.

## Reference inputs (DESIGN; none collected in this lane)

Read-only disk paths, beneath `release/references/`:

- `blender_CUDA.png`, `blender_CUDA.json`, `blender_OPTIX.png`, `blender_OPTIX.json`.
- `llama_cuda.txt`, `llama_cuda.json`.

**CODE READ:** each JSON must bind `schema:1`, `kind:"bare-metal"`, full source `revision`, `date`, nonempty
`evidence`, and the reference file's full `sha256`. Missing inputs are `REFERENCE_NOTFOUND`, never a pass.
The test fixtures contain synthetic metadata only; there is no committed passing reference.

Blender additionally binds `backend`, GPU **model** (no UUID), `blender_version`, `scene_sha256` of
`scripts/apps/src/blender_render.py`, `seed:1`, `samples:256`, `width:960`, `height:540`.
The runner executes that unchanged scene through Blender's Python, then checks every pixel using
`release_oracles.py`. Both backends must emit their comparison success before the parent emits
`CYCLES_PARITY_ALL_PASS`. A mere `BLENDER_OK OPTIX` no longer passes the release definition.

LLM additionally binds `prompt:"Explain in three sentences why the sky is blue."`, `seed:1`, `tokens:64`,
`temperature:0`, and full `executable_sha256` / `model_sha256`. Whitespace-normalized output must match
exactly and exceed 20 characters. This is Windows-native parity with the pinned Windows llama.cpp build;
the existing Linux `scripts/bench/llm_parity.sh` is context, not a cross-build equality oracle.

## Source-built DirectX inputs (CODE READ; build and hardware unverified)

Pinned repository: [Microsoft DirectX-Graphics-Samples](https://github.com/microsoft/DirectX-Graphics-Samples/tree/68a2b1e97f71642924e74c7757ca21756ad221b5),
commit `68a2b1e97f71642924e74c7757ca21756ad221b5` (MIT). The projects are
`D3D12RaytracingHelloWorld`, `D3D12MeshletRender`, and `D3D12VariableRateShading`.
**MEASURED, GPU-free:** the packaging dry-run resolves all three projects at this pin, and
`git apply --check` accepts `directx_vrs_gate.patch` against it. No MSVC compilation was performed here.

**CODE READ:** the stock VRS sample defaults to 1×1 and can draw without VRS. The recorded build patch
requires tier 1 and selects 2×2 shading for the refraction, scene and postprocess passes before initialization.
Its hash is part of the source receipt. CPU-only C++ tests compile the actual patch additions against enum/error
stubs: unsupported fails, tier 1 and tier 2 select coarse shading. These tests do not exercise D3D12 itself.

The source-build leg of the bundle script is `build_release_bundle.ps1`. On a Windows packaging workstation,
with MSVC v143, the Windows SDK, dxc and nuget.exe installed:

```powershell
.\scripts\bench\windows\appmatrix\build_release_bundle.ps1 -Source C:\src\DirectX-Graphics-Samples -Out C:\kf-release-assets -DryRun
.\scripts\bench\windows\appmatrix\build_release_bundle.ps1 -Source C:\src\DirectX-Graphics-Samples -Out C:\kf-release-assets -References C:\native-references
```

**CODE READ:** it checks the clean source pin, builds in its own temporary source worktree, applies only the
recorded VRS patch, restores the source-pinned `packages.config` versions, rebuilds the projects with two jobs,
and copies the runtime binaries, shaders, data, licences and optional six reference files into the bundle tree.
It records each file's SHA-256 and removes its temporary worktree. It runs no sample and performs no upload.
The optional references must already have been collected independently; packaging does not bless them.

`appdisk.py build --release-assets DIR` verifies the exact
source/project/patch receipt and every asset hash, then puts these files under `release/` in the ISO and its
file manifest. The old `package_open_bundle.sh --release-assets` wrapper is deprecated; the WB1 shell-free
`package_bundle.py` builds the canonical open bundle without these separately source-built assets. Definitions
that need them remain blocked until supplied by the separate release workflow. An open bundle still omits
ffmpeg and GravityMark. A future owner-supplied release bundle must
include the packages listed for all 17 definitions; no software release bundle was built or stored by this lane.

**OWNER DECISION, 2026-11-07:** retain a blocked sampler-feedback definition pending an approved Microsoft
source. The pinned Microsoft tree has no sampler-feedback project. The absent binary cannot pass;
this lane does not substitute a sample from another source.

## Media pin (MEASURED download/format checks; GPU behavior unverified)

The old ffmpeg URL returned 404 in the earlier run. J1 downloaded the dated replacement from the
[official BtbN release](https://github.com/BtbN/FFmpeg-Builds/releases/tag/autobuild-2026-11-07-12-47):
`ffmpeg-n8.1.3-34-ga35157abb9-win64-gpl-8.1.zip`, 194355634 bytes,
SHA-256 `06bfc0069beb764b75f9eed956acc90c4d651bf06aed340fbf0708d10a6f76dc`.
The computed hash equals the release's `checksums.sha256` entry and GitHub asset digest; ZIP CRC and
the ffmpeg/ffprobe entries were checked. No downloaded executable was run. `manifest.json` carries the new pin;
its old image/file receipt remains explicitly historical until a new bundle is built. MEDIA-PIN-NEEDED is
resolved for this dated artifact; neither redistribution approval nor hardware qualification is implied.

## GPU-free validation commands

**CODE READ:** `.github/workflows/vast-box-scripts.yml` runs the complete appmatrix suite and release
mutants in a separate GPU-free job; no software in these definitions is launched on a GPU by that job.

**CODE READ / GPU-free regression:** the larger mock campaign exposed a stateful QGA read replay defect.
The timeout path already refuses retry (the initial timeout-replay hypothesis was falsified), but the EOF
reconnect path could replay a read after its handle had advanced and return EOF as an empty result.
Deterministic head/tail reply-loss tests failed before the fix and pass with an absolute seek before each
read retry. Timeout and understood-refusal behavior is unchanged. The ssh transport does not use this RPC path.

```sh
python3 -m unittest discover -s scripts/bench/windows/appmatrix/tests
python3 scripts/bench/windows/appmatrix/release_mutants.py --jobs 2
python3 scripts/bench/windows/appmatrix/gen_apps.py --check
python3 scripts/bench/windows/appmatrix/gen_doc.py --check
python3 scripts/bench/windows/appmatrix/gate_profiles.py check
```

**Not verified:** native image/text references, Windows MSVC sample builds, browser execution, CUDA or codec
execution, any new guest result, sampler-feedback source, final bundle bytes, or the disabled Worker path.
